//! In-guest raw-FUSE tunnel bootstrap — injects the cross-built pump ELF into a
//! sandbox and points it at a host-side [`workspace::TunnelServer`], so the
//! workspace's unified VFS is served into the guest at a mount point (see
//! [`crate::state::session`]).
//!
//! The host FUSE engine itself lives in the `workspace` crate
//! ([`workspace::TunnelServer`]); this module is only the guest side (ailoy
//! `Console` wiring + the embedded pump binary), the counterpart of the guest
//! pump `crates/ailoy-vfs-tunnel`.

use std::path::Path;
use std::sync::Arc;

use ailoy::runenv::Console;
use tokio::runtime::Handle;
use workspace::{ForwardFs, TunnelServer};

/// Guest path the tunnel pump binary is written to.
const GUEST_TUNNEL_BIN: &str = "/opt/ailoy/vfs-tunnel";

/// The static in-guest raw-FUSE tunnel pump ELF, cross-built by
/// [`build.rs`](../../build.rs). Empty if the musl target wasn't available.
const TUNNEL_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ailoy-vfs-tunnel"));

/// Whether a tunnel pump ELF was built into this binary.
pub fn tunnel_available() -> bool {
    !TUNNEL_ELF.is_empty()
}

/// Start a host [`TunnelServer`] for `fs`, inject + launch the in-guest raw-FUSE
/// pump in `console`'s sandbox, and mount at `mount_root` with the default read
/// mode (readahead). Returns the live server (keep it alive for the mount's
/// lifetime). The guest does NO protocol conversion — it relays raw FUSE bytes
/// and the host runs the FUSE engine.
pub async fn mount_vfs_tunnel_in_guest(
    console: &impl Console,
    fs: Arc<dyn ForwardFs>,
    mount_root: &str,
    rt: Handle,
) -> anyhow::Result<TunnelServer> {
    if !tunnel_available() {
        anyhow::bail!(
            "in-guest VFS tunnel pump was not built into this binary (see backend/build.rs)"
        );
    }
    let srv = TunnelServer::spawn(fs, rt)?;
    // `write` creates the parent dir (/opt/ailoy) itself. The pump self-daemonizes
    // — it creates the mountpoint, clears any stale mount, mounts, and only then
    // backgrounds itself — so this single `exec` returns exactly when the mount
    // is up (non-zero + a diagnostic if setup failed). No init script, no poll.
    console.write(Path::new(GUEST_TUNNEL_BIN), TUNNEL_ELF).await?;

    let port = srv.port();
    let token = srv.token();
    let script = format!(
        "chmod +x {GUEST_TUNNEL_BIN} && \
         VFS_HOST='http://host.microsandbox.internal:{port}' VFS_TOKEN='{token}' \
         {GUEST_TUNNEL_BIN} {mount_root}"
    );
    let out = console.exec_shell(script, Some(30)).await?;
    if out.exit_code != 0 {
        anyhow::bail!(
            "in-guest tunnel mount failed (exit {}): {}{}",
            out.exit_code,
            out.stdout.trim(),
            out.stderr.trim()
        );
    }
    Ok(srv)
}
