use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::vfs::{
    accessor::{GmailAccessor, GmailConfig, encode_b64url},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, LocalResource, Resource},
};

use super::gmail_sync::{account_mirror_dir, mirror_tree};

pub(super) const GMAIL_SUFFIX: &str = ".json";

/// The Gmail mount: a read-gate over the account's **on-disk mailbox mirror**
/// (built by [`super::gmail_sync::sync_gmail_mirror`]). Reads are plain local
/// file serving — `ls`/`grep`/`cat`/`find` never touch the API — while `rm`
/// goes through the API (trash) before touching the mirror, and writes are
/// rejected. Mail becomes visible as the sync writes it (newest first), so a
/// listing taken mid-sync is a prefix of the mailbox, not the whole of it.
pub struct GmailResource {
    accessor: GmailAccessor,
    /// The served tree (`<mirror>/tree/`) as plain local files; `None` when no
    /// mirror root was configured (tests, library users) — serves an empty
    /// mailbox.
    local: Option<LocalResource>,
    tree: Option<PathBuf>,
    /// Label id → display name, fetched once on first `rm`: trashing must
    /// drop every hardlinked appearance of the message across label dirs, and
    /// the message JSON carries label *ids*.
    labels: tokio::sync::Mutex<Option<HashMap<String, String>>>,
}

impl GmailResource {
    /// `mirror_root` is the deployment-level mirror directory (the backend
    /// passes `<data_root>/mirror`); the account's subdir is derived from its
    /// email. `None` disables serving (empty mailbox) — the sync worker is a
    /// separate concern and may still be filling the mirror.
    pub fn new(
        config: &GmailConfig,
        mirror_root: Option<&std::path::Path>,
    ) -> anyhow::Result<Self> {
        let tree = mirror_root.map(|r| mirror_tree(&account_mirror_dir(r, &config.account_email)));
        if let Some(t) = &tree {
            // Pre-sync, the tree may not exist yet; an empty dir serves an
            // empty (but valid) mailbox instead of erroring.
            std::fs::create_dir_all(t)?;
        }
        Ok(Self {
            accessor: GmailAccessor::new(config)?,
            local: tree.clone().map(LocalResource::new),
            tree,
            labels: tokio::sync::Mutex::new(None),
        })
    }

    /// Label id → display (system labels display as their id, user labels as
    /// their name — the same rule the sync uses to name label dirs).
    async fn label_map(&self) -> HashMap<String, String> {
        let mut guard = self.labels.lock().await;
        if let Some(m) = guard.as_ref() {
            return m.clone();
        }
        let map: HashMap<String, String> = match self.accessor.list_labels().await {
            Ok(raw) => raw
                .iter()
                .filter_map(|lb| {
                    let id = lb.get("id").and_then(|x| x.as_str())?;
                    let display = if lb.get("type").and_then(|t| t.as_str()) == Some("system") {
                        id.to_string()
                    } else {
                        lb.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or(id)
                            .to_string()
                    };
                    Some((id.to_string(), display))
                })
                .collect(),
            Err(e) => {
                tracing::warn!("gmail: labels fetch for rm failed: {e}");
                HashMap::new()
            }
        };
        *guard = Some(map.clone());
        map
    }

