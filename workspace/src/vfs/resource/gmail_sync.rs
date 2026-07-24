//! Full-mailbox mirror sync for the Gmail provider.
//!
//! Writes the mailbox to server disk as **real files in exactly the shape the
//! agent sees** — `<label>/<yyyy>/<mm>/<subject>__<id>.gmail.json` plus
//! decoded attachment bytes in the sibling dir — so after sync, `ls`/`grep`/
//! `cat`/`find` run at local-disk speed with no API round trips.
//!
//! Two phases:
//! 1. **List every id** (500/page — cheap and fast). This fixes the progress
//!    denominator up front: remaining = total − fetched.
//! 2. **Fetch newest-first** (the order `messages.list` returns) and write
//!    into a staging area. Because the stream is ordered by received time,
//!    the moment it crosses into an older month, the previous month can no
//!    longer gain members — that month's dirs are then renamed into the
//!    served tree **atomically, for every label at once**. Visible months are
//!    therefore always complete: a grep over what's visible never lies about
//!    what's visible.
//!
//! Layout under the mirror root (only `tree/` is ever served):
//! ```text
//! <root>/tree/…      # the served, agent-visible mirror (complete months only)
//! <root>/work/…      # staging: months still accumulating
//! <root>/state.json  # { total, fetched, completed } — progress + resume
//! <root>/done.ids    # append-only log of fully written ids (resume skip-set)
//! ```
//!
//! A message carrying several labels is written once and **hard-linked** into
//! the other label trees — agents see ordinary files, storage holds one copy.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vfs::accessor::{GmailAccessor, GmailConfig};

use super::gmail::{
    attach_dir_name, attachments, epoch_ms_to_date, msg_filename, msg_time, process_message,
    unique_attachment_names,
};

const TREE_DIR: &str = "tree";
const WORK_DIR: &str = "work";
const STATE_FILE: &str = "state.json";
const DONE_LOG: &str = "done.ids";

/// Ids fetched per batch window. Windows are processed strictly in list order
/// (newest-first) so month boundaries are detected correctly; the batch call
/// inside a window still runs its chunks concurrently.
const WINDOW: usize = 100;
/// Persist `state.json` every this many messages, so progress survives a
/// restart without a write per message.
const STATE_EVERY: usize = 25;

/// Sync progress, persisted to `state.json` after phase 1 and periodically
/// during phase 2. `completed` flips only after the final month is promoted.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GmailSyncState {
    pub total: usize,
    pub fetched: usize,
    pub completed: bool,
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

/// Mirror the whole mailbox (bounded by [`GmailConfig::index_cap`] if set)
/// under `root`. Safe to re-run: already-written ids are skipped via the done
/// log, and months already promoted merge instead of clobbering. Returns the
/// final state (`completed == true` unless the id listing was empty-failed).
pub async fn sync_gmail_mirror(config: &GmailConfig, root: &Path) -> anyhow::Result<GmailSyncState> {
    let accessor = GmailAccessor::new(config)?;
    let writer = MirrorWriter::open(root)?;

    // Label id → display name, once (the same display rule the tree used:
    // system labels show their id, user labels their name).
    let labels: HashMap<String, String> = accessor
        .list_labels()
        .await?
        .iter()
        .filter_map(|lb| {
            let id = lb.get("id").and_then(|x| x.as_str())?;
            let display = if lb.get("type").and_then(|t| t.as_str()) == Some("system") {
                id.to_string()
            } else {
                lb.get("name").and_then(|n| n.as_str()).unwrap_or(id).to_string()
            };
            Some((id.to_string(), display))
        })
        .collect();

    // Phase 1: the full (newest-first) id list — the progress denominator.
    let cap = config.index_cap.unwrap_or(usize::MAX);
    let ids = accessor.list_account_message_ids(cap).await?;
    let done = writer.load_done();
    let mut state = GmailSyncState {
        total: ids.len(),
        fetched: done.len().min(ids.len()),
        completed: false,
    };
    state.save(root)?;

    // Phase 2: fetch in list order; promote a month once the stream passes it.
    let mut current_month: Option<(String, String)> = None;
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
            let month = month_of(raw);
            if let (Some(prev), Some(cur)) = (&current_month, &month)
                && prev != cur
            {
                writer.promote_month(prev)?;
            }
            if month.is_some() {
                current_month = month;
            }

            // Attachments: download the decoded bytes now; a failure skips the
            // one attachment (logged), not the message.
            let atts = attachments(raw);
            let names = unique_attachment_names(&atts);
            let mut att_bytes: Vec<(String, Vec<u8>)> = Vec::with_capacity(atts.len());
            for (a, name) in atts.iter().zip(&names) {
                match accessor.get_attachment(id, &a.attachment_id).await {
                    Ok(bytes) => att_bytes.push((name.clone(), bytes)),
                    Err(e) => tracing::warn!("gmail sync: attachment {name} of {id}: {e}"),
                }
            }
            writer.write_message(raw, &labels, &att_bytes)?;
            writer.mark_done(id)?;
            state.fetched += 1;
            since_save += 1;
            if since_save >= STATE_EVERY {
                state.save(root)?;
                since_save = 0;
            }
        }
    }

    // Promote whatever is still staged (the oldest month, and on resume any
    // months whose completion we only know now the stream is exhausted).
    writer.promote_all()?;
    state.completed = true;
    state.save(root)?;
    Ok(state)
}

