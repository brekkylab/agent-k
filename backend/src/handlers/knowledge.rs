use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
};

use agent_k::knowledge_base::{FileType, PdfEngine, Store};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    auth::AuthUser,
    error::{ApiResult, AppError},
    state::AppState,
};

use super::dirent::{DirentScope, scope_root};
use super::project::resolve_project_id;

/// Scope-relative prefix of the knowledge corpus folder under `shared/`.
pub(crate) const KNOWLEDGE_PREFIX: &str = "knowledge";

/// True if `scope` is `Shared` and `tail` is exactly the knowledge root
/// (the fixed folder itself, which must not be renamed or deleted).
pub(crate) fn is_knowledge_root(scope: &DirentScope, tail: &str) -> bool {
    matches!(scope, DirentScope::Shared { .. }) && tail.trim_matches('/') == KNOWLEDGE_PREFIX
}

/// True if `tail` (scope-relative path) is inside the knowledge folder.
pub(crate) fn tail_in_knowledge(tail: &str) -> bool {
    let t = tail.trim_matches('/');
    t == KNOWLEDGE_PREFIX || t.starts_with(&format!("{KNOWLEDGE_PREFIX}/"))
}

/// Spawn a background resync if a knowledge path was touched. Runs once per
/// request regardless of how many files changed.
pub(crate) fn maybe_trigger_resync(state: &Arc<AppState>, project_id: Uuid, touched: bool) {
    if !touched {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move { resync_knowledge(state, project_id).await });
}

/// Map a knowledge-folder filename to an indexable [`FileType`]. Accepts the
/// store's own types plus `.txt`/`.markdown` as markdown. Unsupported files are
/// left in the folder but not indexed.
fn indexable_filetype(name: &str) -> Option<FileType> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => Some(FileType::PDF),
        "md" | "markdown" | "txt" => Some(FileType::MD),
        "html" | "htm" => Some(FileType::HTML),
        _ => None,
    }
}

/// Rebuild the project's corpus index to match the current contents of its
/// `shared/knowledge/` folder: ingest new/changed files, purge documents whose
/// source is gone, then compact. Errors are logged, not propagated (background).
pub(crate) async fn resync_knowledge(state: Arc<AppState>, project_id: Uuid) {
    // Held for the whole resync; its Drop decrements the indexing count on any
    // exit path (return, error, or panic), so the UI never sticks on "indexing".
    let _guard = state.begin_indexing(project_id);
    // Serialize per project: `resync_inner` scans the folder to build its
    // desired-id set, so concurrent passes could let an older scan purge a
    // newer one's additions. The last-queued pass scans last and wins.
    let resync_lock = state.resync_lock_for(project_id);
    let _serialized = resync_lock.lock().await;
    match resync_inner(&state, project_id).await {
        Ok(()) => state.set_resync_error(project_id, None),
        Err(e) => {
            tracing::warn!(%project_id, "knowledge resync failed: {e}");
            state.set_resync_error(project_id, Some(e));
        }
    }
}

