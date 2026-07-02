use std::path::PathBuf;

use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, User, parse_ts, parse_uuid};

mod fs;

pub use fs::*;

/// A workspace: both a database row and a directory tree on disk.
///
/// A workspace is the top-level container holding a file store (exposed over
/// WebDAV, see [`crate::router`]) plus the agents and sessions scoped to it. A
/// user's default workspace shares that user's id.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Construct with an explicit id. A user's default workspace uses that
    /// user's id.
    pub fn with_id(id: Uuid, title: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            title,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_updated_at(mut self) -> Self {
        self.updated_at = Utc::now();
        self
    }

    fn from_sqlite_row(row: &SqliteRow) -> StateResult<Self> {
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "workspaces.id")?,
            title: row.get("title"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "workspaces.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "workspaces.updated_at")?,
        })
    }
}

/// Workspace persistence plus the per-workspace filesystem.
///
/// Holds the SQLite pool (for the [`Workspace`] rows) and the `data_root` (for
/// the on-disk file trees). Hands out a [`WorkspaceFs`] per workspace via
/// [`Self::get_fs`]; that handle is the single entry point for *all* filesystem
/// operations on a workspace and performs the workspace's side-processing
/// (currently `knowledge/` ingestion) itself. The WebDAV layer wraps a
/// [`WorkspaceFs`] rather than touching the disk directly.
pub struct WorkspacesState {
    db: SqlitePool,
    data_root: PathBuf,
}

