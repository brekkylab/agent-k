use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use knowledge_agent::{
    AgentConfig, IndexSettings, SearchIndex, Step, ToolConfig, build_agent, check_or_build_index,
    run_with_trace,
};
use serde::Deserialize;

// ─── Paths ───────────────────────────────────────────────────────────────────

fn md_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/financebench")
}

fn finance_index_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/financebench/index/e2e_react")
}

fn books_dir() -> String {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["txt_dir"].as_str().unwrap().to_string()
}

fn qa_file() -> String {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["qa_file"].as_str().unwrap().to_string()
}

fn novel_index_dir() -> PathBuf {
    PathBuf::from("/tmp/knowledge_agent_e2e_react_index")
}

fn report_path(name: &str) -> String {
    format!("{}/{}", env!("CARGO_MANIFEST_DIR"), name)
}

// ─── QA types ────────────────────────────────────────────────────────────────

const HF_BASE: &str = "https://datasets-server.huggingface.co/rows?dataset=PatronusAI%2Ffinancebench&config=default&split=train";

#[derive(Deserialize, Debug)]
struct HfResponse {
    rows: Vec<HfRow>,
}

#[derive(Deserialize, Debug)]
struct HfRow {
    row: FinanceQa,
}

#[derive(Deserialize, Debug, Clone)]
struct FinanceQa {
    financebench_id: String,
    question: String,
    answer: String,
    doc_name: String,
    company: String,
}

#[derive(Deserialize, Debug, Clone)]
struct NovelQa {
    book_id: String,
    book_title: String,
    question_id: String,
    question: String,
}

async fn fetch_finance_qa() -> Vec<FinanceQa> {
    let client = reqwest::Client::new();
    let mut all = Vec::new();
    for offset in (0..200).step_by(100) {
        let url = format!("{HF_BASE}&offset={offset}&length=100");
        let raw = client
            .get(&url)
            .send()
            .await
            .expect("HF request failed")
            .text()
            .await
            .expect("Failed to read body");
        let hf: HfResponse = serde_json::from_str(&raw).expect("Failed to parse HF JSON");
        if hf.rows.is_empty() {
            break;
        }
        all.extend(hf.rows.into_iter().map(|r| r.row));
    }
    all
}

fn load_novel_qa() -> Vec<NovelQa> {
    let raw = fs::read_to_string(qa_file()).expect("failed to read QA file");
    serde_json::from_str(&raw).expect("failed to parse QA file")
}

const DELAY_MS: u64 = 15000; // 15s between questions to stay under TPM

fn count_tool_calls(steps: &[Step]) -> (usize, usize, usize) {
    let s = steps
        .iter()
        .filter(|s| matches!(s, Step::ToolCall { name, .. } if name == "search_document"))
        .count();
    let f = steps
        .iter()
        .filter(|s| matches!(s, Step::ToolCall { name, .. } if name == "find_in_document"))
        .count();
    let o = steps
        .iter()
        .filter(|s| matches!(s, Step::ToolCall { name, .. } if name == "open_document"))
        .count();
    (s, f, o)
}

