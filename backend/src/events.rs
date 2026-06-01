use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunUserMessage {
    pub sender_user_id: String,
    pub content: String,
    pub attachments: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    SessionTitleUpdated {
        session_id: String,
        project_id: String,
        title: String,
    },
    AgentRunStarted {
        session_id: String,
        user_message: RunUserMessage,
    },
    AgentMessage {
        session_id: String,
        seq: u64,
        output: ailoy::message::MessageOutput,
    },
    AgentError {
        session_id: String,
        message: String,
    },
    AgentRunDone {
        session_id: String,
    },
}
