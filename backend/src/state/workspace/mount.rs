//! Persistence for a workspace's external-provider mounts.
//!
//! Each [`WorkspaceMount`] binds a virtual top-level prefix (e.g. `/s3-prod`)
//! to a [`ProviderConfig`] carrying that mount's credentials. Rows are loaded
//! and assembled into a [`WorkspaceFs`] by [`WorkspacesState::build_fs`], which
//! [`WorkspacesState::get_fs`](super::WorkspacesState::get_fs) injects into the
//! per-workspace filesystem so mount prefixes route to the provider.

use ::workspace::{
    FsConfig, GdriveConfig, GmailConfig, LOCAL_MOUNT, MountSpec, NotionConfig, ProviderConfig,
    S3Config, WorkspaceFs,
};
use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::WorkspacesState;
use crate::state::{StateError, StateResult, parse_ts, parse_uuid};

/// A configured external-provider mount for a workspace.
#[derive(Clone)]
pub struct WorkspaceMount {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Virtual top-level prefix, absolute and single-segment (e.g. `/s3-prod`).
    pub prefix: String,
    /// Provider kind plus its credentials.
    pub provider: ProviderConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceMount {
    pub fn new(workspace_id: Uuid, prefix: String, provider: ProviderConfig) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            prefix,
            provider,
            created_at: now,
            updated_at: now,
        }
    }

    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        let provider_kind: String = row.get("provider");
        let config_json: String = row.get("config");
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "workspace_mounts.id")?,
            workspace_id: parse_uuid(
                row.get::<String, _>("workspace_id"),
                "workspace_mounts.workspace_id",
            )?,
            prefix: row.get("prefix"),
            provider: decode_provider(&provider_kind, &config_json)?,
            created_at: parse_ts(
                &row.get::<String, _>("created_at"),
                "workspace_mounts.created_at",
            )?,
            updated_at: parse_ts(
                &row.get::<String, _>("updated_at"),
                "workspace_mounts.updated_at",
            )?,
        })
    }

    /// `(provider discriminator, config JSON)` for persistence.
    fn encode(&self) -> StateResult<(&'static str, String)> {
        Ok(match &self.provider {
            ProviderConfig::S3(c) => ("s3", serde_json::to_string(c)?),
            ProviderConfig::Notion(c) => ("notion", serde_json::to_string(c)?),
            ProviderConfig::Gmail(c) => ("gmail", serde_json::to_string(c)?),
            ProviderConfig::Gdrive(c) => ("gdrive", serde_json::to_string(c)?),
        })
    }
}

/// Rebuild a [`ProviderConfig`] from its stored discriminator + config JSON.
fn decode_provider(kind: &str, config_json: &str) -> StateResult<ProviderConfig> {
    match kind {
        "s3" => Ok(ProviderConfig::S3(serde_json::from_str::<S3Config>(
            config_json,
        )?)),
        "notion" => Ok(ProviderConfig::Notion(
            serde_json::from_str::<NotionConfig>(config_json)?,
        )),
        "gmail" => Ok(ProviderConfig::Gmail(serde_json::from_str::<GmailConfig>(
            config_json,
        )?)),
        "gdrive" => Ok(ProviderConfig::Gdrive(
            serde_json::from_str::<GdriveConfig>(config_json)?,
        )),
        other => Err(StateError::InvalidData(format!(
            "workspace_mounts.provider: unknown provider {other}"
        ))),
    }
}

/// Normalise + validate a mount prefix: absolute, exactly one non-empty path
/// segment (no nested slashes), so it maps to a single virtual top-level dir.
fn normalize_prefix(prefix: &str) -> StateResult<String> {
    let trimmed = prefix.trim().trim_matches('/');
    if trimmed.is_empty() || trimmed.contains('/') {
        return Err(StateError::InvalidData(format!(
            "mount prefix must be a single top-level segment, got {prefix:?}"
        )));
    }
    if trimmed == LOCAL_MOUNT.trim_start_matches('/') {
        return Err(StateError::InvalidData(format!(
            "mount prefix {trimmed:?} is reserved for the workspace's local files"
        )));
    }
    Ok(format!("/{trimmed}"))
}

