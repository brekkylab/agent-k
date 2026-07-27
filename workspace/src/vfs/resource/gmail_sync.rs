//! Full-mailbox mirror sync for the Gmail provider.
//!
//! Writes the mailbox to server disk as **real files in exactly the shape the
//! agent sees** — `<label>/<yyyy>/<mm>/<subject>__<id>.json` plus
//! decoded attachment bytes in the sibling dir — so after sync, `ls`/`grep`/
//! `cat`/`find` run at local-disk speed with no API round trips.
//!
//! Two phases:
//! 1. **List every id** (500/page — cheap and fast). This fixes the progress
//!    denominator up front: remaining = total − fetched.
//! 2. **Fetch newest-first** (the order `messages.list` returns) and write
//!    straight into the served tree, so mail is readable as it lands. Mid-sync
//!    the newest months are therefore complete and the oldest one in flight is
//!    partial; `state.json` says how far along it is.
//!
//! Layout under the mirror root (only `tree/` is ever served):
//! ```text
//! <root>/tree/…      # the served, agent-visible mirror
//! <root>/state.json  # { total, fetched, completed, history_id }
//! <root>/done.ids    # append-only log of fully written ids (resume skip-set)
//! ```
//!
//! A message carrying several labels is written once and **hard-linked** into
//! the other label trees — agents see ordinary files, storage holds one copy.
//!
//! After the initial full sync, [`sync_gmail_incremental`] keeps the mirror
//! fresh via `history.list` (Gmail's change journal, 2 quota units): the full
//! sync stores the newest message's `historyId` in `state.json`, and each
//! incremental run replays everything after that cursor — new messages are
//! fetched and written, deletions removed from every label dir, label changes
//! re-placed. If Gmail has expired the cursor (HTTP 404 — history is only
//! retained for a week or so), the run degrades to a full sync, which is a
//! complete reconcile in its own right: already-written ids skip their
//! (re-)fetch via the done log, an untruncated listing removes mirrored
//! messages the mailbox no longer contains, and a labelIds-only sweep over
//! the skipped ids re-places any whose labels changed while history was
//! unavailable.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vfs::accessor::{GmailAccessor, GmailConfig};

use super::gmail::{
    GMAIL_SUFFIX, attach_dir_name, attachments, epoch_ms_to_date, id_from_name, msg_filename,
    msg_time, process_message, unique_attachment_names,
};

const TREE_DIR: &str = "tree";
const STATE_FILE: &str = "state.json";
const DONE_LOG: &str = "done.ids";

/// Ids fetched per batch window, processed in list order (newest-first); the
/// batch call inside a window still runs its chunks concurrently.
const WINDOW: usize = 100;
/// Persist `state.json` every this many messages, so progress survives a
/// restart without a write per message.
const STATE_EVERY: usize = 25;

/// Sync progress, persisted to `state.json` after phase 1 and periodically
/// during phase 2. `completed` flips once the stream is exhausted.
/// `history_id` is the partial-sync cursor (the newest message's `historyId`
/// at full-sync time, advanced by every incremental run); absent on mirrors
/// written before partial sync existed — those full-sync once more to seed it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GmailSyncState {
    pub total: usize,
    pub fetched: usize,
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_id: Option<String>,
}

