use std::path::PathBuf;
use std::sync::Arc;

use uuid::Uuid;

use crate::models::{Speedwagon, SpeedwagonIndexStatus};
use crate::repository::Repository;
use crate::state::AppState;

/// Collect (filename, file_path) for each source_id from the repository.
pub async fn collect_speedwagon_sources(
    repository: &dyn Repository,
    source_ids: &[Uuid],
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut result = Vec::new();
    for &sid in source_ids {
        let source = repository
            .get_source(sid)
            .await
            .map_err(|e| format!("DB error fetching source {sid}: {e}"))?
            .ok_or_else(|| format!("source {sid} not found"))?;
        let path_str = source
            .file_path
            .ok_or_else(|| format!("source {sid} has no file_path"))?;
        result.push((source.name, PathBuf::from(path_str)));
    }
    Ok(result)
}

/// Validate file count and size limits.
fn validate_file_limits(sources: &[(String, PathBuf)]) -> Result<(), String> {
    if sources.len() > 200 {
        return Err(format!("too many files: {} (max 200)", sources.len()));
    }
    for (name, path) in sources {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > 100 * 1024 * 1024 => {
                return Err(format!("file '{name}' exceeds 100MB limit"));
            }
            Err(e) => {
                return Err(format!("cannot stat file '{name}': {e}"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Create corpus directory with symlinks to source files.
fn prepare_corpus_dir(
    sw_dir: &std::path::Path,
    sources: &[(String, PathBuf)],
) -> Result<(PathBuf, PathBuf), String> {
    let corpus_dir = sw_dir.join("corpus");
    let index_tmp_dir = sw_dir.join("index_tmp");

    // Create corpus dir (clean slate)
    if corpus_dir.exists() {
        std::fs::remove_dir_all(&corpus_dir)
            .map_err(|e| format!("failed to clean corpus dir: {e}"))?;
    }
    std::fs::create_dir_all(&corpus_dir)
        .map_err(|e| format!("failed to create corpus dir: {e}"))?;

    // Create symlinks in corpus_dir pointing to each source file
    for (name, src_path) in sources {
        let link_path = corpus_dir.join(name);
        let abs_src = std::fs::canonicalize(src_path).unwrap_or_else(|_| src_path.clone());
        std::os::unix::fs::symlink(&abs_src, &link_path)
            .map_err(|e| format!("failed to create symlink for '{name}': {e}"))?;
    }

    // Clean up any leftover index_tmp
    if index_tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&index_tmp_dir);
    }

    Ok((corpus_dir, index_tmp_dir))
}

/// Finalize indexing: swap index_tmp -> index, update DB status, invalidate caches.
async fn finalize_indexing(
    repository: &Arc<dyn Repository>,
    state: &actix_web::web::Data<AppState>,
    id: Uuid,
    index_tmp_dir: &std::path::Path,
    index_dir: &std::path::Path,
    corpus_dir: &std::path::Path,
) -> Result<(), String> {
    // Atomic swap: index_tmp -> index
    if index_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(index_dir) {
            eprintln!("[indexing] failed to remove old index: {e}");
        }
    }
    std::fs::rename(index_tmp_dir, index_dir)
        .map_err(|e| format!("failed to swap index_tmp to index: {e}"))?;

    let indexed_at = chrono::Utc::now();
    let index_dir_str = index_dir.to_string_lossy().to_string();
    let corpus_dir_str = corpus_dir.to_string_lossy().to_string();
    let _ = repository
        .update_speedwagon_index_status(
            id,
            SpeedwagonIndexStatus::Indexed,
            None,
            Some(index_dir_str),
            Some(corpus_dir_str),
            None,
            Some(indexed_at),
        )
        .await;

    // Invalidate runtime cache for sessions using this speedwagon
    if let Ok(session_ids) = repository.get_sessions_by_speedwagon_id(id).await {
        for session_id in session_ids {
            state.invalidate_session_runtime(session_id);
        }
    }
    eprintln!("[indexing] speedwagon {id} indexed successfully");
    Ok(())
}

/// Main indexing orchestration — runs inside a spawned background task.
pub async fn start_indexing(
    repository: Arc<dyn Repository>,
    state: actix_web::web::Data<AppState>,
    speedwagon_data_dir: PathBuf,
    sw: Speedwagon,
) -> Result<(), String> {
    let id = sw.id;

    // Collect source file paths from DB
    let sources = match collect_speedwagon_sources(&*repository, &sw.source_ids).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[indexing] failed to collect sources for speedwagon {id}: {e}");
            let _ = repository
                .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(e.clone()), None, None, None, None)
                .await;
            return Err(e);
        }
    };

    // File limit validation
    if let Err(msg) = validate_file_limits(&sources) {
        let _ = repository
            .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(msg.clone()), None, None, None, None)
            .await;
        return Err(msg);
    }

    let sw_dir = speedwagon_data_dir.join(id.to_string());
    let index_dir = sw_dir.join("index");

    // Prepare corpus dir and clean up leftover index_tmp
    let (corpus_dir, index_tmp_dir) = match prepare_corpus_dir(&sw_dir, &sources) {
        Ok(dirs) => dirs,
        Err(msg) => {
            let _ = repository
                .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(msg.clone()), None, None, None, None)
                .await;
            return Err(msg);
        }
    };

    // Run knowledge_agent::build_index_multi in spawn_blocking (it's a synchronous CPU-intensive operation)
    let corpus_dirs = vec![corpus_dir.clone()];
    let index_tmp = index_tmp_dir.clone();
    let index_result = tokio::task::spawn_blocking(move || {
        knowledge_agent::build_index_multi(
            &index_tmp,
            &corpus_dirs,
            &knowledge_agent::IndexSettings { schema_version: 1, no_merge: false },
            true,
        )
    })
    .await;

    match index_result {
        Ok(Ok(_report)) => {
            if let Err(msg) = finalize_indexing(&repository, &state, id, &index_tmp_dir, &index_dir, &corpus_dir).await {
                let _ = repository
                    .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(msg.clone()), None, None, None, None)
                    .await;
                return Err(msg);
            }
            Ok(())
        }
        Ok(Err(e)) => {
            // Indexing failed -- clean up index_tmp, preserve existing index
            if index_tmp_dir.exists() {
                let _ = std::fs::remove_dir_all(&index_tmp_dir);
            }
            let msg = format!("indexing failed: {e}");
            eprintln!("[indexing] speedwagon {id} error: {msg}");
            let _ = repository
                .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(msg.clone()), None, None, None, None)
                .await;
            Err(msg)
        }
        Err(e) => {
            if index_tmp_dir.exists() {
                let _ = std::fs::remove_dir_all(&index_tmp_dir);
            }
            let msg = format!("spawn_blocking panicked: {e}");
            eprintln!("[indexing] speedwagon {id} panic: {msg}");
            let _ = repository
                .update_speedwagon_index_status(id, SpeedwagonIndexStatus::Error, Some(msg.clone()), None, None, None, None)
                .await;
            Err(msg)
        }
    }
}
