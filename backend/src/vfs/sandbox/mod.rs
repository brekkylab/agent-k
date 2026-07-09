//! Sandbox VFS frontend — exposes the [`Vfs`](crate::vfs::Vfs) core to an
//! in-guest FUSE forwarder so a sandboxed agent can read mounts as real files.
//!
//! Vendored/adapted from ailoy's `src/vfs/sandbox/` (61c4c43). This first step
//! ports only the host-side forward server ([`VfsForward`]) — a tiny HTTP server
//! with **no microsandbox coupling**, hence no environment risk. The guest
//! binary injection + session run-loop wiring (ailoy's `guest.rs` / `AgentVfs`)
//! land in later steps and must be adapted to agent-k's ailoy `Console` API.
mod forward;
mod fwdfs;
mod guest;

pub use forward::VfsForward;
pub use fwdfs::{ForwardFs, FwdEntry, FwdStat};
pub(crate) use fwdfs::secs_since_epoch;
pub use guest::{forwarder_available, mount_vfs_in_guest};

#[cfg(test)]
mod e2e {
    //! In-VM checks for the unified FUSE mount. Boots a sandbox VM, injects the
    //! cross-built in-guest FUSE forwarder, mounts the unified workspace tree at
    //! `/mnt/workspace`, and reads a local file + real Notion `page.json` from
    //! inside the guest with `cat` — exactly how the agent reads them. Ignored
    //! by default; needs the forwarder ELF built, a working microsandbox, and
    //! (for the Notion read) `NOTION_API_KEY`:
    //!
    //!   NOTION_API_KEY=ntn_… cargo test -p agent-k-backend unified_workspace_readable_in_guest -- --ignored --nocapture
    use std::path::Path;
    use std::sync::Arc;

    use ailoy::runenv::{Console as _, Machine as _, SandboxBuilder, SandboxNetwork};

    use crate::vfs::{MountSpec, NotionConfig, ProviderConfig, Vfs, VfsConfig, VfsForward};

    // Multi-thread runtime so the host forward server's accept loop is driven on
    // its own worker while the test awaits long-running guest execs.
    /// Networking diagnostic: which guest-visible address routes to the host
    /// forward server on microsandbox 0.5.5? (The forwarder resolved
    /// host.microsandbox.internal → 172.16.0.13 but connect refused fast.) No
    /// forwarder binary / Notion key needed. Run:
    ///   cargo test -p agent-k-backend guest_host_connectivity -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "boots a VM to probe guest->host connectivity"]
    async fn guest_host_connectivity() {
        if std::env::var_os("MSB_HOME").is_none() {
            if let Some(home) = std::env::var_os("HOME") {
                let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
                unsafe { std::env::set_var("MSB_HOME", &d) };
            }
        }

        // A forward server over an empty workspace is enough — we only probe reachability.
        let data_root = tempfile::tempdir().expect("tempdir");
        let fs: Arc<dyn crate::vfs::ForwardFs> =
            Arc::new(crate::state::workspace_fs(data_root.path(), uuid::Uuid::new_v4(), None));
        let fwd = VfsForward::spawn(fs, &tokio::runtime::Handle::current()).expect("spawn");
        let port = fwd.port();
        println!("host forward server on port {port}");

        let mut sandbox = SandboxBuilder::new()
            .image("brekkylab/agent-k-libreoffice:latest")
            .cpus(2)
            .memory_mib(1024)
            // Open guest->host egress so the forwarder can reach the host
            // forward server at host.microsandbox.internal.
            .network(SandboxNetwork::Public)
            .build()
            .await
            .expect("build sandbox");
        let console = sandbox.start().await.expect("start VM");

        // .replace() (not format!) so awk's `{ }` don't collide with format args.
        let script = PROBE_SCRIPT.replace("__PORT__", &port.to_string());
        let r = console.exec_shell(script, Some(60)).await.expect("exec probe");
        println!("--- guest network probe (exit {}) ---\n{}", r.exit_code, r.stdout);
        if !r.stderr.trim().is_empty() {
            println!("stderr:\n{}", r.stderr);
        }
        let _ = sandbox.stop().await;
    }

