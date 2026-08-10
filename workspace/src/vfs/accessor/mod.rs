//! Provider accessors: each holds its config (credentials/endpoint) and the
//! client built from it.
//!
//! Config structs intentionally do not derive `Debug` to avoid leaking
//! credentials into logs. They stay host-only.

mod gdrive;
mod github;
mod gmail;
mod google;
mod notion;
mod s3;

pub use gdrive::{GdriveAccessor, GdriveConfig, GdriveExchange, exchange_gdrive_code};
pub use github::{
    DEFAULT_INDEX_CAP, EntryKind, GithubAccessor, GithubConfig, GithubSource, IssueRow,
    MAX_BLOB_BYTES, RepoRow, Tree, TreeEntry,
};
pub use gmail::{GmailAccessor, GmailConfig, GmailExchange, encode_b64url, exchange_gmail_code};
pub use google::Origins;
pub use notion::{NotionAccessor, NotionConfig};
pub use s3::{S3Accessor, S3Config};
