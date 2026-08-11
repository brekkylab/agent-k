use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::vfs::{
    accessor::{GdriveAccessor, GdriveConfig, GoogleClient},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

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

/// Whether a listing entry is a document's JSON rather than a file's own bytes:
/// its content type is the *source* mime, which only the Docs-editors types have.
#[cfg(test)]
fn is_native_json(e: &DirEntry) -> bool {
    e.content_type
        .as_deref()
        .is_some_and(|t| t.starts_with("application/vnd.google-apps."))
}

/// How each Docs-editors type is served: the API that answers for it, and the
/// suffix its entry carries.
///
/// These types hold no bytes of their own — Drive can only export a rendering of
/// them — so the document's own API is the only form that carries everything:
/// formulas, slide geometry, and the character indices an edit has to address.
/// The suffix says which document it is, since the Drive name has no extension.
const NATIVE_KINDS: &[(&str, NativeApi, &str)] = &[
    (
        "application/vnd.google-apps.document",
        NativeApi::Doc,
        ".gdoc.json",
    ),
    (
        "application/vnd.google-apps.spreadsheet",
        NativeApi::Sheet,
        ".gsheet.json",
    ),
    (
        "application/vnd.google-apps.presentation",
        NativeApi::Slides,
        ".gslide.json",
    ),
];

/// The API and suffix for a native mime, if it is one.
fn native_kind(mime: &str) -> Option<(NativeApi, &'static str)> {
    NATIVE_KINDS
        .iter()
        .find(|(m, _, _)| *m == mime)
        .map(|(_, api, suffix)| (*api, *suffix))
}

/// Per-directory listing TTL, matching the metadata cache's own listing TTL.
///
/// This cache is not the freshness policy — the wrapper above it is, and it does the
/// expiring, invalidating and negative caching. This one exists because a path has to
/// become a Drive id before anything can be read, and that means walking parent
/// listings. Holding a *different* number here bought nothing and cost two things: a
/// read whose parent listing had expired here but not there re-fetched it (an extra
/// `files.list` per file, on any traversal outrunning the shorter TTL), and for the
/// span between the two numbers `ls` answered from one snapshot while reads resolved
/// against another.
const DIR_TTL: Duration = Duration::from_secs(300);
/// Ceiling on the cell values one spreadsheet's JSON will carry, spent tab by tab
/// in the workbook's own order until it runs out.
///
/// Values are proportional to what is actually filled in — 443 B to 105 KB per tab
/// measured, an 8-tab workbook around 185 KB — so this bounds the outlier rather
/// than the common case. A workbook of 634,410 cells exists in the corpus.
const GRID_BYTES_BUDGET: u64 = 8 * 1024 * 1024;
/// Tabs whose values are requested in one `batchGet`. Each title rides in the
/// query string, so an unbounded count would eventually build an unsendable URL.
const MAX_TABS: usize = 64;

/// Size reported for a document whose length nobody has learned yet.
///
/// It cannot be 0. Reads run under the guest's FUSE mount with `direct_io`, and a
/// 0-length file was measured (in ailoy's Drive mount, which hit this first) to
/// clamp reads to nothing — and a search tool skips a file it is told is empty. An
/// over-estimate is safe in the other direction: the read returns the real bytes
/// and then empty at EOF, so `cat` stops at the true end.
///
/// 8 MiB, matching the content cache's per-object limit: anything at or under it
/// reports its exact length from the moment it is first read, so the placeholder is
/// what an unread document shows, not what a document shows. Measured real lengths
/// were 52 KB to 3.4 MB.
///
/// This is a placeholder, not a measurement — `find -size` and `ls -l` see it until
/// something reads the file. Making it exact up front costs one render per
/// document: measured 2.4s for a six-document folder listing, 6.3s when the
/// kernel's per-entry `getattr` serialises them. See
/// [`GdriveResource::resolve_size_on_stat`].
const UNKNOWN_LENGTH_SIZE: u64 = 8 * 1024 * 1024;

/// Cap on remembered probe results — one per file ever sized in this mount.
const MAX_PROBED_LENGTHS: usize = 50_000;

/// Safety ceiling on one folder's listing (10 pages). Beyond this the listing
/// truncates (the accessor logs it) — a >10k-child folder is pathological to
/// `ls` anyway.
const MAX_FOLDER_FILES: usize = 10_000;

/// Hits one content search returns. Drive ranks by relevance, and a reader that
/// needs more than this wants a narrower phrase, not a longer list.
const SEARCH_MAX_HITS: usize = 100;

const GDRIVE_PROMPT: &str = "\
Google Drive (read-only). Files at their Drive names; the layout mirrors Drive's
own sidebar:
  My Drive/…              # the account's folder tree; descend with ls
  Shared with me/         # everything shared to this account
  <shared drive>/         # each shared drive, when the account has any

  Folders are directories, files are files: `cat`, `head` and `grep` read the real
  bytes, `cp` copies out of the mount. Two files of one name are numbered before the
  extension (`report (2).pdf`).

  Docs, Sheets and Slides hold no bytes of their own, so each appears as its own
  API's JSON — the only form that carries everything about it:
    <name>.gdoc.json    paragraphs, styles, tables, the character indices an edit
                        addresses
    <name>.gsheet.json  tabs, named ranges, charts, and each tab's cell values under
                        sheets[].values (sheets[].valuesOmitted past the size budget)
    <name>.gslide.json  pages, shapes, transforms, speaker notes
  Their text is split across style runs: search words, not phrases, and `-A`/`-B`
  shows JSON siblings rather than the document's next lines. Every other
  Google-native type (Forms, Drawings, Maps, Apps Script) is not listed at all.

  A PDF, pptx, xlsx or docx keeps its text compressed and font-encoded, so grepping
  it finds nothing — and `grep` abandons any file whose first buffer looks binary, so
  archives never match either. Drive's own index did read inside them, scanned pages
  included:
    echo 'quarterly revenue' > .cmd/search    # then read .cmd/search back
  One JSON line per hit — name, id, mimeType, size — and a line carrying an
  `unlisted` note has no path here: open it in Drive instead.

  Every read costs network:
  - a real file reads by the window: `head` moves what it asks for, `cat` of a 70MB
    file moves 70MB. A glob filter never opens the files it excludes
  - a document has no windows: any read produces the whole JSON (a second or two,
    then cached), and until then its listed size is a placeholder — `ls -l` and
    `find -size` are wrong about an unread one
  - a listing is cached 5 minutes, so a change just made in Drive may not show yet
  Read-only apart from the control path: no rm, no mkdir, no writing files.";

/// Whether Drive holds real bytes for this row. The Docs-editors types (and
/// Forms, Maps, Drawings) do not — `alt=media` answers 403 for them.
fn has_original_bytes(mime: &str) -> bool {
    !mime.starts_with("application/vnd.google-apps.")
}

/// What an entry hands back when read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Serves {
    /// A directory — nothing to read.
    Nothing,
    /// The file's own bytes (`alt=media`), ranged.
    Original,
    /// The document's own structure, from its own API (`documents.get` and
    /// friends) rather than Drive.
    Native(NativeApi),
}

