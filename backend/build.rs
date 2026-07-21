//! Cross-compiles the in-guest VFS forwarder (`crates/ailoy-vfs-forwarder`) for
//! the guest arch (= this build's target arch, under libkrun same-arch
//! virtualization) as a static `…-unknown-linux-musl` ELF, and stages it in
//! `OUT_DIR/ailoy-vfs-fwd` so `src/vfs/sandbox/guest.rs` can `include_bytes!` it.
//!
//! Best-effort: if the musl target isn't installed (or the build fails), an
//! empty stub is written instead so the crate still compiles — the in-guest
//! mount path then errors clearly at runtime. Mirrors ailoy's build.rs recipe
//! (Rust's bundled `ld.lld`, no external toolchain — cross-links to Linux from
//! a macOS host too).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let fwd_dir = manifest_dir.join("../crates/ailoy-vfs-forwarder");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("ailoy-vfs-fwd");

    println!(
        "cargo:rerun-if-changed={}",
        fwd_dir.join("src/main.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        fwd_dir.join("Cargo.toml").display()
    );

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target = format!("{arch}-unknown-linux-musl");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    let output = Command::new(&cargo)
        .args(["build", "--release", "--target", &target, "--manifest-path"])
        .arg(fwd_dir.join("Cargo.toml"))
        // Rust's bundled lld links the musl target without an external toolchain
        // (works from a macOS build host too).
        .env("RUSTFLAGS", "-C linker-flavor=ld.lld")
        // Cargo sets these for build-script children; leaving them would make the
        // sub-build inherit the OUTER build's flags/target-dir/wrapper — most
        // importantly `CARGO_ENCODED_RUSTFLAGS`, which conflicts with the
        // `RUSTFLAGS` we set ("both … were set"). Clear them so the forwarder
        // builds in its own workspace with just our lld flag.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output();

    let built = fwd_dir.join(format!("target/{target}/release/ailoy-vfs-fwd"));
    match output {
        Ok(o) if o.status.success() && built.exists() => {
            std::fs::copy(&built, &dest).expect("stage forwarder ELF into OUT_DIR");
        }
        other => {
            // Keep the crate compiling; the mount path checks for an empty ELF.
            std::fs::write(&dest, []).expect("write empty forwarder stub");
            let detail = match other {
                Ok(o) => String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .rev()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join(" | "),
                Err(e) => e.to_string(),
            };
            println!(
                "cargo:warning=in-guest VFS forwarder not built for {target}; in-VM mounts \
                 unavailable. cause: {detail}"
            );
        }
    }
}
