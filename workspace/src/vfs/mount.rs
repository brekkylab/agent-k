//! Mount configuration and assembly for [`WorkspaceFs`](crate::WorkspaceFs).
//!
//! A [`Mount`] binds a virtual top-level prefix to an instantiated [`Resource`];
//! [`FsConfig`] is the (credential-carrying, host-only, never-persisted) recipe
//! for a workspace's mount table, and [`build_mounts`] turns it into live
//! [`Mount`]s. `WorkspaceFs` owns the resulting mounts and does the routing —
//! there is no separate `Vfs` type.

use std::path::PathBuf;
use std::sync::Arc;

use crate::vfs::{
    accessor::{NotionConfig, S3Config, SlackConfig},
    cache::CachedResource,
    resource::{LocalResource, NotionResource, Resource, S3Resource, SlackResource},
};

/// Reserved mount prefix for the workspace's local file tree. A provider mount
/// may not use it (the backend rejects it at mount-create time).
pub const LOCAL_MOUNT: &str = "/files";

/// Per-mount provider configuration (carries credentials, host-only).
#[derive(Clone)]
pub enum ProviderConfig {
    S3(S3Config),
    Notion(NotionConfig),
    Slack(SlackConfig),
}

/// One mount spec: a virtual top-level prefix bound to a provider config. The
/// same provider type may appear multiple times with different credentials.
#[derive(Clone)]
pub struct MountSpec {
    pub prefix: String,
    pub provider: ProviderConfig,
}

/// The recipe for a workspace's mount table, assembled into live [`Mount`]s by
/// [`build_mounts`] and handed to
/// [`WorkspaceFs::from_config`](crate::WorkspaceFs::from_config).
#[derive(Clone, Default)]
pub struct FsConfig {
    /// The workspace's local file root, mounted at [`LOCAL_MOUNT`]. `None` builds
    /// a provider-only mount table (unit tests). This is an in-memory assembly
    /// struct, never persisted — the DB stores per-mount rows and the caller
    /// fills this.
    pub local_root: Option<PathBuf>,
    pub mounts: Vec<MountSpec>,
}

/// A live mount: prefix bound to an instantiated [`Resource`].
pub struct Mount {
    pub prefix: String,
    pub resource: Arc<dyn Resource>,
}

/// Instantiate the resources described by an [`FsConfig`] into live [`Mount`]s:
/// the local file tree (when `local_root` is set) at [`LOCAL_MOUNT`] plus the
/// provider mounts. [`WorkspaceFs::from_config`](crate::WorkspaceFs::from_config)
/// validates and sorts the result.
pub(crate) fn build_mounts(config: FsConfig) -> anyhow::Result<Vec<Mount>> {
    let mut mounts = Vec::with_capacity(config.mounts.len() + 1);
    // Local disk is a mount like any other, but not wrapped in the metadata
    // cache — it's live and cheap, unlike the remote providers below.
    if let Some(root) = config.local_root {
        mounts.push(Mount {
            prefix: LOCAL_MOUNT.to_string(),
            resource: Arc::new(LocalResource::new(root)),
        });
    }
    for spec in config.mounts {
        let provider: Arc<dyn Resource> = match spec.provider {
            ProviderConfig::S3(c) => Arc::new(S3Resource::new(&c)?),
            ProviderConfig::Notion(c) => Arc::new(NotionResource::new(&c)?),
            ProviderConfig::Slack(c) => Arc::new(SlackResource::new(&c)?),
        };
        // Wrap every provider in the metadata index cache so `stat` after a
        // `readdir` (e.g. `ls -la`) is served from memory.
        mounts.push(Mount {
            prefix: spec.prefix,
            resource: Arc::new(CachedResource::new(provider)),
        });
    }
    Ok(mounts)
}
