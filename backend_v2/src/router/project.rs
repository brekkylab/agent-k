use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{AppState, Project, StateError};

use super::error::ApiError;

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Project> for ProjectResponse {
    fn from(p: Project) -> Self {
        Self {
            id: p.id,
            title: p.title,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectListResponse {
    pub items: Vec<ProjectResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProjectRequest {
    pub title: Option<String>,
}

pub(super) async fn list_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProjectListResponse>, ApiError> {
    let projects = state.projects.list().await?;
    Ok(Json(ProjectListResponse {
        items: projects.into_iter().map(ProjectResponse::from).collect(),
    }))
}

pub(super) async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), ApiError> {
    let project = Project::new(payload.title);
    state.projects.upsert(project.clone()).await?;
    Ok((StatusCode::CREATED, Json(ProjectResponse::from(project))))
}

pub(super) async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ProjectResponse>, ApiError> {
    let project = state
        .projects
        .get(id)
        .await?
        .ok_or(StateError::NotFound)?;
    Ok(Json(ProjectResponse::from(project)))
}

pub(super) async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectResponse>, ApiError> {
    let existing = state
        .projects
        .get(id)
        .await?
        .ok_or(StateError::NotFound)?;
    let updated = match payload.title {
        Some(t) => existing.with_title(t).with_updated_at(),
        None => existing,
    };
    state.projects.upsert(updated.clone()).await?;
    Ok(Json(ProjectResponse::from(updated)))
}

pub(super) async fn delete_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.projects.remove(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
