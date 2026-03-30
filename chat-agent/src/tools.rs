//! Main-agent tool definitions.
//!
//! Contains the default tools (`utc_now`, `add_integers`) and the `read_source`
//! tool. These are tools used directly by the parent ChatAgent — **not** by
//! speedwagon sub-agents (which live in `speedwagon/`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ailoy::agent::ToolFunc;
use ailoy::{ToolDescBuilder, ToolRuntime, ToolSet, Value};

pub const DEFAULT_TOOL_UTC_NOW: &str = "utc_now";
pub const DEFAULT_TOOL_ADD_INTEGERS: &str = "add_integers";
pub const READ_SOURCE_TOOL: &str = "read_source";

// ---------------------------------------------------------------------------
// Default tool set (utc_now + add_integers)
// ---------------------------------------------------------------------------

pub fn build_default_tool_set() -> ToolSet {
    let mut tool_set = ToolSet::new();
    tool_set.insert(
        DEFAULT_TOOL_UTC_NOW.to_string(),
        ToolRuntime::new(utc_now_desc(), utc_now_func()),
    );
    tool_set.insert(
        DEFAULT_TOOL_ADD_INTEGERS.to_string(),
        ToolRuntime::new(add_integers_desc(), add_integers_func()),
    );
    tool_set
}

fn utc_now_desc() -> ailoy::ToolDesc {
    ToolDescBuilder::new(DEFAULT_TOOL_UTC_NOW)
        .description("Return the current UTC Unix timestamp in seconds.")
        .parameters(Value::object([
            ("type", Value::string("object")),
            ("properties", Value::object_empty()),
        ]))
        .returns(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([(
                    "unix_seconds",
                    Value::object([("type", Value::string("number"))]),
                )]),
            ),
        ]))
        .build()
}

fn add_integers_desc() -> ailoy::ToolDesc {
    ToolDescBuilder::new(DEFAULT_TOOL_ADD_INTEGERS)
        .description("Add two integer values and return their sum.")
        .parameters(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "a",
                        Value::object([("type", Value::string("number"))]),
                    ),
                    (
                        "b",
                        Value::object([("type", Value::string("number"))]),
                    ),
                ]),
            ),
            (
                "required",
                Value::array([Value::string("a"), Value::string("b")]),
            ),
        ]))
        .returns(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "sum",
                        Value::object([("type", Value::string("number"))]),
                    ),
                    (
                        "error",
                        Value::object([("type", Value::string("string"))]),
                    ),
                ]),
            ),
        ]))
        .build()
}

fn utc_now_func() -> Arc<ToolFunc> {
    Arc::new(|_args: Value| {
        Box::pin(async move {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => Value::object([("unix_seconds", Value::unsigned(duration.as_secs()))]),
                Err(_) => Value::object([("error", Value::string("time_before_unix_epoch"))]),
            }
        })
    })
}

fn add_integers_func() -> Arc<ToolFunc> {
    Arc::new(|args: Value| Box::pin(async move { add_integers_result(args) }))
}

pub(crate) fn add_integers_result(args: Value) -> Value {
    let Some(args_map) = args.as_object() else {
        return invalid_arguments_value();
    };

    let Some(a) = args_map.get("a").and_then(Value::as_integer) else {
        return invalid_arguments_value();
    };
    let Some(b) = args_map.get("b").and_then(Value::as_integer) else {
        return invalid_arguments_value();
    };

    match a.checked_add(b) {
        Some(sum) => Value::object([("sum", Value::integer(sum))]),
        None => Value::object([("error", Value::string("overflow"))]),
    }
}

fn invalid_arguments_value() -> Value {
    Value::object([("error", Value::string("invalid_arguments"))])
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
                        (
                            "description",
                            Value::string("ID of the source to read"),
                        ),
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

fn error_value(msg: &str) -> Value {
    Value::object([("error", Value::string(msg))])
}
