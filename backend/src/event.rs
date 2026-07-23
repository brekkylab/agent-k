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

/// `message/{session_id}` — fanout for everything happening in a session: the
/// messages appended to its history, live streaming deltas, and run lifecycle
/// transitions. Publishers (the run loop) and subscribers (the WS handler) both
/// build the name through this helper so they stay aligned.
pub fn message_channel(session_id: Uuid) -> String {
    format!("message/{session_id}")
}

/// Payload shape for the `message/{session_id}` channel, JSON-encoded before
/// being handed to [`EventQueue::publish`]. The WS handler forwards these to
/// the client verbatim; only `Message` carries a `seq` (it is the persisted,
/// catch-up-able record — `Delta` and `Run` are ephemeral and exist solely on
/// the live stream).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// A message persisted to the session's history at sequence `seq`. Mirrors
    /// the HTTP `GET /sessions/{id}/messages` item shape so catch-up over
    /// either transport is interchangeable on the client.
    Message { seq: i64, message: Message },

    /// Newly streamed top-level assistant text for the in-progress turn.
    /// Ephemeral: not persisted, no seq. `text` is the newly produced
    /// fragment; `cum_len` is the turn's total length (in UTF-16 code units,
    /// matching JS `String.length`) after appending it. The client uses
    /// `cum_len` to dedup/order across the catch-up↔live boundary: skip if
    /// already applied, slice off any overlap. The catch-up path sends the
    /// whole partial turn as one delta. Reconciled by the completed `Message`
    /// that follows.
    Delta { text: String, cum_len: u64 },

    /// Run lifecycle transition. `Started` is also re-sent by the WS catch-up
    /// when a client attaches while a run is in flight, so it reads as "a run
    /// is active", not strictly "a run just began".
    Run {
        #[serde(flatten)]
        status: RunStatus,
    },
}

/// Terminal (and initial) states of an agent run, flattened into
/// [`SessionEvent::Run`] as `{"status": "...", ...}`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunStatus {
    /// A run is in flight for this session.
    Started,
    /// The run finished on its own.
    Done,
    /// The run was cut short by a stop request.
    Stopped,
    /// The run failed; `message` is the error rendered for display.
    Error { message: String },
}
