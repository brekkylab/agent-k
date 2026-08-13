//! In-VM checks over the raw-FUSE tunnel: boot a sandbox, mount an S3- or
//! Notion-backed workspace via [`agent_k_backend::sandbox_tunnel`], and read
//! through it from the guest. All ignored by default — each boots a microsandbox
//! VM and needs credentials + the built pump ELF. Run explicitly, e.g.:
//!
//!   AWS_S3_BUCKET=… AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… \
//!     cargo test -p agent-k-backend --test tunnel_e2e s3_read_throughput -- --ignored --nocapture

use std::sync::Arc;

use ailoy::runenv::{Console as _, Machine as _, SandboxBuilder, SandboxNetwork};
use workspace::{
    ForwardFs, FsConfig, MountSpec, NotionConfig, ProviderConfig, S3Config, WorkspaceFs,
};

use agent_k_backend::sandbox_tunnel::{
    attach_vfs_tunnel_in_guest, spawn_vfs_tunnel, tunnel_available,
};

/// Build a provider-only workspace fs (no local files) — enough to serve `/s3`
/// or `/notion` into the guest for these read checks.
fn provider_fs(prefix: &str, provider: ProviderConfig) -> Arc<dyn ForwardFs> {
    let config = FsConfig {
        local_root: None,
        mirror_root: None,
        google_oauth: None,
        mounts: vec![MountSpec {
            prefix: prefix.into(),
            provider,
        }],
    };
    Arc::new(WorkspaceFs::from_config(config).expect("workspace fs"))
}

fn s3_config_from_env() -> Option<S3Config> {
    Some(S3Config {
        bucket: std::env::var("AWS_S3_BUCKET").ok()?,
        region: std::env::var("AWS_DEFAULT_REGION").unwrap_or_else(|_| "us-east-1".into()),
        access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok()?,
        secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok()?,
        endpoint: std::env::var("AWS_S3_ENDPOINT").ok(),
        key_prefix: std::env::var("AWS_S3_KEY_PREFIX").ok(),
    })
}

/// Isolate the pinned microsandbox state from other versions on the host.
fn set_dedicated_msb_home() {
    if std::env::var_os("MSB_HOME").is_none()
        && let Some(home) = std::env::var_os("HOME")
    {
        let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
        // SAFETY: called at test start before the microsandbox runtime spawns.
        unsafe { std::env::set_var("MSB_HOME", &d) };
    }
}

/// `tunnel_port` is the host port the tunnel server is already listening on.
/// The guest can only reach it if the policy names it, and the policy is fixed
/// when the sandbox is created — hence the argument.
fn build_sandbox_builder(tunnel_port: u16) -> SandboxBuilder {
    SandboxBuilder::new()
        .image("brekkylab/agent-k-libreoffice:latest")
        .cpus(2)
        .memory_mib(1024)
        .network(SandboxNetwork::Public.with_host_ports([tunnel_port]))
}

/// Guest shell that times a cold stat, a cold `ls -la`, then 3 reads — each
/// preceded by a cache drop so every read genuinely hits the tunnel (fair for
/// the page-cache/readahead path, which would otherwise serve reads 2+ from
/// RAM). dd reports bytes + rate itself, so a short/failed read shows.
/// `date +%s%N` is Linux-only (the guest); integer ns→ms math, no `bc`.
fn bench_script(file: &str, dir: &str) -> String {
    format!(
        r#"ns() {{ date +%s%N; }}
s=$(ns); timeout 20 stat -c '%s' "{file}" >/dev/null 2>&1; e=$(ns); echo "stat_cold_ms=$(( (e-s)/1000000 ))"
s=$(ns); timeout 20 ls -la "{dir}" >/dev/null 2>&1;        e=$(ns); echo "ls_la_cold_ms=$(( (e-s)/1000000 ))"
for i in 1 2 3; do
  sync; echo 1 > /proc/sys/vm/drop_caches 2>/dev/null || true
  timeout 40 dd if="{file}" of=/dev/null bs=65536 2>&1 | awk -v i=$i '/copied/{{print "read_"i": "$0}}'
done
echo "file_bytes=$(timeout 20 stat -c %s "{file}" 2>/dev/null)"
"#
    )
}