/// Which API answers for a document's structure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NativeApi {
    Doc,
    Sheet,
    Slides,
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
    /// name — plus a `.json` suffix when the entry serves a document's API JSON.
    vfs_name: String,
    id: String,
    /// Set when the entry lives in a shared drive (listing scope).
    drive_id: Option<String>,
    kind: GKind,
    /// Drive's own `mimeType`, reported as the entry's content type. For a document
    /// that is its Google type (a Sheet, not the JSON we serve), which is what a
    /// client picking an icon wants and the only place it exists at all — the Drive
    /// names those come from carry no extension.
    mime_type: Option<String>,
    mtime: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    /// What this entry hands back when read.
    serves: Serves,
    /// Byte length when it is known without fetching: Drive reports it for a file
    /// it holds bytes for. `None` = only producing the bytes can tell, which
    /// `entry_size` reports as [`UNKNOWN_LENGTH_SIZE`].
    size: Option<u64>,
}

/// One folder's children as the cache holds them: shared, so a `stat` or a read
/// borrows the listing instead of copying it (a shared folder in the corpus lists
/// 10,000 entries).
type CachedListing = (Instant, Arc<Vec<Child>>);

pub struct GdriveResource {
    accessor: GdriveAccessor,
    /// Lengths learned by probing (file id → bytes), for the rows Drive listed
    /// without a `size`. One probe per file per session: `stat` is asked once per
    /// entry after every listing, and a probe is a request.
    probed: Mutex<HashMap<String, u64>>,
    /// Per-directory listing cache (folder path → children). Path resolution walks
    /// parent listings, so one cached listing answers readdir, stat and the lookup
    /// a read starts with; fetching the bytes themselves still costs a request.
    dir_cache: Mutex<HashMap<String, CachedListing>>,
}

