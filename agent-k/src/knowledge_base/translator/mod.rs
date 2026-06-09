use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::FileType;

mod html;
mod pdf;

/// PDF-to-markdown engine. `Kreuzberg` is native and fast; `Docling` is slower
/// but preserves layout better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PdfEngine {
    #[default]
    Kreuzberg,
    Docling,
}

impl PdfEngine {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "kreuzberg" => Some(Self::Kreuzberg),
            "docling" => Some(Self::Docling),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kreuzberg => "kreuzberg",
            Self::Docling => "docling",
        }
    }
}

/// Converts an origin file to a markdown file at `corpus_path`, dispatching by file type.
pub async fn translate(
    filetype: FileType,
    origin_path: &Path,
    corpus_path: &Path,
    pdf_engine: PdfEngine,
) -> Result<()> {
    match filetype {
        FileType::PDF => pdf::translate_pdf(origin_path, corpus_path, pdf_engine).await,
        FileType::HTML => html::translate_html(origin_path, corpus_path),
        FileType::MD => bail!("unsupported file type for translator: md"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::super::FileType;
    use super::{PdfEngine, translate};

    #[tokio::test]
    async fn translate_rejects_md_input() {
        let err = translate(
            FileType::MD,
            Path::new("/tmp/in.md"),
            Path::new("/tmp/out.md"),
            PdfEngine::default(),
        )
        .await
        .expect_err("translator should reject md passthrough type");
        assert!(err.to_string().contains("unsupported file type"));
    }

    #[tokio::test]
    async fn translate_html_dispatches_to_html_converter() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let html_path = temp.path().join("sample.html");
        let md_path = temp.path().join("sample.md");
        let html = r#"
<!doctype html>
<html lang="en">
  <head><title>Sample Title</title></head>
  <body>
    <main>
      <article>
        <h1>Sample Title</h1>
        <p>This is a long enough paragraph to pass readability scoring and ensure the html translator emits markdown content reliably for testing purposes.</p>
        <p>Another paragraph with additional text content that helps the extractor pick a strong candidate node from the document body.</p>
      </article>
    </main>
  </body>
</html>
"#;
        fs::write(&html_path, html).expect("test html should be written");

        translate(FileType::HTML, &html_path, &md_path, PdfEngine::default())
            .await
            .expect("html translation should succeed");
        let md = fs::read_to_string(&md_path).expect("translated markdown should be readable");
        assert!(md.starts_with("---\n"));
        assert!(md.contains("converter: html-to-markdown-rs"));
    }

    #[cfg(feature = "internal")]
    #[tokio::test]
    #[ignore = "requires docling bundle & network access"]
    async fn translate_pdf_from_financebench() {
        use knowledge_base_examples::{Cached, DocSet as _, FinanceBench};

        let kb = Cached::new(
            FinanceBench::new()
                .await
                .expect("failed to init FinanceBench"),
        )
        .expect("failed to create cache");

        let name = kb.filename(0).await.unwrap_or_else(|| "doc-0".into());
        let bytes: Vec<u8> = kb
            .read_origin(0)
            .await
            .unwrap_or_else(|| panic!("failed to fetch {name}"))
            .into();

        let temp = tempfile::tempdir().expect("temp dir should be created");
        let pdf_path = temp.path().join("sample.pdf");
        let md_path = temp.path().join("sample.md");
        fs::write(&pdf_path, &bytes).expect("failed to write origin pdf");

        translate(FileType::PDF, &pdf_path, &md_path, PdfEngine::default())
            .await
            .unwrap_or_else(|e| panic!("pdf translation failed for {name}: {e}"));

        let md = fs::read_to_string(&md_path).expect("translated markdown should be readable");
        assert!(!md.trim().is_empty(), "markdown is empty for {name}");
    }

    #[tokio::test]
    async fn translate_pdf_kreuzberg_extracts_text() {
        let pdf = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj\n4 0 obj<</Length 52>>stream\nBT /F1 24 Tf 72 700 Td (Knowledge corpus marker) Tj ET\nendstream endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\ntrailer<</Root 1 0 R/Size 6>>\n%%EOF";
        let temp = tempfile::tempdir().unwrap();
        let pdf_path = temp.path().join("in.pdf");
        let md_path = temp.path().join("out.md");
        fs::write(&pdf_path, pdf).unwrap();

        translate(FileType::PDF, &pdf_path, &md_path, PdfEngine::Kreuzberg)
            .await
            .expect("kreuzberg pdf translation should succeed");

        let md = fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("Knowledge corpus marker"), "got: {md}");
    }
}
