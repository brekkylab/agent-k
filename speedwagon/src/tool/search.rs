use std::sync::Arc;

use ailoy::{
    datatype::Value,
    message::{ToolDesc, ToolDescBuilder},
    to_value,
    tool::ToolFunc,
};

use crate::store::Store;

fn result_to_value<T: serde::Serialize>(result: &T) -> Value {
    let json = serde_json::to_value(result).unwrap_or(serde_json::Value::Null);
    serde_json::from_value::<Value>(json).unwrap_or(Value::Null)
}

pub fn make_search_document_tool(store: Arc<Store>, page_size: u32) -> (ToolDesc, ToolFunc) {
    let desc = ToolDescBuilder::new("search_document")
        .description(
            concat!(
                "Search for relevant documents using BM25 full-text search. ",
                "Returns a page of results ranked by relevance with filepath and score. ",
                "Use the returned filepath with find_in_document or open_document for detailed content. ",
                "Increment `page` to retrieve further results."
            ),
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "BM25 search query"
                },
                "page": {
                    "type": "integer",
                    "description": "Page number (0-indexed, default 0)"
                }
            },
            "required": ["query"]
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

            match store.search_page(&query, page, page_size) {
                Ok(output) => result_to_value(&output),
                Err(e) => to_value!({"error": e.to_string()}),
            }
        }
    });

    (desc, func)
}
