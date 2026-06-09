use std::path::Path;

use anyhow::{Context as _, Result};
use docling_sys::{PdfOptions, convert_pdf_file};
use kreuzberg::{ExtractionConfig, PDF_MIME_TYPE, extract_bytes};

use super::PdfEngine;

pub(super) async fn translate_pdf(
    pdf_path: &Path,
    md_path: &Path,
    engine: PdfEngine,
) -> Result<()> {
    let markdown = match engine {
        PdfEngine::Kreuzberg => {
            let bytes = tokio::fs::read(pdf_path)
                .await
                .with_context(|| format!("failed to read {}", pdf_path.display()))?;
            extract_bytes(&bytes, PDF_MIME_TYPE, &ExtractionConfig::default())
                .await
                .with_context(|| format!("kreuzberg extraction failed for {}", pdf_path.display()))?
                .content
        }
        PdfEngine::Docling => convert_pdf_file(pdf_path, &PdfOptions::default())
            .await
            .with_context(|| format!("docling conversion failed for {}", pdf_path.display()))?,
    };
    tokio::fs::write(md_path, markdown)
        .await
        .with_context(|| format!("failed to write markdown to {}", md_path.display()))?;
    Ok(())
}
