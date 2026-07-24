mod base;
mod gmail;
mod gmail_sync;
mod local;
mod notion;
mod s3;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub use gmail::GmailResource;
pub use gmail_sync::{
    GmailSyncDelta, GmailSyncState, mirror_tree, sync_gmail_incremental, sync_gmail_mirror,
};
pub use local::LocalResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
