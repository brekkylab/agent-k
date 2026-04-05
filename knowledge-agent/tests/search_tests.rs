use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Once},
};

use ailoy::message::Part;
use knowledge_agent::{
    IndexSettings, SearchIndex, SearchOutput, SearchResult, ToolConfig, build_tool_set,
    check_or_build_index,
};
use serde::Deserialize;

const INDEX_DIR: &str = "/tmp/knowledge_agent_test_index";

fn books_dir() -> String {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["txt_dir"].as_str().unwrap().to_string()
}

fn qa_file() -> String {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["qa_file"].as_str().unwrap().to_string()
}

fn report_path(name: &str) -> String {
    format!("{}/{}", env!("CARGO_MANIFEST_DIR"), name)
}

static INIT_INDEX: Once = Once::new();

fn ensure_index() -> SearchIndex {
    INIT_INDEX.call_once(|| {
        let settings = IndexSettings {
            schema_version: 1,
            no_merge: false,
        };
        check_or_build_index(
            Path::new(INDEX_DIR),
            Path::new(&books_dir()),
            &settings,
            true,
            false,
        )
        .expect("indexing failed");
    });
    SearchIndex::open(Path::new(INDEX_DIR)).expect("open failed")
}

#[derive(Deserialize)]
struct QaItem {
    book_id: String,
    book_title: String,
    question_id: String,
    question: String,
}

fn load_qa() -> Vec<QaItem> {
    let raw = fs::read_to_string(qa_file()).expect("failed to read QA file");
    serde_json::from_str(&raw).expect("failed to parse QA file")
}

/// 전체 책에서 질문을 샘플링하고 book별 summary를 포함한 리포트 생성
fn run_full_search_test<F>(
    index: &SearchIndex,
    all_qa: &[QaItem],
    top_k: usize,
    search_fn: F,
) -> serde_json::Value
where
    F: Fn(&str, usize) -> anyhow::Result<Vec<SearchResult>>,
{
    let indexed_paths = index.indexed_filepaths().expect("failed to get filepaths");

    let mut by_book: HashMap<String, Vec<&QaItem>> = HashMap::new();
    for qa in all_qa {
        let has_book = indexed_paths.iter().any(|p| p.contains(&qa.book_id));
        if has_book {
            by_book.entry(qa.book_id.clone()).or_default().push(qa);
        }
    }

    let mut book_ids: Vec<String> = by_book.keys().cloned().collect();
    book_ids.sort();

    let mut total_queries = 0usize;
    let mut total_hits = 0usize;
    let mut book_summaries = Vec::new();
    let mut all_query_reports = Vec::new();

    for book_id in &book_ids {
        let questions = by_book.get(book_id).unwrap();
        let book_title = &questions[0].book_title;
        let sample: Vec<&&QaItem> = questions.iter().take(10).collect();

        let mut book_hits = 0usize;
        for qa in &sample {
            total_queries += 1;
            let results = search_fn(&qa.question, top_k).expect("search failed");
            let hit = results.iter().any(|r| r.filepath.contains(&qa.book_id));
            if hit {
                total_hits += 1;
                book_hits += 1;
            }
            all_query_reports.push(serde_json::json!({
                "question_id": qa.question_id,
                "book_id": qa.book_id,
                "book_title": qa.book_title,
                "question": qa.question,
                "hit": hit,
                "results": results,
            }));
        }

        book_summaries.push(serde_json::json!({
            "book_id": book_id,
            "book_title": book_title,
            "queries": sample.len(),
            "hits": book_hits,
            "hit_rate_pct": if sample.is_empty() { 0.0 } else { book_hits as f64 / sample.len() as f64 * 100.0 },
        }));
    }

    serde_json::json!({
        "total_books": book_ids.len(),
        "total_queries": total_queries,
        "top_k": top_k,
        "total_hits": total_hits,
        "hit_rate_pct": if total_queries == 0 { 0.0 } else { total_hits as f64 / total_queries as f64 * 100.0 },
        "book_summaries": book_summaries,
        "queries": all_query_reports,
    })
}

