mod r#impl;

use std::path::Path;

use ailoy::runenv::RunEnv;
use async_trait::async_trait;

use crate::DirSource;

pub use r#impl::*;

#[async_trait]
pub trait Dir: Send + Sync {
    async fn ingest(
        &mut self,
        runenv: &dyn RunEnv,
        filepath: &Path,
        source: &dyn DirSource,
    ) -> anyhow::Result<()>;

    async fn read(
        &self,
        runenv: &dyn RunEnv,
        filepath: &Path,
        offset: usize,
        len: usize,
    ) -> anyhow::Result<Vec<u8>>;
}

#[async_trait]
pub trait DirRetrieve: Dir {
    async fn retrieve(&self, query: &str) -> anyhow::Result<()>;
}
