//! microsandbox-on-this-host spike (Step 2/3 gate for the "agent reads my Notion
//! in the VM" goal). Boots a real sandbox VM with the same image agent-k's
//! coworker CLI uses, execs a couple of commands, and checks `/dev/fuse` exists
//! (the in-guest FUSE forwarder mounts through it). No LLM keys needed.
//!
//! Ignored by default (pulls an image + boots a VM). Run explicitly on the host
//! you intend to verify on:
//!
//!   cargo test -p agent-k-backend --test sandbox_spike -- --ignored --nocapture
//!
//! Success here means microsandbox works on this machine and the FUSE forwarder
//! path (Steps 2–3) is viable locally.

use ailoy::runenv::{Console as _, Machine as _, SandboxBuilder, SandboxNetwork};

#[tokio::test]
#[ignore = "boots a microsandbox VM; run explicitly to verify sandbox works on this host"]
async fn sandbox_boots_and_has_fuse() {
    // Keep the pinned microsandbox state isolated from other versions installed
    // on the host. Override by exporting MSB_HOME.
    if std::env::var_os("MSB_HOME").is_none()
        && let Some(home) = std::env::var_os("HOME")
    {
        let dedicated = std::path::PathBuf::from(home).join(".microsandbox-agentk");
        // SAFETY: set before any microsandbox call, at test start, single-threaded.
        unsafe { std::env::set_var("MSB_HOME", &dedicated) };
        println!("using dedicated MSB_HOME={}", dedicated.display());
    }

    // Same image the coworker CLI uses, so this mirrors a known-good config.
    let mut sandbox = SandboxBuilder::new()
        .image("brekkylab/agent-k-libreoffice:latest")
        .cpus(2)
        .memory_mib(1024)
        // Stated rather than inherited, as at every other sandbox. This one only
        // execs `echo`, `uname` and a `/dev/fuse` check, so the narrow posture is
        // what it actually needs.
        .network(SandboxNetwork::HostOnly)
        .build()
        .await
        .expect("build sandbox (image pull + VM create) — fails here if microsandbox can't run on this host");

    let console = sandbox.start().await.expect("start VM");

    let r = console
        .exec_shell(
            "echo hello-from-guest && uname -a && (test -e /dev/fuse && echo 'HAS /dev/fuse' || echo 'NO /dev/fuse')".to_string(),
            Some(60),
        )
        .await
        .expect("exec in guest");

    println!("--- exit {} ---", r.exit_code);
    println!("stdout:\n{}", r.stdout);
    if !r.stderr.trim().is_empty() {
        println!("stderr:\n{}", r.stderr);
    }

    assert_eq!(r.exit_code, 0, "guest command failed");
    assert!(
        r.stdout.contains("hello-from-guest"),
        "guest did not echo back"
    );
    // The forwarder mounts via /dev/fuse directly; confirm the guest kernel has it.
    assert!(
        r.stdout.contains("HAS /dev/fuse"),
        "guest kernel lacks /dev/fuse — the in-guest FUSE forwarder needs it"
    );

    let _ = sandbox.stop().await;
}
