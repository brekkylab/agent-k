use std::path::PathBuf;
use std::sync::Arc;

use ::workspace::{FsEvent, FsHook, Vfs, VfsConfig, WorkspaceFs};
use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, User, parse_ts, parse_uuid};

mod mount;

pub use mount::*;

/// A workspace: both a database row and a directory tree on disk.
///
/// A workspace is the top-level container holding a file store (exposed over
/// WebDAV, see [`crate::router`]) plus the agents and sessions scoped to it. A
/// user's default workspace shares that user's id.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    /// Construct with an explicit id and owner. A user's default workspace uses
    /// that user's id for both.
    pub fn with_id(id: Uuid, user_id: Uuid, title: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            user_id,
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
            user_id: parse_uuid(row.get::<String, _>("user_id"), "workspaces.user_id")?,
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
        let row = sqlx::query(
            "SELECT id, user_id, title, created_at, updated_at FROM workspaces WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.as_ref().map(Workspace::from_sqlite_row).transpose()
    }

    /// Fetch `wid` only if it is owned by `user_id`. This is the single
    /// definition of the workspace access rule, reused by every caller (HTTP
    /// routes, WebDAV, message WS). A workspace the user doesn't own is
    /// indistinguishable from a missing one (`None`), so existence can't be
    /// probed.
    pub async fn get_for_user(&self, user_id: Uuid, wid: Uuid) -> StateResult<Option<Workspace>> {
        Ok(self.get(wid).await?.filter(|w| w.user_id == user_id))
    }

    /// Insert or update by `id`. Returns the prior row if one was overwritten,
    /// `None` if freshly inserted. Also provisions the workspace's on-disk file
    /// root; `create_dir_all` is idempotent, so a re-upsert is a no-op there.
    pub async fn upsert(&self, item: Workspace) -> StateResult<Option<Workspace>> {
        let id = item.id;
        let prior = self.get(id).await?;
        sqlx::query(
            "INSERT INTO workspaces (id, user_id, title, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 title = excluded.title, \
                 updated_at = excluded.updated_at",
        )
        .bind(item.id.to_string())
        .bind(item.user_id.to_string())
        .bind(&item.title)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        // Provision the file root. The DB row is the sole source of truth for
        // the workspace's data (title); nothing is mirrored to disk.
        tokio::fs::create_dir_all(self.get_root(id)).await?;
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

    /// Remove a workspace's on-disk directory tree without touching the
    /// database. Idempotent. Deleting files before the rows means a filesystem
    /// failure aborts before anything is removed from the database.
    pub async fn remove_files(&self, id: Uuid) -> StateResult<()> {
        let dir = self.workspace_dir(id);
        if tokio::fs::try_exists(&dir).await? {
            tokio::fs::remove_dir_all(&dir).await?;
        }
        Ok(())
    }

    /// Provision a user's default workspace: its id mirrors the user's id and
    /// its title is derived from `username`.
    pub async fn create_default(&self, user: &User) -> StateResult<Workspace> {
        let ws = Workspace::with_id(user.id, user.id, format!("{}'s workspace", user.username));
        self.upsert(ws.clone()).await?;
        Ok(ws)
    }

    /// Ensure the user's default workspace exists both as a row and on disk.
    /// Creates it when the row is missing, and re-materializes the file root when
    /// only the on-disk tree is gone — e.g. after an account deletion whose
    /// row-delete failed after the files were already removed.
    pub async fn ensure_provisioned(&self, user: &User) -> StateResult<()> {
        match self.get(user.id).await? {
            None => {
                self.create_default(user).await?;
            }
            Some(ws) => {
                if !tokio::fs::try_exists(self.get_root(ws.id)).await? {
                    tokio::fs::create_dir_all(self.get_root(ws.id)).await?;
                }
            }
        }
        Ok(())
    }

    /// A filesystem handle scoped to workspace `wid`'s file root, with the
    /// workspace's external-provider mounts attached (paths under a mount prefix
    /// route to the provider; everything else stays local).
    pub async fn get_fs(&self, wid: Uuid) -> StateResult<WorkspaceFs> {
        // `build_vfs` assembles the full mount table (local `/files` + providers).
        let vfs = self.build_vfs(wid).await?;
        Ok(WorkspaceFs::new(vfs, wid).with_hook(knowledge_hook()))
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
}

/// True when a workspace-relative path lives under `knowledge/`. Classification
/// is the backend's policy, not the `workspace` crate's.
fn is_knowledge(rel: &str) -> bool {
    // Local files live under the `/files` mount, so knowledge files are
    // `files/knowledge/…` in the unified namespace.
    rel.trim_start_matches('/').starts_with("files/knowledge/")
}

/// The change hook attached to every [`WorkspaceFs`] this backend builds: it
/// runs the `knowledge/` side-processing (today just logging; ingestion/indexing
/// lands later) and ignores everything else.
struct KnowledgeHook;

impl FsHook for KnowledgeHook {
    fn on_change(&self, wid: Uuid, event: FsEvent<'_>) {
        match event {
            FsEvent::Created(p) if is_knowledge(p) => {
                tracing::info!("insert_knowledge (workspace={wid}, path={p})");
            }
            FsEvent::Modified(p) if is_knowledge(p) => {
                tracing::info!("update_knowledge (workspace={wid}, path={p})");
            }
            FsEvent::Removed(p) if is_knowledge(p) => {
                tracing::info!("remove_knowledge (workspace={wid}, path={p})");
            }
            _ => {}
        }
    }
}

fn knowledge_hook() -> Option<Arc<dyn FsHook>> {
    Some(Arc::new(KnowledgeHook))
}

/// Build a [`WorkspaceFs`] for `wid` rooted under `data_root`, with `vfs`
/// attached. The filesystem-layer counterpart of
/// [`build_workspace_vfs`](mount::build_workspace_vfs): where that assembles the
/// provider mounts, this wraps them together with the local file root into the
/// unified tree. Standalone (takes `data_root` + a prebuilt `vfs`) so the
/// session run loop — which holds only the pool + data root — can mount the
/// unified workspace into a sandbox guest without a [`WorkspacesState`].
///
/// The `workspaces/{wid}/files` layout mirrors
/// [`WorkspacesState::get_root`]; keep the two in step.
pub(crate) fn workspace_fs(
    data_root: &std::path::Path,
    wid: Uuid,
    mut config: VfsConfig,
) -> StateResult<WorkspaceFs> {
    let root = data_root
        .join("workspaces")
        .join(wid.to_string())
        .join("files");
    config.local_root = Some(root);
    let vfs = Vfs::from_config(config).map_err(|e| StateError::InvalidData(format!("vfs: {e}")))?;
    Ok(WorkspaceFs::new(Arc::new(vfs), wid).with_hook(knowledge_hook()))
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

    // Insert a user row so workspaces (FK workspaces.user_id -> users) can be
    // created for it.
    async fn insert_user(pool: &SqlitePool, u: &User) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users \
                 (id, username, password_hash, role, is_active, preferred_language, created_at, updated_at) \
             VALUES (?, ?, 'x', 'user', 1, 'en', ?, ?)",
        )
        .bind(u.id.to_string())
        .bind(&u.username)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn workspace_crud_round_trip() {
        let pool = fresh_db().await;
        let owner = user("owner");
        insert_user(&pool, &owner).await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let ws = Workspace::with_id(Uuid::new_v4(), owner.id, "Alpha".into());
        let id = ws.id;

        assert!(state.upsert(ws.clone()).await.unwrap().is_none());
        // The file root is provisioned on upsert.
        assert!(tokio::fs::try_exists(state.get_root(id)).await.unwrap());

        let fetched = state.get(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.title, "Alpha");

        let bumped = fetched.clone().with_title("Alpha v2");
        let prior = state.upsert(bumped).await.unwrap().expect("prior row");
        assert_eq!(prior.title, "Alpha");
        assert_eq!(state.get(id).await.unwrap().unwrap().title, "Alpha v2");

        let removed = state.remove(id).await.unwrap();
        assert_eq!(removed.id, id);
        assert!(state.get(id).await.unwrap().is_none());
        assert!(!tokio::fs::try_exists(state.workspace_dir(id)).await.unwrap());
        assert!(matches!(state.remove(id).await, Err(StateError::NotFound)));
    }

    #[tokio::test]
    async fn get_for_user_enforces_ownership() {
        let pool = fresh_db().await;
        let owner = user("owner");
        insert_user(&pool, &owner).await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let uid = owner.id;
        // A default workspace (id == uid) and a non-default one, both owned by uid.
        state.upsert(Workspace::with_id(uid, uid, "W".into())).await.unwrap();
        let other = Uuid::new_v4();
        state.upsert(Workspace::with_id(other, uid, "W2".into())).await.unwrap();

        // The owner reaches both — including the non-default (id != uid).
        assert!(state.get_for_user(uid, uid).await.unwrap().is_some());
        assert!(state.get_for_user(uid, other).await.unwrap().is_some());
        // A different user gets None even though the workspace exists — no leak.
        assert!(state.get_for_user(Uuid::new_v4(), other).await.unwrap().is_none());
        // A workspace that doesn't exist → None.
        assert!(state.get_for_user(uid, Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_default_mirrors_uid() {
        let pool = fresh_db().await;
        let u = user("tester");
        insert_user(&pool, &u).await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        let user_id = u.id;
        let ws = state.create_default(&u).await.unwrap();
        // The default workspace's id and owner are the user's id; title from name.
        assert_eq!(ws.id, user_id);
        assert_eq!(ws.user_id, user_id);
        assert_eq!(ws.title, "tester's workspace");

        // The file root lives under workspaces/{uid}/files.
        assert!(tokio::fs::try_exists(state.get_root(user_id)).await.unwrap());

        // Removal drops the on-disk directory.
        state.remove(user_id).await.unwrap();
        assert!(!tokio::fs::try_exists(state.workspace_dir(user_id)).await.unwrap());
    }

    #[tokio::test]
    async fn ensure_provisioned_rematerializes_missing_files() {
        let pool = fresh_db().await;
        let u = user("healme");
        insert_user(&pool, &u).await;
        let tmp = tempfile::tempdir().unwrap();
        let state = WorkspacesState::new(pool, tmp.path().to_path_buf());

        state.create_default(&u).await.unwrap();
        // Simulate an interrupted deletion: files gone, row still present.
        state.remove_files(u.id).await.unwrap();
        assert!(!tokio::fs::try_exists(state.get_root(u.id)).await.unwrap());
        assert!(state.get(u.id).await.unwrap().is_some());

        // Heal re-materializes the file root even though the row still exists.
        state.ensure_provisioned(&u).await.unwrap();
        assert!(tokio::fs::try_exists(state.get_root(u.id)).await.unwrap());
    }
}
