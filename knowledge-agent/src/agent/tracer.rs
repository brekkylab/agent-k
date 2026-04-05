use serde::{Deserialize, Serialize};

/// Tool call chain의 각 단계를 기록하는 enum.
/// claude_agent_retriever.rs의 Step 패턴을 참고.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    /// LLM의 내부 추론 (thinking/chain-of-thought)
    Thinking { content: String },
    /// LLM의 중간 추론 — tool call 사이의 판단/계획
    Reasoning { content: String },
    /// LLM의 최종 텍스트 응답
    Answer { content: String },
    /// Tool 호출 (입력 포함)
    ToolCall {
        name: String,
        input: serde_json::Value,
    },
    /// Tool 결과 (출력 요약)
    ToolResult {
        name: String,
        summary: String,
        output: serde_json::Value,
    },
}

/// Tool call의 입력을 사람이 읽기 쉽게 한 줄로 요약.
pub fn summarize_tool_call(name: &str, input: &serde_json::Value) -> String {
    match name {
        "glob_document" => {
            let pattern = input["pattern"].as_str().unwrap_or("?");
            let limit = input["limit"].as_i64();
            match limit {
                Some(l) => format!("glob_document(pattern=\"{}\", limit={})", pattern, l),
                None => format!("glob_document(pattern=\"{}\")", pattern),
            }
        }
        "search_document" => {
            let query = input["query"].as_str().unwrap_or("?");
            let top_k = input["top_k"].as_i64().unwrap_or(3);
            format!("search_document(query=\"{}\", top_k={})", query, top_k)
        }
        "find_in_document" => {
            let filepath = input["filepath"].as_str().unwrap_or("?");
            let query = input["query"].as_str().unwrap_or("?");
            format!(
                "find_in_document(filepath=\"{}\", query=\"{}\")",
                filepath, query
            )
        }
        "open_document" => {
            let filepath = input["filepath"].as_str().unwrap_or("?");
            let start = input["start_line"].as_i64();
            let end = input["end_line"].as_i64();
            match (start, end) {
                (Some(s), Some(e)) => {
                    format!(
                        "open_document(filepath=\"{}\", lines={}-{})",
                        filepath, s, e
                    )
                }
                (Some(s), None) => {
                    format!("open_document(filepath=\"{}\", from={})", filepath, s)
                }
                _ => format!("open_document(filepath=\"{}\")", filepath),
            }
        }
        "run_bash" => {
            let command = input["command"].as_str().unwrap_or("?");
            let truncated = if command.len() > 80 {
                format!("{}...", &command[..80])
            } else {
                command.to_string()
            };
            format!("run_bash(command=\"{}\")", truncated)
        }
        "run_python" => {
            let code = input["code"].as_str().unwrap_or("?");
            let first_line = code.lines().next().unwrap_or("?");
            let lines = code.lines().count();
            if lines > 1 {
                format!("run_python(\"{}\" +{} lines)", first_line, lines - 1)
            } else {
                format!("run_python(\"{}\")", first_line)
            }
        }
        "calculate" => {
            let expr = input["expression"].as_str().unwrap_or("?");
            format!("calculate(\"{}\")", expr)
        }
        "summarize_document" => {
            let filepath = input["filepath"].as_str().unwrap_or("?");
            let focus = input["focus"].as_str();
            match focus {
                Some(f) => format!(
                    "summarize_document(filepath=\"{}\", focus=\"{}\")",
                    filepath, f
                ),
                None => format!("summarize_document(filepath=\"{}\")", filepath),
            }
        }
        _ => {
            format!("{}({})", name, input)
        }
    }
}

