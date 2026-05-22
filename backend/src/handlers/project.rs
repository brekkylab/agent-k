use std::{future::Future, sync::Arc};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    handlers::session::cleanup_session_resources,
    model::{
        AddMemberRequest, CreateProjectRequest, ProjectListResponse, ProjectMemberListResponse,
        ProjectMemberResponse, ProjectResponse, UpdateProjectRequest,
    },
    repository::{RepositoryError, RepositoryResult, SqliteRepository},
    state::AppState,
};

// ── Slug helpers ──────────────────────────────────────────────────────────────

/// Resolve a slug to its project UUID.
///
/// Falls back to the slug history if no active project owns the slug, so
/// links to a renamed project keep working. Returns 404 only when neither
/// the active nor the retired lookup finds anything.
pub async fn resolve_project_id(state: &Arc<AppState>, slug: &str) -> ApiResult<Uuid> {
    if let Some(p) = state
        .repository
        .get_project_by_slug(slug)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        return Ok(p.id);
    }
    if let Some(id) = state
        .repository
        .get_project_id_by_retired_slug(slug)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        return Ok(id);
    }
    Err(AppError::not_found("project not found"))
}

/// Whether a slug is available for assignment to a new (or renamed) project.
///
/// Considers both active project slugs and retired ones — retired slugs are
/// permanently reserved so the redirect history stays unambiguous.
async fn is_slug_taken(repo: &SqliteRepository, candidate: &str) -> RepositoryResult<bool> {
    if repo.get_project_by_slug(candidate).await?.is_some() {
        return Ok(true);
    }
    if repo
        .get_project_id_by_retired_slug(candidate)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(false)
}

/// Reject a slug that another project has already retired.
///
/// A retired slug stays bound to whichever project owned it last — links to
/// the old URL must keep resolving — so a different project cannot claim it.
/// The project itself is allowed to reuse a slug it had previously retired.
async fn ensure_slug_not_retired_by_other(
    repo: &SqliteRepository,
    slug: &str,
    self_project_id: Option<Uuid>,
) -> ApiResult<()> {
    let owner = repo
        .get_project_id_by_retired_slug(slug)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    match owner {
        Some(o) if Some(o) != self_project_id => Err(AppError::conflict("slug already in use")),
        _ => Ok(()),
    }
}

/// Generate a slug that is unique among active and retired project slugs.
///
/// Starts from `slug::slugify(name)`; if that is empty, falls back to
/// `project-<6-char nanoid>`. Appends `-2`, `-3`, … on collision.
fn generate_unique_slug<'a>(
    name: &'a str,
    repo: &'a SqliteRepository,
) -> impl Future<Output = RepositoryResult<String>> + 'a {
    let base = {
        let s = slug::slugify(name);
        if s.is_empty() {
            format!("project-{}", nanoid::nanoid!(6))
        } else {
            s
        }
    };
    async move {
        let mut candidate = base.clone();
        let mut suffix = 2u32;
        while is_slug_taken(repo, &candidate).await? {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        Ok(candidate)
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /projects
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreateProjectRequest>,
) -> ApiResult<(StatusCode, Json<ProjectResponse>)> {
    let slug = match payload.slug {
        Some(s) => {
            ensure_slug_not_retired_by_other(&state.repository, &s, None).await?;
            s
        }
        None => generate_unique_slug(&payload.name, &state.repository)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?,
    };

    let project = state
        .repository
        .create_project(payload.name, payload.description, auth_user.id, slug)
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("slug already in use"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(id = %project.id, slug = %project.slug, owner = %auth_user.id, "project created");
    Ok((StatusCode::CREATED, Json(ProjectResponse::from(project))))
}

/// GET /projects
pub async fn list_projects(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
) -> ApiResult<Json<ProjectListResponse>> {
    let projects = state
        .repository
        .list_projects_for_user(auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(ProjectListResponse {
        items: projects.into_iter().map(ProjectResponse::from).collect(),
    }))
}

/// GET /projects/{project_slug}
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_slug): Path<String>,
) -> ApiResult<Json<ProjectResponse>> {
    let project_id = resolve_project_id(&state, &project_slug).await?;

    let project = state
        .repository
        .get_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("project not found"))?;

    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }

    Ok(Json(ProjectResponse::from(project)))
}

