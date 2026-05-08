use std::{fs, path::Path};

use anyhow::{Context as _, Result};

/// Convert a local HTML file to Markdown with YAML frontmatter.
///
/// Single-pass: `html-to-markdown-rs` runs metadata extraction over the
/// `<head>` while converting the body to Markdown. Chrome (HTML5 semantic
/// elements, aria landmarks, MediaWiki/Fandom noise classes, …) is excluded
/// from the body via the converter's `exclude_selectors` option.
pub(super) fn translate_html(html_path: &Path, md_path: &Path) -> Result<()> {
    let html = fs::read_to_string(html_path)
        .with_context(|| format!("failed to read HTML file: {html_path:?}"))?;

    let mut options = html_to_markdown_rs::ConversionOptions::default();
    options.exclude_selectors = CHROME_STRIP_SELECTORS
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    let result = html_to_markdown_rs::convert(&html, Some(options))
        .map_err(|e| anyhow::anyhow!("html-to-markdown-rs: {e}"))?;
    let md_body = result
        .content
        .clone()
        .context("html-to-markdown-rs returned no content")?;
    let doc_meta = &result.metadata.document;
    let title = doc_meta.title.clone().unwrap_or_default();

    let mut out = String::from("---\n");
    out.push_str("converter: html-to-markdown-rs\n");
    add_field(&mut out, "title", &title);
    if let Some(s) = doc_meta.author.as_deref() {
        add_field(&mut out, "author", s);
    }
    if let Some(s) = doc_meta.description.as_deref() {
        add_field(&mut out, "excerpt", s);
    }
    if let Some(s) = doc_meta.open_graph.get("site_name") {
        add_field(&mut out, "site_name", s);
    }
    if let Some(s) = doc_meta.language.as_deref() {
        add_field(&mut out, "language", s);
    }
    // md-rs stores `<meta property="article:…">` keys with `:` → `-`.
    if let Some(s) = doc_meta.meta_tags.get("article-published_time") {
        add_field(&mut out, "published_time", s);
    }
    if let Some(s) = doc_meta.meta_tags.get("article-modified_time") {
        add_field(&mut out, "modified_time", s);
    }
    out.push_str("---\n\n");

    if !title.is_empty() && !body_has_h1(&md_body) {
        out.push_str("# ");
        out.push_str(&title);
        out.push_str("\n\n");
    }
    out.push_str(&md_body);

    fs::write(md_path, out).with_context(|| format!("failed to write corpus: {md_path:?}"))?;
    Ok(())
}

/// Selectors stripped from the body before Markdown conversion. Combines
/// generic HTML5 semantic chrome, ARIA UI landmarks, hidden nodes, and
/// MediaWiki/Wikipedia/Fandom in-content noise.
const CHROME_STRIP_SELECTORS: &[&str] = &[
    // Generic HTML5 chrome.
    "script",
    "style",
    "noscript",
    "nav",
    "header",
    "footer",
    "aside",
    "button",
    "[role=navigation]",
    "[role=banner]",
    "[role=contentinfo]",
    "[role=complementary]",
    "[role=menu]",
    "[role=menubar]",
    "[role=toolbar]",
    "[role=dialog]",
    "[hidden]",
    "[aria-hidden=true]",
    ".menu-bar",
    // MediaWiki / Wikipedia / Fandom in-content noise. The `.mw-*` prefix is
    // unambiguous; remaining names are MW-specific enough that collateral
    // matches on non-MW sites are negligible.
    ".mw-editsection",
    ".mw-jump-link",
    ".mw-indicators",
    ".mw-empty-elt",
    ".printfooter",
    ".catlinks",
    ".sistersitebox",
    ".hatnote",
    ".mbox-text-span",
    ".navbox",
    ".navbox-inner",
];

fn body_has_h1(body: &str) -> bool {
    body.lines().any(|l| l.trim_start().starts_with("# "))
}

/// YAML 1.2 single-quoted scalar (`'` is the only escape).
fn yaml_str(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    let escaped = one_line.replace('\'', "''");
    format!("'{escaped}'")
}

fn add_field(out: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("{key}: {}\n", yaml_str(value)));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{translate_html, yaml_str};

    fn run_on(html: &str) -> String {
        let temp = tempfile::tempdir().expect("tempdir");
        let in_path = temp.path().join("in.html");
        let out_path = temp.path().join("out.md");
        fs::write(&in_path, html).expect("write html");
        translate_html(&in_path, &out_path).expect("translate must not fail on degenerate input");
        fs::read_to_string(&out_path).expect("read md")
    }

    #[test]
    fn translate_html_succeeds_on_degenerate_inputs() {
        for input in ["", "   \n\t  ", "<!doctype html>", "<html><body></body></html>"] {
            let md = run_on(input);
            assert!(md.starts_with("---\n"), "no frontmatter for input {input:?}");
            assert!(md.contains("converter: html-to-markdown-rs"));
            assert!(!md.contains("title:"), "unexpected title for input {input:?}");
        }
    }

    #[test]
    fn translate_html_emits_metadata_fields() {
        let html = r#"<!doctype html>
<html lang="ko">
<head>
<title>Page Title</title>
<meta name="author" content="Jane Doe">
<meta name="description" content="A short summary.">
<meta property="og:site_name" content="My Site">
<meta property="article:published_time" content="2025-01-02T03:04:05Z">
</head>
<body><p>Body paragraph that's long enough to be content.</p></body>
</html>"#;
        let md = run_on(html);
        assert!(md.contains("title: 'Page Title'"));
        assert!(md.contains("author: 'Jane Doe'"));
        assert!(md.contains("excerpt: 'A short summary.'"));
        assert!(md.contains("site_name: 'My Site'"));
        assert!(md.contains("language: 'ko'"));
        assert!(md.contains("published_time: '2025-01-02T03:04:05Z'"));
    }

    #[test]
    fn yaml_str_wraps_in_single_quotes() {
        assert_eq!(yaml_str("Hello world"), "'Hello world'");
    }

    #[test]
    fn yaml_str_doubles_embedded_single_quote() {
        assert_eq!(yaml_str("Don't stop"), "'Don''t stop'");
    }

    #[test]
    fn yaml_str_passes_double_quote_and_backslash_literally() {
        assert_eq!(
            yaml_str(r#"That "Smart" Move with C:\path"#),
            r#"'That "Smart" Move with C:\path'"#,
        );
    }

    #[test]
    fn yaml_str_collapses_newlines() {
        assert_eq!(yaml_str("line1\nline2\rline3"), "'line1 line2 line3'");
    }
}
