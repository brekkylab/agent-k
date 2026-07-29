use std::ops::Range;

use async_trait::async_trait;
use futures::StreamExt;
use object_store::{
    Error as OsError, GetOptions, GetRange, ObjectStore, ObjectStoreExt, PutPayload,
    path::Path as OsPath,
};

use crate::vfs::{
    accessor::{S3Accessor, S3Config},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

const S3_PROMPT: &str = "\
Amazon S3 (read/write). Object keys map to paths; directories are key prefixes.
Standard shell tools work: ls, cat, head, grep, find, tee, rm, cp/mv.
Remote mount — prefer head/grep over cat on large objects.";

pub struct S3Resource {
    accessor: S3Accessor,
}

impl S3Resource {
    pub fn new(config: &S3Config) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: S3Accessor::new(config)?,
        })
    }

    // `Path::parse` (not `Path::from`): `from` percent-encodes `% # ? [ ] ~`, so a
    // key readdir surfaced raw (`50%off.txt`) would be addressed as `50%25off.txt`
    // by stat/read/write — a different object. `parse` preserves those bytes, and
    // still rejects `.`/`..` segments, so the mount can't address keys outside
    // `key_prefix`. An unparseable key can't name an object → NotFound.
    fn os_path(&self, path: &MountPath) -> ResourceResult<OsPath> {
        OsPath::parse(self.accessor.key(path)).map_err(|_| ResourceError::NotFound)
    }

    fn list_prefix(&self, path: &MountPath) -> ResourceResult<Option<OsPath>> {
        let key = self.accessor.key(path);
        if key.is_empty() {
            Ok(None)
        } else {
            Ok(Some(
                OsPath::parse(key).map_err(|_| ResourceError::NotFound)?,
            ))
        }
    }
}

