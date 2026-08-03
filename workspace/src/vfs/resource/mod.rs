mod base;
mod local;
mod notion;
mod s3;
mod slack;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub use local::LocalResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
pub use slack::SlackResource;