// ─── FinanceBench E2E ────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_e2e_react_financebench() {
    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env")).ok();

    // 1. Index
    let settings = IndexSettings { schema_version: 1, no_merge: false };
    check_or_build_index(&finance_index_dir(), &md_dir(), &settings, true, false)
        .expect("indexing failed");
    let search_index =
        Arc::new(SearchIndex::open(&finance_index_dir()).expect("failed to open index"));
    let indexed_docs = search_index
        .indexed_filepaths()
        .expect("failed to get filepaths");

    // 2. Fetch QA and sample 30 diverse questions
    let qa_list = fetch_finance_qa().await;
    let mut seen_companies = HashSet::new();
    let mut sample: Vec<FinanceQa> = Vec::new();
    for qa in &qa_list {
        let expected_filepath = format!("{}.md", qa.doc_name);
        if indexed_docs.contains(&expected_filepath) && seen_companies.insert(qa.company.clone()) {
            sample.push(qa.clone());
            if sample.len() >= 30 {
                break;
            }
        }
    }

    let agent_config = AgentConfig::default();
    let tools_config = ToolConfig::default();
    let corpus_dirs = vec![md_dir()];
    println!("\n{}", "═".repeat(70));
    println!(
        "  FinanceBench E2E ReAct Test — {} questions, model: {}",
        sample.len(),
        agent_config.model_name
    );
    println!("{}\n", "═".repeat(70));

    let mut results = Vec::new();
    let total_start = Instant::now();

    for (i, qa) in sample.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
        }

        println!(
            "┌─ Q{} ─────────────────────────────────────────────",
            i + 1
        );
        println!("│ {}", qa.question);
        println!(
            "│ expected doc: {} | expected answer: {}",
            qa.doc_name,
            &qa.answer[..qa.answer.len().min(80)]
        );
        println!("├─────────────────────────────────────────────────────");

        let mut agent = build_agent(
            &agent_config,
            &tools_config,
            &search_index,
            corpus_dirs.clone(),
        );
        let start = Instant::now();
        let (answer, steps) = match run_with_trace(&mut agent, &qa.question).await {
            Ok((a, s)) => (a, s),
            Err(e) => {
                println!("│ ERROR: {}", e);
                (format!("ERROR: {}", e), vec![])
            }
        };
        let elapsed = start.elapsed().as_secs_f64();
        let (sc, fc, oc) = count_tool_calls(&steps);

        // Check if the last inspected document matches the expected one
        let expected_filepath = format!("{}.md", qa.doc_name);
        let doc_hit = steps
            .iter()
            .rev()
            .find_map(|s| {
                if let Step::ToolResult { name, output, .. } = s {
                    match name.as_str() {
                        "open_document" | "find_in_document" => {
                            let fp = output["filepath"].as_str()?;
                            Some(fp == expected_filepath)
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .unwrap_or(false);

        let (gc, _, _, _) = (
            steps
                .iter()
                .filter(|s| matches!(s, Step::ToolCall { name, .. } if name == "glob_document"))
                .count(),
            sc,
            fc,
            oc,
        );

        println!("│");
        println!(
            "│ ANSWER: {}",
            &answer[..answer
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= 200)
                .last()
                .unwrap_or(0)]
        );
        println!(
            "│ doc_hit: {} | glob={} search={} find={} open={} | {:.1}s",
            doc_hit, gc, sc, fc, oc, elapsed
        );
        println!("└─────────────────────────────────────────────────────\n");

        results.push(serde_json::json!({
            "financebench_id": qa.financebench_id,
            "question": qa.question,
            "expected_doc": qa.doc_name,
            "expected_answer": qa.answer,
            "answer": answer,
            "doc_hit": doc_hit,
            "elapsed_secs": elapsed,
            "tool_counts": { "glob": gc, "search": sc, "find": fc, "open": oc },
            "total_steps": steps.len(),
            "steps": steps,
        }));
    }

    let total_elapsed = total_start.elapsed().as_secs_f64();
    let doc_hits = results
        .iter()
        .filter(|r| r["doc_hit"].as_bool() == Some(true))
        .count();

    // Summary
    println!("{}", "═".repeat(70));
    println!("  FinanceBench Summary");
    println!("{}", "─".repeat(70));
    println!(
        "  Questions: {} | Doc Hit: {}/{} ({:.0}%) | Total: {:.1}s",
        sample.len(),
        doc_hits,
        sample.len(),
        doc_hits as f64 / sample.len() as f64 * 100.0,
        total_elapsed
    );
    println!("{}", "═".repeat(70));

    let report = serde_json::json!({
        "test": "e2e_react_financebench",
        "model": agent_config.model_name,
        "system_prompt": agent_config.system_prompt,
        "total_questions": sample.len(),
        "doc_hit_count": doc_hits,
        "doc_hit_rate_pct": doc_hits as f64 / sample.len() as f64 * 100.0,
        "total_elapsed_secs": total_elapsed,
        "results": results,
    });

    let json = serde_json::to_string_pretty(&report).unwrap();
    fs::write(
        report_path("test_report_e2e_react_financebench.json"),
        &json,
    )
    .expect("failed to write report");
    println!("Report: test_report_e2e_react_financebench.json");
}

// ─── NovelQA E2E ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY"]
async fn test_e2e_react_novelqa() {
    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env")).ok();

    // 1. Index
    let bdir = books_dir();
    let settings = IndexSettings { schema_version: 1, no_merge: false };
    check_or_build_index(&novel_index_dir(), Path::new(&bdir), &settings, true, false)
        .expect("indexing failed");
    let search_index =
        Arc::new(SearchIndex::open(&novel_index_dir()).expect("failed to open index"));
    let indexed_paths = search_index
        .indexed_filepaths()
        .expect("failed to get filepaths");

    // 2. Load QA and sample 30 diverse questions from different books
    let all_qa = load_novel_qa();
    let mut seen_books = HashSet::new();
    let mut sample: Vec<NovelQa> = Vec::new();
    for qa in &all_qa {
        // Check if any indexed filepath contains the book_id
        let has_book = indexed_paths.iter().any(|p| p.contains(&qa.book_id));
        if has_book && seen_books.insert(qa.book_id.clone()) {
            sample.push(qa.clone());
            if sample.len() >= 30 {
                break;
            }
        }
    }

    let agent_config = AgentConfig::default();
    let tools_config = ToolConfig::default();
    let target_dirs = vec![PathBuf::from(books_dir())];
    println!("\n{}", "═".repeat(70));
    println!(
        "  NovelQA E2E ReAct Test — {} questions, model: {}",
        sample.len(),
        agent_config.model_name
    );
    println!("{}\n", "═".repeat(70));

    let mut results = Vec::new();
    let total_start = Instant::now();

    for (i, qa) in sample.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS)).await;
        }

        println!(
            "┌─ Q{} ─────────────────────────────────────────────",
            i + 1
        );
        println!("│ [{}] {}", qa.book_title, qa.question);
        println!("│ expected book: {} ({})", qa.book_id, qa.book_title);
        println!("├─────────────────────────────────────────────────────");

        let mut agent = build_agent(
            &agent_config,
            &tools_config,
            &search_index,
            target_dirs.clone(),
        );

        // Include book title hint in prompt so the agent knows which book to search
        let prompt = format!(
            "This question is about the book \"{}\". {}",
            qa.book_title, qa.question
        );

        let start = Instant::now();
        let (answer, steps) = match run_with_trace(&mut agent, &prompt).await {
            Ok((a, s)) => (a, s),
            Err(e) => {
                println!("│ ERROR: {}", e);
                (format!("ERROR: {}", e), vec![])
            }
        };
        let elapsed = start.elapsed().as_secs_f64();
        let (sc, fc, oc) = count_tool_calls(&steps);

        // Check if the last inspected document matches the expected book
        let book_hit = steps
            .iter()
            .rev()
            .find_map(|s| {
                if let Step::ToolResult { name, output, .. } = s {
                    match name.as_str() {
                        "open_document" | "find_in_document" => {
                            let fp = output["filepath"].as_str()?;
                            Some(fp.contains(&qa.book_id))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .unwrap_or(false);

        let gc = steps
            .iter()
            .filter(|s| matches!(s, Step::ToolCall { name, .. } if name == "glob_document"))
            .count();

        println!("│");
        println!(
            "│ ANSWER: {}",
            &answer[..answer
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= 200)
                .last()
                .unwrap_or(0)]
        );
        println!(
            "│ book_hit: {} | glob={} search={} find={} open={} | {:.1}s",
            book_hit, gc, sc, fc, oc, elapsed
        );
        println!("└─────────────────────────────────────────────────────\n");

        results.push(serde_json::json!({
            "question_id": qa.question_id,
            "book_id": qa.book_id,
            "book_title": qa.book_title,
            "question": qa.question,
            "answer": answer,
            "book_hit": book_hit,
            "elapsed_secs": elapsed,
            "tool_counts": { "glob": gc, "search": sc, "find": fc, "open": oc },
            "total_steps": steps.len(),
            "steps": steps,
        }));
    }

    let total_elapsed = total_start.elapsed().as_secs_f64();
    let book_hits = results
        .iter()
        .filter(|r| r["book_hit"].as_bool() == Some(true))
        .count();

    // Summary
    println!("{}", "═".repeat(70));
    println!("  NovelQA Summary");
    println!("{}", "─".repeat(70));
    println!(
        "  Questions: {} | Book Hit: {}/{} ({:.0}%) | Total: {:.1}s",
        sample.len(),
        book_hits,
        sample.len(),
        book_hits as f64 / sample.len() as f64 * 100.0,
        total_elapsed
    );
    println!("{}", "═".repeat(70));

    let report = serde_json::json!({
        "test": "e2e_react_novelqa",
        "model": agent_config.model_name,
        "system_prompt": agent_config.system_prompt,
        "total_questions": sample.len(),
        "book_hit_count": book_hits,
        "book_hit_rate_pct": book_hits as f64 / sample.len() as f64 * 100.0,
        "total_elapsed_secs": total_elapsed,
        "results": results,
    });

    let json = serde_json::to_string_pretty(&report).unwrap();
    fs::write(report_path("test_report_e2e_react_novelqa.json"), &json)
        .expect("failed to write report");
    println!("Report: test_report_e2e_react_novelqa.json");
}
