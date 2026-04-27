use std::sync::Arc;

use ailoy::{
    datatype::Value,
    message::ToolDescBuilder,
    to_value,
    tool::{ToolFactory, ToolFunc},
};
use uuid::Uuid;

use crate::store::Store;

pub fn build_read_document_tool(store: Arc<Store>) -> ToolFactory {
    let desc = ToolDescBuilder::new("read_document")
        .description(concat!(
            "Read a byte range of a document's content. ",
            "Use 'offset' and 'len' to page through large documents. ",
            "Byte offsets from find_in_document results can be used to target specific sections.",
        ))
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "ID of the document to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset to start reading from"
                },
                "len": {
                    "type": "integer",
                    "description": "Number of bytes to read"
                }
            },
            "required": ["id", "offset", "len"]
        }))
        .returns(to_value!({
            "type": "string",
            "description": "The document content for the requested byte range"
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
            let offset = match args.pointer("/offset").and_then(|v: &Value| v.as_integer()) {
                Some(v) => v.max(0) as usize,
                None => return to_value!({"error": "missing required parameter: offset"}),
            };
            let len = match args.pointer("/len").and_then(|v: &Value| v.as_integer()) {
                Some(v) => v.max(1) as usize,
                None => return to_value!({"error": "missing required parameter: len"}),
            };

            match store.read(id, offset, len) {
                Some(content) => Value::from(content),
                None => to_value!({"error": format!("document not found: {id_str}")}),
            }
        }
    });

    ToolFactory::simple(desc, func)
}
