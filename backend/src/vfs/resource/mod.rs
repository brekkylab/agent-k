mod base;
mod gmail;
mod notion;
mod s3;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub use gmail::GmailResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