impl WorkspacesState {
    /// Every mount configured for `workspace_id`, ordered by prefix.
    pub async fn list_mounts(&self, workspace_id: Uuid) -> StateResult<Vec<WorkspaceMount>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, prefix, provider, config, created_at, updated_at \
             FROM workspace_mounts WHERE workspace_id = ? ORDER BY prefix ASC",
        )
        .bind(workspace_id.to_string())
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(WorkspaceMount::from_row).collect()
    }

    /// A single mount by id, or `None`.
    pub async fn get_mount(&self, id: Uuid) -> StateResult<Option<WorkspaceMount>> {
        let row = sqlx::query(
            "SELECT id, workspace_id, prefix, provider, config, created_at, updated_at \
             FROM workspace_mounts WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.as_ref().map(WorkspaceMount::from_row).transpose()
    }

    /// Insert a new mount. The prefix is normalised (absolute, single segment);
    /// a prefix already used in the same workspace surfaces as
    /// [`StateError::UniqueViolation`].
    pub async fn create_mount(&self, mut mount: WorkspaceMount) -> StateResult<WorkspaceMount> {
        mount.prefix = normalize_prefix(&mount.prefix)?;
        let (provider, config) = mount.encode()?;
        sqlx::query(
            "INSERT INTO workspace_mounts \
                 (id, workspace_id, prefix, provider, config, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(mount.id.to_string())
        .bind(mount.workspace_id.to_string())
        .bind(&mount.prefix)
        .bind(provider)
        .bind(config)
        .bind(mount.created_at.to_rfc3339())
        .bind(mount.updated_at.to_rfc3339())
        .execute(&self.db)
        .await
        .map_err(map_mount_sqlx_error)?;
        // Mounts changed → rebuild the fs on next access.
        self.invalidate_fs(mount.workspace_id);
        Ok(mount)
    }

    /// Delete a mount by id, returning the removed row. Provider-side cleanup
    /// runs after the row is gone (e.g. Gmail's mirror GC — see
    /// [`WorkspacesState::on_mount_removed`]).
    pub async fn remove_mount(&self, id: Uuid) -> StateResult<WorkspaceMount> {
        let existing = self.get_mount(id).await?.ok_or(StateError::NotFound)?;
        sqlx::query("DELETE FROM workspace_mounts WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        self.invalidate_fs(existing.workspace_id);
        self.on_mount_removed(&existing.provider).await;
        Ok(existing)
    }

    /// Build the workspace's filesystem: the local `/files` mount plus any
    /// configured provider mounts.
    pub(super) async fn build_fs(&self, workspace_id: Uuid) -> StateResult<WorkspaceFs> {
        let mut config =
            build_workspace_vfs(&self.db, workspace_id, self.google_oauth.clone()).await?;
        config.local_root = Some(self.get_root(workspace_id));
        config.mirror_root = Some(self.mirror_root());
        WorkspaceFs::from_config(config)
            .map_err(|e| StateError::InvalidData(format!("workspace fs: {e}")))
    }
}

/// Load a workspace's provider mounts into an [`FsConfig`], with `local_root`
/// left unset — the caller fills it, since only it knows the on-disk file root.
///
/// `google_oauth` is the deployment's OAuth client, required by a Gmail or Drive
/// mount and taken as an argument rather than read from a row: it belongs to the
/// installation, and a copy stored beside each mount's refresh token would make that
/// row usable on its own. A caller with none passes `None`, and a Google mount then
/// fails to build instead of serving broken.
/// Standalone (takes the pool) so both [`WorkspacesState::build_fs`] and the
/// session run loop (which only holds the pool) can build it.
pub(crate) async fn build_workspace_vfs(
    db: &SqlitePool,
    workspace_id: Uuid,
    google_oauth: Option<::workspace::GoogleClient>,
) -> StateResult<FsConfig> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, prefix, provider, config, created_at, updated_at \
         FROM workspace_mounts WHERE workspace_id = ? ORDER BY prefix ASC",
    )
    .bind(workspace_id.to_string())
    .fetch_all(db)
    .await?;
    let mounts = rows
        .iter()
        .map(WorkspaceMount::from_row)
        .collect::<StateResult<Vec<_>>>()?;
    Ok(FsConfig {
        local_root: None,
        mirror_root: None,
        google_oauth,
        mounts: mounts
            .into_iter()
            .map(|m| MountSpec {
                prefix: m.prefix,
                provider: m.provider,
            })
            .collect(),
    })
}