impl GdriveResource {
    pub fn new(config: &GdriveConfig, oauth: &GoogleClient) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: GdriveAccessor::new(config, oauth)?,
            probed: Mutex::new(HashMap::new()),
            dir_cache: Mutex::new(HashMap::new()),
        })
    }

    /// The mount root's virtual sections: `My Drive`, `Shared with me`, and
    /// (best-effort — needs scope; accounts without any list none) each shared
    /// drive as its own top-level directory.
    async fn root_sections(&self) -> (Vec<Child>, bool) {
        let section = |name: &str, id: &str, kind: GKind, drive_id: Option<String>| Child {
            vfs_name: name.to_string(),
            id: id.to_string(),
            drive_id,
            kind,
            // A section is a directory, and a directory's type is a directory.
            mime_type: None,
            mtime: None,
            created: None,
            serves: Serves::Nothing,
            size: None,
        };
        let mut children = vec![
            section(MY_DRIVE_NAME, MY_DRIVE_ID, GKind::Folder, None),
            section(SHARED_WITH_ME_NAME, SHARED_WITH_ME_ID, GKind::Folder, None),
        ];
        let listed = self.accessor.list_shared_drives().await;
        // A failure here is indistinguishable from an account with no shared drives,
        // so say so and let the caller keep the reduced root out of the cache: a
        // listing cached for the TTL would hide them for minutes with nothing to
        // explain their absence.
        if let Err(e) = &listed {
            tracing::warn!("gdrive: shared drives could not be listed, root omits them: {e:#}");
        }
        let complete = listed.is_ok();
        if let Ok(drives) = listed {
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
        (children, complete)
    }

    /// List a directory's immediate children (cached). The root is virtual (see
    /// [`Self::root_sections`]); everything else is a Drive listing.
    async fn list_dir(&self, folder: &str) -> ResourceResult<Arc<Vec<Child>>> {
        {
            let cache = self.dir_cache.lock().await;
            if let Some((at, children)) = cache.get(folder)
                && at.elapsed() < DIR_TTL
            {
                return Ok(children.clone());
            }
        }
        let mut complete = true;
        let mut children = if folder == "/" {
            let (sections, ok) = self.root_sections().await;
            complete = ok;
            sections
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

        let children = Arc::new(children);
        if complete {
            let mut cache = self.dir_cache.lock().await;
            // Drop what has aged out before adding to it. Nothing else ever removed an
            // entry, so a listing stayed for the life of the mount long after its TTL
            // made it unusable — one entry per folder ever visited, and the corpus has
            // a folder that lists 10,000 of them.
            cache.retain(|_, (at, _)| at.elapsed() < DIR_TTL);
            cache.insert(folder.to_string(), (Instant::now(), children.clone()));
        }
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

    /// A spreadsheet's JSON: its structure, with each tab's cell values folded in
    /// under `values`.
    ///
    /// Two calls, because Sheets has no single one that answers both cheaply.
    /// `spreadsheets.get` gives the workbook's shape (3.7-19.5 KB measured) and,
    /// with it, the tab titles that name the ranges; `values:batchGet` then returns
    /// the used range of every tab at once. The flag that looks like the direct
    /// route — `includeGridData=true` — bills per *allocated* cell, and the first
    /// real workbook tried allocated 210,125 for an estimated 189 MB, so it is not
    /// a route at all. See [`GdriveAccessor::sheet_values_batch`].
    ///
    /// A tab whose values exceed the budget is left out with a `valuesOmitted` note
    /// on it, so a reader sees a stated omission rather than an empty sheet.
    async fn spreadsheet_bytes(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let mut v: Value = serde_json::from_slice(&self.accessor.spreadsheet_json(id).await?)?;
        let titles: Vec<String> = tab_titles(&v);
        if titles.is_empty() {
            return pretty(&v);
        }
        // Whole tabs by name: an A1 range with no cell part means "everything used".
        // A values endpoint that is missing (a Drive-only mock) or forbidden (the
        // Sheets API not enabled on the project) costs the cells, not the read —
        // the workbook's shape is still worth serving, with the reason attached.
        let batch = match self.accessor.sheet_values_batch(id, &titles).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("gdrive: {id} values unavailable: {e:#}");
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("valuesUnavailable".into(), Value::String(format!("{e:#}")));
                }
                return pretty(&v);
            }
        };
        fold_values(&mut v, &batch, &titles);
        pretty(&v)
    }

    /// The probed length of `id`, remembered so the kernel's per-entry `getattr`
    /// storm costs one request, not one per stat. `None` when the probe fails —
    /// the placeholder stands rather than a wrong number.
    async fn probed_len(&self, id: &str) -> Option<u64> {
        if let Some(n) = self.probed.lock().await.get(id) {
            return Some(*n);
        }
        match self.accessor.probe_len(id).await {
            Ok(n) => {
                let mut probed = self.probed.lock().await;
                // A learned length is an optimisation, so losing one costs a request
                // rather than correctness — which is what lets this be a flat cap
                // instead of bookkeeping.
                if probed.len() >= MAX_PROBED_LENGTHS {
                    probed.clear();
                }
                probed.insert(id.to_string(), n);
                Some(n)
            }
            Err(e) => {
                tracing::debug!("gdrive: probing {id} for its length failed: {e:#}");
                None
            }
        }
    }

    /// How many listings are retained. Tests only: growth here is invisible from
    /// outside, which is how it went unbounded.
    #[cfg(test)]
    pub(crate) async fn listings_retained(&self) -> usize {
        self.dir_cache.lock().await.len()
    }

    /// Age every retained listing past its TTL. Tests only.
    #[cfg(test)]
    pub(crate) async fn age_listings_for_test(&self) {
        let mut cache = self.dir_cache.lock().await;
        let stale = Instant::now() - DIR_TTL - Duration::from_secs(1);
        for (at, _) in cache.values_mut() {
            *at = stale;
        }
    }

    /// Resolve any path (file or folder) to its child entry via its parent dir.
    async fn resolve(&self, path: &str) -> ResourceResult<Child> {
        let (parent, name) = split_last(path);
        let children = self.list_dir(&parent).await?;
        children
            .iter()
            .find(|c| c.vfs_name == name)
            .cloned()
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
                // fall back to slicing if it sent the whole object anyway. Saturating,
                // because `end - start` on a backwards range underflows and takes the
                // process with it.
                return Ok(match &range {
                    Some(r) if bytes.len() as u64 > r.end.saturating_sub(r.start) => {
                        slice(&bytes, range)
                    }
                    _ => bytes,
                });
            }
            // A document from its own API. A spreadsheet takes two calls, its
            // structure and its cells — see [`Self::spreadsheet_bytes`].
            Serves::Native(NativeApi::Sheet) => {
                let bytes = self
                    .spreadsheet_bytes(&child.id)
                    .await
                    .map_err(not_found_or_backend)?;
                return Ok(slice(&bytes, range));
            }
            Serves::Native(api) => {
                let bytes = match api {
                    NativeApi::Doc => self.accessor.document_json(&child.id).await,
                    _ => self.accessor.presentation_json(&child.id).await,
                }
                .map_err(not_found_or_backend)?;
                return Ok(slice(&bytes, range));
            }
            // Only a directory serves nothing, and directories were rejected
            // above — so this is unreachable for a resolved file.
            Serves::Nothing => Err(ResourceError::NotFound),
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
        // A file Drive listed without a size: learn the real one from one ranged
        // response header rather than leaving the placeholder, which would make
        // `ls -l` lie and a WebDAV `GET` truncate the body at 8 MB.
        let size = match (&child.serves, child.size) {
            (Serves::Original, None) => self.probed_len(&child.id).await,
            _ => None,
        };
        Ok(FileStat {
            kind: if child.kind.is_dir() {
                FileKind::Dir
            } else {
                FileKind::File
            },
            size: size.unwrap_or_else(|| entry_size(&child)),
            mtime: child.mtime,
            created: child.created,
            content_type: child.mime_type.clone(),
            // A probe that answered turned the placeholder into a length; one that
            // failed leaves it a placeholder, and saying so is what keeps a reader
            // from trusting it.
            size_is_estimate: size.is_none() && is_estimate(&child),
            // A document comes from its own API, which has no notion of a range.
            serves_whole: matches!(child.serves, Serves::Native(_)),
            ..Default::default()
        })
    }

    /// Content search over Drive's own index, designed to hang off the (not yet
    /// wired) `.cmd/` control path. Dormant today: nothing routes a write on
    /// `.cmd/<name>` here, so the prompt's line is a promise the transport has yet
    /// to keep. Kept rather than deferred because the search itself is the whole
    /// answer to a PDF's contents, which no read of the mount can give.
    ///
    /// The body is the phrase, plain text. One JSON line per hit, so a reader can
    /// pipe it into anything: `{"name","id","mimeType","size","modifiedTime"}`, plus an
    /// `unlisted` explanation on a hit this mount has no entry for (a Form, a Drawing —
    /// nothing can render them). Reporting those beats dropping them: the index found
    /// the phrase, and an answer of silence would read as an absence.
    async fn command(&self, name: &str, body: &[u8]) -> ResourceResult<Vec<u8>> {
        if name != "search" {
            return Err(ResourceError::Unsupported);
        }
        let phrase = std::str::from_utf8(body)
            .map_err(|e| ResourceError::Backend(anyhow::anyhow!("search: body not utf-8: {e}")))?
            .trim();
        if phrase.is_empty() {
            return Err(ResourceError::Backend(anyhow::anyhow!(
                "search: empty phrase"
            )));
        }
        let hits = self
            .accessor
            .search_fulltext(phrase, SEARCH_MAX_HITS)
            .await
            .map_err(not_found_or_backend)?;
        let mut out = Vec::new();
        for f in &hits {
            // A hit the mount has no entry for (a Form, a Map, a Drawing) is still a
            // hit: dropping it would answer "not found" for a file the index found.
            // Report it with the reason attached — a reader meets these lines on their
            // own, so the line has to explain itself rather than rely on a flag whose
            // meaning lives elsewhere.
            let child = child_from_file(f);
            let mut line = serde_json::json!({
                "name": child
                    .as_ref()
                    .map(|c| c.vfs_name.clone())
                    .or_else(|| f.get("name").and_then(|n| n.as_str()).map(str::to_string)),
                "id": child.as_ref().map(|c| c.id.clone()).or_else(|| {
                    f.get("id").and_then(|i| i.as_str()).map(str::to_string)
                }),
                "mimeType": f.get("mimeType"),
                "size": f.get("size"),
                "modifiedTime": f.get("modifiedTime"),
            });
            if child.is_none()
                && let Some(obj) = line.as_object_mut()
            {
                obj.insert(
                    "unlisted".into(),
                    Value::String(
                        "this type has no readable form, so it is not in the tree: \
                         no path to read, open it in Drive by id"
                            .into(),
                    ),
                );
            }
            out.extend_from_slice(&serde_json::to_vec(&line)?);
            out.push(b'\n');
        }
        Ok(out)
    }

    fn prompt(&self) -> &str {
        GDRIVE_PROMPT
    }

    /// Every file entry carries Drive's `mimeType`. A native doc is served as
    /// `.gdoc.json`/`.gsheet.json`/`.gslide.json`, whose extension says "JSON" and
    /// not which of the three it came from; Drive's own mime says that.
    fn reports_content_type(&self) -> bool {
        true
    }

    /// A document's JSON has to be produced before anyone knows how long it is, so
    /// sizing a listing means rendering every document in it. Measured against a
    /// real account: 2.4s for one folder's six documents concurrently, 6.3s when
    /// the kernel's per-entry `getattr` serialises them — spent every time a
    /// listing goes cold, on a number that `ls`, `find -name` and `Glob` never
    /// read. So the listing reports [`UNKNOWN_LENGTH_SIZE`] and the length
    /// becomes exact as soon as anything reads the file: a handle resolves it on
    /// `open` (`File::metadata`), and the cache answers from the bytes it kept.
    ///
    /// Notion opts in instead — its render is a page fetch, not a document export.
    fn resolve_size_on_stat(&self) -> bool {
        false
    }

    // `listings_complete` stays at the default `true`: a per-folder
    // `files.list` is complete at fetch time (the >MAX_FOLDER_FILES truncation
    // case is pathological and logged), so a missing child really is missing.
}

