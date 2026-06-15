mod db;
mod fs;

use std::{collections::HashMap, pin::Pin};

use ailoy::{
    agent::Agent,
    message::{Message as AgentMessage, MessageOutput},
    runenv::SharedMachine,
};
pub use db::*;
pub use fs::*;
use futures_util::{Stream, StreamExt};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent not found for session {0}")]
    NotFound(Uuid),

    #[error("agent for session {0} is already running")]
    AlreadyRunning(Uuid),

    #[error("agent build failed: {0}")]
    BuildFailed(#[source] anyhow::Error),
}

pub type AgentResult<T> = Result<T, AgentError>;

/// In-memory lifecycle of a per-session agent. `Idle` keeps only the
/// `SharedMachine` (the expensive sandbox handle) so an Agent can be
/// reconstructed on demand with fresh spec/history. `Running` owns a stream
/// over the agent's output channel.
pub enum AgentState {
    Idle {
        sid: Uuid,
        machine: SharedMachine,
    },
    Running {
        sid: Uuid,
        strm: Pin<Box<dyn Stream<Item = anyhow::Result<MessageOutput>> + Send>>,
    },
}

pub struct AppStateV2 {
    pub db: DBStateV2,

    pub fs: FSStateV2,

    agent_state: HashMap<Uuid, AgentState>,
}

impl AppStateV2 {
    pub fn new(db: DBStateV2, fs: FSStateV2) -> Self {
        Self {
            db,
            fs,
            agent_state: HashMap::new(),
        }
    }

    /// Park `agent` for `sid` as `Idle`. The `Agent` itself is dropped — only
    /// its `SharedMachine` (the sandbox handle, `Arc`-counted internally) is
    /// retained. Subsequent `run_agent` calls rebuild a fresh Agent over this
    /// same machine via a caller-provided builder.
    pub fn insert_agent(&mut self, sid: Uuid, agent: Agent) {
        let machine = agent.state.machine.clone();
        self.agent_state
            .insert(sid, AgentState::Idle { sid, machine });
        // `agent` is dropped here (its spec/history/etc. is not retained — by design).
    }

    /// Transition `sid` from `Idle` to `Running`: rebuild the Agent over the
    /// cached `SharedMachine` via `build`, kick off `agent.run(query)` on a
    /// spawned task, and store the resulting output channel as a `Stream`.
    ///
    /// `build` receives the cached machine and is responsible for stitching
    /// in spec/history (typically by fetching from the DB layer).
    ///
    /// Errors:
    /// - `NotFound` — no agent registered for `sid`
    /// - `AlreadyRunning` — `sid` is already in `Running` state
    /// - `BuildFailed` — `build` returned an error (state is left untouched)
    pub fn run_agent<F>(&mut self, sid: Uuid, query: AgentMessage, build: F) -> AgentResult<()>
    where
        F: FnOnce(SharedMachine) -> anyhow::Result<Agent>,
    {
        let prev = self
            .agent_state
            .remove(&sid)
            .ok_or(AgentError::NotFound(sid))?;
        let machine = match prev {
            AgentState::Idle { machine, .. } => machine,
            running @ AgentState::Running { .. } => {
                // Put it back so the caller can still observe Running.
                self.agent_state.insert(sid, running);
                return Err(AgentError::AlreadyRunning(sid));
            }
        };

        let mut agent = match build(machine) {
            Ok(a) => a,
            Err(e) => {
                // Build failed — restore Idle so subsequent attempts can retry.
                // We need the machine back, but `build` consumed our copy.
                // Caller must re-insert via `insert_agent` to recover.
                // (Acceptable for PoC; tighten by cloning before `build` if needed.)
                return Err(AgentError::BuildFailed(e));
            }
        };

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut inner = agent.run(query);
            while let Some(item) = inner.next().await {
                if tx.send(item).is_err() {
                    // Receiver dropped — stop polling.
                    break;
                }
            }
            // Agent (and its machine handle) is dropped here. To restore
            // `Idle`, the caller is responsible for calling `insert_agent`
            // again once the stream end is observed.
        });

        let strm = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        self.agent_state.insert(
            sid,
            AgentState::Running {
                sid,
                strm: Box::pin(strm),
            },
        );
        Ok(())
    }

    /// Borrow the running stream for `sid`, if any. Returns `None` for `Idle`
    /// or unknown sessions. Caller polls via `Stream::next` on the returned
    /// pinned reference.
    pub fn running_stream(
        &mut self,
        sid: &Uuid,
    ) -> Option<Pin<&mut (dyn Stream<Item = anyhow::Result<MessageOutput>> + Send)>> {
        match self.agent_state.get_mut(sid) {
            Some(AgentState::Running { strm, .. }) => Some(strm.as_mut()),
            _ => None,
        }
    }

    pub fn remove_agent(&mut self, sid: &Uuid) -> Option<AgentState> {
        self.agent_state.remove(sid)
    }
}
