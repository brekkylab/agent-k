//! Virtual filesystem over external providers (S3, Notion).
//!
//! Vendored from ailoy's `feat/vfs-provider-mounts` branch (commit 61c4c43),
//! reduced to the **frontend-agnostic core**: the FUSE / sandbox frontends,
//! the per-agent `AgentVfs` handle, and the Google Drive accessor were dropped.
//! agent-k wraps this core in its WebDAV workspace layer (see
//! [`crate::state`] / [`crate::router::webdav`]) instead of a FUSE mount.
//!
//! A single [`Vfs`] holds the provider [`Resource`]s and routes virtual paths
//! to them by longest-prefix match; it is the single source of truth for
//! provider access.

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

#[allow(clippy::module_inception)]
mod vfs;

pub use accessor::{NotionConfig, S3Config};
pub use error::{VfsError, VfsResult};
pub use path::VPath;
pub use resource::{DirEntry, FileKind, FileStat, Resource, S3Resource};
pub use vfs::{Mount, MountSpec, ProviderConfig, Vfs, VfsConfig};
