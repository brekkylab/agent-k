use ailoy::message::MessageOutput;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMessageRequest {
    pub content: String,
}

pub type SendMessageResponse = Vec<MessageOutput>;
