//! Change hook for a workspace's local file tree.

/// A mutation to a workspace's local files. Paths are workspace-relative
/// (leading `/`, e.g. `/knowledge/a.txt`).
pub enum FsEvent<'a> {
    /// A new file appeared at a previously-absent path.
    Created(&'a str),
    /// An existing file was overwritten in place.
    Modified(&'a str),
    /// A file or directory was removed.
    Removed(&'a str),
}

/// Observes mutations to a workspace's local files so the host can react —
/// ingestion/indexing, audit logging, cache invalidation, etc. The
/// [`WorkspaceFs`](crate::WorkspaceFs) calls this after the on-disk change
/// lands. Attach with [`WorkspaceFs::with_hook`](crate::WorkspaceFs::with_hook);
/// when unset the workspace fires nothing. Any classification (e.g. "is this
/// under `knowledge/`?") lives in the implementation, not the crate — as does
/// any identity the host needs (e.g. which workspace), captured by the impl.
pub trait FsHook: Send + Sync {
    fn on_change(&self, event: FsEvent<'_>);
}