    /// Remove one message's files from the mirror under every label dir that
    /// carries it (hardlinked appearances), plus its attachment dir; empty
    /// parents are pruned. Best-effort — the API trash already succeeded, and
    /// the incremental sync reconciles anything missed.
    async fn remove_mirror_entries(&self, rm_path: &MountPath, fname: &str) {
        let Some(tree) = &self.tree else { return };
        let seg = segments(rm_path);
        let [_, y, m, _] = seg.as_slice() else { return };
        let att_dir = fname.trim_end_matches(GMAIL_SUFFIX).to_string();

        // Label ids from the message JSON (read before we delete anything).
        let label_ids: Vec<String> = self
            .local
            .as_ref()
            .and_then(|_| std::fs::read(tree.join(rm_path.as_str().trim_start_matches('/'))).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| {
                v.get("labels").and_then(|l| l.as_array()).map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default();
        let map = if label_ids.is_empty() {
            HashMap::new()
        } else {
            self.label_map().await
        };

        let mut dirs: Vec<PathBuf> = label_ids
            .iter()
            .filter_map(|id| map.get(id))
            .map(|display| tree.join(display).join(y).join(m))
            .collect();
        // Always include the path actually rm'ed, even if label mapping failed.
        dirs.push(
            tree.join(rm_path.as_str().trim_start_matches('/'))
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| tree.clone()),
        );
        dirs.sort();
        dirs.dedup();
        for dir in dirs {
            let _ = std::fs::remove_file(dir.join(fname));
            let _ = std::fs::remove_dir_all(dir.join(&att_dir));
            // Prune now-empty month/year/label dirs (never the tree root).
            let mut cur = dir;
            while cur.starts_with(tree) && cur != *tree {
                if std::fs::remove_dir(&cur).is_err() {
                    break;
                }
                match cur.parent() {
                    Some(p) => cur = p.to_path_buf(),
                    None => break,
                }
            }
        }
    }
}

#[async_trait]
impl Resource for GmailResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<std::ops::Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        match &self.local {
            Some(l) => l.read_bytes(path, range).await,
            None => Err(ResourceError::NotFound),
        }
    }

    async fn read_bytes_pinned(
        &self,
        path: &MountPath,
        range: Option<std::ops::Range<u64>>,
        stat: &FileStat,
    ) -> ResourceResult<Vec<u8>> {
        match &self.local {
            Some(l) => l.read_bytes_pinned(path, range, stat).await,
            None => Err(ResourceError::NotFound),
        }
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        // The mirror is read-only for the guest; mail mutations go through the
        // API (rm → trash below; send stays dormant scaffolding in command()).
        Err(ResourceError::Unsupported)
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        match &self.local {
            Some(l) => l.readdir(path).await,
            None if path.is_root() => Ok(Vec::new()),
            None => Err(ResourceError::NotFound),
        }
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        match &self.local {
            Some(l) => l.stat(path).await,
            None if path.is_root() => Ok(FileStat {
                kind: FileKind::Dir,
                ..Default::default()
            }),
            None => Err(ResourceError::NotFound),
        }
    }

    /// `rm <…>.json` moves the message to Trash — API first, mirror
    /// cleanup after, so a scope rejection (403 under `gmail.readonly`)
    /// leaves the mirror untouched. `gmail.modify` activates it, no code
    /// change.
    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        let seg = segments(path);
        match seg.as_slice() {
            [_label, _year, _month, file] if file.ends_with(GMAIL_SUFFIX) => {
                let id = id_from_name(file.trim_end_matches(GMAIL_SUFFIX));
                self.accessor.trash(&id).await?;
                self.remove_mirror_entries(path, file).await;
                Ok(())
            }
            _ => Err(ResourceError::Unsupported),
        }
    }

    /// Domain write commands (`send` / `reply` / `reply-all` / `forward`),
    /// designed to hang off the (not yet wired) `.cmd/` control path. Dormant
    /// today — the read-only `gmail.readonly` provisioning 403s them — but
    /// kept for the planned write-scoped flow rather than removed.
    async fn command(&self, name: &str, body: &[u8]) -> ResourceResult<Vec<u8>> {
        let v: Value = serde_json::from_slice(body).map_err(|e| {
            ResourceError::Backend(anyhow::anyhow!("gmail {name}: invalid JSON: {e}"))
        })?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        let result = match name {
            "send" => {
                let to = s("to")
                    .ok_or_else(|| ResourceError::Backend(anyhow::anyhow!("send: missing to")))?;
                let subject = s("subject").unwrap_or_default();
                let body = s("body").unwrap_or_default();
                let raw = encode_b64url(&build_mime(&to, None, &subject, &body, &[]));
                self.accessor.send_raw(&raw, None).await?
            }
            "reply" | "reply-all" => {
                let mid = s("message_id").ok_or_else(|| {
                    ResourceError::Backend(anyhow::anyhow!("{name}: missing message_id"))
                })?;
                let body = s("body").unwrap_or_default();
                let orig = self.accessor.get_message_full(&mid).await?;
                let thread_id = orig.get("threadId").and_then(|t| t.as_str());
                let mut subject = header(&orig, "Subject");
                if !subject.to_lowercase().starts_with("re:") {
                    subject = format!("Re: {subject}");
                }
                let sender = header(&orig, "From");
                let to = if name == "reply-all" {
                    let orig_to = header(&orig, "To");
                    [sender.as_str(), orig_to.as_str()]
                        .iter()
                        .filter(|x| !x.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    sender
                };
                let cc = if name == "reply-all" {
                    let c = header(&orig, "Cc");
                    if c.is_empty() { None } else { Some(c) }
                } else {
                    None
                };
                let msg_id_hdr = header(&orig, "Message-ID");
                let mut extra: Vec<(&str, String)> = Vec::new();
                if let Some(cc) = &cc {
                    extra.push(("Cc", cc.clone()));
                }
                if !msg_id_hdr.is_empty() {
                    extra.push(("In-Reply-To", msg_id_hdr.clone()));
                    extra.push(("References", msg_id_hdr.clone()));
                }
                let raw = encode_b64url(&build_mime(&to, None, &subject, &body, &extra));
                self.accessor.send_raw(&raw, thread_id).await?
            }
            "forward" => {
                let mid = s("message_id").ok_or_else(|| {
                    ResourceError::Backend(anyhow::anyhow!("forward: missing message_id"))
                })?;
                let to = s("to").ok_or_else(|| {
                    ResourceError::Backend(anyhow::anyhow!("forward: missing to"))
                })?;
                let raw_msg = self.accessor.get_message_full(&mid).await?;
                let p = process_message(&raw_msg);
                let mut subject = p
                    .get("subject")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if !subject.to_lowercase().starts_with("fwd:") {
                    subject = format!("Fwd: {subject}");
                }
                let from_email = p
                    .get("from")
                    .and_then(|f| f.get("email"))
                    .and_then(|e| e.as_str())
                    .unwrap_or("");
                let date = p.get("date").and_then(|d| d.as_str()).unwrap_or("");
                let orig_subject = p.get("subject").and_then(|s| s.as_str()).unwrap_or("");
                let body_text = p.get("body_text").and_then(|b| b.as_str()).unwrap_or("");
                let fwd = format!(
                    "---------- Forwarded message ----------\nFrom: {from_email}\nDate: {date}\nSubject: {orig_subject}\n\n{body_text}"
                );
                let raw = encode_b64url(&build_mime(&to, None, &subject, &fwd, &[]));
                self.accessor.send_raw(&raw, None).await?
            }
            other => {
                return Err(ResourceError::Backend(anyhow::anyhow!(
                    "unknown gmail command: {other}"
                )));
            }
        };
        Ok(serde_json::to_vec(&result)?)
    }

    fn prompt(&self) -> &str {
        GMAIL_PROMPT
    }
}

// ---- path helpers ---------------------------------------------------------

/// Mount-relative path segments (`/INBOX/2026/05/x__id.json` -> 4).
fn segments(path: &MountPath) -> Vec<String> {
    path.as_str()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The message id encoded in a `<subject>__<id>` name (last `__`-separated
/// field; sanitized subjects use single underscores, the separator is `__`).
pub(super) fn id_from_name(name: &str) -> String {
    name.rsplit_once("__")
        .map(|(_, id)| id)
        .unwrap_or(name)
        .to_string()
}

pub(super) fn msg_filename(subject: &str, id: &str) -> String {
    format!("{}__{id}{GMAIL_SUFFIX}", sanitize(subject))
}

pub(super) fn attach_dir_name(subject: &str, id: &str) -> String {
    format!("{}__{id}", sanitize(subject))
}

const TITLE_MAX: usize = 80;
/// Byte ceiling for the title part, on top of the [`TITLE_MAX`] character cap.
/// A filename is capped at 255 **bytes** on the filesystems this deploys to
/// (ext4/XFS), and exceeding it fails the write with `ENAMETOOLONG` — which
/// would abort a sync mid-mailbox. macOS caps at 255 *characters* instead, so
/// a name Linux rejects writes fine on a dev machine; only the byte bound
/// catches it. Leaves ~50 bytes for the `__<id>.json` tail.
const TITLE_MAX_BYTES: usize = 200;

/// Sanitize a subject for use as a path segment:
/// keep word chars / spaces / `-._`, collapse the rest to `_`, spaces->`_`,
/// squeeze repeats, trim, cap length (characters *and* bytes).
fn sanitize(text: &str) -> String {
    if text.trim().is_empty() {
        return "No_Subject".to_string();
    }
    let mut s = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            s.push(ch);
        } else {
            // whitespace and every other disallowed char collapse to `_`
            s.push('_');
        }
    }
    // squeeze repeated underscores
    let mut squeezed = String::with_capacity(s.len());
    let mut prev_us = false;
    for ch in s.chars() {
        if ch == '_' {
            if !prev_us {
                squeezed.push(ch);
            }
            prev_us = true;
        } else {
            squeezed.push(ch);
            prev_us = false;
        }
    }
    let trimmed = squeezed.trim_matches('_');
    let mut out: String = trimmed.chars().collect();
    if out.chars().count() > TITLE_MAX {
        out = out.chars().take(TITLE_MAX - 3).collect::<String>() + "...";
    }
    // Then bound the bytes: `TITLE_MAX` counts characters, and 80 CJK or emoji
    // characters run to 240–320 bytes on their own. Cut on a char boundary so
    // multibyte text is shortened, never corrupted.
    if out.len() > TITLE_MAX_BYTES {
        let keep = TITLE_MAX_BYTES - 3; // room for the ellipsis
        let cut = out
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i <= keep)
            .last()
            .unwrap_or(0);
        out.truncate(cut);
        out.push_str("...");
    }
    if out.is_empty() {
        "No_Subject".to_string()
    } else {
        out
    }
}

