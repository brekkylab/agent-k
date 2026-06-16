// use std::{collections::HashMap, sync::Arc};

// use ailoy::{
//     agent::{Agent, AgentSpec, AgentState},
//     message::{Message, MessageOutput},
//     runenv::Sandbox,
// };
// use futures_util::StreamExt;
// use thiserror::Error;
// use tokio::sync::{Mutex, broadcast};
// use uuid::Uuid;

// #[derive(Debug, Error)]
// pub enum AgentError {
//     #[error("agent not found for session {0}")]
//     NotFound(Uuid),

//     #[error("agent for session {0} is already running")]
//     AlreadyRunning(Uuid),

//     #[error("agent build failed: {0}")]
//     BuildFailed(#[source] anyhow::Error),
// }

// pub type AgentResult<T> = Result<T, AgentError>;

// /// In-memory lifecycle of a per-session agent.
// ///
// /// * `Idle` parks the spec, message history, and live sandbox handle.
// /// * `Active` is held while an [`Agent`] is running on a spawned task.
// ///   Subscribers observe streamed outputs by calling `tx.subscribe()`. The
// ///   task automatically decomposes the agent back into `Idle` when the stream
// ///   ends.
// enum SessionAgentState {
//     Idle {
//         spec: AgentSpec,

//         history: Vec<Message>,

//         runenv: Arc<Mutex<Sandbox>>,
//     },

//     Active {
//         agent: Agent,
//     },
// }

// /// Per-session agent state machine.
// pub struct SessionsStateV2 {
//     runs: Arc<Mutex<HashMap<Uuid, SessionAgentState>>>,
// }

// const BROADCAST_CAPACITY: usize = 64;

// impl Default for SessionsStateV2 {
//     fn default() -> Self {
//         Self::new()
//     }
// }

// impl SessionsStateV2 {
//     pub fn new() -> Self {
//         Self {
//             runs: Arc::new(Mutex::new(HashMap::new())),
//         }
//     }

//     /// Make agent usable
//     pub async fn insert(
//         &self,
//         sid: Uuid,
//         spec: AgentSpec,
//         runenv: Arc<Mutex<Sandbox>>,
//         history: impl IntoIterator<Item = Message>,
//     ) -> &mut Agent {
//         todo!()
//         // let mut runs = self.runs.lock().await;
//         // runs.insert(
//         //     sid,
//         //     AgentRun::Idle {
//         //         spec,
//         //         history: history.into_iter().collect(),
//         //         runenv,
//         //     },
//         // );
//     }

//     /// Remove the entry for `sid`. For `Active`, the broadcast sender is
//     /// dropped; the spawned run task will find no `Active` entry when it
//     /// completes and will not restore `Idle`.
//     pub async fn inactivate(&self, sid: Uuid) {
//         self.runs.lock().await.remove(&sid);
//     }

//     /// Transition `sid` from `Idle` to `Active`: build an [`Agent`] from the
//     /// parked spec + history + sandbox handle (via the `"default"` provider),
//     /// drive its `run(query)` on a spawned task, and return a
//     /// `broadcast::Receiver` for the `MessageOutput` stream. When the stream
//     /// ends, the task decomposes the agent and restores `Idle`.
//     ///
//     /// Errors:
//     /// - `NotFound` — no entry for `sid`.
//     /// - `AlreadyRunning` — entry is `Active` (left unchanged).
//     /// - `BuildFailed` — agent construction failed. The entry is left
//     ///   removed; the caller must re-insert to recover.
//     pub async fn run(
//         &self,
//         sid: Uuid,
//         query: Message,
//     ) -> AgentResult<broadcast::Receiver<MessageOutput>> {
//         // Extract Idle. On any other state, put the entry back and report.
//         let (spec, history, runenv) = {
//             let mut runs = self.runs.lock().await;
//             let prev = runs.remove(&sid).ok_or(AgentError::NotFound(sid))?;
//             match prev {
//                 AgentRun::Idle {
//                     spec,
//                     history,
//                     runenv,
//                 } => (spec, history, runenv),
//                 other @ AgentRun::Active { .. } => {
//                     runs.insert(sid, other);
//                     return Err(AgentError::AlreadyRunning(sid));
//                 }
//             }
//         };

//         // Build the agent via the `"default"` provider. Clone spec + runenv
//         // so we can re-park them as `Idle` after the run finishes — the
//         // originals are consumed by `try_with_state` and the agent's
//         // internal `AgentState`.
//         let agent_state = AgentState::new()
//             .with_runenv(runenv.clone())
//             .with_history(history);
//         let mut agent =
//             Agent::try_with_state(spec.clone(), agent_state).map_err(AgentError::BuildFailed)?;

//         let (tx, rx_for_caller) = broadcast::channel::<MessageOutput>(BROADCAST_CAPACITY);

//         // Mark Active.
//         self.runs
//             .lock()
//             .await
//             .insert(sid, AgentRun::Active { tx: tx.clone() });

//         // Drive the agent's stream on a spawned task; restore Idle when done.
//         let runs_handle = self.runs.clone();
//         tokio::spawn(async move {
//             {
//                 let mut stream = agent.run(query);
//                 while let Some(item) = stream.next().await {
//                     match item {
//                         Ok(out) => {
//                             // Lagging/closed receivers are ignored — emitting
//                             // continues so history is still accumulated.
//                             let _ = tx.send(out);
//                         }
//                         Err(e) => {
//                             tracing::warn!(?sid, error = %e, "agent run yielded error");
//                         }
//                     }
//                 }
//             }

//             let history = std::mem::take(&mut agent.state.history);
//             drop(agent);

//             let mut runs = runs_handle.lock().await;
//             // Only restore if we're still the Active entry (the caller may
//             // have removed us, or replaced via insert, in the meantime).
//             if !matches!(runs.get(&sid), Some(AgentRun::Active { .. })) {
//                 return;
//             }
//             runs.insert(
//                 sid,
//                 AgentRun::Idle {
//                     spec,
//                     history,
//                     runenv,
//                 },
//             );
//         });

//         Ok(rx_for_caller)
//     }
// }
