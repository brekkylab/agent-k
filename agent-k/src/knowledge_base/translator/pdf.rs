use std::path::Path;

use anyhow::{Context as _, Result};
use docling_sys::{PdfOptions, convert_pdf_file};

pub(super) async fn translate_pdf(pdf_path: &Path, md_path: &Path) -> Result<()> {
    let markdown = convert_pdf_file(pdf_path, &PdfOptions::default())
        .await
        .with_context(|| format!("docling conversion failed for {}", pdf_path.display()))?;
    tokio::fs::write(md_path, markdown)
        .await
        .with_context(|| format!("failed to write markdown to {}", md_path.display()))?;
    Ok(())
}
