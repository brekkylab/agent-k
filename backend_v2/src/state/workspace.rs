use std::path::{Path, PathBuf};

use uuid::Uuid;

/// Workspace-level concerns that sit on top of the per-project WebDAV
/// filesystem — currently just knowledge-folder ingestion. It holds the
/// `data_root` so ingestion can resolve a project's files on disk; further
/// dependencies (knowledge store, indexer, …) are wired in as those features
/// land.
pub struct WorkspaceState {
    data_root: PathBuf,
}

impl WorkspaceState {
    pub fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    /// Absolute path of project `pid`'s workspace root on disk
    /// (`data_root/projects/{pid}/workspace`).
    pub fn root(&self, pid: Uuid) -> PathBuf {
        self.data_root
            .join("projects")
            .join(pid.to_string())
            .join("workspace")
    }

    /// Absolute path of `rel_path` (a workspace-relative path such as
    /// `/knowledge/foo.txt`) inside project `pid`'s workspace.
    fn workspace_path(&self, pid: Uuid, rel_path: &str) -> PathBuf {
        self.root(pid).join(rel_path.trim_start_matches('/'))
    }

    /// Handle a *new* file appearing in a project's `knowledge/` directory.
    ///
    /// Invoked from the WebDAV layer (see [`crate::router`]) once a create —
    /// a `PUT`/`COPY`/`MOVE` that lands a file at a path that did not exist
    /// before — succeeds. Stub for now: the real ingestion (parsing, indexing
    /// into the knowledge store) lands later; today it only logs.
    pub fn insert_knowledge(&self, pid: Uuid, rel_path: &str) {
        let path: &Path = &self.workspace_path(pid, rel_path);
        tracing::info!(
            "insert_knowledge (project={pid}, path={})",
            path.display()
        );
    }

    /// Handle an existing `knowledge/` file being overwritten in place.
    ///
    /// Invoked from the WebDAV layer once a `PUT`/`COPY`/`MOVE` onto a path
    /// that already existed succeeds. Stub for now (re-index the changed file
    /// later); today it only logs.
    pub fn update_knowledge(&self, pid: Uuid, rel_path: &str) {
        let path: &Path = &self.workspace_path(pid, rel_path);
        tracing::info!(
            "update_knowledge (project={pid}, path={})",
            path.display()
        );
    }

    /// Handle a file leaving a project's `knowledge/` directory.
    ///
    /// Invoked from the WebDAV layer once a `DELETE` (or a `MOVE` whose source
    /// was under `knowledge/`) succeeds. The file is already gone from disk by
    /// this point — `rel_path` is the path it occupied. Stub for now (drop it
    /// from the knowledge store later); today it only logs.
    pub fn remove_knowledge(&self, pid: Uuid, rel_path: &str) {
        let path: &Path = &self.workspace_path(pid, rel_path);
        tracing::info!(
            "remove_knowledge (project={pid}, path={})",
            path.display()
        );
    }
}
