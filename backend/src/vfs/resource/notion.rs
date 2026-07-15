use async_trait::async_trait;
use serde_json::{Value, json};

use crate::vfs::{
    accessor::{NotionAccessor, NotionConfig},
    error::{VfsError, VfsResult},
    path::VPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

/// Extract `(mtime, ctime)` from a rendered `page.json`: Notion's
/// `last_edited_time` → mtime, `created_time` → ctime (the nearest ctime analog
/// the API exposes). Notion has no access time, so atime stays `None`.
fn page_times(page_json: &[u8]) -> (Option<std::time::SystemTime>, Option<std::time::SystemTime>) {
    let Ok(v) = serde_json::from_slice::<Value>(page_json) else {
        return (None, None);
    };
    let t = |key: &str| v.get(key).and_then(|x| x.as_str()).and_then(rfc3339_to_systemtime);
    (t("last_edited_time"), t("created_time"))
}

/// Parse an RFC 3339 timestamp into a `SystemTime` (pre-epoch → `None`).
fn rfc3339_to_systemtime(s: &str) -> Option<std::time::SystemTime> {
    let secs = chrono::DateTime::parse_from_rfc3339(s).ok()?.timestamp();
    (secs >= 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

const NOTION_PROMPT: &str = "\
Notion (read + write). Mirrors the workspace page tree:
  pages/<title>__<page-id>/page.json     — page metadata, markdown body, raw blocks
  pages/<title>__<page-id>/<child>__<id>/ — nested sub-pages, recursively
`pages/` lists only top-level (workspace) pages; descend a page dir to find its
sub-pages. The <page-id> is the part after the last `__`.
Domain writes (write JSON to the control path):
  echo '{\"parent\":{\"page_id\":\"ID\"},\"properties\":{\"title\":[{\"text\":{\"content\":\"T\"}}]}}' > .cmd/page-create
  echo '{\"block_id\":\"ID\",\"children\":[{\"object\":\"block\",\"type\":\"paragraph\",\"paragraph\":{\"rich_text\":[{\"type\":\"text\",\"text\":{\"content\":\"hi\"}}]}}]}' > .cmd/block-append";

pub struct NotionResource {
    accessor: NotionAccessor,
}

impl NotionResource {
    pub fn new(config: &NotionConfig) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: NotionAccessor::new(config)?,
        })
    }

    /// Top-level (workspace) pages as `<title>__<id>` directory entries,
    /// carrying each page's `last_edited_time`/`created_time`.
    async fn top_level_page_dirs(&self) -> anyhow::Result<Vec<DirEntry>> {
        let pages = self.accessor.search_pages().await?;
        Ok(pages
            .iter()
            .filter(|p| {
                p.get("parent")
                    .and_then(|x| x.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("workspace")
            })
            .map(|p| {
                dir_t(
                    &page_dirname(p),
                    page_time(p, "last_edited_time"),
                    page_time(p, "created_time"),
                )
            })
            .collect())
    }

    /// Contents of a page directory: its `page.json` plus a subdirectory per
    /// `child_page` block.
    async fn page_dir_entries(&self, page_id: &str) -> anyhow::Result<Vec<DirEntry>> {
        let blocks = self.accessor.list_children(page_id).await?;
        let mut out = vec![file("page.json", 0)];
        for b in &blocks {
            if b.get("type").and_then(|t| t.as_str()) != Some("child_page") {
                continue;
            }
            let child_title = b
                .get("child_page")
                .and_then(|c| c.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("untitled");
            let child_id = b.get("id").and_then(|i| i.as_str()).unwrap_or("");
            out.push(dir_t(
                &format!("{}__{}", sanitize_name(child_title), child_id),
                page_time(b, "last_edited_time"),
                page_time(b, "created_time"),
            ));
        }
        Ok(out)
    }

    async fn render_page_json(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let page = self.accessor.get_page(id).await?;
        let blocks = self.accessor.list_block_tree(id).await?;
        let normalized = normalize_page(&page, &blocks);
        Ok(serde_json::to_vec_pretty(&normalized)?)
    }
}

#[async_trait]
impl Resource for NotionResource {
    async fn read_bytes(
        &self,
        path: &VPath,
        range: Option<std::ops::Range<u64>>,
    ) -> VfsResult<Vec<u8>> {
        let segs = segments(path);
        if segs.len() >= 3
            && segs[0] == "pages"
            && segs.last().map(String::as_str) == Some("page.json")
        {
            let id = page_id(&segs[segs.len() - 2]);
            let data = self.render_page_json(&id).await?;
            return Ok(slice(data, range));
        }
        Err(VfsError::NotFound)
    }

    async fn write_bytes(&self, _path: &VPath, _data: Vec<u8>) -> VfsResult<()> {
        // Notion is read-only for file writes; domain writes go through the
        // `.cmd/` control path (see [`Self::command`]).
        Err(VfsError::Unsupported)
    }

    async fn readdir(&self, path: &VPath) -> VfsResult<Vec<DirEntry>> {
        let segs = segments(path);
        match segs.as_slice() {
            [] => Ok(vec![dir("pages")]),
            [p] if p == "pages" => self.top_level_page_dirs().await.map_err(VfsError::from),
            [p, rest @ ..] if p == "pages" && !rest.is_empty() => {
                let last = rest.last().unwrap();
                if last == "page.json" {
                    return Err(VfsError::NotFound);
                }
                self.page_dir_entries(&page_id(last))
                    .await
                    .map_err(VfsError::from)
            }
            _ => Err(VfsError::NotFound),
        }
    }

    async fn stat(&self, path: &VPath) -> VfsResult<FileStat> {
        let segs = segments(path);
        match segs.as_slice() {
            [] | [_] => Ok(FileStat {
                kind: FileKind::Dir,
                ..Default::default()
            }),
            [p, rest @ ..] if p == "pages" && !rest.is_empty() => {
                if rest.last().map(String::as_str) == Some("page.json") {
                    // The enclosing page dir is the segment before page.json;
                    // a bare /pages/page.json has none — NotFound, not an
                    // index underflow on rest[rest.len() - 2].
                    let Some(dir) = rest.iter().nth_back(1) else {
                        return Err(VfsError::NotFound);
                    };
                    let id = page_id(dir);
                    let bytes = self.render_page_json(&id).await?;
                    let (mtime, ctime) = page_times(&bytes);
                    Ok(FileStat {
                        kind: FileKind::File,
                        size: bytes.len() as u64,
                        mtime,
                        ctime,
                        ..Default::default()
                    })
                } else {
                    // N1: don't blindly report a page dir as existing — verify the
                    // page is real by rendering it (also reused for the times). A
                    // render failure propagates as a backend error rather than a
                    // flat NotFound, so a transient rate-limit isn't misreported as
                    // a missing page.
                    let id = page_id(rest.last().unwrap());
                    let bytes = self.render_page_json(&id).await?;
                    let (mtime, ctime) = page_times(&bytes);
                    Ok(FileStat {
                        kind: FileKind::Dir,
                        mtime,
                        ctime,
                        ..Default::default()
                    })
                }
            }
            _ => Err(VfsError::NotFound),
        }
    }

    async fn command(&self, name: &str, body: &[u8]) -> VfsResult<Vec<u8>> {
        let v: Value = serde_json::from_slice(body)
            .map_err(|e| VfsError::Backend(anyhow::anyhow!("notion {name}: invalid JSON body: {e}")))?;
        let result = match name {
            "page-create" => self.accessor.create_page(v).await?,
            "block-append" => {
                let block_id = v
                    .get("block_id")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| anyhow::anyhow!("block-append: missing block_id"))?;
                let children = v
                    .get("children")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("block-append: missing children"))?;
                self.accessor.append_blocks(block_id, children).await?
            }
            "comment-add" => self.accessor.add_comment(v).await?,
            _ => return Err(VfsError::Unsupported),
        };
        Ok(serde_json::to_vec(&result)?)
    }

    fn prompt(&self) -> &str {
        NOTION_PROMPT
    }
}

