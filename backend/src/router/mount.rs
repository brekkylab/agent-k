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

use ::workspace::{GmailConfig, NotionConfig, ProviderConfig, S3Config};

use crate::{
    auth::AuthUser,
    state::{AppState, GoogleOAuth, WorkspaceMount},
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
    /// Gmail via Google OAuth.
    ///
    /// Scope: request `https://www.googleapis.com/auth/gmail.modify` at consent.
    /// It covers everything this provider does or would do — read (list/get/full/
    /// attachments/labels), trash, and even send if that's wired later — and only
    /// omits permanent delete (`messages.delete`), which we don't use (we trash).
    /// NOT `gmail.metadata` (forbids `q=`/`format=full`); `mail.google.com` is
    /// over-broad. Consent must use `access_type=offline` + `prompt=consent` so
    /// Google actually returns a refresh token.
    ///
    /// The frontend runs the OAuth consent and sends only the authorization
    /// `code` (+ the `redirect_uri` used at consent). The backend exchanges it
    /// server-side with the app's config-held client credentials
    /// ([`GoogleOAuth`]) into a refresh token, so the browser never handles the
    /// client secret.
    Gmail {
        code: String,
        redirect_uri: String,
    },
}

impl ProviderSpec {
    /// Resolve into a live [`ProviderConfig`]. S3/Notion carry their credentials
    /// directly; Gmail exchanges its authorization `code` for a refresh token
    /// server-side with the app's [`GoogleOAuth`] client, so the browser never
    /// handles the client secret.
    async fn resolve(self, oauth: &GoogleOAuth) -> Result<ProviderConfig, ApiError> {
        Ok(match self {
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
            ProviderSpec::Gmail { code, redirect_uri } => {
                let (client_id, client_secret) = oauth.credentials().ok_or_else(|| {
                    err(
                        StatusCode::BAD_REQUEST,
                        "Gmail is not configured on this server \
                         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)",
                    )
                })?;
                let refresh_token = ::workspace::exchange_gmail_code(
                    client_id,
                    client_secret,
                    &code,
                    &redirect_uri,
                )
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("gmail oauth: {e}")))?;
                ProviderConfig::Gmail(GmailConfig {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                    refresh_token,
                })
            }
        })
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
    /// Gmail carries only OAuth secrets — nothing non-secret to show.
    Gmail {},
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
            ProviderConfig::Gmail(_) => ProviderInfo::Gmail {},
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
    let provider = payload.provider.resolve(&state.google_oauth).await?;
    let mount = WorkspaceMount::new(wid, payload.prefix, provider);
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