    const PROBE_SCRIPT: &str = r#"
echo "=== /etc/hosts ==="; cat /etc/hosts 2>&1
echo "=== default gateway ==="; ip route 2>/dev/null | awk '/default/{print $3}'
echo "=== ipv4 addrs ==="; ip -4 addr 2>/dev/null | grep inet
echo "=== getent host.microsandbox.internal ==="; getent hosts host.microsandbox.internal 2>&1
echo "=== bash present ==="; command -v bash || echo NO-BASH
echo "=== connect probes to port __PORT__ ==="
GW=$(ip route 2>/dev/null | awk '/default/{print $3}')
HMI=$(getent hosts host.microsandbox.internal 2>/dev/null | awk '{print $1}')
for A in "$HMI" "$GW" 172.16.0.1 10.0.2.2 192.168.127.254 192.168.127.1; do
  [ -z "$A" ] && continue
  if command -v bash >/dev/null 2>&1; then
    if timeout 3 bash -c "exec 3<>/dev/tcp/$A/__PORT__" >/dev/null 2>&1; then
      echo "CONNECT OK   -> $A:__PORT__"
    else
      echo "CONNECT FAIL -> $A:__PORT__"
    fi
  fi
done
"#;

    /// Does the network policy survive archive → try_from_archive → start?
    /// `create_session` builds + archives the sandbox and `run()` restores it, so
    /// the host-egress policy must persist across that round-trip for the
    /// forwarder to reach the host at run time. If this fails, the policy must be
    /// (re)applied at run-time start, not only at create. Run:
    ///   cargo test -p agent-k-backend host_egress_survives_archive -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "boots a VM, archives + restores it, probes host connectivity"]
    async fn host_egress_survives_archive() {
        if std::env::var_os("MSB_HOME").is_none() {
            if let Some(home) = std::env::var_os("HOME") {
                let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
                unsafe { std::env::set_var("MSB_HOME", &d) };
            }
        }
        // A forward server over an empty workspace is enough — we only probe reachability.
        let data_root = tempfile::tempdir().expect("tempdir");
        let fs: Arc<dyn crate::vfs::ForwardFs> =
            Arc::new(crate::state::workspace_fs(data_root.path(), uuid::Uuid::new_v4(), None));
        let fwd = VfsForward::spawn(fs, &tokio::runtime::Handle::current()).expect("spawn");
        let port = fwd.port();

        // Build WITH host egress, start once, stop, archive, drop.
        let archive = std::env::temp_dir().join("agentk-egress-archive.tar.zst");
        if archive.exists() {
            let _ = std::fs::remove_file(&archive);
        }
        {
            let mut sb = SandboxBuilder::new()
                .image("brekkylab/agent-k-libreoffice:latest")
                .cpus(2)
                .memory_mib(1024)
                .network(SandboxNetwork::Public)
                .build()
                .await
                .expect("build");
            sb.start().await.expect("start (fresh)");
            sb.stop().await.expect("stop");
            sb.archive(&archive).await.expect("archive");
        }

        // Restore WITH host egress re-applied (the archive doesn't carry the
        // policy), start, and probe host reachability from the guest.
        let mut restored = ailoy::runenv::Sandbox::try_from_archive_with_network(&archive, SandboxNetwork::Public)
            .await
            .expect("restore from archive");
        let console = restored.start().await.expect("start (restored)");
        let script = format!(
            "if command -v bash >/dev/null 2>&1 && timeout 3 bash -c 'exec 3<>/dev/tcp/host.microsandbox.internal/{port}' >/dev/null 2>&1; then echo REACHABLE; else echo UNREACHABLE; fi"
        );
        let r = console.exec_shell(script, Some(30)).await.expect("probe");
        println!("archived+restored host reachability: {}", r.stdout.trim());
        let _ = restored.stop().await;
        let _ = std::fs::remove_file(&archive);
        assert!(
            r.stdout.contains("REACHABLE"),
            "host-egress did NOT survive archive/restore — apply the policy at run-time start"
        );
    }

