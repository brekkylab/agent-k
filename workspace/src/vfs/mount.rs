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
        UnavailableResource,
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
    /// (tests, library users with no Google mount) mounts such a source as
    /// [`UnavailableResource`], which fails every op with a reason: a missing env var
    /// costs the source it configures and not `/files` along with it, without the
    /// source looking empty to whoever reads it.
    pub google_oauth: Option<GoogleClient>,
    pub mounts: Vec<MountSpec>,
}

/// A live mount: prefix bound to an instantiated [`Resource`].
pub struct Mount {
    pub prefix: String,
    pub resource: Arc<dyn Resource>,
}

/// Stand a Google mount up in name only, when the deployment has no OAuth client.
///
/// The mount keeps its prefix and fails every op with a reason, rather than being left
/// out of the table: an absent prefix is unroutable, and `NotFound` reads to a caller as
/// "there is nothing here" rather than "this could not be reached". See
/// [`UnavailableResource`] for what that costs.
fn unavailable_google_mount(provider: &str, prefix: &str) -> Arc<dyn Resource> {
    tracing::warn!(
        "{provider} mount {prefix} cannot serve: no Google OAuth client configured \
         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)"
    );
    Arc::new(UnavailableResource::new(format!(
        "{provider} is connected but this deployment has no Google OAuth client \
         configured (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET), so it cannot \
         authenticate"
    )))
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
    let mirror_root = config.mirror_root;
    let google_oauth = config.google_oauth;
    for spec in config.mounts {
        let provider: Arc<dyn Resource> = match spec.provider {
            ProviderConfig::S3(c) => Arc::new(S3Resource::new(&c)?),
            ProviderConfig::Notion(c) => Arc::new(NotionResource::new(&c)?),
            ProviderConfig::Gmail(c) => {
                let Some(oauth) = google_oauth.as_ref() else {
                    mounts.push(Mount {
                        resource: unavailable_google_mount("Gmail", &spec.prefix),
                        prefix: spec.prefix,
                    });
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
                    mounts.push(Mount {
                        resource: unavailable_google_mount("Drive", &spec.prefix),
                        prefix: spec.prefix,
                    });
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

    /// A deployment with no OAuth client keeps its Google mounts, failing, and keeps
    /// `/files` working.
    ///
    /// Three shapes were possible and the other two are worse. Failing the build takes
    /// `/files` down too, in every workspace holding such a mount. Dropping the mount
    /// makes its prefix unroutable, so every op answers `NotFound` and a caller that
    /// reads that as "nothing here" acts on it: the knowledge index purges every
    /// document it had ingested from the source, and a session prompt lists a source the
    /// tree does not contain.
    #[tokio::test]
    async fn a_missing_oauth_client_keeps_the_mount_and_fails_it_loudly() {
        let tmp = tempfile::tempdir().unwrap();
        let mounts = build_mounts(FsConfig {
            local_root: Some(tmp.path().to_path_buf()),
            mirror_root: Some(tmp.path().to_path_buf()),
            google_oauth: None,
            mounts: vec![gmail_spec(), gdrive_spec()],
        })
        .expect("the workspace still assembles");

        let mut prefixes: Vec<_> = mounts.iter().map(|m| m.prefix.clone()).collect();
        prefixes.sort();
        assert_eq!(
            prefixes,
            vec!["/files", "/gdrive", "/gmail"],
            "local disk survives and the sources keep their names"
        );

        // The distinction the knowledge index depends on: reachable-but-broken, not
        // absent. `NotFound` here would let it purge what it had already ingested.
        for m in mounts.iter().filter(|m| m.prefix != LOCAL_MOUNT) {
            let err = m
                .resource
                .readdir(&crate::vfs::path::MountPath::new("/"))
                .await
                .expect_err("a mount that cannot authenticate does not answer");
            assert!(
                matches!(err, crate::vfs::error::ResourceError::Backend(_)),
                "{} answered {err:?}, which reads as an empty source",
                m.prefix
            );
            assert!(
                err.to_string().contains("GOOGLE_CLIENT_ID"),
                "the error says what is missing: {err}"
            );
        }
    }

    /// The property the knowledge index depends on, through the layer it actually sees.
    ///
    /// `add_target` swallows `NotFound` (a removed ref is how a document is meant to
    /// leave the index) and bails on anything else. So the mapping from the provider's
    /// error to `FsError` is what decides between "retry this pass" and "purge what was
    /// ingested from here", and it is worth pinning rather than inferring.
    #[tokio::test]
    async fn an_unavailable_mount_reads_as_failure_not_absence() {
        let tmp = tempfile::tempdir().unwrap();
        let fs = crate::WorkspaceFs::from_config(FsConfig {
            local_root: Some(tmp.path().to_path_buf()),
            mirror_root: Some(tmp.path().to_path_buf()),
            google_oauth: None,
            mounts: vec![gmail_spec()],
        })
        .expect("assembles");

        let err = fs
            .metadata("/gmail/INBOX")
            .await
            .expect_err("an unauthenticated source does not answer");
        assert!(
            matches!(err, crate::fs::FsError::GeneralFailure),
            "got {err:?}; NotFound here would let the knowledge index purge the source"
        );
        // And the prefix is still listed, so nothing has to infer the source is gone.
        assert!(fs.mount_names().iter().any(|n| n == "gmail"));
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
