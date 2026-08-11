//! Mount configuration and assembly for [`WorkspaceFs`](crate::WorkspaceFs).
//!
//! A [`Mount`] binds a virtual top-level prefix to an instantiated [`Resource`];
//! [`FsConfig`] is the (credential-carrying, host-only, never-persisted) recipe
//! for a workspace's mount table, and [`build_mounts`] turns it into live
//! [`Mount`]s. `WorkspaceFs` owns the resulting mounts and does the routing —
//! there is no separate `Vfs` type.

use std::{path::PathBuf, sync::Arc};

use crate::vfs::{
    accessor::{GdriveConfig, GmailConfig, GoogleClient, NotionConfig, S3Config},
    cache::CachedResource,
    resource::{
        GdriveResource, GmailResource, LocalResource, NotionResource, Resource, S3Resource,
    },
};

/// Reserved mount prefix for the workspace's local file tree. A provider mount
/// may not use it (the backend rejects it at mount-create time).
pub const LOCAL_MOUNT: &str = "/files";

/// Per-mount provider configuration (carries credentials, host-only).
#[derive(Clone)]
pub enum ProviderConfig {
    S3(S3Config),
    Notion(NotionConfig),
    Gmail(GmailConfig),
    Gdrive(GdriveConfig),
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
    /// Deployment-level mirror directory (the backend passes
    /// `<data_root>/mirror`): providers that serve a synced on-disk mirror
    /// (Gmail) derive their account dirs under it. Must live *outside* every
    /// mount so the guest never sees sync state. `None` (tests, library
    /// users) serves such mounts empty.
    pub mirror_root: Option<PathBuf>,
    /// The deployment's Google OAuth client, needed by the Gmail and Drive mounts to
    /// refresh their access tokens. Deliberately not part of a mount's stored config:
    /// it belongs to the installation, not to any one mount, and a stored copy beside
    /// a refresh token would make that row a usable credential on its own. `None`
    /// (tests, library users with no Google mount) leaves such a mount out of the
    /// table rather than failing the whole workspace, so a missing env var costs the
    /// source it configures and not `/files` along with it.
    pub google_oauth: Option<GoogleClient>,
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
/// Leave a Google mount out, loudly, when the deployment has no OAuth client.
///
/// Dropping one mount rather than failing the build is deliberate: the missing value is
/// deployment-wide, so erroring would take `/files` down with it in every workspace
/// that happens to have a Google source. This matches how a Gmail mount already treats
/// a missing `mirror_root` (it serves empty instead of erroring) and keeps a
/// misconfigured deployment costing only what it misconfigured.
fn skip_without_client(provider: &str, prefix: &str) {
    tracing::warn!(
        "{provider} mount {prefix} left unmounted: no Google OAuth client configured \
         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)"
    );
}

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
    let mirror_root = config.mirror_root;
    let google_oauth = config.google_oauth;
    for spec in config.mounts {
        let provider: Arc<dyn Resource> = match spec.provider {
            ProviderConfig::S3(c) => Arc::new(S3Resource::new(&c)?),
            ProviderConfig::Notion(c) => Arc::new(NotionResource::new(&c)?),
            ProviderConfig::Gmail(c) => {
                let Some(oauth) = google_oauth.as_ref() else {
                    skip_without_client("Gmail", &spec.prefix);
                    continue;
                };
                // Serves local mirror files — live and cheap like the local
                // mount, so it skips the metadata cache (whose listing TTL
                // would hide mail the sync just wrote).
                mounts.push(Mount {
                    prefix: spec.prefix,
                    resource: Arc::new(GmailResource::new(&c, mirror_root.as_deref(), oauth)?),
                });
                continue;
            }
            ProviderConfig::Gdrive(c) => {
                let Some(oauth) = google_oauth.as_ref() else {
                    skip_without_client("Drive", &spec.prefix);
                    continue;
                };
                Arc::new(GdriveResource::new(&c, oauth)?)
            }
        };
        // Wrap remote providers in the metadata index cache so `stat` after a
        // `readdir` (e.g. `ls -la`) is served from memory.
        mounts.push(Mount {
            prefix: spec.prefix,
            resource: Arc::new(CachedResource::new(provider)),
        });
    }
    Ok(mounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gmail_spec() -> MountSpec {
        MountSpec {
            prefix: "/gmail".into(),
            provider: ProviderConfig::Gmail(GmailConfig {
                refresh_token: "rt".into(),
                account_email: "a@b.c".into(),
                origins: Default::default(),
                index_cap: None,
            }),
        }
    }

    fn gdrive_spec() -> MountSpec {
        MountSpec {
            prefix: "/gdrive".into(),
            provider: ProviderConfig::Gdrive(GdriveConfig {
                refresh_token: "rt".into(),
                origins: Default::default(),
            }),
        }
    }

    /// A deployment with no OAuth client loses its Google sources and nothing else.
    ///
    /// The client became injected rather than stored, which introduced a way for one
    /// missing env var to fail the whole assembly — and with it `/files`, in every
    /// workspace that happens to have a Google mount. A workspace losing the source it
    /// cannot authenticate is the cost; losing local disk is not.
    #[test]
    fn a_missing_oauth_client_costs_only_its_own_mounts() {
        let tmp = tempfile::tempdir().unwrap();
        let mounts = build_mounts(FsConfig {
            local_root: Some(tmp.path().to_path_buf()),
            mirror_root: Some(tmp.path().to_path_buf()),
            google_oauth: None,
            mounts: vec![gmail_spec(), gdrive_spec()],
        })
        .expect("the workspace still assembles");

        let prefixes: Vec<_> = mounts.iter().map(|m| m.prefix.as_str()).collect();
        assert_eq!(prefixes, vec![LOCAL_MOUNT], "local disk survives");
    }

    /// With one configured, the same table carries both Google sources.
    #[test]
    fn a_configured_client_mounts_both_google_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let mounts = build_mounts(FsConfig {
            local_root: Some(tmp.path().to_path_buf()),
            mirror_root: Some(tmp.path().to_path_buf()),
            google_oauth: Some(GoogleClient {
                client_id: "cid".into(),
                client_secret: "cs".into(),
            }),
            mounts: vec![gmail_spec(), gdrive_spec()],
        })
        .expect("assembles");

        let mut prefixes: Vec<_> = mounts.iter().map(|m| m.prefix.clone()).collect();
        prefixes.sort();
        assert_eq!(prefixes, vec!["/files", "/gdrive", "/gmail"]);
    }
}
