use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use chrono::Utc;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument, Term, merge_policy::NoMergePolicy, schema::*};

// ---------------------------------------------------------------------------
// Types (same as indexer.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct IndexMeta {
    pub schema_version: u32,
    pub corpus_path: String,
    pub file_count: usize,
    pub build_timestamp: String,
    pub build_duration_secs: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct FileIndexResult {
    pub filename: String,
    pub id: String,
    pub chars: usize,
    pub success: bool,
    pub error: Option<String>,
    pub read_us: u128,
    pub index_us: u128,
    pub total_us: u128,
}

#[derive(Debug, Serialize)]
pub struct IndexReport {
    pub total_files: usize,
    pub success_count: usize,
    pub fail_count: usize,
    pub skipped_count: usize,
    pub total_chars: usize,
    pub total_index_secs: f64,
    pub total_duration_secs: f64,
    pub commit_secs: f64,
    pub files: Vec<FileIndexResult>,
}

pub struct IndexSettings {
    pub schema_version: u32,
    /// When true, use NoMergePolicy and commit after each document (1 doc = 1 segment).
    pub no_merge: bool,
}

// ---------------------------------------------------------------------------
// Internal helpers (same as indexer.rs)
// ---------------------------------------------------------------------------

struct DocIdentity {
    filepath: String,
}

fn extract_doc_identity(path: &Path, corpus_dir: &Path) -> Option<DocIdentity> {
    let ext = path.extension()?.to_string_lossy();
    match ext.as_ref() {
        "txt" | "md" => {
            let relative = path.strip_prefix(corpus_dir).ok()?;
            Some(DocIdentity {
                filepath: relative.to_string_lossy().into_owned(),
            })
        }
        _ => None,
    }
}

fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(true)
        .follow_links(true)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path().to_owned())
        .filter(|p| {
            p.extension()
                .map(|x| x == "txt" || x == "md")
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    entries
}

fn save_meta(
    index_dir: &Path,
    settings: &IndexSettings,
    corpus_path: &str,
    file_count: usize,
    duration_secs: f64,
) -> Result<()> {
    let meta = IndexMeta {
        schema_version: settings.schema_version,
        corpus_path: corpus_path.to_string(),
        file_count,
        build_timestamp: Utc::now().to_rfc3339(),
        build_duration_secs: duration_secs,
    };
    let meta_path = index_dir.join("index_meta.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    println!("Saved index_meta.json to {}", meta_path.display());
    Ok(())
}

fn indexed_filepaths(index: &Index) -> Result<HashSet<String>> {
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()?;
    reader.reload()?;
    let searcher = reader.searcher();
    let field = index.schema().get_field("filepath")?;

    let mut set = HashSet::new();
    for seg in searcher.segment_readers() {
        let inv = seg.inverted_index(field)?;
        let mut stream = inv.terms().stream()?;
        while stream.advance() {
            if let Ok(s) = std::str::from_utf8(stream.key()) {
                if !s.is_empty() {
                    set.insert(s.to_string());
                }
            }
        }
    }
    Ok(set)
}

// ---------------------------------------------------------------------------
// Full build (from indexer.rs)
// ---------------------------------------------------------------------------

pub fn build_index(
    index_dir: &Path,
    corpus_dir: &Path,
    settings: &IndexSettings,
) -> Result<(IndexMeta, IndexReport)> {
    if index_dir.exists() {
        fs::remove_dir_all(index_dir)
            .with_context(|| format!("Failed to remove old index at {}", index_dir.display()))?;
    }
    fs::create_dir_all(index_dir)?;

    let mut sb = Schema::builder();
    let filepath_f = sb.add_text_field("filepath", STRING | STORED);
    let content_f = sb.add_text_field("content", TEXT | STORED);
    let schema = sb.build();

    let index = Index::create_in_dir(index_dir, schema)?;
    let mut writer: IndexWriter = index.writer(128_000_000)?;
    if settings.no_merge {
        writer.set_merge_policy(Box::new(NoMergePolicy));
    }

    let entries = collect_files(corpus_dir);

    let total_start = Instant::now();
    let mut file_count = 0usize;
    let mut file_results: Vec<FileIndexResult> = Vec::new();
    let mut skipped_count = 0usize;

    for path in &entries {
        let filename = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some(ident) = extract_doc_identity(path, corpus_dir) else {
            skipped_count += 1;
            eprintln!("Skipping (unrecognized pattern): {}", filename);
            continue;
        };

        let file_start = Instant::now();

        let result = (|| -> Result<(usize, u128, u128)> {
            let read_start = Instant::now();
            let text = fs::read_to_string(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let read_us = read_start.elapsed().as_micros();
            let chars = text.len();

            let index_start = Instant::now();
            let mut doc = TantivyDocument::default();
            doc.add_text(filepath_f, &ident.filepath);
            doc.add_text(content_f, &text);
            writer.add_document(doc)?;
            if settings.no_merge {
                writer.commit()?;
            }
            let index_us = index_start.elapsed().as_micros();

            Ok((chars, read_us, index_us))
        })();

        let total_us = file_start.elapsed().as_micros();

        match result {
            Ok((chars, read_us, index_us)) => {
                file_count += 1;
                println!(
                    "  [{file_count:>3}] {} ({chars} chars) — read: {read_us}μs, index: {index_us}μs",
                    ident.filepath
                );
                file_results.push(FileIndexResult {
                    filename,
                    id: ident.filepath,
                    chars,
                    success: true,
                    error: None,
                    read_us,
                    index_us,
                    total_us,
                });
            }
            Err(e) => {
                eprintln!("  [ERR] {} ({filename}): {e}", ident.filepath);
                file_results.push(FileIndexResult {
                    filename,
                    id: ident.filepath,
                    chars: 0,
                    success: false,
                    error: Some(format!("{e}")),
                    read_us: 0,
                    index_us: 0,
                    total_us,
                });
            }
        }
    }

    let commit_start = Instant::now();
    if !settings.no_merge {
        writer.commit()?;
    }
    let commit_secs = commit_start.elapsed().as_secs_f64();
    let total_secs = total_start.elapsed().as_secs_f64();

    let fail_count = file_results.iter().filter(|r| !r.success).count();
    println!(
        "Indexing complete: {file_count} files ({fail_count} failed, {skipped_count} skipped), commit: {commit_secs:.2}s, total: {total_secs:.1}s"
    );

    let total_chars: usize = file_results.iter().map(|r| r.chars).sum();
    let total_index_secs: f64 =
        file_results.iter().map(|r| r.index_us).sum::<u128>() as f64 / 1_000_000.0;

    let report = IndexReport {
        total_files: file_results.len(),
        success_count: file_count,
        fail_count,
        skipped_count,
        total_chars,
        total_index_secs,
        total_duration_secs: total_secs,
        commit_secs,
        files: file_results,
    };

    let meta = IndexMeta {
        schema_version: settings.schema_version,
        corpus_path: corpus_dir.to_string_lossy().to_string(),
        file_count,
        build_timestamp: Utc::now().to_rfc3339(),
        build_duration_secs: total_secs,
    };

    let meta_path = index_dir.join("index_meta.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    println!("Saved index_meta.json to {}", meta_path.display());

    Ok((meta, report))
}

pub fn check_or_build_index(
    index_dir: &Path,
    corpus_dir: &Path,
    settings: &IndexSettings,
    rebuild_on_mismatch: bool,
    force_reindex: bool,
) -> Result<Option<IndexReport>> {
    let meta_path = index_dir.join("index_meta.json");

    if force_reindex {
        println!("--reindex flag: forcing rebuild.");
        let (_, report) = build_index(index_dir, corpus_dir, settings)?;
        return Ok(Some(report));
    }

    if !meta_path.exists() {
        println!("No existing index found. Building...");
        let (_, report) = build_index(index_dir, corpus_dir, settings)?;
        return Ok(Some(report));
    }

    let raw = fs::read_to_string(&meta_path)?;
    let existing: IndexMeta = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse {}", meta_path.display()))?;

    let file_count = collect_files(corpus_dir).len();

    let matches = existing.schema_version == settings.schema_version
        && existing.corpus_path == corpus_dir.to_string_lossy()
        && existing.file_count == file_count;

    if matches {
        println!(
            "Index is up to date ({file_count} files, built at {}, {:.1}s).",
            existing.build_timestamp, existing.build_duration_secs
        );
        return Ok(None);
    }

    if rebuild_on_mismatch {
        println!("Index mismatch detected. Rebuilding...");
        let (_, report) = build_index(index_dir, corpus_dir, settings)?;
        Ok(Some(report))
    } else {
        anyhow::bail!(
            "Index mismatch detected. Set rebuild_on_mismatch=true or use --reindex to rebuild."
        );
    }
}

pub fn build_index_multi(
    index_dir: &Path,
    corpus_dirs: &[PathBuf],
    settings: &IndexSettings,
    force_reindex: bool,
) -> Result<IndexReport> {
    let meta_path = index_dir.join("index_meta.json");

    let total_files_on_disk: usize = corpus_dirs.iter().map(|d| collect_files(d).len()).sum();

    if !force_reindex && meta_path.exists() {
        if let Ok(raw) = fs::read_to_string(&meta_path) {
            if let Ok(existing) = serde_json::from_str::<IndexMeta>(&raw) {
                if existing.schema_version == settings.schema_version
                    && existing.file_count == total_files_on_disk
                {
                    println!(
                        "Index is up to date ({total_files_on_disk} files, built at {}, {:.1}s).",
                        existing.build_timestamp, existing.build_duration_secs
                    );
                    return Ok(IndexReport {
                        total_files: total_files_on_disk,
                        success_count: total_files_on_disk,
                        fail_count: 0,
                        skipped_count: 0,
                        total_chars: 0,
                        total_index_secs: 0.0,
                        total_duration_secs: 0.0,
                        commit_secs: 0.0,
                        files: vec![],
                    });
                }
            }
        }
    }

    if index_dir.exists() {
        fs::remove_dir_all(index_dir)
            .with_context(|| format!("Failed to remove old index at {}", index_dir.display()))?;
    }
    fs::create_dir_all(index_dir)?;

    let mut sb = Schema::builder();
    let filepath_f = sb.add_text_field("filepath", STRING | STORED);
    let content_f = sb.add_text_field("content", TEXT | STORED);
    let schema = sb.build();

    let index = Index::create_in_dir(index_dir, schema)?;
    let mut writer: IndexWriter = index.writer(128_000_000)?;
    if settings.no_merge {
        writer.set_merge_policy(Box::new(NoMergePolicy));
    }

    let total_start = Instant::now();
    let mut file_count = 0usize;
    let mut file_results: Vec<FileIndexResult> = Vec::new();
    let mut skipped_count = 0usize;

    for corpus_dir in corpus_dirs {
        let entries = collect_files(corpus_dir);
        println!(
            "Indexing {} from {}...",
            entries.len(),
            corpus_dir.display()
        );

        for path in &entries {
            let filename = path.file_name().unwrap().to_string_lossy().into_owned();
            let Some(ident) = extract_doc_identity(path, corpus_dir) else {
                skipped_count += 1;
                eprintln!("Skipping (unrecognized pattern): {}", filename);
                continue;
            };

            let file_start = Instant::now();

            let result = (|| -> Result<(usize, u128, u128)> {
                let read_start = Instant::now();
                let text = fs::read_to_string(path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                let read_us = read_start.elapsed().as_micros();
                let chars = text.len();

                let index_start = Instant::now();
                let mut doc = TantivyDocument::default();
                doc.add_text(filepath_f, &ident.filepath);
                doc.add_text(content_f, &text);
                writer.add_document(doc)?;
                if settings.no_merge {
                    writer.commit()?;
                }
                let index_us = index_start.elapsed().as_micros();

                Ok((chars, read_us, index_us))
            })();

            let total_us = file_start.elapsed().as_micros();

            match result {
                Ok((chars, read_us, index_us)) => {
                    file_count += 1;
                    println!(
                        "  [{file_count:>3}] {} ({chars} chars) — read: {read_us}μs, index: {index_us}μs",
                        ident.filepath
                    );
                    file_results.push(FileIndexResult {
                        filename,
                        id: ident.filepath,
                        chars,
                        success: true,
                        error: None,
                        read_us,
                        index_us,
                        total_us,
                    });
                }
                Err(e) => {
                    eprintln!("  [ERR] {} ({filename}): {e}", ident.filepath);
                    file_results.push(FileIndexResult {
                        filename,
                        id: ident.filepath,
                        chars: 0,
                        success: false,
                        error: Some(format!("{e}")),
                        read_us: 0,
                        index_us: 0,
                        total_us,
                    });
                }
            }
        }
    }

    let commit_start = Instant::now();
    if !settings.no_merge {
        writer.commit()?;
    }
    let commit_secs = commit_start.elapsed().as_secs_f64();
    let total_secs = total_start.elapsed().as_secs_f64();

    let fail_count = file_results.iter().filter(|r| !r.success).count();
    println!(
        "Indexing complete: {file_count} files ({fail_count} failed, {skipped_count} skipped), commit: {commit_secs:.2}s, total: {total_secs:.1}s"
    );

    let total_chars: usize = file_results.iter().map(|r| r.chars).sum();
    let total_index_secs: f64 =
        file_results.iter().map(|r| r.index_us).sum::<u128>() as f64 / 1_000_000.0;

    let report = IndexReport {
        total_files: file_results.len(),
        success_count: file_count,
        fail_count,
        skipped_count,
        total_chars,
        total_index_secs,
        total_duration_secs: total_secs,
        commit_secs,
        files: file_results,
    };

    let meta = IndexMeta {
        schema_version: settings.schema_version,
        corpus_path: corpus_dirs
            .first()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        file_count,
        build_timestamp: Utc::now().to_rfc3339(),
        build_duration_secs: total_secs,
    };

    let meta_path = index_dir.join("index_meta.json");
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    println!("Saved index_meta.json to {}", meta_path.display());

    Ok(report)
}

// ---------------------------------------------------------------------------
// Incremental sync (new)
// ---------------------------------------------------------------------------

/// Diff result between disk and index.
#[derive(Debug)]
pub struct SyncPlan {
    pub to_add: Vec<(String, PathBuf)>,  // (relative filepath, absolute path)
    pub to_delete: Vec<String>,           // relative filepaths to remove
    pub unchanged: usize,
}

/// Incremental sync report.
#[derive(Debug)]
pub struct SyncReport {
    pub added: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub add_ms: f64,
    pub delete_ms: f64,
    pub commit_ms: f64,
    pub total_ms: f64,
}

/// Compare disk files against indexed filepaths and produce a sync plan.
pub fn plan_sync(index: &Index, corpus_dirs: &[PathBuf]) -> Result<SyncPlan> {
    let existing = indexed_filepaths(index)?;

    let mut disk_files: Vec<(String, PathBuf)> = Vec::new();
    let mut disk_ids: HashSet<String> = HashSet::new();

    for corpus_dir in corpus_dirs {
        for path in collect_files(corpus_dir) {
            if let Some(ident) = extract_doc_identity(&path, corpus_dir) {
                disk_ids.insert(ident.filepath.clone());
                disk_files.push((ident.filepath, path));
            }
        }
    }

    let to_add: Vec<(String, PathBuf)> = disk_files
        .into_iter()
        .filter(|(id, _)| !existing.contains(id))
        .collect();

    let to_delete: Vec<String> = existing
        .into_iter()
        .filter(|id| !disk_ids.contains(id))
        .collect();

    let unchanged = disk_ids.len() - to_add.len();

    Ok(SyncPlan {
        to_add,
        to_delete,
        unchanged,
    })
}

/// Apply a sync plan: delete removed docs, add new docs, commit once.
pub fn apply_sync(
    index: &Index,
    writer: &mut IndexWriter,
    plan: &SyncPlan,
) -> Result<SyncReport> {
    let schema = index.schema();
    let filepath_f = schema.get_field("filepath")?;
    let content_f = schema.get_field("content")?;

    let total_start = Instant::now();

    // Delete
    let t = Instant::now();
    for fp in &plan.to_delete {
        writer.delete_term(Term::from_field_text(filepath_f, fp));
        println!("  -{fp}");
    }
    let delete_ms = t.elapsed().as_secs_f64() * 1000.0;

    // Add
    let t = Instant::now();
    for (rel, path) in &plan.to_add {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let chars = text.len();

        let mut doc = TantivyDocument::default();
        doc.add_text(filepath_f, rel);
        doc.add_text(content_f, &text);
        writer.add_document(doc)?;
        println!("  +{rel} ({chars} chars)");
    }
    let add_ms = t.elapsed().as_secs_f64() * 1000.0;

    // Commit
    let t = Instant::now();
    writer.commit()?;
    let commit_ms = t.elapsed().as_secs_f64() * 1000.0;

    let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

    Ok(SyncReport {
        added: plan.to_add.len(),
        deleted: plan.to_delete.len(),
        unchanged: plan.unchanged,
        add_ms,
        delete_ms,
        commit_ms,
        total_ms,
    })
}

/// Open existing index, diff against disk, apply incremental changes.
/// Returns None if already in sync.
pub fn sync_index(
    index_dir: &Path,
    corpus_dirs: &[PathBuf],
    settings: &IndexSettings,
) -> Result<Option<SyncReport>> {
    let index = Index::open_in_dir(index_dir)
        .with_context(|| format!("Failed to open index at {}", index_dir.display()))?;

    let plan = plan_sync(&index, corpus_dirs)?;

    if plan.to_add.is_empty() && plan.to_delete.is_empty() {
        println!("Index in sync: {} files, nothing to do.", plan.unchanged);
        return Ok(None);
    }

    println!(
        "Sync plan: +{} add, -{} delete, {} unchanged",
        plan.to_add.len(),
        plan.to_delete.len(),
        plan.unchanged
    );

    let mut writer: IndexWriter = index.writer(128_000_000)?;
    let report = apply_sync(&index, &mut writer, &plan)?;

    // Update meta
    let total_count = report.added + report.unchanged;
    save_meta(
        index_dir,
        settings,
        &corpus_dirs
            .first()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        total_count,
        report.total_ms / 1000.0,
    )?;

    println!(
        "Sync complete: +{} added, -{} deleted, {} unchanged — {:.1}ms",
        report.added, report.deleted, report.unchanged, report.total_ms
    );

    Ok(Some(report))
}

/// Smart entry point: full build if no index exists or force_reindex,
/// otherwise incremental sync.
pub fn sync_or_build_multi(
    index_dir: &Path,
    corpus_dirs: &[PathBuf],
    settings: &IndexSettings,
    force_reindex: bool,
) -> Result<IndexReport> {
    // Force rebuild or no existing index → full build (same as build_index_multi)
    if force_reindex || !index_dir.join("meta.json").exists() {
        return build_index_multi(index_dir, corpus_dirs, settings, force_reindex);
    }

    // Existing index → try incremental sync
    match sync_index(index_dir, corpus_dirs, settings)? {
        Some(sr) => {
            // Wrap SyncReport into IndexReport for caller compatibility
            Ok(IndexReport {
                total_files: sr.added + sr.unchanged,
                success_count: sr.added,
                fail_count: 0,
                skipped_count: 0,
                total_chars: 0,
                total_index_secs: sr.add_ms / 1000.0,
                total_duration_secs: sr.total_ms / 1000.0,
                commit_secs: sr.commit_ms / 1000.0,
                files: vec![],
            })
        }
        None => {
            let total = corpus_dirs.iter().map(|d| collect_files(d).len()).sum();
            Ok(IndexReport {
                total_files: total,
                success_count: total,
                fail_count: 0,
                skipped_count: 0,
                total_chars: 0,
                total_index_secs: 0.0,
                total_duration_secs: 0.0,
                commit_secs: 0.0,
                files: vec![],
            })
        }
    }
}
