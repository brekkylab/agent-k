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

use agent_k_backend::sandbox_tunnel::{mount_vfs_tunnel_in_guest, tunnel_available};

/// Build a provider-only workspace fs (no local files) — enough to serve `/s3`
/// or `/notion` into the guest for these read checks.
fn provider_fs(prefix: &str, provider: ProviderConfig) -> Arc<dyn ForwardFs> {
    let config = FsConfig {
        local_root: None,
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

fn build_sandbox_builder() -> SandboxBuilder {
    SandboxBuilder::new()
        .image("brekkylab/agent-k-libreoffice:latest")
        .cpus(2)
        .memory_mib(1024)
        .network(SandboxNetwork::Public)
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

    let mut sandbox = build_sandbox_builder().build().await.expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");

    let rt = tokio::runtime::Handle::current();
    let mounted = mount_vfs_tunnel_in_guest(console, fs.clone(), "/mnt/tunnel", rt).await;
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

    let mut sandbox = build_sandbox_builder().build().await.expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");

    let rt = tokio::runtime::Handle::current();
    let _srv = mount_vfs_tunnel_in_guest(console, fs.clone(), "/mnt/ws", rt)
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

    let mut sandbox = build_sandbox_builder().build().await.expect("build sandbox");
    let console = sandbox.start().await.expect("start VM");

    let rt = tokio::runtime::Handle::current();
    let _srv = mount_vfs_tunnel_in_guest(console, make_fs(), "/mnt/ws", rt)
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
