//! Integration tests for incremental indexer sync.
//!
//! Tests add, delete, and no-op scenarios using a temporary corpus directory.
//!
//! Run: cargo test -p knowledge-agent --test sync_tests -- --nocapture

use std::fs;
use std::path::Path;

use knowledge_agent::{IndexSettings, SearchIndex, sync_or_build_multi};

fn settings() -> IndexSettings {
    IndexSettings { schema_version: 1, no_merge: false }
}

/// Create a temp dir with given text files. Returns the temp dir path.
fn create_corpus(dir: &Path, files: &[(&str, &str)]) {
    fs::create_dir_all(dir).unwrap();
    for (name, content) in files {
        fs::write(dir.join(name), content).unwrap();
    }
}

fn indexed_count(index_dir: &Path) -> usize {
    let search_index = SearchIndex::open(index_dir).unwrap();
    search_index.indexed_filepaths().unwrap().len()
}

#[test]
fn full_build_then_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    let index = tmp.path().join("index");

    create_corpus(&corpus, &[
        ("doc1.txt", "The quick brown fox jumps over the lazy dog."),
        ("doc2.txt", "A tale of two cities by Charles Dickens."),
        ("doc3.md", "# Revenue Report\nTotal revenue was $1 billion."),
    ]);

    // Full build (no existing index)
    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(report.success_count, 3);
    assert_eq!(indexed_count(&index), 3);

    // No-op (nothing changed)
    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(report.total_files, 3);
    assert_eq!(report.success_count, 3);
    // No actual indexing work done
    assert_eq!(report.total_duration_secs, 0.0);
}

#[test]
fn incremental_add() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    let index = tmp.path().join("index");

    // Start with 2 files
    create_corpus(&corpus, &[
        ("doc1.txt", "First document content."),
        ("doc2.txt", "Second document content."),
    ]);

    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(report.success_count, 2);
    assert_eq!(indexed_count(&index), 2);

    // Add a third file
    fs::write(corpus.join("doc3.md"), "# Third document\nNew content here.").unwrap();

    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    // Should have indexed only the new file
    assert_eq!(report.success_count, 1);
    assert_eq!(indexed_count(&index), 3);
}

#[test]
fn incremental_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    let index = tmp.path().join("index");

    // Start with 3 files
    create_corpus(&corpus, &[
        ("doc1.txt", "First document."),
        ("doc2.txt", "Second document."),
        ("doc3.md", "Third document."),
    ]);

    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(report.success_count, 3);
    assert_eq!(indexed_count(&index), 3);

    // Delete one file
    fs::remove_file(corpus.join("doc2.txt")).unwrap();

    sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(indexed_count(&index), 2);
}

#[test]
fn incremental_add_and_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    let index = tmp.path().join("index");

    create_corpus(&corpus, &[
        ("doc1.txt", "Alpha."),
        ("doc2.txt", "Bravo."),
        ("doc3.md", "Charlie."),
    ]);

    sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(indexed_count(&index), 3);

    // Delete doc1, add doc4
    fs::remove_file(corpus.join("doc1.txt")).unwrap();
    fs::write(corpus.join("doc4.txt"), "Delta.").unwrap();

    let report = sync_or_build_multi(&index, &[corpus.clone()], &settings(), false).unwrap();
    assert_eq!(report.success_count, 1); // 1 added
    assert_eq!(indexed_count(&index), 3); // 3 total (2 old + 1 new)
}
