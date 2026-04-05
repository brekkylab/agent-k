use std::{
    fs,
    path::Path,
    sync::{Arc, Once},
};

use ailoy::message::Part;
use knowledge_agent::{
    IndexSettings, SearchIndex, ToolConfig, build_tool_set, check_or_build_index,
};

const INDEX_DIR: &str = "/tmp/knowledge_agent_find_open_test_index";

fn books_dir() -> String {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["txt_dir"].as_str().unwrap().to_string()
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

// ─── get_document_by_id ──────────────────────────────────────────────────────

#[test]
fn test_get_document() {
    let index = ensure_index();
    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let doc = index.get_document(first).expect("get_document failed");
    assert_eq!(doc.filepath, *first);
    assert!(!doc.content.is_empty());
    println!("get_document({}): {} chars", first, doc.content.len(),);
}

#[test]
fn test_get_document_not_found() {
    let index = ensure_index();
    let result = index.get_document("nonexistent/file.txt");
    assert!(result.is_err());
    println!("Not found error: {}", result.unwrap_err());
}

// ─── find_in_document ────────────────────────────────────────────────────────

#[test]
fn test_find_in_document() {
    let index = ensure_index();
    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let doc = index.get_document(first).unwrap();
    let first_line = doc.content.lines().nth(10).unwrap_or("the");
    let keyword = first_line
        .split_whitespace()
        .find(|w| w.len() > 4)
        .unwrap_or("chapter");

    let result = knowledge_agent::find_in_document(
        first,
        &doc.content,
        keyword,
        0,
        ToolConfig::default().max_matches,
    );

    println!("find_in_document({}, '{}'):", first, keyword);
    println!("  total_lines: {}", result.total_lines);
    println!("  matches: {}", result.matches.len());
    assert!(!result.matches.is_empty(), "should find at least one match");

    for (i, m) in result.matches.iter().take(3).enumerate() {
        println!(
            "  match[{}]: \"{}\" at {}-{}, line: {}",
            i,
            m.keyword,
            m.start,
            m.end,
            m.line_content.chars().take(80).collect::<String>()
        );
    }

    let first_match = &result.matches[0];
    assert!(first_match.start.col >= 1);
    assert!(!first_match.line_content.is_empty());
}

#[test]
fn test_find_no_matches() {
    let index = ensure_index();
    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let doc = index.get_document(first).unwrap();
    let result = knowledge_agent::find_in_document(
        first,
        &doc.content,
        "xyzzy_nonexistent_keyword_12345",
        0,
        ToolConfig::default().max_matches,
    );

    assert!(result.matches.is_empty());
    println!("No matches found (expected)");
}

// ─── open_document ───────────────────────────────────────────────────────────

#[test]
fn test_open_document_range() {
    let index = ensure_index();
    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let doc = index.get_document(first).unwrap();
    let result = knowledge_agent::open_document(
        first,
        &doc.content,
        Some(10),
        Some(20),
        ToolConfig::default().max_content_chars,
        ToolConfig::default().max_lines_per_open,
    );

    println!("open_document({}, 10, 20):", first);
    println!("  total_lines: {}", result.total_lines);
    println!("  range: {}-{}", result.start_line, result.end_line);
    println!("  content:\n{}", result.content);

    assert_eq!(result.start_line, 10);
    assert!(result.end_line <= 20);
    assert!(result.content.contains("10:"));
}

#[test]
fn test_open_document_defaults() {
    let index = ensure_index();
    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let doc = index.get_document(first).unwrap();
    let result = knowledge_agent::open_document(
        first,
        &doc.content,
        None,
        None,
        ToolConfig::default().max_content_chars,
        ToolConfig::default().max_lines_per_open,
    );

    assert_eq!(result.start_line, 1);
    assert!(result.end_line >= 1);
    assert!(result.end_line <= 101.min(result.total_lines));
    println!(
        "open_document({}, default): lines {}-{} of {} (truncated={})",
        first, result.start_line, result.end_line, result.total_lines, result.truncated
    );
}

// ─── search → find → open 체인 시뮬레이션 ──────────────────────────────────

#[test]
fn test_search_find_open_chain() {
    let index = ensure_index();

    // Step 1: search
    let output = index.search_raw("chapter", 3).expect("search failed");
    assert!(!output.results.is_empty());
    let top = &output.results[0];
    println!(
        "SEARCH: top result = {} (score: {:.2})",
        top.filepath, top.score
    );

    // Step 2: find within top result
    let find_result = knowledge_agent::find_in_document(
        &top.filepath,
        &top.content,
        "chapter",
        0,
        ToolConfig::default().max_matches,
    );
    assert!(!find_result.matches.is_empty());
    let first_match = &find_result.matches[0];
    println!(
        "FIND: {} matches, first at {}",
        find_result.matches.len(),
        first_match.start
    );

    // Step 3: open around the match
    let open_result = knowledge_agent::open_document(
        &top.filepath,
        &top.content,
        Some(first_match.start.line),
        Some(first_match.end.line + 10),
        ToolConfig::default().max_content_chars,
        ToolConfig::default().max_lines_per_open,
    );
    println!(
        "OPEN: lines {}-{}\n{}",
        open_result.start_line,
        open_result.end_line,
        open_result
            .content
            .lines()
            .take(5)
            .collect::<Vec<_>>()
            .join("\n")
    );

    println!("\nFull chain: search → find → open completed successfully");
}

// ─── ToolRuntime 통합 테스트 ─────────────────────────────────────────────────

#[tokio::test]
async fn test_find_tool() {
    let index = Arc::new(ensure_index());
    let tool_set = build_tool_set(
        index.clone(),
        &ToolConfig::default(),
        vec![std::path::PathBuf::from(books_dir())],
    );

    let tool = tool_set
        .get("find_in_document")
        .expect("find tool not found");
    let desc = tool.desc();
    assert_eq!(desc.name, "find_in_document");
    println!(
        "Tool: {} — {}",
        desc.name,
        desc.description.as_deref().unwrap_or("")
    );

    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let args = ailoy::to_value!({
        "filepath": first.clone(),
        "query": "chapter"
    });
    let tool_call = Part::function("find_in_document", args);
    let result_msg = tool.run(tool_call).await.expect("tool run failed");
    let result_val = result_msg.contents[0].as_value().expect("expected value");
    let json_val = serde_json::to_value(result_val).unwrap();
    println!(
        "find_in_document result: {}",
        serde_json::to_string_pretty(&json_val).unwrap()
    );

    assert!(json_val["total_lines"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_open_tool() {
    let index = Arc::new(ensure_index());
    let tool_set = build_tool_set(
        index.clone(),
        &ToolConfig::default(),
        vec![std::path::PathBuf::from(books_dir())],
    );

    let tool = tool_set.get("open_document").expect("open tool not found");
    let desc = tool.desc();
    assert_eq!(desc.name, "open_document");

    let paths = index.indexed_filepaths().expect("failed to get filepaths");
    let first = paths.iter().next().expect("no indexed docs");

    let args = ailoy::to_value!({
        "filepath": first.clone(),
        "start_line": 1,
        "end_line": 10
    });
    let tool_call = Part::function("open_document", args);
    let result_msg = tool.run(tool_call).await.expect("tool run failed");
    let result_val = result_msg.contents[0].as_value().expect("expected value");
    let json_val = serde_json::to_value(result_val).unwrap();
    println!(
        "open_document result: {}",
        serde_json::to_string_pretty(&json_val).unwrap()
    );

    assert_eq!(json_val["start_line"].as_u64().unwrap(), 1);
}

#[tokio::test]
async fn test_search_find_open_tool_chain() {
    let index = Arc::new(ensure_index());
    let tool_set = build_tool_set(
        index.clone(),
        &ToolConfig::default(),
        vec![std::path::PathBuf::from(books_dir())],
    );

    let search_tool = tool_set.get("search_document").expect("search tool");
    let find_tool = tool_set.get("find_in_document").expect("find tool");
    let open_tool = tool_set.get("open_document").expect("open tool");

    // Step 1: search
    let args = ailoy::to_value!({ "query": "love", "top_k": 3 });
    let result_msg = search_tool
        .run(Part::function("search_document", args))
        .await
        .expect("search failed");
    let result_val = result_msg.contents[0].as_value().expect("value");
    let json = serde_json::to_value(result_val).unwrap();
    let results = json["results"].as_array().expect("results array");
    assert!(!results.is_empty());

    let filepath = results[0]["filepath"].as_str().expect("filepath");
    println!("SEARCH → filepath: {}", filepath);

    // Step 2: find
    let args = ailoy::to_value!({ "filepath": filepath, "query": "love" });
    let result_msg = find_tool
        .run(Part::function("find_in_document", args))
        .await
        .expect("find failed");
    let result_val = result_msg.contents[0].as_value().expect("value");
    let json = serde_json::to_value(result_val).unwrap();
    let matches = json["matches"].as_array().expect("matches");
    assert!(!matches.is_empty());

    let first_line = matches[0]["start"]["line"].as_u64().unwrap();
    println!(
        "FIND → {} matches, first at line {}",
        matches.len(),
        first_line
    );

    // Step 3: open
    let ctx_start = matches[0]["context_start"].as_u64().unwrap();
    let ctx_end = matches[0]["context_end"].as_u64().unwrap();
    let args = ailoy::to_value!({ "filepath": filepath, "start_line": ctx_start, "end_line": ctx_end + 10 });
    let result_msg = open_tool
        .run(Part::function("open_document", args))
        .await
        .expect("open failed");
    let result_val = result_msg.contents[0].as_value().expect("value");
    let json = serde_json::to_value(result_val).unwrap();
    println!("OPEN → lines {}-{}", json["start_line"], json["end_line"]);

    println!("\nTool chain: search → find → open completed successfully");
}