#[test]
fn test_searcher_full() {
    let index = ensure_index();
    let all_qa = load_qa();
    let top_k = 3;

    let mut report =
        run_full_search_test(&index, &all_qa, top_k, |query, k| index.search(query, k));
    report
        .as_object_mut()
        .unwrap()
        .insert("test".into(), "test_searcher_full".into());

    let json = serde_json::to_string_pretty(&report).unwrap();
    fs::write(report_path("test_report_searcher.json"), &json)
        .expect("failed to write searcher report");

    println!("=== Searcher Full Test ===");
    println!(
        "Books: {}, Queries: {}, Hits: {}, Hit Rate: {:.1}%",
        report["total_books"],
        report["total_queries"],
        report["total_hits"],
        report["hit_rate_pct"].as_f64().unwrap()
    );
    for bs in report["book_summaries"].as_array().unwrap() {
        println!(
            "  {} ({}) — {}/{} ({:.0}%)",
            bs["book_id"],
            bs["book_title"],
            bs["hits"],
            bs["queries"],
            bs["hit_rate_pct"].as_f64().unwrap()
        );
    }
}

#[tokio::test]
async fn test_ailoy_tool_full() {
    let index_for_tool = Arc::new(ensure_index());
    let index_for_meta = ensure_index();
    let tool_set = build_tool_set(
        index_for_tool,
        &ToolConfig::default(),
        vec![std::path::PathBuf::from(books_dir())],
    );
    let tool = tool_set
        .get("search_document")
        .expect("search tool not found");
    let all_qa = load_qa();
    let top_k = 3;

    let desc = tool.desc();
    assert_eq!(desc.name, "search_document");
    assert!(desc.description.as_deref().unwrap().contains("BM25"));

    let indexed_paths = index_for_meta
        .indexed_filepaths()
        .expect("failed to get filepaths");

    let mut by_book: HashMap<String, Vec<&QaItem>> = HashMap::new();
    for qa in &all_qa {
        let has_book = indexed_paths.iter().any(|p| p.contains(&qa.book_id));
        if has_book {
            by_book.entry(qa.book_id.clone()).or_default().push(qa);
        }
    }
    let mut book_ids: Vec<String> = by_book.keys().cloned().collect();
    book_ids.sort();

    let mut total_queries = 0usize;
    let mut total_hits = 0usize;
    let mut book_summaries = Vec::new();
    let mut all_query_reports = Vec::new();

    for book_id in &book_ids {
        let questions = by_book.get(book_id).unwrap();
        let book_title = &questions[0].book_title;
        let sample: Vec<&&QaItem> = questions.iter().take(10).collect();

        let mut book_hits = 0usize;
        for qa in &sample {
            total_queries += 1;
            let args = ailoy::to_value!({
                "query": qa.question.clone(),
                "top_k": top_k
            });
            let result_msg = tool
                .run(Part::function("search_document", args))
                .await
                .expect("tool run failed");
            let result_val = result_msg.contents[0].as_value().expect("value");
            let json_val = serde_json::to_value(result_val).unwrap();
            let output: SearchOutput = serde_json::from_value(json_val).unwrap();
            let results = &output.results;

            let hit = results.iter().any(|r| r.filepath.contains(&qa.book_id));
            if hit {
                total_hits += 1;
                book_hits += 1;
            }
            all_query_reports.push(serde_json::json!({
                "question_id": qa.question_id,
                "book_id": qa.book_id,
                "book_title": qa.book_title,
                "question": qa.question,
                "hit": hit,
                "results": results,
            }));
        }

        book_summaries.push(serde_json::json!({
            "book_id": book_id,
            "book_title": book_title,
            "queries": sample.len(),
            "hits": book_hits,
            "hit_rate_pct": if sample.is_empty() { 0.0 } else { book_hits as f64 / sample.len() as f64 * 100.0 },
        }));
    }

    let report = serde_json::json!({
        "test": "test_ailoy_tool_full",
        "tool_name": desc.name,
        "tool_description": desc.description,
        "total_books": book_ids.len(),
        "total_queries": total_queries,
        "top_k": top_k,
        "total_hits": total_hits,
        "hit_rate_pct": if total_queries == 0 { 0.0 } else { total_hits as f64 / total_queries as f64 * 100.0 },
        "book_summaries": book_summaries,
        "queries": all_query_reports,
    });

    let json = serde_json::to_string_pretty(&report).unwrap();
    fs::write(report_path("test_report_ailoy.json"), &json).expect("failed to write ailoy report");

    println!("=== Ailoy Tool Full Test ===");
    println!(
        "Tool: {} | Books: {}, Queries: {}, Hits: {}, Hit Rate: {:.1}%",
        report["tool_name"],
        report["total_books"],
        report["total_queries"],
        report["total_hits"],
        report["hit_rate_pct"].as_f64().unwrap()
    );
    for bs in report["book_summaries"].as_array().unwrap() {
        println!(
            "  {} ({}) — {}/{} ({:.0}%)",
            bs["book_id"],
            bs["book_title"],
            bs["hits"],
            bs["queries"],
            bs["hit_rate_pct"].as_f64().unwrap()
        );
    }
}