/// Tool 결과를 사람이 읽기 쉽게 한 줄로 요약.
pub fn summarize_tool_result(name: &str, output: &serde_json::Value) -> String {
    match name {
        "glob_document" => {
            let total = output["total_found"].as_u64().unwrap_or(0);
            let truncated = output["truncated"].as_bool().unwrap_or(false);
            if let Some(arr) = output["matches"].as_array() {
                let items: Vec<String> = arr
                    .iter()
                    .take(10)
                    .map(|m| format!("  {}", m["filepath"].as_str().unwrap_or("?")))
                    .collect();
                let more = if arr.len() > 10 {
                    format!("\n  ... +{} more", arr.len() - 10)
                } else {
                    String::new()
                };
                let suffix = if truncated { " [truncated]" } else { "" };
                format!("→ {} files:{}\n{}{}", total, suffix, items.join("\n"), more)
            } else {
                format!("→ {} files", total)
            }
        }
        "search_document" => {
            if let Some(arr) = output["results"].as_array() {
                let items: Vec<String> = arr
                    .iter()
                    .map(|r| {
                        let filepath = r["filepath"].as_str().unwrap_or("?");
                        let score = r["score"].as_f64().unwrap_or(0.0);
                        format!("  {}  score={:.2}", filepath, score)
                    })
                    .collect();
                format!("→ {} results:\n{}", arr.len(), items.join("\n"))
            } else {
                "→ (unexpected format)".to_string()
            }
        }
        "find_in_document" => {
            let filepath = output["filepath"].as_str().unwrap_or("?");
            let total = output["total_lines"].as_u64().unwrap_or(0);
            if let Some(matches) = output["matches"].as_array() {
                let positions: Vec<String> = matches
                    .iter()
                    .take(5)
                    .map(|m| {
                        let kw = m["keyword"].as_str().unwrap_or("?");
                        let sl = m["start"]["line"].as_u64().unwrap_or(0);
                        let sc = m["start"]["col"].as_u64().unwrap_or(0);
                        let el = m["end"]["line"].as_u64().unwrap_or(0);
                        let ec = m["end"]["col"].as_u64().unwrap_or(0);
                        format!("  \"{}\" {}:{}-{}:{}", kw, sl, sc, el, ec)
                    })
                    .collect();
                let more = if matches.len() > 5 {
                    format!("\n  ... +{} more", matches.len() - 5)
                } else {
                    String::new()
                };
                format!(
                    "→ {} matches in {} ({} lines):\n{}{}",
                    matches.len(),
                    filepath,
                    total,
                    positions.join("\n"),
                    more
                )
            } else {
                format!("→ 0 matches in {} ({} lines)", filepath, total)
            }
        }
        "open_document" => {
            let filepath = output["filepath"].as_str().unwrap_or("?");
            let start = output["start_line"].as_u64().unwrap_or(0);
            let end = output["end_line"].as_u64().unwrap_or(0);
            let total = output["total_lines"].as_u64().unwrap_or(0);
            let content_len = output["content"].as_str().map(|s| s.len()).unwrap_or(0);
            let truncated = output["truncated"].as_bool().unwrap_or(false);
            let suffix = if truncated { " [truncated]" } else { "" };
            format!(
                "→ {} lines {}-{} of {} ({} chars){}",
                filepath, start, end, total, content_len, suffix
            )
        }
        "run_bash" | "run_python" => {
            if let Some(err) = output["error"].as_str() {
                return format!("→ error: {}", err);
            }
            let exit_code = output["exit_code"].as_i64().unwrap_or(-1);
            let timed_out = output["timed_out"].as_bool().unwrap_or(false);
            let stdout_len = output["stdout"].as_str().map(|s| s.len()).unwrap_or(0);
            let stderr_len = output["stderr"].as_str().map(|s| s.len()).unwrap_or(0);
            if timed_out {
                "→ timed out".to_string()
            } else if exit_code == -1 {
                let stderr = output["stderr"].as_str().unwrap_or("");
                let preview = if stderr.len() > 100 {
                    format!("{}...", &stderr[..100])
                } else {
                    stderr.to_string()
                };
                format!("→ BLOCKED: {}", preview)
            } else {
                format!(
                    "→ exit={} stdout={} chars stderr={} chars",
                    exit_code, stdout_len, stderr_len
                )
            }
        }
        "calculate" => {
            if let Some(result) = output["result"].as_f64() {
                let expr = output["expression"].as_str().unwrap_or("?");
                format!("→ {} = {}", expr, result)
            } else {
                let err = output["error"].as_str().unwrap_or("unknown error");
                format!("→ error: {}", err)
            }
        }
        "summarize_document" => {
            if let Some(err) = output["error"].as_str() {
                format!("→ error: {}", err)
            } else {
                let filepath = output["filepath"].as_str().unwrap_or("?");
                let lines = output["original_lines"].as_u64().unwrap_or(0);
                let chunks = output["chunks_processed"].as_u64().unwrap_or(0);
                let summary_len = output["summary"].as_str().map(|s| s.len()).unwrap_or(0);
                let failed = output["chunks_failed"].as_u64().unwrap_or(0);
                let truncated = output["reduce_truncated"].as_bool().unwrap_or(false);
                let failed_note = if failed > 0 {
                    format!(" [{} chunks failed]", failed)
                } else {
                    String::new()
                };
                let trunc_note = if truncated { " [reduce truncated]" } else { "" };
                format!(
                    "→ {} ({} lines, {} chunks) summary={} chars{}{}",
                    filepath, lines, chunks, summary_len, failed_note, trunc_note
                )
            }
        }
        _ => {
            let s = serde_json::to_string(output).unwrap_or_default();
            let truncated = if s.len() > 200 {
                format!("{}...", &s[..200])
            } else {
                s
            };
            format!("→ {}", truncated)
        }
    }
}

/// Step을 콘솔에 출력.
pub fn print_step(step_num: usize, step: &Step) {
    match step {
        Step::Thinking { content } => {
            println!("  [{}] THINK: {}", step_num, truncate(content, 200));
        }
        Step::Reasoning { content } => {
            println!("  [{}] REASON: {}", step_num, truncate(content, 200));
        }
        Step::Answer { content } => {
            println!("  [{}] ANSWER: {}", step_num, content);
        }
        Step::ToolCall { name, input } => {
            println!(
                "  [{}] CALL: {}",
                step_num,
                summarize_tool_call(name, input)
            );
        }
        Step::ToolResult { name, summary, .. } => {
            let label = match name.as_str() {
                "search_document" => "RESULT_SEARCH",
                "glob_document" => "RESULT_GLOB",
                "find_in_document" => "RESULT_FIND",
                "open_document" => "RESULT_OPEN",
                "run_bash" => "RESULT_BASH",
                "run_python" => "RESULT_PYTHON",
                "calculate" => "RESULT_CALC",
                "summarize_document" => "RESULT_SUMMARY",
                _ => "RESULT",
            };
            println!("  [{}] {}: {}", step_num, label, summary);
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Find a valid char boundary at or before max
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}
