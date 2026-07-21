//! HTTP API for a workspace's external-provider mounts.
//!
//! `GET/POST /workspaces/{wid}/mounts` and `DELETE
//! /workspaces/{wid}/mounts/{mount_id}`. Every route is gated by
//! [`require_owned_workspace`], so a workspace the caller can't reach is a 404.
//!
//! Responses never echo credentials back: [`ProviderInfo`] carries only the
//! non-secret identifying fields (bucket, region, …), never the access keys or
//! API tokens the request supplied.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ::workspace::{NotionConfig, ProviderConfig, S3Config};

use crate::{
    auth::AuthUser,
    state::{AppState, WorkspaceMount},
};

use super::{error::ApiError, error::err, workspace::require_owned_workspace};

/// Provider configuration as supplied when creating a mount (carries secrets).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderSpec {
    S3 {
        bucket: String,
        /// Defaults to `us-east-1` when omitted.
        #[serde(default)]
        region: Option<String>,
        access_key_id: String,
        secret_access_key: String,
        /// Custom endpoint for S3-compatible stores (MinIO, R2, …).
        #[serde(default)]
        endpoint: Option<String>,
        /// Restrict the mount to keys under this prefix.
        #[serde(default)]
        key_prefix: Option<String>,
    },
    Notion {
        api_key: String,
    },
}

impl From<ProviderSpec> for ProviderConfig {
    fn from(spec: ProviderSpec) -> Self {
        match spec {
            ProviderSpec::S3 {
                bucket,
                region,
                access_key_id,
                secret_access_key,
                endpoint,
                key_prefix,
            } => ProviderConfig::S3(S3Config {
                bucket,
                region: region.unwrap_or_else(|| "us-east-1".to_string()),
                access_key_id,
                secret_access_key,
                endpoint,
                key_prefix,
            }),
            ProviderSpec::Notion { api_key } => ProviderConfig::Notion(NotionConfig { api_key }),
        }
    }
}

/// Non-secret view of a mount's provider, safe to return in responses.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderInfo {
    S3 {
        bucket: String,
        region: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        key_prefix: Option<String>,
    },
    /// Notion carries only the API token, which is a secret — nothing to show.
    Notion {},
}

impl From<&ProviderConfig> for ProviderInfo {
    fn from(p: &ProviderConfig) -> Self {
        match p {
            ProviderConfig::S3(c) => ProviderInfo::S3 {
                bucket: c.bucket.clone(),
                region: c.region.clone(),
                endpoint: c.endpoint.clone(),
                key_prefix: c.key_prefix.clone(),
            },
            ProviderConfig::Notion(_) => ProviderInfo::Notion {},
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MountResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub prefix: String,
    pub provider: ProviderInfo,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<WorkspaceMount> for MountResponse {
    fn from(m: WorkspaceMount) -> Self {
        Self {
            id: m.id,
            workspace_id: m.workspace_id,
            prefix: m.prefix,
            provider: ProviderInfo::from(&m.provider),
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct MountListResponse {
    pub items: Vec<MountResponse>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMountRequest {
    /// Virtual top-level prefix (e.g. `/s3-prod` or `s3-prod`); normalised to a
    /// single absolute segment.
    pub prefix: String,
    pub provider: ProviderSpec,
}

/// `GET /workspaces/{wid}/mounts` — list the workspace's mounts.
pub(super) async fn list_mounts(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
) -> Result<Json<MountListResponse>, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let items = state
        .workspaces
        .list_mounts(wid)
        .await?
        .into_iter()
        .map(MountResponse::from)
        .collect();
    Ok(Json(MountListResponse { items }))
}

/// `POST /workspaces/{wid}/mounts` — attach a new external-provider mount.
pub(super) async fn create_mount(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path(wid): Path<Uuid>,
    Json(payload): Json<CreateMountRequest>,
) -> Result<(StatusCode, Json<MountResponse>), ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let mount = WorkspaceMount::new(wid, payload.prefix, payload.provider.into());
    let created = state.workspaces.create_mount(mount).await?;
    Ok((StatusCode::CREATED, Json(MountResponse::from(created))))
}

/// `DELETE /workspaces/{wid}/mounts/{mount_id}` — detach a mount. A mount that
/// belongs to another workspace is reported as `404`, like a missing one.
pub(super) async fn delete_mount(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((wid, mount_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    match state.workspaces.get_mount(mount_id).await? {
        Some(m) if m.workspace_id == wid => {}
        _ => return Err(err(StatusCode::NOT_FOUND, "mount not found")),
    }
    state.workspaces.remove_mount(mount_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
