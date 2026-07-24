//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion / Gmail) plus the unified [`WorkspaceFs`], which presents
//! a workspace's local files and its provider mounts as one tree, and a host-side
//! forward server ([`ForwardServer`]) for serving that tree to an in-guest FUSE
//! forwarder.
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
    DirEntry as ResourceDirEntry, FileKind, FileStat, ForwardFs, ForwardServer, FsConfig, FwdEntry,
    FwdStat, GmailConfig, GmailExchange, GmailResource, GmailSyncState, LOCAL_MOUNT,
    LocalResource, Mount, MountPath, MountSpec, NotionConfig, NotionResource, ProviderConfig,
    Resource, ResourceError, ResourceResult, S3Config, S3Resource, exchange_gmail_code,
    mirror_tree, sync_gmail_mirror,
};
