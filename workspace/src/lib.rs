//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion) plus the unified [`WorkspaceFs`], which presents
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
    LOCAL_MOUNT, LocalResource, Mount, MountPath, MountSpec, NotionConfig, NotionResource,
    ProviderConfig, Resource, ResourceError, ResourceResult, S3Config, S3Resource, TunnelServer,
};
