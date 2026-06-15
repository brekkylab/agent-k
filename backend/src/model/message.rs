use ailoy::message::{Message as AiloyMessage, Part as AiloyPart};
use uuid::Uuid;

pub enum Message {
    User {
        id: Uuid,
        content: Vec<AiloyPart>,
    },
    Assistant {
        thinking: Vec<String>,
        content: Vec<AiloyPart>,
        tool_call: Vec<AiloyPart>,
    },
    Tool {
        content: Vec<AiloyPart>,
    },
}

impl Message {
    pub fn from_ailoy_message(msg: AiloyMessage) -> Self {
        todo!()
    }
}
