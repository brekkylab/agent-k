//! In-guest FUSE forwarder bootstrap — the production counterpart of the e2e
//! test. Injects the cross-built forwarder ELF into a sandbox, starts it, and
//! mounts the VFS inside the guest so the agent reads mounts as real files.
//!
//! Adapted from ailoy's `src/vfs/sandbox/guest.rs`, retargeted onto agent-k's
//! ailoy `Console` API (`write` + `exec_shell`).

use std::path::Path;
use std::sync::Arc;

use ailoy::runenv::Console;

use crate::vfs::{Vfs, VfsForward};

/// Guest path the forwarder binary is written to.
const GUEST_FWD_BIN: &str = "/opt/ailoy/vfs-fwd";

/// The static in-guest FUSE forwarder ELF, cross-compiled for the guest arch by
/// [`build.rs`](../../../build.rs). Empty if the build couldn't produce it (e.g.
/// the `…-linux-musl` target isn't installed) — [`mount_vfs_in_guest`] then
/// errors clearly instead of silently doing nothing.
const FORWARDER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ailoy-vfs-fwd"));

/// Whether an in-guest forwarder ELF was built into this binary.
pub fn forwarder_available() -> bool {
    !FORWARDER_ELF.is_empty()
}

/// Spawn a host forward server for `vfs`, inject + start the in-guest FUSE
/// forwarder in `console`'s sandbox, and mount the VFS at `mount_root` inside
/// the guest. Returns the live [`VfsForward`] — keep it alive as long as the
/// mount is needed; dropping it tears the host server down. Blocks until the
/// mount appears in the guest's `/proc/mounts`.
///
/// The guest reaches the host forward server via
/// `host.microsandbox.internal:<port>`, so the sandbox must have been built with
/// host egress allowed (`SandboxBuilder::allow_host_egress(true)`).
pub async fn mount_vfs_in_guest(
    console: &impl Console,
    vfs: Arc<Vfs>,
    mount_root: &str,
) -> anyhow::Result<VfsForward> {
    if !forwarder_available() {
        anyhow::bail!(
            "in-guest VFS forwarder was not built into this binary \
             (see backend_v2/build.rs; is the <arch>-unknown-linux-musl target installed?)"
        );
    }

    let fwd = VfsForward::spawn(vfs, &tokio::runtime::Handle::current())?;
    console
        .write(Path::new(GUEST_FWD_BIN), FORWARDER_ELF)
        .await?;

    let port = fwd.port();
    let token = fwd.token();
    // Detach any stale mount first (e.g. a dead forwarder left the mountpoint in
    // "Transport endpoint is not connected"), then start the forwarder in its own
    // session so it survives this exec, and wait for the mount to appear.
    let script = format!(
        r#"set -e
mkdir -p /opt/ailoy
umount -l {mount_root} 2>/dev/null || fusermount3 -u {mount_root} 2>/dev/null || true
mkdir -p {mount_root}
chmod +x {GUEST_FWD_BIN}
export VFS_HOST="http://host.microsandbox.internal:{port}"
export VFS_TOKEN="{token}"
setsid sh -c '{GUEST_FWD_BIN} {mount_root} </dev/null >/tmp/ailoy-vfs.log 2>&1' </dev/null >/dev/null 2>&1 &
for _ in $(seq 1 100); do
  if grep -q " {mount_root} " /proc/mounts 2>/dev/null; then exit 0; fi
  sleep 0.1
done
echo "forwarder mount did not appear at {mount_root}" >&2
cat /tmp/ailoy-vfs.log >&2 2>/dev/null || true
exit 1
"#
    );
    let out = console.exec_shell(script, Some(30)).await?;
    if out.exit_code != 0 {
        anyhow::bail!(
            "in-guest forwarder mount failed (exit {}): {}",
            out.exit_code,
            out.stderr.trim()
        );
    }
    Ok(fwd)
}
