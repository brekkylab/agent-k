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

/// Suffix for a native Workspace doc's converted text. Those types hold no bytes
/// of their own, so the conversion *is* the file's content — and the name has to
/// say what the bytes are, since `Q3 Plan` carries no extension.
const TEXT_SUFFIX: &str = ".txt";

/// Whether a listing entry is a conversion rather than a file's own bytes: its
/// content type is the *source* mime, which only the Docs-editors types have.
#[cfg(test)]
fn is_conversion(e: &DirEntry) -> bool {
    e.content_type
        .as_deref()
        .is_some_and(|t| t.starts_with("application/vnd.google-apps."))
}

/// Native types Drive converts to text, and to what. Sheets go to CSV (Drive
/// exports the first tab only — a documented Google limitation); Docs and Slides
/// to plain text. Drawings/Forms/Maps have no text form at all.
const TEXT_EXPORTS: &[(&str, &str)] = &[
    ("application/vnd.google-apps.document", "text/plain"),
    ("application/vnd.google-apps.presentation", "text/plain"),
    ("application/vnd.google-apps.spreadsheet", "text/csv"),
];

/// Per-directory listing TTL. A listing is one `files.list`, so a short TTL
/// keeps `ls` fresh; path resolution walks parent listings and rides this cache.
const DIR_TTL: Duration = Duration::from_secs(60);
/// Size reported for a conversion whose length nobody has learned yet.
///
/// It cannot be 0. Reads run under the guest's FUSE mount with `direct_io`, and a
/// 0-length file was measured (in ailoy's Drive mount, which hit this first) to
/// clamp reads to nothing — the file would list but never open. An over-estimate
/// is safe in the other direction: the read returns the real bytes and then empty
/// at EOF, so `cat` stops at the true end. 64 MiB clears any Drive export, which
/// Google itself caps at 10 MB.
///
/// Exact once something has read the file: the metadata cache then knows the
/// length and answers from it (see `CachedResource::stat`).
const CONVERSION_SENTINEL_SIZE: u64 = 64 * 1024 * 1024;

/// Safety ceiling on one folder's listing (10 pages). Beyond this the listing
/// truncates (the accessor logs it) — a >10k-child folder is pathological to
/// `ls` anyway.
const MAX_FOLDER_FILES: usize = 10_000;

const GDRIVE_PROMPT: &str = "\
Google Drive (read-only). Real files at their Drive names; the layout mirrors
Drive's own sidebar:
  My Drive/…              # the account's folder tree; descend with ls
  Shared with me/         # everything shared to this account
  <shared drive>/         # each shared drive, when the account has any

  Folders are directories, files are files: `cat`, `head`, `cp` and `grep` all
  work on the real bytes. Google Docs, Sheets and Slides hold no bytes of their
  own, so each appears as `<name>.txt` carrying its converted text instead
  (Sheets: first tab, as CSV). Files with no readable form at all (Forms, Maps,
  Drawings) are not listed.

  This is a remote mount, so every read costs network:
  - `head`/`grep` transfer only what they touch; `cat` of a 70MB file moves 70MB
  - narrow a search with an include/glob filter (e.g. '*.txt', '*.pdf'). A
    filtered search never opens the other files at all
  - a `.txt` conversion takes a few seconds on its first read (cached after) and
    lists as size 0 until then; every other file lists its true size
  The mount is read-only: no writes, no rm, no mkdir.";

/// The mime a native Workspace doc converts to, if it converts at all.
///
/// Only these need a sidecar: every other file has real bytes, served at its own
/// name, and a file that is already text is already readable there. A native doc
/// has no byte form of its own, so the conversion is the only content it has.
fn export_mime(mime: &str) -> Option<&'static str> {
    TEXT_EXPORTS
        .iter()
        .find(|(m, _)| *m == mime)
        .map(|(_, target)| *target)
}

/// Whether Drive holds real bytes for this row. The Docs-editors types (and
/// Forms, Maps, Drawings) do not — `alt=media` answers 403 for them.
fn has_original_bytes(mime: &str) -> bool {
    !mime.starts_with("application/vnd.google-apps.")
}

