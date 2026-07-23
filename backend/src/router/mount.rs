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

use ::workspace::{GdriveConfig, GmailConfig, NotionConfig, ProviderConfig, S3Config};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    error::{ApiError, err},
    workspace::require_owned_workspace,
};
use crate::{
    auth::AuthUser,
    state::{AppState, GoogleOAuth, WorkspaceMount},
};

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
    /// Scope: request `https://www.googleapis.com/auth/gmail.readonly` at consent
    /// — the mount is read-only (list/get/full/attachments/labels), so this is
    /// least-privilege. NOT `gmail.metadata` (forbids `q=`/`format=full`).
    /// Consent must use `access_type=offline` + `prompt=consent` so Google
    /// actually returns a refresh token.
    ///
    /// The provider keeps dormant write paths (trash-on-rm, send/reply/forward
    /// commands) that today just 403 under this scope; switching consent to
    /// `gmail.modify` later activates them without further code changes.
    ///
    /// The frontend runs the OAuth consent and sends only the authorization
    /// `code` (+ the `redirect_uri` used at consent). The backend exchanges it
    /// server-side with the app's config-held client credentials
    /// ([`GoogleOAuth`]) into a refresh token, so the browser never handles the
    /// client secret.
    Gmail {
        code: String,
        redirect_uri: String,
        /// Optional per-label index ceiling (newest-N); omitted = mirror the
        /// whole mailbox. A performance knob, safe to accept from the client.
        #[serde(default)]
        index_cap: Option<usize>,
    },
    /// Google Drive via Google OAuth (read-only mount).
    ///
    /// Scope: request `https://www.googleapis.com/auth/drive.readonly` at
    /// consent (least-privilege for a read-only mount), with
    /// `access_type=offline` + `prompt=consent` so Google returns a refresh
    /// token. Like Gmail, the frontend runs the OAuth consent and sends only
    /// the authorization `code` (+ the `redirect_uri` used at consent); the
    /// backend exchanges it server-side with the app's [`GoogleOAuth`] client
    /// credentials, so the browser never handles the client secret.
    Gdrive {
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
            ProviderSpec::Gmail {
                code,
                redirect_uri,
                index_cap,
            } => {
                let (client_id, client_secret) = oauth.credentials().ok_or_else(|| {
                    err(
                        StatusCode::BAD_REQUEST,
                        "Gmail is not configured on this server \
                         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)",
                    )
                })?;
                let exchanged = ::workspace::exchange_gmail_code(
                    client_id,
                    client_secret,
                    &code,
                    &redirect_uri,
                    oauth.base_url.as_deref(),
                )
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("gmail oauth: {e}")))?;
                ProviderConfig::Gmail(GmailConfig {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                    refresh_token: exchanged.refresh_token,
                    account_email: exchanged.account_email,
                    index_cap,
                    // Deployment-level override (mock/gateway), inherited from
                    // backend config — never from the request.
                    base_url: oauth.base_url.clone(),
                })
            }
            ProviderSpec::Gdrive { code, redirect_uri } => {
                let (client_id, client_secret) = oauth.credentials().ok_or_else(|| {
                    err(
                        StatusCode::BAD_REQUEST,
                        "Google Drive is not configured on this server \
                         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)",
                    )
                })?;
                let exchanged = ::workspace::exchange_gdrive_code(
                    client_id,
                    client_secret,
                    &code,
                    &redirect_uri,
                    oauth.base_url.as_deref(),
                )
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, format!("gdrive oauth: {e}")))?;
                ProviderConfig::Gdrive(GdriveConfig {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                    refresh_token: exchanged.refresh_token,
                    account_email: exchanged.account_email,
                    // Deployment-level override (mock/gateway), inherited from
                    // backend config — never from the request.
                    base_url: oauth.base_url.clone(),
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
    /// Gmail: the OAuth pieces are secret, but the account email (resolved at
    /// mount-create) is shown so the UI can tell mounts apart.
    Gmail { email: String },
    /// Google Drive: the OAuth pieces are secret, but the account email
    /// (resolved at mount-create) is shown so the UI can tell mounts apart.
    Gdrive { email: String },
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
            ProviderConfig::Gmail(c) => ProviderInfo::Gmail {
                email: c.account_email.clone(),
            },
            ProviderConfig::Gdrive(c) => ProviderInfo::Gdrive {
                email: c.account_email.clone(),
            },
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
    if let ProviderConfig::Gmail(c) = &created.provider {
        // Kick the initial full mailbox sync in the background; the response
        // doesn't wait for it. Later freshness is frontend-driven via
        // `POST …/mounts/{id}/sync`; progress via `GET …/mounts/{id}/sync`.
        state.workspaces.spawn_gmail_sync(c);
    }
    Ok((StatusCode::CREATED, Json(MountResponse::from(created))))
}

/// Sync status of a Gmail mount's on-disk mailbox mirror.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MountSyncResponse {
    /// A sync run is active right now.
    pub running: bool,
    /// This request started a new run (`POST` only; `false` when one was
    /// already in flight — its result is equivalent, just poll `running`).
    pub started: bool,
    /// Messages in the mailbox at the last full listing (progress denominator).
    pub total: usize,
    /// Messages mirrored so far.
    pub fetched: usize,
    /// The initial full sync has finished (the mirror serves complete data;
    /// later runs only fold in changes).
    pub completed: bool,
}