impl GmailSyncState {
    /// Read the persisted state under `root`, if any.
    pub fn load(root: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(root.join(STATE_FILE)).ok()?).ok()
    }

    fn save(&self, root: &Path) -> anyhow::Result<()> {
        let tmp = root.join(format!("{STATE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, root.join(STATE_FILE))?;
        Ok(())
    }
}

/// The served tree under a mirror root — what a resource should expose.
pub fn mirror_tree(root: &Path) -> PathBuf {
    root.join(TREE_DIR)
}

/// One account's mirror root under the deployment mirror dir: the sanitized
/// plain email names the directory (an identifier, not a secret — readable
/// for ops, and stable across re-consents unlike anything token-derived).
/// Both the sync worker and the serving resource derive paths through this.
pub fn account_mirror_dir(mirror_root: &Path, account_email: &str) -> PathBuf {
    let safe: String = account_email
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    mirror_root.join("gmail").join(safe)
}

/// Mirror the whole mailbox (bounded by [`GmailConfig::index_cap`] if set)
/// under `root`. Safe to re-run: already-written ids are skipped via the done
/// log, and a re-written message overwrites its own files. Returns the final
/// state (`completed == true` unless the id listing was empty-failed).
pub async fn sync_gmail_mirror(
    config: &GmailConfig,
    root: &Path,
) -> anyhow::Result<GmailSyncState> {
    let accessor = GmailAccessor::new(config)?;
    let writer = MirrorWriter::open(root)?;
    let labels = fetch_label_map(&accessor).await?;

    // Phase 1: the full (newest-first) id list — the progress denominator.
    let cap = config.index_cap.unwrap_or(usize::MAX);
    let ids = accessor.list_account_message_ids(cap).await?;
    let done = writer.load_done();
    let mut state = GmailSyncState {
        total: ids.len(),
        fetched: done.len().min(ids.len()),
        completed: false,
        history_id: None,
    };
    state.save(root)?;

    // Phase 2: fetch in list order and write straight into the served tree, so
    // mail is readable as it lands.
    let mut since_save = 0usize;
    for window in ids.chunks(WINDOW) {
        let need: Vec<String> = window
            .iter()
            .filter(|id| !done.contains(*id))
            .cloned()
            .collect();
        let mut by_id: HashMap<String, Value> = HashMap::new();
        if !need.is_empty() {
            for v in accessor.get_messages_batch(&need, "full").await? {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    by_id.insert(id.to_string(), v);
                }
            }
        }
        for id in window {
            if done.contains(id) {
                continue;
            }
            let Some(raw) = by_id.get(id.as_str()) else {
                // Vanished between list and get (trashed/deleted): count it so
                // progress still reaches total, but write nothing.
                state.fetched += 1;
                continue;
            };
            let (att_bytes, att_failed) = fetch_attachments(&accessor, id, raw).await;
            writer.place_message(&writer.tree, raw, &labels, &att_bytes)?;
            // A message whose attachment download failed is written (body +
            // whatever arrived) but NOT marked done, so the next run refetches
            // it and retries the missing attachment instead of never trying
            // again.
            if att_failed == 0 {
                writer.mark_done(id)?;
            }
            state.fetched += 1;
            since_save += 1;
            if since_save >= STATE_EVERY {
                state.save(root)?;
                since_save = 0;
            }
        }
    }

    // Reconcile the tree against the authoritative listing (one walk feeds
    // both passes). This is what makes the full sync a real fallback when the
    // history cursor has expired: the gap's changes must be derivable from
    // listing + disk alone.
    let live: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let on_disk = find_message_files(&writer.tree, None);

    // Deletions: anything mirrored that the mailbox no longer contains is a
    // leftover — remove it. Skipped when the listing was cap-truncated
    // (absence proves nothing then).
    if ids.len() < cap {
        for (id, paths) in &on_disk {
            if !live.contains(id.as_str()) {
                remove_message_files(&writer.tree, paths);
            }
        }
    }

    // Label drift: messages skipped above (already in the done log) kept the
    // placement of their *last* fetch. Re-check just their labelIds (the sync
    // guide's format=minimal refetch for cached messages), and re-place the
    // ones whose label dirs no longer match. First sync: `done` is empty, so
    // this costs nothing.
    let candidates: Vec<String> = on_disk
        .keys()
        .filter(|id| done.contains(*id) && live.contains(id.as_str()))
        .cloned()
        .collect();
    let mut drifted: Vec<String> = Vec::new();
    for window in candidates.chunks(WINDOW) {
        for v in accessor.get_messages_batch(window, "labels").await? {
            let Some(id) = v.get("id").and_then(|i| i.as_str()) else {
                continue;
            };
            let want: HashSet<String> = v
                .get("labelIds")
                .and_then(|l| l.as_array())
                .into_iter()
                .flatten()
                .filter_map(|l| labels.get(l.as_str()?).cloned())
                .collect();
            let have = labels_on_disk(&writer.tree, &on_disk[id]);
            if want != have {
                drifted.push(id.to_string());
            }
        }
    }
    if !drifted.is_empty() {
        tracing::info!(
            "gmail sync: re-placing {} label-drifted message(s)",
            drifted.len()
        );
    }
    for window in drifted.chunks(WINDOW) {
        for raw in accessor.get_messages_batch(window, "full").await? {
            let Some(id) = raw.get("id").and_then(|i| i.as_str()) else {
                continue;
            };
            if let Some(paths) = on_disk.get(id) {
                remove_message_files(&writer.tree, paths);
            }
            let (att_bytes, _) = fetch_attachments(&accessor, id, &raw).await;
            writer.place_message(&writer.tree, &raw, &labels, &att_bytes)?;
        }
    }

    // Seed the partial-sync cursor: the newest message's historyId (the sync
    // guide's pattern — anything that changed after it replays via
    // history.list, including changes that landed mid-sync).
    if let Some(newest) = ids.first() {
        match accessor.get_message_history_id(newest).await {
            Ok(h) => state.history_id = h,
            Err(e) => tracing::warn!("gmail sync: history cursor fetch failed: {e}"),
        }
    }

    state.completed = true;
    state.save(root)?;
    Ok(state)
}

