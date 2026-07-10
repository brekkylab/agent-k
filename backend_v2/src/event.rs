use std::sync::Arc;

use ailoy::message::Message;
use dashmap::DashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

const CHANNEL_CAPACITY: usize = 1024;

/// In-process publish/subscribe keyed by string channel name.
///
/// - Channels are created lazily by [`EventQueue::subscribe`] — subscribers
///   control the channel set.
/// - [`EventQueue::publish`] is a no-op when no one is listening; the
///   delivered-count return lets producers short-circuit work that nobody
///   would observe.
/// - Each channel is a `tokio::sync::broadcast`, so it is lossy under slow
///   subscribers (`RecvError::Lagged`). Consumers that care about
///   completeness must reconcile from the source of truth on lag.
#[derive(Clone, Default)]
pub struct EventQueue {
    channels: Arc<DashMap<String, broadcast::Sender<String>>>,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to `channel`, creating it if it doesn't exist yet.
    pub fn subscribe(&self, channel: &str) -> broadcast::Receiver<String> {
        let entry = self
            .channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
        entry.subscribe()
    }

    /// Publish `payload` to `channel`. Returns the number of receivers that
    /// observed the send. `0` means nothing is listening — the channel may
    /// not even exist yet, since subscribers create channels on demand.
    pub fn publish(&self, channel: &str, payload: String) -> usize {
        let Some(sender) = self.channels.get(channel) else {
            return 0;
        };
        sender.send(payload).unwrap_or(0)
    }

    /// Drop a channel. Any live subscribers will see `RecvError::Closed` on
    /// their next `recv()` and unwind cleanly. Used on session deletion to
    /// kick attached WS clients off the now-dead session.
    pub fn remove_channel(&self, channel: &str) {
        self.channels.remove(channel);
    }
}

// channels & payloads

/// `message/{session_id}` — fanout for messages appended to a session's
/// history. Publishers (the run loop) and subscribers (the WS handler) both
/// build the name through this helper so they stay aligned.
pub fn message_channel(session_id: Uuid) -> String {
    format!("message/{session_id}")
}

/// Payload shape for the `message/{session_id}` channel. Encoded to a JSON
/// `String` before being handed to [`EventQueue::publish`]; subscribers parse
/// the same shape back out to filter by `seq`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageEvent {
    pub seq: i64,
    pub message: Message,
}

/// Run lifecycle payload on the `message/{session_id}` channel. Tagged by the
/// `run` field so payloads serialize as `{"run":"started"}`,
/// `{"run":"finished"}`, `{"run":"error","message":"..."}`. Carries no `seq`,
/// which the WS handler already skips — old WS clients are unaffected.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "run", rename_all = "snake_case")]
pub enum RunEvent {
    Started,
    Finished,
    /// Terminal failure; `message` is the formatted anyhow error chain.
    Error { message: String },
    /// Attach-time status snapshot synthesized by the SSE handler when no
    /// run is active. Never published on the channel itself.
    Idle,
}

/// Title-update payload on the `message/{session_id}` channel. Published once
/// when a session's auto-generated title is first set (see
/// [`crate::state::SessionsState::run`]). Like [`RunEvent`] it carries no
/// `seq`; the SSE handler routes it to the `title` event by the presence of
/// the `title` field, so seq-filtering clients ignore it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TitleEvent {
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_event_json() {
        let json = serde_json::to_string(&TitleEvent {
            title: "Hello".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"title":"Hello"}"#);
    }

    #[test]
    fn run_event_started_json() {
        let json = serde_json::to_string(&RunEvent::Started).unwrap();
        assert_eq!(json, r#"{"run":"started"}"#);
    }

    #[test]
    fn run_event_finished_json() {
        let json = serde_json::to_string(&RunEvent::Finished).unwrap();
        assert_eq!(json, r#"{"run":"finished"}"#);
    }

    #[test]
    fn run_event_error_json() {
        let json = serde_json::to_string(&RunEvent::Error {
            message: "x".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"run":"error","message":"x"}"#);
    }

    #[test]
    fn run_event_idle_json() {
        let json = serde_json::to_string(&RunEvent::Idle).unwrap();
        assert_eq!(json, r#"{"run":"idle"}"#);
    }

    #[test]
    fn run_event_started_round_trip() {
        let original = r#"{"run":"started"}"#;
        let event: RunEvent = serde_json::from_str(original).unwrap();
        assert!(matches!(event, RunEvent::Started));
    }
}
