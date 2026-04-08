//! `chat-agent` provides a focused wrapper for constructing a chat runtime.
//!
//! Current phase scope: runtime creation and user-message execution encapsulation.
//! Session management and backend integration are handled in later phases.

mod chat_agent;
pub mod speedwagon;
pub mod tools;

pub use chat_agent::{ChatAgent, ChatAgentRunError, ChatEvent, ToolCallEntry};
pub use speedwagon::{KbEntry, SubAgentProvider, SubAgentSpec};

use ailoy::Value;

/// Tool 실행 에러를 나타내는 공통 헬퍼. `{ "error": "<msg>" }` 형태의 Value를 반환.
pub(crate) fn error_value(msg: &str) -> Value {
    Value::object([("error", Value::string(msg))])
}