/// What one incremental run applied to the mirror.
#[derive(Clone, Copy, Debug, Default)]
pub struct GmailSyncDelta {
    pub added: usize,
    pub deleted: usize,
    pub relabeled: usize,
    /// True when there was no usable cursor (first run, pre-cursor mirror, or
    /// Gmail expired it) and the run degraded to a full sync instead.
    pub full_resync: bool,
}

/// Bring a completed mirror up to date via `history.list`. Falls back to
/// [`sync_gmail_mirror`] when no cursor is available or Gmail returns 404 for
/// it. Idempotent: replaying a journal segment twice converges to the same
/// tree, so crashing between applying changes and saving the cursor is safe.
pub async fn sync_gmail_incremental(
    config: &GmailConfig,
    root: &Path,
) -> anyhow::Result<GmailSyncDelta> {
    let prior = GmailSyncState::load(root);
    let cursor = match prior
        .as_ref()
        .filter(|s| s.completed)
        .and_then(|s| s.history_id.clone())
    {
        Some(c) => c,
        None => {
            sync_gmail_mirror(config, root).await?;
            return Ok(GmailSyncDelta {
                full_resync: true,
                ..Default::default()
            });
        }
    };
    let accessor = GmailAccessor::new(config)?;

    // Page the journal, folding records into net per-message effects.
    let mut added = HashSet::new();
    let mut deleted = HashSet::new();
    let mut relabeled = HashSet::new();
    let mut latest = cursor.clone();
    let mut page_token: Option<String> = None;
    loop {
        let Some(page) = accessor
            .list_history(&cursor, page_token.as_deref())
            .await?
        else {
            tracing::info!("gmail sync: history cursor {cursor} expired; full resync");
            sync_gmail_mirror(config, root).await?;
            return Ok(GmailSyncDelta {
                full_resync: true,
                ..Default::default()
            });
        };
        if let Some(h) = page.get("historyId").and_then(|h| h.as_str()) {
            latest = h.to_string();
        }
        scan_history_page(&page, &mut added, &mut deleted, &mut relabeled);
        match page.get("nextPageToken").and_then(|t| t.as_str()) {
            Some(t) => page_token = Some(t.to_string()),
            None => break,
        }
    }

    // Net effects: a deletion wins over anything else (added-then-deleted
    // never touches disk); an addition subsumes its own label changes.
    relabeled.retain(|id| !deleted.contains(id) && !added.contains(id));
    added.retain(|id| !deleted.contains(id));

    let writer = MirrorWriter::open(root)?;
    let mut delta = GmailSyncDelta::default();

    // One tree walk locates everything we must remove or re-place.
    let of_interest: HashSet<String> = deleted.union(&relabeled).cloned().collect();
    let located = find_message_files(&writer.tree, Some(&of_interest));
    for id in &deleted {
        if let Some(paths) = located.get(id) {
            remove_message_files(&writer.tree, paths);
            delta.deleted += 1;
        }
    }

    // Fetch current state for new + relabeled messages and write them straight
    // into the served tree (its months are complete; a message that exists must
    // be visible). Ids the batch can't resolve vanished since the journal was
    // written — skipping them is the correct final state.
    if !added.is_empty() || !relabeled.is_empty() {
        let labels = fetch_label_map(&accessor).await?;
        let fetch_ids: Vec<String> = added.union(&relabeled).cloned().collect();
        for window in fetch_ids.chunks(WINDOW) {
            for raw in accessor.get_messages_batch(window, "full").await? {
                let Some(id) = raw.get("id").and_then(|i| i.as_str()).map(String::from) else {
                    continue;
                };
                if let Some(paths) = located.get(&id) {
                    // Old label placement is stale; the rewrite below is authoritative.
                    remove_message_files(&writer.tree, paths);
                }
                let (att_bytes, att_failed) = fetch_attachments(&accessor, &id, &raw).await;
                writer.place_message(&writer.tree, &raw, &labels, &att_bytes)?;
                if added.contains(&id) {
                    // Same retry rule as the full sync: an attachment failure
                    // leaves the id out of the done log, so the next full
                    // (fallback) sync refetches it. The journal itself won't
                    // replay it — the cursor moves past this record.
                    if att_failed == 0 {
                        writer.mark_done(&id)?;
                    }
                    delta.added += 1;
                } else {
                    delta.relabeled += 1;
                }
            }
        }
    }

    // Advance the cursor only after the tree reflects the journal.
    let mut state = prior.unwrap_or_default();
    state.total = state
        .total
        .saturating_add(delta.added)
        .saturating_sub(delta.deleted);
    state.fetched = state
        .fetched
        .saturating_add(delta.added)
        .saturating_sub(delta.deleted);
    state.completed = true;
    state.history_id = Some(latest);
    state.save(root)?;
    Ok(delta)
}