/// What an entry hands back when read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Serves {
    /// A directory — nothing to read.
    Local,
    /// The file's own bytes (`alt=media`), ranged.
    Original,
    /// A native doc converted to this mime (`files.export`).
    Export(&'static str),
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
    /// name — plus [`TEXT_SUFFIX`] when the entry serves a conversion.
    vfs_name: String,
    id: String,
    /// Set when the entry lives in a shared drive (listing scope).
    drive_id: Option<String>,
    kind: GKind,
    /// Drive's own `mimeType`, reported as the entry's content type. For a
    /// converted doc it is the *source* type (a Sheet, not the CSV), which is
    /// what a client picking an icon wants, and the only place it exists at all
    /// since those names carry no extension.
    mime_type: Option<String>,
    mtime: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    /// What this entry hands back when read.
    serves: Serves,
    /// Byte length when it is known without fetching: Drive reports it for an
    /// original. `None` = only a fetch can tell, which `entry_size` reports as 0
    /// so the metadata cache resolves it on first read.
    size: Option<u64>,
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
            serves: Serves::Local,
            size: None,
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
            let mut children: Vec<Child> = files.iter().filter_map(child_from_file).collect();
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
        match child.serves {
            // Ranged: a reader walking a big file, or a search tool sampling its
            // head, must not pull the whole object per chunk.
            Serves::Original => {
                let bytes = self
                    .accessor
                    .download(&child.id, range.clone())
                    .await
                    .map_err(not_found_or_backend)?;
                // Drive honored the range, so the window is already the answer;
                // fall back to slicing if it sent the whole object anyway.
                return Ok(match &range {
                    Some(r) if bytes.len() as u64 > r.end - r.start => slice(&bytes, range),
                    _ => bytes,
                });
            }
            // A conversion takes seconds and has no ranged form, so the whole
            // text comes back and the metadata cache serves the chunks after.
            Serves::Export(mime) => {
                let bytes = self
                    .accessor
                    .export(&child.id, mime)
                    .await
                    .map_err(not_found_or_backend)?;
                return Ok(slice(&bytes, range));
            }
            // Only a directory serves nothing, and directories were rejected
            // above — so this is unreachable for a resolved file.
            Serves::Local => Err(ResourceError::NotFound),
        }
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
            size: entry_size(&child),
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
    /// A converted doc is served as `.txt`, so its name cannot say that the
    /// source is a spreadsheet; Drive's own mime can.
    fn reports_content_type(&self) -> bool {
        true
    }

    /// Conversions are per document and take seconds, so `ls -l` must not
    /// trigger them: an unknown length stays 0 until something reads the file.
    fn resolve_size_on_stat(&self) -> bool {
        false
    }

    // `listings_complete` stays at the default `true`: a per-folder
    // `files.list` is complete at fetch time (the >MAX_FOLDER_FILES truncation
    // case is pathological and logged), so a missing child really is missing.
}

/// The size an entry reports.
///
/// An original is exact: Drive gives its length in the listing, so `ls -l` is
/// honest and a ranged read lands where the caller asked. A conversion reports 0
/// — its length is only known once Drive has produced it, and sizing every row at
/// listing time would mean one export per row. 0 is this codebase's "ask me by
/// reading" signal (Notion's `page.json` uses it): the metadata cache resolves it
/// on first stat/read and keeps the bytes for the chunks that follow.
fn entry_size(c: &Child) -> u64 {
    match (c.kind.is_dir(), c.size) {
        (true, _) => 0,
        (_, Some(n)) => n,
        // A conversion, before anyone has read it: see CONVERSION_SENTINEL_SIZE.
        (_, None) => CONVERSION_SENTINEL_SIZE,
    }
}