// ---- message processing ---------------------------------------------------

pub(super) struct Attach {
    pub(super) filename: String,
    pub(super) attachment_id: String,
    pub(super) size: u64,
    pub(super) mime_type: String,
}

/// The message's received time (`internalDate`, epoch ms) as a `SystemTime` —
/// stamped as mtime on listings/stat so date-based agent tools (`ls -lt`,
/// `find -newermt`) work instead of seeing the epoch.
pub(super) fn msg_time(raw: &Value) -> Option<std::time::SystemTime> {
    let ms = raw
        .get("internalDate")
        .and_then(|d| d.as_str())
        .and_then(|s| s.parse::<u64>().ok())?;
    Some(std::time::UNIX_EPOCH + Duration::from_millis(ms))
}

fn header(raw: &Value, name: &str) -> String {
    raw.get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(|h| h.as_array())
        .into_iter()
        .flatten()
        .find(|h| {
            h.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
        .and_then(|h| h.get("value").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string()
}

/// Readable text body from a payload: prefers `text/plain`, falling back to a
/// `text/html` part stripped to text (see [`html_to_text`]) for HTML-only mail.
fn decode_body(payload: &Value) -> String {
    if let Some(plain) = find_part(payload, "text/plain") {
        return plain;
    }
    if let Some(html) = find_part(payload, "text/html") {
        return html_to_text(&html);
    }
    String::new()
}

/// Depth-first search for the first part of `mime` whose body decodes to
/// non-empty text. `None` if no such part exists (or its base64 is invalid).
fn find_part(payload: &Value, mime: &str) -> Option<String> {
    if payload.get("mimeType").and_then(|m| m.as_str()) == Some(mime)
        && let Some(data) = payload
            .get("body")
            .and_then(|b| b.get("data"))
            .and_then(|d| d.as_str())
        && !data.is_empty()
        && let Some(text) = decode_b64url_str(data)
        && !text.trim().is_empty()
    {
        return Some(text);
    }
    if let Some(parts) = payload.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            if let Some(t) = find_part(part, mime) {
                return Some(t);
            }
        }
    }
    None
}

/// Decode a Gmail base64url body payload to a (lossy) UTF-8 string, tolerating
/// missing padding. `None` only if the base64 itself is invalid.
fn decode_b64url_str(data: &str) -> Option<String> {
    let trimmed = data.trim_end_matches('=');
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, trimmed)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// HTML → plain text for an email body, via `html2text` (html5ever-backed):
/// drops `<script>`/`<style>`, decodes entities, tolerates malformed markup.
/// `raw_mode` linearises the layout tables marketing emails are built from
/// (single column, no borders/padding) so they don't become token noise;
/// `TrivialDecorator` keeps it markup-free and `&nbsp;` becomes a space. A
/// final [`tidy_lines`] pass trims trailing space and collapses blank runs.
fn html_to_text(html: &str) -> String {
    let rendered = html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
        .raw_mode(true)
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_default()
        .replace('\u{a0}', " ");
    tidy_lines(&rendered)
}

/// General whitespace hygiene on the rendered text: trim each line's trailing
/// space and collapse runs of blank lines to a single separator.
fn tidy_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_blank = false;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            pending_blank = true;
            continue;
        }
        if pending_blank && !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
        pending_blank = false;
    }
    out.trim_end().to_string()
}

fn parse_address(raw: &str) -> Value {
    let raw = raw.trim();
    if let (Some(lt), Some(gt)) = (raw.find('<'), raw.find('>'))
        && lt < gt
    {
        let name = raw[..lt].trim().trim_matches('"').to_string();
        let email = raw[lt + 1..gt].trim().to_string();
        return json!({ "name": name, "email": email });
    }
    json!({ "name": "", "email": raw })
}

fn parse_address_list(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return json!([]);
    }
    Value::Array(raw.split(',').map(|a| parse_address(a.trim())).collect())
}

/// Sanitize an attachment's filename into a single path segment. The name is
/// sender-controlled (an arbitrary MIME `filename`), so `/`, `\`, and control
/// chars collapse to `_`, and a name that is empty, `.`, or `..` after that
/// falls back to a placeholder — otherwise it would leak past its attachment
/// dir as extra path segments (guest dirents, metadata-cache keys are built by
/// string concatenation). Dotfiles and other ordinary names pass through.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => "attachment".to_string(),
        _ => cleaned,
    }
}