/// Fast tunnel-only debug: mount the raw tunnel, try a couple of ops with short
/// timeouts, and dump the in-guest pump log. Isolates guest-side issues.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM, mounts the raw FUSE tunnel, dumps the guest pump log (needs AWS_S3_* creds, built ELF, microsandbox)"]
async fn tunnel_debug() {
    dotenvy::dotenv().ok();
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");
    let Some(s3) = s3_config_from_env() else {
        eprintln!("set AWS_S3_* to run");
        return;
    };
    let fs = provider_fs("/s3", ProviderConfig::S3(s3));

    let rt = tokio::runtime::Handle::current();
    let srv = spawn_vfs_tunnel(fs.clone(), rt).expect("spawn tunnel server");
    let mut sandbox = build_sandbox_builder(srv.port())
        .build()
        .await
        .expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");
    let mounted = attach_vfs_tunnel_in_guest(console, &srv, "/mnt/tunnel").await;
    println!("mount result: {:?}", mounted.as_ref().map(|_| "ok"));

    // A FUSE op can wedge in uninterruptible D-state (even `timeout` can't kill
    // it), so first fetch the pump log + mount table, touching NO FUSE path —
    // this returns fast and reveals why the pump stopped servicing /dev/fuse.
    let log = console
        .exec_shell(
            "echo '[mounts]'; grep ailoyvfs /proc/mounts 2>&1; \
             echo '[pump log]'; cat /tmp/ailoy-vfs-tunnel.log 2>&1"
                .to_string(),
            Some(20),
        )
        .await
        .expect("log exec");
    println!("\n---- tunnel debug (log only) ----\n{}", log.stdout.trim_end());
    if !log.stderr.trim().is_empty() {
        println!("stderr: {}", log.stderr.trim_end());
    }

    let ls = console
        .exec_shell(
            "timeout 15 ls -la /mnt/tunnel 2>&1; echo \"ls_exit=$?\"; \
             timeout 15 ls -la /mnt/tunnel/s3 2>&1; echo \"ls_s3_exit=$?\""
                .to_string(),
            Some(40),
        )
        .await;
    match &ls {
        Ok(o) => println!("\n---- tunnel ls ----\n{}", o.stdout.trim_end()),
        Err(e) => println!("\n---- tunnel ls FAILED/timed out: {e} ----"),
    }

    let log2 = console
        .exec_shell("cat /tmp/ailoy-vfs-tunnel.log 2>&1".to_string(), Some(20))
        .await;
    if let Ok(o) = &log2 {
        println!("\n---- pump log after ops ----\n{}", o.stdout.trim_end());
    }

    let _ = sandbox.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM, mounts S3 over the raw-FUSE tunnel, benchmarks read throughput (needs AWS_S3_* creds, built ELF, working microsandbox)"]
async fn s3_read_throughput() {
    dotenvy::dotenv().ok();
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");
    let Some(s3) = s3_config_from_env() else {
        eprintln!("set AWS_S3_BUCKET / AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY to run");
        return;
    };
    let fs = provider_fs("/s3", ProviderConfig::S3(s3));

    // Discover the largest file anywhere under /s3 (bounded BFS), host-side via
    // the same fs the mount serves — robust to whatever the bucket holds.
    let mut queue = vec!["/s3".to_string()];
    let mut best: Option<(String, u64)> = None;
    let mut budget = 50u32;
    while let Some(d) = queue.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(entries) = ForwardFs::readdir(&*fs, &d).await else {
            continue;
        };
        for e in entries {
            let p = format!("{d}/{}", e.name);
            if e.is_dir {
                queue.push(p);
            } else if best.as_ref().is_none_or(|(_, s)| e.size > *s) {
                best = Some((p, e.size));
            }
        }
    }
    let (file_rel, _) = best.expect("no file found under the S3 bucket to benchmark");
    let dir_rel = file_rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("/s3");
    println!("benchmarking against s3:/{file_rel}");

    let rt = tokio::runtime::Handle::current();
    let srv = spawn_vfs_tunnel(fs.clone(), rt).expect("spawn tunnel server");
    let mut sandbox = build_sandbox_builder(srv.port())
        .build()
        .await
        .expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");
    attach_vfs_tunnel_in_guest(console, &srv, "/mnt/ws")
        .await
        .expect("mount tunnel");

    let fpath = format!("/mnt/ws{file_rel}");
    let dpath = format!("/mnt/ws{dir_rel}");
    let r = console
        .exec_shell(bench_script(&fpath, &dpath), Some(200))
        .await
        .expect("bench exec");
    println!("\n==================== S3 read (readahead) ====================");
    println!("{}", r.stdout.trim_end());
    if !r.stderr.trim().is_empty() {
        println!("stderr: {}", r.stderr.trim_end());
    }

    let _ = sandbox.stop().await;
}