fn segments(path: &VPath) -> Vec<String> {
    path.as_str()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Page id encoded as the part after the last `__` in a directory name.
fn page_id(dir_name: &str) -> String {
    dir_name
        .rsplit_once("__")
        .map(|(_, id)| id)
        .unwrap_or(dir_name)
        .to_string()
}

/// Directory name for a page: `<sanitized-title>__<id>`, falling back to
/// `untitled` when the page has no title.
fn page_dirname(page: &Value) -> String {
    let title = extract_title(page);
    let id = page.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let label = if title.is_empty() {
        "untitled".to_string()
    } else {
        sanitize_name(&title)
    };
    format!("{label}__{id}")
}

/// Concatenated plain text of the page's `title` property (returns "" when
/// there is no title property).
fn extract_title(page: &Value) -> String {
    let props = match page.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return String::new(),
    };
    for prop in props.values() {
        if prop.get("type").and_then(|t| t.as_str()) == Some("title") {
            return prop
                .get("title")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
                        .collect::<String>()
                })
                .unwrap_or_default();
        }
    }
    String::new()
}

/// Page metadata + markdown body + raw blocks. `child_page`/`child_database`
/// blocks are excluded from both `markdown` and `blocks` (they surface as
/// subdirectories instead).
fn normalize_page(page: &Value, blocks: &[Value]) -> Value {
    let parent = page.get("parent").cloned().unwrap_or_else(|| json!({}));
    let parent_type = parent.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let parent_id = parent
        .get(parent_type)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let content_blocks: Vec<Value> = blocks
        .iter()
        .filter(|b| {
            let t = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
            t != "child_page" && t != "child_database"
        })
        .cloned()
        .collect();
    json!({
        "page_id": page.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        "title": extract_title(page),
        "url": page.get("url").and_then(|v| v.as_str()).unwrap_or(""),
        "created_time": page.get("created_time").and_then(|v| v.as_str()).unwrap_or(""),
        "last_edited_time": page.get("last_edited_time").and_then(|v| v.as_str()).unwrap_or(""),
        "parent_type": parent_type,
        "parent_id": parent_id,
        "archived": page.get("archived").and_then(|v| v.as_bool()).unwrap_or(false),
        "created_by": page.get("created_by").and_then(|c| c.get("id")).and_then(|v| v.as_str()).unwrap_or(""),
        "last_edited_by": page.get("last_edited_by").and_then(|c| c.get("id")).and_then(|v| v.as_str()).unwrap_or(""),
        "markdown": blocks_to_markdown(&content_blocks),
        "blocks": content_blocks,
    })
}

