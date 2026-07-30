use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::vfs::{
    accessor::{GdriveAccessor, GdriveConfig},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// Suffix on every file entry: the mount serves a metadata *card* (JSON, with
/// the Drive link inside), not the file's bytes, and the name has to say so — a
/// `report.pdf` whose content is JSON would be a lie. `report.pdf` →
/// `report.pdf.json`. Same plain `.json` the gmail mirror settled on.
const CARD_SUFFIX: &str = ".json";

/// The mount root mirrors Drive's own sidebar as virtual sections. `My Drive` is
/// the literal Drive folder id `root`; `Shared with me` is a sentinel resolved
/// to a `sharedWithMe=true` listing (shared items carry no `parents`, so the
/// folder tree alone would never surface them). The sentinel contains `@`, which
/// the Drive id alphabet (`[A-Za-z0-9_-]`) never does, so it can't collide with
/// a real file id. Names are Drive's own English section labels.
const MY_DRIVE_ID: &str = "root";
const MY_DRIVE_NAME: &str = "My Drive";
const SHARED_WITH_ME_ID: &str = "@sharedWithMe";
const SHARED_WITH_ME_NAME: &str = "Shared with me";

/// Suffix on the text sidecar: a second entry beside the card, holding the
/// document's text as plain lines. Text cannot live *in* the card — JSON escapes
/// every newline, so the whole document would be one line, unmatchable by a
/// grep spanning a line break and unreadable by `cat`.
const TEXT_SUFFIX: &str = ".txt";

/// Native types Drive converts to text, and to what. Sheets go to CSV (Drive
/// exports the first tab only — a documented Google limitation); Docs and Slides
/// to plain text. Drawings/Forms/Maps have no text form at all.
const TEXT_EXPORTS: &[(&str, &str)] = &[
    ("application/vnd.google-apps.document", "text/plain"),
    ("application/vnd.google-apps.presentation", "text/plain"),
    ("application/vnd.google-apps.spreadsheet", "text/csv"),
];

/// A blob is served as its own text only when it already *is* text and Drive
/// reported a size small enough that one read can't blow up.
const BLOB_TEXT_MAX: u64 = 1024 * 1024;

/// Per-directory listing TTL. A listing is one `files.list`, so a short TTL
/// keeps `ls` fresh; path resolution walks parent listings and rides this cache.
const DIR_TTL: Duration = Duration::from_secs(60);
/// Safety ceiling on one folder's listing (10 pages). Beyond this the listing
/// truncates (the accessor logs it) — a >10k-child folder is pathological to
/// `ls` anyway.
const MAX_FOLDER_FILES: usize = 10_000;

const GDRIVE_PROMPT: &str = "\
Google Drive (read-only, metadata + links). Layout mirrors Drive's own sidebar:
  My Drive/…              # the account's folder tree; descend with ls
  Shared with me/         # everything shared to this account
  <shared drive>/         # each shared drive, when the account has any

  Folders are directories. Every file appears as `<drive name>.json` holding
  only its metadata card — NOT the file's bytes:
    {\"name\",\"id\",\"mime_type\",\"size\",\"modified_time\",\"created_time\",
     \"owner\":{\"name\",\"email\"},\"web_view_link\"}
  `size` is the file's real size in Drive when Drive reports one (folders and
  shortcuts have none); the entry itself is only as big as the card. To let
  someone see the original, hand them `web_view_link` — this mount serves no
  original bytes. Duplicate names get ` (2)`, ` (3)`, ….

  Google Docs, Sheets, Slides and small already-text files ALSO get a sibling
  `<drive name>.txt` holding the document's text (Sheets: first tab, as CSV).
  That file is the only content here: `cat` it to read a document, and
  `Glob **/*.txt` lists everything whose contents are readable at all. PDFs,
  Office files, hwp, archives, images and video have no `.txt` — find those by
  name, mime_type or owner, then hand over `web_view_link`.

  Reading a `.txt` converts the document through Drive: expect a few seconds on
  the first read of each (cached after), and expect a listing to report its size
  as 0 until then. So `grep -r` over a whole folder pays that per document — grep
  a narrowed subtree, or match on names and cards first.
  The mount is read-only: no writes, no rm, no mkdir.";

/// How a file's text is obtained, when it can be at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TextSource {
    /// A native Workspace doc converted by Drive (`files.export`).
    Export(&'static str),
    /// A file that is already text — fetched as-is (`alt=media`).
    Download,
}

/// How (or whether) this row's text can be fetched. Everything else — PDFs,
/// Office files, hwp, archives, images, video, Forms/Maps — has no text form
/// here, and gets no sidecar.
fn text_source(f: &Value, mime: &str) -> Option<TextSource> {
    if let Some((_, target)) = TEXT_EXPORTS.iter().find(|(m, _)| *m == mime) {
        return Some(TextSource::Export(target));
    }
    let is_text = mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/x-yaml"
        );
    let size = f
        .get("size")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u64>().ok());
    match (is_text, size) {
        (true, Some(n)) if n <= BLOB_TEXT_MAX => Some(TextSource::Download),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GKind {
    Folder,
    SharedDrive,
    File,
}

impl GKind {
    fn is_dir(self) -> bool {
        matches!(self, GKind::Folder | GKind::SharedDrive)
    }
}

/// One resolved Drive entry, as the VFS sees it.
#[derive(Clone)]
struct Child {
    /// Listing name: the sanitized (and, on collision, disambiguated) Drive
    /// name, with [`CARD_SUFFIX`] appended for files.
    vfs_name: String,
    id: String,
    /// Set when the entry lives in a shared drive (listing scope).
    drive_id: Option<String>,
    kind: GKind,
    /// Drive's own `mimeType`. The entry's *bytes* are always a JSON card, so
    /// this is the only place the original type survives — and for the
    /// Docs-editors types it is the only place it exists at all, since those
    /// names carry no extension.
    mime_type: Option<String>,
    mtime: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    /// The metadata card this entry reads as — built once from the listing row
    /// (see [`card_bytes`]). `Null` for directories, and for a text sidecar,
    /// whose content is fetched on read.
    card: Value,
    /// Set on a `.txt` sidecar entry: how to fetch the text it serves.
    text: Option<TextSource>,
}

pub struct GdriveResource {
    accessor: GdriveAccessor,
    /// Per-directory listing cache (folder path → children). Path resolution
    /// walks parent listings, so this serves resolve/stat/read as well — reads
    /// render from a `Child` and never hit the network.
    dir_cache: Mutex<HashMap<String, (Instant, Vec<Child>)>>,
}

impl GdriveResource {
    pub fn new(config: &GdriveConfig) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: GdriveAccessor::new(config)?,
            dir_cache: Mutex::new(HashMap::new()),
        })
    }

    /// The mount root's virtual sections: `My Drive`, `Shared with me`, and
    /// (best-effort — needs scope; accounts without any list none) each shared
    /// drive as its own top-level directory.
    async fn root_sections(&self) -> Vec<Child> {
        let section = |name: &str, id: &str, kind: GKind, drive_id: Option<String>| Child {
            vfs_name: name.to_string(),
            id: id.to_string(),
            drive_id,
            kind,
            // A section is a directory, and a directory's type is a directory.
            mime_type: None,
            mtime: None,
            created: None,
            card: Value::Null,
            text: None,
        };
        let mut children = vec![
            section(MY_DRIVE_NAME, MY_DRIVE_ID, GKind::Folder, None),
            section(SHARED_WITH_ME_NAME, SHARED_WITH_ME_ID, GKind::Folder, None),
        ];
        if let Ok(drives) = self.accessor.list_shared_drives().await {
            let mut existing: HashSet<String> =
                children.iter().map(|c| c.vfs_name.clone()).collect();
            for d in &drives {
                if let (Some(id), Some(name)) = (
                    d.get("id").and_then(|x| x.as_str()),
                    d.get("name").and_then(|x| x.as_str()),
                ) {
                    let vfs_name = unique_name(&sanitize_name(name), &existing);
                    existing.insert(vfs_name.clone());
                    children.push(section(
                        &vfs_name,
                        id,
                        GKind::SharedDrive,
                        Some(id.to_string()),
                    ));
                }
            }
        }
        children
    }

    /// List a directory's immediate children (cached). The root is virtual (see
    /// [`Self::root_sections`]); everything else is a Drive listing.
    async fn list_dir(&self, folder: &str) -> ResourceResult<Vec<Child>> {
        {
            let cache = self.dir_cache.lock().await;
            if let Some((at, children)) = cache.get(folder)
                && at.elapsed() < DIR_TTL
            {
                return Ok(children.clone());
            }
        }
        let mut children = if folder == "/" {
            self.root_sections().await
        } else {
            let (folder_id, drive_id) = self.folder_id_of(folder).await?;
            let files = if folder_id == SHARED_WITH_ME_ID {
                self.accessor.list_shared_with_me(MAX_FOLDER_FILES).await
            } else {
                self.accessor
                    .list_files(&folder_id, drive_id.as_deref(), MAX_FOLDER_FILES)
                    .await
            }
            .map_err(not_found_or_backend)?;
            // A card per row, plus a `.txt` sidecar for the rows Drive can hand
            // over as text — the only content this mount can actually serve.
            let mut children: Vec<Child> = files.iter().flat_map(children_from_file).collect();
            // Children of a shared drive stay scoped to it (list_files needs the
            // drive id); the drive's own listing rows don't carry `driveId`.
            if let Some(d) = &drive_id {
                for c in children.iter_mut() {
                    c.drive_id.get_or_insert_with(|| d.clone());
                }
            }
            children
        };

        // Two Drive files can share a name; disambiguate so every entry is
        // reachable (readdir shows distinct names, resolve finds each one).
        disambiguate(&mut children);

        self.dir_cache
            .lock()
            .await
            .insert(folder.to_string(), (Instant::now(), children.clone()));
        Ok(children)
    }

    /// Resolve a folder path to its Drive id (+ shared-drive id) by walking
    /// parent listings from the root sections (`/My Drive` = the literal Drive
    /// id `root`; `/` itself is virtual and handled by [`Self::list_dir`]).
    async fn folder_id_of(&self, folder: &str) -> ResourceResult<(String, Option<String>)> {
        let (parent, name) = split_last(folder);
        let children = Box::pin(self.list_dir(&parent)).await?;
        let entry = children
            .iter()
            .find(|c| c.vfs_name == name && c.kind.is_dir())
            .ok_or(ResourceError::NotFound)?;
        Ok((entry.id.clone(), entry.drive_id.clone()))
    }

    /// Resolve any path (file or folder) to its child entry via its parent dir.
    async fn resolve(&self, path: &str) -> ResourceResult<Child> {
        let (parent, name) = split_last(path);
        let children = self.list_dir(&parent).await?;
        children
            .into_iter()
            .find(|c| c.vfs_name == name)
            .ok_or(ResourceError::NotFound)
    }
}

