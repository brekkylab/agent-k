use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;

const PYINSTALLER_ARGS: &[&str] = &[
    "run_docling.py",
    "--onedir",
    "--noconfirm",
    "--recursive-copy-metadata=docling",
    "--collect-all=docling",
    "--collect-all=docling_core",
    "--collect-all=docling_ibm_models",
    "--collect-all=docling_parse",
    "--exclude-module=hf_xet",
    "--exclude-module=faker",
    "--exclude-module=tree_sitter",
    "--exclude-module=tree_sitter_typescript",
    "--exclude-module=tree_sitter_c",
    "--exclude-module=tree_sitter_javascript",
    "--exclude-module=tree_sitter_python",
];

fn main() {
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let python_dir = crate_dir.join("python");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let inputs = [
        crate_dir.join("build.rs"),
        python_dir.join("pyproject.toml"),
        python_dir.join("uv.lock"),
        python_dir.join("run_docling.py"),
    ];
    for path in &inputs {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-env-changed=DOCLING_SYS_SKIP_BUNDLE");

    let dist_root = out_dir.join("dist");
    let bundle_dir = dist_root.join("run_docling");
    let exe_name = if cfg!(windows) {
        "run_docling.exe"
    } else {
        "run_docling"
    };
    let exe_path = bundle_dir.join(exe_name);

    let skip = cfg!(feature = "skip-bundle")
        || env::var("DOCLING_SYS_SKIP_BUNDLE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    if skip {
        println!("cargo:warning=docling-sys: skip-bundle enabled; runtime API will return an error");
        return;
    }

    let cpu_only = cfg!(feature = "cpu");

    let stamp_path = out_dir.join(".bundle-stamp");
    let current_hash = match input_hash(&inputs, cpu_only) {
        Ok(h) => h,
        Err(err) => fail(&format!("hash inputs: {err}")),
    };

    let cached = fs::read_to_string(&stamp_path)
        .ok()
        .map(|s| s.trim().to_string());

    if cached.as_deref() == Some(&current_hash) && exe_path.exists() {
        link_bundle_into_profile(&out_dir, &bundle_dir);
        return;
    }

    println!("cargo:warning=docling-sys: building bundle (this will take a few minutes the first time)");

    if which("uv").is_none() {
        fail("`uv` not found in PATH. Install via https://docs.astral.sh/uv/#installation");
    }

    let mut sync_cmd = Command::new("uv");
    sync_cmd.arg("sync").arg("--project").arg(&python_dir);
    if cpu_only {
        sync_cmd.arg("--extra").arg("cpu");
    }
    run(&mut sync_cmd, "uv sync");

    let venv_bin = python_dir.join(".venv").join(if cfg!(windows) {
        "Scripts"
    } else {
        "bin"
    });
    let pyinstaller = venv_bin.join(if cfg!(windows) {
        "pyinstaller.exe"
    } else {
        "pyinstaller"
    });
    if !pyinstaller.exists() {
        fail(&format!(
            "pyinstaller not found at {} after `uv sync`",
            pyinstaller.display()
        ));
    }

    if dist_root.exists() {
        let _ = fs::remove_dir_all(&dist_root);
    }
    let workpath = out_dir.join("build");
    if workpath.exists() {
        let _ = fs::remove_dir_all(&workpath);
    }

    let mut cmd = Command::new(&pyinstaller);
    cmd.current_dir(&python_dir)
        .args(PYINSTALLER_ARGS)
        .arg("--distpath")
        .arg(&dist_root)
        .arg("--workpath")
        .arg(&workpath)
        .arg("--specpath")
        .arg(&out_dir);
    run(&mut cmd, "pyinstaller");

    if !exe_path.exists() {
        fail(&format!(
            "expected bundle binary at {} but it was not produced",
            exe_path.display()
        ));
    }

    if let Err(err) = fs::write(&stamp_path, &current_hash) {
        fail(&format!("write stamp file: {err}"));
    }

    link_bundle_into_profile(&out_dir, &bundle_dir);
}

/// Place the freshly-built bundle wherever cargo lands executables that
/// might call into this crate. The runtime lookup in `lib.rs` only checks
/// the directory of `current_exe()`, so we drop the bundle into:
///
///   - `target/{profile}/`            — `cargo run`, regular bin targets
///   - `target/{profile}/deps/`       — `cargo test` integration tests
///   - `target/{profile}/examples/`   — `cargo run --example`
///
/// Uses a directory symlink (Unix or Windows) and falls back to a
/// recursive copy if symlinks aren't permitted.
fn link_bundle_into_profile(out_dir: &PathBuf, bundle_dir: &PathBuf) {
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        println!("cargo:warning=docling-sys: could not locate profile dir from OUT_DIR");
        return;
    };
    for sub in ["", "deps", "examples"] {
        let parent = if sub.is_empty() {
            profile_dir.to_path_buf()
        } else {
            profile_dir.join(sub)
        };
        if let Err(err) = fs::create_dir_all(&parent) {
            println!(
                "cargo:warning=docling-sys: mkdir {} failed ({err})",
                parent.display()
            );
            continue;
        }
        place_bundle_link(bundle_dir, &parent.join("run_docling"));
    }
}

fn place_bundle_link(bundle_dir: &PathBuf, link: &PathBuf) {
    match fs::symlink_metadata(link) {
        Ok(meta) => {
            let result = if meta.file_type().is_symlink() || meta.file_type().is_file() {
                fs::remove_file(link)
            } else {
                fs::remove_dir_all(link)
            };
            if let Err(err) = result {
                println!(
                    "cargo:warning=docling-sys: failed to clear stale {} ({err})",
                    link.display()
                );
                return;
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            println!(
                "cargo:warning=docling-sys: stat {} failed ({err})",
                link.display()
            );
            return;
        }
    }

    if let Err(err) = symlink_dir(bundle_dir, link) {
        println!(
            "cargo:warning=docling-sys: symlink {} failed ({err}); falling back to copy",
            link.display()
        );
        if let Err(err) = copy_dir_all(bundle_dir, link) {
            fail(&format!("copy bundle to {}: {err}", link.display()));
        }
    }
}

#[cfg(unix)]
fn symlink_dir(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn symlink_dir(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
}

fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn input_hash(paths: &[PathBuf], cpu_only: bool) -> std::io::Result<String> {
    let mut hasher = DefaultHasher::new();
    cpu_only.hash(&mut hasher);
    for path in paths {
        let bytes = fs::read(path)?;
        path.file_name().unwrap().to_string_lossy().hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    Ok(format!("{:x}", hasher.finish()))
}

fn run(cmd: &mut Command, label: &str) {
    let status = match cmd.status() {
        Ok(s) => s,
        Err(err) => fail(&format!("{label}: failed to spawn: {err}")),
    };
    if !status.success() {
        fail(&format!("{label}: exited with {status}"));
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn fail(msg: &str) -> ! {
    eprintln!("docling-sys build error: {msg}");
    std::process::exit(1);
}

