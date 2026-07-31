//! Standalone workspace filesystem.
//!
//! A provider VFS (S3 / Notion) plus the unified [`WorkspaceFs`], which presents
//! a workspace's local files and its provider mounts as one tree.
//!
//! Framework-free: no database, HTTP framework, or sandbox VM. The backend wraps
//! this with persistence (mount config) and a WebDAV adapter, and injects a
//! [`KnowledgeHook`] for `knowledge/` side-processing. The workspace tree is
//! served into agent sandboxes over cortex virtio-fs, not from here.

mod fs;
mod hook;
mod vfs;

pub use fs::{
    DirEntry, DirStream, File, FsError, FsResult, NodeKind, OpenOptions, Stat, WorkspaceFs,
};
pub use hook::{FsEvent, FsHook};
pub use vfs::{
    DirEntry as ResourceDirEntry, FileKind, FileStat, FsConfig, LOCAL_MOUNT, LocalResource, Mount,
    MountPath, MountSpec, NotionConfig, NotionResource, ProviderConfig, Resource, ResourceError,
    ResourceResult, S3Config, S3Resource,
};
