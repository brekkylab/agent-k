//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion) plus the unified [`WorkspaceFs`], which presents
//! a workspace's local files and its provider mounts as one tree, and a host-side
//! forward server ([`VfsForward`]) for serving that tree to an in-guest FUSE
//! forwarder.
//!
//! Framework-free: no database, HTTP framework, or sandbox VM. The backend wraps
//! this with persistence (mount config), a WebDAV adapter, and sandbox-session
//! wiring, and injects a [`KnowledgeHook`] for `knowledge/` side-processing.

mod fs;
mod hook;
mod vfs;

pub use fs::{
    DirEntry, DirStream, File, FsError, FsResult, NodeKind, OpenOptions, ReadDirMeta, Stat,
    WorkspaceFs,
};
pub use hook::{FsEvent, FsHook};
pub use vfs::{
    DirEntry as VfsDirEntry, FileKind, FileStat, ForwardFs, FwdEntry, FwdStat, Mount, MountSpec,
    NotionConfig, NotionResource, ProviderConfig, Resource, S3Config, S3Resource, VPath, Vfs,
    VfsConfig, VfsError,
    VfsForward, VfsResult,
};
