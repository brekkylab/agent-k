use ailoy::message::Message;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMessageRequest {
    pub content: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SendMessageResponse {
    pub messages: Vec<Message>,
    pub final_content: String,
}
