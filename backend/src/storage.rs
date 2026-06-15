use std::{
    io,
    path::{Component, Path, PathBuf},
};

use ailoy::runenv::SandboxSnapshot;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub enum Dirent {
    Dir {},
    File {},
}

pub struct Storage {
    data_root: PathBuf,
}

impl Storage {
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub fn get_project_dir(&self, pid: impl AsRef<str>) -> PathBuf {
        self.data_root.join("projects").join(pid.as_ref())
    }

    pub fn get_workspace_dir(&self, pid: impl AsRef<str>) -> PathBuf {
        self.get_project_dir(pid).join("workspace")
    }

    pub fn get_session_dir(&self, pid: impl AsRef<str>, sid: impl AsRef<str>) -> PathBuf {
        self.get_project_dir(pid)
            .join("sessions")
            .join(sid.as_ref())
    }

    pub fn get_attachments_dir(&self, pid: impl AsRef<str>, sid: impl AsRef<str>) -> PathBuf {
        self.get_session_dir(pid, sid).join("attachments")
    }

    pub fn get_artifacts_dir(&self, pid: impl AsRef<str>, sid: impl AsRef<str>) -> PathBuf {
        self.get_session_dir(pid, sid).join("artifacts")
    }

    pub async fn create_project(&self, pid: impl AsRef<str>) -> io::Result<()> {
        let pid = pid.as_ref();
        tokio::fs::create_dir(self.get_project_dir(pid)).await?;
        tokio::fs::create_dir(self.get_workspace_dir(pid)).await?;
        tokio::fs::create_dir(self.get_project_dir(pid).join("sessions")).await?;
        Ok(())
    }

