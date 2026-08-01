use std::path::Path;

use anyhow::{Context as _, Result};
use docling_sys::{PdfOptions, convert_pdf_file};
use kreuzberg::{ExtractionConfig, PDF_MIME_TYPE, extract_bytes};

use super::PdfEngine;

/// Extraction settings for the Kreuzberg engine.
///
/// The only departure from the default is a bounded budget. Kreuzberg documents
/// `extraction_timeout_secs` as "`None` means no timeout (unbounded extraction
/// time)", and `None` is the default — so a PDF that makes pdfium spin otherwise
/// runs for as long as the host process does. The input is a document from
/// outside, uploaded or fetched from the web, which is the same reason the
/// docling path is time-boxed; reusing its constant keeps one budget for "how
/// long a PDF conversion may run" rather than two that drift.
///
/// This releases the caller, not the CPU: the budget is a `tokio::time::timeout`
/// around the extraction future, and Kreuzberg runs pdfium on the blocking pool,
/// so the work itself keeps going and holds one blocking thread until it
/// finishes. Bounding that needs the parser out of process, which is a larger
/// change than this.
fn kreuzberg_config() -> ExtractionConfig {
    ExtractionConfig {
        extraction_timeout_secs: Some(docling_sys::DEFAULT_TIMEOUT.as_secs()),
        ..Default::default()
    }
}

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
            extract_bytes(&bytes, PDF_MIME_TYPE, &kreuzberg_config())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine every caller actually gets is Kreuzberg (`PdfEngine::default()`),
    /// so this config is what bounds a knowledge-ingest conversion. Reverting it
    /// to `ExtractionConfig::default()` puts the unbounded behaviour back without
    /// failing anything else, which is what this pins.
    #[test]
    fn kreuzberg_extraction_is_time_boxed() {
        let config = kreuzberg_config();
        assert_eq!(
            config.extraction_timeout_secs,
            Some(docling_sys::DEFAULT_TIMEOUT.as_secs()),
            "the Kreuzberg path must carry the same budget as the docling path"
        );
        assert!(
            ExtractionConfig::default().extraction_timeout_secs.is_none(),
            "the default is still unbounded, so setting it explicitly is what matters"
        );
    }

    /// A budget that rejected ordinary documents would be worse than none. Uses a
    /// real PDF rather than a stub because the timeout wraps the whole extraction
    /// future, so a stub would not exercise it.
    #[tokio::test]
    async fn a_normal_pdf_still_converts_within_the_budget() {
        let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/test_case/cases/payslips.pdf");
        let out = tempfile::tempdir().unwrap().path().join("out.md");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();

        translate_pdf(&pdf, &out, PdfEngine::Kreuzberg)
            .await
            .expect("a normal PDF must convert with the budget set");

        let md = std::fs::read_to_string(&out).unwrap();
        assert!(!md.trim().is_empty(), "conversion produced no markdown");
    }
}