/// PATCH /projects/{project_slug} — owner only
pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_slug): Path<String>,
    Json(payload): Json<UpdateProjectRequest>,
) -> ApiResult<Json<ProjectResponse>> {
    let project_id = resolve_project_id(&state, &project_slug).await?;
    require_owner(&state, auth_user.id, project_id).await?;

    if let Some(ref new_slug) = payload.slug {
        ensure_slug_not_retired_by_other(&state.repository, new_slug, Some(project_id)).await?;
    }

    let updated = state
        .repository
        .update_project(
            project_id,
            payload.name,
            payload.description.map(Some),
            payload.slug,
        )
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("slug already in use"),
            other => AppError::internal(other.to_string()),
        })?;

    Ok(Json(ProjectResponse::from(updated)))
}

/// DELETE /projects/{project_slug} — owner only (cascades sessions)
pub async fn delete_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_slug): Path<String>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_slug).await?;
    require_owner(&state, auth_user.id, project_id).await?;

    // Clean up agent + sandbox for every session before the DB cascade removes them.
    let sessions = state
        .repository
        .list_all_sessions_in_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    for session in sessions {
        cleanup_session_resources(&state, session.id).await;
    }

    state
        .repository
        .delete_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    // Best-effort cleanup after DB delete; ignore NotFound (project may never have had uploads)
    let project_dir = state
        .data_root
        .join("projects")
        .join(project_id.to_string());
    if let Err(e) = tokio::fs::remove_dir_all(&project_dir).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(id = %project_id, "failed to remove project dir: {e}");
        }
    }

    tracing::info!(id = %project_id, slug = %project_slug, "project deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /projects/{project_slug}/members
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_slug): Path<String>,
) -> ApiResult<Json<ProjectMemberListResponse>> {
    let project_id = resolve_project_id(&state, &project_slug).await?;
    require_member(&state, auth_user.id, project_id).await?;

    let members = state
        .repository
        .list_project_members(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let items = members
        .into_iter()
        .map(|(u, added_at)| ProjectMemberResponse {
            user_id: u.id,
            username: u.username,
            display_name: u.display_name,
            added_at,
        })
        .collect();

    Ok(Json(ProjectMemberListResponse { items }))
}

/// POST /projects/{project_slug}/members — owner only, body: { username }
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_slug): Path<String>,
    Json(payload): Json<AddMemberRequest>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_slug).await?;
    require_owner(&state, auth_user.id, project_id).await?;

    let target = state
        .repository
        .get_user_by_username(&payload.username)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found("user not found"))?;

    state
        .repository
        .add_project_member(project_id, target.id)
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("user is already a member"),
            other => AppError::internal(other.to_string()),
        })?;

    tracing::info!(project = %project_id, user = %target.id, "member added");
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /projects/{project_slug}/members/{user_id}
/// Owner can remove anyone. Member can only remove themselves (leave).
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((project_slug, target_user_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_slug).await?;

    let is_owner = state
        .repository
        .user_is_project_owner(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if !is_owner {
        // Non-owner may only remove themselves
        if auth_user.id != target_user_id {
            return Err(AppError::forbidden(
                "only the project owner can remove other members",
            ));
        }
        // Owner is not in the members table, but guard against the edge case anyway
        let is_target_owner = state
            .repository
            .user_is_project_owner(target_user_id, project_id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        if is_target_owner {
            return Err(AppError::bad_request(
                "owner cannot leave; transfer ownership first",
            ));
        }
        // Confirm requester is a member (not just any user)
        let is_member = state
            .repository
            .user_in_project(auth_user.id, project_id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        if !is_member {
            return Err(AppError::forbidden("not a member of this project"));
        }
    } else if auth_user.id == target_user_id {
        // Owner trying to remove themselves
        return Err(AppError::bad_request(
            "owner cannot leave; transfer ownership first",
        ));
    }

    let removed = state
        .repository
        .remove_project_member(project_id, target_user_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if !removed {
        return Err(AppError::not_found("member not found"));
    }

    tracing::info!(project = %project_id, user = %target_user_id, "member removed");
    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn require_member(state: &Arc<AppState>, user_id: Uuid, project_id: Uuid) -> ApiResult<()> {
    let exists = state
        .repository
        .get_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();
    if !exists {
        return Err(AppError::not_found("project not found"));
    }
    let is_member = state
        .repository
        .user_in_project(user_id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        Err(AppError::forbidden("not a member of this project"))
    } else {
        Ok(())
    }
}

async fn require_owner(state: &Arc<AppState>, user_id: Uuid, project_id: Uuid) -> ApiResult<()> {
    let exists = state
        .repository
        .get_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .is_some();
    if !exists {
        return Err(AppError::not_found("project not found"));
    }
    let is_owner = state
        .repository
        .user_is_project_owner(user_id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_owner {
        Err(AppError::forbidden("owner access required"))
    } else {
        Ok(())
    }
}