#[async_trait]
impl Resource for GdriveResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<std::ops::Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        if path.is_root() {
            return Err(ResourceError::Backend(anyhow::anyhow!("is a directory: /")));
        }
        let child = self.resolve(path.as_str()).await?;
        if child.kind.is_dir() {
            return Err(ResourceError::Backend(anyhow::anyhow!(
                "is a directory: {}",
                path.as_str()
            )));
        }
        if let Some(source) = child.text {
            // The one path in this mount that transfers content. An export takes
            // seconds, so it is deliberately not something a listing triggers:
            // only reading the `.txt` does, and the metadata cache holds the
            // result for the chunked reads that follow.
            let bytes = match source {
                TextSource::Export(mime) => self.accessor.export(&child.id, mime).await,
                TextSource::Download => self.accessor.download(&child.id).await,
            }
            .map_err(not_found_or_backend)?;
            return Ok(slice(&bytes, range));
        }
        // No network: the card was built from the listing this resolved from.
        Ok(slice(&card_bytes(&child), range))
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        Err(ResourceError::Unsupported)
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        let children = self.list_dir(path.as_str()).await?;
        Ok(children.iter().map(dir_entry_for).collect())
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        if path.is_root() {
            return Ok(FileStat {
                kind: FileKind::Dir,
                ..Default::default()
            });
        }
        let child = self.resolve(path.as_str()).await?;
        Ok(FileStat {
            kind: if child.kind.is_dir() {
                FileKind::Dir
            } else {
                FileKind::File
            },
            // A card is exact — it renders from data the listing already
            // carried. A sidecar reports 0, the codebase's "ask me by reading"
            // signal: its length needs the export, and the metadata cache
            // resolves it once and caches the bytes.
            size: match (child.kind.is_dir(), child.text.is_some()) {
                (true, _) => 0,
                (_, true) => 0,
                _ => card_bytes(&child).len() as u64,
            },
            mtime: child.mtime,
            created: child.created,
            content_type: child.mime_type.clone(),
            ..Default::default()
        })
    }

    fn prompt(&self) -> &str {
        GDRIVE_PROMPT
    }

    /// Every file entry carries Drive's `mimeType`: the served bytes are a JSON
    /// card, so the name can never say what the file actually is.
    fn reports_content_type(&self) -> bool {
        true
    }

    // `listings_complete` stays at the default `true`: a per-folder
    // `files.list` is complete at fetch time (the >MAX_FOLDER_FILES truncation
    // case is pathological and logged), so a missing child really is missing.
}

