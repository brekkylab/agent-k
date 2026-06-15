//! Active-run registry for per-session agent runs.
//!
//! Exists so that:
//! - `POST /sessions/{id}/messages` can atomically claim a session for a new
//!   run and reject overlapping requests with HTTP 409.
//! - Late-joining WS clients can replay all outputs emitted so far, and learn
//!   whether the run has already ended (so the UI can leave the streaming
//!   state).
//!
//! A `Run` is the unit of work; a `Registry` is just the index keyed by
//! session id. Once you hold an `Arc<Run>` you don't need to look it up
//! again — the lifecycle (`append`/`end`/`snapshot`) is on `Run` itself.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ailoy::message::MessageOutput;
use dashmap::{DashMap, mapref::entry::Entry};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::events::RunUserMessage;

/// A single agent run. Outputs are streamed in via `append` and the run is
/// marked terminal via `end`. After `end` the buffer is kept around for late
/// WS joiners until the registry entry is replaced or forgotten.
pub struct Run {
    pub run_id: Uuid,
    pub user_message: RunUserMessage,
    /// `true` once `end` has been called. Atomic so `is_running` is lock-free
    /// for the hot path (`Registry::try_claim`'s 409 check).
    ended: AtomicBool,
    outputs: RwLock<Vec<MessageOutput>>,
}

impl Run {
    fn new(user_message: RunUserMessage) -> Self {
        Self {
            run_id: Uuid::new_v4(),
            user_message,
            ended: AtomicBool::new(false),
            outputs: RwLock::new(Vec::new()),
        }
    }

    /// Lock-free check used by `Registry::try_claim` to gate concurrent runs
    /// and by callers that just want to know "is this run still streaming?".
    pub fn is_running(&self) -> bool {
        !self.ended.load(Ordering::Acquire)
    }

    /// Append an output; returns the assigned `seq` (zero-based, dense).
    pub async fn append(&self, output: MessageOutput) -> u64 {
        let mut outs = self.outputs.write().await;
        let seq = outs.len() as u64;
        outs.push(output);
        seq
    }

    /// Flip the run to ended. Idempotent.
    pub fn end(&self) {
        self.ended.store(true, Ordering::Release);
    }

    /// Capture a consistent view of the run for WS replay.
    /// `outputs` are returned with their implicit seq (== index).
    pub async fn snapshot(&self) -> RunSnapshot {
        let outs = self.outputs.read().await;
        RunSnapshot {
            run_id: self.run_id,
            user_message: self.user_message.clone(),
            outputs: outs.clone(),
            ended: !self.is_running(),
        }
    }
}

pub struct RunSnapshot {
    pub run_id: Uuid,
    pub user_message: RunUserMessage,
    pub outputs: Vec<MessageOutput>,
    /// `true` if the run has already terminated. Lets WS subscribe decide
    /// whether to follow up with a synthetic `AgentRunDone` after replay.
    pub ended: bool,
}

/// Returned by `Registry::try_claim` when a still-running run holds the
/// session. Maps to HTTP 409.
#[derive(Debug)]
pub struct AlreadyRunning;

#[derive(Default)]
pub struct Registry {
    runs: DashMap<Uuid, Arc<Run>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim the session for a new run. Returns the claimed `Run` so the
    /// caller can `append`/`end` it directly without re-looking-up by sid.
    ///
    /// - No entry → insert and return.
    /// - Entry exists but `is_running() == false` (Waiting) → replace and return.
    /// - Entry exists and is still running → `Err(AlreadyRunning)`.
    ///
    /// The check + replace happen under the same shard lock, so two concurrent
    /// `POST /messages` cannot both succeed.
    pub fn try_claim(
        &self,
        session_id: Uuid,
        user_message: RunUserMessage,
    ) -> Result<Arc<Run>, AlreadyRunning> {
        match self.runs.entry(session_id) {
            Entry::Occupied(mut occ) => {
                if occ.get().is_running() {
                    return Err(AlreadyRunning);
                }
                let run = Arc::new(Run::new(user_message));
                *occ.get_mut() = run.clone();
                Ok(run)
            }
            Entry::Vacant(vac) => {
                let run = Arc::new(Run::new(user_message));
                vac.insert(run.clone());
                Ok(run)
            }
        }
    }

    /// Look up the current run for a session (running or already ended).
    /// `None` means no run has ever happened, or the slot was `forget`-ten.
    pub fn get(&self, session_id: &Uuid) -> Option<Arc<Run>> {
        self.runs.get(session_id).map(|e| e.value().clone())
    }

    /// Drop the entry for a session. Used by `delete_session` and the agent
    /// idle evictor so memory doesn't accumulate forever for inactive sessions.
    pub fn forget(&self, session_id: &Uuid) {
        self.runs.remove(session_id);
    }
}
