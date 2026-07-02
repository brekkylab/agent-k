use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    /// Incremental text delta for the in-progress assistant turn. Ephemeral: not
    /// persisted, no seq. `delta` is the newly produced text; `cum_len` is the
    /// turn's total length (in UTF-16 code units, matching JS `String.length`)
    /// after appending it. The client uses `cum_len` to dedup/order across the
    /// replay↔live boundary: skip if already applied, slice off any overlap. The
    /// resume path (subscribe replay / load) sends the whole turn as one delta.
    /// Reconciled by the completed `AgentMessage` that follows.
    AgentDelta {
        session_id: String,
        run_id: String,
        delta: String,
        cum_len: u64,
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
