use std::path::{Path, PathBuf};

use ailoy::runenv::RunEnv;
use async_trait::async_trait;

use crate::{Dir, DirRetrieve, DirSource, Dirent, FileType};

/// Advanced directory that converts ingested documents (PDF / Markdown / Text)
/// to markdown via docling and indexes them with tantivy for full-text search.
/// Non-document files are silently ignored.
pub struct KnowledgeBaseDir {
    root: PathBuf,
}

impl KnowledgeBaseDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl Dir for KnowledgeBaseDir {
    async fn ingest(
        &mut self,
        _runenv: &dyn RunEnv,
        filepath: &Path,
        source: &dyn DirSource,
    ) -> anyhow::Result<()> {
        let dirent = source
            .list(filepath)
            .await
            .ok_or_else(|| anyhow::anyhow!("source list failed: {}", filepath.display()))?;
        let ftype = match dirent {
            Dirent::File { ftype, .. } => ftype,
            Dirent::Dir { .. } => return Ok(()),
        };
        match ftype {
            FileType::PDF | FileType::Markdown | FileType::Text => {
                // TODO: read bytes via `source`, convert PDF/Markdown/Text → markdown
                // with docling, write the converted markdown under `self.root` on
                // `runenv`, and add it to a tantivy index.
                todo!("docling conversion + tantivy indexing")
            }
            FileType::Raw => Ok(()),
        }
    }

    async fn read(
        &self,
        _runenv: &dyn RunEnv,
        _filepath: &Path,
        _offset: usize,
        _len: usize,
    ) -> anyhow::Result<Vec<u8>> {
        // TODO: serve the converted markdown stored under `self.root` on `runenv`.
        todo!()
    }
}

#[async_trait]
impl DirRetrieve for KnowledgeBaseDir {
    async fn retrieve(&self, _query: &str) -> anyhow::Result<()> {
        // TODO: tantivy full-text search over the ingested markdown.
        todo!()
    }
}