/// Look up `mount_id` in `wid` and return its Gmail config, or the right error
/// (foreign/missing mount → 404 like everywhere else; non-Gmail → 400 since
/// only Gmail mounts have a mirror to sync).
async fn gmail_mount_config(
    state: &AppState,
    wid: Uuid,
    mount_id: Uuid,
) -> Result<GmailConfig, ApiError> {
    let mount = match state.workspaces.get_mount(mount_id).await? {
        Some(m) if m.workspace_id == wid => m,
        _ => return Err(err(StatusCode::NOT_FOUND, "mount not found")),
    };
    match mount.provider {
        ProviderConfig::Gmail(c) => Ok(c),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "mount provider does not support sync",
        )),
    }
}

/// `GET /workspaces/{wid}/mounts/{mount_id}/sync` — sync progress of a Gmail
/// mount's mirror (for the initial-sync progress display).
pub(super) async fn get_mount_sync(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((wid, mount_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<MountSyncResponse>, ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let cfg = gmail_mount_config(&state, wid, mount_id).await?;
    let (sync, running) = state.workspaces.gmail_sync_status(&cfg);
    let sync = sync.unwrap_or_default();
    Ok(Json(MountSyncResponse {
        running,
        started: false,
        total: sync.total,
        fetched: sync.fetched,
        completed: sync.completed,
    }))
}

/// `POST /workspaces/{wid}/mounts/{mount_id}/sync` — refresh the mount's
/// mirror in the background: journal replay when the mirror is current, a
/// (resumed) full sync when it isn't. Idempotent under concurrency: if a run
/// is already active the call just reports it (`started: false`).
pub(super) async fn trigger_mount_sync(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthUser>,
    Path((wid, mount_id)): Path<(Uuid, Uuid)>,
) -> Result<(StatusCode, Json<MountSyncResponse>), ApiError> {
    require_owned_workspace(&state, &auth, wid).await?;
    let cfg = gmail_mount_config(&state, wid, mount_id).await?;
    let started = state.workspaces.spawn_gmail_sync(&cfg);
    let (sync, running) = state.workspaces.gmail_sync_status(&cfg);
    let sync = sync.unwrap_or_default();
    Ok((
        StatusCode::ACCEPTED,
        Json(MountSyncResponse {
            running,
            started,
            total: sync.total,
            fetched: sync.fetched,
            completed: sync.completed,
        }),
    ))
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
