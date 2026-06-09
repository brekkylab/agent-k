use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
};

use agent_k::knowledge_base::{FileType, PdfEngine};
use uuid::Uuid;

use crate::state::AppState;

use super::dirent::{DirentScope, scope_root};

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
    if let Err(e) = resync_inner(&state, project_id).await {
        tracing::warn!(%project_id, "knowledge resync failed: {e}");
    }
}

async fn resync_inner(state: &Arc<AppState>, project_id: Uuid) -> Result<(), String> {
    let root = scope_root(state, &DirentScope::Shared { project_id }).join(KNOWLEDGE_PREFIX);

    let mut items: Vec<(Vec<u8>, FileType)> = Vec::new();
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
                let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
                items.push((bytes, filetype));
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
    Ok(())
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
