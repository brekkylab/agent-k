//! Sandbox VFS frontend — the host-side forward server that exposes a
//! [`ForwardFs`] (a provider-only [`Vfs`](crate::vfs::Vfs) or the unified
//! `WorkspaceFs`) over a tiny HTTP/1.1 API for an in-guest FUSE forwarder.
//!
//! Vendored/adapted from ailoy's `src/vfs/sandbox/` (61c4c43). The guest-side
//! injection (ailoy `Console` wiring + the embedded forwarder ELF) lives in the
//! backend, which consumes [`VfsForward`] / [`ForwardFs`] from here — this crate
//! carries no microsandbox coupling.
mod forward;
mod fwdfs;

pub use forward::VfsForward;
pub use fwdfs::{ForwardFs, FwdEntry, FwdStat};
pub(crate) use fwdfs::secs_since_epoch;
