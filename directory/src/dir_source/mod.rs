mod r#impl;

pub use r#impl::*;

use std::{path::Path, time::SystemTime};

use async_trait::async_trait;

pub enum FileType {
    Raw,
    PDF,
    Markdown,
    Text,
}

pub enum Dirent {
    Dir {
        name: String,
        children: Vec<Dirent>,
        created_at: SystemTime,
        modified_at: SystemTime,
    },
    File {
        name: String,
        ftype: FileType,
        sz: usize,
        created_at: SystemTime,
        modified_at: SystemTime,
    },
}

impl Dirent {
    pub fn is_dir(&self) -> bool {
        todo!()
    }

    pub fn is_file(&self) -> bool {
        todo!()
    }
}

#[async_trait]
pub trait DirSource: Send + Sync {
    async fn list(&self, path: &Path) -> Option<Dirent>;

    async fn read(&self, path: &Path) -> Option<Vec<u8>>;
}
