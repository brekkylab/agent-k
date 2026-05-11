//! `read_source` tool — read the raw content of a session source file by ID.

use std::path::PathBuf;
use std::sync::Arc;

use ailoy::agent::ToolAsyncFunc;
use ailoy::{ToolDescBuilder, ToolRuntime, Value};

use crate::error_value;

pub const READ_SOURCE_TOOL: &str = "read_source";

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
    Some((
        READ_SOURCE_TOOL.to_string(),
        ToolRuntime::new_async(desc, func),
    ))
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

fn read_source_func(source_paths: Vec<(String, String, PathBuf)>) -> Arc<ToolAsyncFunc> {
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
