use ailoy::{
    datatype::Value,
    to_value,
    tool::{ToolDesc, ToolDescBuilder, ToolFunc},
    tool_func,
};
use uuid::Uuid;

use crate::knowledge_base::SharedStore;

pub fn get_read_document_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("read_document")
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
        .build()
}

pub fn get_read_document_tool_func(store: SharedStore) -> ToolFunc {
    tool_func!(async |args: Value| -> Value with [store = store.clone()] {
        let id_str = match args.pointer("/id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return to_value!({"error": "missing required parameter: id"}),
        };
        let id = match Uuid::parse_str(&id_str) {
            Ok(id) => id,
            Err(_) => return to_value!({"error": "invalid document ID"}),
        };
        let offset = match args.pointer("/offset").and_then(|v| v.as_integer()) {
            Some(v) => v.max(0) as usize,
            None => return to_value!({"error": "missing required parameter: offset"}),
        };
        let len = match args.pointer("/len").and_then(|v| v.as_integer()) {
            Some(v) => v.max(1) as usize,
            None => return to_value!({"error": "missing required parameter: len"}),
        };

        let guard = store.read().await;
        match guard.read(id, offset, len) {
            Some(content) => Value::from(content),
            None => to_value!({"error": format!("document not found: {id_str}")}),
        }
    })
}
