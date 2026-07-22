mod base;
mod gmail;
mod local;
mod notion;
mod s3;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub(crate) use base::SEARCH_DIR;
pub use gmail::GmailResource;
pub use local::LocalResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