/// Sanitize a name for use in a virtual path segment: keep word chars /
/// whitespace / `-` / `.`, replace the rest with `_`, fold spaces and runs of
/// `_`, strip, cap at 100 chars.
fn sanitize_name(name: &str) -> String {
    if name.trim().is_empty() {
        return "unknown".to_string();
    }
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c.is_whitespace() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.replace(' ', "_");
    // Collapse runs of '_' into a single '_'.
    let mut folded = String::with_capacity(cleaned.len());
    let mut prev_underscore = false;
    for c in cleaned.chars() {
        if c == '_' {
            if !prev_underscore {
                folded.push(c);
            }
            prev_underscore = true;
        } else {
            folded.push(c);
            prev_underscore = false;
        }
    }
    let trimmed = folded.trim_matches('_');
    trimmed.chars().take(100).collect()
}

fn rich_text_to_md(list: &[Value]) -> String {
    let mut parts = String::new();
    for rt in list {
        let mut text = rt
            .get("plain_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let flag = |k: &str| {
            rt.get("annotations")
                .and_then(|a| a.get(k))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };
        if flag("code") {
            text = format!("`{text}`");
        }
        if flag("bold") {
            text = format!("**{text}**");
        }
        if flag("italic") {
            text = format!("*{text}*");
        }
        if flag("strikethrough") {
            text = format!("~~{text}~~");
        }
        if let Some(href) = rt.get("href").and_then(|v| v.as_str())
            && !href.is_empty()
        {
            text = format!("[{text}]({href})");
        }
        parts.push_str(&text);
    }
    parts
}

fn block_to_md(block: &Value, indent: usize) -> String {
    let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let content = block.get(btype).cloned().unwrap_or_else(|| json!({}));
    let rich_text = content
        .get("rich_text")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let text = rich_text_to_md(&rich_text);
    let prefix = "  ".repeat(indent);

    match btype {
        "paragraph" => format!("{prefix}{text}"),
        "heading_1" => format!("# {text}"),
        "heading_2" => format!("## {text}"),
        "heading_3" => format!("### {text}"),
        "bulleted_list_item" => format!("{prefix}- {text}"),
        "numbered_list_item" => format!("{prefix}1. {text}"),
        "to_do" => {
            let checked = content
                .get("checked")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let marker = if checked { "x" } else { " " };
            format!("{prefix}- [{marker}] {text}")
        }
        "toggle" => format!("{prefix}<details><summary>{text}</summary></details>"),
        "code" => {
            let language = content
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("```{language}\n{text}\n```")
        }
        "quote" => format!("{prefix}> {text}"),
        "callout" => {
            let icon = content.get("icon");
            let emoji =
                if icon.and_then(|i| i.get("type")).and_then(|t| t.as_str()) == Some("emoji") {
                    icon.and_then(|i| i.get("emoji"))
                        .and_then(|e| e.as_str())
                        .unwrap_or("")
                } else {
                    ""
                };
            format!("{prefix}> {emoji} {text}")
        }
        "divider" => "---".to_string(),
        "image" => {
            let inner = content.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let img = content.get(inner).cloned().unwrap_or_else(|| json!({}));
            let url = img.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let caption = rich_text_to_md(
                &content
                    .get("caption")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default(),
            );
            format!("![{caption}]({url})")
        }
        "bookmark" => {
            let url = content.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let caption = rich_text_to_md(
                &content
                    .get("caption")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default(),
            );
            let label = if caption.is_empty() {
                url.to_string()
            } else {
                caption
            };
            format!("[{label}]({url})")
        }
        "equation" => {
            let expr = content
                .get("expression")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("$${expr}$$")
        }
        "table_of_contents" => "[TOC]".to_string(),
        "child_page" | "child_database" => String::new(),
        _ => {
            if text.is_empty() {
                String::new()
            } else {
                format!("{prefix}{text}")
            }
        }
    }
}