pub(super) fn attachments(raw: &Value) -> Vec<Attach> {
    let mut out = Vec::new();
    let mut push_part = |part: &Value| {
        let filename = part.get("filename").and_then(|f| f.as_str()).unwrap_or("");
        let body = part.get("body");
        let aid = body
            .and_then(|b| b.get("attachmentId"))
            .and_then(|a| a.as_str())
            .unwrap_or("");
        if !filename.is_empty() && !aid.is_empty() {
            out.push(Attach {
                filename: sanitize_filename(filename),
                attachment_id: aid.to_string(),
                size: body
                    .and_then(|b| b.get("size"))
                    .and_then(|s| s.as_u64())
                    .unwrap_or(0),
                mime_type: part
                    .get("mimeType")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    };
    if let Some(parts) = raw
        .get("payload")
        .and_then(|p| p.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            push_part(part);
            if let Some(subs) = part.get("parts").and_then(|p| p.as_array()) {
                for sub in subs {
                    push_part(sub);
                }
            }
        }
    }
    out
}

/// Disambiguate attachment display names within one message. Gmail lets two
/// parts share a `filename` (e.g. two inline `image.png`); left as-is they'd be
/// two identical dir entries and the second would be unreachable (`readdir`
/// lists a name twice, `stat`/`read` match only the first). Keep the first
/// occurrence verbatim and suffix later collisions with ` (n)` before the
/// extension. The part order Gmail returns is stable across `messages.get`, so
/// `readdir`, `stat`, `read`, and the message listing all derive the same
/// unique name for a given part — and the `att_cache` key
/// (`(message id, name)`) becomes per-attachment too.
pub(super) fn unique_attachment_names(atts: &[Attach]) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(atts.len());
    for a in atts {
        let mut name = a.filename.clone();
        let mut n = 1;
        while !used.insert(name.clone()) {
            n += 1;
            name = suffix_before_ext(&a.filename, n);
        }
        out.push(name);
    }
    out
}

/// Insert ` (n)` before a filename's extension: `image.png` -> `image (2).png`.
/// A leading dot (dotfile) isn't treated as an extension separator.
fn suffix_before_ext(name: &str, n: usize) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

/// Build the processed email JSON (the message file content).
pub(super) fn process_message(raw: &Value) -> Value {
    let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
    let body_text = decode_body(&payload);
    // A Drive chip's URL belongs to `drive_files`, not `links` — one URL, one
    // field, so a client showing both doesn't render it twice. The split comes
    // from the same parse, so if chip detection ever fails the URL simply
    // stays in `links`.
    let drive = drive_files(&payload);
    let drive_urls: std::collections::HashSet<&str> =
        drive.iter().filter_map(|d| d["url"].as_str()).collect();
    let links: Vec<String> = body_links(&payload)
        .into_iter()
        .filter(|u| !drive_urls.contains(u.as_str()))
        .collect();
    let atts_raw = attachments(raw);
    let names = unique_attachment_names(&atts_raw);
    let atts: Vec<Value> = atts_raw
        .iter()
        .zip(&names)
        .map(|(a, name)| {
            json!({
                "id": a.attachment_id,
                "filename": name,
                "mime_type": a.mime_type,
                "size": a.size,
            })
        })
        .collect();
    json!({
        "id": raw.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        "thread_id": raw.get("threadId").and_then(|i| i.as_str()).unwrap_or(""),
        "from": parse_address(&header(raw, "From")),
        "to": parse_address_list(&header(raw, "To")),
        "cc": parse_address_list(&header(raw, "Cc")),
        "subject": header(raw, "Subject"),
        "date": header(raw, "Date"),
        "body_text": body_text,
        "links": links,
        "drive_files": drive,
        "snippet": raw.get("snippet").and_then(|s| s.as_str()).unwrap_or(""),
        "labels": raw.get("labelIds").cloned().unwrap_or(json!([])),
        "attachments": atts,
    })
}

/// Cap on `links`: a marketing mail can carry hundreds of tracking URLs, and
/// the field exists to make real destinations reachable, not to mirror every
/// pixel.
const MAX_LINKS: usize = 50;

/// Hyperlink targets from the message's HTML part, in document order and
/// deduped.
///
/// The rendered `body_text` keeps only visible text — an anchor becomes
/// "click here" and its `href` is gone. That loses the *destination* of a mail
/// whose point is a link, most visibly a file too big to attach: Gmail strips
/// attachments over 25 MB and leaves a Drive URL in the body instead, so
/// without this the file is unreachable and ungreppable. Plain-text bodies
/// already carry their URLs inline, so this only reads the HTML part.
fn body_links(payload: &Value) -> Vec<String> {
    let Some(html) = find_part(payload, "text/html") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for (i, _) in html.match_indices("href") {
        let Some(url) = attr_value(&html[i + 4..]) else {
            continue;
        };
        if (url.starts_with("http://") || url.starts_with("https://")) && !out.contains(&url) {
            out.push(url);
            if out.len() >= MAX_LINKS {
                break;
            }
        }
    }
    out
}

/// Files Gmail moved to Drive instead of attaching, as `{filename, url}`.
///
/// Over 25 MB Gmail drops the attachment and writes a Drive link, so the API
/// reports nothing at all — no attachment part, no header, nothing in
/// [`attachments`]. The composer does leave structured markup, and it is the
/// only place the name and its link appear *together*:
///
/// ```text
/// <div class="… gmail_drive_chip"><a href="URL" aria-label="NAME">…<span>NAME</span></a></div>
/// ```
///
/// That class is Gmail's own and undocumented, so treat this as best-effort:
/// if the markup ever changes, this goes empty and the URL still shows up in
/// [`body_links`] — a chip degrades to a link, never to nothing.
fn drive_files(payload: &Value) -> Vec<Value> {
    let Some(html) = find_part(payload, "text/html") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, _) in html.match_indices("gmail_drive_chip") {
        // The chip's anchor is what carries both fields; stop at its close so
        // a later chip's attributes can't be read into this one.
        let chip = &html[i..];
        let chip = &chip[..chip.find("</a>").unwrap_or(chip.len())];
        let Some(url) = attr_in(chip, "href") else {
            continue;
        };
        if !url.starts_with("https://") {
            continue;
        }
        let filename = attr_in(chip, "aria-label")
            .filter(|n| !n.is_empty())
            .or_else(|| span_text(chip))
            .unwrap_or_default();
        if out.len() < MAX_LINKS {
            out.push(json!({ "filename": filename, "url": url }));
        }
    }
    out
}

/// Text inside the first `<span …>…</span>`, the fallback name source when a
/// Drive chip carries no `aria-label`.
fn span_text(s: &str) -> Option<String> {
    let open = s.find("<span")?;
    let start = open + s[open..].find('>')? + 1;
    let end = start + s[start..].find("</span>")?;
    let t = decode_entities(s[start..end].trim());
    (!t.is_empty()).then_some(t)
}

/// Value of attribute `name` in `s` (the first occurrence).
fn attr_in(s: &str, name: &str) -> Option<String> {
    let at = s.find(name)?;
    attr_value(&s[at + name.len()..])
}

/// Parse `="…"` / `='…'` / `=bare` at the head of `s` — i.e. `s` starts right
/// after an attribute's name — and return the entity-decoded value.
fn attr_value(s: &str) -> Option<String> {
    let rest = s.trim_start().strip_prefix('=')?.trim_start();
    let (quote, rest) = match rest.as_bytes().first()? {
        b'"' => ('"', &rest[1..]),
        b'\'' => ('\'', &rest[1..]),
        _ => (' ', rest),
    };
    let end = rest
        .find(|c: char| c == quote || (quote == ' ' && (c == '>' || c.is_whitespace())))
        .unwrap_or(rest.len());
    Some(decode_entities(rest[..end].trim()))
}

