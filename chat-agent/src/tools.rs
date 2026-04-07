//! Main-agent tool definitions.
//!
//! Contains the default tools (`web_search`) and the `read_source`
//! tool. These are tools used directly by the parent ChatAgent — **not** by
//! speedwagon sub-agents (which live in `speedwagon/`).
//!
//! ## Adding a new tool
//!
//! 1. Define `build_X_tool(...) -> Option<(String, ToolRuntime)>` in this module
//!    (or a new module for complex tools — see `speedwagon/dispatch.rs`).
//! 2. Add the call to `build_tool_set()` in `chat_agent.rs`.
//!    Names and runtimes are collected together, so no separate registration step.
//!
//! This convention is intentionally simple for the current scale (4 tools).
//! A trait-based plugin system may replace it when dynamic tool loading is needed.

use std::path::PathBuf;
use std::sync::Arc;

use crate::error_value;

use ailoy::agent::ToolFunc;
use ailoy::{ToolDescBuilder, ToolRuntime, ToolSet, Value};

pub const READ_SOURCE_TOOL: &str = "read_source";

// ---------------------------------------------------------------------------
// Default tool set (web_search)
// ---------------------------------------------------------------------------

pub fn build_default_tool_set() -> ToolSet {
    ToolSet::new().with_builtin("web_search")
}

// ---------------------------------------------------------------------------
// read_source tool
// ---------------------------------------------------------------------------

/// Build the `read_source` tool from a list of (source_id, source_name, file_path) tuples.
/// Returns `None` if source_paths is empty.
pub fn build_read_source_tool(
    source_paths: Vec<(String, String, PathBuf)>,
) -> Option<(String, ToolRuntime)> {
    if source_paths.is_empty() {
        return None;
    }
    let desc = read_source_desc(&source_paths);
    let func = read_source_func(source_paths);
    Some((READ_SOURCE_TOOL.to_string(), ToolRuntime::new(desc, func)))
}

fn read_source_desc(source_paths: &[(String, String, PathBuf)]) -> ailoy::ToolDesc {
    let source_list = source_paths
        .iter()
        .map(|(id, name, _)| format!("- \"{id}\": {name}"))
        .collect::<Vec<_>>()
        .join("\n");

    let enum_values: Vec<Value> = source_paths
        .iter()
        .map(|(id, _, _)| Value::string(id))
        .collect();

    ToolDescBuilder::new(READ_SOURCE_TOOL)
        .description(format!(
            "Read the raw content of a source file by its ID.\n\
             Available sources:\n{source_list}"
        ))
        .parameters(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([(
                    "source_id",
                    Value::object([
                        ("type", Value::string("string")),
                        ("description", Value::string("ID of the source to read")),
                        ("enum", Value::array(enum_values)),
                    ]),
                )]),
            ),
            ("required", Value::array([Value::string("source_id")])),
        ]))
        .build()
}

fn read_source_func(source_paths: Vec<(String, String, PathBuf)>) -> Arc<ToolFunc> {
    Arc::new(move |args: Value| {
        let source_paths = source_paths.clone();
        Box::pin(async move {
            let args_map = match args.as_object() {
                Some(m) => m,
                None => return error_value("invalid_arguments"),
            };

            let source_id = match args_map.get("source_id").and_then(Value::as_str) {
                Some(s) => s.to_string(),
                None => return error_value("missing source_id"),
            };

            let file_path = match source_paths.iter().find(|(id, _, _)| *id == source_id) {
                Some((_, _, path)) => path.clone(),
                None => return error_value(&format!("unknown source_id: {source_id}")),
            };

            match tokio::fs::read_to_string(&file_path).await {
                Ok(content) => Value::object([("content", Value::string(content))]),
                Err(e) => error_value(&format!("failed to read file: {e}")),
            }
        })
    })
}
