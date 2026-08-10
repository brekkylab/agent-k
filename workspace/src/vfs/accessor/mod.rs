//! Provider accessors: each holds its config (credentials/endpoint) and the
//! client built from it.
//!
//! Config structs intentionally do not derive `Debug` to avoid leaking
//! credentials into logs. They stay host-only.

mod gdrive;
mod gmail;
mod google;
mod notion;
mod s3;
mod slack;

pub use gdrive::{GdriveAccessor, GdriveConfig, GdriveExchange, exchange_gdrive_code};
pub use gmail::{GmailAccessor, GmailConfig, GmailExchange, encode_b64url, exchange_gmail_code};
pub use google::Origins;
pub use notion::{NotionAccessor, NotionConfig};
pub use s3::{S3Accessor, S3Config};
pub use slack::{
    SlackAccessor, SlackApiError, SlackConfig, SlackExchange, exchange_slack_code,
    is_missing_scope, is_read_denied,
};
