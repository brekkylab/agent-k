use std::sync::Arc;

use ailoy::{
    datatype::Value,
    message::ToolDescBuilder,
    to_value,
    tool::{ToolFactory, ToolFunc},
};

use crate::store::{SearchPage, Store};

fn result_to_value(page: &SearchPage) -> Value {
    let results: Vec<Value> = page
        .results
        .iter()
        .map(|r| {
            to_value!({
                "score": r.score as f64,
                "id": r.document.id.clone(),
                "title": r.document.title.clone(),
                "len": r.document.len,
                "content_preview": r.content_preview.clone(),
            })
        })
        .collect();
    Value::Array(results)
}

pub fn make_search_document_tool(store: Arc<Store>) -> ToolFactory {
    let desc = ToolDescBuilder::new("search_document")
        .description(concat!(
            "Search for relevant documents for a given query. ",
            "Results are ranked by relevance score. ",
            "Use the returned document ID with find_in_document or open_document for detailed content.",
        ))
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "page": {
                    "type": "integer",
                    "description": "Page number",
                    "default": 0,
                },
                "page_size": {
                    "type": "integer",
                    "description": "Page size",
                    "default": 10,
                }
            },
            "required": ["query"]
        })).returns(to_value!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "score": {
                        "type": "number",
                        "description": "Relevance score of the document for the given query. Higher scores indicate better matches."
                    },
                    "id": {
                        "type": "string",
                        "description": "Unique identifier of the document."
                    },
                    "title": {
                        "type": "string",
                        "description": "Title of the document."
                    },
                    "len": {
                        "type": "integer",
                        "description": "Total length of the document content in bytes."
                    },
                    "content_preview": {
                        "type": "string",
                        "description": "A short excerpt from the document."
                    },
                }
            }
        }))
        .build();

    let func = ToolFunc::new(move |args: Value| {
        let store = store.clone();
        async move {
            let query = match args.pointer("/query").and_then(|v: &Value| v.as_str()) {
                Some(q) => q.to_string(),
                None => {
                    return to_value!({"error": "missing required parameter: query"});
                }
            };
            let page = args
                .pointer("/page")
                .and_then(|v: &Value| v.as_integer())
                .unwrap_or(0)
                .max(0) as u32;
            let page_size = args
                .pointer("/page_size")
                .and_then(|v: &Value| v.as_integer())
                .unwrap_or(10)
                .max(1) as u32;

            match store.search(&query, page, page_size) {
                Ok(output) => result_to_value(&output),
                Err(e) => to_value!({"error": e.to_string()}),
            }
        }
    });

    ToolFactory::simple(desc, func)
}
