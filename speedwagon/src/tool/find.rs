use std::sync::Arc;

use ailoy::{
    datatype::Value,
    message::{ToolDesc, ToolDescBuilder},
    to_value,
    tool::ToolFunc,
};
use uuid::Uuid;

use crate::store::{FindResult, Store};

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
    let relaxation = match result.relaxation {
        Some(r) => Value::from(r),
        None => Value::Null,
    };
    to_value!({
        "next_cursor": next_cursor,
        "relaxation": relaxation,
        "matches": Value::Array(matches),
    })
}

pub fn build_find_in_document_tool(store: Arc<Store>) -> (ToolDesc, ToolFunc) {
    let desc = ToolDescBuilder::new("find_in_document")
        .description(concat!(
            "Find occurrences of a query within a document. Matching is line-oriented and ",
            "case-insensitive; one match is reported per matching line. ",
            "Query syntax (subset of structured query syntax): ",
            "bare words (e.g. `revenue cost`) are treated as keywords joined by AND, ",
            "with progressive fallback — if no line has all of them, the tool retries with ",
            "≥half, then ≥one. The fallback level (\"all\"/\"half\"/\"any\") is reported in ",
            "`relaxation`; null means no fallback was needed or the query used explicit operators. ",
            "Use `\"phrase\"` for an exact phrase, `+term`/`-term` (or `NOT term`) to require/exclude, ",
            "`AND`/`OR` for boolean combinations, `(group)` for grouping, and `/regex/` for a regex ",
            "literal — any of these disables the bare-word fallback so the query is evaluated as written. ",
            "Returns matches with byte offsets and surrounding context bytes. ",
            "Paginate by passing `next_cursor` back as `cursor`; null means no more results.",
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
                    "description": "Query string. Bare words (e.g. `revenue cost`) get AND→half→any fallback. Use `\"phrase\"`, `+term`, `-term`, `AND`, `OR`, `NOT`, `(group)`, or `/regex/` for explicit semantics (no fallback)."
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
                "relaxation": {
                    "type": ["string", "null"],
                    "description": "Fallback level used for bare-word queries: 'all' (every keyword on the matched line), 'half' (≥half), 'any' (≥one). Null when no fallback was applied or the query used explicit operators."
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

            match store.find(id, &pattern, cursor, k, context_bytes) {
                Some(result) => result_to_value(&result),
                None => to_value!({"error": format!("document not found: {id_str}")}),
            }
        }
    });

    (desc, func)
}
