//! Cross-compiles the in-guest raw-FUSE tunnel pump (`crates/ailoy-vfs-tunnel`)
//! for the guest arch (= this build's target arch, under libkrun same-arch
//! virtualization) as a static `…-unknown-linux-musl` ELF, and stages it in
//! `OUT_DIR/ailoy-vfs-tunnel` so `src/sandbox_tunnel.rs` can `include_bytes!` it.
//!
//! Best-effort: if the musl target isn't installed (or the build fails), an
//! empty stub is written instead so the crate still compiles — the in-guest
//! mount path then errors clearly at runtime. Uses Rust's bundled `ld.lld`, no
//! external toolchain — cross-links to Linux from a macOS host too.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let pump_dir = manifest_dir.join("../crates/ailoy-vfs-tunnel");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("ailoy-vfs-tunnel");

    println!(
        "cargo:rerun-if-changed={}",
        pump_dir.join("src/main.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        pump_dir.join("Cargo.toml").display()
    );

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target = format!("{arch}-unknown-linux-musl");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    let linker = bundled_rust_lld().unwrap_or_else(|| PathBuf::from("lld"));
    let encoded_rustflags = format!(
        "-Clinker={}\u{1f}-Clinker-flavor=ld.lld",
        linker.to_string_lossy()
    );
    let output = Command::new(&cargo)
        .args(["build", "--release", "--target", &target, "--manifest-path"])
        .arg(pump_dir.join("Cargo.toml"))
        // Use the rustup toolchain's bundled rust-lld directly. Merely selecting
        // the ld.lld flavor still makes rustc search PATH for an `lld` binary,
        // which rustup does not install there.
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags)
        // Cargo sets these for build-script children; leaving them would make the
        // sub-build inherit the OUTER build's flags/target-dir/wrapper — most
        // importantly `RUSTFLAGS`, which conflicts with the encoded flags above.
        // Clear them so the pump builds in its own workspace with just our lld
        // configuration.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output();

    let built = pump_dir.join(format!("target/{target}/release/ailoy-vfs-tunnel"));
    match output {
        Ok(o) if o.status.success() && built.exists() => {
            std::fs::copy(&built, &dest).expect("stage tunnel pump ELF into OUT_DIR");
        }
        other => {
            // Keep the crate compiling; the mount path checks for an empty ELF.
            std::fs::write(&dest, []).expect("write empty tunnel pump stub");
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
                "cargo:warning=in-guest VFS tunnel pump not built for {target}; in-VM mounts \
                 unavailable. cause: {detail}"
            );
        }
    }
}

fn bundled_rust_lld() -> Option<PathBuf> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let host = std::env::var("HOST").ok()?;
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let executable = if cfg!(windows) {
        "rust-lld.exe"
    } else {
        "rust-lld"
    };
    let linker = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim())
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin")
        .join(executable);
    linker.is_file().then_some(linker)
}