    pub async fn remove_project(&self, pid: impl AsRef<str>) -> io::Result<()> {
        match tokio::fs::remove_dir_all(self.get_project_dir(pid)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn create_session(
        &self,
        pid: impl AsRef<str>,
        sid: impl AsRef<str>,
    ) -> io::Result<()> {
        let pid = pid.as_ref();
        let sid = sid.as_ref();
        if !self.get_project_dir(pid).join("sessions").exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "project not created",
            ));
        }
        tokio::fs::create_dir(self.get_session_dir(pid, sid)).await?;
        tokio::fs::create_dir(self.get_attachments_dir(pid, sid)).await?;
        tokio::fs::create_dir(self.get_artifacts_dir(pid, sid)).await?;
        Ok(())
    }

    pub async fn remove_session(
        &self,
        pid: impl AsRef<str>,
        sid: impl AsRef<str>,
    ) -> io::Result<()> {
        match tokio::fs::remove_dir_all(self.get_session_dir(pid, sid)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Create a workspace subdirectory at `rel` (and any missing parents).
    pub async fn insert_workspace_dir(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
    ) -> io::Result<()> {
        let target = safe_join(&self.get_workspace_dir(pid), rel)?;
        tokio::fs::create_dir_all(&target).await?;
        Ok(())
    }

    /// Recursively remove a workspace subdirectory. Idempotent on `NotFound`.
    pub async fn remove_workspace_dir(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
    ) -> io::Result<()> {
        let target = safe_join(&self.get_workspace_dir(pid), rel)?;
        match tokio::fs::remove_dir_all(&target).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn insert_workspace_file<R>(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
        reader: R,
        max_bytes: u64,
    ) -> io::Result<()>
    where
        R: AsyncRead + Unpin,
    {
        let target = safe_join(&self.get_workspace_dir(pid), rel)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        self.insert_file(&target, reader, max_bytes).await?;
        Ok(())
    }

    /// Remove a workspace file. Errors with `InvalidInput` if `rel` resolves to
    /// a directory (use `remove_workspace_dir` for that). Idempotent on `NotFound`.
    pub async fn remove_workspace_file(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
    ) -> io::Result<()> {
        let target = safe_join(&self.get_workspace_dir(pid), rel)?;
        match tokio::fs::symlink_metadata(&target).await {
            Ok(m) if m.is_dir() => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "target is a directory; use remove_workspace_dir",
            )),
            Ok(_) => tokio::fs::remove_file(&target).await,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Copy a workspace entry (file or directory) from `src` to `dst`,
    /// both relative to the workspace root. Missing parent dirs of `dst` are
    /// created. For directories the contents are copied recursively; symlinks
    /// are skipped.
    pub async fn copy_workspace_entry(
        &self,
        pid: impl AsRef<str>,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> io::Result<()> {
        let workspace = self.get_workspace_dir(pid);
        let src_path = safe_join(&workspace, src)?;
        let dst_path = safe_join(&workspace, dst)?;
        if let Some(parent) = dst_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        copy_path_recursive(&src_path, &dst_path).await
    }

    /// Move a workspace entry from `src` to `dst` (both workspace-relative).
    /// Uses `rename(2)`; since both paths live under `workspace/`, this is
    /// always within the same filesystem.
    pub async fn move_workspace_entry(
        &self,
        pid: impl AsRef<str>,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> io::Result<()> {
        let workspace = self.get_workspace_dir(pid);
        let src_path = safe_join(&workspace, src)?;
        let dst_path = safe_join(&workspace, dst)?;
        if let Some(parent) = dst_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&src_path, &dst_path).await
    }

    /// Load the session's sandbox archive (`archive.tar.zst`) if present.
    /// Returns `Ok(None)` when no archive has been written for the session.
    pub async fn get_session_archive(
        &self,
        pid: impl AsRef<str>,
        sid: impl AsRef<str>,
    ) -> io::Result<Option<SandboxSnapshot>> {
        let path = self.get_session_dir(pid, sid).join("archive.tar.zst");
        match tokio::fs::symlink_metadata(&path).await {
            Ok(_) => {
                let snap = SandboxSnapshot::try_from_archive(&path)
                    .await
                    .map_err(io::Error::other)?;
                Ok(Some(snap))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }

    /// Write `snapshot` to the session's archive at `archive.tar.zst`.
    /// The session directory must already exist (created by `create_session`).
    pub async fn update_session_archive(
        &self,
        pid: impl AsRef<str>,
        sid: impl AsRef<str>,
        snapshot: SandboxSnapshot,
    ) -> io::Result<()> {
        snapshot
            .archive(&self.get_session_dir(pid, sid), "archive")
            .await
            .map_err(io::Error::other)?;
        Ok(())
    }

    pub async fn insert_attachment_file<R>(
        &self,
        pid: impl AsRef<str>,
        sid: impl AsRef<str>,
        rel: impl AsRef<Path>,
        reader: R,
        max_bytes: u64,
    ) -> io::Result<()>
    where
        R: AsyncRead + Unpin,
    {
        let target = safe_join(&self.get_attachments_dir(pid, sid), rel)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        self.insert_file(&target, reader, max_bytes).await?;
        Ok(())
    }

    // // ── File operations ──────────────────────────────────────────────────

    /// Atomically write `reader` to `path` with a `max_bytes` cap.
    ///
    /// Caller is responsible for path validation and ensuring the parent dir exists.
    /// On any error the partial temp file is cleaned up and `path` is left untouched.
    async fn insert_file<R>(
        &self,
        path: impl AsRef<Path>,
        mut reader: R,
        max_bytes: u64,
    ) -> io::Result<u64>
    where
        R: AsyncRead + Unpin,
    {
        let path = path.as_ref();
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
        // Temp file lives in the same dir as `path` so the final rename stays on the same fs.
        let tmp = parent.join(format!(".tmp.{}", Uuid::new_v4().simple()));

        let result: io::Result<u64> = async {
            let mut file = tokio::fs::File::create(&tmp).await?;
            let mut buf = vec![0u8; 64 * 1024];
            let mut total: u64 = 0;
            loop {
                let n = reader.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                let n_u64 = n as u64;
                if total.saturating_add(n_u64) > max_bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        format!("file exceeds maximum size ({max_bytes} bytes)"),
                    ));
                }
                total += n_u64;
                file.write_all(&buf[..n]).await?;
            }
            file.sync_all().await?;
            tokio::fs::rename(&tmp, path).await?;
            Ok(total)
        }
        .await;

        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Copy `src` to `dst` recursively, handling both files and directories.
/// Iterative (no async recursion). Symlinks are skipped.
async fn copy_path_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((s, d)) = stack.pop() {
        let meta = tokio::fs::symlink_metadata(&s).await?;
        if meta.is_dir() {
            tokio::fs::create_dir_all(&d).await?;
            let mut rd = tokio::fs::read_dir(&s).await?;
            while let Some(entry) = rd.next_entry().await? {
                stack.push((entry.path(), d.join(entry.file_name())));
            }
        } else if meta.is_file() {
            if let Some(parent) = d.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&s, &d).await?;
        }
        // Symlinks are intentionally skipped — they're not used by Storage's writers.
    }
    Ok(())
}

fn safe_join(root: &Path, rel: impl AsRef<Path>) -> io::Result<PathBuf> {
    let rel = rel.as_ref();
    if rel.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must not be empty",
        ));
    }
    if rel.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path must not contain NUL bytes",
        ));
    }

    let mut normalized = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must not contain '..' segments",
                ));
            }
            Component::RootDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must not be absolute",
                ));
            }
            Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must not contain a drive prefix",
                ));
            }
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is empty after normalization",
        ));
    }

    Ok(root.join(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const MAX: u64 = 1024 * 1024;

    fn pid() -> &'static str {
        "pid"
    }
    fn sid() -> &'static str {
        "sid"
    }

    #[tokio::test]
    async fn project_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());

        storage.create_project(pid()).await.unwrap();
        assert!(tmp.path().join("projects/pid/workspace").is_dir());
        assert!(tmp.path().join("projects/pid/sessions").is_dir());

        storage.remove_project(pid()).await.unwrap();
        assert!(!tmp.path().join("projects/pid").exists());
        // idempotent
        storage.remove_project(pid()).await.unwrap();
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());

        storage.create_project(pid()).await.unwrap();
        storage.create_session(pid(), sid()).await.unwrap();
        let session = tmp.path().join("projects/pid/sessions/sid");
        assert!(session.join("attachments").is_dir());
        assert!(session.join("artifacts").is_dir());

        storage.remove_session(pid(), sid()).await.unwrap();
        assert!(!session.exists());
        assert!(tmp.path().join("projects/pid/workspace").is_dir());
    }

    #[tokio::test]
    async fn create_session_errors_when_project_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());

        let err = storage.create_session(pid(), sid()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn insert_workspace_file_writes_content() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();

        storage
            .insert_workspace_file(pid(), "out/report.txt", Cursor::new(b"hello".to_vec()), MAX)
            .await
            .unwrap();
        let target = tmp.path().join("projects/pid/workspace/out/report.txt");
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn insert_workspace_file_enforces_size_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();

        let err = storage
            .insert_workspace_file(pid(), "big.bin", Cursor::new(vec![0u8; 10]), 5)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge);

        // Atomic write rolled back — neither the target nor a stray tmp file remain.
        let workspace = tmp.path().join("projects/pid/workspace");
        let mut entries = tokio::fs::read_dir(&workspace).await.unwrap();
        assert!(
            entries.next_entry().await.unwrap().is_none(),
            "workspace should be empty"
        );
    }

    #[tokio::test]
    async fn insert_attachment_file_writes_under_session_attachments() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();
        storage.create_session(pid(), sid()).await.unwrap();

        storage
            .insert_attachment_file(pid(), sid(), "note.md", Cursor::new(b"hi".to_vec()), MAX)
            .await
            .unwrap();
        let target = tmp
            .path()
            .join("projects/pid/sessions/sid/attachments/note.md");
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"hi");
    }

    #[tokio::test]
    async fn move_and_remove_workspace_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();

        storage
            .insert_workspace_file(pid(), "a.txt", Cursor::new(b"a".to_vec()), MAX)
            .await
            .unwrap();
        storage
            .move_workspace_entry(pid(), "a.txt", "sub/b.txt")
            .await
            .unwrap();
        let moved = tmp.path().join("projects/pid/workspace/sub/b.txt");
        assert_eq!(tokio::fs::read(&moved).await.unwrap(), b"a");

        storage
            .remove_workspace_file(pid(), "sub/b.txt")
            .await
            .unwrap();
        assert!(!moved.exists());
        // Idempotent on NotFound.
        storage
            .remove_workspace_file(pid(), "sub/b.txt")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn remove_workspace_file_rejects_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();
        storage.insert_workspace_dir(pid(), "sub").await.unwrap();

        let err = storage
            .remove_workspace_file(pid(), "sub")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn copy_workspace_entry_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();

        storage
            .insert_workspace_file(pid(), "src/a.txt", Cursor::new(b"a".to_vec()), MAX)
            .await
            .unwrap();
        storage
            .insert_workspace_file(pid(), "src/sub/b.txt", Cursor::new(b"b".to_vec()), MAX)
            .await
            .unwrap();
        storage
            .copy_workspace_entry(pid(), "src", "dst")
            .await
            .unwrap();

        let root = tmp.path().join("projects/pid/workspace");
        assert_eq!(tokio::fs::read(root.join("dst/a.txt")).await.unwrap(), b"a");
        assert_eq!(
            tokio::fs::read(root.join("dst/sub/b.txt")).await.unwrap(),
            b"b"
        );
        // Original is still there (copy, not move).
        assert!(root.join("src/a.txt").exists());
    }

    #[tokio::test]
    async fn safe_join_rejects_unsafe_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = Storage::new(tmp.path());
        storage.create_project(pid()).await.unwrap();

        for bad in ["", "../escape", "/abs", "."] {
            let err = storage
                .insert_workspace_file(pid(), bad, Cursor::new(b"x".to_vec()), MAX)
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "bad={bad}");
        }
    }
}
