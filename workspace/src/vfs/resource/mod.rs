mod base;
mod gdrive;
mod github;
mod gmail;
mod gmail_sync;
mod local;
mod notion;
mod s3;
#[cfg(test)]
mod tests;

pub use base::{DirEntry, FileKind, FileStat, Resource};
pub use gdrive::GdriveResource;
pub use github::GithubResource;
pub use gmail::GmailResource;
pub use gmail_sync::{
    GmailSyncDelta, GmailSyncState, account_mirror_dir, mirror_tree, sync_gmail_incremental,
    sync_gmail_mirror,
};
pub use local::LocalResource;
pub use notion::NotionResource;
pub use s3::S3Resource;
