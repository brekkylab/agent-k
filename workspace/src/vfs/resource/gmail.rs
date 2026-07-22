use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use serde_json::{Value, json};

use crate::vfs::{
    accessor::{GmailAccessor, GmailConfig, encode_b64url},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

/// Listing TTL for the label set (cheap, changes rarely).
const LABEL_TTL: Duration = Duration::from_secs(60);
/// Raw-message cache TTL — dedups the per-message fetches that readdir, stat,
/// read, and unlink all need for the same id. Matches [`INDEX_TTL`]: message
/// content is immutable (only the `labels` field can change externally, and
/// the label index already tolerates that much staleness), so a longer TTL
/// can't serve wrong bytes — while a short one made a scan re-fetch: grep's
/// `cat`s of a listed month often trail the listing (which batch-warmed the
/// cache) by more than 30s, so entries expired mid-scan and were re-fetched
/// one by one. Mutations through the mount (trash) invalidate explicitly;
/// memory stays bounded by [`MSG_CACHE_MAX`], not the TTL.
const MSG_TTL: Duration = Duration::from_secs(600);
/// Max entries in `msg_cache`. Each is a full message JSON, so an unbounded
/// cache would grow without limit during a large scan (`grep`); past this,
/// entries are evicted oldest-first (which is also expired-first).
const MSG_CACHE_MAX: usize = 2048;
/// Concurrency for the per-message fetches a directory listing fans out.
const FETCH_CONCURRENCY: usize = 6;
/// TTL for a label's date→ids index. The index is a full label scan, so it's
/// cached (like a directory listing) and shared by every `ls` under the label.
const INDEX_TTL: Duration = Duration::from_secs(600);
/// Two message-listings of the same label within this window read as a
/// sequential (grep-style) scan rather than a one-off browse. Generous because
/// grep reads a whole month's files *between* listings — a big month takes
/// tens of seconds, and a tight window would reset detection on exactly the
/// mailboxes that benefit from read-ahead.
const SEQ_WINDOW: Duration = Duration::from_secs(60);
/// Consecutive same-label listings needed before we treat it as a scan and
/// read ahead one month. 2 = the second month dir grep opens; a single
/// targeted browse (one month) never reaches it, so browse pays no read-ahead.
const SEQ_THRESHOLD: u32 = 2;
/// Safety ceiling on how many messages a label's index covers. The scan costs
/// one `messages.get` per message under Gmail's per-user rate limit (~5k index
/// ≈ 1–2 min; reading all their bodies is ~2x more), so an unbounded scan of a
/// pathologically large label (millions) would hang and burn quota. Beyond this
/// the newest N are indexed; older ones are omitted.
const MAX_INDEX_MESSAGES: usize = 5_000;
/// Over-estimate reported by `stat` for an `.gmail.json` whose processed length
/// isn't known without fetching. The guest kernel clamps reads at the reported
/// size even under direct_io, so this must exceed any real email JSON; reads
/// return the real bytes then empty at EOF. Attachments report their exact size
/// (known from the part).
const MSG_SENTINEL_SIZE: u64 = 16 * 1024 * 1024;
const GMAIL_SUFFIX: &str = ".gmail.json";

/// Attachment-bytes cache TTL. The attachments endpoint has no ranged fetch,
/// so without this every FUSE chunk read of one file re-downloads the whole
/// attachment (~200 full downloads to `cat` a 25 MB file). 30s absorbs the
/// chunk sequence of a single read plus near-term re-reads (grep then cat);
/// the bytes are immutable, so a longer TTL would also be correct — the cap
/// only bounds memory.
const ATT_TTL: Duration = Duration::from_secs(30);
/// Total-bytes budget for cached attachments. Unlike message JSON (small),
/// attachments run to ~25 MB each, so the cache evicts (expired first, then
/// oldest) to stay under this.
const ATT_CACHE_BUDGET: u64 = 128 * 1024 * 1024;

/// `(display_name, label_id)` pairs for every Gmail label.
type LabelList = Vec<(String, String)>;

/// A label's message ids grouped by received (UTC) date: `yyyy-mm-dd` → ids.
/// A `BTreeMap` keeps dates sorted so the listing is deterministic.
type DateIndex = BTreeMap<String, Vec<String>>;

pub struct GmailResource {
    accessor: GmailAccessor,
    /// `(display_name, label_id)` for every label, cached briefly.
    label_cache: tokio::sync::Mutex<Option<(Instant, LabelList)>>,
    /// Full raw message JSON by id, cached briefly and bounded ([`MsgCache`]).
    msg_cache: tokio::sync::Mutex<MsgCache>,
    /// Per-label "date → message ids" index, built by one full label scan and
    /// cached ([`INDEX_TTL`]) so every `ls` under the label — and `find` — shares
    /// that scan instead of re-listing per date. Invalidated on a mutation.
    date_index: tokio::sync::Mutex<HashMap<String, (Instant, DateIndex)>>,
    /// Global "message id → UTC date" cache. `internalDate` is immutable, so this
    /// never goes stale; it lets a label's index build skip re-fetching the date
    /// of a message already seen under another label (Gmail labels overlap — an
    /// INBOX message is also in a CATEGORY_*, UNREAD, IMPORTANT, …). Small (id +
    /// date ≈ 26 bytes/entry), bounded by the mailbox's unique message count.
    id_date: tokio::sync::Mutex<HashMap<String, String>>,
    /// Detects a sequential (grep-style) label scan to enable read-ahead only
    /// then — `(label, last access, consecutive count)`. A targeted browse
    /// (single month) stays at count 1 and never prefetches.
    seq_scan: tokio::sync::Mutex<Option<(String, Instant, u32)>>,
    /// Whole-attachment bytes, briefly cached ([`ATT_TTL`], [`ATT_CACHE_BUDGET`])
    /// so the chunked reads of one file don't each re-download it. Keyed by
    /// `(message id, filename)` — *not* the attachment id, which Gmail does not
    /// keep stable across `messages.get` calls; the message is immutable, so
    /// filename → content is.
    att_cache: tokio::sync::Mutex<AttCache>,
}

/// Attachment-cache key: `(message id, filename)`.
type AttKey = (String, String);
/// Cached attachment bytes plus their fetch time.
type AttEntry = (Instant, std::sync::Arc<Vec<u8>>);

/// See [`GmailResource::att_cache`].
#[derive(Default)]
struct AttCache {
    map: HashMap<AttKey, AttEntry>,
    total: u64,
}

impl AttCache {
    fn get(&self, key: &AttKey) -> Option<std::sync::Arc<Vec<u8>>> {
        self.map
            .get(key)
            .filter(|(at, _)| at.elapsed() < ATT_TTL)
            .map(|(_, v)| v.clone())
    }

    /// Insert, then bring the cache back under [`ATT_CACHE_BUDGET`]: drop
    /// expired entries first, then the oldest, never the one just inserted.
    fn put(&mut self, key: AttKey, bytes: std::sync::Arc<Vec<u8>>) {
        if let Some((_, old)) = self
            .map
            .insert(key.clone(), (Instant::now(), bytes.clone()))
        {
            self.total -= old.len() as u64;
        }
        self.total += bytes.len() as u64;
        while self.total > ATT_CACHE_BUDGET && self.map.len() > 1 {
            let victim = self
                .map
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            if let Some((_, v)) = self.map.remove(&victim) {
                self.total -= v.len() as u64;
            }
        }
    }
}

/// Full-message-JSON cache, bounded to [`MSG_CACHE_MAX`] entries (evict
/// oldest-first) so a large scan can't grow it without limit. `get` honors
/// [`MSG_TTL`]; entries are immutable message bodies, so the TTL only bounds
/// staleness after a mutation elsewhere.
#[derive(Default)]
struct MsgCache {
    map: HashMap<String, (Instant, Value)>,
}

impl MsgCache {
    fn get(&self, id: &str) -> Option<Value> {
        self.map
            .get(id)
            .filter(|(at, _)| at.elapsed() < MSG_TTL)
            .map(|(_, v)| v.clone())
    }

    fn put(&mut self, id: String, v: Value) {
        self.map.insert(id, (Instant::now(), v));
        while self.map.len() > MSG_CACHE_MAX {
            let victim = self
                .map
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            self.map.remove(&victim);
        }
    }

    /// Drop one id (a mutation — trash — made its cached body stale).
    fn remove(&mut self, id: &str) {
        self.map.remove(id);
    }
}

impl GmailResource {
    pub fn new(config: &GmailConfig) -> anyhow::Result<Self> {
        Ok(Self {
            accessor: GmailAccessor::new(config)?,
            label_cache: tokio::sync::Mutex::new(None),
            msg_cache: tokio::sync::Mutex::new(MsgCache::default()),
            date_index: tokio::sync::Mutex::new(HashMap::new()),
            id_date: tokio::sync::Mutex::new(HashMap::new()),
            seq_scan: tokio::sync::Mutex::new(None),
            att_cache: tokio::sync::Mutex::new(AttCache::default()),
        })
    }

    // ---- labels -----------------------------------------------------------

    /// `(display_name, id)` for every label. System labels display as their id
    /// (INBOX, SENT, …); user labels display as their name.
    async fn labels(&self) -> anyhow::Result<LabelList> {
        {
            let c = self.label_cache.lock().await;
            if let Some((at, v)) = c.as_ref()
                && at.elapsed() < LABEL_TTL
            {
                return Ok(v.clone());
            }
        }
        let raw = self.accessor.list_labels().await?;
        let mut out = Vec::new();
        for lb in &raw {
            let id = lb.get("id").and_then(|x| x.as_str()).unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let display = if lb.get("type").and_then(|t| t.as_str()) == Some("system") {
                id.to_string()
            } else {
                lb.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(id)
                    .to_string()
            };
            out.push((display, id.to_string()));
        }
        *self.label_cache.lock().await = Some((Instant::now(), out.clone()));
        Ok(out)
    }

    async fn label_id(&self, display: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .labels()
            .await?
            .into_iter()
            .find(|(d, _)| d == display)
            .map(|(_, id)| id))
    }

    // ---- message fetch (cached) ------------------------------------------

    async fn message_full(&self, id: &str) -> anyhow::Result<Value> {
        if let Some(v) = self.msg_cache.lock().await.get(id) {
            return Ok(v);
        }
        let v = self.accessor.get_message_full(id).await?;
        self.msg_cache.lock().await.put(id.to_string(), v.clone());
        Ok(v)
    }

    /// Whole bytes of one attachment, served from [`Self::att_cache`] while
    /// fresh — the ranged reads a guest `cat`/`grep` issues each re-enter here,
    /// and the API only serves whole attachments, so without the cache every
    /// chunk would re-download the file.
    async fn attachment_bytes(
        &self,
        msg_id: &str,
        attachment_id: &str,
        filename: &str,
    ) -> anyhow::Result<std::sync::Arc<Vec<u8>>> {
        let key = (msg_id.to_string(), filename.to_string());
        if let Some(bytes) = self.att_cache.lock().await.get(&key) {
            return Ok(bytes);
        }
        let bytes = std::sync::Arc::new(self.accessor.get_attachment(msg_id, attachment_id).await?);
        self.att_cache.lock().await.put(key, bytes.clone());
        Ok(bytes)
    }

    /// Fetch messages by id in one batch request (`format` = "full"/"minimal"),
    /// falling back to concurrent per-message fetches if the batch call fails so
    /// a listing still resolves. Order is not preserved; callers key on `id`.
    async fn fetch_many(&self, ids: &[String], format: &str) -> Vec<Value> {
        if ids.is_empty() {
            return Vec::new();
        }
        if let Ok(raws) = self.accessor.get_messages_batch(ids, format).await
            && !raws.is_empty()
        {
            return raws;
        }
        let minimal = format == "minimal";
        stream::iter(ids.iter().cloned())
            .map(|id| async move {
                if minimal {
                    self.accessor.get_message_minimal(&id).await.ok()
                } else {
                    self.accessor.get_message_full(&id).await.ok()
                }
            })
            .buffer_unordered(FETCH_CONCURRENCY)
            .filter_map(|x| async move { x })
            .collect()
            .await
    }

    /// Full messages for a listing, warming `msg_cache` so a following `cat` of a
    /// listed message reuses the body without another fetch.
    async fn fetch_full_many(&self, ids: &[String]) -> Vec<Value> {
        let raws = self.fetch_many(ids, "full").await;
        let mut cache = self.msg_cache.lock().await;
        for v in &raws {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                cache.put(id.to_string(), v.clone());
            }
        }
        raws
    }

    /// Full messages for `ids`, serving from `msg_cache` where fresh and fetching
    /// only the misses (in batches) — a repeat listing within [`MSG_TTL`] costs
    /// no re-fetch.
    async fn ensure_full(&self, ids: &[String]) -> Vec<Value> {
        let mut out = Vec::with_capacity(ids.len());
        let mut missing = Vec::new();
        {
            let cache = self.msg_cache.lock().await;
            for id in ids {
                match cache.get(id) {
                    Some(v) => out.push(v),
                    None => missing.push(id.clone()),
                }
            }
        }
        if !missing.is_empty() {
            out.extend(self.fetch_full_many(&missing).await);
        }
        out
    }

    /// Record a message-listing of `label` and report whether this looks like a
    /// sequential (grep-style) scan we should read ahead for — true while the
    /// same label keeps being listed within [`SEQ_WINDOW`] ([`SEQ_THRESHOLD`]th
    /// listing onward), so every month of a scan prefetches the next one. A
    /// single targeted browse stays at count 1 and never prefetches.
    async fn note_scan(&self, label: &str) -> bool {
        let now = Instant::now();
        let mut g = self.seq_scan.lock().await;
        let run = match g.as_ref() {
            Some((l, at, n)) if l == label && at.elapsed() < SEQ_WINDOW => n + 1,
            _ => 1,
        };
        *g = Some((label.to_string(), now, run));
        run >= SEQ_THRESHOLD
    }

    // ---- readdir levels ---------------------------------------------------

    async fn readdir_labels(&self) -> anyhow::Result<Vec<DirEntry>> {
        Ok(self
            .labels()
            .await?
            .into_iter()
            .map(|(display, _)| dir_entry(display))
            .collect())
    }

    /// The label's date→ids index, rebuilt when absent/stale: scan every message
    /// id (paginated), batch-fetch each internalDate (minimal), bucket by UTC
    /// date. Cached and shared by every `readdir` under the label (years, months,
    /// messages) so a listing scans the label once (and repeat/`find` reuse it).
    async fn index_for(&self, label: &str) -> anyhow::Result<DateIndex> {
        {
            let c = self.date_index.lock().await;
            if let Some((at, idx)) = c.get(label)
                && at.elapsed() < INDEX_TTL
            {
                return Ok(idx.clone());
            }
        }
        let Some(label_id) = self.label_id(label).await? else {
            anyhow::bail!("no such label: {label}");
        };
        let ids = self
            .accessor
            .list_all_message_ids(&label_id, MAX_INDEX_MESSAGES)
            .await?;
        let id_dates = self.dates_for_ids(&ids).await;
        // Bucket in list order (newest-first) so a date's ids are deterministic.
        let mut idx: DateIndex = BTreeMap::new();
        for id in &ids {
            if let Some(date) = id_dates.get(id) {
                idx.entry(date.clone()).or_default().push(id.clone());
            }
        }
        self.date_index
            .lock()
            .await
            .insert(label.to_string(), (Instant::now(), idx.clone()));
        Ok(idx)
    }

    /// Resolve each id's UTC date, reusing the global [`Self::id_date`] cache so a
    /// message already seen under another label isn't re-fetched — only ids
    /// missing from the cache cost a `messages.get` (minimal). `internalDate` is
    /// immutable, so cached entries never go stale.
    async fn dates_for_ids(&self, ids: &[String]) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::with_capacity(ids.len());
        let mut missing: Vec<String> = Vec::new();
        {
            let cache = self.id_date.lock().await;
            for id in ids {
                match cache.get(id) {
                    Some(date) => {
                        out.insert(id.clone(), date.clone());
                    }
                    None => missing.push(id.clone()),
                }
            }
        }
        if !missing.is_empty() {
            let fetched = self.fetch_many(&missing, "minimal").await;
            let mut cache = self.id_date.lock().await;
            for v in &fetched {
                if let Some(id) = v.get("id").and_then(|i| i.as_str())
                    && let Some(ms) = v.get("internalDate").and_then(|d| d.as_str())
                {
                    let date = epoch_ms_to_date(ms);
                    cache.insert(id.to_string(), date.clone());
                    out.insert(id.to_string(), date);
                }
            }
        }
        out
    }

    /// Year dirs present in a label — complete and newest-first, from the shared
    /// [`Self::index_for`] scan. Messages are bucketed `<label>/<yyyy>/<mm>/…` so
    /// each listing stays small (a flat per-day layout was hundreds of dirs).
    async fn readdir_years(&self, label: &str) -> anyhow::Result<Vec<DirEntry>> {
        let index = self.index_for(label).await?;
        // keys are `yyyy-mm-dd` sorted ascending, so equal years are adjacent.
        let mut years: Vec<&str> = index.keys().map(|d| &d[..4]).collect();
        years.dedup();
        Ok(years
            .into_iter()
            .rev()
            .map(|y| dir_entry(y.to_string()))
            .collect())
    }

    /// Month dirs (`mm`) present under `<label>/<yyyy>`, newest-first.
    async fn readdir_months(&self, label: &str, year: &str) -> anyhow::Result<Vec<DirEntry>> {
        if !is_valid_year(year) {
            anyhow::bail!("invalid year dir: {year}");
        }
        let index = self.index_for(label).await?;
        let prefix = format!("{year}-");
        let mut months: Vec<&str> = index
            .keys()
            .filter(|d| d.starts_with(&prefix))
            .map(|d| &d[5..7])
            .collect();
        months.dedup();
        Ok(months
            .into_iter()
            .rev()
            .map(|m| dir_entry(m.to_string()))
            .collect())
    }

    /// Messages (and per-message attachment dirs) within `<label>/<yyyy>/<mm>` —
    /// complete, served from the shared label index (no search, no cap).
    async fn readdir_messages(
        &self,
        label: &str,
        year: &str,
        month: &str,
    ) -> anyhow::Result<Vec<DirEntry>> {
        if !is_valid_year(year) || !is_valid_month(month) {
            anyhow::bail!("invalid month dir: {year}/{month}");
        }
        let index = self.index_for(label).await?;
        let prefix = format!("{year}-{month}-");
        let ids: Vec<String> = index
            .iter()
            .filter(|(d, _)| d.starts_with(&prefix))
            .flat_map(|(_, ids)| ids.iter().cloned())
            .collect();
        // Read-ahead: on a detected sequential (grep-style) scan, piggyback the
        // next month in traversal order (listings are newest-first, so the
        // adjacent older one) onto this month's batch — its chunks ride the
        // concurrency slots this month leaves idle, so each listing overlaps
        // the fetch of the following one. Bounded to one month: it can't blow
        // [`MSG_CACHE_MAX`] or outlive [`MSG_TTL`] before grep reads it, and
        // `ensure_full` skips ids the previous window already cached, so steady
        // state costs nothing extra. A single targeted browse never triggers
        // it — see [`Self::note_scan`].
        let mut fetch_ids = ids.clone();
        if self.note_scan(label).await {
            fetch_ids.extend(next_month_ids(&index, year, month));
            fetch_ids.truncate(MSG_CACHE_MAX);
        }
        let raws = self.ensure_full(&fetch_ids).await;
        // Only this month's messages are listed; the read-ahead tail stays
        // cache-only until its own readdir.
        let listed: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        for raw in &raws {
            let id = raw.get("id").and_then(|i| i.as_str()).unwrap_or("");
            if id.is_empty() || !listed.contains(id) {
                continue;
            }
            let subject = header(raw, "Subject");
            let subject = if subject.is_empty() {
                "No Subject".to_string()
            } else {
                subject
            };
            let size = raw.get("sizeEstimate").and_then(|s| s.as_u64());
            out.push(DirEntry {
                name: msg_filename(&subject, id),
                kind: FileKind::File,
                size: size.unwrap_or(0),
                mtime: None,
                atime: None,
                ctime: None,
                created: None,
                etag: None,
            });
            if !attachments(raw).is_empty() {
                out.push(dir_entry(attach_dir_name(&subject, id)));
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Attachment files within a message's attachment dir.
    async fn readdir_attachments(&self, msg_id: &str) -> anyhow::Result<Vec<DirEntry>> {
        let raw = self.message_full(msg_id).await?;
        let atts = attachments(&raw);
        let names = unique_attachment_names(&atts);
        Ok(atts
            .into_iter()
            .zip(names)
            .map(|(a, name)| DirEntry {
                name,
                kind: FileKind::File,
                size: a.size,
                mtime: None,
                atime: None,
                ctime: None,
                created: None,
                etag: None,
            })
            .collect())
    }
}

#[async_trait]
impl Resource for GmailResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<std::ops::Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        let seg = segments(path);
        let data = match seg.as_slice() {
            // <label>/<yyyy>/<mm>/<file>.gmail.json -> processed email JSON
            [_label, _year, _month, file] if file.ends_with(GMAIL_SUFFIX) => {
                let id = id_from_name(file.trim_end_matches(GMAIL_SUFFIX));
                let raw = self.message_full(&id).await?;
                serde_json::to_vec(&process_message(&raw))?
            }
            // <label>/<yyyy>/<mm>/<subject>__<id>/<filename> -> attachment bytes
            // (cached whole; only the requested range is copied out)
            [_label, _year, _month, dir, fname] => {
                let id = id_from_name(dir);
                let raw = self.message_full(&id).await?;
                let atts = attachments(&raw);
                let names = unique_attachment_names(&atts);
                let idx = names
                    .iter()
                    .position(|n| n == fname)
                    .ok_or(ResourceError::NotFound)?;
                let bytes = self
                    .attachment_bytes(&id, &atts[idx].attachment_id, &names[idx])
                    .await?;
                return Ok(slice_ref(&bytes, range));
            }
            _ => return Err(ResourceError::NotFound),
        };
        Ok(slice(data, range))
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        // Gmail is read-only for file writes; send/reply/forward go through the
        // `.cmd/` control path (see [`Self::command`]).
        Err(ResourceError::Unsupported)
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        let seg = segments(path);
        match seg.as_slice() {
            [] => self.readdir_labels().await.map_err(ResourceError::from),
            [label] => self.readdir_years(label).await.map_err(ResourceError::from),
            [label, year] => self
                .readdir_months(label, year)
                .await
                .map_err(ResourceError::from),
            [label, year, month] => self
                .readdir_messages(label, year, month)
                .await
                .map_err(ResourceError::from),
            // attachment dir: <label>/<yyyy>/<mm>/<subject>__<id>
            [_label, _year, _month, dir] if !dir.ends_with(GMAIL_SUFFIX) => self
                .readdir_attachments(&id_from_name(dir))
                .await
                .map_err(ResourceError::from),
            _ => Err(ResourceError::NotFound),
        }
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        let seg = segments(path);
        match seg.as_slice() {
            [] => Ok(dir_stat()),
            // a label: confirm it exists (cheap, cached) — else ENOENT
            [label] => {
                if self.label_id(label).await?.is_some() {
                    Ok(dir_stat())
                } else {
                    Err(ResourceError::NotFound)
                }
            }
            // a year dir: a well-formed yyyy is a (possibly empty) dir
            [_label, year] => {
                if is_valid_year(year) {
                    Ok(dir_stat())
                } else {
                    Err(ResourceError::NotFound)
                }
            }
            // a month dir: a well-formed mm is a (possibly empty) dir
            [_label, _year, month] => {
                if is_valid_month(month) {
                    Ok(dir_stat())
                } else {
                    Err(ResourceError::NotFound)
                }
            }
            // a message file (sentinel size — don't fetch just to size it) or
            // an attachment dir
            [_label, _year, _month, name] => {
                if name.ends_with(GMAIL_SUFFIX) {
                    Ok(FileStat {
                        kind: FileKind::File,
                        size: MSG_SENTINEL_SIZE,
                        ..Default::default()
                    })
                } else {
                    Ok(dir_stat())
                }
            }
            // an attachment file: exact size from the message's part metadata
            [_label, _year, _month, dir, fname] => {
                let raw = self.message_full(&id_from_name(dir)).await?;
                let atts = attachments(&raw);
                let names = unique_attachment_names(&atts);
                let idx = names
                    .iter()
                    .position(|n| n == fname)
                    .ok_or(ResourceError::NotFound)?;
                Ok(FileStat {
                    kind: FileKind::File,
                    size: atts[idx].size,
                    ..Default::default()
                })
            }
            _ => Err(ResourceError::NotFound),
        }
    }

    /// `rm <…>.gmail.json` moves the message to Trash. Dormant today: the
    /// mount is provisioned read-only (`gmail.readonly` — see the backend
    /// mount router), so Gmail rejects the trash call with 403. Kept so `rm`
    /// works unchanged once a write scope (`gmail.modify`) is granted at
    /// consent.
    async fn unlink(&self, path: &MountPath) -> ResourceResult<()> {
        let seg = segments(path);
        match seg.as_slice() {
            [_label, _year, _month, file] if file.ends_with(GMAIL_SUFFIX) => {
                let id = id_from_name(file.trim_end_matches(GMAIL_SUFFIX));
                self.accessor.trash(&id).await?;
                self.msg_cache.lock().await.remove(&id);
                self.id_date.lock().await.remove(&id);
                // The label indexes now list a trashed message; drop them so the
                // next `ls` rebuilds without it (and it appears under TRASH).
                self.date_index.lock().await.clear();
                Ok(())
            }
            _ => Err(ResourceError::Unsupported),
        }
    }

    /// Domain write commands (`send` / `reply` / `reply-all` / `forward`),
    /// designed to hang off the (not yet wired) `.cmd/` control path. Dormant
    /// today for the same reason as [`Self::unlink`] — the read-only
    /// `gmail.readonly` provisioning 403s them — but kept for the planned
    /// write-scoped flow rather than removed.
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
                let orig = self.message_full(&mid).await?;
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
                let raw_msg = self.message_full(&mid).await?;
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

    /// A listing is complete for the snapshot it was built from (the date index
    /// holds every date and every id for the label), but that index is cached
    /// with a TTL, so mail arriving before the TTL expires isn't in it yet.
    /// Keep negative caching off so a date/message absent from a listing is still
    /// probed against the adapter rather than answered `NotFound` from a possibly
    /// stale parent listing (e.g. `stat INBOX/2026-05-01` after `ls INBOX`).
    fn listings_complete(&self) -> bool {
        false
    }
}