/// The handful of XML entities that appear inside an `href` (`&amp;` above all,
/// which HTML-escaped query strings are full of).
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

// ---- MIME build (RFC 2822) ------------------------------------------------

/// Build a minimal text/plain RFC-2822 message. Non-ASCII subjects are RFC-2047
/// encoded so they survive transport.
fn build_mime(
    to: &str,
    from: Option<&str>,
    subject: &str,
    body: &str,
    extra_headers: &[(&str, String)],
) -> Vec<u8> {
    let mut h = String::new();
    if let Some(f) = from {
        h.push_str(&format!("From: {f}\r\n"));
    }
    h.push_str(&format!("To: {to}\r\n"));
    h.push_str(&format!("Subject: {}\r\n", encode_header(subject)));
    for (k, v) in extra_headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str("MIME-Version: 1.0\r\n");
    h.push_str("Content-Type: text/plain; charset=\"utf-8\"\r\n");
    h.push_str("Content-Transfer-Encoding: 8bit\r\n");
    h.push_str("\r\n");
    let mut bytes = h.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// RFC-2047-encode a header value if it contains non-ASCII; else pass through.
fn encode_header(value: &str) -> String {
    if value.is_ascii() {
        return value.to_string();
    }
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value.as_bytes());
    format!("=?UTF-8?B?{b64}?=")
}

// ---- date helpers (dependency-free civil calendar) ------------------------

/// Gmail `internalDate` (epoch ms, string) -> `YYYY-MM-DD` in UTC.
pub(super) fn epoch_ms_to_date(ms: &str) -> String {
    let ms: i64 = ms.parse().unwrap_or(0);
    let days = (ms / 1000).div_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) -> days since 1970-01-01. Howard Hinnant's algorithm.
/// Only the inverse ([`civil_from_days`]) is needed in prod now (date bucketing);
/// this forward direction is kept for the round-trip test.
#[cfg(test)]
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

// ---- small builders -------------------------------------------------------

