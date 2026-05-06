use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use async_trait::async_trait;

use crate::{Dirent, DirSource, FileType};

pub struct LocalDirSource {
    root: PathBuf,
}

impl LocalDirSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &Path) -> PathBuf {
        let rel = path.strip_prefix("/").unwrap_or(path);
        self.root.join(rel)
    }
}

#[async_trait]
impl DirSource for LocalDirSource {
    async fn list(&self, path: &Path) -> Option<Dirent> {
        let abs = self.resolve(path);
        let meta = tokio::fs::metadata(&abs).await.ok()?;

        let name = abs
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let created_at = meta.created().unwrap_or(SystemTime::UNIX_EPOCH);
        let modified_at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        if meta.is_dir() {
            let mut children = Vec::new();
            let mut rd = tokio::fs::read_dir(&abs).await.ok()?;
            while let Ok(Some(entry)) = rd.next_entry().await {
                let child_rel = path.join(entry.file_name());
                if let Some(d) = self.list(&child_rel).await {
                    children.push(d);
                }
            }
            Some(Dirent::Dir {
                name,
                children,
                created_at,
                modified_at,
            })
        } else if meta.is_file() {
            Some(Dirent::File {
                name,
                ftype: detect_ftype(&abs),
                sz: meta.len() as usize,
                created_at,
                modified_at,
            })
        } else {
            None
        }
    }

    async fn read(&self, path: &Path) -> Option<Vec<u8>> {
        tokio::fs::read(self.resolve(path)).await.ok()
    }
}

fn detect_ftype(path: &Path) -> FileType {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => FileType::PDF,
        Some("md" | "markdown") => FileType::Markdown,
        Some("txt") => FileType::Text,
        _ => FileType::Raw,
    }
}