/// `("2026", "07")` from the message's `internalDate`.
fn month_of(raw: &Value) -> Option<(String, String)> {
    let ms = raw.get("internalDate").and_then(|d| d.as_str())?;
    let date = epoch_ms_to_date(ms); // yyyy-mm-dd
    Some((date[..4].to_string(), date[5..7].to_string()))
}

/// Writes messages into `work/` and promotes complete months into `tree/`.
/// Synchronous std::fs on purpose: everything is local disk, and keeping it
/// sync makes it directly unit-testable.
struct MirrorWriter {
    root: PathBuf,
    tree: PathBuf,
    work: PathBuf,
}

impl MirrorWriter {
    fn open(root: &Path) -> anyhow::Result<Self> {
        let tree = root.join(TREE_DIR);
        let work = root.join(WORK_DIR);
        std::fs::create_dir_all(&tree)?;
        std::fs::create_dir_all(&work)?;
        Ok(Self {
            root: root.to_path_buf(),
            tree,
            work,
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
    /// label it carries: real files for the first label, hard links for the
    /// rest. Files get the received time as mtime so `find -newermt` works.
    fn write_message(
        &self,
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
            let s = processed.get("subject").and_then(|s| s.as_str()).unwrap_or("");
            if s.is_empty() { "No Subject" } else { s }.to_string()
        };
        let mtime = msg_time(raw);

        let label_dirs: Vec<PathBuf> = raw
            .get("labelIds")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| labels.get(l.as_str()?))
            .map(|display| self.work.join(display).join(&y).join(&m))
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

    /// Move every label's staged `<yyyy>/<mm>` into the served tree. A month
    /// already present there (a previous partial run) merges entry-by-entry
    /// instead of failing the rename.
    fn promote_month(&self, (y, m): &(String, String)) -> anyhow::Result<()> {
        for label_rel in staged_label_dirs(&self.work, &self.work)? {
            let src = self.work.join(&label_rel).join(y).join(m);
            if !src.exists() {
                continue;
            }
            let dst = self.tree.join(&label_rel).join(y).join(m);
            move_dir(&src, &dst)?;
            prune_empty(&self.work, &self.work.join(&label_rel).join(y))?;
        }
        Ok(())
    }

    /// Promote everything still staged (end of stream / resume sweep).
    fn promote_all(&self) -> anyhow::Result<()> {
        for label_rel in staged_label_dirs(&self.work, &self.work)? {
            let label_dir = self.work.join(&label_rel);
            for y in dir_names(&label_dir)? {
                for m in dir_names(&label_dir.join(&y))? {
                    move_dir(
                        &label_dir.join(&y).join(&m),
                        &self.tree.join(&label_rel).join(&y).join(&m),
                    )?;
                }
                prune_empty(&self.work, &label_dir.join(&y))?;
            }
            prune_empty(&self.work, &label_dir)?;
        }
        Ok(())
    }
}

fn write_file(path: &Path, bytes: &[u8], mtime: Option<std::time::SystemTime>) -> anyhow::Result<()> {
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

/// Relative paths of the label dirs under `work` — i.e. every directory chain
/// down to (but excluding) a `yyyy` component. Label display names may contain
/// `/` (Gmail's nested labels), so labels can be nested dirs; a dir whose name
/// parses as a 4-digit year terminates the label path.
fn staged_label_dirs(work_root: &Path, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for name in dir_names(dir)? {
        let is_year = name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit());
        if is_year {
            // `dir` itself is a label dir.
            let rel = dir.strip_prefix(work_root).unwrap_or(dir).to_path_buf();
            if !out.contains(&rel) {
                out.push(rel);
            }
        } else {
            out.extend(staged_label_dirs(work_root, &dir.join(&name))?);
        }
    }
    Ok(out)
}

fn dir_names(dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Move a directory into place; if the destination already exists (a month
/// promoted by an earlier run), merge the source's entries into it.
fn move_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !dst.exists() {
        std::fs::rename(src, dst)?;
        return Ok(());
    }
    for e in std::fs::read_dir(src)?.flatten() {
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            move_dir(&e.path(), &to)?;
        } else {
            if to.exists() {
                std::fs::remove_file(&to)?;
            }
            std::fs::rename(e.path(), &to)?;
        }
    }
    std::fs::remove_dir(src)?;
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

    fn raw_msg(id: &str, subject: &str, epoch_ms: i64, labels: &[&str], att: Option<&str>) -> Value {
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
    fn writes_hardlinks_and_promotes_months_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();

        // Newest message (July, two labels, one attachment) staged in work/.
        let m1 = raw_msg("id1", "Hello July", JUL, &["INBOX", "IMPORTANT"], Some("a.pdf"));
        w.write_message(&m1, &labels, &[("a.pdf".into(), b"PDF00".to_vec())])
            .unwrap();
        let jul = month_of(&m1).unwrap();
        let staged = tmp
            .path()
            .join("work/INBOX")
            .join(&jul.0)
            .join(&jul.1)
            .join("Hello_July__id1.gmail.json");
        assert!(staged.exists(), "staged in work/");
        assert!(!tmp.path().join("tree/INBOX").exists(), "not yet visible");

        // Same file under both labels shares an inode (hard link).
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let a = std::fs::metadata(&staged).unwrap();
            let b = std::fs::metadata(
                tmp.path()
                    .join("work/IMPORTANT")
                    .join(&jul.0)
                    .join(&jul.1)
                    .join("Hello_July__id1.gmail.json"),
            )
            .unwrap();
            assert_eq!(a.ino(), b.ino(), "labels share one inode");
        }

        // Stream crosses into June → July promotes for BOTH labels at once.
        let m2 = raw_msg("id2", "June mail", JUN, &["INBOX"], None);
        let jun = month_of(&m2).unwrap();
        assert_ne!(jul, jun, "test anchors must span two months");
        w.promote_month(&jul).unwrap();
        for label in ["INBOX", "IMPORTANT"] {
            let vis = tmp
                .path()
                .join("tree")
                .join(label)
                .join(&jul.0)
                .join(&jul.1)
                .join("Hello_July__id1.gmail.json");
            assert!(vis.exists(), "{label} July visible after promote");
        }
        // Attachment bytes came along, decoded.
        let att = tmp
            .path()
            .join("tree/INBOX")
            .join(&jul.0)
            .join(&jul.1)
            .join("Hello_July__id1")
            .join("a.pdf");
        assert_eq!(std::fs::read(att).unwrap(), b"PDF00");

        // June stays staged until promote_all.
        w.write_message(&m2, &labels, &[]).unwrap();
        assert!(!tmp.path().join("tree/INBOX").join(&jun.0).join(&jun.1).exists());
        w.promote_all().unwrap();
        assert!(
            tmp.path()
                .join("tree/INBOX")
                .join(&jun.0)
                .join(&jun.1)
                .join("June_mail__id2.gmail.json")
                .exists()
        );
        // work/ fully drained.
        assert!(dir_names(&tmp.path().join("work")).unwrap().is_empty());
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
        };
        st.save(tmp.path()).unwrap();
        let loaded = GmailSyncState::load(tmp.path()).unwrap();
        assert_eq!(loaded.total, 10);
        assert_eq!(loaded.fetched, 2);
        assert!(!loaded.completed);
    }

    #[test]
    fn promoting_over_an_existing_month_merges() {
        let tmp = tempfile::tempdir().unwrap();
        let w = MirrorWriter::open(tmp.path()).unwrap();
        let labels = labels_map();
        let ym = month_of(&raw_msg("x", "x", JUL, &["INBOX"], None)).unwrap();

        // Run 1 promoted one July message.
        w.write_message(&raw_msg("id1", "First", JUL, &["INBOX"], None), &labels, &[])
            .unwrap();
        w.promote_month(&ym).unwrap();
        // Run 2 (resume) stages another July message; promote must merge.
        w.write_message(&raw_msg("id2", "Second", JUL, &["INBOX"], None), &labels, &[])
            .unwrap();
        w.promote_all().unwrap();
        let month = tmp.path().join("tree/INBOX").join(&ym.0).join(&ym.1);
        assert!(month.join("First__id1.gmail.json").exists());
        assert!(month.join("Second__id2.gmail.json").exists());
    }
}
