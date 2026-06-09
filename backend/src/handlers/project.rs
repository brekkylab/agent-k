use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    events::WsEvent,
    handlers::session::cleanup_session_resources,
    model::{
        AddMemberRequest, CreateProjectRequest, ProjectListResponse, ProjectMemberListResponse,
        ProjectMemberResponse, ProjectResponse, UpdateProjectRequest,
    },
    repository::{RepositoryError, RepositoryResult, SqliteRepository},
    state::AppState,
};

// ── Slug helpers ──────────────────────────────────────────────────────────────

/// Resolve a project reference (UUID, active slug, or retired slug) to its UUID.
///
/// Mirrors `resolve_session_id` in `handlers::session`: a path segment can be
/// either the canonical UUID or a slug, and the handler doesn't care which.
/// Retired slug fallback keeps links to a renamed project working. Returns 404
/// only when none of the three resolutions succeed.
pub async fn resolve_project_id(state: &Arc<AppState>, project_ref: &str) -> ApiResult<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(project_ref) {
        return Ok(uuid);
    }
    if let Some(p) = state
        .repository
        .get_project_by_slug(project_ref)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        return Ok(p.id);
    }
    if let Some(id) = state
        .repository
        .get_project_id_by_retired_slug(project_ref)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
    {
        return Ok(id);
    }
    Err(AppError::not_found("project not found"))
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

/// Validate a user-provided slug against the same shape `slug::slugify` produces.
///
/// The server-generated path is always safe by construction; this guard mirrors
/// the same rule for explicit slugs that arrive via the API contract, so seed
/// scripts and any future rename UI cannot quietly stash a malformed slug in
/// the database (`my/project`, `한글`, leading/trailing hyphens, etc.).
fn validate_explicit_slug(s: &str) -> ApiResult<()> {
    if s.is_empty() || s.len() > 64 {
        return Err(AppError::bad_request("slug must be 1-64 characters"));
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(AppError::bad_request(
            "slug must start and end with an alphanumeric character",
        ));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::bad_request(
            "slug must contain only lowercase letters, digits, and hyphens",
        ));
    }
    if s.contains("--") {
        return Err(AppError::bad_request(
            "slug cannot contain consecutive hyphens",
        ));
    }
    Ok(())
}

/// Generate a slug that is unique among active and retired project slugs.
///
/// Starts from `slug::slugify(name)`; if that is empty, falls back to
/// `project-<6-char nanoid>`. Appends `-2`, `-3`, … on collision. When
/// `self_project_id` is provided, slugs already owned (active or retired) by
/// that project are not treated as collisions — useful for rename flows where
/// re-deriving the same slug should yield a no-op rather than `-2`.
async fn generate_unique_slug(
    name: &str,
    repo: &SqliteRepository,
    self_project_id: Option<Uuid>,
) -> RepositoryResult<String> {
    let mut base = slug::slugify(name);
    if base.is_empty() {
        base = format!("project-{}", nanoid::nanoid!(6));
    }
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    loop {
        let active_owner = repo.get_project_by_slug(&candidate).await?.map(|p| p.id);
        let retired_owner = repo.get_project_id_by_retired_slug(&candidate).await?;
        let taken_by_other = match (active_owner, retired_owner) {
            (Some(id), _) => Some(id) != self_project_id,
            (None, Some(id)) => Some(id) != self_project_id,
            (None, None) => false,
        };
        if !taken_by_other {
            return Ok(candidate);
        }
        candidate = format!("{base}-{suffix}");
        suffix += 1;
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
            validate_explicit_slug(&s)?;
            ensure_slug_not_retired_by_other(&state.repository, &s, None).await?;
            s
        }
        None => generate_unique_slug(&payload.name, &state.repository, None)
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

/// GET /projects/{project_ref}
///
/// `project_ref` may be a UUID, an active slug, or a retired slug; the response
/// `id`/`slug` fields tell the caller whether a redirect is needed (retired slug
/// resolves to a project whose current slug differs from the request path).
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
) -> ApiResult<Json<ProjectResponse>> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
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