async fn resync_inner(state: &Arc<AppState>, project_id: Uuid) -> Result<(), String> {
    let scope_dir = scope_root(state, &DirentScope::Shared { project_id });
    let root = scope_dir.join(KNOWLEDGE_PREFIX);

    let mut items: Vec<(Vec<u8>, FileType)> = Vec::new();
    // Scope-relative path of each item, parallel to `items`, so a failed
    // ingest (reported by input index) can be mapped back to its file.
    let mut item_paths: Vec<String> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
            let ft = entry.file_type().await.map_err(|e| e.to_string())?;
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(filetype) = indexable_filetype(&name) {
                // Skip a file we can't read rather than aborting the whole
                // resync — otherwise one unreadable file (transient I/O, a race
                // with a concurrent delete) would block the purge of files that
                // really are gone, leaving stale entries in the corpus.
                match tokio::fs::read(&path).await {
                    Ok(bytes) => {
                        let rel = path
                            .strip_prefix(&scope_dir)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| name.clone());
                        items.push((bytes, filetype));
                        item_paths.push(rel);
                    }
                    Err(e) => tracing::warn!(%project_id, ?path, "skipping unreadable knowledge file: {e}"),
                }
            }
        }
    }

    let pdf_engine = project_pdf_engine(state, project_id).await;

    let store = state
        .store_for(project_id)
        .await
        .map_err(|(status, _)| format!("store_for failed: {status}"))?;
    let mut store = store.write().await;

    let result = store
        .ingest_many(items, pdf_engine)
        .await
        .map_err(|e| e.to_string())?;
    // Record per-file ingest failures (a bad PDF, a parse error) so the status
    // endpoint can mark those files failed instead of forever "pending".
    let mut failed_paths: HashSet<String> = HashSet::new();
    for f in &result.failed {
        let path = item_paths.get(f.index).cloned().unwrap_or_default();
        tracing::warn!(%project_id, %path, "knowledge file failed to index: {}", f.error);
        if !path.is_empty() {
            failed_paths.insert(path);
        }
    }
    state.set_failed_files(project_id, failed_paths);
    let desired: HashSet<Uuid> = result.succeeded.into_iter().collect();

    let stale: Vec<Uuid> = store
        .list(false, 0, u32::MAX)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|d| Uuid::parse_str(&d.id).ok())
        .filter(|id| !desired.contains(id))
        .collect();
    if !stale.is_empty() {
        store.purge_many(stale);
    }

    store.compact().map_err(|e| e.to_string())?;
    // Refresh the cached (title, line_count) summary so message-fetch citation
    // checks read it instead of loading every document's content (see #5).
    state.set_corpus_summary(project_id, corpus_summary(&store));
    Ok(())
}

