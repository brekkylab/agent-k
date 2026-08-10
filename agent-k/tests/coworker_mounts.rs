//! Boots the coworker runenv and checks what the guest may write.
//!
//! `get_coworker_agent_runenv` makes the user's attachments a read-only bind.
//! Nothing else in the tree proves the mount flag survives into the guest, and
//! a silently-writable mount looks identical to a correct one from the host, so
//! this asks the guest directly. Needs no model or API key — only microsandbox.

use agent_k::agents::get_coworker_agent_runenv;
use ailoy::runenv::{Console as _, Machine as _};

fn set_dedicated_msb_home() {
    if std::env::var_os("MSB_HOME").is_none()
        && let Some(home) = std::env::var_os("HOME")
    {
        let d = std::path::PathBuf::from(home).join(".microsandbox-agentk");
        // SAFETY: called at test start before the microsandbox runtime spawns.
        unsafe { std::env::set_var("MSB_HOME", &d) };
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots a microsandbox VM; run explicitly"]
async fn attachments_are_read_only_in_the_guest_and_artifacts_are_not() {
    set_dedicated_msb_home();

    let root = tempfile::tempdir().expect("tempdir");
    // macOS puts tempdirs under /var, a symlink to /private/var; microsandbox
    // wants the real path.
    let base = root.path().canonicalize().expect("canonicalize tempdir");
    let input = base.join("data");
    let shared = base.join("shared");
    let artifacts = base.join("artifacts");
    for d in [&input, &shared, &artifacts] {
        std::fs::create_dir_all(d).expect("create dir");
    }
    std::fs::write(input.join("upload.txt"), b"original").expect("seed attachment");

    let mut sandbox = get_coworker_agent_runenv(&input, &shared, &artifacts)
        .await
        .expect("build coworker runenv");
    let console = sandbox.start().await.expect("start VM");

    let probe = |cmd: &str| {
        let cmd = cmd.to_string();
        async {
            console
                .exec_shell(cmd, Some(30))
                .await
                .expect("exec in guest")
        }
    };

    let read = probe("cat /root/attached/upload.txt").await;
    let overwrite = probe("echo tampered > /root/attached/upload.txt").await;
    let create = probe("touch /root/attached/planted.sh").await;
    let artifact = probe("echo out > /root/artifacts/result.txt").await;

    let _ = sandbox.stop().await;

    println!("read      exit={} out={:?}", read.exit_code, read.stdout.trim());
    println!("overwrite exit={} err={:?}", overwrite.exit_code, overwrite.stderr.trim());
    println!("create    exit={} err={:?}", create.exit_code, create.stderr.trim());
    println!("artifact  exit={}", artifact.exit_code);

    assert_eq!(read.exit_code, 0, "the guest must still be able to read attachments");
    assert_eq!(read.stdout.trim(), "original");
    assert_ne!(overwrite.exit_code, 0, "the guest overwrote a read-only attachment");
    assert_ne!(create.exit_code, 0, "the guest created a file in a read-only mount");
    assert_eq!(artifact.exit_code, 0, "artifacts must stay writable");

    // The host's copy is the thing actually being protected.
    assert_eq!(
        std::fs::read_to_string(input.join("upload.txt")).unwrap(),
        "original",
        "the host-side attachment changed"
    );
    assert!(!input.join("planted.sh").exists(), "the guest planted a file on the host");
    assert_eq!(
        std::fs::read_to_string(artifacts.join("result.txt")).unwrap().trim(),
        "out",
        "artifacts did not reach the host"
    );
}
