use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Query, State},
};

use crate::{
    authn::AuthUser,
    error::{ApiResult, AppError},
    model::{ModelCatalogResponse, ProjectChains, catalog_response},
    state::AppState,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ListModelsQuery {
    /// Project UUID or slug; when given, chains reflect that project's custom
    /// overrides. Omit for the built-in default chains.
    pub project_ref: Option<String>,
}

/// GET /models — catalog (grouped by tier), per-agent recommendation chains, and
/// live provider availability. `?project_ref=` applies that project's custom chains.
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Query(query): Query<ListModelsQuery>,
) -> ApiResult<Json<ModelCatalogResponse>> {
    let chains = match query.project_ref {
        None => ProjectChains::default(),
        Some(ref project_ref) => {
            let project_id = super::project::resolve_project_id(&state, project_ref).await?;
            let is_member = state
                .repository
                .user_in_project(auth_user.id, project_id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
            if !is_member {
                return Err(AppError::forbidden("not a member of this project"));
            }
            let project = state
                .repository
                .get_project(project_id)
                .await
                .map_err(|e| AppError::internal(e.to_string()))?
                .ok_or_else(|| AppError::not_found("project not found"))?;
            ProjectChains::parse(project.recommended_chains.as_deref())
        }
    };

    Ok(Json(catalog_response(&chains)))
}
