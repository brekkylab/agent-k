//! Convert PDFs to Markdown via a PyInstaller-bundled `docling` binary.
//!
//! `build.rs` runs `uv sync` + `pyinstaller` against the Python sources in
//! `python/`, producing a self-contained bundle directory inside `OUT_DIR`.
//! [`convert_pdf_to_md`] spawns that bundle as a subprocess, pipes the PDF
//! bytes to its stdin, and returns the markdown from stdout.
//!
//! ## Distribution
//!
//! [`bundle_dir`] points into the cargo `OUT_DIR` and is only valid while the
//! build tree exists. When packaging a downstream binary for shipping, copy
//! `bundle_dir()` next to the executable and set [`override_bundle_dir`] before
//! the first call so the runtime resolves the relocated path.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::{Context, anyhow};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const COMPILED_BUNDLE_DIR: &str = env!("DOCLING_BUNDLE_DIR");

static BUNDLE_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Path to the directory containing the bundled `convert_pdf_to_md` binary
/// and its `_internal` resources, or `None` if the crate was built with
/// `skip-bundle` and no override has been set.
pub fn bundle_dir() -> Option<&'static Path> {
    if let Some(p) = BUNDLE_DIR_OVERRIDE.get() {
        return Some(p.as_path());
    }
    if COMPILED_BUNDLE_DIR.is_empty() {
        None
    } else {
        Some(Path::new(COMPILED_BUNDLE_DIR))
    }
}

/// Override the bundle location at runtime. Call once, before the first
/// conversion, when the bundle has been relocated next to the consuming
/// binary. Subsequent calls are ignored.
pub fn override_bundle_dir(path: impl Into<PathBuf>) {
    let _ = BUNDLE_DIR_OVERRIDE.set(path.into());
}

/// Convert PDF bytes to Markdown.
pub async fn convert_pdf_to_md(pdf_bytes: &[u8]) -> anyhow::Result<String> {
    let dir = bundle_dir()
        .ok_or_else(|| anyhow!("docling-sys was built without a bundle (skip-bundle)"))?;
    let exe = dir.join(if cfg!(windows) {
        "convert_pdf_to_md.exe"
    } else {
        "convert_pdf_to_md"
    });

    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", exe.display()))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("child stdin unavailable"))?;
        stdin.write_all(pdf_bytes).await?;
        stdin.shutdown().await?;
    }

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("convert_pdf_to_md exited with {}: {}", output.status, stderr);
    }
    String::from_utf8(output.stdout).context("convert_pdf_to_md stdout was not valid UTF-8")
}

/// Read a file from disk and convert it to Markdown.
pub async fn convert_pdf_file(path: impl AsRef<Path>) -> anyhow::Result<String> {
    let bytes = tokio::fs::read(path.as_ref())
        .await
        .with_context(|| format!("failed to read {}", path.as_ref().display()))?;
    convert_pdf_to_md(&bytes).await
}