/// The size an entry reports.
///
/// A file Drive holds bytes for is exact: its length comes in the listing, so
/// `ls -l` is honest and a ranged read lands where the caller asked. Everything
/// else reports [`UNKNOWN_LENGTH_SIZE`], because knowing the real length would mean
/// producing the document first.
fn entry_size(c: &Child) -> u64 {
    match (c.kind.is_dir(), c.size) {
        (true, _) => 0,
        (_, Some(n)) => n,
        // A document nobody has read yet: see UNKNOWN_LENGTH_SIZE.
        (_, None) => UNKNOWN_LENGTH_SIZE,
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
        size_is_estimate: is_estimate(c),
        serves_whole: matches!(c.serves, Serves::Native(_)),
    }
}

/// Whether this entry's size is a placeholder rather than a length.
///
/// True for a document, whose JSON nobody has measured until it is produced, and
/// true for a file Drive listed without a `size` — the row this once called exact,
/// on the reasoning that resolving it meant downloading the object. `probe_len`
/// retired that reasoning: the length is one ranged byte away.
///
/// Calling it exact was not a cosmetic error. The listing's placeholder is served
/// verbatim by the metadata cache's stat fast path, so `probe_len` never ran for any
/// caller that lists before it reads — which is all of them — and the read strategy
/// read 8 MiB as a known length at or under the cacheable limit, fetching the whole
/// object once per window and then discarding it. Measured through the wrapper
/// production installs: four 256 KiB windows of a 20 MB file moved 80 MB.
fn is_estimate(c: &Child) -> bool {
    matches!(c.serves, Serves::Native(_)) || c.size.is_none()
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
/// has nothing to serve for it (Forms, Maps and Drawings answer no API and export
/// to nothing — a name that cannot be read is worse than an absence).
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
            serves: Serves::Nothing,
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
        // A native document: served as its own API's JSON.
        let (api, suffix) = native_kind(mime)?;
        (format!("{name}{suffix}"), Serves::Native(api), None)
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

/// Attach each tab's values to the tab they belong to.
///
/// Paired by sheet title, not by position. `valueRanges` comes back in request order,
/// but the request is built from a *filtered and truncated* view of `sheets` — a sheet
/// with no title cannot be addressed and a workbook past [`MAX_TABS`] is cut off — so
/// walking the two in step hands one tab another's cells and shifts every tab after
/// it. `valueRanges[].range` names its own sheet, which removes the guesswork.
///
/// A tab that ends up with no values says why: it was never requested, nothing came
/// back for it, or its cells did not fit the budget. The budget is spent tab by tab
/// and an oversized one does not consume the remainder — a 20-byte tab after a large
/// one still fits.
fn fold_values(workbook: &mut Value, batch: &Value, requested: &[String]) {
    let mut by_title: HashMap<String, Value> = HashMap::new();
    for vr in batch
        .get("valueRanges")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
    {
        let Some(title) = vr.get("range").and_then(|r| r.as_str()).map(range_title) else {
            continue;
        };
        let values = vr.get("values").cloned().unwrap_or(Value::Array(vec![]));
        by_title.insert(title, values);
    }
    let asked: HashSet<&str> = requested.iter().map(String::as_str).collect();

    let mut budget = GRID_BYTES_BUDGET;
    for tab in workbook
        .get_mut("sheets")
        .and_then(|s| s.as_array_mut())
        .into_iter()
        .flatten()
    {
        let title = tab
            .pointer("/properties/title")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let Some(obj) = tab.as_object_mut() else {
            continue;
        };
        let note = |reason: &str| serde_json::json!({ "reason": reason });
        let Some(title) = title else {
            obj.insert(
                "valuesOmitted".into(),
                note("this sheet has no title, so its cells cannot be addressed"),
            );
            continue;
        };
        if !asked.contains(title.as_str()) {
            obj.insert(
                "valuesOmitted".into(),
                serde_json::json!({
                    "reason": "past the tab cap, so its values were never requested",
                    "tabCap": MAX_TABS,
                }),
            );
            continue;
        }
        let Some(values) = by_title.get(&title) else {
            obj.insert(
                "valuesOmitted".into(),
                note("the values request returned nothing for this sheet"),
            );
            continue;
        };
        let cost = served_len(values);
        if cost > budget {
            obj.insert(
                "valuesOmitted".into(),
                serde_json::json!({
                    "reason": "over the size budget",
                    "bytes": cost,
                    "budgetLeft": budget,
                }),
            );
            continue;
        }
        budget -= cost;
        obj.insert("values".into(), values.clone());
    }
}

/// The sheet a returned A1 range belongs to: `'메인화면'!A1:Z968` -> `메인화면`.
///
/// The title is everything before the last `!`, unquoted — a quoted title may itself
/// contain `!`, and a literal quote inside one arrives doubled.
fn range_title(range: &str) -> String {
    let sheet = match range.rsplit_once('!') {
        Some((sheet, _)) => sheet,
        None => range,
    };
    match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => sheet.to_string(),
    }
}

/// Pretty-printed JSON with a trailing newline — the form every document is served
/// in, so the bytes read as lines rather than one long string.
fn pretty(v: &Value) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(v)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// A workbook's tab titles, in order, capped at [`MAX_TABS`].
fn tab_titles(workbook: &Value) -> Vec<String> {
    workbook
        .get("sheets")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.pointer("/properties/title")?.as_str())
        .map(str::to_string)
        .take(MAX_TABS)
        .collect()
}

