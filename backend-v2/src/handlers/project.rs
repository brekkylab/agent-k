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
    model::{
        AddMemberRequest, CreateProjectRequest, ProjectListResponse, ProjectMemberListResponse,
        ProjectMemberResponse, ProjectResponse, UpdateProjectRequest,
    },
    repository::RepositoryError,
    state::AppState,
};

/// POST /projects
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Json(payload): Json<CreateProjectRequest>,
) -> ApiResult<(StatusCode, Json<ProjectResponse>)> {
    let project = state
        .repository
        .create_project(payload.name, payload.description, auth_user.id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    tracing::info!(id = %project.id, owner = %auth_user.id, "project created");
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

/// GET /projects/{project_id}
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<ProjectResponse>> {
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

/// PATCH /projects/{project_id} — owner only
pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<UpdateProjectRequest>,
) -> ApiResult<Json<ProjectResponse>> {
    require_owner(&state, auth_user.id, project_id).await?;

    let updated = state
        .repository
        .update_project(project_id, payload.name, payload.description.map(Some))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(ProjectResponse::from(updated)))
}

/// DELETE /projects/{project_id} — owner only (cascades sessions)
pub async fn delete_project(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    require_owner(&state, auth_user.id, project_id).await?;

    state
        .repository
        .delete_project(project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    tracing::info!(id = %project_id, "project deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// GET /projects/{project_id}/members
pub async fn list_members(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
) -> ApiResult<Json<ProjectMemberListResponse>> {
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

/// POST /projects/{project_id}/members — owner only, body: { username }
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<AddMemberRequest>,
) -> ApiResult<StatusCode> {
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

/// DELETE /projects/{project_id}/members/{user_id}
/// Owner can remove anyone. Member can only remove themselves (leave).
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((project_id, target_user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let is_owner = state
        .repository
        .user_is_project_owner(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    if !is_owner {
        // Non-owner may only remove themselves
        if auth_user.id != target_user_id {
            return Err(AppError::forbidden("only the project owner can remove other members"));
        }
        // Owner is not in the members table, but guard against the edge case anyway
        let is_target_owner = state
            .repository
            .user_is_project_owner(target_user_id, project_id)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
        if is_target_owner {
            return Err(AppError::bad_request("owner cannot leave; transfer ownership first"));
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
        return Err(AppError::bad_request("owner cannot leave; transfer ownership first"));
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
