use std::sync::Arc;

use ailoy::{ToolDescBuilder, ToolRuntime, Value, agent::ToolFunc};
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::json;

use super::{
    common::{extract_optional_i64, extract_required_str, result_to_value},
    search::SearchIndex,
};

#[derive(Debug, Clone, Serialize)]
pub struct OpenResult {
    pub filepath: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub truncated: bool,
    pub content: String,
}

/// Read a range of lines from document content.
///
/// Content is truncated at config.max_content_chars to prevent token explosion.
pub fn open_document(
    filepath: &str,
    content: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
    max_content_chars: usize,
    max_lines_per_open: usize,
) -> OpenResult {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(start + 100).min(total_lines);
    let end = end.min(start + max_lines_per_open - 1);

    let start_idx = (start - 1).min(total_lines);
    let end_idx = end.min(total_lines);

    let mut out = String::new();
    let mut truncated = false;
    let mut actual_end = start_idx;

    for (i, line) in lines[start_idx..end_idx].iter().enumerate() {
        let formatted = format!("{}: {}\n", start + i, line);
        if out.len() + formatted.len() > max_content_chars {
            truncated = true;
            break;
        }
        out.push_str(&formatted);
        actual_end = start_idx + i + 1;
    }

    if truncated {
        out.push_str(&format!(
            "\n[truncated at {} chars — use a smaller line range]",
            max_content_chars
        ));
    }

    OpenResult {
        filepath: filepath.to_string(),
        start_line: start,
        end_line: actual_end,
        total_lines,
        truncated,
        content: out,
    }
}

pub fn build_open_document_tool(
    index: Arc<SearchIndex>,
    max_content_chars: usize,
    max_lines_per_open: usize,
) -> ToolRuntime {
    let desc = ToolDescBuilder::new("open_document")
            .description(
                "Read a range of lines from a specific document. \
                 Returns line-numbered content. \
                 Use filepath from search_document results and line numbers from find_in_document. \
                 Keep ranges small (20-40 lines) to be efficient."
            )
            .parameters(json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string", "description": "File path from search results" },
                    "start_line": { "type": "integer", "description": "Start line (1-based, default 1)" },
                    "end_line": { "type": "integer", "description": "End line (default start+100)" }
                },
                "required": ["filepath"]
            }))
            .build();

    let idx = index.clone();
    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let idx = idx.clone();
        Box::pin(async move {
            let filepath = match extract_required_str(&args, "filepath") {
                Ok(d) => d,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let start_line = extract_optional_i64(&args, "start_line").map(|v| v.max(1) as usize);
            let end_line = extract_optional_i64(&args, "end_line").map(|v| v.max(1) as usize);

            match idx.get_document(&filepath) {
                Ok(doc) => {
                    let result = open_document(
                        &filepath,
                        &doc.content,
                        start_line,
                        end_line,
                        max_content_chars,
                        max_lines_per_open,
                    );
                    result_to_value(&result)
                }
                Err(e) => json!({ "error": e.to_string() }).into(),
            }
        })
    });

    ToolRuntime::new(desc, f)
}