/// The bytes a file entry reads as: its metadata card, pretty-printed (same
/// style as the gmail mirror's JSON) with a trailing newline. Deterministic, so
/// `stat` can size an entry without rendering it twice inconsistently.
fn card_bytes(child: &Child) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(&child.card).unwrap_or_else(|_| b"{}".to_vec());
    bytes.push(b'\n');
    bytes
}

/// The listing row for one child. Files report the exact length of their card,
/// so `readdir` never reports a size a read can't fill.
fn dir_entry_for(c: &Child) -> DirEntry {
    DirEntry {
        name: c.vfs_name.clone(),
        kind: if c.kind.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        },
        size: match (c.kind.is_dir(), c.text.is_some()) {
            (true, _) => 0,
            // See `sidecar_of`: 0 means "length needs a fetch", which the
            // metadata cache resolves on first stat/read.
            (_, true) => 0,
            _ => card_bytes(c).len() as u64,
        },
        mtime: c.mtime,
        atime: None,
        ctime: None,
        created: c.created,
        etag: None,
        content_type: c.mime_type.clone(),
    }
}

/// Map an accessor error to [`ResourceError`]: an upstream HTTP 404 (a file id
/// that no longer exists) becomes `NotFound` so the WebDAV layer answers 404
/// instead of 500; everything else is `Backend`.
fn not_found_or_backend(e: anyhow::Error) -> ResourceError {
    let is_404 = e
        .downcast_ref::<reqwest::Error>()
        .and_then(reqwest::Error::status)
        == Some(reqwest::StatusCode::NOT_FOUND);
    if is_404 {
        ResourceError::NotFound
    } else {
        ResourceError::Backend(e)
    }
}

/// Map one `files.list` row into a [`Child`], building its metadata card.
/// `None` only for malformed rows — every Drive type is listable here,
/// including the ones with no downloadable content (Forms, Maps, Vids…),
/// because a card and a link always exist.
fn child_from_file(f: &Value) -> Option<Child> {
    let name = f.get("name")?.as_str()?;
    let id = f.get("id")?.as_str()?;
    let mime = f.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
    let is_folder = mime == FOLDER_MIME;
    Some(Child {
        vfs_name: if is_folder {
            sanitize_name(name)
        } else {
            format!("{}{CARD_SUFFIX}", sanitize_name(name))
        },
        id: id.to_string(),
        drive_id: f.get("driveId").and_then(|d| d.as_str()).map(String::from),
        kind: if is_folder {
            GKind::Folder
        } else {
            GKind::File
        },
        mime_type: (!is_folder).then(|| mime.to_string()),
        mtime: time_field(f, "modifiedTime"),
        created: time_field(f, "createdTime"),
        // Directories have no content; files carry the card they read as.
        card: if is_folder {
            Value::Null
        } else {
            card_of(f, name, id, mime)
        },
        // This entry is the card; its sidecar (if any) is a separate `Child`
        // built by `text_sidecar`.
        text: None,
    })
}

/// The entries one listing row becomes: its card, plus a `.txt` sidecar when
/// Drive can hand the file over as text.
fn children_from_file(f: &Value) -> Vec<Child> {
    let Some(card) = child_from_file(f) else {
        return Vec::new();
    };
    let mime = f.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
    match text_source(f, mime) {
        Some(src) if !card.kind.is_dir() => {
            let side = sidecar_of(&card, src);
            vec![card, side]
        }
        _ => vec![card],
    }
}