/// Notion `page.json` is RENDERED on read and the raw provider reports size 0.
/// Verifies the readahead (page-cache) path still reads the FULL content: the
/// workspace VFS resolves the real length for render-on-read files, so the
/// kernel's getattr sees an accurate size and cached reads aren't clamped short.
/// Reads it cold (fresh fs) through the guest and asserts full bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM, mounts real Notion over the tunnel, reads a page.json (needs NOTION_API_KEY, built ELF, microsandbox)"]
async fn notion_read_full_content() {
    dotenvy::dotenv().ok();
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");
    let Ok(api_key) = std::env::var("NOTION_API_KEY") else {
        eprintln!("set NOTION_API_KEY to run");
        return;
    };
    // Fresh fs (own metadata cache) per use so the guest's getattr is a genuine
    // cold first touch.
    let make_fs = || {
        provider_fs(
            "/notion",
            ProviderConfig::Notion(NotionConfig {
                api_key: api_key.clone(),
            }),
        )
    };

    // Discover the page + true length on a throwaway fs (keeps the mount cold).
    let probe = make_fs();
    let pages = ForwardFs::readdir(&*probe, "/notion/pages")
        .await
        .expect("readdir /notion/pages");
    let page = pages
        .first()
        .expect("no Notion pages shared with this integration")
        .name
        .clone();
    let pj = format!("/notion/pages/{page}/page.json");
    let true_len = ForwardFs::read(&*probe, &pj, None, None)
        .await
        .expect("host read page.json")
        .len();
    println!("page.json true content = {true_len} bytes (Notion renders on read)");

    let rt = tokio::runtime::Handle::current();
    let srv = spawn_vfs_tunnel(make_fs(), rt).expect("spawn tunnel server");
    let mut sandbox = build_sandbox_builder(srv.port())
        .build()
        .await
        .expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");
    attach_vfs_tunnel_in_guest(console, &srv, "/mnt/ws")
        .await
        .expect("mount tunnel");

    // Single-quote the path; Notion dir names are `title__id`.
    let script = format!(
        r#"f='/mnt/ws/notion/pages/{page}/page.json'
echo "kernel_stat_size=$(timeout 15 stat -c %s "$f" 2>&1)"
echo "cat_bytes=$(timeout 20 cat "$f" 2>/dev/null | wc -c)"
echo -n "head: "; timeout 20 head -c 100 "$f" 2>/dev/null; echo"#
    );
    let r = console
        .exec_shell(script, Some(60))
        .await
        .expect("notion read exec");
    println!("\n==================== NOTION read (readahead) ====================");
    println!("{}", r.stdout.trim_end());
    if !r.stderr.trim().is_empty() {
        println!("stderr: {}", r.stderr.trim_end());
    }

    let _ = sandbox.stop().await;

    let cat_bytes: usize = r
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("cat_bytes="))
        .and_then(|v| v.trim().parse().ok())
        .expect("cat_bytes line");
    assert_eq!(
        cat_bytes, true_len,
        "readahead read {cat_bytes} bytes but page.json is {true_len} — clamped short?"
    );
}

/// The mount works with exactly one host port granted — the tunnel server's.
///
/// Needs no provider credentials: a local-files workspace is enough to prove the
/// path end to end. This is the check that the spawn-then-grant-then-attach order
/// actually produces a working mount, rather than only compiling.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM and mounts the raw FUSE tunnel (needs the built pump ELF + microsandbox)"]
async fn local_workspace_mounts_with_only_the_tunnel_port_granted() {
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello.txt"), b"tunnel-ok").expect("write host file");
    let fs: Arc<dyn ForwardFs> = Arc::new(WorkspaceFs::local(dir.path().to_path_buf()));

    let rt = tokio::runtime::Handle::current();
    let srv = spawn_vfs_tunnel(fs, rt).expect("spawn tunnel server");
    let mut sandbox = build_sandbox_builder(srv.port())
        .build()
        .await
        .expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");
    attach_vfs_tunnel_in_guest(console, &srv, "/mnt/ws")
        .await
        .expect("mount tunnel with the port granted");

    let r = console
        .exec_shell("cat /mnt/ws/files/hello.txt".to_string(), Some(30))
        .await
        .expect("read through the mount");
    let _ = sandbox.stop().await;
    assert_eq!(
        r.stdout.trim(),
        "tunnel-ok",
        "guest could not read the host file through the tunnel: exit={} stderr={}",
        r.exit_code,
        r.stderr.trim()
    );
}

