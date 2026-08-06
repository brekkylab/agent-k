//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion / Gmail) plus the unified [`WorkspaceFs`], which presents
//! a workspace's local files and its provider mounts as one tree, exposed to the
//! backend as a [`ForwardFs`] to serve to an in-guest FUSE client.
//!
//! Framework-free: no database, HTTP framework, or sandbox VM. The backend wraps
//! this with persistence (mount config), a WebDAV adapter, and sandbox-session
//! wiring, and injects a [`KnowledgeHook`] for `knowledge/` side-processing.

mod fs;
mod hook;
mod vfs;

pub use fs::{
    DirEntry, DirStream, File, FsError, FsResult, NodeKind, OpenOptions, Stat, WorkspaceFs,
};
pub use hook::{FsEvent, FsHook};
pub use vfs::{
    DirEntry as ResourceDirEntry, FileKind, FileStat, ForwardFs, FsConfig, FwdEntry, FwdStat,
    GmailConfig, GmailExchange, GmailResource, GmailSyncDelta, GmailSyncState, LOCAL_MOUNT,
    LocalResource, Mount, MountPath, MountSpec, NotionConfig, NotionResource, Origins,
    ProviderConfig, Resource, ResourceError, ResourceResult, S3Config, S3Resource, SlackConfig,
    SlackExchange, SlackResource, TunnelServer, account_mirror_dir, exchange_gmail_code,
    exchange_slack_code, mirror_tree, sync_gmail_incremental, sync_gmail_mirror,
};
