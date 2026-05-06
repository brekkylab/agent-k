use std::path::{Path, PathBuf};

use ailoy::runenv::RunEnv;
use async_trait::async_trait;

use crate::{Dir, DirSource};

pub struct RawDir {
    root: PathBuf,
}

impl RawDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, filepath: &Path) -> PathBuf {
        let rel = filepath.strip_prefix("/").unwrap_or(filepath);
        self.root.join(rel)
    }
}

#[async_trait]
impl Dir for RawDir {
    async fn ingest(
        &mut self,
        runenv: &dyn RunEnv,
        filepath: &Path,
        source: &dyn DirSource,
    ) -> anyhow::Result<()> {
        let bytes = source
            .read(filepath)
            .await
            .ok_or_else(|| anyhow::anyhow!("source read failed: {}", filepath.display()))?;
        runenv.write(&self.resolve(filepath), &bytes).await
    }

    async fn read(
        &self,
        runenv: &dyn RunEnv,
        filepath: &Path,
        offset: usize,
        len: usize,
    ) -> anyhow::Result<Vec<u8>> {
        let bytes = runenv.read(&self.resolve(filepath)).await?;
        let start = offset.min(bytes.len());
        let end = start.saturating_add(len).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }
}
