//! Virtual filesystem over external providers (S3, Notion, Gmail, GDrive).
//!
//! Vendored from ailoy's `feat/vfs-provider-mounts` branch (commit 61c4c43),
//! reduced to the **frontend-agnostic core**: the FUSE / sandbox frontends and
//! the per-agent `AgentVfs` handle were dropped. (The Google Drive accessor
//! was also dropped then, and later re-added natively — exporting Workspace
//! docs to Office formats rather than ailoy's raw-API-JSON presentation.)
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

pub use accessor::{
    GdriveConfig, GdriveExchange, GmailConfig, GmailExchange, NotionConfig, S3Config,
    exchange_gdrive_code, exchange_gmail_code,
};
pub use error::{ResourceError, ResourceResult};
pub(crate) use mount::build_mounts;
pub use mount::{FsConfig, LOCAL_MOUNT, Mount, MountSpec, ProviderConfig};
pub use path::MountPath;
pub use resource::{
    DirEntry, FileKind, FileStat, GdriveResource, GmailResource, GmailSyncDelta, GmailSyncState,
    LocalResource, NotionResource, Resource, S3Resource, account_mirror_dir, mirror_tree,
    sync_gmail_incremental, sync_gmail_mirror,
};
pub use sandbox::{ForwardFs, ForwardServer, FwdEntry, FwdStat};
