//! Virtual filesystem over external providers (S3, Notion).
//!
//! Vendored from ailoy's `feat/vfs-provider-mounts` branch (commit 61c4c43),
//! reduced to the **frontend-agnostic core**: the FUSE / sandbox frontends,
//! the per-agent `AgentVfs` handle, and the Google Drive accessor were dropped.
//! agent-k wraps this core in its WebDAV workspace layer (see
//! [`crate::state`] / [`crate::router::webdav`]) instead of a FUSE mount.
//!
//! [`WorkspaceFs`](crate::WorkspaceFs) holds the mounts (each bound to a
//! [`Resource`]) and routes virtual paths to them by longest-prefix match.

// This module is vendored, largely intact, from ailoy's VFS core and exposes a
// fuller provider API (writes, domain commands, extra metadata) than agent-k's
// current read-only mount consumer exercises. Allow the resulting dead-code /
// unused-export noise rather than trimming the vendored surface, so re-syncing
// with upstream stays mechanical.
#![allow(dead_code, unused_imports)]

pub mod accessor;
mod cache;
pub mod error;
pub mod path;
pub mod resource;
pub mod sandbox;

mod mount;

pub use accessor::{NotionConfig, S3Config};
pub use error::{ResourceError, ResourceResult};
pub(crate) use mount::build_mounts;
pub use mount::{FsConfig, LOCAL_MOUNT, Mount, MountSpec, ProviderConfig};
pub use path::MountPath;
pub use resource::{
    DirEntry, FileKind, FileStat, LocalResource, NotionResource, Resource, S3Resource,
};
pub use sandbox::{ForwardFs, FwdEntry, FwdStat, TunnelServer};