/// Fold one `history.list` page into the per-message effect sets.
fn scan_history_page(
    page: &Value,
    added: &mut HashSet<String>,
    deleted: &mut HashSet<String>,
    relabeled: &mut HashSet<String>,
) {
    fn collect(rec: &Value, key: &str, out: &mut HashSet<String>) {
        for e in rec
            .get(key)
            .and_then(|a| a.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(id) = e
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(|i| i.as_str())
            {
                out.insert(id.to_string());
            }
        }
    }
    for rec in page
        .get("history")
        .and_then(|h| h.as_array())
        .into_iter()
        .flatten()
    {
        collect(rec, "messagesAdded", added);
        collect(rec, "messagesDeleted", deleted);
        collect(rec, "labelsAdded", relabeled);
        collect(rec, "labelsRemoved", relabeled);
    }
}

/// Label id → display name (system labels display as their id, user labels as
/// their name — the rule that names the label dirs).
async fn fetch_label_map(accessor: &GmailAccessor) -> anyhow::Result<HashMap<String, String>> {
    Ok(accessor
        .list_labels()
        .await?
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
        .collect())
}

/// Download a message's attachments (decoded bytes). A failure skips the one
/// attachment (logged), not the message; the failure count lets callers
/// withhold the done mark so a later run retries the missing bytes.
async fn fetch_attachments(
    accessor: &GmailAccessor,
    id: &str,
    raw: &Value,
) -> (Vec<(String, Vec<u8>)>, usize) {
    let atts = attachments(raw);
    let names = unique_attachment_names(&atts);
    let mut out = Vec::with_capacity(atts.len());
    let mut failed = 0usize;
    for (a, name) in atts.iter().zip(&names) {
        match accessor.get_attachment(id, &a.attachment_id).await {
            Ok(bytes) => out.push((name.clone(), bytes)),
            Err(e) => {
                failed += 1;
                tracing::warn!("gmail sync: attachment {name} of {id}: {e} (will retry next run)");
            }
        }
    }
    (out, failed)
}

/// Walk `tree` and map message id → every path carrying it (its hardlinked
/// appearances across label dirs). `filter` restricts the map to ids of
/// interest; `None` maps everything.
///
/// A message file is identified by **position as well as suffix**: it sits
/// directly in a `<yyyy>/<mm>` dir. Attachment names are sender-controlled
/// (an arbitrary MIME `filename`, dots included), so a suffix-only test would
/// let an attachment called `x__<id>.json` pass as a message and be
/// deleted or re-placed along with the id it names. Month dirs hold only
/// message files and attachment dirs, so the walk stops descending there.
fn find_message_files(
    tree: &Path,
    filter: Option<&HashSet<String>>,
) -> HashMap<String, Vec<PathBuf>> {
    let mut out: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut stack = vec![tree.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let in_month = is_month_dir(&dir);
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                // Below a month there are only attachment dirs.
                if !in_month {
                    stack.push(p);
                }
            } else if in_month
                && let Some(stem) = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(GMAIL_SUFFIX))
            {
                let id = id_from_name(stem);
                if filter.is_none_or(|f| f.contains(&id)) {
                    out.entry(id).or_default().push(p);
                }
            }
        }
    }
    out
}

