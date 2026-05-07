//! Convert PDFs to Markdown via a PyInstaller-bundled `docling` binary.
//!
//! `build.rs` runs `uv sync` + `pyinstaller` against the Python sources in
//! `python/`, producing a self-contained bundle directory. At runtime, the
//! library expects the bundle to sit as a `run_docling/` folder directly
//! beside the consuming executable. `build.rs` arranges this for cargo
//! builds; when shipping, copy `run_docling/` next to your executable.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const BUNDLE_DIR_NAME: &str = "run_docling";

#[cfg(windows)]
const BUNDLE_BINARY: &str = "run_docling.exe";
#[cfg(not(windows))]
const BUNDLE_BINARY: &str = "run_docling";

static RESOLVED_BUNDLE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Path to the bundle directory (`run_docling/`) sitting beside the
/// current executable, or `None` if it is not present.
pub fn bundle_dir() -> Option<&'static Path> {
    RESOLVED_BUNDLE_DIR
        .get_or_init(|| {
            let dir = std::env::current_exe().ok()?.parent()?.join(BUNDLE_DIR_NAME);
            dir.join(BUNDLE_BINARY).is_file().then_some(dir)
        })
        .as_deref()
}

/// TableFormer extraction mode. `Accurate` trades speed for quality.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TableStructureMode {
    Fast,
    Accurate,
}

/// Hardware device for model inference. Mirrors docling's `AcceleratorDevice`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AcceleratorDevice {
    Auto,
    Cpu,
    Cuda,
    Mps,
    Xpu,
}

/// Options forwarded to the `PdfPipelineOptions` constructor on the Python
/// side. Defaults match the previously-hardcoded behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfOptions {
    pub do_ocr: bool,
    pub do_table_structure: bool,
    pub do_cell_matching: bool,
    pub table_structure_mode: TableStructureMode,
    pub do_picture_classification: bool,
    pub do_picture_description: bool,
    pub do_chart_extraction: bool,
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub generate_page_images: bool,
    pub generate_picture_images: bool,
    pub num_threads: u32,
    pub device: AcceleratorDevice,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            do_ocr: false,
            do_table_structure: true,
            do_cell_matching: true,
            table_structure_mode: TableStructureMode::Accurate,
            do_picture_classification: false,
            do_picture_description: false,
            do_chart_extraction: false,
            do_code_enrichment: false,
            do_formula_enrichment: false,
            generate_page_images: false,
            generate_picture_images: false,
            num_threads: 4,
            device: AcceleratorDevice::Auto,
        }
    }
}

/// Convert PDF bytes to Markdown.
pub async fn convert_pdf_to_md(
    pdf_bytes: &[u8],
    options: &PdfOptions,
) -> anyhow::Result<String> {
    let dir = bundle_dir().ok_or_else(|| {
        anyhow!(
            "docling bundle not found next to the current executable; \
             expected a `{BUNDLE_DIR_NAME}/` directory containing `{BUNDLE_BINARY}`"
        )
    })?;
    let exe = dir.join(BUNDLE_BINARY);

    let options_json =
        serde_json::to_string(options).context("serialize PdfOptions")?;

    let mut child = Command::new(&exe)
        .arg("--options")
        .arg(&options_json)
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
        anyhow::bail!("run_docling exited with {}: {}", output.status, stderr);
    }
    String::from_utf8(output.stdout).context("run_docling stdout was not valid UTF-8")
}

/// Read a file from disk and convert it to Markdown.
pub async fn convert_pdf_file(
    path: impl AsRef<Path>,
    options: &PdfOptions,
) -> anyhow::Result<String> {
    let bytes = tokio::fs::read(path.as_ref())
        .await
        .with_context(|| format!("failed to read {}", path.as_ref().display()))?;
    convert_pdf_to_md(&bytes, options).await
}
