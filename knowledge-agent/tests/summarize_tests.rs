use std::path::Path;
use std::sync::Once;

use ailoy::{LangModelAPISchema, LangModelProvider};
use knowledge_agent::{
    IndexSettings, SearchIndex, SummarizeConfig, check_or_build_index, summarize_document,
};
use url::Url;

const INDEX_DIR: &str = "/tmp/knowledge_agent_summarize_test_index";

fn md_dir() -> String {
    format!("{}/data/financebench", env!("CARGO_MANIFEST_DIR"))
}

static INIT_INDEX: Once = Once::new();

fn build_index() -> SearchIndex {
    INIT_INDEX.call_once(|| {
        let settings = IndexSettings {
            schema_version: 1,
            no_merge: false,
        };
        check_or_build_index(
            Path::new(INDEX_DIR),
            Path::new(&md_dir()),
            &settings,
            true,
            false,
        )
        .expect("indexing failed");
    });
    SearchIndex::open(Path::new(INDEX_DIR)).expect("open failed")
}

fn make_config() -> SummarizeConfig {
    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env")).ok();
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    SummarizeConfig::new(
        "gpt-4.1-mini".to_string(),
        LangModelProvider::openai(api_key),
    )
}

/// Basic test: verify SummarizeConfig can be constructed.
#[test]
fn summarize_config_construction() {
    let config = SummarizeConfig::new(
        "gpt-4.1-mini".to_string(),
        LangModelProvider::openai("test-key".into()),
    );
    assert_eq!(config.model_name, "gpt-4.1-mini");
    match config.model_provider {
        LangModelProvider::API { api_key, .. } => {
            assert!(api_key.is_some_and(|key| key == "test-key"));
        }
    }
}

/// Single-pass summarize: document under SINGLE_PASS_MAX_LINES (4000).
/// Verifies that the summary is shorter than the original and chunks_processed == 1.
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn summarize_single_pass() {
    let index = build_index();
    let config = make_config();

    let result = summarize_document(&index, "COSTCO_2023Q1_EARNINGS.md", 500, None, &config)
        .await
        .expect("summarize failed");

    assert_eq!(result.chunks_processed, 1);
    assert_eq!(result.chunks_failed, 0);
    assert!(!result.reduce_truncated);
    assert!(result.summary.len() <= 800); // 500 target + some margin
    assert!(!result.summary.is_empty());
    println!(
        "Single-pass: {} lines → {} chars summary",
        result.original_lines,
        result.summary.len()
    );
}

/// Map-reduce summarize: use a large document that exceeds SINGLE_PASS_MAX_LINES.
/// Verifies chunked processing and that chunks_processed > 1.
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn summarize_map_reduce() {
    let index = build_index();
    let config = make_config();

    // Find a large document (>4000 lines)
    let filepaths = index.indexed_filepaths().expect("filepaths");
    let large_doc = filepaths.iter().find(|fp| {
        let doc = index.get_document(fp).unwrap();
        doc.content.lines().count() > 4000
    });

    let filepath = match large_doc {
        Some(fp) => fp.as_str(),
        None => {
            println!("No document >4000 lines found, skipping map-reduce test");
            return;
        }
    };

    let result = summarize_document(&index, filepath, 500, None, &config)
        .await
        .expect("summarize failed");

    assert!(result.chunks_processed > 1);
    assert!(result.summary.len() <= 800);
    assert!(!result.summary.is_empty());
    println!(
        "Map-reduce: {} lines, {} chunks ({} failed) → {} chars summary",
        result.original_lines,
        result.chunks_processed,
        result.chunks_failed,
        result.summary.len()
    );
}

/// Summarize with focus parameter.
#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn summarize_with_focus() {
    let index = build_index();
    let config = make_config();

    let result = summarize_document(
        &index,
        "COSTCO_2023Q1_EARNINGS.md",
        500,
        Some("revenue and net income"),
        &config,
    )
    .await
    .expect("summarize failed");

    assert!(!result.summary.is_empty());
    assert!(result.summary.len() <= 800);
    println!("Focused summary: {}", result.summary);
}
