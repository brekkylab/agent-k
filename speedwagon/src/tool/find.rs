use ailoy::{
    datatype::Value,
    message::ToolDescBuilder,
    to_value,
    tool::{ToolFactory, ToolFunc},
};
use uuid::Uuid;

use crate::store::{FindResult, SharedStore};

fn result_to_value(result: &FindResult) -> Value {
    let matches: Vec<Value> = result
        .matches
        .iter()
        .map(|m| {
            to_value!({
                "keyword": m.keyword.clone(),
                "start": m.start,
                "end": m.end,
                "context": m.context.clone(),
            })
        })
        .collect();
    let next_cursor = match result.next_cursor {
        Some(c) => Value::from(c),
        None => Value::Null,
    };
    to_value!({
        "next_cursor": next_cursor,
        "matches": Value::Array(matches),
    })
}

pub fn build_find_in_document_tool(store: SharedStore) -> ToolFactory {
    let desc = ToolDescBuilder::new("find_in_document")
        .description(concat!(
            "Find all occurrences of a regex pattern within a document. ",
            "Matching is always case-insensitive. ",
            "Returns matches with surrounding context and byte offsets. ",
            "Results are paginated — pass 'next_cursor' back as 'cursor' to fetch the next batch. ",
            "When 'next_cursor' is null, all matches have been returned.",
        ))
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "ID of the document to search"
                },
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "cursor": {
                    "type": "integer",
                    "description": "Pagination cursor from a previous call; omit or pass 0 to start from the beginning",
                    "default": 0
                },
                "k": {
                    "type": "integer",
                    "description": "Maximum number of matches to return per call",
                    "default": 10
                },
                "context_bytes": {
                    "type": "integer",
                    "description": "Number of bytes to include before and after each match as context",
                    "default": 256
                }
            },
            "required": ["id", "pattern"]
        }))
        .returns(to_value!({
            "type": "object",
            "properties": {
                "next_cursor": {
                    "type": ["integer", "null"],
                    "description": "Pass this value as 'cursor' in the next call to get the next batch; null means no more results"
                },
                "matches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "keyword": {
                                "type": "string",
                                "description": "The matched text"
                            },
                            "start": {
                                "type": "integer",
                                "description": "Byte offset of the match start within the document"
                            },
                            "end": {
                                "type": "integer",
                                "description": "Byte offset of the match end within the document"
                            },
                            "context": {
                                "type": "string",
                                "description": "Text surrounding the match, up to context_bytes before and after"
                            }
                        }
                    }
                }
            }
        }))
        .build();

    let func = ToolFunc::new(move |args: Value| {
        let store = store.clone();
        async move {
            let id_str = match args.pointer("/id").and_then(|v: &Value| v.as_str()) {
                Some(s) => s.to_string(),
                None => return to_value!({"error": "missing required parameter: id"}),
            };
            let id = match Uuid::parse_str(&id_str) {
                Ok(id) => id,
                Err(_) => return to_value!({"error": "invalid document ID"}),
            };
            let pattern = match args.pointer("/pattern").and_then(|v: &Value| v.as_str()) {
                Some(q) => q.to_string(),
                None => return to_value!({"error": "missing required parameter: pattern"}),
            };
            let cursor = args
                .pointer("/cursor")
                .and_then(|v: &Value| v.as_integer())
                .unwrap_or(0)
                .max(0) as usize;
            let k = args
                .pointer("/k")
                .and_then(|v: &Value| v.as_integer())
                .unwrap_or(10)
                .max(1) as usize;
            let context_bytes = args
                .pointer("/context_bytes")
                .and_then(|v: &Value| v.as_integer())
                .unwrap_or(256)
                .max(0) as usize;

            let guard = store.read().await;
            match guard.find(id, &pattern, cursor, k, context_bytes) {
                Some(result) => result_to_value(&result),
                None => to_value!({"error": format!("document not found: {id_str}")}),
            }
        }
    });

    ToolFactory::simple(desc, func)
}
