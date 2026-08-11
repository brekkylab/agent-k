//! Mount configuration and assembly for [`WorkspaceFs`](crate::WorkspaceFs).
//!
//! A [`Mount`] binds a virtual top-level prefix to an instantiated [`Resource`];
//! [`FsConfig`] is the (credential-carrying, host-only, never-persisted) recipe
//! for a workspace's mount table, and [`build_mounts`] turns it into live
//! [`Mount`]s. `WorkspaceFs` owns the resulting mounts and does the routing —
//! there is no separate `Vfs` type.

use std::{ops::Range, path::PathBuf, sync::Arc};

use crate::vfs::{
    accessor::{GdriveConfig, GmailConfig, GoogleClient, NotionConfig, S3Config},
    cache::CachedResource,
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{
        DirEntry, FileStat, GdriveResource, GmailResource, LocalResource, NotionResource, Resource,
        S3Resource,
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
    /// (tests, library users with no Google mount) mounts such a source as one that fails
    /// every read: a missing env var
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

/// A mount that keeps its place in the table and fails every read of it.
///
/// Reads are what matters here; the mutating ops keep the trait's `Unsupported`, which a
/// read-only mount would answer anyway.
///
/// `Backend` and not `NotFound`, which is the whole point: an absent or empty source
/// reads to a caller as "nothing to do here", and the knowledge index acts on that by
/// purging what it had already ingested from it. This shape is what a rejected
/// credential already produces, so it lands in the arm that retries.
struct Unavailable(&'static str);

#[async_trait::async_trait]
impl Resource for Unavailable {
    async fn read_bytes(&self, _: &MountPath, _: Option<Range<u64>>) -> ResourceResult<Vec<u8>> {
        Err(ResourceError::Backend(anyhow::anyhow!("{}", self.0)))
    }
    async fn read_bytes_pinned(
        &self,
        p: &MountPath,
        r: Option<Range<u64>>,
        _: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        self.read_bytes(p, r).await
    }
    async fn write_bytes(&self, _: &MountPath, _: Vec<u8>) -> ResourceResult<()> {
        Err(ResourceError::Backend(anyhow::anyhow!("{}", self.0)))
    }
    async fn readdir(&self, _: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        Err(ResourceError::Backend(anyhow::anyhow!("{}", self.0)))
    }
    async fn stat(&self, _: &MountPath) -> ResourceResult<FileStat> {
        Err(ResourceError::Backend(anyhow::anyhow!("{}", self.0)))
    }
}

/// Stand a Google mount up in name only, when the deployment has no OAuth client. The
/// message is what an operator gets: `FsError` carries no payload, so it does not travel
/// past the filesystem boundary.
fn unavailable_google_mount(provider: &str, prefix: &str) -> Arc<dyn Resource> {
    tracing::warn!(
        "{provider} mount {prefix} cannot serve: no Google OAuth client configured \
         (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)"
    );
    Arc::new(Unavailable(
        "no Google OAuth client configured (GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET)",
    ))
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
                index_cap: None,
            }),
        }
    }

    fn gdrive_spec() -> MountSpec {
        MountSpec {
            prefix: "/gdrive".into(),
            provider: ProviderConfig::Gdrive(GdriveConfig {
                refresh_token: "rt".into(),
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
        // Mirrors `add_target`'s own condition rather than naming a variant: it swallows
        // `NotFound | Forbidden` and bails on everything else, so which side of that line
        // this lands on is the whole invariant. Asserting `GeneralFailure` specifically
        // would fail on a harmless remapping.
        assert!(
            !matches!(
                err,
                crate::fs::FsError::NotFound | crate::fs::FsError::Forbidden
            ),
            "got {err:?}; on that side the knowledge index purges what it ingested here"
        );
        // And the prefix is still listed, so nothing has to infer the source is gone.
        assert!(fs.mount_names().iter().any(|n| n == "gmail"));

        // The same distinction one layer down, where the guest sees it. `ForwardFs::stat`
        // used to fold every error into `exists: false`, which `lookup` reports as ENOENT,
        // so an unreachable source read as an absent one inside the sandbox.
        use crate::vfs::ForwardFs;
        assert!(
            ForwardFs::stat(&fs, "/gmail/INBOX").await.is_err(),
            "an unreachable path must not be reported to the guest as simply missing"
        );
        let absent = ForwardFs::stat(&fs, "/files/nope")
            .await
            .expect("a path that is genuinely not there is still an answer");
        assert!(!absent.exists);
    }

    /// With one configured, the same table carries both Google sources.
    #[tokio::test]
    async fn a_configured_client_mounts_both_google_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let mounts = build_mounts(FsConfig {
            local_root: Some(tmp.path().to_path_buf()),
            mirror_root: Some(tmp.path().to_path_buf()),
            google_oauth: Some(GoogleClient {
                client_id: "cid".into(),
                client_secret: "cs".into(),
                origins: Default::default(),
            }),
            mounts: vec![gmail_spec(), gdrive_spec()],
        })
        .expect("assembles");

        let mut prefixes: Vec<_> = mounts.iter().map(|m| m.prefix.clone()).collect();
        prefixes.sort();
        assert_eq!(prefixes, vec!["/files", "/gdrive", "/gmail"]);

        // Reading through them, not just counting names: with only the prefixes asserted,
        // an arm that ignored the client and mounted the unavailable stand-in anyway went
        // unnoticed. The stand-in says what it is, and a real provider never does.
        for m in mounts.iter().filter(|m| m.prefix != LOCAL_MOUNT) {
            if let Err(e) = m.resource.readdir(&MountPath::new("/")).await {
                assert!(
                    !e.to_string().contains("GOOGLE_CLIENT_ID"),
                    "{} was mounted unavailable despite a configured client",
                    m.prefix
                );
            }
        }
    }
}
