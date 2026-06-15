use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid path: {0}")]
    InvalidPath(String),
}

pub type FsResult<T> = Result<T, FsError>;

pub struct FSStateV2 {
    root: PathBuf,
}

impl FSStateV2 {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create the project directory tree (`projects/{pid}/{workspace,sessions}`).
    /// Errors with `Io(AlreadyExists)` if the project already exists.
    pub async fn create_project(&self, pid: impl AsRef<str>) -> FsResult<()> {
        validate_pid(pid.as_ref())?;
        let dir = self.root.join("projects").join(pid.as_ref());
        tokio::fs::create_dir_all(self.root.join("projects")).await?;
        tokio::fs::create_dir(&dir).await?;
        tokio::fs::create_dir(dir.join("workspace")).await?;
        tokio::fs::create_dir(dir.join("sessions")).await?;
        Ok(())
    }

    /// Recursively remove a project's directory tree. Idempotent — returns
    /// `Ok(false)` if it didn't exist.
    pub async fn delete_project(&self, pid: impl AsRef<str>) -> FsResult<bool> {
        validate_pid(pid.as_ref())?;
        let dir = self.root.join("projects").join(pid.as_ref());
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Create a workspace subdirectory at `rel` (and any missing parents).
    /// Idempotent: returns `Ok(())` if the directory already exists.
    pub async fn insert_workspace_dir(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
    ) -> FsResult<()> {
        let target = self.workspace_safe_join(pid.as_ref(), rel.as_ref())?;
        tokio::fs::create_dir_all(&target).await?;
        Ok(())
    }

    /// Atomically write `reader` (capped at `max_bytes`) to `rel`. Returns the
    /// number of bytes written. Missing parent directories are created. On
    /// size-cap violation or any other error the destination is left
    /// untouched and the partial tmp file is cleaned up.
    pub async fn insert_workspace_file<R>(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
        reader: R,
        max_bytes: u64,
    ) -> FsResult<u64>
    where
        R: AsyncRead + Unpin,
    {
        let target = self.workspace_safe_join(pid.as_ref(), rel.as_ref())?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        atomic_write(&target, reader, max_bytes).await
    }

    /// Remove a workspace entry (file or directory) at `rel`. Directories are
    /// removed recursively. Idempotent — returns `Ok(false)` if absent.
    pub async fn remove_workspace_entry(
        &self,
        pid: impl AsRef<str>,
        rel: impl AsRef<Path>,
    ) -> FsResult<bool> {
        let target = self.workspace_safe_join(pid.as_ref(), rel.as_ref())?;
        match tokio::fs::symlink_metadata(&target).await {
            Ok(m) if m.is_dir() => {
                tokio::fs::remove_dir_all(&target).await?;
                Ok(true)
            }
            Ok(_) => {
                tokio::fs::remove_file(&target).await?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Copy a workspace entry from `src` to `dst`. Directories are copied
    /// recursively; symlinks are skipped. Missing parents of `dst` are created.
    pub async fn copy_workspace_entry(
        &self,
        pid: impl AsRef<str>,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> FsResult<()> {
        let pid = pid.as_ref();
        let src_path = self.workspace_safe_join(pid, src.as_ref())?;
        let dst_path = self.workspace_safe_join(pid, dst.as_ref())?;
        if let Some(parent) = dst_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        copy_recursive(&src_path, &dst_path).await
    }

    /// Rename a workspace entry from `src` to `dst`. Both paths live under the
    /// same workspace, so this is always a same-filesystem rename.
    pub async fn move_workspace_entry(
        &self,
        pid: impl AsRef<str>,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> FsResult<()> {
        let pid = pid.as_ref();
        let src_path = self.workspace_safe_join(pid, src.as_ref())?;
        let dst_path = self.workspace_safe_join(pid, dst.as_ref())?;
        if let Some(parent) = dst_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&src_path, &dst_path).await?;
        Ok(())
    }

    fn workspace_safe_join(&self, pid: &str, rel: &Path) -> FsResult<PathBuf> {
        validate_pid(pid)?;
        let workspace = self.root.join("projects").join(pid).join("workspace");
        safe_join(&workspace, rel)
    }
}

fn validate_pid(pid: &str) -> FsResult<()> {
    if pid.is_empty() {
        return Err(FsError::InvalidPath("pid must not be empty".into()));
    }
    if pid.contains('/') || pid.contains('\\') || pid.contains('\0') {
        return Err(FsError::InvalidPath(format!(
            "pid contains illegal char: {pid}"
        )));
    }
    if pid == "." || pid == ".." {
        return Err(FsError::InvalidPath("pid must not be '.' or '..'".into()));
    }
    Ok(())
}

/// Normalize and join `rel` onto `root`, rejecting empty paths, NUL bytes,
/// `..`, absolute roots, and Windows drive prefixes.
fn safe_join(root: &Path, rel: &Path) -> FsResult<PathBuf> {
    if rel.as_os_str().is_empty() {
        return Err(FsError::InvalidPath("path must not be empty".into()));
    }
    if rel.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(FsError::InvalidPath(
            "path must not contain NUL bytes".into(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::ParentDir => {
                return Err(FsError::InvalidPath(
                    "path must not contain '..' segments".into(),
                ));
            }
            Component::RootDir => {
                return Err(FsError::InvalidPath("path must not be absolute".into()));
            }
            Component::Prefix(_) => {
                return Err(FsError::InvalidPath(
                    "path must not contain a drive prefix".into(),
                ));
            }
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(FsError::InvalidPath(
            "path is empty after normalization".into(),
        ));
    }
    Ok(root.join(normalized))
}

/// Atomic write via tmp-file + rename. On any error the partial tmp file is
/// removed and `path` is left untouched. Caller ensures `path.parent()` exists.
async fn atomic_write<R>(path: &Path, mut reader: R, max_bytes: u64) -> FsResult<u64>
where
    R: AsyncRead + Unpin,
{
    let parent = path
        .parent()
        .ok_or_else(|| FsError::InvalidPath("path has no parent".into()))?;
    // Tmp lives next to the final path so the rename stays on the same filesystem.
    let tmp = parent.join(format!(".tmp.{}", Uuid::new_v4().simple()));

    let result: FsResult<u64> = async {
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
                return Err(FsError::Io(std::io::Error::new(
                    std::io::ErrorKind::FileTooLarge,
                    format!("file exceeds maximum size ({max_bytes} bytes)"),
                )));
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

/// Recursively copy `src` to `dst`. Iterative (no async recursion); symlinks
/// are skipped (not used by this layer's writers).
async fn copy_recursive(src: &Path, dst: &Path) -> FsResult<()> {
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const MAX: u64 = 1024 * 1024;

    fn fresh_fs() -> (tempfile::TempDir, FSStateV2) {
        let tmp = tempfile::tempdir().unwrap();
        let fs = FSStateV2::new(tmp.path());
        (tmp, fs)
    }

    async fn fresh_fs_with_project() -> (tempfile::TempDir, FSStateV2) {
        let (tmp, fs) = fresh_fs();
        fs.create_project("pid").await.unwrap();
        (tmp, fs)
    }

    fn ws(tmp: &tempfile::TempDir, rel: &str) -> PathBuf {
        tmp.path()
            .join("projects")
            .join("pid")
            .join("workspace")
            .join(rel)
    }

    #[tokio::test]
    async fn create_makes_project_subdirs() {
        let (tmp, fs) = fresh_fs();
        fs.create_project("pid").await.unwrap();
        let dir = tmp.path().join("projects").join("pid");
        assert!(dir.is_dir());
        assert!(dir.join("workspace").is_dir());
        assert!(dir.join("sessions").is_dir());
    }

    #[tokio::test]
    async fn create_twice_fails() {
        let (_tmp, fs) = fresh_fs();
        fs.create_project("pid").await.unwrap();
        let err = fs.create_project("pid").await.unwrap_err();
        match err {
            FsError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists),
            other => panic!("expected Io(AlreadyExists), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let (tmp, fs) = fresh_fs();
        fs.create_project("pid").await.unwrap();
        assert!(fs.delete_project("pid").await.unwrap());
        assert!(!tmp.path().join("projects").join("pid").exists());
        assert!(!fs.delete_project("pid").await.unwrap());
    }

    #[tokio::test]
    async fn invalid_pid_rejected() {
        let (_tmp, fs) = fresh_fs();
        for bad in ["", "../escape", "a/b", "a\\b", ".", ".."] {
            let err = fs.create_project(bad).await.unwrap_err();
            assert!(
                matches!(err, FsError::InvalidPath(_)),
                "expected InvalidPath for {bad:?}, got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn insert_workspace_dir_creates_nested_and_is_idempotent() {
        let (tmp, fs) = fresh_fs_with_project().await;
        fs.insert_workspace_dir("pid", "a/b/c").await.unwrap();
        assert!(ws(&tmp, "a/b/c").is_dir());
        // Idempotent — second call OK.
        fs.insert_workspace_dir("pid", "a/b/c").await.unwrap();
    }

    #[tokio::test]
    async fn insert_workspace_file_writes_and_overwrites() {
        let (tmp, fs) = fresh_fs_with_project().await;
        let n = fs
            .insert_workspace_file("pid", "out/r.txt", Cursor::new(b"hello".to_vec()), MAX)
            .await
            .unwrap();
        assert_eq!(n, 5);
        assert_eq!(
            tokio::fs::read(ws(&tmp, "out/r.txt")).await.unwrap(),
            b"hello"
        );

        // Overwrites existing.
        fs.insert_workspace_file("pid", "out/r.txt", Cursor::new(b"world!".to_vec()), MAX)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(ws(&tmp, "out/r.txt")).await.unwrap(),
            b"world!"
        );
    }

    #[tokio::test]
    async fn insert_workspace_file_enforces_size_cap() {
        let (tmp, fs) = fresh_fs_with_project().await;
        let err = fs
            .insert_workspace_file("pid", "big.bin", Cursor::new(vec![0u8; 10]), 5)
            .await
            .unwrap_err();
        match err {
            FsError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::FileTooLarge),
            other => panic!("expected Io(FileTooLarge), got {other:?}"),
        }
        // Rolled back — no leftover file, no stray tmp.
        let workspace = tmp.path().join("projects/pid/workspace");
        let mut entries = tokio::fs::read_dir(&workspace).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_workspace_entry_handles_both_kinds() {
        let (tmp, fs) = fresh_fs_with_project().await;
        fs.insert_workspace_file("pid", "a.txt", Cursor::new(b"a".to_vec()), MAX)
            .await
            .unwrap();
        fs.insert_workspace_dir("pid", "sub").await.unwrap();
        fs.insert_workspace_file("pid", "sub/b.txt", Cursor::new(b"b".to_vec()), MAX)
            .await
            .unwrap();

        assert!(fs.remove_workspace_entry("pid", "a.txt").await.unwrap());
        assert!(!ws(&tmp, "a.txt").exists());

        assert!(fs.remove_workspace_entry("pid", "sub").await.unwrap());
        assert!(!ws(&tmp, "sub").exists());

        // Idempotent on missing.
        assert!(!fs.remove_workspace_entry("pid", "gone").await.unwrap());
    }

    #[tokio::test]
    async fn copy_workspace_entry_is_recursive() {
        let (tmp, fs) = fresh_fs_with_project().await;
        fs.insert_workspace_file("pid", "src/a.txt", Cursor::new(b"a".to_vec()), MAX)
            .await
            .unwrap();
        fs.insert_workspace_file("pid", "src/sub/b.txt", Cursor::new(b"b".to_vec()), MAX)
            .await
            .unwrap();

        fs.copy_workspace_entry("pid", "src", "dst").await.unwrap();
        assert_eq!(tokio::fs::read(ws(&tmp, "dst/a.txt")).await.unwrap(), b"a");
        assert_eq!(
            tokio::fs::read(ws(&tmp, "dst/sub/b.txt")).await.unwrap(),
            b"b"
        );
        // Original still there.
        assert!(ws(&tmp, "src/a.txt").exists());
    }

    #[tokio::test]
    async fn move_workspace_entry_renames() {
        let (tmp, fs) = fresh_fs_with_project().await;
        fs.insert_workspace_file("pid", "a.txt", Cursor::new(b"a".to_vec()), MAX)
            .await
            .unwrap();
        fs.move_workspace_entry("pid", "a.txt", "sub/b.txt")
            .await
            .unwrap();
        assert!(!ws(&tmp, "a.txt").exists());
        assert_eq!(tokio::fs::read(ws(&tmp, "sub/b.txt")).await.unwrap(), b"a");
    }

    #[tokio::test]
    async fn workspace_rejects_unsafe_rel_paths() {
        let (_tmp, fs) = fresh_fs_with_project().await;
        for bad in ["", "../escape", "/abs", "."] {
            let err = fs.insert_workspace_dir("pid", bad).await.unwrap_err();
            assert!(
                matches!(err, FsError::InvalidPath(_)),
                "expected InvalidPath for {bad:?}, got {err:?}"
            );
        }
    }
}
