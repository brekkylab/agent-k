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
        run_id: String,
        user_message: RunUserMessage,
    },
    AgentMessage {
        session_id: String,
        run_id: String,
        seq: u64,
        output: ailoy::message::MessageOutput,
    },
    /// Live token fragment for the in-progress assistant turn. Ephemeral: not
    /// persisted and carries no seq — the client accumulates deltas for live
    /// rendering and is reconciled by the completed `AgentMessage` that follows.
    AgentDelta {
        session_id: String,
        run_id: String,
        delta: ailoy::message::MessageDeltaOutput,
    },
    AgentError {
        session_id: String,
        run_id: String,
        message: String,
    },
    AgentRunDone {
        session_id: String,
        run_id: String,
        stopped: bool,
    },
    AgentRunIdle {
        session_id: String,
    },
    /// Server-internal: signals a WS connection to drop its subscription to a session.
    /// Broadcast when a user's project membership is revoked.
    /// Task B filters this event (never forwarded to the client).
    AccessRevoked {
        session_id: String,
        user_id: String,
    },
}
