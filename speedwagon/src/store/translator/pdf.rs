use std::{io::Write as _, path::Path, path::PathBuf, process::Command};

use anyhow::{Context as _, Result, bail};

const PYTHON_VERSION: &str = "3.12";
const DOCLING_PACKAGE: &str = "docling>=2,<3";

const DOCLING_SOURCE: &str = r#"
import json
import logging
import os
from pathlib import Path

os.environ["HF_HUB_DISABLE_PROGRESS_BARS"] = "1"
logging.getLogger("docling").setLevel(logging.CRITICAL)


def build_converter():
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import (
        PdfPipelineOptions,
        TableStructureOptions,
    )
    from docling.document_converter import DocumentConverter, PdfFormatOption

    pipeline_options = PdfPipelineOptions(
        do_ocr=False,
        do_table_structure=True,
        table_structure_options=TableStructureOptions(
            do_cell_matching=True,
            mode="accurate",
        ),
        accelerator_options={"num_threads": 4, "device": "auto"},
        do_picture_classification=False,
        do_picture_description=False,
        do_chart_extraction=False,
        do_code_enrichment=False,
        do_formula_enrichment=False,
        generate_page_images=False,
        generate_picture_images=False,
    )
    return DocumentConverter(
        format_options={
            InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options),
        }
    )


def main():
    args_path = os.environ.get("AILOY_ARGS_JSON_PATH", "")
    args = json.loads(Path(args_path).read_text(encoding="utf-8"))
    pdf_path = Path(args["pdf_path"])
    md_path = Path(args["md_path"])
    markdown = build_converter().convert(pdf_path).document.export_to_markdown()
    md_path.write_text(markdown, encoding="utf-8")


if __name__ == "__main__":
    main()
"#;

pub(super) fn translate_pdf(pdf_path: &Path, md_path: &Path) -> Result<()> {
    let venv = docling_venv_path()?;
    ensure_venv(&venv)?;

    let args = serde_json::json!({
        "pdf_path": pdf_path,
        "md_path": md_path,
    });
    let mut args_file =
        tempfile::NamedTempFile::new().context("failed to create temp args file")?;
    args_file
        .write_all(args.to_string().as_bytes())
        .context("failed to write args")?;

    let mut script_file = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()
        .context("failed to create temp script file")?;
    script_file
        .write_all(DOCLING_SOURCE.as_bytes())
        .context("failed to write python script")?;

    let python = venv.join("bin").join("python");
    let output = Command::new(&python)
        .arg(script_file.path())
        .env("AILOY_ARGS_JSON_PATH", args_file.path())
        .output()
        .context("failed to spawn python")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("docling conversion failed: {stderr}");
    }

    Ok(())
}

fn docling_venv_path() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .context("cannot determine cache directory")?
        .join("ailoy")
        .join("venvs")
        .join("docling"))
}

fn ensure_venv(venv: &Path) -> Result<()> {
    if venv.join("bin").join("python").exists() {
        return Ok(());
    }

    let status = Command::new("uv")
        .args(["venv", "--python", PYTHON_VERSION])
        .arg(venv)
        .status()
        .context("failed to run `uv venv` — is uv installed?")?;
    if !status.success() {
        bail!("uv venv creation failed");
    }

    let status = Command::new("uv")
        .args(["pip", "install", "--python"])
        .arg(venv.join("bin").join("python"))
        .arg(DOCLING_PACKAGE)
        .status()
        .context("failed to run `uv pip install`")?;
    if !status.success() {
        bail!("uv pip install docling failed");
    }

    Ok(())
}
