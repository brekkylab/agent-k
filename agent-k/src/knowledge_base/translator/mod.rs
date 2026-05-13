use std::path::Path;

use anyhow::{Result, bail};

use super::FileType;

mod html;
mod pdf;

/// Converts an origin file to a markdown file at `corpus_path`, dispatching by file type.
pub async fn translate(filetype: FileType, origin_path: &Path, corpus_path: &Path) -> Result<()> {
    match filetype {
        FileType::PDF => pdf::translate_pdf(origin_path, corpus_path).await,
        FileType::HTML => html::translate_html(origin_path, corpus_path),
        FileType::MD => bail!("unsupported file type for translator: md"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::super::FileType;
    use super::translate;

    #[tokio::test]
    async fn translate_rejects_md_input() {
        let err = translate(
            FileType::MD,
            Path::new("/tmp/in.md"),
            Path::new("/tmp/out.md"),
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

        translate(FileType::HTML, &html_path, &md_path)
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

        translate(FileType::PDF, &pdf_path, &md_path)
            .await
            .unwrap_or_else(|e| panic!("pdf translation failed for {name}: {e}"));

        let md = fs::read_to_string(&md_path).expect("translated markdown should be readable");
        assert!(!md.trim().is_empty(), "markdown is empty for {name}");
    }
}
