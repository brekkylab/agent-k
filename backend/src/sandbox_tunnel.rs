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

/// Where [`attach_vfs_tunnel_in_guest`] puts the unified workspace tree inside the guest.
pub const GUEST_MOUNT_ROOT: &str = "/mnt/workspace";

/// The system-prompt section that tells the agent the workspace mount exists.
///
/// Deliberately minimal: it names the entry point and the connected sources, and
/// leaves the layout for the agent to discover with `ls`. Spelling out each
/// provider's tree here would go stale every time a provider changes, and the
/// listings are self-describing anyway. What the agent cannot discover on its
/// own is that the tree exists at all (nothing else in the prompt or the tool
/// descriptions points outside the home directory) and that walking it costs
/// network round-trips — so those are the two things this says.
///
/// `sources` are the provider mount names (no leading slash), e.g. `["notion"]`.
pub fn workspace_prompt(sources: &[String]) -> String {
    let connected = if sources.is_empty() {
        "none are connected yet".to_string()
    } else {
        sources.join(", ")
    };
    format!(
        "\n\n## Workspace\n\
         - The user's workspace is mounted at `{GUEST_MOUNT_ROOT}`.\n\
         - `files/` holds the user's own files. Connected sources: {connected}.\n\
         - When the query refers to the user's own files, mail or notes and they are not \
         in the attachment or shared-data directories, look here first.\n\
         - Run `ls` to learn a source's layout instead of guessing paths; entry names \
         are not predictable.\n\
         - Every listing and read is a live API call, so walk the tree one level at a \
         time. Do not run recursive `find` or `grep -r` over `{GUEST_MOUNT_ROOT}`.\n\
         - Treat it as input only. Task outputs still belong in the artifacts directory."
    )
}

/// The static in-guest raw-FUSE tunnel pump ELF, cross-built by
/// [`build.rs`](../../build.rs). Empty if the musl target wasn't available.
const TUNNEL_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ailoy-vfs-tunnel"));

/// Whether a tunnel pump ELF was built into this binary.
pub fn tunnel_available() -> bool {
    !TUNNEL_ELF.is_empty()
}

/// Start the host [`TunnelServer`] for `fs`.
///
/// Separate from [`attach_vfs_tunnel_in_guest`] because the guest can only reach
/// the server if the sandbox's network policy names its port, and that policy is
/// fixed when the sandbox is created. So the order is: start the server, read
/// [`TunnelServer::port`], build or restore the sandbox granting that port, then
/// attach. Keep the returned server alive for the mount's lifetime — dropping it
/// tears the mount down.
pub fn spawn_vfs_tunnel(fs: Arc<dyn ForwardFs>, rt: Handle) -> anyhow::Result<TunnelServer> {
    if !tunnel_available() {
        anyhow::bail!(
            "in-guest VFS tunnel pump was not built into this binary (see backend/build.rs)"
        );
    }
    TunnelServer::spawn(fs, rt)
}

/// Inject + launch the in-guest raw-FUSE pump in `console`'s sandbox and mount
/// `srv`'s filesystem at `mount_root` with the default read mode (readahead).
/// The guest does NO protocol conversion — it relays raw FUSE bytes and the host
/// runs the FUSE engine.
///
/// The sandbox must already allow guest egress to `srv.port()` on the host; see
/// [`spawn_vfs_tunnel`].
pub async fn attach_vfs_tunnel_in_guest(
    console: &impl Console,
    srv: &TunnelServer,
    mount_root: &str,
) -> anyhow::Result<()> {
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
    Ok(())
}

#[cfg(test)]
mod prompt {
    use super::{GUEST_MOUNT_ROOT, workspace_prompt};

    #[test]
    fn names_the_mount_root_and_connected_sources() {
        let p = workspace_prompt(&["notion".to_string(), "gmail".to_string()]);
        assert!(p.contains(GUEST_MOUNT_ROOT), "must name the entry point");
        assert!(
            p.contains("notion, gmail"),
            "must list the connected sources: {p}"
        );
    }

    #[test]
    fn a_workspace_with_no_providers_still_describes_the_mount() {
        // Local files alone are worth pointing at, so this must not read as if
        // the mount were absent.
        let p = workspace_prompt(&[]);
        assert!(p.contains(GUEST_MOUNT_ROOT));
        assert!(p.contains("files/"));
        assert!(p.contains("none are connected yet"));
    }

    #[test]
    fn appends_as_its_own_section() {
        // It is concatenated onto an existing instruction, so it has to open its
        // own block rather than run into the previous line.
        let p = workspace_prompt(&[]);
        assert!(p.starts_with("\n\n## "), "must not run into the prior text");
    }
}