/// Is `dir` a `<yyyy>/<mm>` month dir — the one messages sit in directly?
fn is_month_dir(dir: &Path) -> bool {
    let digits = |name: Option<&std::ffi::OsStr>, len: usize| {
        name.and_then(|s| s.to_str())
            .is_some_and(|s| s.len() == len && s.bytes().all(|b| b.is_ascii_digit()))
    };
    digits(dir.file_name(), 2) && digits(dir.parent().and_then(Path::file_name), 4)
}

/// The label dirs a mirrored message currently occupies, read back from its
/// located paths: each is `<label…>/<yyyy>/<mm>/<file>` under `tree`, so the
/// label display name is everything above the last three components (nested
/// user labels span several). The counterpart of the placement rule in
/// [`MirrorWriter::place_message`], used to detect label drift.
fn labels_on_disk(tree: &Path, paths: &[PathBuf]) -> HashSet<String> {
    paths
        .iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(tree).ok()?;
            let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
            let label = parts.get(..parts.len().checked_sub(3)?)?;
            if label.is_empty() {
                None
            } else {
                Some(label.join("/"))
            }
        })
        .collect()
}

/// Remove a message's file and attachment dir at every located path, pruning
/// dirs this empties (never the tree root).
fn remove_message_files(tree: &Path, paths: &[PathBuf]) {
    for p in paths {
        let _ = std::fs::remove_file(p);
        if let Some(stem) = p
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(GMAIL_SUFFIX))
        {
            let _ = std::fs::remove_dir_all(p.with_file_name(stem));
        }
        if let Some(parent) = p.parent() {
            let _ = prune_empty(tree, parent);
        }
    }
}

/// `("2026", "07")` from the message's `internalDate`.
fn month_of(raw: &Value) -> Option<(String, String)> {
    let ms = raw.get("internalDate").and_then(|d| d.as_str())?;
    let date = epoch_ms_to_date(ms); // yyyy-mm-dd
    Some((date[..4].to_string(), date[5..7].to_string()))
}

/// Writes messages into the served tree. Synchronous std::fs on purpose:
/// everything is local disk, and keeping it sync makes it directly
/// unit-testable.
struct MirrorWriter {
    root: PathBuf,
    tree: PathBuf,
}

impl MirrorWriter {
    fn open(root: &Path) -> anyhow::Result<Self> {
        let tree = root.join(TREE_DIR);
        std::fs::create_dir_all(&tree)?;
        Ok(Self {
            root: root.to_path_buf(),
            tree,
        })
    }

    fn load_done(&self) -> HashSet<String> {
        std::fs::read_to_string(self.root.join(DONE_LOG))
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn mark_done(&self, id: &str) -> anyhow::Result<()> {
        let mut f = std::fs::File::options()
            .create(true)
            .append(true)
            .open(self.root.join(DONE_LOG))?;
        writeln!(f, "{id}")?;
        Ok(())
    }

    /// Write one message (processed JSON + decoded attachments) under every
    /// label it carries below `base`: real files for the first label, hard
    /// links for the rest. Files get the received time as mtime so
    /// `find -newermt` works. `base` is always the served tree today; it stays
    /// a parameter because both sync paths pass their own handle.
    fn place_message(
        &self,
        base: &Path,
        raw: &Value,
        labels: &HashMap<String, String>,
        att_bytes: &[(String, Vec<u8>)],
    ) -> anyhow::Result<()> {
        let id = raw
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or_else(|| anyhow::anyhow!("message without id"))?;
        let Some((y, m)) = month_of(raw) else {
            anyhow::bail!("message {id} without internalDate");
        };
        let processed = process_message(raw);
        let subject = {
            let s = processed
                .get("subject")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            if s.is_empty() { "No Subject" } else { s }.to_string()
        };
        let mtime = msg_time(raw);

        let label_dirs: Vec<PathBuf> = raw
            .get("labelIds")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| labels.get(l.as_str()?))
            .map(|display| base.join(display).join(&y).join(&m))
            .collect();
        if label_dirs.is_empty() {
            return Ok(()); // unlabeled (shouldn't happen); nothing to place
        }

        let fname = msg_filename(&subject, id);
        let body = serde_json::to_vec(&processed)?;

        let first = &label_dirs[0];
        std::fs::create_dir_all(first)?;
        write_file(&first.join(&fname), &body, mtime)?;
        if !att_bytes.is_empty() {
            let att_dir = first.join(attach_dir_name(&subject, id));
            std::fs::create_dir_all(&att_dir)?;
            for (name, bytes) in att_bytes {
                write_file(&att_dir.join(name), bytes, mtime)?;
            }
        }
        for dir in &label_dirs[1..] {
            std::fs::create_dir_all(dir)?;
            link_or_copy(&first.join(&fname), &dir.join(&fname))?;
            if !att_bytes.is_empty() {
                let src_dir = first.join(attach_dir_name(&subject, id));
                let dst_dir = dir.join(attach_dir_name(&subject, id));
                std::fs::create_dir_all(&dst_dir)?;
                for (name, _) in att_bytes {
                    link_or_copy(&src_dir.join(name), &dst_dir.join(name))?;
                }
            }
        }
        Ok(())
    }
}

