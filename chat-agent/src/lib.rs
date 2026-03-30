//! `chat-agent` provides a focused wrapper for constructing a chat runtime.
//!
//! Current phase scope: runtime creation and user-message execution encapsulation.
//! Session management and backend integration are handled in later phases.

mod chat_agent;
pub mod speedwagon;
pub mod tools;

pub use chat_agent::{ChatAgent, ChatAgentRunError, ToolCallEntry};
pub use speedwagon::{KbEntry, SubAgentProvider};