/// How many bytes `v` will add to the served file, counted without building them.
///
/// Pretty-printed, because that is the form the file is served in: a values array is
/// rows of columns of short strings, and indenting one puts every cell on its own
/// line. Measured on a 200x20 grid, that is 1.66x the compact form — so a budget
/// checked against compact bytes admits a file half again as large as it allows.
fn served_len(v: &Value) -> u64 {
    struct Counting(u64);
    impl std::io::Write for Counting {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len() as u64;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Counting(0);
    serde_json::to_writer_pretty(&mut c, v)
        .map(|()| c.0)
        .unwrap_or(0)
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

/// Give every child a unique `vfs_name`: on a collision, number it ` (2)`, ` (3)`,
/// … so duplicate Drive names don't shadow each other in readdir/resolve.
///
/// The number goes *before* the extension. Appending it (`sheet.gsheet.json (2)`)
/// kept the name unique but took the entry out of every glob a reader would use to
/// find it — measured against a real account, two of 33 spreadsheets were invisible
/// to `**/*.gsheet.json`.
fn disambiguate(children: &mut [Child]) {
    let mut seen: HashSet<String> = HashSet::new();
    for c in children.iter_mut() {
        if seen.insert(c.vfs_name.clone()) {
            continue;
        }
        let (stem, ext) = split_extension(&c.vfs_name, c.serves);
        let mut n = 2;
        loop {
            let cand = format!("{stem} ({n}){ext}");
            if seen.insert(cand.clone()) {
                c.vfs_name = cand;
                break;
            }
            n += 1;
        }
    }
}

/// Split a listing name into the part a number can follow and the extension it must
/// stay in front of.
///
/// A document's suffix is known exactly (`.gsheet.json`, not `.json`). A file keeps
/// whatever follows its last dot when that looks like an extension. A directory has
/// no extension to protect, so `v1.2` numbers as `v1.2 (2)`.
fn split_extension(name: &str, serves: Serves) -> (&str, &str) {
    if serves == Serves::Nothing {
        return (name, "");
    }
    if let Serves::Native(api) = serves {
        let suffix = NATIVE_KINDS
            .iter()
            .find(|(_, a, _)| *a == api)
            .map(|(_, _, s)| *s)
            .unwrap_or("");
        if let Some(stem) = name.strip_suffix(suffix) {
            return (stem, suffix);
        }
    }
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, &name[stem.len()..])
        }
        _ => (name, ""),
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

/// The requested window of `data`, clamped to what exists.
///
/// Both ends are clamped and the end is never allowed below the start: a range that
/// asks backwards is empty, not a panic. The caller is a filesystem read, so the range
/// arrives from whatever a client asked for, and `data[4..2]` aborts the process.
fn slice(data: &[u8], range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len()).max(start);
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

        // Docs-editors types: one entry, the document's own API JSON. The suffix
        // says which kind it is, since the Drive name carries no extension.
        for (mime, suffix, api) in [
            (
                "application/vnd.google-apps.document",
                ".gdoc.json",
                NativeApi::Doc,
            ),
            (
                "application/vnd.google-apps.spreadsheet",
                ".gsheet.json",
                NativeApi::Sheet,
            ),
            (
                "application/vnd.google-apps.presentation",
                ".gslide.json",
                NativeApi::Slides,
            ),
        ] {
            let c = entry("Q3 Plan", mime).unwrap();
            assert_eq!(c.vfs_name, format!("Q3 Plan{suffix}"), "{mime}");
            assert_eq!(c.serves, Serves::Native(api), "{mime}");
            // Its length is only known once the API has answered.
            assert_eq!(entry_size(&c), UNKNOWN_LENGTH_SIZE, "{mime}");
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
        assert_eq!(doc.vfs_name, "분기 보고서.gdoc.json");
    }

    /// What a `stat` claims about a node decides how a read of it is fetched, so the
    /// two flags have to line up with what the entry actually is.
    #[test]
    fn a_document_says_it_has_no_ranges_and_a_file_says_it_has() {
        let doc = child_from_file(&file_row(
            "가이드",
            "d1",
            "application/vnd.google-apps.document",
        ))
        .unwrap();
        assert!(matches!(doc.serves, Serves::Native(_)));
        assert!(is_estimate(&doc), "its length is unknown until rendered");

        let pdf = child_from_file(&file_row("보고서.pdf", "p1", "application/pdf")).unwrap();
        assert!(matches!(pdf.serves, Serves::Original));
        assert!(!is_estimate(&pdf), "Drive gave the length in the listing");

        let mut row = file_row("mystery.bin", "m1", "application/octet-stream");
        row.as_object_mut().unwrap().remove("size");
        let unsized_file = child_from_file(&row).unwrap();
        assert!(matches!(unsized_file.serves, Serves::Original));
        assert_eq!(unsized_file.size, None, "the length has to be asked for");
    }

    /// A row Drive reported no size for still lists, with the placeholder rather
    /// than 0 — under the guest's `direct_io` mount a 0 was measured to clamp reads
    /// to nothing, and a search tool skips a file it is told is empty.
    #[test]
    fn an_unknown_size_is_a_placeholder_never_zero() {
        let mut row = file_row("mystery.bin", "id1", "application/octet-stream");
        row.as_object_mut().unwrap().remove("size");
        let c = child_from_file(&row).unwrap();
        assert_eq!(c.serves, Serves::Original);
        assert_eq!(entry_size(&c), UNKNOWN_LENGTH_SIZE);
        // Never 0 (that reads as empty), and at or under the content cache's
        // per-object limit, so the first read of a document replaces the
        // placeholder with its exact length for every later listing.
        const _: () = assert!(UNKNOWN_LENGTH_SIZE > 0);
        const _: () = assert!(UNKNOWN_LENGTH_SIZE <= 8 * 1024 * 1024);
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
        assert_eq!(native.vfs_name, "untitled.gdoc.json");
    }

    #[test]
    fn disambiguate_keeps_a_name_findable_by_its_extension() {
        let mk = |n: &str, serves: Serves| Child {
            vfs_name: n.to_string(),
            id: "x".into(),
            drive_id: None,
            kind: if matches!(serves, Serves::Nothing) {
                GKind::Folder
            } else {
                GKind::File
            },
            mime_type: None,
            mtime: None,
            created: None,
            serves,
            size: None,
        };
        let mut children = vec![
            // Three spreadsheets of the same Drive name: the number has to land
            // before `.gsheet.json` or a `**/*.gsheet.json` search loses two of them.
            mk("report.gsheet.json", Serves::Native(NativeApi::Sheet)),
            mk("report.gsheet.json", Serves::Native(NativeApi::Sheet)),
            mk("report.gsheet.json", Serves::Native(NativeApi::Sheet)),
            // A plain file keeps its own extension.
            mk("photo.jpeg", Serves::Original),
            mk("photo.jpeg", Serves::Original),
            // No extension to preserve, and a folder is not renamed around a dot.
            mk("notes", Serves::Original),
            mk("notes", Serves::Original),
            mk("v1.2", Serves::Nothing),
            mk("v1.2", Serves::Nothing),
        ];
        disambiguate(&mut children);
        let names: Vec<&str> = children.iter().map(|c| c.vfs_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "report.gsheet.json",
                "report (2).gsheet.json",
                "report (3).gsheet.json",
                "photo.jpeg",
                "photo (2).jpeg",
                "notes",
                "notes (2)",
                "v1.2",
                "v1.2 (2)",
            ]
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

    /// The command only answers to its own name, and refuses a body that names
    /// nothing — a search for "" would otherwise return the whole Drive.
    #[tokio::test]
    async fn search_rejects_what_it_cannot_answer() {
        let r = GdriveResource::new(
            &GdriveConfig {
                refresh_token: "x".into(),
                origins: crate::vfs::accessor::Origins::behind("http://127.0.0.1:1"),
            },
            &GoogleClient {
                client_id: "x".into(),
                client_secret: "x".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            r.command("page-create", b"{}").await,
            Err(ResourceError::Unsupported)
        ));
        for empty in ["", "   ", "\n"] {
            assert!(
                r.command("search", empty.as_bytes()).await.is_err(),
                "an empty phrase must not become a search"
            );
        }
    }

    /// The budget bounds the file that gets served, so it has to be measured in the
    /// form that gets served: indenting a grid of short cells costs half again its
    /// compact size (1.66x measured here), and a budget checked against the compact
    /// form quietly allows that much more.
    #[test]
    fn a_tabs_cost_is_measured_as_it_will_be_written() {
        let values: Value = serde_json::json!(
            (0..200)
                .map(|r| (0..20).map(|c| format!("{r}-{c}")).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        let compact = serde_json::to_vec(&values).unwrap().len() as u64;
        let served = served_len(&values);
        assert_eq!(
            served,
            serde_json::to_vec_pretty(&values).unwrap().len() as u64,
            "counted, not estimated"
        );
        assert!(
            served * 2 > compact * 3,
            "indenting a grid costs at least half again: {compact} -> {served}"
        );
    }

    /// The two listing caches have to expire together: a read resolves its path
    /// through this one, and a `readdir` answers from the wrapper's. Different
    /// numbers meant re-fetching a listing the wrapper still held, and a window
    /// where the two disagreed about what a folder contained.
    #[test]
    fn the_listing_ttl_matches_the_cache_above_it() {
        assert_eq!(DIR_TTL, crate::vfs::cache::listing_ttl());
    }

    /// A search reports what the index found, including the types this mount cannot
    /// serve — a Form has no readable form, but "the phrase is in this Form" is still
    /// the answer to the question.
    #[tokio::test]
    async fn a_search_hit_the_mount_cannot_serve_is_still_reported() {
        let form = file_row("설문지", "f1", "application/vnd.google-apps.form");
        assert!(
            child_from_file(&form).is_none(),
            "a Form has no entry in the tree"
        );
        // The command's shaping of a hit keeps the name and id either way; the note is
        // what tells them apart, and it says why in the line itself.
        let served = file_row("보고서.pdf", "p1", "application/pdf");
        for (row, listed) in [(&form, false), (&served, true)] {
            let child = child_from_file(row);
            assert_eq!(child.is_some(), listed);
            let name = child
                .as_ref()
                .map(|c| c.vfs_name.clone())
                .or_else(|| row.get("name").and_then(|n| n.as_str()).map(str::to_string));
            assert!(name.is_some(), "a hit always has a name to report");
        }
    }

    fn workbook(titles: &[Option<&str>]) -> Value {
        serde_json::json!({
            "sheets": titles
                .iter()
                .map(|t| match t {
                    Some(t) => serde_json::json!({ "properties": { "title": t } }),
                    None => serde_json::json!({ "properties": {} }),
                })
                .collect::<Vec<_>>()
        })
    }

    fn batch(pairs: &[(&str, &str)]) -> Value {
        serde_json::json!({
            "valueRanges": pairs
                .iter()
                .map(|(range, cell)| serde_json::json!({
                    "range": range,
                    "values": [[cell]],
                }))
                .collect::<Vec<_>>()
        })
    }

    fn values_of(wb: &Value, i: usize) -> Option<String> {
        wb["sheets"][i]["values"][0][0].as_str().map(str::to_string)
    }

    fn omitted_reason(wb: &Value, i: usize) -> Option<String> {
        wb["sheets"][i]["valuesOmitted"]["reason"]
            .as_str()
            .map(str::to_string)
    }

    /// Values are paired by the sheet they name, not by their position in the reply.
    /// A sheet the request had to skip — one with no title — used to consume the next
    /// sheet's values and shift the rest, which is the same wrong-cells outcome
    /// `quote_a1` exists to prevent, reached from the other side.
    #[test]
    fn a_skipped_sheet_does_not_shift_everyone_elses_values() {
        let mut wb = workbook(&[Some("Alpha"), None, Some("Gamma")]);
        // The request only asked for the titled sheets.
        let asked = vec!["Alpha".to_string(), "Gamma".to_string()];
        fold_values(
            &mut wb,
            &batch(&[("Alpha!A1", "ALPHA-CELL"), ("Gamma!A1", "GAMMA-CELL")]),
            &asked,
        );
        assert_eq!(values_of(&wb, 0).as_deref(), Some("ALPHA-CELL"));
        assert_eq!(values_of(&wb, 2).as_deref(), Some("GAMMA-CELL"));
        assert_eq!(values_of(&wb, 1), None, "the titleless sheet gets nothing");
        assert!(
            omitted_reason(&wb, 1).is_some_and(|r| r.contains("no title")),
            "and says why"
        );
    }

    /// Past the cap no request was made, so the tail has no values — and used to have
    /// no explanation either, the one silent omission in this file.
    #[test]
    fn a_tab_past_the_cap_says_it_was_never_asked_for() {
        let titles: Vec<String> = (0..MAX_TABS + 2).map(|i| format!("T{i}")).collect();
        let mut wb = workbook(&titles.iter().map(|t| Some(t.as_str())).collect::<Vec<_>>());
        let asked = titles[..MAX_TABS].to_vec();
        let pairs: Vec<(String, String)> = asked
            .iter()
            .map(|t| (format!("{t}!A1"), format!("{t}-CELL")))
            .collect();
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(r, c)| (r.as_str(), c.as_str()))
            .collect();
        fold_values(&mut wb, &batch(&refs), &asked);

        assert_eq!(values_of(&wb, 0).as_deref(), Some("T0-CELL"));
        for i in MAX_TABS..MAX_TABS + 2 {
            assert_eq!(values_of(&wb, i), None);
            assert!(
                omitted_reason(&wb, i).is_some_and(|r| r.contains("tab cap")),
                "tab {i} must not be silently empty"
            );
        }
    }

    /// One oversized tab used to zero the budget, so every later tab was dropped
    /// however small. The budget is spent tab by tab, which is what the constant says.
    #[test]
    fn an_oversized_tab_does_not_spend_the_rest_of_the_budget() {
        let mut wb = workbook(&[Some("small1"), Some("huge"), Some("small2")]);
        let huge = "x".repeat(GRID_BYTES_BUDGET as usize + 1);
        let mut b = batch(&[("small1!A1", "a"), ("small2!A1", "b")]);
        b["valueRanges"].as_array_mut().unwrap().insert(
            1,
            serde_json::json!({ "range": "huge!A1", "values": [[huge]] }),
        );
        fold_values(&mut wb, &b, &["small1", "huge", "small2"].map(String::from));

        assert_eq!(values_of(&wb, 0).as_deref(), Some("a"));
        assert!(omitted_reason(&wb, 1).is_some_and(|r| r.contains("budget")));
        assert_eq!(
            values_of(&wb, 2).as_deref(),
            Some("b"),
            "a small tab after a large one still fits"
        );
    }

    /// The reply names its sheet in A1 notation, which is what the pairing reads.
    #[test]
    fn a_returned_range_names_its_sheet() {
        assert_eq!(range_title("Sheet1!A1:Z1000"), "Sheet1");
        assert_eq!(range_title("'메인화면'!A1:Z968"), "메인화면");
        // A quoted title can hold the characters that would otherwise confuse this.
        assert_eq!(range_title("'Sheet1!B2'!A1:Z10"), "Sheet1!B2");
        assert_eq!(range_title("'it''s'!A1"), "it's");
        assert_eq!(range_title("Sheet1"), "Sheet1");
    }

    /// A range that asks backwards is empty, not a crash. The window arrives from
    /// whatever a client asked for, and `data[4..2]` aborts the process rather than
    /// erroring — which makes this the provider's job, not the caller's.
    #[test]
    fn a_backwards_window_is_empty_rather_than_fatal() {
        let data = b"abcdef";
        // Built from values: a literal `4..2` is a lint, but a range arriving from a
        // client is just two numbers.
        let backwards = |start: u64, end: u64| slice(data, Some(start..end));
        assert!(backwards(4, 2).is_empty());
        assert!(backwards(9, 3).is_empty());
        assert!(backwards(6, 6).is_empty());
        // Past the end clamps to the end; the ordinary cases keep working.
        assert_eq!(backwards(4, 99).len(), 2);
        assert_eq!(slice(data, Some(1..3)), b"bc");
        assert_eq!(slice(data, None), data);
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

    /// Config for the enterprise-mock integration test. The mock hands the refresh
    /// token straight back as the bearer token, so any user token from its
    /// `tokens.yaml` (or the admin token) works with the normal OAuth flow.
    ///
    /// The token var is deliberately not `GOOGLE_REFRESH_TOKEN`: sharing it with
    /// [`live_config`] means one shell with both set sends a real Google token to the
    /// mock, which then fails for a reason that looks like a code bug.
    fn mock_config() -> Option<(GdriveConfig, GoogleClient)> {
        Some((
            GdriveConfig {
                refresh_token: std::env::var("GOOGLE_MOCK_TOKEN")
                    .unwrap_or_else(|_| "admin-service-token".into()),
                origins: crate::vfs::accessor::Origins::behind(
                    &std::env::var("GOOGLE_API_BASE_URL").ok()?,
                ),
            },
            // A mock accepts any client; what matters is that one is supplied at all.
            GoogleClient {
                client_id: "mock".into(),
                client_secret: "mock".into(),
            },
        ))
    }

    /// Tree round-trip against an enterprise-mock, local or hosted. Ignored by
    /// default; run with:
    ///
    ///   # local: python -m app.importer.byo \
    ///   #   examples/bring-your-own-corpus/sample_corpus.jsonl
    ///   #        then python -m uvicorn app.main:app --port 8000
    ///   GOOGLE_API_BASE_URL=http://localhost:8000 \
    ///     [GOOGLE_MOCK_TOKEN=…] cargo test -p workspace gdrive_mock -- --ignored --nocapture
    ///
    /// The walk is bounded so the same test runs against a five-file sample
    /// corpus and a 25k-document hosted one; a real corpus also spans several
    /// listing pages, which exercises the accessor's pagination for free.
    #[tokio::test]
    #[ignore = "requires a running enterprise-mock (GOOGLE_API_BASE_URL)"]
    async fn gdrive_mock_tree_and_reads() {
        /// Bounds on the walk — enough to cross a page boundary on a real
        /// corpus, small enough to stay quick on a tiny one.
        const WALK_DIRS: usize = 12;
        const WALK_FILES: usize = 200;

        let Some((cfg, oauth)) = mock_config() else {
            eprintln!("set GOOGLE_API_BASE_URL (e.g. http://localhost:8000) to run");
            return;
        };
        let r = GdriveResource::new(&cfg, &oauth).unwrap();

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
                if is_native_json(e) {
                    // A document's JSON: the listing could only estimate its
                    // length, so just check the read produced something.
                    converted += 1;
                    assert!(!bytes.is_empty(), "{p}: native json was empty");
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

    fn live_config() -> Option<(GdriveConfig, GoogleClient)> {
        Some((
            GdriveConfig {
                refresh_token: std::env::var("GOOGLE_REFRESH_TOKEN").ok()?,
                origins: Default::default(),
            },
            GoogleClient {
                client_id: std::env::var("GOOGLE_CLIENT_ID").ok()?,
                client_secret: std::env::var("GOOGLE_CLIENT_SECRET").ok()?,
            },
        ))
    }

    /// Live tree round-trip against real Google Drive: the root sections, both
    /// section listings, and one ranged read per section at the length the
    /// listing promised. Ignored by default; run with:
    ///
    ///   GOOGLE_CLIENT_ID=… GOOGLE_CLIENT_SECRET=… GOOGLE_REFRESH_TOKEN=… \
    ///     cargo test -p workspace gdrive_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires GOOGLE_* env + network"]
    async fn gdrive_live_tree_and_reads() {
        let Some((cfg, oauth)) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg, &oauth).unwrap();

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
                .find(|e| e.kind == FileKind::File && !is_native_json(e) && e.size > 0)
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
    /// Live: a Docs-editors document is served as its own API's JSON, and a
    /// spreadsheet's stays small — the grid it deliberately omits runs to hundreds
    /// of megabytes.
    #[tokio::test]
    #[ignore = "requires GOOGLE_* env + network"]
    async fn gdrive_live_native_json_is_served() {
        let Some((cfg, oauth)) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg, &oauth).unwrap();

        let mut seen = 0usize;
        for section in [SHARED_WITH_ME_NAME, MY_DRIVE_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            for suffix in [".gsheet.json", ".gdoc.json", ".gslide.json"] {
                let Some(e) = entries.iter().find(|e| e.name.ends_with(suffix)) else {
                    continue;
                };
                let p = MountPath::new(format!("/{section}/{}", e.name));
                let t0 = std::time::Instant::now();
                let bytes = r.read_bytes(&p, None).await.expect("read native json");
                let v: Value = serde_json::from_slice(&bytes).expect("native json parses");
                eprintln!(
                    "  {} -> {} bytes in {:.2}s, top keys {:?}",
                    e.name,
                    bytes.len(),
                    t0.elapsed().as_secs_f64(),
                    v.as_object().map(|o| o.keys().take(4).collect::<Vec<_>>())
                );
                // A spreadsheet carries its cells, unless its allocated grid is
                // over the limit — then it says so instead of moving 200-349MB.
                if suffix == ".gsheet.json" {
                    assert!(v.get("sheets").is_some(), "workbook shape is present");
                    // `includeGridData` would put cells under sheets[].data. Nothing
                    // should be taking that route — it measured 189MB on this very
                    // workbook.
                    assert!(
                        !v["sheets"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|s| s.get("data").is_some()),
                        "{}: cells must come from values, not the allocated grid",
                        e.name
                    );
                    let tabs = v["sheets"].as_array().cloned().unwrap_or_default();
                    for t in &tabs {
                        eprintln!(
                            "    tab {:?}: {} rows{}",
                            t.pointer("/properties/title").and_then(|x| x.as_str()),
                            t["values"].as_array().map_or(0, |r| r.len()),
                            if t.get("valuesOmitted").is_some() {
                                " (values omitted)"
                            } else {
                                ""
                            }
                        );
                    }
                    assert!(
                        tabs.iter()
                            .all(|t| t.get("values").is_some() || t.get("valuesOmitted").is_some()),
                        "{}: every tab carries its values or says why not",
                        e.name
                    );
                    // The point of carrying values at all: a cell's text is in the
                    // bytes, so a reader searching the tree finds it. Cells holding a
                    // line break are excluded on purpose — JSON escapes those to
                    // `\n`, so a phrase spanning one is not literally in the file.
                    let a_cell = tabs
                        .iter()
                        .flat_map(|t| t["values"].as_array().cloned().unwrap_or_default())
                        .flat_map(|row| row.as_array().cloned().unwrap_or_default())
                        .find_map(|c| {
                            c.as_str()
                                .filter(|s| s.trim().len() > 3 && !s.contains(['\n', '"', '\\']))
                                .map(str::to_string)
                        })
                        .expect("some cell holds plain text");
                    eprintln!("    a cell reads {a_cell:?}");
                    assert!(
                        String::from_utf8_lossy(&bytes).contains(&a_cell),
                        "a cell's text is greppable in the served bytes"
                    );
                }
                seen += 1;
            }
            if seen > 0 {
                break;
            }
        }
        assert!(seen > 0, "no native document found");
    }

    /// Live: an original is a real file, and a ranged read transfers only its
    /// range — what makes serving originals affordable, since a search tool
    /// samples a file's head before deciding it is binary.
    /// The account a token belongs to is a question the token can answer, which is why
    /// nothing stores it: `about.get` is the single source of truth, and two mounts
    /// consented to two accounts return two different emails.
    #[tokio::test]
    #[ignore = "requires GOOGLE_* env + network"]
    async fn gdrive_live_a_token_names_its_own_account() {
        let Some((cfg, oauth)) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        let email = GdriveAccessor::new(&cfg, &oauth)
            .unwrap()
            .account_email()
            .await
            .expect("about.get");
        // Printed by shape, not by value: a live run should not put someone's address
        // into a log.
        let (local, domain) = email.split_once('@').expect("an email address");
        eprintln!(
            "  token belongs to an account at {domain} (local part {} chars)",
            local.chars().count()
        );
        assert!(!local.is_empty() && domain.contains('.'));
        assert_eq!(email, email.to_lowercase(), "normalised for comparison");
    }

    /// The one search the mount cannot do by reading: a phrase that lives inside a
    /// PDF, which no read of the bytes will match.
    #[tokio::test]
    #[ignore = "requires GOOGLE_* env + network"]
    async fn gdrive_live_search_reaches_inside_a_pdf() {
        let Some((cfg, oauth)) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg, &oauth).unwrap();
        let phrase = std::env::var("GOOGLE_SEARCH_PHRASE").unwrap_or_else(|_| "cloud".into());
        let t0 = std::time::Instant::now();
        let out = r
            .command("search", phrase.as_bytes())
            .await
            .expect("search");
        let lines: Vec<&str> = std::str::from_utf8(&out)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        eprintln!(
            "search {phrase:?}: {} hits in {:.2}s",
            lines.len(),
            t0.elapsed().as_secs_f64()
        );
        for l in lines.iter().take(8) {
            let v: Value = serde_json::from_str(l).expect("each hit is one JSON line");
            eprintln!(
                "  {:>12} {:<44} {}",
                v["size"].as_str().unwrap_or("-"),
                v["name"].as_str().unwrap_or("?"),
                v["mimeType"].as_str().unwrap_or("-")
            );
            assert!(v.get("id").is_some(), "a hit names the file it found");
        }
    }

    #[tokio::test]
    #[ignore = "requires GOOGLE_* env + network"]
    async fn gdrive_live_originals_read_by_range() {
        let Some((cfg, oauth)) = live_config() else {
            eprintln!("set GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET / GOOGLE_REFRESH_TOKEN to run");
            return;
        };
        let r = GdriveResource::new(&cfg, &oauth).unwrap();

        for section in [MY_DRIVE_NAME, SHARED_WITH_ME_NAME] {
            let entries = r
                .readdir(&MountPath::new(format!("/{section}")))
                .await
                .expect("section readdir");
            // The biggest original in the section: exactly the file a full read
            // must never be needed for.
            let Some(big) = entries
                .iter()
                .filter(|e| e.kind == FileKind::File && !is_native_json(e) && e.size > 0)
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
