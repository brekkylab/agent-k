//! Sandbox VFS frontend:
//! - [`ForwardFs`] — the filesystem surface (the unified `WorkspaceFs`: local
//!   files + provider mounts) served to an in-guest FUSE client.
//! - [`TunnelServer`] — the host-side raw-FUSE engine that answers a guest's
//!   tunneled `/dev/fuse` traffic against a `ForwardFs`.
//!
//! The guest-side injection (ailoy `Console` wiring + the pump ELF) lives in the
//! backend, which starts a [`TunnelServer`] from here — this crate carries no
//! microsandbox coupling.
mod fwdfs;
mod tunnel;

pub(crate) use fwdfs::secs_since_epoch;
pub use fwdfs::{ForwardFs, FwdEntry, FwdStat};
pub use tunnel::TunnelServer;