/// Map a SQLite UNIQUE violation on `(workspace_id, prefix)` to a typed error so
/// the router can answer `409 Conflict`. Everything else passes through.
fn map_mount_sqlx_error(e: sqlx::Error) -> StateError {
    if let sqlx::Error::Database(ref db_err) = e
        && (db_err
            .code()
            .map(|c| c == "2067" || c == "1555")
            .unwrap_or(false)
            || db_err.message().contains("UNIQUE"))
    {
        return StateError::UniqueViolation("workspace_mounts.prefix".to_string());
    }
    StateError::Sqlx(e)
}

#[cfg(test)]
mod tests {
    use ::workspace::S3Config;

    use super::*;

    /// Fresh in-memory DB with a seeded user + workspace; returns the state and
    /// the workspace id.
    /// A deployment OAuth client, as `AppState` would supply it. No network is
    /// reached in these tests; what matters is that one exists to be injected.
    fn test_oauth() -> ::workspace::GoogleClient {
        ::workspace::GoogleClient {
            client_id: "deployment-client".into(),
            client_secret: "deployment-secret".into(),
            origins: Default::default(),
        }
    }

    async fn fresh_state() -> (WorkspacesState, tempfile::TempDir, Uuid) {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let uid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users \
                 (id, username, password_hash, role, is_active, preferred_language, created_at, updated_at) \
             VALUES (?, ?, 'x', 'user', 1, 'en', ?, ?)",
        )
        .bind(uid.to_string())
        .bind(format!("u-{uid}"))
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, user_id, title, created_at, updated_at) \
             VALUES (?, ?, 'W', ?, ?)",
        )
        .bind(uid.to_string())
        .bind(uid.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf(), Some(test_oauth()));
        (state, tmp, uid)
    }

    fn s3_provider() -> ProviderConfig {
        ProviderConfig::S3(S3Config {
            bucket: "b".into(),
            region: "us-east-1".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            endpoint: None,
            key_prefix: None,
        })
    }

    #[tokio::test]
    async fn mount_crud_prefix_and_vfs() {
        let (state, _tmp, wid) = fresh_state().await;

        // A prefix without a leading slash is normalised on insert.
        let created = state
            .create_mount(WorkspaceMount::new(wid, "s3-prod".into(), s3_provider()))
            .await
            .unwrap();
        assert_eq!(created.prefix, "/s3-prod");

        let listed = state.list_mounts(wid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].prefix, "/s3-prod");

        // The config round-trips through the DB (credentials preserved).
        match &listed[0].provider {
            ProviderConfig::S3(c) => {
                assert_eq!(c.bucket, "b");
                assert_eq!(c.access_key_id, "k");
            }
            _ => panic!("expected S3"),
        }

        // build_fs assembles the mount (no network — just client construction),
        // alongside the always-present local `/files` mount.
        let vfs = state.build_fs(wid).await.unwrap();
        let mut names = vfs.mount_names();
        names.sort();
        assert_eq!(names, vec!["files".to_string(), "s3-prod".to_string()]);

        // get_fs builds a filesystem with the mount attached.
        assert!(state.get_fs(wid).await.is_ok());

        // A duplicate prefix conflicts.
        let dup = WorkspaceMount::new(wid, "/s3-prod".into(), s3_provider());
        assert!(matches!(
            state.create_mount(dup).await,
            Err(StateError::UniqueViolation(_))
        ));

        // Removal drops it; with no provider mounts, the VFS still has local `/files`.
        state.remove_mount(created.id).await.unwrap();
        assert!(state.list_mounts(wid).await.unwrap().is_empty());
        assert_eq!(
            state.build_fs(wid).await.unwrap().mount_names(),
            vec!["files".to_string()]
        );
        assert!(matches!(
            state.remove_mount(created.id).await,
            Err(StateError::NotFound)
        ));
    }

    /// Removing a Gmail mount GCs the account's on-disk mirror — but only when
    /// no other mount (case-insensitively) references the same account.
    #[tokio::test]
    async fn gmail_mirror_gc_on_last_reference() {
        let (state, _tmp, wid) = fresh_state().await;
        let gmail = |email: &str| {
            ProviderConfig::Gmail(GmailConfig {
                refresh_token: "r".into(),
                account_email: email.into(),
                index_cap: None,
            })
        };
        // Two mounts referencing one account — the email case differs, but
        // both map to the same (sanitized, lowercased) mirror dir.
        let m1 = state
            .create_mount(WorkspaceMount::new(
                wid,
                "g1".into(),
                gmail("Sync.Test@x.com"),
            ))
            .await
            .unwrap();
        let m2 = state
            .create_mount(WorkspaceMount::new(
                wid,
                "g2".into(),
                gmail("sync.test@x.com"),
            ))
            .await
            .unwrap();

        // A fabricated mirror as the sync would leave it.
        let acct = ::workspace::account_mirror_dir(&state.mirror_root(), "sync.test@x.com");
        std::fs::create_dir_all(acct.join("tree/INBOX")).unwrap();
        std::fs::write(acct.join("state.json"), b"{}").unwrap();

        // First removal: the account is still referenced — mirror stays.
        state.remove_mount(m1.id).await.unwrap();
        assert!(acct.exists(), "mirror kept while a reference remains");

        // Last removal: mirror dir is deleted wholesale.
        state.remove_mount(m2.id).await.unwrap();
        assert!(!acct.exists(), "mirror removed with its last mount");
    }

    /// A gdrive mount round-trips through encode/decode with its credentials intact,
    /// and builds into the VFS.
    #[tokio::test]
    async fn gdrive_mount_round_trips_and_builds() {
        let (state, _tmp, wid) = fresh_state().await;
        let provider = ProviderConfig::Gdrive(GdriveConfig {
            refresh_token: "rt".into(),
        });
        state
            .create_mount(WorkspaceMount::new(wid, "gdrive".into(), provider))
            .await
            .unwrap();

        let listed = state.list_mounts(wid).await.unwrap();
        match &listed[0].provider {
            ProviderConfig::Gdrive(c) => assert_eq!(c.refresh_token, "rt"),
            _ => panic!("expected Gdrive"),
        }

        // What actually reaches the row. The deployment's client secret must not be
        // among it: a stored copy beside the refresh token would make one leaked row
        // a working Google credential, and it is injected at build time instead.
        let stored: String = sqlx::query_scalar("SELECT config FROM workspace_mounts")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert!(stored.contains("rt"), "the mount's own half is stored");
        assert!(
            !stored.contains("deployment-secret") && !stored.contains("deployment-client"),
            "no deployment credential in the row: {stored}"
        );

        // Assembles alongside `/files` (no network — client construction only).
        let vfs = state.build_fs(wid).await.unwrap();
        let mut names = vfs.mount_names();
        names.sort();
        assert_eq!(names, vec!["files".to_string(), "gdrive".to_string()]);
    }

    #[tokio::test]
    async fn nested_prefix_rejected() {
        let (state, _tmp, wid) = fresh_state().await;
        let nested = WorkspaceMount::new(wid, "/a/b".into(), s3_provider());
        assert!(matches!(
            state.create_mount(nested).await,
            Err(StateError::InvalidData(_))
        ));
    }

    #[tokio::test]
    async fn reserved_files_prefix_rejected() {
        let (state, _tmp, wid) = fresh_state().await;
        // `files` is reserved for the workspace's local file mount.
        let reserved = WorkspaceMount::new(wid, "files".into(), s3_provider());
        assert!(matches!(
            state.create_mount(reserved).await,
            Err(StateError::InvalidData(_))
        ));
    }

    /// A mount create/remove must evict the cached `WorkspaceFs` so the next
    /// `get_fs` reflects the change (a stale cache would keep listing a removed
    /// mount). Checked via the root mount-name list (no network).
    #[tokio::test]
    async fn get_fs_reflects_mount_changes() {
        let (state, _tmp, wid) = fresh_state().await;

        let created = state
            .create_mount(WorkspaceMount::new(wid, "s3-prod".into(), s3_provider()))
            .await
            .unwrap();
        let fs = state.get_fs(wid).await.unwrap();
        assert!(
            fs.mount_names().iter().any(|n| n == "s3-prod"),
            "mount should appear after create"
        );

        // Removing must evict the cache, or get_fs still lists it.
        state.remove_mount(created.id).await.unwrap();
        let fs = state.get_fs(wid).await.unwrap();
        assert!(
            !fs.mount_names().iter().any(|n| n == "s3-prod"),
            "mount must be gone after remove (cache invalidated)"
        );
    }
}