fn write_file(
    path: &Path,
    bytes: &[u8],
    mtime: Option<std::time::SystemTime>,
) -> anyhow::Result<()> {
    std::fs::write(path, bytes)?;
    if let Some(t) = mtime {
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(t));
    }
    Ok(())
}

/// Hard-link `src` to `dst`, falling back to a copy on filesystems without
/// hard links. Overwrites a stale `dst` from an interrupted earlier run.
fn link_or_copy(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if dst.exists() {
        std::fs::remove_file(dst)?;
    }
    if std::fs::hard_link(src, dst).is_err() {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Remove now-empty dirs from `dir` up to (excluding) `root`.
fn prune_empty(root: &Path, dir: &Path) -> anyhow::Result<()> {
    let mut cur = dir.to_path_buf();
    while cur != *root && cur.starts_with(root) {
        match std::fs::remove_dir(&cur) {
            Ok(()) => {}
            Err(_) => break, // not empty (or gone) — stop
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw_msg(
        id: &str,
        subject: &str,
        epoch_ms: i64,
        labels: &[&str],
        att: Option<&str>,
    ) -> Value {
        let mut parts = vec![json!({
            "mimeType": "text/plain",
            "body": { "data": base64_url(format!("body of {subject}").as_bytes()) }
        })];
        if let Some(name) = att {
            parts.push(json!({
                "filename": name,
                "mimeType": "application/pdf",
                "body": { "attachmentId": format!("att-{id}"), "size": 5 }
            }));
        }
        json!({
            "id": id,
            "threadId": id,
            "labelIds": labels,
            "snippet": "s",
            "internalDate": epoch_ms.to_string(),
            "payload": {
                "headers": [{ "name": "Subject", "value": subject }],
                "parts": parts
            }
        })
    }

    fn base64_url(b: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    }

    fn labels_map() -> HashMap<String, String> {
        [("INBOX", "INBOX"), ("IMPORTANT", "IMPORTANT")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    // 2026-07 and 2026-06 anchors (epoch ms).
    const JUL: i64 = 1_784_000_000_000;
    const JUN: i64 = 1_781_000_000_000;

    #[test]
    fn writes_every_label_as_hardlinks_visible_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();

        // July message, two labels, one attachment.
        let m1 = raw_msg(
            "id1",
            "Hello July",
            JUL,
            &["INBOX", "IMPORTANT"],
            Some("a.pdf"),
        );
        w.place_message(
            &w.tree,
            &m1,
            &labels,
            &[("a.pdf".into(), b"PDF00".to_vec())],
        )
        .unwrap();
        let jul = month_of(&m1).unwrap();

        // Readable as soon as it's written — no staging step in between.
        let inbox = w
            .tree
            .join("INBOX")
            .join(&jul.0)
            .join(&jul.1)
            .join("Hello_July__id1.json");
        assert!(inbox.exists(), "visible right away");

        // Same file under both labels shares an inode (hard link).
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let a = std::fs::metadata(&inbox).unwrap();
            let b = std::fs::metadata(
                w.tree
                    .join("IMPORTANT")
                    .join(&jul.0)
                    .join(&jul.1)
                    .join("Hello_July__id1.json"),
            )
            .unwrap();
            assert_eq!(a.ino(), b.ino(), "labels share one inode");
        }

        // Attachment bytes came along, decoded, in the message's own dir.
        let att = w
            .tree
            .join("INBOX")
            .join(&jul.0)
            .join(&jul.1)
            .join("Hello_July__id1")
            .join("a.pdf");
        assert_eq!(std::fs::read(att).unwrap(), b"PDF00");

        // An older month lands in its own dir alongside, not replacing it.
        let m2 = raw_msg("id2", "June mail", JUN, &["INBOX"], None);
        let jun = month_of(&m2).unwrap();
        assert_ne!(jul, jun, "test anchors must span two months");
        w.place_message(&w.tree, &m2, &labels, &[]).unwrap();
        assert!(
            w.tree
                .join("INBOX")
                .join(&jun.0)
                .join(&jun.1)
                .join("June_mail__id2.json")
                .exists()
        );
        assert!(inbox.exists(), "July untouched by the June write");
    }

    #[test]
    fn done_log_and_state_survive_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        w.mark_done("a").unwrap();
        w.mark_done("b").unwrap();
        let done = MirrorWriter::open(tmp.path()).unwrap().load_done();
        assert!(done.contains("a") && done.contains("b") && done.len() == 2);

        let st = GmailSyncState {
            total: 10,
            fetched: 2,
            completed: false,
            history_id: Some("12345".into()),
        };
        st.save(tmp.path()).unwrap();
        let loaded = GmailSyncState::load(tmp.path()).unwrap();
        assert_eq!(loaded.total, 10);
        assert_eq!(loaded.fetched, 2);
        assert!(!loaded.completed);
        assert_eq!(loaded.history_id.as_deref(), Some("12345"));

        // Pre-cursor state files (no history_id) still load — cursor is None,
        // which routes the next incremental run through a full sync.
        std::fs::write(
            tmp.path().join("state.json"),
            br#"{"total": 3, "fetched": 3, "completed": true}"#,
        )
        .unwrap();
        let old = GmailSyncState::load(tmp.path()).unwrap();
        assert!(old.completed && old.history_id.is_none());
    }

    /// A resumed run adds to a month that already holds messages from the
    /// interrupted one, rather than replacing it.
    #[test]
    fn a_resumed_run_adds_to_an_existing_month() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();
        let ym = month_of(&raw_msg("x", "x", JUL, &["INBOX"], None)).unwrap();

        w.place_message(
            &w.tree,
            &raw_msg("id1", "First", JUL, &["INBOX"], None),
            &labels,
            &[],
        )
        .unwrap();
        // Second run, same month.
        let w = MirrorWriter::open(tmp.path()).unwrap();
        w.place_message(
            &w.tree,
            &raw_msg("id2", "Second", JUL, &["INBOX"], None),
            &labels,
            &[],
        )
        .unwrap();
        let month = tmp.path().join("tree/INBOX").join(&ym.0).join(&ym.1);
        assert!(month.join("First__id1.json").exists());
        assert!(month.join("Second__id2.json").exists());
    }

    #[test]
    fn history_pages_fold_into_net_effects() {
        let page = json!({
            "historyId": "9000",
            "history": [
                { "messagesAdded":   [ { "message": { "id": "new1" } }, { "message": { "id": "gone-fast" } } ] },
                { "labelsAdded":     [ { "message": { "id": "moved" }, "labelIds": ["IMPORTANT"] } ] },
                { "labelsRemoved":   [ { "message": { "id": "new1" }, "labelIds": ["UNREAD"] } ] },
                { "messagesDeleted": [ { "message": { "id": "gone-fast" } }, { "message": { "id": "old1" } } ] }
            ]
        });
        let (mut added, mut deleted, mut relabeled) =
            (HashSet::new(), HashSet::new(), HashSet::new());
        scan_history_page(&page, &mut added, &mut deleted, &mut relabeled);

        // Net rules applied by the sync: deletion wins; an addition subsumes
        // its own label churn.
        relabeled.retain(|id: &String| !deleted.contains(id) && !added.contains(id));
        added.retain(|id| !deleted.contains(id));
        assert_eq!(added, HashSet::from(["new1".to_string()]));
        assert_eq!(
            deleted,
            HashSet::from(["gone-fast".to_string(), "old1".to_string()])
        );
        assert_eq!(relabeled, HashSet::from(["moved".to_string()]));
    }

    #[test]
    fn incremental_removals_cover_every_label_and_prune() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();

        // Two messages in the served tree, one carrying two labels and an
        // attachment.
        let m1 = raw_msg("id1", "Doomed", JUL, &["INBOX", "IMPORTANT"], Some("a.pdf"));
        let m2 = raw_msg("id2", "Kept", JUL, &["INBOX"], None);
        w.place_message(
            &w.tree,
            &m1,
            &labels,
            &[("a.pdf".into(), b"PDF00".to_vec())],
        )
        .unwrap();
        w.place_message(&w.tree, &m2, &labels, &[]).unwrap();

        // The walk finds every hardlinked appearance of the filtered id.
        let filter = HashSet::from(["id1".to_string()]);
        let located = find_message_files(&w.tree, Some(&filter));
        assert_eq!(located.len(), 1);
        assert_eq!(located["id1"].len(), 2, "one path per label");

        remove_message_files(&w.tree, &located["id1"]);
        // File + attachment dir gone under both labels; IMPORTANT (now empty)
        // pruned away entirely; the other message untouched.
        assert!(find_message_files(&w.tree, Some(&filter)).is_empty());
        assert!(!w.tree.join("IMPORTANT").exists());
        let jul = month_of(&m2).unwrap();
        assert!(
            w.tree
                .join("INBOX")
                .join(&jul.0)
                .join(&jul.1)
                .join("Kept__id2.json")
                .exists()
        );
    }

    #[test]
    fn labels_on_disk_reads_back_placement_including_nested_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        // "Work/Receipts" is one Gmail label whose display name nests dirs.
        let labels: HashMap<String, String> = [("INBOX", "INBOX"), ("Label_7", "Work/Receipts")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();

        let m = raw_msg("id1", "Receipt", JUL, &["INBOX", "Label_7"], None);
        w.place_message(&w.tree, &m, &labels, &[]).unwrap();
        let located = find_message_files(&w.tree, None);
        assert_eq!(
            labels_on_disk(&w.tree, &located["id1"]),
            HashSet::from(["INBOX".to_string(), "Work/Receipts".to_string()])
        );

        // Drift repair converges: re-place with one label gone — the stale
        // appearance disappears, the survivor stays, and the readback agrees.
        let moved = raw_msg("id1", "Receipt", JUL, &["Label_7"], None);
        remove_message_files(&w.tree, &located["id1"]);
        w.place_message(&w.tree, &moved, &labels, &[]).unwrap();
        let relocated = find_message_files(&w.tree, None);
        assert_eq!(
            labels_on_disk(&w.tree, &relocated["id1"]),
            HashSet::from(["Work/Receipts".to_string()])
        );
        assert!(!w.tree.join("INBOX").exists(), "stale label pruned");
    }

    #[test]
    fn a_crafted_attachment_name_cannot_pass_as_a_message() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();

        // Attachment filenames come from the sender. This one is dressed up as
        // another message's file, one level below the month dir.
        let evil = "notice__victim9999.json";
        let m = raw_msg("real1", "Invoice", JUL, &["INBOX"], Some(evil));
        w.place_message(&w.tree, &m, &labels, &[(evil.into(), b"x".to_vec())])
            .unwrap();
        let jul = month_of(&m).unwrap();
        let planted = w
            .tree
            .join("INBOX")
            .join(&jul.0)
            .join(&jul.1)
            .join("Invoice__real1")
            .join(evil);
        assert!(planted.exists(), "the attachment was written as named");

        // Only the real message — sitting directly in <yyyy>/<mm> — is indexed.
        let found = find_message_files(&w.tree, None);
        assert_eq!(found.len(), 1);
        assert!(found.contains_key("real1"));
        assert!(
            !found.contains_key("victim9999"),
            "an attachment must not be indexed as a message"
        );

        // So deleting the real message takes its attachment with it (it lives
        // in the message's own dir) and nothing else is dragged along.
        remove_message_files(&w.tree, &found["real1"]);
        assert!(find_message_files(&w.tree, None).is_empty());
    }
}