/// The `.txt` sidecar beside a card.
///
/// Named off the card (`report.pdf.json` → `report.pdf.txt`), so the two sort
/// together and `Glob **/*.txt` finds exactly what is searchable. Size stays 0:
/// the length is only known once Drive has converted the document, and sizing
/// every entry at listing time would mean one export per row. The metadata cache
/// resolves the real length on first stat/read and keeps the bytes — the same
/// path Notion's `page.json` takes.
fn sidecar_of(card: &Child, source: TextSource) -> Child {
    let base = card
        .vfs_name
        .strip_suffix(CARD_SUFFIX)
        .unwrap_or(&card.vfs_name);
    Child {
        vfs_name: format!("{base}{TEXT_SUFFIX}"),
        id: card.id.clone(),
        drive_id: card.drive_id.clone(),
        kind: GKind::File,
        // The sidecar really is plain text, whatever the original was.
        mime_type: Some("text/plain".to_string()),
        mtime: card.mtime,
        created: card.created,
        card: Value::Null,
        text: Some(source),
    }
}

/// The metadata card a file entry reads as. Carries the *original* Drive name
/// (the entry's own name is sanitized and may be disambiguated), what the thing
/// is, how big it really is in Drive, its times, its owner, and the link that
/// opens it. Absent fields are omitted rather than rendered as `null`, so a
/// card only states what Drive actually reported.
fn card_of(f: &Value, name: &str, id: &str, mime: &str) -> Value {
    let mut card = serde_json::Map::new();
    card.insert("name".into(), Value::from(name));
    card.insert("id".into(), Value::from(id));
    card.insert("mime_type".into(), Value::from(mime));
    // Drive reports `size` as a string — keep it numeric for consumers. Some
    // rows have none (folders, shortcuts; also native docs on enterprise-mock,
    // which diverges from Google there), and the card then omits the field.
    if let Some(size) = f
        .get("size")
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u64>().ok())
    {
        card.insert("size".into(), Value::from(size));
    }
    for (key, out) in [
        ("modifiedTime", "modified_time"),
        ("createdTime", "created_time"),
    ] {
        if let Some(t) = f.get(key).and_then(|t| t.as_str()) {
            card.insert(out.into(), Value::from(t));
        }
    }
    // First owner only: Drive allows several, but one identifies "whose file is
    // this" — the question a shared item raises.
    if let Some(owner) = f
        .get("owners")
        .and_then(|o| o.as_array())
        .and_then(|a| a.first())
    {
        let mut who = serde_json::Map::new();
        if let Some(n) = owner.get("displayName").and_then(|n| n.as_str()) {
            who.insert("name".into(), Value::from(n));
        }
        if let Some(e) = owner.get("emailAddress").and_then(|e| e.as_str()) {
            who.insert("email".into(), Value::from(e));
        }
        if !who.is_empty() {
            card.insert("owner".into(), Value::Object(who));
        }
    }
    if let Some(link) = f.get("webViewLink").and_then(|l| l.as_str()) {
        card.insert("web_view_link".into(), Value::from(link));
    }
    Value::Object(card)
}

fn time_field(f: &Value, key: &str) -> Option<std::time::SystemTime> {
    rfc3339_to_systemtime(f.get(key)?.as_str()?)
}