// ---- path helpers ---------------------------------------------------------

/// Mount-relative path segments (`/INBOX/2026-05-03/x.gmail.json` -> 3).
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
fn id_from_name(name: &str) -> String {
    name.rsplit_once("__")
        .map(|(_, id)| id)
        .unwrap_or(name)
        .to_string()
}

fn msg_filename(subject: &str, id: &str) -> String {
    format!("{}__{id}{GMAIL_SUFFIX}", sanitize(subject))
}

fn attach_dir_name(subject: &str, id: &str) -> String {
    format!("{}__{id}", sanitize(subject))
}

const TITLE_MAX: usize = 80;

/// Sanitize a subject for use as a path segment:
/// keep word chars / spaces / `-._`, collapse the rest to `_`, spaces->`_`,
/// squeeze repeats, trim, cap length.
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
    if out.is_empty() {
        "No_Subject".to_string()
    } else {
        out
    }
}

// ---- message processing ---------------------------------------------------

struct Attach {
    filename: String,
    attachment_id: String,
    size: u64,
    mime_type: String,
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

fn attachments(raw: &Value) -> Vec<Attach> {
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
/// `readdir`, `stat`, `read`, and the `.gmail.json` listing all derive the same
/// unique name for a given part — and the `att_cache` key
/// (`(message id, name)`) becomes per-attachment too.
fn unique_attachment_names(atts: &[Attach]) -> Vec<String> {
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

/// Build the processed email JSON (the `.gmail.json` content).
fn process_message(raw: &Value) -> Value {
    let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
    let body_text = decode_body(&payload);
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
        "snippet": raw.get("snippet").and_then(|s| s.as_str()).unwrap_or(""),
        "labels": raw.get("labelIds").cloned().unwrap_or(json!([])),
        "attachments": atts,
    })
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
fn epoch_ms_to_date(ms: &str) -> String {
    let ms: i64 = ms.parse().unwrap_or(0);
    let days = (ms / 1000).div_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Whether `name` is a well-formed `yyyy` / `mm` path segment. Listings come from
/// the label index (not a date search), so these only validate the path shape —
/// a valid-but-empty year/month is a legitimately empty directory.
fn is_valid_year(name: &str) -> bool {
    name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit())
}

fn is_valid_month(name: &str) -> bool {
    name.len() == 2 && matches!(name.parse::<u32>(), Ok(m) if (1..=12).contains(&m))
}

/// Ids of the month directly older than `year-month` in `index` — the next dir
/// a newest-first scan will list, so the read-ahead window. Empty when there is
/// no older month. The month itself need not exist in the index (its ids are
/// then simply the adjacent older month's).
fn next_month_ids(index: &DateIndex, year: &str, month: &str) -> Vec<String> {
    let current = format!("{year}-{month}");
    // Keys are `yyyy-mm-dd` ascending; walk newest → oldest to the first month
    // before the current one.
    let next = index
        .keys()
        .rev()
        .map(|d| &d[..7])
        .find(|m| *m < current.as_str());
    let Some(next) = next else {
        return Vec::new();
    };
    let prefix = format!("{next}-");
    index
        .iter()
        .filter(|(d, _)| d.starts_with(&prefix))
        .flat_map(|(_, ids)| ids.iter().cloned())
        .collect()
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

fn dir_entry(name: String) -> DirEntry {
    DirEntry {
        name,
        kind: FileKind::Dir,
        size: 0,
        mtime: None,
        atime: None,
        ctime: None,
        created: None,
        etag: None,
    }
}

fn dir_stat() -> FileStat {
    FileStat {
        kind: FileKind::Dir,
        ..Default::default()
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

/// [`slice`] over borrowed bytes (cached attachments stay in the cache; only
/// the requested window is copied out).
fn slice_ref(data: &[u8], range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len());
            data[start..end].to_vec()
        }
        None => data.to_vec(),
    }
}

const GMAIL_PROMPT: &str = "\
Gmail (read + trash on delete). Layout:
  <label>/<yyyy>/<mm>/<subject>__<message-id>.gmail.json   # the email (JSON)
  <label>/<yyyy>/<mm>/<subject>__<message-id>/<filename>   # attachments (only if any)

  <label>       INBOX, SENT, DRAFT, IMPORTANT, STARRED, TRASH, SPAM, or a user label
  <yyyy>/<mm>   received year then month; `ls <label>` lists years, then months,
                then that month's messages (kept small per level)
  <subject>     sanitized subject (don't construct it; ls the month dir)
  <message-id>  Gmail message id (the field after the last `__`)

  cat <…>.gmail.json (keep the suffix) returns:
    {\"id\",\"thread_id\",\"from\":{\"name\",\"email\"},\"to\":[…],\"cc\":[…],
     \"subject\",\"date\",\"body_text\",\"snippet\",\"labels\":[…],
     \"attachments\":[{\"id\",\"filename\",\"mime_type\",\"size\"}]}
  The sibling dir (same name without .gmail.json) holds attachment bytes; cat a
  file inside to download it. ENOENT there means the message has no attachments.

  rm <…>.gmail.json    moves the message to Trash (only .gmail.json is removable).";

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::*;

    #[test]
    fn att_cache_serves_fresh_and_expires_stale() {
        let key = ("m1".to_string(), "a.pdf".to_string());
        let mut c = AttCache::default();
        c.put(key.clone(), std::sync::Arc::new(vec![1, 2, 3]));
        assert_eq!(c.get(&key).unwrap().as_slice(), &[1, 2, 3]);
        // Backdate the entry past the TTL: no longer served.
        if let Some(stale) = Instant::now().checked_sub(ATT_TTL + Duration::from_secs(1)) {
            c.map.get_mut(&key).unwrap().0 = stale;
            assert!(c.get(&key).is_none());
        }
        // Re-putting the same key replaces (total stays consistent).
        c.put(key.clone(), std::sync::Arc::new(vec![9; 5]));
        assert_eq!(c.total, 5);
    }

    #[test]
    fn att_cache_evicts_oldest_over_budget_but_not_newest() {
        let mut c = AttCache::default();
        let big = (ATT_CACHE_BUDGET / 2 + 1) as usize;
        let k1 = ("m1".to_string(), "a".to_string());
        let k2 = ("m2".to_string(), "b".to_string());
        c.put(k1.clone(), std::sync::Arc::new(vec![0; big]));
        // Make k1 strictly older so eviction order is deterministic.
        if let Some(older) = Instant::now().checked_sub(Duration::from_secs(1)) {
            c.map.get_mut(&k1).unwrap().0 = older;
        }
        // Two halves exceed the budget: the older k1 is evicted, k2 survives.
        c.put(k2.clone(), std::sync::Arc::new(vec![0; big]));
        assert!(c.get(&k1).is_none());
        assert!(c.get(&k2).is_some());
        assert_eq!(c.total, big as u64);
        // A single over-budget entry is kept (never evict the just-inserted).
        let mut solo = AttCache::default();
        let k = ("m".to_string(), "x".to_string());
        solo.put(
            k.clone(),
            std::sync::Arc::new(vec![0; (ATT_CACHE_BUDGET + 1) as usize]),
        );
        assert!(solo.get(&k).is_some());
    }

    #[test]
    fn slice_ref_windows_and_clamps() {
        let data = [1u8, 2, 3, 4, 5];
        assert_eq!(slice_ref(&data, None), vec![1, 2, 3, 4, 5]);
        assert_eq!(slice_ref(&data, Some(1..3)), vec![2, 3]);
        // Out-of-bounds range clamps instead of panicking.
        assert_eq!(slice_ref(&data, Some(3..99)), vec![4, 5]);
        assert!(slice_ref(&data, Some(9..12)).is_empty());
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
                "image.png",     // first occurrence kept verbatim
                "doc.pdf",
                "image (2).png", // suffix before the extension
                "image (3).png",
                "README",        // no extension
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

    #[test]
    fn epoch_ms_to_date_anchors() {
        assert_eq!(epoch_ms_to_date("0"), "1970-01-01");
        // 2026-05-03T10:00:00Z = 1777800000 s
        assert_eq!(epoch_ms_to_date("1777802400000"), "2026-05-03");
        assert_eq!(epoch_ms_to_date("bogus"), "1970-01-01");
    }

    #[test]
    fn year_and_month_validation() {
        assert!(is_valid_year("2026"));
        assert!(is_valid_year("1970"));
        assert!(!is_valid_year("26")); // wrong length
        assert!(!is_valid_year("202x")); // non-digit
        assert!(!is_valid_year("2026-05")); // not a bare year

        assert!(is_valid_month("01"));
        assert!(is_valid_month("12"));
        assert!(!is_valid_month("5")); // unpadded
        assert!(!is_valid_month("13")); // out of range
        assert!(!is_valid_month("00")); // month 0
        assert!(!is_valid_month("xx")); // non-digit
    }

    #[test]
    fn next_month_ids_walks_newest_to_oldest() {
        let mut idx: DateIndex = BTreeMap::new();
        idx.insert("2025-12-31".into(), vec!["d".into()]);
        idx.insert("2026-03-10".into(), vec!["a".into()]);
        idx.insert("2026-05-01".into(), vec!["b".into()]);
        idx.insert("2026-05-20".into(), vec!["c".into()]);
        // after 2026-05, the next older month with mail is 2026-03
        assert_eq!(next_month_ids(&idx, "2026", "05"), vec!["a"]);
        // after 2026-03 → 2025-12
        assert_eq!(next_month_ids(&idx, "2026", "03"), vec!["d"]);
        // oldest month → nothing to read ahead
        assert!(next_month_ids(&idx, "2025", "12").is_empty());
        // a month with no mail still finds the adjacent older one
        assert_eq!(next_month_ids(&idx, "2026", "04"), vec!["a"]);
        // a multi-day month returns all its ids (date order)
        assert_eq!(next_month_ids(&idx, "2026", "07"), vec!["b", "c"]);
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
