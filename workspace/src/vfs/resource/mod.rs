mod base;
mod gmail;
mod gmail_sync;
mod local;
mod notion;
mod s3;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub use gmail_sync::{GmailSyncState, mirror_tree, sync_gmail_mirror};
pub use gmail::GmailResource;
pub use local::LocalResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