fn walk_block(block: &Value, indent: usize, lines: &mut Vec<String>) {
    let line = block_to_md(block, indent);
    let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if !line.is_empty() || btype == "paragraph" {
        lines.push(line);
    }
    if let Some(children) = block.get("children").and_then(|c| c.as_array()) {
        for child in children {
            walk_block(child, indent + 1, lines);
        }
    }
}

fn blocks_to_markdown(blocks: &[Value]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for b in blocks {
        walk_block(b, 0, &mut lines);
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n\n"))
    }
}

fn slice(data: Vec<u8>, range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len());
            data[start..end].to_vec()
        }
        None => data,
    }
}

fn dir(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::Dir,
        size: 0,
        mtime: None,
        atime: None,
        ctime: None,
    }
}

/// A page directory carrying the page's times (`last_edited_time` → mtime,
/// `created_time` → ctime) so `ls -l` shows real times through the cache.
fn dir_t(
    name: &str,
    mtime: Option<std::time::SystemTime>,
    ctime: Option<std::time::SystemTime>,
) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::Dir,
        size: 0,
        mtime,
        atime: None,
        ctime,
    }
}

/// Read an RFC 3339 timestamp field (e.g. `last_edited_time`) off a Notion
/// page/block object into a `SystemTime`.
fn page_time(v: &Value, key: &str) -> Option<std::time::SystemTime> {
    v.get(key).and_then(|x| x.as_str()).and_then(rfc3339_to_systemtime)
}

fn file(name: &str, size: u64) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::File,
        size,
        mtime: None,
        atime: None,
        ctime: None,
    }
}

#[cfg(test)]
mod live {
    //! Live Notion smoke check for the vendored provider — Step 0 of the
    //! "agent reads my Notion" goal. Proves `NotionResource` reads real pages
    //! through the Notion API, independent of any frontend (WebDAV/FUSE) or the
    //! sandbox. Ignored by default; run explicitly with a real integration
    //! token (which must be shared with the target pages):
    //!
    //!   NOTION_API_KEY=ntn_… cargo test -p agent-k-backend notion_live_read -- --ignored --nocapture
    use super::NotionResource;
    use crate::vfs::{NotionConfig, Resource, VPath, VfsError};

    #[tokio::test]
    #[ignore = "requires NOTION_API_KEY + network"]
    async fn notion_live_read() {
        let api_key =
            std::env::var("NOTION_API_KEY").expect("set NOTION_API_KEY to run this live check");
        let res = NotionResource::new(&NotionConfig { api_key }).expect("build NotionResource");

        // Root lists `pages/`.
        let root = res.readdir(&VPath::root()).await.expect("readdir /");
        println!(
            "root entries: {:?}",
            root.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
        );

        // Top-level (workspace) pages shared with the integration.
        let pages = res
            .readdir(&VPath::new("/pages"))
            .await
            .expect("readdir /pages");
        println!("{} top-level page dir(s):", pages.len());
        for p in &pages {
            println!("  - {}", p.name);
        }

        // Read the first page's rendered page.json to prove content flows through.
        let Some(first) = pages.first() else {
            println!("no pages are shared with this integration — share a page and retry");
            return;
        };
        let page_json = format!("/pages/{}/page.json", first.name);
        let bytes = res
            .read_bytes(&VPath::new(&page_json), None)
            .await
            .expect("read page.json");
        let text = String::from_utf8_lossy(&bytes);
        println!("--- {page_json} ({} bytes) ---", bytes.len());
        println!("{}", &text[..text.len().min(1500)]);
    }

    /// `/pages/page.json` has no enclosing page dir: stat must return NotFound,
    /// not underflow `rest[rest.len() - 2]`. Short-circuits before any render,
    /// so no key/network is needed.
    #[tokio::test]
    async fn stat_pages_page_json_is_not_found_not_panic() {
        let res = NotionResource::new(&NotionConfig {
            api_key: "test".into(),
        })
        .expect("build NotionResource");
        assert!(matches!(
            res.stat(&VPath::new("/pages/page.json")).await,
            Err(VfsError::NotFound)
        ));
    }
}