#[async_trait]
impl Resource for S3Resource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        let os_path = self.os_path(path)?;
        let opts = GetOptions {
            range: range.clone().map(GetRange::Bounded),
            ..Default::default()
        };
        match self.accessor.store.get_opts(&os_path, opts).await {
            Ok(res) => Ok(res.bytes().await?.to_vec()),
            Err(e) => {
                // S3-2: a bounded range starting at/after EOF returns 416. Treat
                // it as a clean EOF (empty) rather than EIO, so cat/wc/dd over a
                // direct_io mount (size unknown up front) stop cleanly. Only the
                // error path pays the extra head.
                if let Some(r) = &range
                    && let Ok(meta) = self.accessor.store.head(&os_path).await
                    && r.start >= meta.size
                {
                    return Ok(Vec::new());
                }
                Err(e.into())
            }
        }
    }

    async fn read_bytes_pinned(
        &self,
        path: &MountPath,
        range: Option<Range<u64>>,
        stat: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        // Pin the read to the snapshot's ETag: if the object changed since the
        // read opened, S3 returns 412 rather than newer bytes, so a multi-chunk
        // read can't stitch two versions together.
        let os_path = self.os_path(path)?;
        let opts = GetOptions {
            range: range.clone().map(GetRange::Bounded),
            if_match: stat.etag.clone(),
            ..Default::default()
        };
        match self.accessor.store.get_opts(&os_path, opts).await {
            Ok(res) => Ok(res.bytes().await?.to_vec()),
            Err(OsError::Precondition { .. }) => Err(ResourceError::Backend(anyhow::anyhow!(
                "object changed during read (if-match precondition failed)"
            ))),
            Err(e) => {
                // Same clean-EOF handling as `read_bytes`: a range at/after EOF
                // yields 416, which we treat as an empty (EOF) read.
                if let Some(r) = &range
                    && let Ok(meta) = self.accessor.store.head(&os_path).await
                    && r.start >= meta.size
                {
                    return Ok(Vec::new());
                }
                Err(e.into())
            }
        }
    }

    async fn write_bytes(&self, path: &MountPath, data: Vec<u8>) -> ResourceResult<()> {
        self.accessor
            .store
            .put(&self.os_path(path)?, PutPayload::from(data))
            .await?;
        Ok(())
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        let listing = self.list_prefix(path)?;
        let res = self
            .accessor
            .store
            .list_with_delimiter(listing.as_ref())
            .await?;
        // The key we listed under; an object whose key equals it is the
        // zero-byte "directory marker" for this prefix and must be skipped.
        let marker = listing.as_ref().map(|p| p.as_ref()).unwrap_or("");
        let mut out = Vec::new();
        for cp in res.common_prefixes {
            if let Some(name) = cp.filename() {
                out.push(DirEntry {
                    name: name.to_string(),
                    kind: FileKind::Dir,
                    size: 0,
                    mtime: None,
                    atime: None,
                    ctime: None,
                    created: None,
                    etag: None,
                    content_type: None,
                });
            }
        }
        for obj in res.objects {
            if obj.location.as_ref() == marker {
                continue;
            }
            if let Some(name) = obj.location.filename() {
                out.push(DirEntry {
                    name: name.to_string(),
                    kind: FileKind::File,
                    size: obj.size,
                    // Carry per-entry mtime so the stat fast-path (`ls -l`)
                    // serves it from cache instead of the epoch (R2). S3 has no
                    // access/change time.
                    mtime: Some(obj.last_modified.into()),
                    atime: None,
                    ctime: None,
                    created: None,
                    // Carry the listing ETag so a read opened off the cached
                    // stat can pin itself to it (`If-Match`).
                    etag: obj.e_tag.clone(),
                    content_type: None,
                });
            }
        }
        // Return one merged, name-sorted listing; coreutils `ls` re-sorts
        // anyway, but this keeps the raw readdir order stable.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        if path.is_root() {
            return Ok(FileStat {
                kind: FileKind::Dir,
                ..Default::default()
            });
        }
        match self.accessor.store.head(&self.os_path(path)?).await {
            Ok(meta) => Ok(FileStat {
                kind: FileKind::File,
                size: meta.size,
                mtime: Some(meta.last_modified.into()),
                // S3 reports only LastModified; no access/change/birth time.
                atime: None,
                ctime: None,
                created: None,
                etag: meta.e_tag.clone(),
                version: meta.version.clone(),
                // The key is the truth (and carries its own extension).
                content_type: None,
            }),
            Err(OsError::NotFound { .. }) => {
                let res = self
                    .accessor
                    .store
                    .list_with_delimiter(self.list_prefix(path)?.as_ref())
                    .await?;
                if res.common_prefixes.is_empty() && res.objects.is_empty() {
                    return Err(ResourceError::NotFound);
                }
                Ok(FileStat {
                    kind: FileKind::Dir,
                    ..Default::default()
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        self.accessor.store.delete(&self.os_path(path)?).await?;
        Ok(())
    }

    async fn mkdir(&self, _path: &MountPath) -> ResourceResult<()> {
        // Object stores have no real directories: a prefix exists implicitly once
        // a key is written under it, and `object_store::Path` can't represent a
        // trailing-slash marker. So mkdir is a no-op success (the dir appears as
        // soon as something is written into it).
        Ok(())
    }

    async fn rmdir(&self, path: &MountPath) -> ResourceResult<()> {
        // Recursively delete everything under the prefix (mirrors mirage's
        // prefix batch delete).
        let prefix = self.list_prefix(path)?;
        let mut stream = self.accessor.store.list(prefix.as_ref());
        while let Some(item) = stream.next().await {
            let meta = item?;
            self.accessor.store.delete(&meta.location).await?;
        }
        Ok(())
    }

    async fn rename(&self, from: &MountPath, to: &MountPath) -> ResourceResult<()> {
        // S3 has no native rename: copy then delete the source (mirrors mirage).
        let (from, to) = (self.os_path(from)?, self.os_path(to)?);
        self.accessor.store.copy(&from, &to).await?;
        self.accessor.store.delete(&from).await?;
        Ok(())
    }

    fn prompt(&self) -> &str {
        S3_PROMPT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::accessor::S3Config;

    fn resource() -> S3Resource {
        S3Resource::new(&S3Config {
            bucket: "b".into(),
            region: "us-east-1".into(),
            access_key_id: "k".into(),
            secret_access_key: "s".into(),
            endpoint: None,
            key_prefix: None,
        })
        .unwrap()
    }

    #[test]
    fn special_char_key_round_trips() {
        let r = resource();
        for name in ["50%off.txt", "a#b.txt", "note~1", "q?x", "a[b]"] {
            let vp = MountPath::new(&format!("/{name}"));
            assert_eq!(
                r.os_path(&vp).unwrap().as_ref(),
                r.accessor.key(&vp),
                "round-trip broken for {name:?}"
            );
        }
    }

    #[test]
    fn dot_segments_are_rejected() {
        let r = resource();
        for bad in ["/../up", "/a/../b", "/."] {
            assert!(
                matches!(
                    r.os_path(&MountPath::new(bad)),
                    Err(ResourceError::NotFound)
                ),
                "should reject {bad:?}"
            );
        }
    }

    /// An [`S3Config`] from `S3_*` env vars, or `None` when the required ones
    /// (bucket + credentials) are unset. Point `S3_ENDPOINT` at MinIO/localstack
    /// for a local run, or leave it unset for real AWS.
    fn live_config() -> Option<S3Config> {
        Some(S3Config {
            bucket: std::env::var("S3_BUCKET").ok()?,
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key_id: std::env::var("S3_ACCESS_KEY_ID").ok()?,
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY").ok()?,
            endpoint: std::env::var("S3_ENDPOINT").ok(),
            key_prefix: std::env::var("S3_KEY_PREFIX").ok(),
        })
    }

    /// Live round-trip against a real S3-compatible bucket: write -> stat ->
    /// read (full + ranged) -> readdir -> delete. Ignored by default; set the
    /// `S3_*` env vars to run:
    ///
    ///   S3_BUCKET=… S3_ACCESS_KEY_ID=… S3_SECRET_ACCESS_KEY=… [S3_REGION=…] \
    ///   [S3_ENDPOINT=…] cargo test -p workspace s3_live_round_trip -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires S3_* env + network"]
    async fn s3_live_round_trip() {
        let Some(cfg) = live_config() else {
            eprintln!("set S3_BUCKET / S3_ACCESS_KEY_ID / S3_SECRET_ACCESS_KEY to run");
            return;
        };
        let r = S3Resource::new(&cfg).expect("build S3Resource");

        // Unique key under a dedicated test prefix so a stray failure can't
        // clobber real data.
        let name = format!("agentk-livetest-{}.txt", uuid::Uuid::new_v4());
        let dir = MountPath::new("/agentk-livetest");
        let vp = MountPath::new(format!("/agentk-livetest/{name}"));
        let body = b"hello s3 live".to_vec();

        r.write_bytes(&vp, body.clone()).await.expect("write");

        let st = r.stat(&vp).await.expect("stat");
        assert!(matches!(st.kind, FileKind::File));
        assert_eq!(st.size, body.len() as u64);

        assert_eq!(r.read_bytes(&vp, None).await.expect("read"), body);
        assert_eq!(
            r.read_bytes(&vp, Some(6..13)).await.expect("ranged read"),
            b"s3 live"
        );

        let entries = r.readdir(&dir).await.expect("readdir");
        assert!(
            entries.iter().any(|e| e.name == name),
            "listing missing {name}: {:?}",
            entries.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        r.unlink(&vp).await.expect("unlink");
        assert!(matches!(r.stat(&vp).await, Err(ResourceError::NotFound)));
    }
}