/// `(title, line_count)` for every corpus document — the input
/// [`verify_citations`] reads. Computed off the message-fetch hot path.
pub(crate) fn corpus_summary(store: &Store) -> Vec<(String, usize)> {
    store
        .list(true, 0, u32::MAX)
        .map(|docs| {
            docs.into_iter()
                .map(|d| (d.title, d.content.as_deref().map(|c| c.lines().count()).unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default()
}

async fn project_pdf_engine(state: &Arc<AppState>, project_id: Uuid) -> PdfEngine {
    state
        .repository
        .get_project(project_id)
        .await
        .ok()
        .flatten()
        .and_then(|p| p.pdf_engine)
        .and_then(|s| PdfEngine::from_str_opt(&s))
        .unwrap_or_default()
}

/// Ensure the fixed `shared/knowledge/` folder exists for a project.
pub(crate) async fn ensure_knowledge_folder(state: &AppState, project_id: Uuid) {
    let dir: PathBuf = scope_root(state, &DirentScope::Shared { project_id }).join(KNOWLEDGE_PREFIX);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!(%project_id, "failed to create knowledge folder: {e}");
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KnowledgeStatusResponse {
    /// A background resync is in flight (files were just uploaded/changed).
    pub indexing: bool,
    /// Documents currently in the searchable corpus. `None` when the store is
    /// momentarily locked by an in-flight resync (the count is unknown right
    /// then; `indexing` will be true).
    pub document_count: Option<u32>,
    /// The last resync error, if the most recent background resync failed and
    /// none has succeeded since. Lets the UI surface a stuck/failed corpus.
    pub error: Option<String>,
}

/// GET /projects/{project_ref}/knowledge/status — membership-gated. Lets the
/// Files UI show "indexing..." until the background resync settles.
pub async fn knowledge_status(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
) -> ApiResult<Json<KnowledgeStatusResponse>> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }
    // Don't block on the store write lock: a resync holds it across PDF
    // parsing, and the status poll must stay responsive. If the lock is held,
    // report indexing with the count omitted rather than stalling the request.
    let store = state.store_for(project_id).await?;
    let document_count = store.try_read().ok().map(|s| s.count());
    Ok(Json(KnowledgeStatusResponse {
        indexing: state.is_indexing(project_id),
        document_count,
        error: state.resync_error(project_id),
    }))
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KnowledgeFileStatus {
    /// Scope-relative path under the knowledge folder, e.g. `knowledge/report.pdf`.
    pub path: String,
    /// True once this file's content is in the searchable corpus.
    pub indexed: bool,
    /// True if the latest resync tried and failed to index this file (a bad
    /// PDF, a parse error). Distinguishes a real failure from "still indexing".
    pub failed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KnowledgeFilesResponse {
    /// Indexable files in the knowledge folder, each with its corpus status.
    pub files: Vec<KnowledgeFileStatus>,
    /// A background resync is in flight; statuses may still be settling.
    pub indexing: bool,
}

/// GET /projects/{project_ref}/knowledge/files — membership-gated per-file
/// corpus status. Each file's id is the UUIDv5 of its bytes (as in `ingest`),
/// so a store lookup tells whether it is indexed — no filename-to-id table.
pub async fn knowledge_files(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path(project_ref): Path<String>,
) -> ApiResult<Json<KnowledgeFilesResponse>> {
    let project_id = resolve_project_id(&state, &project_ref).await?;
    let is_member = state
        .repository
        .user_in_project(auth_user.id, project_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    if !is_member {
        return Err(AppError::forbidden("not a member of this project"));
    }

    let scope_dir = scope_root(&state, &DirentScope::Shared { project_id });
    let root = scope_dir.join(KNOWLEDGE_PREFIX);
    let store = state.store_for(project_id).await?;

    // Resolve each file's content id without holding the store lock; membership
    // is checked afterwards under one non-blocking read.
    let mut entries: Vec<(String, Option<Uuid>)> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(AppError::internal(e.to_string())),
        };
        while let Some(entry) = rd.next_entry().await.map_err(|e| AppError::internal(e.to_string()))? {
            let ft = entry.file_type().await.map_err(|e| AppError::internal(e.to_string()))?;
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if indexable_filetype(&name).is_none() {
                continue;
            }
            // The content id is the UUIDv5 of the file bytes. Reuse the cached id
            // when (mtime, size) are unchanged so an unchanged file isn't re-read
            // and re-hashed on every poll; only a changed file pays that cost.
            let meta = entry.metadata().await.ok();
            let key = meta
                .as_ref()
                .and_then(|m| Some((m.modified().ok()?, m.len())));
            let id = match key.and_then(|(mt, sz)| state.cached_file_id(project_id, &path, mt, sz)) {
                Some(id) => Some(id),
                None => match tokio::fs::read(&path).await {
                    Ok(bytes) => {
                        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &bytes);
                        if let Some((mt, sz)) = key {
                            state.cache_file_id(project_id, &path, mt, sz, id);
                        }
                        Some(id)
                    }
                    Err(_) => None,
                },
            };
            let rel = path
                .strip_prefix(&scope_dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(name);
            entries.push((rel, id));
        }
    }

    // Don't block on the store write lock: a resync holds it across PDF parsing,
    // so report not-indexed if it's held (the banner shows "indexing"). The
    // guard spans only these synchronous lookups, never an await.
    let guard = store.try_read().ok();
    let mut files: Vec<KnowledgeFileStatus> = entries
        .into_iter()
        .map(|(path, id)| {
            let indexed = matches!((&guard, id), (Some(s), Some(id)) if s.get(id).is_some());
            // A file is "failed" only if it isn't indexed and the last resync
            // recorded it as failed — an indexed file is never failed.
            let failed = !indexed && state.file_failed(project_id, &path);
            KnowledgeFileStatus { path, indexed, failed }
        })
        .collect();
    drop(guard);
    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(KnowledgeFilesResponse {
        files,
        indexing: state.is_indexing(project_id),
    }))
}

/// One citation parsed from a Speedwagon answer, with whether it could be
/// matched back to the corpus.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CitationCheck {
    /// Footnote number, e.g. 1 for `[^1]`.
    pub index: u32,
    /// The footnote definition text after `[^N]:`.
    pub label: String,
    /// `corpus` (cites a document) or `web` (cites a URL).
    pub kind: String,
    /// For a corpus citation, whether the cited title matches a document in the
    /// store. A web citation is `true` when it carries a URL (not cross-checked).
    pub verified: bool,
}

/// Normalize a title for tolerant matching (case-fold, collapse whitespace).
fn norm_title(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Parse the line range out of a citation label, e.g. `... (lines 3-12)` or
/// `... (line 5)`, returning the inclusive `(start, end)` if present.
fn parse_line_range(label: &str) -> Option<(usize, usize)> {
    let open = label.rfind("(line")?;
    let inner = &label[open..];
    let inner = inner.trim_start_matches("(lines").trim_start_matches("(line");
    let inner = inner.trim_start().trim_end_matches(')');
    let digits: String = inner
        .chars()
        .map(|c| if c.is_ascii_digit() { c } else { ' ' })
        .collect();
    let mut nums = digits.split_whitespace().filter_map(|n| n.parse::<usize>().ok());
    let start = nums.next()?;
    let end = nums.next().unwrap_or(start);
    Some((start.min(end), start.max(end)))
}

/// Parse `[^N]: ...` footnote definitions from a Speedwagon answer and check
/// each against the corpus. A corpus citation is verified when its title matches
/// a known document AND, if it states a line range, that range lies within the
/// document's line count. A web citation (carrying a URL) is reported as
/// verified without an external lookup.
///
/// `docs` is `(title, line_count)` for each corpus document.
pub fn verify_citations(text: &str, docs: &[(String, usize)]) -> Vec<CitationCheck> {
    let by_title: std::collections::HashMap<String, usize> = docs
        .iter()
        .map(|(t, lines)| (norm_title(t), *lines))
        .collect();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("[^") else { continue };
        let Some((num, after)) = rest.split_once("]:") else { continue };
        let Ok(index) = num.trim().parse::<u32>() else { continue };
        let label = after.trim().to_string();
        let is_web = label.contains("http://") || label.contains("https://");
        let (kind, verified) = if is_web {
            ("web", true)
        } else {
            // Corpus citation: title is the text before " (lines" / " (line".
            let title = label
                .split(" (line")
                .next()
                .unwrap_or(&label)
                .trim_end_matches(['—', '-', ' ']);
            let verified = match by_title.get(&norm_title(title)) {
                // Title matches; if a line range is stated, it must fit the doc.
                Some(&line_count) => match parse_line_range(&label) {
                    Some((start, end)) => start >= 1 && end <= line_count.max(1),
                    None => true,
                },
                None => false,
            };
            ("corpus", verified)
        };
        out.push(CitationCheck {
            index,
            label,
            kind: kind.to_string(),
            verified,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_citations_matches_corpus_and_flags_unknown() {
        let docs = vec![("Team Handbook".to_string(), 3usize)];
        let text = "The mascot is Pibble.[^1]\n\n## Sources\n[^1]: Team Handbook (lines 1-3)\n[^2]: Nonexistent Report (lines 4-9)\n[^3]: Example — https://example.com";
        let checks = verify_citations(text, &docs);
        assert_eq!(checks.len(), 3);
        assert!(checks[0].verified && checks[0].kind == "corpus");
        assert!(!checks[1].verified, "unknown title must be unverified");
        assert!(checks[2].verified && checks[2].kind == "web");
    }

    #[test]
    fn verify_citations_rejects_out_of_range_lines() {
        let docs = vec![("Team Handbook".to_string(), 3usize)];
        // Title matches but the line range exceeds the document's 3 lines.
        let text = "x[^1]\n\n## Sources\n[^1]: Team Handbook (lines 40-50)";
        let checks = verify_citations(text, &docs);
        assert_eq!(checks.len(), 1);
        assert!(!checks[0].verified, "line range past the doc length must be unverified");

        // A matching title with no line range stays verified.
        let text2 = "x[^1]\n\n## Sources\n[^1]: Team Handbook";
        assert!(verify_citations(text2, &docs)[0].verified);
    }
}