const GMAIL_PROMPT: &str = "\
Gmail (read + trash on delete). A synced on-disk mirror of the mailbox:
  <label>/<yyyy>/<mm>/<subject>__<message-id>.json   # the email (JSON)
  <label>/<yyyy>/<mm>/<subject>__<message-id>/<filename>   # attachments (only if any)

  <label>       INBOX, SENT, DRAFT, IMPORTANT, STARRED, TRASH, SPAM, or a user label
  <yyyy>/<mm>   received year then month; `ls <label>` lists years, then months,
                then that month's messages (kept small per level)
  <subject>     sanitized subject (don't construct it; ls the month dir)
  <message-id>  Gmail message id (the field after the last `__`)

  cat <…>.json returns:
    {\"id\",\"thread_id\",\"from\":{\"name\",\"email\"},\"to\":[…],\"cc\":[…],
     \"subject\",\"date\",\"body_text\",\"snippet\",\"labels\":[…],
     \"links\":[…],\"drive_files\":[{\"filename\",\"url\"}],
     \"attachments\":[{\"id\",\"filename\",\"mime_type\",\"size\"}]}
  body_text is the visible text only; \"links\" holds the URLs its hyperlinks
  pointed at. A file over 25MB isn't attached at all — Gmail sends a Drive
  link instead, listed in \"drive_files\" (name + url, no bytes to read) and
  kept out of \"links\", so each URL appears once.
  The sibling dir (same name without .json) holds attachment bytes; cat a
  file inside to download it. ENOENT there means the message has no attachments.
  The initial sync fills the mirror newest-first, so until it finishes older
  mail may be missing and the oldest visible month may be partial.


  rm <…>.json    moves the message to Trash (only a message file is removable).";

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::*;

    /// Live mirror round-trip: sync a capped slice of the real mailbox into a
    /// temp mirror, then serve it through the gate resource. Run with:
    ///
    ///   GOOGLE_CLIENT_ID=… GOOGLE_CLIENT_SECRET=… GOOGLE_REFRESH_TOKEN=… \
    ///   [GMAIL_INDEX_CAP=60] cargo test -p workspace gmail_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires GMAIL_* env + network"]
    async fn gmail_live_mirror() {
        use crate::vfs::resource::sync_gmail_mirror;

        let Some(mut cfg) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        if cfg.index_cap.is_none() {
            cfg.index_cap = Some(60); // keep the live probe cheap by default
        }
        let deploy = tempfile::tempdir().unwrap();
        let acct = account_mirror_dir(deploy.path(), &cfg.account_email);

        let t0 = std::time::Instant::now();
        let state = sync_gmail_mirror(&cfg, &acct).await.expect("sync");
        eprintln!(
            "sync: {}/{} messages in {:?} (completed={})",
            state.fetched,
            state.total,
            t0.elapsed(),
            state.completed
        );
        assert!(state.completed);

        let r = GmailResource::new(&cfg, Some(deploy.path())).unwrap();
        let labels = r.readdir(&MountPath::new("/")).await.expect("labels");
        eprintln!(
            "labels ({}): {:?}",
            labels.len(),
            labels.iter().map(|e| &e.name).take(12).collect::<Vec<_>>()
        );
        assert!(!labels.is_empty(), "mirror serves at least one label");

        // Walk newest year/month of the first label that has content.
        let label = labels.first().unwrap().name.clone();
        let years = r
            .readdir(&MountPath::new(format!("/{label}")))
            .await
            .expect("years");
        let y = years.iter().map(|e| e.name.clone()).max().expect("a year");
        let months = r
            .readdir(&MountPath::new(format!("/{label}/{y}")))
            .await
            .expect("months");
        let m = months
            .iter()
            .map(|e| e.name.clone())
            .max()
            .expect("a month");
        let entries = r
            .readdir(&MountPath::new(format!("/{label}/{y}/{m}")))
            .await
            .expect("month listing");
        eprintln!("{label}/{y}/{m}: {} entries", entries.len());
        let f = entries
            .iter()
            .find(|e| e.name.ends_with(GMAIL_SUFFIX))
            .expect("a message file");
        assert!(f.mtime.is_some(), "mirror files carry received-time mtimes");
        let bytes = r
            .read_bytes(
                &MountPath::new(format!("/{label}/{y}/{m}/{}", f.name)),
                None,
            )
            .await
            .expect("cat");
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        eprintln!(
            "cat {}: subject={:?} body_text={}B",
            f.name,
            v["subject"],
            v["body_text"].as_str().unwrap_or("").len()
        );
        assert!(v.get("id").is_some(), "processed JSON shape");

        // Partial sync: the full sync above seeded a history cursor, so an
        // incremental run replays the (typically empty) journal instead of
        // degrading to a full resync.
        use crate::vfs::resource::{GmailSyncState, sync_gmail_incremental};
        let seeded = GmailSyncState::load(&acct).expect("state");
        assert!(seeded.history_id.is_some(), "full sync seeds the cursor");
        let t1 = std::time::Instant::now();
        let delta = sync_gmail_incremental(&cfg, &acct)
            .await
            .expect("incremental");
        eprintln!(
            "incremental: +{} -{} ~{} in {:?} (full_resync={})",
            delta.added,
            delta.deleted,
            delta.relabeled,
            t1.elapsed(),
            delta.full_resync
        );
        assert!(!delta.full_resync, "cursor was honored");
        let after = GmailSyncState::load(&acct).expect("state");
        assert!(after.history_id.is_some() && after.completed);

        // Fallback shape (what an expired cursor degrades to): a full re-sync
        // over an already-complete mirror. Every id short-circuits on the done
        // log, then the label sweep re-checks placements — the mirror must
        // come through complete with a fresh cursor.
        let t2 = std::time::Instant::now();
        let resync = sync_gmail_mirror(&cfg, &acct)
            .await
            .expect("fallback resync");
        eprintln!(
            "fallback resync: {}/{} in {:?}",
            resync.fetched,
            resync.total,
            t2.elapsed()
        );
        assert!(resync.completed && resync.history_id.is_some());
    }

    #[test]
    fn unique_attachment_names_disambiguates_duplicates() {
        let att = |f: &str| Attach {
            filename: f.to_string(),
            attachment_id: String::new(),
            size: 0,
            mime_type: String::new(),
        };
        let atts = [
            att("image.png"),
            att("doc.pdf"),
            att("image.png"),
            att("image.png"),
            att("README"),
            att("README"),
        ];
        assert_eq!(
            unique_attachment_names(&atts),
            vec![
                "image.png", // first occurrence kept verbatim
                "doc.pdf",
                "image (2).png", // suffix before the extension
                "image (3).png",
                "README", // no extension
                "README (2)",
            ]
        );
    }

    #[test]
    fn attachment_filenames_are_confined_to_one_segment() {
        // ordinary names (incl. dotfiles, spaces, unicode) pass through
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("my file (1).png"), "my file (1).png");
        assert_eq!(sanitize_filename(".env"), ".env");
        assert_eq!(sanitize_filename("보고서.pdf"), "보고서.pdf");
        // path separators collapse so the name can't escape its dir
        assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize_filename("a/b.png"), "a_b.png");
        assert_eq!(sanitize_filename("a\\b.png"), "a_b.png");
        // control chars can't reach a dirent
        assert_eq!(sanitize_filename("bad\nname"), "bad_name");
        // degenerate names fall back to a placeholder
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("   "), "attachment");
        assert_eq!(sanitize_filename("."), "attachment");
        assert_eq!(sanitize_filename(".."), "attachment");
        // sanitized collisions are then disambiguated as usual
        let att = |f: &str| Attach {
            filename: sanitize_filename(f),
            attachment_id: String::new(),
            size: 0,
            mime_type: String::new(),
        };
        assert_eq!(
            unique_attachment_names(&[att("a/b.png"), att("a_b.png")]),
            vec!["a_b.png", "a_b (2).png"]
        );
    }

    #[test]
    fn sanitize_subject_rules() {
        assert_eq!(sanitize("Hello World"), "Hello_World");
        assert_eq!(sanitize("Re: [urgent] ping!"), "Re_urgent_ping");
        assert_eq!(sanitize("a / b \\ c"), "a_b_c");
        assert_eq!(sanitize("   "), "No_Subject");
        assert_eq!(sanitize(""), "No_Subject");
        // keeps word chars, dash, dot, underscore
        assert_eq!(sanitize("file-name.v2_final"), "file-name.v2_final");
        // length cap (80) with ellipsis
        let long = "x".repeat(100);
        let s = sanitize(&long);
        assert_eq!(s.chars().count(), 80);
        assert!(s.ends_with("..."));
    }

    /// The char cap alone doesn't bound the *name*: ext4/XFS cap a filename at
    /// 255 **bytes**, and 80 CJK/emoji characters are 240–320 bytes before the
    /// `__<id>.json` tail is added. (macOS caps at 255 characters, so a
    /// dev machine happily writes a name Linux would reject with
    /// ENAMETOOLONG — which aborts the sync mid-mailbox.)
    #[test]
    fn filenames_stay_within_the_filesystem_byte_limit() {
        const NAME_MAX: usize = 255;
        let id = "19f8a471bcd83ca4";
        for subject in [
            "가".repeat(200),                    // Hangul, 3 bytes each
            "🙂".repeat(200),                    // emoji, 4 bytes each
            "日本語のとても長い件名".repeat(30), // CJK mix
            format!("{} {}", "ascii ".repeat(40), "한글".repeat(60)),
        ] {
            let f = msg_filename(&subject, id);
            assert!(
                f.len() <= NAME_MAX,
                "message filename is {} bytes (> {NAME_MAX}): {f}",
                f.len()
            );
            let d = attach_dir_name(&subject, id);
            assert!(
                d.len() <= NAME_MAX,
                "attachment dir is {} bytes (> {NAME_MAX}): {d}",
                d.len()
            );
            // Truncation must not split a character.
            assert!(std::str::from_utf8(f.as_bytes()).is_ok());
        }
        // Short subjects are untouched by the byte bound.
        assert_eq!(
            msg_filename("짧은 제목", id),
            format!("짧은_제목__{id}.json")
        );
    }

    #[test]
    fn id_parsed_from_last_double_underscore() {
        assert_eq!(id_from_name("Subject__abc123"), "abc123");
        // single underscores in the (sanitized) subject don't confuse it
        assert_eq!(id_from_name("a_b_c__99zz"), "99zz");
        assert_eq!(id_from_name("Re_urgent_ping__ff00ab"), "ff00ab");
        // no separator -> the whole name
        assert_eq!(id_from_name("noseparator"), "noseparator");
        // round-trips with the filename builders
        assert_eq!(
            id_from_name(msg_filename("Hi there", "ID42").trim_end_matches(GMAIL_SUFFIX)),
            "ID42"
        );
        assert_eq!(id_from_name(&attach_dir_name("Hi there", "ID42")), "ID42");
    }

    fn live_config() -> Option<GmailConfig> {
        Some(GmailConfig {
            client_id: std::env::var("GOOGLE_CLIENT_ID").ok()?,
            client_secret: std::env::var("GOOGLE_CLIENT_SECRET").ok()?,
            refresh_token: std::env::var("GOOGLE_REFRESH_TOKEN").ok()?,
            // This one *is* read: it names the mirror directory, so a live run against
            // two accounts must not have them share a tree.
            account_email: std::env::var("GMAIL_EMAIL").unwrap_or_else(|_| "live-test".into()),
            origins: match std::env::var("GOOGLE_API_BASE_URL") {
                Ok(host) => crate::vfs::accessor::Origins::behind(&host),
                Err(_) => Default::default(),
            },
            index_cap: std::env::var("GMAIL_INDEX_CAP")
                .ok()
                .and_then(|v| v.parse().ok()),
        })
    }

    /// Full-stack probe against a configurable endpoint (`GOOGLE_API_BASE_URL` →
    /// enterprise mock): labels → year/month navigation → `cat` (asserts real
    /// body text) → attachment bytes → warm relist. With a
    /// base_url set it also checks `rm` against an endpoint lacking trash
    /// support surfaces an error instead of pretending success.
    #[tokio::test]
    async fn unlink_accepts_only_message_files_no_network_needed() {
        let r = GmailResource::new(
            &GmailConfig {
                client_id: "id".into(),
                client_secret: "sec".into(),
                refresh_token: "tok".into(),
                account_email: "t@example.com".into(),
                origins: Default::default(),
                index_cap: None,
            },
            None,
        )
        .unwrap();
        // Every shape that isn't a message file is rejected by the path match
        // alone, before any network I/O (dummy credentials would fail loudly
        // otherwise).
        for p in [
            "/INBOX",
            "/INBOX/2026/07",
            "/INBOX/2026/07/subject__id",          // attachment dir
            "/INBOX/2026/07/subject__id/file.pdf", // attachment file
        ] {
            let got = r.unlink(&MountPath::new(p)).await;
            assert!(
                matches!(got, Err(ResourceError::Unsupported)),
                "{p} must be Unsupported, got {got:?}"
            );
        }
    }

    #[test]
    fn msg_time_from_internal_date() {
        use std::time::{Duration as D, UNIX_EPOCH};
        let raw = json!({ "internalDate": "1777802400000" });
        assert_eq!(
            msg_time(&raw),
            Some(UNIX_EPOCH + D::from_millis(1_777_802_400_000))
        );
        assert_eq!(msg_time(&json!({ "internalDate": "0" })), Some(UNIX_EPOCH));
        // absent or malformed → None (entry falls back to no mtime)
        assert_eq!(msg_time(&json!({})), None);
        assert_eq!(msg_time(&json!({ "internalDate": "bogus" })), None);
    }

    #[test]
    fn epoch_ms_to_date_anchors() {
        assert_eq!(epoch_ms_to_date("0"), "1970-01-01");
        // 2026-05-03T10:00:00Z = 1777800000 s
        assert_eq!(epoch_ms_to_date("1777802400000"), "2026-05-03");
        assert_eq!(epoch_ms_to_date("bogus"), "1970-01-01");
    }

    #[test]
    fn civil_calendar_roundtrips() {
        for &(y, m, d) in &[
            (1970, 1, 1),
            (2000, 2, 29),
            (2026, 5, 3),
            (2027, 1, 1),
            (1999, 12, 31),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m as u32, d as u32));
        }
    }

    #[test]
    /// An HTML-only mail's `href`s survive as `links`. The motivating case is
    /// an over-25 MB file: Gmail replaces the attachment with a Drive URL, and
    /// the rendered text keeps only the anchor's words.
    #[test]
    fn html_hrefs_are_captured_as_links() {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let html = "<p>Your file is ready: \
             <a href=\"https://drive.google.com/file/d/ABC?usp=share&amp;x=1\">Open in Drive</a></p>\
             <p><a href='http://example.com/a'>a</a> \
             <a href=https://example.com/bare>bare</a> \
             <a href=\"mailto:x@y.z\">mail</a> \
             <a href=\"https://drive.google.com/file/d/ABC?usp=share&amp;x=1\">dup</a></p>";
        let payload = json!({
            "mimeType": "multipart/alternative",
            "parts": [{ "mimeType": "text/html", "body": { "data": b64(html) } }],
        });

        // The URL is gone from the prose…
        let body = decode_body(&payload);
        assert!(body.contains("Open in Drive"));
        assert!(!body.contains("drive.google.com"), "got: {body:?}");

        // …but reachable, deduped, entity-decoded, and non-http dropped.
        assert_eq!(
            body_links(&payload),
            vec![
                "https://drive.google.com/file/d/ABC?usp=share&x=1",
                "http://example.com/a",
                "https://example.com/bare",
            ]
        );

        // A plain-text-only mail already has its URLs inline — nothing to add.
        let plain = json!({
            "mimeType": "multipart/alternative",
            "parts": [{ "mimeType": "text/plain", "body": { "data": b64("see https://x.io") } }],
        });
        assert!(body_links(&plain).is_empty());
    }

    /// A >25 MB file arrives as a Drive chip, not an attachment. The markup
    /// below is verbatim from a real send (trimmed of styling), and it is the
    /// only place the filename and its URL are paired.
    #[test]
    fn drive_chips_pair_filename_with_url() {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let html = "<div dir=\"ltr\">25메가 이상 첨부파일 테스트.\
            <div contenteditable=\"false\" class=\"gmail_chip gmail_drive_chip\" style=\"width:386px\">\
            <a href=\"https://drive.google.com/file/d/15qL/view?usp=drive_web\" target=\"_blank\" \
            aria-label=\"OpenWiki_0.3.20_aarch64.dmg\">\
            <img src=\"https://ssl.gstatic.com/docs/doclist/images/icon.png\">\
            <span dir=\"ltr\">OpenWiki_0.3.20_aarch64.dmg</span></a></div>\
            <div class=\"gmail_chip gmail_drive_chip\">\
            <a href=\"https://drive.google.com/file/d/ZZZ/view\"><span>second &amp; last.zip</span></a>\
            </div></div>";
        let payload = json!({
            "mimeType": "multipart/alternative",
            "parts": [{ "mimeType": "text/html", "body": { "data": b64(html) } }],
        });

        assert_eq!(
            drive_files(&payload),
            vec![
                json!({
                    "filename": "OpenWiki_0.3.20_aarch64.dmg",
                    "url": "https://drive.google.com/file/d/15qL/view?usp=drive_web"
                }),
                // No aria-label → falls back to the chip's span text.
                json!({ "filename": "second & last.zip", "url": "https://drive.google.com/file/d/ZZZ/view" }),
            ]
        );
        // Each chip keeps its own url — the second must not inherit the first.
        // The icon's <img src> is not a hyperlink, so it stays out of `links`.
        assert_eq!(
            body_links(&payload),
            vec![
                "https://drive.google.com/file/d/15qL/view?usp=drive_web",
                "https://drive.google.com/file/d/ZZZ/view",
            ]
        );

        // In the assembled message a chip URL lives in `drive_files` only —
        // no client should have to de-duplicate the two fields.
        let msg = process_message(&json!({ "id": "m1", "payload": payload }));
        assert_eq!(msg["links"], json!([]));
        assert_eq!(msg["drive_files"].as_array().unwrap().len(), 2);
        // An ordinary mail has no chips.
        let plain = json!({
            "mimeType": "text/html",
            "body": { "data": b64("<a href=\"https://x.io\">x</a>") },
        });
        assert!(drive_files(&plain).is_empty());
    }

    #[test]
    fn html_to_text_strips_scripts_styles_and_decodes() {
        let html = "<html><head>\
            <style>.x{color:red}</style>\
            <script>alert('nope')</script></head><body>\
            <p>Hello&nbsp;<b>World</b></p>\
            <div>line2 &amp; more &#39;quoted&#39;</div>\
            <!-- drop me --></body></html>";
        let t = html_to_text(html);
        assert!(t.contains("Hello World"), "got: {t:?}");
        assert!(t.contains("line2 & more 'quoted'"), "got: {t:?}");
        assert!(
            !t.contains("color:red"),
            "style content must be dropped: {t:?}"
        );
        assert!(
            !t.contains("alert"),
            "script content must be dropped: {t:?}"
        );
        assert!(!t.contains("drop me"), "comments must be dropped: {t:?}");
        assert!(!t.contains('<'), "tags must be stripped: {t:?}");
    }

    #[test]
    fn html_to_text_linearises_tables_without_border_noise() {
        // Marketing emails are built from layout tables; `raw_mode` must render
        // them as linear text — no box-drawing borders, cell padding, or blank
        // runs (all pure token noise for the agent).
        let html = "<table><tr><td>Line A</td></tr><tr><td>Line B</td></tr></table>";
        let t = html_to_text(html);
        assert_eq!(t, "Line A\nLine B", "got: {t:?}");
        assert!(
            !t.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
            "table-border box chars leaked: {t:?}"
        );
    }

    #[test]
    fn html_to_text_preserves_utf8_and_survives_malformed() {
        // Non-ASCII text is preserved; an unclosed tag doesn't panic.
        assert_eq!(html_to_text("<p>안녕 <b>세계"), "안녕 세계");
        assert_eq!(html_to_text("plain, no tags"), "plain, no tags");
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn decode_body_prefers_plain_then_falls_back_to_html() {
        let b64 = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);

        // text/plain present → used verbatim (html ignored).
        let both = json!({
            "mimeType": "multipart/alternative",
            "parts": [
                {"mimeType": "text/plain", "body": {"data": b64("plain body")}},
                {"mimeType": "text/html",  "body": {"data": b64("<p>html body</p>")}},
            ]
        });
        assert_eq!(decode_body(&both).trim(), "plain body");

        // html-only → stripped to text.
        let html_only = json!({
            "mimeType": "multipart/alternative",
            "parts": [
                {"mimeType": "text/html", "body": {"data": b64("<p>only <i>html</i></p>")}},
            ]
        });
        assert_eq!(decode_body(&html_only).trim(), "only html");

        // neither → empty.
        let none = json!({ "mimeType": "multipart/mixed", "parts": [] });
        assert_eq!(decode_body(&none), "");
    }

    #[test]
    fn process_message_shapes_the_email() {
        // a minimal raw Gmail message with a plain-text body and one attachment
        let body_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello body");
        let raw = json!({
            "id": "m1",
            "threadId": "t1",
            "snippet": "hello",
            "labelIds": ["INBOX", "IMPORTANT"],
            "payload": {
                "headers": [
                    {"name": "From", "value": "Alice <alice@example.com>"},
                    {"name": "To", "value": "bob@example.com, carol@example.com"},
                    {"name": "Subject", "value": "Hi"},
                    {"name": "Date", "value": "Mon, 3 May 2026 10:00:00 -0700"}
                ],
                "parts": [
                    {"mimeType": "text/plain", "body": {"data": body_b64}},
                    {"filename": "a.pdf", "mimeType": "application/pdf",
                     "body": {"attachmentId": "att1", "size": 12345}}
                ]
            }
        });
        let p = process_message(&raw);
        assert_eq!(p["id"], "m1");
        assert_eq!(p["thread_id"], "t1");
        assert_eq!(p["from"]["name"], "Alice");
        assert_eq!(p["from"]["email"], "alice@example.com");
        assert_eq!(p["to"].as_array().unwrap().len(), 2);
        assert_eq!(p["subject"], "Hi");
        assert_eq!(p["body_text"], "hello body");
        assert_eq!(p["labels"], json!(["INBOX", "IMPORTANT"]));
        let atts = p["attachments"].as_array().unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["filename"], "a.pdf");
        assert_eq!(atts[0]["id"], "att1");
        assert_eq!(atts[0]["size"], 12345);
    }

    #[test]
    fn build_mime_encodes_nonascii_subject() {
        let bytes = build_mime("to@x.com", None, "안녕 hi", "body", &[]);
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("To: to@x.com\r\n"));
        assert!(
            s.contains("Subject: =?UTF-8?B?"),
            "non-ascii subject must be RFC-2047 encoded"
        );
        assert!(s.ends_with("body"));
        // ascii subject passes through
        let bytes = build_mime("to@x.com", None, "Plain", "b", &[]);
        assert!(String::from_utf8_lossy(&bytes).contains("Subject: Plain\r\n"));
    }
}
