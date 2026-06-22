use std::path::PathBuf;

use uuid::Uuid;

mod fs;

pub use fs::*;

/// Workspace-level concerns that sit on top of the per-project filesystem.
///
/// Holds the `data_root` so a project's files can be resolved on disk. Hands
/// out a [`WorkspaceFs`] per project via [`Self::fs`]; that handle is the single
/// entry point for *all* filesystem operations on a workspace, and it performs
/// the workspace's side-processing (currently `knowledge/` ingestion) itself.
/// The WebDAV layer (see [`crate::router`]) wraps a [`WorkspaceFs`] rather than
/// touching the disk directly.
pub struct WorkspaceState {
    data_root: PathBuf,
}

impl WorkspaceState {
    pub fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    /// A filesystem handle scoped to project `pid`'s workspace.
    pub fn fs(&self, pid: Uuid) -> WorkspaceFs {
        WorkspaceFs::new(self.root(pid), pid)
    }

    /// Absolute path of project `pid`'s workspace root on disk
    /// (`data_root/projects/{pid}/workspace`).
    fn root(&self, pid: Uuid) -> PathBuf {
        self.data_root
            .join("projects")
            .join(pid.to_string())
            .join("workspace")
    }
}