/// The same setup with the port left out must fail to mount. Without this, the
/// test above would still pass if the policy granted every host port again, so
/// this is what pins the grant to one port rather than to "some port".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM and mounts the raw FUSE tunnel (needs the built pump ELF + microsandbox)"]
async fn mount_fails_when_the_tunnel_port_is_not_granted() {
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello.txt"), b"tunnel-ok").expect("write host file");
    let fs: Arc<dyn ForwardFs> = Arc::new(WorkspaceFs::local(dir.path().to_path_buf()));

    let rt = tokio::runtime::Handle::current();
    let srv = spawn_vfs_tunnel(fs, rt).expect("spawn tunnel server");
    // Public egress, no host port. Same as the granted case in every other way.
    let mut sandbox = SandboxBuilder::new()
        .image("brekkylab/agent-k-libreoffice:latest")
        .cpus(2)
        .memory_mib(1024)
        .network(SandboxNetwork::Public)
        .build()
        .await
        .expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");
    let result = attach_vfs_tunnel_in_guest(console, &srv, "/mnt/ws").await;
    let _ = sandbox.stop().await;

    let err = result.expect_err("mount must fail without the host port granted");
    println!("mount refused as expected: {err}");
}
/// Everything above proves the tree is *readable* from the guest. This proves
/// the agent actually *goes there* — that
/// [`workspace_prompt`](agent_k_backend::sandbox_tunnel::workspace_prompt) is
/// enough for the model to discover the mount on its own.
///
/// The query deliberately never says where to look: it asks for a fact that
/// exists only in a file under the mount. Passing means the model reasoned its
/// way from "the user's own notes" to `/mnt/workspace`; failing means it
/// searched the home directory, gave up, or hallucinated. Needs an LLM key on
/// top of the usual VM requirements:
///
///   ANTHROPIC_API_KEY=… cargo test -p agent-k-backend --test tunnel_e2e agent_finds_the_mount_unprompted -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a VM + spends real LLM tokens (needs ANTHROPIC_API_KEY, built pump ELF, working microsandbox)"]
async fn agent_finds_the_mount_unprompted() {
    use agent_k_backend::sandbox_tunnel::{GUEST_MOUNT_ROOT, workspace_prompt};
    use ailoy::{
        agent::{Agent, AgentState},
        message::{Message, Part, Role},
    };
    use futures_util::StreamExt;
    use tokio::sync::Mutex;

    // A string that exists nowhere else — not in the prompt, not on the guest's
    // disk, not in the model's training data. The only way into the answer is
    // through the mount.
    const MARKER: &str = "PLATYPUS-7";

    dotenvy::dotenv().ok();
    std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY to run this e2e check");
    set_dedicated_msb_home();
    assert!(tunnel_available(), "tunnel pump ELF not built");

    let data_root = tempfile::tempdir().unwrap();
    let files = data_root.path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(
        files.join("project.md"),
        format!("# Internal\n\nProject codename: {MARKER}\n"),
    )
    .unwrap();

    // Local files only: this is about discovery, not about any one provider, and
    // a plain file keeps the assertion deterministic.
    let unified: Arc<dyn ForwardFs> = Arc::new(
        WorkspaceFs::from_config(FsConfig {
            local_root: Some(files),
            mirror_root: None,
            google_oauth: None,
            mounts: vec![],
        })
        .expect("workspace fs"),
    );

    // Server first: its port has to be named in the network policy, which is
    // fixed when the sandbox is created.
    let srv = spawn_vfs_tunnel(unified, tokio::runtime::Handle::current()).expect("spawn tunnel");
    let sandbox = build_sandbox_builder(srv.port())
        .build()
        .await
        .expect("build sandbox");
    let sandbox = Arc::new(Mutex::new(sandbox));

    {
        let mut guard = sandbox.lock().await;
        let console = guard.start().await.expect("start VM");
        attach_vfs_tunnel_in_guest(console, &srv, GUEST_MOUNT_ROOT)
            .await
            .expect("mount workspace in guest");
    }

    // The same spec the backend builds for a coworker session, plus the mount
    // section the session run loop appends once the mount is up.
    let spec =
        agent_k::agents::get_coworker_agent_spec("agent-k", "anthropic/claude-sonnet-4-5", false);
    let instruction = format!(
        "{}{}",
        spec.instruction.clone().unwrap_or_default(),
        workspace_prompt(&[]),
    );
    let spec = spec.instruction(instruction);
    let state = AgentState::new().with_runenv(sandbox.clone());
    let mut agent = Agent::try_with_state(spec, state).expect("build agent");

    let query = Message::new(Role::User).with_contents([Part::text(
        "What is the project codename? Answer with just the codename.",
    )]);
    let mut transcript = String::new();
    let mut stream = agent.run(query);
    while let Some(event) = stream.next().await {
        let event = event.expect("agent turn");
        let msg = &event.message;
        for part in &msg.contents {
            if let Some(t) = part.as_text() {
                transcript.push_str(t);
                transcript.push('\n');
            }
        }
        if let Some(tcs) = &msg.tool_calls {
            for tc in tcs {
                if let Some((_id, name, args)) = tc.as_function() {
                    let args = serde_json::to_string(args).unwrap_or_default();
                    println!("  tool: {name} {args}");
                    transcript.push_str(&args);
                    transcript.push('\n');
                }
            }
        }
    }
    drop(stream);
    println!("--- transcript ---\n{transcript}");

    let _ = sandbox.lock().await.stop().await;

    assert!(
        transcript.contains(GUEST_MOUNT_ROOT),
        "the agent never touched {GUEST_MOUNT_ROOT}; it did not discover the mount"
    );
    assert!(
        transcript.contains(MARKER),
        "the agent never surfaced {MARKER}; it did not read the file through the mount"
    );
}
