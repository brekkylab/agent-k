use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;

// Bump this when build.rs args/logic change in a way that should invalidate
// the cached bundle even if Python inputs are byte-identical.
const BUILD_VERSION: u32 = 1;

const PYINSTALLER_ARGS: &[&str] = &[
    "convert_pdf_to_md.py",
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
        python_dir.join("pyproject.toml"),
        python_dir.join("uv.lock"),
        python_dir.join("convert_pdf_to_md.py"),
    ];
    for path in &inputs {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=DOCLING_SYS_SKIP_BUNDLE");

    let dist_root = out_dir.join("dist");
    let bundle_dir = dist_root.join("convert_pdf_to_md");
    let exe_name = if cfg!(windows) {
        "convert_pdf_to_md.exe"
    } else {
        "convert_pdf_to_md"
    };
    let exe_path = bundle_dir.join(exe_name);

    let skip = cfg!(feature = "skip-bundle")
        || env::var("DOCLING_SYS_SKIP_BUNDLE")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    if skip {
        println!("cargo:warning=docling-sys: skip-bundle enabled; runtime API will return an error");
        println!("cargo:rustc-env=DOCLING_BUNDLE_DIR=");
        return;
    }

    let stamp_path = out_dir.join(".bundle-stamp");
    let current_hash = match input_hash(&inputs) {
        Ok(h) => h,
        Err(err) => fail(&format!("hash inputs: {err}")),
    };

    let cached = fs::read_to_string(&stamp_path)
        .ok()
        .map(|s| s.trim().to_string());

    if cached.as_deref() == Some(&current_hash) && exe_path.exists() {
        println!("cargo:rustc-env=DOCLING_BUNDLE_DIR={}", bundle_dir.display());
        return;
    }

    println!("cargo:warning=docling-sys: building bundle (this will take a few minutes the first time)");

    if which("uv").is_none() {
        fail("`uv` not found in PATH. Install via https://docs.astral.sh/uv/#installation");
    }

    run(
        Command::new("uv")
            .arg("sync")
            .arg("--project")
            .arg(&python_dir)
            .arg("--extra")
            .arg("cpu"),
        "uv sync",
    );

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

    println!("cargo:rustc-env=DOCLING_BUNDLE_DIR={}", bundle_dir.display());
}

fn input_hash(paths: &[PathBuf]) -> std::io::Result<String> {
    let mut hasher = DefaultHasher::new();
    BUILD_VERSION.hash(&mut hasher);
    for arg in PYINSTALLER_ARGS {
        arg.hash(&mut hasher);
    }
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