impl WorkspacesState {
    pub fn new(db: SqlitePool, data_root: PathBuf) -> Self {
        Self { db, data_root }
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<Workspace>> {
        let row = sqlx::query("SELECT id, title, created_at, updated_at FROM workspaces WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(Workspace::from_sqlite_row).transpose()
    }

    /// Fetch `wid` only if `user_id` may access it. Access is currently limited
    /// to a user's default workspace, whose id equals the user's id — this is
    /// the single definition of the workspace access rule, reused by every
    /// caller (HTTP routes, WebDAV, message WS). A workspace the user cannot
    /// access is indistinguishable from a missing one (`None`), so existence
    /// can't be probed.
    pub async fn get_for_user(&self, user_id: Uuid, wid: Uuid) -> StateResult<Option<Workspace>> {
        if wid != user_id {
            return Ok(None);
        }
        self.get(wid).await
    }

    /// Insert or update by `id`. Returns the prior row if one was overwritten,
    /// `None` if freshly inserted. Also provisions the workspace's on-disk file
    /// root; `create_dir_all` is idempotent, so a re-upsert is a no-op there.
    pub async fn upsert(&self, item: Workspace) -> StateResult<Option<Workspace>> {
        let id = item.id;
        let prior = self.get(id).await?;
        sqlx::query(
            "INSERT INTO workspaces (id, title, created_at, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 title = excluded.title, \
                 updated_at = excluded.updated_at",
        )
        .bind(item.id.to_string())
        .bind(&item.title)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        // Provision the file root and mirror the title to `.title` on disk, so
        // the workspace is fully described without a DB read.
        tokio::fs::create_dir_all(self.get_root(id)).await?;
        tokio::fs::write(self.title_path(id), &item.title).await?;
        Ok(prior)
    }

    /// Delete the on-disk files first, then the row. Cascades (agents, sessions)
    /// are handled by the database's foreign keys.
    pub async fn remove(&self, id: Uuid) -> StateResult<Workspace> {
        let existing = self.get(id).await?.ok_or(StateError::NotFound)?;
        self.remove_files(id).await?;
        sqlx::query("DELETE FROM workspaces WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(existing)
    }

    /// Remove a workspace's on-disk artifacts — its directory tree and the
    /// `users/{id}/workspace` convenience symlink — without touching the
    /// database. Idempotent. Deleting files before the rows means a filesystem
    /// failure aborts before anything is removed from the database.
    pub async fn remove_files(&self, id: Uuid) -> StateResult<()> {
        let dir = self.workspace_dir(id);
        if tokio::fs::try_exists(&dir).await? {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        // A default workspace's id equals its user's id, so its convenience
        // symlink lives at `users/{id}/workspace`; drop it if present.
        let link = self.user_default_link(id);
        if tokio::fs::symlink_metadata(&link).await.is_ok() {
            tokio::fs::remove_file(&link).await.ok();
        }
        Ok(())
    }

    /// Provision a user's default workspace. Its id mirrors the user's id, its
    /// title is derived from `username`, and a convenience symlink
    /// `users/{uid}/workspace` → the workspace directory is created so the file
    /// tree is reachable by a user-centric path too.
    pub async fn create_default(&self, user: &User) -> StateResult<Workspace> {
        let ws = Workspace::with_id(user.id, format!("{}'s workspace", user.username));
        self.upsert(ws.clone()).await?;
        self.link_user_default(user.id).await?;
        Ok(ws)
    }

    /// A filesystem handle scoped to workspace `wid`'s file root.
    pub fn get_fs(&self, wid: Uuid) -> WorkspaceFs {
        WorkspaceFs::new(self.get_root(wid), wid)
    }

    /// Absolute on-disk path of workspace `wid`'s file root
    /// (`data_root/workspaces/{wid}/files`).
    fn get_root(&self, wid: Uuid) -> PathBuf {
        self.workspace_dir(wid).join("files")
    }

    /// The per-workspace directory (`data_root/workspaces/{wid}`), holding the
    /// file root and room for sibling metadata.
    fn workspace_dir(&self, wid: Uuid) -> PathBuf {
        self.data_root.join("workspaces").join(wid.to_string())
    }

    /// Path of the on-disk title mirror (`data_root/workspaces/{wid}/.title`).
    /// It sits beside — not inside — the `files` root, so it is never exposed
    /// through WebDAV.
    fn title_path(&self, wid: Uuid) -> PathBuf {
        self.workspace_dir(wid).join(".title")
    }

    /// Path of the per-user convenience symlink
    /// (`data_root/users/{uid}/workspace`).
    fn user_default_link(&self, user_id: Uuid) -> PathBuf {
        self.data_root
            .join("users")
            .join(user_id.to_string())
            .join("workspace")
    }

    /// (Re)create the `users/{uid}/workspace` → `workspaces/{uid}` symlink. The
    /// default workspace's id equals the user's id, so the target is
    /// `workspaces/{uid}`. The link is *relative* so the whole `data_root` can
    /// be relocated without breaking it.
    async fn link_user_default(&self, user_id: Uuid) -> StateResult<()> {
        let link = self.user_default_link(user_id);
        if let Some(parent) = link.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Idempotent: replace any existing link so re-provisioning is safe.
        if tokio::fs::symlink_metadata(&link).await.is_ok() {
            tokio::fs::remove_file(&link).await.ok();
        }
        // From `users/{uid}/` the workspace dir is `../../workspaces/{uid}`.
        let target = PathBuf::from("..")
            .join("..")
            .join("workspaces")
            .join(user_id.to_string());
        #[cfg(unix)]
        tokio::fs::symlink(&target, &link).await?;
        #[cfg(not(unix))]
        {
            let _ = target;
            tracing::warn!("workspace symlink is not supported on this platform");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn user(username: &str) -> User {
        let now = Utc::now();
        User {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash: "x".into(),
            role: Role::User,
            display_name: None,
            is_active: true,
            preferred_language: "en".into(),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn workspace_crud_round_trip() {
        let pool = fresh_db().await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let ws = Workspace::with_id(Uuid::new_v4(), "Alpha".into());
        let id = ws.id;

        assert!(state.upsert(ws.clone()).await.unwrap().is_none());
        // The file root is provisioned and the title mirrored to `.title`.
        assert!(tokio::fs::try_exists(state.get_root(id)).await.unwrap());
        assert_eq!(
            tokio::fs::read_to_string(state.title_path(id)).await.unwrap(),
            "Alpha"
        );

        let fetched = state.get(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.title, "Alpha");

        let bumped = fetched.clone().with_title("Alpha v2");
        let prior = state.upsert(bumped).await.unwrap().expect("prior row");
        assert_eq!(prior.title, "Alpha");
        assert_eq!(state.get(id).await.unwrap().unwrap().title, "Alpha v2");
        // The `.title` mirror tracks the rename.
        assert_eq!(
            tokio::fs::read_to_string(state.title_path(id)).await.unwrap(),
            "Alpha v2"
        );

        let removed = state.remove(id).await.unwrap();
        assert_eq!(removed.id, id);
        assert!(state.get(id).await.unwrap().is_none());
        assert!(!tokio::fs::try_exists(state.workspace_dir(id)).await.unwrap());
        assert!(matches!(state.remove(id).await, Err(StateError::NotFound)));
    }

    #[tokio::test]
    async fn get_for_user_enforces_default_ownership() {
        let pool = fresh_db().await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let uid = Uuid::new_v4();
        state
            .upsert(Workspace::with_id(uid, "W".into()))
            .await
            .unwrap();

        // Owner (wid == uid) can reach it.
        assert!(state.get_for_user(uid, uid).await.unwrap().is_some());
        // A different user gets None even though the workspace exists — no
        // existence leak.
        assert!(state.get_for_user(Uuid::new_v4(), uid).await.unwrap().is_none());
        // Owner id but a workspace that doesn't exist → None.
        assert!(state.get_for_user(uid, Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_default_mirrors_uid_and_symlinks() {
        let pool = fresh_db().await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let u = user("tester");
        let user_id = u.id;
        let ws = state.create_default(&u).await.unwrap();
        // The default workspace's id equals the user's id, title from username.
        assert_eq!(ws.id, user_id);
        assert_eq!(ws.title, "tester's workspace");

        // The file root lives under workspaces/{uid}/files.
        assert!(tokio::fs::try_exists(state.get_root(user_id)).await.unwrap());

        // users/{uid}/workspace is a symlink that resolves onto the workspace
        // directory, so its `files` child is reachable through the link.
        let link = state.user_default_link(user_id);
        let meta = tokio::fs::symlink_metadata(&link).await.unwrap();
        assert!(meta.file_type().is_symlink());
        assert!(tokio::fs::try_exists(link.join("files")).await.unwrap());

        // Removing the default workspace also drops the dangling symlink.
        state.remove(user_id).await.unwrap();
        assert!(tokio::fs::symlink_metadata(&link).await.is_err());
    }
}
