use std::{fs, path::Path, sync::Once, time::Instant};

use knowledge_agent::{IndexSettings, SearchIndex, ToolConfig, check_or_build_index};

const INDEX_DIR: &str = "/tmp/knowledge_agent_find_comparison_index";

fn books_dir() -> String {
    let raw = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/settings.json"))
        .expect("Cannot read settings.json");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("Invalid settings.json");
    v["data"]["txt_dir"].as_str().unwrap().to_string()
}

static INIT_INDEX: Once = Once::new();

fn ensure_index() -> SearchIndex {
    INIT_INDEX.call_once(|| {
        let settings = IndexSettings { schema_version: 1, no_merge: false };
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

struct FindTestCase {
    book_id: &'static str,
    query: &'static str,
    label: &'static str,
}

const TEST_CASES: &[FindTestCase] = &[
    FindTestCase {
        book_id: "B03",
        query: "Scarlett",
        label: "single keyword",
    },
    FindTestCase {
        book_id: "B03",
        query: "Scarlett husband",
        label: "two keywords",
    },
    FindTestCase {
        book_id: "B62",
        query: "love",
        label: "common word in large doc",
    },
    FindTestCase {
        book_id: "B24",
        query: "Mr Phillips told Matthew smartest scholar school",
        label: "long query",
    },
    FindTestCase {
        book_id: "B03",
        query: "revenue|income|earnings",
        label: "regex OR pattern",
    },
    FindTestCase {
        book_id: "B62",
        query: r"\blove\b",
        label: "regex word boundary",
    },
    FindTestCase {
        book_id: "B20",
        query: "kill(ed|ing|s)?",
        label: "regex morphology",
    },
    FindTestCase {
        book_id: "B14",
        query: "lift eyes",
        label: "two keywords in classic",
    },
];

#[test]
fn test_find_substring_vs_regex() {
    let index = ensure_index();
    let config = ToolConfig::default();

    println!("\n{:=<70}", "");
    println!("  find_in_document test (regex default, substring fallback)");
    println!("{:=<70}\n", "");
    println!("{:<30} {:>8} {:>10} {:>10}", "label", "found", "us", "ms");
    println!("{:-<70}", "");

    let mut report_items = Vec::new();

    let indexed_paths = index.indexed_filepaths().expect("failed to get filepaths");

    for tc in TEST_CASES {
        let filepath = match indexed_paths.iter().find(|p| p.contains(tc.book_id)) {
            Some(p) => p.clone(),
            None => {
                println!("  {} — doc not found, skipping", tc.book_id);
                continue;
            }
        };
        let doc = index.get_document(&filepath).expect("get_document failed");

        let t0 = Instant::now();
        let result = knowledge_agent::find_in_document(
            &filepath,
            &doc.content,
            tc.query,
            0,
            config.max_matches,
        );
        let elapsed_us = t0.elapsed().as_micros();

        println!(
            "{:<30} {:>8} {:>10} {:>10.2}",
            tc.label,
            result.total_found,
            elapsed_us,
            elapsed_us as f64 / 1000.0,
        );

        report_items.push(serde_json::json!({
            "label": tc.label,
            "filepath": filepath,
            "query": tc.query,
            "total_found": result.total_found,
            "returned": result.returned,
            "elapsed_us": elapsed_us,
        }));
    }

    println!("{:=<70}\n", "");

    let report = serde_json::json!({
        "test": "find_in_document_regex_default",
        "cases": report_items,
    });
    let path = format!(
        "{}/test_report_find_comparison.json",
        env!("CARGO_MANIFEST_DIR")
    );
    fs::write(&path, serde_json::to_string_pretty(&report).unwrap())
        .expect("failed to write report");
    println!("Report: {}", path);
}
