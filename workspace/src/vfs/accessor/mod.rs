//! Provider accessors: each holds its config (credentials/endpoint) and the
//! client built from it.
//!
//! Config structs intentionally do not derive `Debug` to avoid leaking
//! credentials into logs. They stay host-only.
//!
//! A config holds only what belongs to *that* mount. Anything the deployment owns —
//! the Google OAuth client, service origins — is passed in at construction instead,
//! so it cannot end up duplicated into every stored row (see [`GoogleClient`]).

mod gdrive;
mod gmail;
mod google;
mod notion;
mod s3;

pub use gdrive::{GdriveAccessor, GdriveConfig, GdriveExchange, exchange_gdrive_code};
pub use gmail::{GmailAccessor, GmailConfig, GmailExchange, encode_b64url, exchange_gmail_code};
pub use google::{GoogleClient, Origins};
pub use notion::{NotionAccessor, NotionConfig};
pub use s3::{S3Accessor, S3Config};
