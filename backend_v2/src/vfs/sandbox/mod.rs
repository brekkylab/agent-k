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
    //! Step 3/4 — the a1 goal check. Boots a sandbox VM, injects the
    //! cross-built in-guest FUSE forwarder, mounts the Notion-backed VFS, and
    //! reads a real Notion `page.json` **from inside the guest with `cat`** —
    //! i.e. exactly how the agent would read it. Ignored by default; needs a
    //! Notion token, the forwarder ELF built (Step 2), and a host where
    //! microsandbox runs:
    //!
    //!   cargo test -p agent-k-backend notion_readable_in_guest -- --ignored --nocapture
    use std::path::Path;
    use std::sync::Arc;

    use ailoy::runenv::{Console as _, Machine as _, SandboxBuilder, VolumeMount};

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

        // A forward server over an empty Vfs is enough — we only test reachability.
        let vfs = Arc::new(Vfs::new(vec![]).expect("empty vfs"));
        let fwd = VfsForward::spawn(vfs, &tokio::runtime::Handle::current()).expect("spawn");
        let port = fwd.port();
        println!("host forward server on port {port}");

        let mut sandbox = SandboxBuilder::new()
            .image("brekkylab/agent-k-libreoffice:latest")
            .cpus(2)
            .memory_mib(1024)
            // Open guest->host egress so the forwarder can reach the host
            // forward server at host.microsandbox.internal.
            .allow_host_egress(true)
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

    /// Does `allow_host_egress` survive archive → try_from_archive → start?
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
        let vfs = Arc::new(Vfs::new(vec![]).expect("empty vfs"));
        let fwd = VfsForward::spawn(vfs, &tokio::runtime::Handle::current()).expect("spawn");
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
                .allow_host_egress(true)
                .build()
                .await
                .expect("build");
            sb.start().await.expect("start (fresh)");
            sb.stop().await.expect("stop");
            sb.archive(&archive).await.expect("archive");
        }

        // Restore WITH host egress re-applied (the archive doesn't carry the
        // policy), start, and probe host reachability from the guest.
        let mut restored = ailoy::runenv::Sandbox::try_from_archive_with_host_egress(&archive)
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

    /// Workspace local `files/` reach the guest via a plain `VolumeMount::Bind`
    /// (the mechanism `create_session` uses at `/workspace/files`). Verifies read
    /// and host-visible write-back. Run:
    ///   cargo test -p agent-k-backend workspace_files_bind_mount -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "boots a VM; verifies a host dir bind-mounts into the guest"]
    async fn workspace_files_bind_mount() {
        if std::env::var_os("MSB_HOME").is_none() {
            if let Some(home) = std::env::var_os("HOME") {
                let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
                unsafe { std::env::set_var("MSB_HOME", &d) };
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), b"workspace-file-content").unwrap();

        let mut sandbox = SandboxBuilder::new()
            .image("brekkylab/agent-k-libreoffice:latest")
            .cpus(2)
            .memory_mib(1024)
            .mount(VolumeMount::Bind {
                host: tmp.path().to_path_buf(),
                guest: "/workspace/files".to_string(),
                readonly: false,
            })
            .build()
            .await
            .expect("build sandbox");
        let console = sandbox.start().await.expect("start VM");

        // Read a host file from inside the guest.
        let r = console
            .exec_shell("cat /workspace/files/hello.txt".to_string(), Some(30))
            .await
            .expect("cat");
        println!("guest cat /workspace/files/hello.txt -> {}", r.stdout.trim());
        assert_eq!(r.exit_code, 0, "guest cat failed: {}", r.stderr);
        assert!(r.stdout.contains("workspace-file-content"));

        // Write from the guest; it must appear back on the host.
        let w = console
            .exec_shell(
                "echo from-guest > /workspace/files/guest.txt".to_string(),
                Some(30),
            )
            .await
            .expect("write");
        assert_eq!(w.exit_code, 0, "guest write failed: {}", w.stderr);

        let _ = sandbox.stop().await;
        assert!(
            tmp.path().join("guest.txt").exists(),
            "guest write should appear on the host bind source"
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
            "forwarder ELF not built into this binary; see backend_v2/build.rs"
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
            .allow_host_egress(true)
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "boots a VM + mounts FUSE + reads real Notion (needs NOTION_API_KEY, built forwarder, working microsandbox)"]
    async fn notion_readable_in_guest() {
        let api_key =
            std::env::var("NOTION_API_KEY").expect("set NOTION_API_KEY to run this e2e check");

        // The forwarder ELF cross-built in Step 2 (aarch64-unknown-linux-musl).
        let bin_path = format!(
            "{}/../crates/ailoy-vfs-forwarder/target/aarch64-unknown-linux-musl/release/ailoy-vfs-fwd",
            env!("CARGO_MANIFEST_DIR")
        );
        let fwd_bin = std::fs::read(&bin_path).unwrap_or_else(|e| {
            panic!("forwarder ELF missing ({bin_path}): {e}\nbuild it (Step 2) first")
        });
        println!("forwarder ELF: {} bytes", fwd_bin.len());

        // Run agent-k's microsandbox 0.5.5 in its own home so it can't collide
        // with a newer microsandbox's state DB on this host.
        if std::env::var_os("MSB_HOME").is_none() {
            if let Some(home) = std::env::var_os("HOME") {
                let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
                // SAFETY: set once at test start, before any microsandbox call.
                unsafe { std::env::set_var("MSB_HOME", &d) };
            }
        }

        // Host forward server over a Notion-mounted Vfs.
        let vfs = Arc::new(
            Vfs::from_config(VfsConfig {
                mounts: vec![MountSpec {
                    prefix: "/notion".into(),
                    provider: ProviderConfig::Notion(NotionConfig { api_key }),
                }],
            })
            .expect("build vfs"),
        );
        let fwd = VfsForward::spawn(vfs.clone(), &tokio::runtime::Handle::current())
            .expect("spawn forward server");
        let (port, token) = (fwd.port(), fwd.token().to_string());
        println!("host forward server on port {port}");

        // Pick a real page dir to target (host-side lookup).
        let (res, vp) = vfs.route("/notion/pages").expect("route /notion/pages");
        let entries = res.readdir(&vp).await.expect("readdir pages");
        let page = entries
            .first()
            .expect("no Notion pages shared with this integration")
            .name
            .clone();
        println!("target page dir: {page}");

        // Boot the sandbox (network on by default so the guest can reach the host).
        let mut sandbox = SandboxBuilder::new()
            .image("brekkylab/agent-k-libreoffice:latest")
            .cpus(2)
            .memory_mib(1024)
            // Open guest->host egress so the forwarder can reach the host
            // forward server at host.microsandbox.internal.
            .allow_host_egress(true)
            .build()
            .await
            .expect("build sandbox");
        let console = sandbox.start().await.expect("start VM");

        // Inject the forwarder binary into the guest.
        console
            .write(Path::new("/opt/ailoy/vfs-fwd"), &fwd_bin)
            .await
            .expect("write forwarder into guest");

        // Mount it (adapted from ailoy's src/vfs/sandbox/guest.rs). The guest
        // reaches the host forward server at host.microsandbox.internal:<port>.
        let mount_root = "/mnt/vfs";
        let script = format!(
            r#"set -e
mkdir -p /opt/ailoy
umount -l {mount_root} 2>/dev/null || fusermount3 -u {mount_root} 2>/dev/null || true
mkdir -p {mount_root}
chmod +x /opt/ailoy/vfs-fwd
export VFS_HOST="http://host.microsandbox.internal:{port}"
export VFS_TOKEN="{token}"
export VFS_DIAG=1
setsid sh -c '/opt/ailoy/vfs-fwd {mount_root} </dev/null >/tmp/ailoy-vfs.log 2>&1' </dev/null >/dev/null 2>&1 &
for _ in $(seq 1 100); do
  if grep -q " {mount_root} " /proc/mounts 2>/dev/null; then exit 0; fi
  sleep 0.1
done
echo "mount did not appear at {mount_root}" >&2
echo "--- forwarder log ---" >&2
cat /tmp/ailoy-vfs.log >&2 2>/dev/null || true
exit 1
"#
        );
        let m = console.exec_shell(script, Some(60)).await.expect("exec mount");
        if m.exit_code != 0 {
            panic!(
                "forwarder mount failed (exit {}):\nstdout:{}\nstderr:{}",
                m.exit_code, m.stdout, m.stderr
            );
        }
        println!("mounted at {mount_root}");

        // Walk the tree top-down so we can see exactly which level (if any) the
        // guest fails to resolve. Each `ls` triggers FUSE lookups/readdirs that
        // call the host forward server.
        for p in [
            mount_root.to_string(),
            format!("{mount_root}/notion"),
            format!("{mount_root}/notion/pages"),
        ] {
            let r = console
                .exec_shell(format!("ls -la '{p}' 2>&1"), Some(30))
                .await
                .expect("exec ls");
            println!("--- guest: ls {p} (exit {}) ---\n{}", r.exit_code, r.stdout);
        }

        // THE GOAL: the agent reads Notion as a file, from inside the VM.
        let cat = console
            .exec_shell(
                format!("cat '{mount_root}/notion/pages/{page}/page.json' 2>&1"),
                Some(30),
            )
            .await
            .expect("exec cat");
        println!(
            "--- guest: cat page.json (exit {}) ---\n{}",
            cat.exit_code,
            &cat.stdout[..cat.stdout.len().min(1500)]
        );

        // Dump both forwarder logs regardless: the redirected stdout/stderr and
        // the VFS_DIAG trace (they are separate files).
        for logf in ["/tmp/ailoy-vfs.log", "/tmp/ailoy-vfs-fwd.log"] {
            let l = console
                .exec_shell(format!("echo '=== {logf} ==='; tail -80 '{logf}' 2>&1 || true"), Some(30))
                .await
                .expect("exec log");
            println!("{}", l.stdout);
        }

        let _ = sandbox.stop().await;

        assert_eq!(
            cat.exit_code, 0,
            "guest cat failed — see the ls walk and forwarder logs above"
        );
        assert!(
            cat.stdout.contains("\"page_id\""),
            "guest did not read a Notion page.json"
        );
    }
}