/// Parse an RFC 3339 timestamp into a `SystemTime` (pre-epoch → `None`).
fn rfc3339_to_systemtime(s: &str) -> Option<std::time::SystemTime> {
    let secs = chrono::DateTime::parse_from_rfc3339(s).ok()?.timestamp();
    (secs >= 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

/// Sanitize a Drive name into a single path segment. Drive allows any character
/// in a name — including `/` — so path separators and control chars collapse to
/// `_`, and a name that is empty, `.`, or `..` after that falls back to a
/// placeholder (it would otherwise escape its directory).
fn sanitize_name(name: &str) -> String {
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
        "" | "." | ".." => "untitled".to_string(),
        _ => cleaned,
    }
}

/// Give every child a unique `vfs_name`: on a collision, append ` (2)`, ` (3)`,
/// … so duplicate Drive names don't shadow each other in readdir/resolve.
fn disambiguate(children: &mut [Child]) {
    let mut seen: HashSet<String> = HashSet::new();
    for c in children.iter_mut() {
        if seen.insert(c.vfs_name.clone()) {
            continue;
        }
        let mut n = 2;
        loop {
            let cand = format!("{} ({n})", c.vfs_name);
            if seen.insert(cand.clone()) {
                c.vfs_name = cand;
                break;
            }
            n += 1;
        }
    }
}

/// Disambiguate a shared-drive name that collides with a root section.
fn unique_name(name: &str, existing: &HashSet<String>) -> String {
    if !existing.contains(name) {
        return name.to_string();
    }
    let mut candidate = format!("{name} [Shared Drive]");
    let mut suffix = 2;
    while existing.contains(&candidate) {
        candidate = format!("{name} [Shared Drive {suffix}]");
        suffix += 1;
    }
    candidate
}

/// Split a path into `(parent_dir, last_segment)`. `/a/b` -> (`/a`, `b`);
/// `/a` -> (`/`, `a`).
fn split_last(path: &str) -> (String, String) {
    let p = path.trim_end_matches('/');
    match p.rsplit_once('/') {
        Some((parent, name)) => {
            let parent = if parent.is_empty() {
                "/".to_string()
            } else {
                parent.to_string()
            };
            (parent, name.to_string())
        }
        None => ("/".to_string(), p.to_string()),
    }
}

/// The requested window of `data`, clamped (an out-of-range read yields empty
/// rather than panicking).
fn slice(data: &[u8], range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len());
            data[start..end].to_vec()
        }
        None => data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file_row(name: &str, id: &str, mime: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "mimeType": mime,
            "size": "47065",
            "modifiedTime": "2026-01-30T09:00:00Z",
            "createdTime": "2025-11-01T12:00:00Z",
            "webViewLink": format!("https://drive.google.com/file/d/{id}/view"),
            "owners": [{"displayName": "Mia Lopez", "emailAddress": "mia@acme.com"}],
        })
    }

    /// The card a file entry reads as, parsed back.
    fn card(c: &Child) -> Value {
        serde_json::from_slice(&card_bytes(c)).expect("card is valid JSON")
    }

    #[test]
    fn every_drive_type_lists_as_a_card_file() {
        // Folders stay directories under their plain name, with no content.
        let folder = child_from_file(&file_row("Reports", "f1", FOLDER_MIME)).unwrap();
        assert_eq!(folder.kind, GKind::Folder);
        assert_eq!(folder.vfs_name, "Reports");
        assert_eq!(folder.card, Value::Null);

        // Everything else — native docs, blobs, AND the types with no
        // downloadable content (Forms/Maps/Vids) — is a `.json` card, because
        // metadata and a link always exist.
        for (name, mime, expected) in [
            (
                "Q3 Plan",
                "application/vnd.google-apps.document",
                "Q3 Plan.json",
            ),
            ("paper.pdf", "application/pdf", "paper.pdf.json"),
            ("Survey", "application/vnd.google-apps.form", "Survey.json"),
            (
                "탄소 지도",
                "application/vnd.google-apps.map",
                "탄소 지도.json",
            ),
            // A real `.json` file doubles the suffix rather than hiding that
            // the entry is a card.
            ("data.json", "application/json", "data.json.json"),
        ] {
            let c = child_from_file(&file_row(name, "x1", mime)).unwrap();
            assert_eq!(c.kind, GKind::File, "{name}");
            assert_eq!(c.vfs_name, expected);
        }
    }

    #[test]
    fn the_card_carries_the_metadata_and_the_size_matches_exactly() {
        let c = child_from_file(&file_row("분기 보고서.pdf", "abc", "application/pdf")).unwrap();
        let v = card(&c);
        // The ORIGINAL Drive name, not the sanitized entry name.
        assert_eq!(v["name"], "분기 보고서.pdf");
        assert_eq!(v["id"], "abc");
        assert_eq!(v["mime_type"], "application/pdf");
        // Drive reports size as a string; the card keeps it numeric.
        assert_eq!(v["size"], 47065);
        assert_eq!(v["modified_time"], "2026-01-30T09:00:00Z");
        assert_eq!(v["created_time"], "2025-11-01T12:00:00Z");
        assert_eq!(v["owner"]["email"], "mia@acme.com");
        assert_eq!(
            v["web_view_link"],
            "https://drive.google.com/file/d/abc/view"
        );

        // Regression guard: a listing/stat size that disagrees with the bytes
        // truncates reads (or, at 0, makes the file look empty).
        let bytes = card_bytes(&c);
        let entry = dir_entry_for(&c);
        assert_eq!(entry.size, bytes.len() as u64);
        assert_ne!(entry.size, 0);
        // Ranged reads clamp instead of panicking.
        assert_eq!(slice(&bytes, Some(0..1)), b"{");
        assert!(slice(&bytes, Some(9999..12000)).is_empty());
    }

    #[test]
    fn absent_drive_fields_are_omitted_not_nulled() {
        let mut row = file_row("odd", "id1", "application/vnd.google-apps.document");
        let obj = row.as_object_mut().unwrap();
        // A native doc has no size, and a row may lack owners/link entirely.
        obj.remove("size");
        obj.remove("owners");
        obj.remove("webViewLink");
        let c = child_from_file(&row).unwrap();
        let v = card(&c);
        assert!(v.get("size").is_none(), "no null size: {v}");
        assert!(v.get("owner").is_none());
        assert!(v.get("web_view_link").is_none());
        // Still a well-formed, correctly-sized entry.
        assert_eq!(v["name"], "odd");
        assert_eq!(dir_entry_for(&c).size, card_bytes(&c).len() as u64);
    }

    /// Non-ASCII names must survive the whole path: entry name, card round-trip,
    /// and sizing. The trap is sizing in *characters* instead of bytes — a
    /// Korean/emoji name makes the card longer in bytes than in chars, and a
    /// short size silently truncates the read.
    #[test]
    fn non_ascii_names_round_trip_and_size_in_bytes() {
        for name in [
            "분기 보고서.pdf",
            "日本語のファイル",
            "Ελληνικά έγγραφο",
            "мой документ",
            "مستند عربي",
            "party 🎉 notes.txt",
        ] {
            let c = child_from_file(&file_row(name, "id1", "application/pdf")).unwrap();
            assert_eq!(c.vfs_name, format!("{name}{CARD_SUFFIX}"), "entry name");
            // The card carries the name back verbatim (no mangling, no escaping
            // surprises once parsed).
            assert_eq!(card(&c)["name"], name);

            let bytes = card_bytes(&c);
            let entry = dir_entry_for(&c);
            assert_eq!(entry.size, bytes.len() as u64, "{name}: size must be bytes");
            assert!(
                bytes.len() > bytes.iter().filter(|b| b.is_ascii()).count(),
                "{name}: this case should actually contain multi-byte text"
            );
            // A range landing mid-character yields partial bytes (correct for a
            // byte range) instead of panicking — we slice `[u8]`, never `str`.
            let mid = bytes.len() as u64 - 2;
            assert!(!slice(&bytes, Some(mid - 1..mid)).is_empty());
        }
    }

    /// A listing has to say what the file *is*, not what its bytes are. Every
    /// entry here is named `….json` and holds JSON, so a client guessing from the
    /// extension learns `application/json` and nothing else — and a Google Doc's
    /// name carries no extension at all, so guessing gets it nowhere. The entry
    /// therefore carries Drive's own `mimeType`, and `stat` reports the same.
    #[test]
    fn a_listing_reports_what_the_file_is_not_what_the_card_is() {
        for (name, mime) in [
            ("paper.pdf", "application/pdf"),
            ("setup.exe", "application/vnd.microsoft.portable-executable"),
            // The case extension-guessing cannot reach.
            ("Q3 Plan", "application/vnd.google-apps.document"),
        ] {
            let c = child_from_file(&file_row(name, "x1", mime)).unwrap();
            let entry = dir_entry_for(&c);
            assert!(entry.name.ends_with(CARD_SUFFIX), "{name}");
            assert_eq!(entry.content_type.as_deref(), Some(mime), "{name}");
            // ...and it agrees with the card's own field, so a client that reads
            // one file gets the same answer as one that only listed the folder.
            assert_eq!(card(&c)["mime_type"], mime);
        }

        // A directory's type is a directory — nothing to report.
        let dir = child_from_file(&file_row("Reports", "f1", FOLDER_MIME)).unwrap();
        assert_eq!(dir_entry_for(&dir).content_type, None);
    }

    /// Which rows get a text sidecar, and what the pair looks like. This is the
    /// only content the mount serves, so the classification is the difference
    /// between a document an agent can read and one it can only link to.
    #[test]
    fn only_convertible_files_get_a_text_sidecar() {
        let names = |row: &Value| -> Vec<String> {
            children_from_file(row)
                .iter()
                .map(|c| c.vfs_name.clone())
                .collect()
        };
        let sized = |name: &str, mime: &str, size: Option<&str>| {
            let mut row = file_row(name, "id1", mime);
            let obj = row.as_object_mut().unwrap();
            match size {
                Some(s) => obj.insert("size".into(), Value::from(s)),
                None => obj.remove("size"),
            };
            row
        };

        // Natives Drive converts: plain text for prose, CSV for a grid.
        assert_eq!(
            names(&sized("Q3 Plan", "application/vnd.google-apps.document", None)),
            ["Q3 Plan.json", "Q3 Plan.txt"]
        );
        assert_eq!(
            names(&sized("Budget", "application/vnd.google-apps.spreadsheet", None)),
            ["Budget.json", "Budget.txt"]
        );
        // Already text, small enough to be worth fetching. The sidecar hangs off
        // the card's name, so the original extension stays visible.
        assert_eq!(
            names(&sized("notes.md", "text/markdown", Some("2048"))),
            ["notes.md.json", "notes.md.txt"]
        );
        // No text form, or too big to fetch on a whim: card only.
        let oversized = (BLOB_TEXT_MAX + 1).to_string();
        for (mime, size) in [
            ("application/pdf", Some("47065")),
            ("application/x-hwp", Some("47065")),
            ("image/jpeg", Some("47065")),
            ("application/zip", Some("47065")),
            ("application/vnd.google-apps.form", None),
            ("text/plain", Some(oversized.as_str())),
            // A text file Drive reported no size for — don't risk the fetch.
            ("text/plain", None),
        ] {
            assert_eq!(names(&sized("thing", mime, size)).len(), 1, "{mime} {size:?}");
        }

        // A folder is a folder: no card, no sidecar.
        assert_eq!(names(&file_row("Reports", "f1", FOLDER_MIME)), ["Reports"]);

        // The sidecar reports size 0 — its length needs the export, and the
        // metadata cache resolves that on first read. The card stays exact.
        let pair = children_from_file(&sized(
            "Q3 Plan",
            "application/vnd.google-apps.document",
            None,
        ));
        let card = dir_entry_for(&pair[0]);
        let side = dir_entry_for(&pair[1]);
        assert_eq!(card.size, card_bytes(&pair[0]).len() as u64);
        assert_eq!(side.size, 0, "0 = ask by reading");
        assert_eq!(side.content_type.as_deref(), Some("text/plain"));
        assert_eq!(pair[1].id, pair[0].id, "same Drive file behind both");
    }

    #[test]
    fn drive_names_cannot_escape_their_directory() {
        // Drive allows `/` in a name; it must not become a path separator.
        let evil = child_from_file(&file_row("../../etc/passwd", "e1", "text/plain")).unwrap();
        assert!(!evil.vfs_name.contains('/'));
        let dotdot = child_from_file(&file_row("..", "e2", "text/plain")).unwrap();
        assert_eq!(dotdot.vfs_name, "untitled.json");
    }

    #[test]
    fn disambiguate_suffixes_collisions() {
        let mk = |n: &str| Child {
            vfs_name: n.to_string(),
            id: "x".into(),
            drive_id: None,
            kind: GKind::File,
            mime_type: None,
            mtime: None,
            created: None,
            card: Value::Null,
            text: None,
        };
        let mut children = vec![
            mk("a.pdf.json"),
            mk("a.pdf.json"),
            mk("a.pdf.json"),
            mk("b"),
        ];
        disambiguate(&mut children);
        let names: Vec<&str> = children.iter().map(|c| c.vfs_name.as_str()).collect();
        assert_eq!(
            names,
            vec!["a.pdf.json", "a.pdf.json (2)", "a.pdf.json (3)", "b"]
        );
    }

    #[test]
    fn shared_drive_names_dodge_the_root_sections() {
        let existing: HashSet<String> =
            [MY_DRIVE_NAME.to_string(), SHARED_WITH_ME_NAME.to_string()].into();
        assert_eq!(unique_name("Team", &existing), "Team");
        assert_eq!(
            unique_name(MY_DRIVE_NAME, &existing),
            "My Drive [Shared Drive]"
        );
    }

    #[test]
    fn split_last_shapes() {
        assert_eq!(split_last("/a/b"), ("/a".to_string(), "b".to_string()));
        assert_eq!(split_last("/a"), ("/".to_string(), "a".to_string()));
        assert_eq!(
            split_last("/My Drive/x.json"),
            ("/My Drive".to_string(), "x.json".to_string())
        );
    }

    /// Config for the enterprise-mock integration test. The mock hands the
    /// refresh token straight back as the bearer token, so any user token from
    /// its `tokens.yaml` (or the admin token) works with the normal OAuth flow;
    /// it has no `about` endpoint, so the email is a fixed test value. The token
    /// var is deliberately NOT `GDRIVE_REFRESH_TOKEN` — sharing it with
    /// [`live_config`] means one shell with both set sends a real Google token
    /// to the mock, which then fails for a reason that looks like a code bug.
    fn mock_config() -> Option<GdriveConfig> {
        Some(GdriveConfig {
            client_id: "mock".into(),
            client_secret: "mock".into(),
            refresh_token: std::env::var("GDRIVE_MOCK_TOKEN")
                .unwrap_or_else(|_| "admin-service-token".into()),
            account_email: "mock@example.com".into(),
            base_url: Some(std::env::var("GDRIVE_BASE_URL").ok()?),
        })
    }

    /// Tree + card round-trip against an enterprise-mock, local or hosted.
    /// Ignored by default; run with:
    ///
    ///   # local: python -m app.importer.byo \
    ///   #   examples/bring-your-own-corpus/sample_corpus.jsonl
    ///   #        then python -m uvicorn app.main:app --port 8000
    ///   GDRIVE_BASE_URL=http://localhost:8000 \
    ///     [GDRIVE_MOCK_TOKEN=…] cargo test -p workspace gdrive_mock -- --ignored --nocapture
    ///
    /// The walk is bounded ([`WALK_DIRS`]/[`WALK_FILES`]) so the same test runs
    /// against a five-file sample corpus and a 25k-document hosted one; a real
    /// corpus also spans several listing pages, which exercises the accessor's
    /// pagination for free.
    #[tokio::test]
    #[ignore = "requires a running enterprise-mock (GDRIVE_BASE_URL)"]
    async fn gdrive_mock_tree_and_cards() {
        /// Bounds on the walk — enough to cross a page boundary on a real
        /// corpus, small enough to stay quick on a tiny one.
        const WALK_DIRS: usize = 12;
        const WALK_FILES: usize = 200;

        let Some(cfg) = mock_config() else {
            eprintln!("set GDRIVE_BASE_URL (e.g. http://localhost:8000) to run");
            return;
        };
        let r = GdriveResource::new(&cfg).unwrap();

        let root = r.readdir(&MountPath::root()).await.expect("root readdir");
        eprintln!(
            "root: {:?}",
            root.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert!(root.iter().any(|e| e.name == MY_DRIVE_NAME));
        assert!(root.iter().any(|e| e.name == SHARED_WITH_ME_NAME));

        // A path that cannot exist must be NotFound (from the parent listing),
        // not a 500 — the WebDAV layer turns this into a 404.
        for bogus in [
            format!("/{MY_DRIVE_NAME}/definitely-not-here-9f3c.json"),
            "/NoSuchSection".to_string(),
            format!("/{MY_DRIVE_NAME}/nope/deeper.json"),
        ] {
            let p = MountPath::new(&bogus);
            assert!(
                matches!(r.stat(&p).await, Err(ResourceError::NotFound)),
                "stat {bogus} should be NotFound"
            );
            assert!(
                matches!(r.read_bytes(&p, None).await, Err(ResourceError::NotFound)),
                "read {bogus} should be NotFound"
            );
        }

        // Walk the tree (bounded); every file must be a `.json` card that
        // parses, names itself, and whose stat size matches the bytes a read
        // returns.
        let mut queue: Vec<String> = root
            .iter()
            .filter(|e| e.kind == FileKind::Dir)
            .map(|e| format!("/{}", e.name))
            .collect();
        let (mut files, mut dirs, mut biggest) = (0usize, 0usize, 0usize);
        let mut sample: Option<MountPath> = None;
        while let Some(dir) = queue.pop() {
            if dirs >= WALK_DIRS || files >= WALK_FILES {
                eprintln!("walk bound reached ({dirs} dirs, {files} files); stopping");
                break;
            }
            dirs += 1;
            let entries = r
                .readdir(&MountPath::new(&dir))
                .await
                .expect("folder readdir");
            biggest = biggest.max(entries.len());
            eprintln!("  {dir} -> {} entries", entries.len());
            for e in entries.iter().take(WALK_FILES.saturating_sub(files)) {
                let p = format!("{dir}/{}", e.name);
                if e.kind == FileKind::Dir {
                    queue.push(p);
                    continue;
                }
                assert!(e.name.ends_with(CARD_SUFFIX), "{}", e.name);
                let mp = MountPath::new(&p);
                let bytes = r.read_bytes(&mp, None).await.expect("read card");
                let st = r.stat(&mp).await.expect("stat card");
                assert_eq!(st.size, bytes.len() as u64, "{p}");
                let v: Value = serde_json::from_slice(&bytes).expect("card parses");
                assert!(v["name"].is_string(), "{p}: {v}");
                assert!(v["id"].is_string(), "{p}: {v}");
                if sample.is_none() {
                    eprintln!("{p} ->\n{}", String::from_utf8_lossy(&bytes));
                    sample = Some(mp);
                }
                files += 1;
            }
        }
        eprintln!("{files} cards checked across {dirs} dirs; biggest listing {biggest}");
        assert!(files > 0, "the corpus should expose files");
        // >1000 in one listing means the accessor followed `nextPageToken`
        // (Drive's page size here) — only true on a corpus that big.
        if biggest > 1000 {
            eprintln!("pagination exercised: one listing spanned {biggest} entries");
        }

        // Ranged reads come out of the same card bytes, clamped at the end.
        let mp = sample.expect("a card to range-read");
        let whole = r.read_bytes(&mp, None).await.expect("whole card");
        assert_eq!(
            r.read_bytes(&mp, Some(0..1)).await.expect("first byte"),
            b"{"
        );
        let tail = whole.len() as u64;
        assert_eq!(
            r.read_bytes(&mp, Some(tail - 1..tail + 999))
                .await
                .expect("clamped tail"),
            b"\n",
            "a range past the end clamps instead of erroring"
        );
        assert!(
            r.read_bytes(&mp, Some(tail + 5..tail + 9))
                .await
                .expect("beyond end")
                .is_empty()
        );
    }

    /// Live: the text sidecar is the only content this mount serves, so read one
    /// end to end against real Drive — a Sheet (CSV) and, if present, a Doc.
    /// Prints the conversion latency, which is what makes reading deliberate
    /// rather than something a listing does.
    ///
    ///   GDRIVE_CLIENT_ID=… GDRIVE_CLIENT_SECRET=… GDRIVE_REFRESH_TOKEN=… \
    ///     cargo test -p workspace gdrive_live_text -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires GDRIVE_* env + network"]
    async fn gdrive_live_text_sidecars_are_readable() {
        let Some(cfg) = live_config() else {
            eprintln!("set GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET / GDRIVE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg).unwrap();

        // Shared-with-me holds this account's documents; My Drive is mostly
        // binaries. Take whichever section offers sidecars.
        let mut checked = 0usize;
        for section in [SHARED_WITH_ME_NAME, MY_DRIVE_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            let sidecars: Vec<&DirEntry> = entries
                .iter()
                .filter(|e| e.name.ends_with(TEXT_SUFFIX))
                .collect();
            eprintln!(
                "{section}: {} entries, {} of them .txt sidecars",
                entries.len(),
                sidecars.len()
            );
            for e in sidecars.iter().take(2) {
                // A listing reports 0 — the length needs the conversion.
                assert_eq!(e.size, 0, "{}", e.name);
                let p = MountPath::new(format!("/{section}/{}", e.name));
                let t0 = std::time::Instant::now();
                let bytes = r.read_bytes(&p, None).await.expect("read sidecar");
                let took = t0.elapsed();
                let text = String::from_utf8_lossy(&bytes);
                eprintln!(
                    "  {} -> {} bytes in {:.2}s | {:?}",
                    e.name,
                    bytes.len(),
                    took.as_secs_f64(),
                    text.chars().take(60).collect::<String>()
                );
                assert!(!bytes.is_empty(), "{}: empty text", e.name);
                // Real lines, not one JSON-escaped blob — the reason the text is
                // a sibling file instead of a card field.
                assert!(!text.contains("\\n"), "{}: escaped newlines", e.name);
                // And the card beside it still parses.
                let card_path = MountPath::new(format!(
                    "/{section}/{}{CARD_SUFFIX}",
                    e.name.trim_end_matches(TEXT_SUFFIX)
                ));
                let card = r.read_bytes(&card_path, None).await.expect("read card");
                let v: Value = serde_json::from_slice(&card).expect("card parses");
                assert!(v["mime_type"].is_string());
                checked += 1;
            }
            if checked > 0 {
                break;
            }
        }
        assert!(checked > 0, "no convertible document found to read");
    }

    fn live_config() -> Option<GdriveConfig> {
        Some(GdriveConfig {
            client_id: std::env::var("GDRIVE_CLIENT_ID").ok()?,
            client_secret: std::env::var("GDRIVE_CLIENT_SECRET").ok()?,
            refresh_token: std::env::var("GDRIVE_REFRESH_TOKEN").ok()?,
            account_email: std::env::var("GDRIVE_EMAIL").unwrap_or_else(|_| "live-test".into()),
            base_url: None,
        })
    }

    /// Live tree + card round-trip against real Google Drive: the two root
    /// sections, both section listings, and one card per section carrying a real
    /// `drive.google.com`/`docs.google.com` link at the exact stat'd size.
    /// Ignored by default; run with:
    ///
    ///   GDRIVE_CLIENT_ID=… GDRIVE_CLIENT_SECRET=… GDRIVE_REFRESH_TOKEN=… \
    ///     cargo test -p workspace gdrive_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires GDRIVE_* env + network"]
    async fn gdrive_live_tree_and_cards() {
        let Some(cfg) = live_config() else {
            eprintln!("set GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET / GDRIVE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg).unwrap();

        let root = r.readdir(&MountPath::root()).await.expect("root readdir");
        eprintln!(
            "root: {:?}",
            root.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert!(root.iter().any(|e| e.name == MY_DRIVE_NAME));

        for section in [MY_DRIVE_NAME, SHARED_WITH_ME_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            eprintln!("{section}: {} entries", entries.len());
            for e in entries.iter().take(5) {
                eprintln!("  {:>6}B {:?} {}", e.size, e.kind, e.name);
            }
            // Read the first file entry: its content is a metadata card whose
            // link is a real Drive URL, sized exactly.
            if let Some(f) = entries.iter().find(|e| e.kind == FileKind::File) {
                let mp = MountPath::new(format!("/{section}/{}", f.name));
                let bytes = r.read_bytes(&mp, None).await.expect("read card");
                eprintln!(
                    "  first card {}:\n{}",
                    f.name,
                    String::from_utf8_lossy(&bytes)
                );
                assert!(f.name.ends_with(CARD_SUFFIX));
                let v: Value = serde_json::from_slice(&bytes).expect("card parses");
                assert!(v["name"].is_string(), "{v}");
                assert!(v["mime_type"].is_string(), "{v}");
                assert!(
                    v["web_view_link"]
                        .as_str()
                        .is_some_and(|l| l.starts_with("https://")),
                    "{v}"
                );
                assert_eq!(
                    r.stat(&mp).await.expect("stat").size,
                    bytes.len() as u64,
                    "stat size must equal the card bytes"
                );
            }
        }
    }
}