    /// Option C end-to-end: the **unified** workspace mount. Boots a VM and,
    /// via the production [`mount_vfs_in_guest`] helper over a `WorkspaceFs`,
    /// mounts the unified tree at `/mnt/workspace`, then proves the guest reads
    /// BOTH a local workspace file AND a real Notion `page.json` under one mount
    /// — the same tree the browser sees over WebDAV. Ignored by default; needs a
    /// Notion token, the forwarder ELF built (build.rs), and working microsandbox:
    ///
    ///   NOTION_API_KEY=ntn_… cargo test -p agent-k-backend unified_workspace_readable_in_guest -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "boots a VM + mounts unified FUSE + reads local file & real Notion (needs NOTION_API_KEY, built forwarder, working microsandbox)"]
    async fn unified_workspace_readable_in_guest() {
        use uuid::Uuid;

        let api_key =
            std::env::var("NOTION_API_KEY").expect("set NOTION_API_KEY to run this e2e check");

        if std::env::var_os("MSB_HOME").is_none() {
            if let Some(home) = std::env::var_os("HOME") {
                let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
                unsafe { std::env::set_var("MSB_HOME", &d) };
            }
        }
        assert!(
            super::forwarder_available(),
            "forwarder ELF not built into this binary; see backend/build.rs"
        );

        // A workspace file tree on disk (data_root/workspaces/{wid}/files) with a
        // local file, plus a Notion mount — assembled into the unified WorkspaceFs.
        let data_root = tempfile::tempdir().unwrap();
        let wid = Uuid::new_v4();
        let files = data_root
            .path()
            .join("workspaces")
            .join(wid.to_string())
            .join("files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("notes.txt"), b"unified-local-marker").unwrap();

        let vfs = Arc::new(
            Vfs::from_config(VfsConfig {
                mounts: vec![MountSpec {
                    prefix: "/notion".into(),
                    provider: ProviderConfig::Notion(NotionConfig { api_key }),
                }],
            })
            .expect("build vfs"),
        );
        // Pick a real page dir to target (host-side lookup on the raw Vfs).
        let (res, vp) = vfs.route("/notion/pages").expect("route /notion/pages");
        let entries = res.readdir(&vp).await.expect("readdir pages");
        let page = entries
            .first()
            .expect("no Notion pages shared with this integration")
            .name
            .clone();
        println!("target page dir: {page}");

        let ws_fs =
            crate::state::workspace_fs(data_root.path(), wid, Some(vfs.clone()));
        let unified: Arc<dyn crate::vfs::ForwardFs> = Arc::new(ws_fs);

        let mut sandbox = SandboxBuilder::new()
            .image("brekkylab/agent-k-libreoffice:latest")
            .cpus(2)
            .memory_mib(1024)
            .network(SandboxNetwork::Public)
            .build()
            .await
            .expect("build sandbox");
        let console = sandbox.start().await.expect("start VM");

        let _fwd = super::mount_vfs_in_guest(console, unified, "/mnt/workspace")
            .await
            .expect("mount unified workspace in guest");
        println!("mounted unified tree at /mnt/workspace");

        // The unified tree lists the reserved `files/` dir alongside the mount.
        let ls = console
            .exec_shell("ls -la /mnt/workspace 2>&1".to_string(), Some(30))
            .await
            .expect("ls");
        println!("--- guest: ls /mnt/workspace (exit {}) ---\n{}", ls.exit_code, ls.stdout);

        // Local workspace file (under files/), read from inside the guest via FUSE.
        let local = console
            .exec_shell("cat /mnt/workspace/files/notes.txt 2>&1".to_string(), Some(30))
            .await
            .expect("cat local");
        println!("--- guest: cat files/notes.txt (exit {}) ---\n{}", local.exit_code, local.stdout);

        // Notion page, under the same mount.
        let notion = console
            .exec_shell(
                format!("cat '/mnt/workspace/notion/pages/{page}/page.json' 2>&1"),
                Some(30),
            )
            .await
            .expect("cat notion");
        println!(
            "--- guest: cat notion page.json (exit {}) ---\n{}",
            notion.exit_code,
            &notion.stdout[..notion.stdout.len().min(1200)]
        );

        let _ = sandbox.stop().await;

        assert_eq!(local.exit_code, 0, "guest failed to read local file: {}", local.stderr);
        assert!(
            local.stdout.contains("unified-local-marker"),
            "guest did not read the local workspace file through the unified mount"
        );
        assert_eq!(notion.exit_code, 0, "guest failed to read notion page: {}", notion.stderr);
        assert!(
            notion.stdout.contains("\"page_id\""),
            "guest did not read a Notion page.json through the unified mount"
        );
    }

}
