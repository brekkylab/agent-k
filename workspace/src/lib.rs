//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion / Gmail / GDrive) plus the unified [`WorkspaceFs`], which presents
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
    FwdStat, GdriveConfig, GdriveExchange, GdriveResource, GmailConfig, GmailExchange,
    GmailResource, GmailSyncDelta, GmailSyncState, LOCAL_MOUNT, LocalResource, Mount, MountPath,
    MountSpec, NotionConfig, NotionResource, ProviderConfig, Resource, ResourceError,
    ResourceResult, S3Config, S3Resource, account_mirror_dir, exchange_gdrive_code,
    exchange_gmail_code, mirror_tree, sync_gmail_incremental, sync_gmail_mirror,
};