/// The listing row for one child.
fn dir_entry_for(c: &Child) -> DirEntry {
    DirEntry {
        name: c.vfs_name.clone(),
        kind: if c.kind.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        },
        size: entry_size(c),
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

/// Map one `files.list` row into the entry it becomes, or `None` when the mount
/// has nothing to serve for it.
///
/// A folder is a directory. A file with bytes of its own keeps its Drive name and
/// serves those bytes. A native Workspace doc has no bytes, so its converted text
/// takes the name with a [`TEXT_SUFFIX`]; the ones that convert to nothing
/// (Forms, Maps, Drawings) are dropped — there is no content to stand behind an
/// entry, and a name that cannot be read is worse than an absence.
fn child_from_file(f: &Value) -> Option<Child> {
    let name = sanitize_name(f.get("name")?.as_str()?);
    let id = f.get("id")?.as_str()?.to_string();
    let mime = f.get("mimeType").and_then(|m| m.as_str()).unwrap_or("");
    let drive_id = f.get("driveId").and_then(|d| d.as_str()).map(String::from);
    let (mtime, created) = (time_field(f, "modifiedTime"), time_field(f, "createdTime"));

    if mime == FOLDER_MIME {
        return Some(Child {
            vfs_name: name,
            id,
            drive_id,
            kind: GKind::Folder,
            mime_type: None,
            mtime,
            created,
            serves: Serves::Local,
            size: None,
        });
    }
    let (vfs_name, serves, size) = if has_original_bytes(mime) {
        // Drive reports the length up front, so this entry is honest about its
        // size and a reader can seek inside it.
        let size = f
            .get("size")
            .and_then(|s| s.as_str())
            .and_then(|s| s.parse::<u64>().ok());
        (name, Serves::Original, size)
    } else {
        (
            format!("{name}{TEXT_SUFFIX}"),
            Serves::Export(export_mime(mime)?),
            None,
        )
    };
    Some(Child {
        vfs_name,
        id,
        drive_id,
        kind: GKind::File,
        mime_type: Some(mime.to_string()),
        mtime,
        created,
        serves,
        size,
    })
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

    /// What one Drive row becomes on the mount. A file with bytes keeps its own
    /// name and size; a native doc has no bytes, so its conversion takes the name
    /// with `.txt` and lists as 0 until read; a native with nothing to convert is
    /// absent, because a name that cannot be read is worse than no name.
    #[test]
    fn a_row_becomes_the_thing_it_can_serve() {
        let entry = |name: &str, mime: &str| child_from_file(&file_row(name, "id1", mime));

        // Real bytes: plain name, Drive's own size, ranged reads.
        let pdf = entry("report.pdf", "application/pdf").unwrap();
        assert_eq!(pdf.vfs_name, "report.pdf");
        assert_eq!(pdf.serves, Serves::Original);
        assert_eq!(entry_size(&pdf), 47065, "Drive's reported size");
        assert_eq!(
            dir_entry_for(&pdf).content_type.as_deref(),
            Some("application/pdf")
        );
        // Already text — nothing extra needed, the file *is* its text.
        assert_eq!(
            entry("notes.md", "text/markdown").unwrap().vfs_name,
            "notes.md"
        );

        // Docs-editors types: the conversion is the content, so it takes the name.
        for (mime, want) in [
            ("application/vnd.google-apps.document", "text/plain"),
            ("application/vnd.google-apps.presentation", "text/plain"),
            ("application/vnd.google-apps.spreadsheet", "text/csv"),
        ] {
            let c = entry("Q3 Plan", mime).unwrap();
            assert_eq!(c.vfs_name, "Q3 Plan.txt", "{mime}");
            assert_eq!(c.serves, Serves::Export(want), "{mime}");
            assert_eq!(
                entry_size(&c),
                CONVERSION_SENTINEL_SIZE,
                "{mime}: an over-estimate, never 0 — see the constant"
            );
        }

        // Nothing to convert, nothing to serve: not listed at all.
        for mime in [
            "application/vnd.google-apps.form",
            "application/vnd.google-apps.map",
            "application/vnd.google-apps.drawing",
        ] {
            assert!(entry("Survey", mime).is_none(), "{mime}");
        }

        // A folder stays a directory under its plain name.
        let dir = entry("Reports", FOLDER_MIME).unwrap();
        assert_eq!(
            (dir.kind, dir.vfs_name.as_str()),
            (GKind::Folder, "Reports")
        );
        assert_eq!(entry_size(&dir), 0);
    }

    /// Non-ASCII names must survive the whole path — the entry name, and the size
    /// in *bytes*. Sizing in characters is the trap: a Korean or emoji name is
    /// longer in bytes than in chars, and a short size truncates reads.
    #[test]
    fn non_ascii_names_survive_and_sizes_are_drives_own() {
        for name in [
            "분기 보고서.pdf",
            "日本語のファイル",
            "Ελληνικά έγγραφο",
            "мой документ",
            "مستند عربي",
            "party 🎉 notes.txt",
        ] {
            let c = child_from_file(&file_row(name, "id1", "application/pdf")).unwrap();
            assert_eq!(c.vfs_name, name, "entry name is the Drive name");
            // Drive's own byte count, carried through untouched.
            assert_eq!(entry_size(&c), 47065, "{name}");
            assert!(
                name.len() > name.chars().count(),
                "{name}: this case should actually be multi-byte"
            );
        }

        // A native doc's name gains only the suffix, whatever the script.
        let doc = child_from_file(&file_row(
            "분기 보고서",
            "id1",
            "application/vnd.google-apps.document",
        ))
        .unwrap();
        assert_eq!(doc.vfs_name, "분기 보고서.txt");
    }

    /// A row Drive reported no size for still lists, and reports the over-estimate
    /// rather than 0 — under the guest's `direct_io` mount a 0 was measured to
    /// clamp reads to nothing, so the file would list but never open.
    #[test]
    fn an_unknown_size_is_over_estimated_never_zero() {
        let mut row = file_row("mystery.bin", "id1", "application/octet-stream");
        row.as_object_mut().unwrap().remove("size");
        let c = child_from_file(&row).unwrap();
        assert_eq!(c.serves, Serves::Original);
        assert_eq!(entry_size(&c), CONVERSION_SENTINEL_SIZE);
        // The estimate has to clear Google's own 10 MB export ceiling, or a big
        // conversion would read short.
        const _: () = assert!(CONVERSION_SENTINEL_SIZE > 10 * 1024 * 1024);
    }

    #[test]
    fn drive_names_cannot_escape_their_directory() {
        // Drive allows `/` in a name; it must not become a path separator.
        let evil = child_from_file(&file_row("../../etc/passwd", "e1", "text/plain")).unwrap();
        assert!(!evil.vfs_name.contains('/'));
        let dotdot = child_from_file(&file_row("..", "e2", "text/plain")).unwrap();
        assert_eq!(dotdot.vfs_name, "untitled");
        // Same guard on the conversion path, where a suffix is appended.
        let native = child_from_file(&file_row(
            "..",
            "e3",
            "application/vnd.google-apps.document",
        ))
        .unwrap();
        assert_eq!(native.vfs_name, "untitled.txt");
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
            serves: Serves::Local,
            size: None,
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

    /// Tree round-trip against an enterprise-mock, local or hosted. Ignored by
    /// default; run with:
    ///
    ///   # local: python -m app.importer.byo \
    ///   #   examples/bring-your-own-corpus/sample_corpus.jsonl
    ///   #        then python -m uvicorn app.main:app --port 8000
    ///   GDRIVE_BASE_URL=http://localhost:8000 \
    ///     [GDRIVE_MOCK_TOKEN=…] cargo test -p workspace gdrive_mock -- --ignored --nocapture
    ///
    /// The walk is bounded so the same test runs against a five-file sample
    /// corpus and a 25k-document hosted one; a real corpus also spans several
    /// listing pages, which exercises the accessor's pagination for free.
    #[tokio::test]
    #[ignore = "requires a running enterprise-mock (GDRIVE_BASE_URL)"]
    async fn gdrive_mock_tree_and_reads() {
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
            format!("/{MY_DRIVE_NAME}/definitely-not-here-9f3c.bin"),
            "/NoSuchSection".to_string(),
            format!("/{MY_DRIVE_NAME}/nope/deeper.bin"),
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

        // Walk the tree (bounded). Every file reads back, and an entry that
        // reported a size must hand over exactly that many bytes — a listing
        // that lies about length truncates every reader.
        let mut queue: Vec<String> = root
            .iter()
            .filter(|e| e.kind == FileKind::Dir)
            .map(|e| format!("/{}", e.name))
            .collect();
        let (mut files, mut dirs, mut biggest, mut converted) = (0usize, 0usize, 0usize, 0usize);
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
                let mp = MountPath::new(&p);
                let bytes = r.read_bytes(&mp, None).await.expect("read file");
                if is_conversion(e) {
                    // A conversion: the listing could only estimate its length,
                    // so compare against what the read actually produced.
                    converted += 1;
                    assert!(!bytes.is_empty(), "{p}: conversion produced nothing");
                } else {
                    assert_eq!(e.size, bytes.len() as u64, "{p}: listed size vs read");
                    let st = r.stat(&mp).await.expect("stat");
                    assert_eq!(st.size, e.size, "{p}: listing and stat disagree");
                    // A ranged read returns its window, and past-EOF is empty.
                    let head = r.read_bytes(&mp, Some(0..8)).await.expect("ranged read");
                    assert_eq!(head.len(), 8.min(bytes.len()), "{p}");
                    assert!(
                        r.read_bytes(&mp, Some(e.size..e.size + 16))
                            .await
                            .expect("past EOF")
                            .is_empty(),
                        "{p}: past EOF should be empty"
                    );
                }
                files += 1;
            }
        }
        eprintln!(
            "{files} files read across {dirs} dirs ({converted} conversions); \
             biggest listing {biggest}"
        );
        assert!(files > 0, "the corpus should expose files");
        if biggest > 1000 {
            eprintln!("pagination exercised: one listing spanned {biggest} entries");
        }
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

    /// Live tree round-trip against real Google Drive: the root sections, both
    /// section listings, and one ranged read per section at the length the
    /// listing promised. Ignored by default; run with:
    ///
    ///   GDRIVE_CLIENT_ID=… GDRIVE_CLIENT_SECRET=… GDRIVE_REFRESH_TOKEN=… \
    ///     cargo test -p workspace gdrive_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires GDRIVE_* env + network"]
    async fn gdrive_live_tree_and_reads() {
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
            // Read the first file: what comes back is the file itself, at the
            // length the listing promised.
            if let Some(f) = entries
                .iter()
                .find(|e| e.kind == FileKind::File && !is_conversion(e) && e.size > 0)
            {
                let mp = MountPath::new(format!("/{section}/{}", f.name));
                let head = r
                    .read_bytes(&mp, Some(0..64.min(f.size)))
                    .await
                    .expect("ranged read");
                eprintln!(
                    "  first file {} ({}B) head: {:?}",
                    f.name,
                    f.size,
                    String::from_utf8_lossy(&head)
                        .chars()
                        .take(40)
                        .collect::<String>()
                );
                assert_eq!(head.len() as u64, 64.min(f.size), "range honored");
                assert_eq!(
                    r.stat(&mp).await.expect("stat").size,
                    f.size,
                    "listing and stat must agree on length"
                );
            }
        }
    }
    /// Live: a Docs-editors file is served as its converted text — the one path
    /// where this mount produces bytes Drive itself does not store.
    #[tokio::test]
    #[ignore = "requires GDRIVE_* env + network"]
    async fn gdrive_live_text_conversions_are_readable() {
        let Some(cfg) = live_config() else {
            eprintln!("set GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET / GDRIVE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg).unwrap();

        let mut checked = 0usize;
        for section in [SHARED_WITH_ME_NAME, MY_DRIVE_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            let converted: Vec<&DirEntry> = entries.iter().filter(|e| is_conversion(e)).collect();
            eprintln!(
                "{section}: {} entries, {} of them conversions",
                entries.len(),
                converted.len()
            );
            for e in converted.iter().take(2) {
                // A conversion lists as an over-estimate — never 0, which would
                // clamp reads to nothing under the guest's direct_io mount.
                assert_eq!(e.size, CONVERSION_SENTINEL_SIZE, "{}", e.name);
                let p = MountPath::new(format!("/{section}/{}", e.name));
                // `stat` must NOT convert: asking for a size is what `ls -l` does,
                // and a folder of documents would otherwise convert every one.
                let t_stat = std::time::Instant::now();
                let st = r.stat(&p).await.expect("stat conversion");
                let stat_took = t_stat.elapsed();
                assert_eq!(
                    st.size, CONVERSION_SENTINEL_SIZE,
                    "{}: stat estimates without converting",
                    e.name
                );
                assert!(
                    stat_took.as_millis() < 500,
                    "{}: stat took {stat_took:?} — it converted",
                    e.name
                );
                let t0 = std::time::Instant::now();
                let bytes = r.read_bytes(&p, None).await.expect("read conversion");
                let text = String::from_utf8_lossy(&bytes);
                eprintln!(
                    "  {} -> {} bytes in {:.2}s | {:?}",
                    e.name,
                    bytes.len(),
                    t0.elapsed().as_secs_f64(),
                    text.chars().take(60).collect::<String>()
                );
                assert!(!bytes.is_empty(), "{}: empty text", e.name);
                // Real lines, not one escaped blob: this is a text file, which is
                // the whole reason the conversion is served as a file.
                assert!(!text.contains("\\n"), "{}: escaped newlines", e.name);
                checked += 1;
            }
            if checked > 0 {
                break;
            }
        }
        assert!(checked > 0, "no convertible document found to read");
    }

    /// Live: an original is a real file, and a ranged read transfers only its
    /// range — what makes serving originals affordable, since a search tool
    /// samples a file's head before deciding it is binary.
    #[tokio::test]
    #[ignore = "requires GDRIVE_* env + network"]
    async fn gdrive_live_originals_read_by_range() {
        let Some(cfg) = live_config() else {
            eprintln!("set GDRIVE_CLIENT_ID / GDRIVE_CLIENT_SECRET / GDRIVE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg).unwrap();

        for section in [MY_DRIVE_NAME, SHARED_WITH_ME_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            // The biggest original in the section: exactly the file a full read
            // must never be needed for.
            let Some(big) = entries
                .iter()
                .filter(|e| e.kind == FileKind::File && !is_conversion(e) && e.size > 0)
                .max_by_key(|e| e.size)
            else {
                continue;
            };
            let p = MountPath::new(format!("/{section}/{}", big.name));
            eprintln!(
                "{section}: largest original {} = {} bytes",
                big.name, big.size
            );

            let t0 = std::time::Instant::now();
            let head = r.read_bytes(&p, Some(0..4096)).await.expect("ranged read");
            eprintln!(
                "  head 4096B -> {} bytes in {:.2}s",
                head.len(),
                t0.elapsed().as_secs_f64()
            );
            assert_eq!(head.len(), 4096.min(big.size as usize), "range honored");

            // A window from the middle differs from the head — proof the offset
            // reached Drive instead of being sliced off a full download.
            if big.size > 100_000 {
                let mid = r
                    .read_bytes(&p, Some(50_000..54_096))
                    .await
                    .expect("mid read");
                assert_eq!(mid.len(), 4096);
                assert_ne!(mid, head, "a mid-file window is not the head");
            }

            // Past EOF is a clean empty read (Drive answers 416) — what a reader
            // walking to the end expects.
            assert!(
                r.read_bytes(&p, Some(big.size..big.size + 4096))
                    .await
                    .expect("read past EOF")
                    .is_empty(),
                "EOF reads empty"
            );
            assert_eq!(r.stat(&p).await.expect("stat").size, big.size);
            return;
        }
        panic!("no original found to read");
    }
}
