use std::{fs, path::Path};

use anyhow::{Context as _, Result};

/// Convert a local HTML file to Markdown with YAML frontmatter.
///
/// Pipeline:
/// 1. Read HTML from `html_path`.
/// 2. Extract metadata via `dom_smoothie` (title, byline, excerpt, dates,
///    language, image, favicon, …). `article.content` is **not** used for
///    the body.
/// 3. Body extraction: generic chrome-strip — drop HTML5 semantic chrome
///    (`<nav>`/`<header>`/`<footer>`/`<aside>`), aria landmarks, hidden
///    nodes, scripts/styles, and known MediaWiki/Fandom in-content noise.
/// 4. Convert the chosen body HTML to Markdown via `html-to-markdown-rs`.
/// 5. Emit YAML frontmatter followed by an optional `# Title` and the
///    Markdown body.
pub(super) fn translate_html(html_path: &Path, md_path: &Path) -> Result<()> {
    // 1. Acquire HTML.
    let html = fs::read_to_string(html_path)
        .with_context(|| format!("failed to read HTML file: {html_path:?}"))?;

    // 2. dom_smoothie — metadata only.
    let cfg = dom_smoothie::Config {
        candidate_select_mode: dom_smoothie::CandidateSelectMode::DomSmoothie,
        readable_min_score: 10.0,
        readable_min_content_length: 70,
        n_top_candidates: 10,
        ..Default::default()
    };
    let mut reader = dom_smoothie::Readability::new(html.as_str(), None, Some(cfg))
        .map_err(|e| anyhow::anyhow!("dom_smoothie::new: {e}"))?;
    let article = reader
        .parse()
        .map_err(|e| anyhow::anyhow!("dom_smoothie::parse: {e}"))?;

    // 3. Body extraction via chrome-strip. dom_smoothie above is used only
    //    for metadata; its `article.content` is *not* used for the body.
    //    Empirical testing on Wikipedia, Fandom, Docusaurus, and Stack
    //    Overflow showed that simple chrome stripping (HTML5 semantic chrome
    //    + MediaWiki-style noise classes) preserves heading structure as
    //    well as Readability-style scoring while avoiding sibling-section
    //    pruning that Readability-style algorithms commonly cause.
    let body_html = default_chrome_strip(&html);

    // 4. HTML → Markdown.
    let result = html_to_markdown_rs::convert(&body_html, Default::default())
        .map_err(|e| anyhow::anyhow!("html-to-markdown-rs: {e}"))?;
    let md_body = result
        .content
        .context("html-to-markdown-rs returned no content")?;

    // 5. YAML frontmatter.
    let mut out = String::from("---\n");
    out.push_str("converter: html-to-markdown-rs\n");
    add_field(&mut out, "title", &article.title);
    if let Some(s) = article.byline.as_deref() {
        add_field(&mut out, "byline", s);
    }
    if let Some(s) = article.excerpt.as_deref() {
        add_field(&mut out, "excerpt", s);
    }
    if let Some(s) = article.site_name.as_deref() {
        add_field(&mut out, "site_name", s);
    }
    if let Some(s) = article.image.as_deref() {
        add_field(&mut out, "image", s);
    }
    if let Some(s) = article.favicon.as_deref() {
        add_field(&mut out, "favicon", s);
    }
    if let Some(s) = article.lang.as_deref() {
        add_field(&mut out, "language", s);
    }
    if let Some(s) = article.published_time.as_deref() {
        add_field(&mut out, "published_time", s);
    }
    if let Some(s) = article.modified_time.as_deref() {
        add_field(&mut out, "modified_time", s);
    }
    if let Some(s) = article.dir.as_deref() {
        add_field(&mut out, "dir", s);
    }
    if article.length > 0 {
        out.push_str(&format!("text_length: {}\n", article.length));
    }
    out.push_str("---\n\n");

    // 6. Optional H1 prepend.
    if !article.title.is_empty() && !body_starts_with_h1(&md_body) {
        out.push_str("# ");
        out.push_str(&article.title);
        out.push_str("\n\n");
    }
    out.push_str(&md_body);

    // 7. Write.
    fs::write(md_path, out).with_context(|| format!("failed to write corpus: {md_path:?}"))?;
    Ok(())
}

/// Generic chrome-strip baseline — drop HTML5 semantic chrome
/// (`nav`/`header`/`footer`/`aside`), `[role=…]` aria landmarks, and
/// `<script>`/`<style>`/`<noscript>` from the document. Then return the
/// `<body>` inner HTML (or the whole document if there's no body).
///
/// Also strips MediaWiki / Wikipedia / Fandom in-content noise classes.
/// The `.mw-*` prefix is unambiguous, and the remaining names are
/// sufficiently MW-specific in practice that collateral matches on
/// non-MW sites are negligible.
fn extract_chrome_stripped(html: &str) -> Option<String> {
    let doc = dom_query::Document::from(html);

    for sel in &[
        // Generic HTML5 chrome.
        "script",
        "style",
        "noscript",
        "nav",
        "header",
        "footer",
        "aside",
        "[role=navigation]",
        "[role=banner]",
        "[role=contentinfo]",
        "[role=complementary]",
        "[hidden]",
        "[aria-hidden=true]",
        // MediaWiki / Wikipedia / Fandom in-content noise.
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
    ] {
        doc.select(sel).remove();
    }

    let body = doc.select("body");
    let html_out = if body.exists() {
        body.inner_html().to_string()
    } else {
        doc.html().to_string()
    };
    if html_out.trim().is_empty() {
        return None;
    }
    Some(html_out)
}

/// Default body extractor. Falls back to an empty string if even
/// chrome-strip yields nothing parseable.
fn default_chrome_strip(html: &str) -> String {
    extract_chrome_stripped(html).unwrap_or_default()
}

fn body_starts_with_h1(body: &str) -> bool {
    body.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim_start().starts_with("# "))
        .unwrap_or(false)
}

fn yaml_str(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    let escaped = one_line.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn add_field(out: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("{key}: {}\n", yaml_str(value)));
    }
}