/// PATCH /projects/{project_ref} — owner only
///
/// Slug isn't taken from the payload; if `name` changes, the new slug is
/// derived from it via `generate_unique_slug` (same path as create). The
/// previous slug is retired so old links keep resolving.
pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
    Json(payload): Json<UpdateProjectRequest>,
) -> ApiResult<Json<ProjectResponse>> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
    require_owner(&state, auth_user.id, project_id).await?;

    let new_slug = if let Some(ref new_name) = payload.name {
        Some(
            generate_unique_slug(new_name, &state.repository, Some(project_id))
                .await
                .map_err(|e| AppError::internal(e.to_string()))?,
        )
    } else {
        None
    };

    // Validate and serialize any recommendation-chain overrides. Keys must be
    // known agent types and values catalogued model ids (provider availability
    // not required); `None` leaves the stored chains unchanged.
    let new_chains = match payload.recommended_chains {
        None => None,
        Some(chains) => {
            crate::model::validate_chains(&chains).map_err(AppError::bad_request)?;
            Some(serde_json::to_string(&chains).map_err(|e| AppError::internal(e.to_string()))?)
        }
    };

    let new_pdf_engine = match payload.pdf_engine {
        None => None,
        Some(ref s) => {
            let engine = agent_k::knowledge_base::PdfEngine::from_str_opt(s)
                .ok_or_else(|| AppError::bad_request("pdf_engine must be 'kreuzberg' or 'docling'"))?;
            Some(engine.as_str().to_string())
        }
    };

    // Did the PDF engine actually change? If so the existing corpus was parsed
    // by the old engine and is stale — it must be rebuilt. Compare against the
    // current stored value before the update.
    let engine_changed = match &new_pdf_engine {
        None => false,
        Some(new) => {
            let current = state
                .repository
                .get_project(project_id)
                .await
                .ok()
                .flatten()
                .and_then(|p| p.pdf_engine);
            current.as_deref() != Some(new.as_str())
        }
    };

    let updated = state
        .repository
        .update_project(
            project_id,
            payload.name,
            payload.description.map(Some),
            new_slug,
            new_chains,
            new_pdf_engine,
        )
        .await
        .map_err(|e| match e {
            RepositoryError::UniqueViolation(_) => AppError::conflict("slug already in use"),
            other => AppError::internal(other.to_string()),
        })?;

    // Rebuild the corpus under the new engine: drop the cached store, delete the
    // derived corpus/index on disk (originals stay in the knowledge folder), and
    // trigger a resync that re-parses every PDF with the new engine.
    if engine_changed {
        state.evict_store(project_id);
        let speedwagon_dir = state
            .data_root
            .join("projects")
            .join(project_id.to_string())
            .join(".speedwagon");
        if let Err(e) = tokio::fs::remove_dir_all(&speedwagon_dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%project_id, "failed to clear corpus on engine change: {e}");
            }
        }
        super::knowledge::maybe_trigger_resync(&state, project_id, true);
    }

    Ok(Json(ProjectResponse::from(updated)))
}

/// DELETE /projects/{project_ref} — owner only (cascades sessions)
pub async fn delete_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
    require_owner(&state, auth_user.id, project_id).await?;

    // Clean up agent + sandbox for every session before the DB cascade removes them.
    let sessions = state
        .repository
        .list_all_sessions_in_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    for session in sessions {
        cleanup_session_resources(&state, session.project_id, session.id).await;
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

    tracing::info!(id = %project_id, "project deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /projects/{project_ref}/members
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
) -> ApiResult<Json<ProjectMemberListResponse>> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
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

/// POST /projects/{project_ref}/members — owner only, body: { username }
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
    Json(payload): Json<AddMemberRequest>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
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

/// DELETE /projects/{project_ref}/members/{user_id}
/// Owner can remove anyone. Member can only remove themselves (leave).
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((project_ref, target_user_id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    let project_id = resolve_project_id(&state, &project_ref).await?;

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

    // [C] Broadcast AccessRevoked for every session in the project so open WS connections
    // belonging to the removed user drop their subscriptions immediately.
    if let Ok(sessions) = state
        .repository
        .list_all_sessions_in_project(project_id)
        .await
    {
        for session in sessions {
            let _ = state.ws_tx.send(WsEvent::AccessRevoked {
                session_id: session.id.to_string(),
                user_id: target_user_id.to_string(),
            });
        }
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

#[cfg(test)]
mod slug_validation_tests {
    use super::validate_explicit_slug;

    #[test]
    fn accepts_well_formed_slugs() {
        for ok in [
            "a",
            "p1",
            "my-project",
            "klient-co-q2",
            "project-1234",
            "abc123-xyz",
        ] {
            assert!(
                validate_explicit_slug(ok).is_ok(),
                "expected {ok:?} to be valid"
            );
        }
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(validate_explicit_slug("").is_err());
        let too_long = "a".repeat(65);
        assert!(validate_explicit_slug(&too_long).is_err());
    }

    #[test]
    fn rejects_uppercase_and_non_ascii() {
        assert!(validate_explicit_slug("MyProject").is_err());
        assert!(validate_explicit_slug("한글-슬러그").is_err());
        assert!(validate_explicit_slug("project-🚀").is_err());
    }

    #[test]
    fn rejects_path_breaking_characters() {
        assert!(validate_explicit_slug("my project").is_err()); // space
        assert!(validate_explicit_slug("my/project").is_err()); // slash
        assert!(validate_explicit_slug("my?project").is_err()); // query
        assert!(validate_explicit_slug("my.project").is_err()); // dot
    }

    #[test]
    fn rejects_edge_hyphens_and_doubles() {
        assert!(validate_explicit_slug("-leading").is_err());
        assert!(validate_explicit_slug("trailing-").is_err());
        assert!(validate_explicit_slug("my--project").is_err());
        assert!(validate_explicit_slug("-").is_err());
    }
}
