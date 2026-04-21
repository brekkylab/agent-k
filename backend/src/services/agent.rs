use uuid::Uuid;

use crate::models::Agent;
use crate::repository::RepositoryError;
use crate::state::AppState;

/// Find an existing agent with the same normalized spec, or create a new one.
///
/// Returns `(agent, created)` where `created` is true if the returned agent is
/// freshly inserted. Atomicity is guaranteed at the DB layer by
/// `UNIQUE(spec_json)` + `INSERT ... ON CONFLICT DO UPDATE RETURNING` inside
/// `create_agent`. Concurrent identical requests converge to the same row.
///
pub async fn find_or_create_agent(
    state: &AppState,
    spec: ailoy::AgentSpec,
) -> Result<(Agent, bool), RepositoryError> {
    state.repository.create_agent(spec).await
}

/// Best-effort cleanup: delete an agent if no sessions reference it anymore.
///
/// Errors are logged as warnings but never propagate — the caller's success is unaffected.
pub async fn cleanup_orphaned_agent(state: &AppState, agent_id: Uuid) {
    match state.repository.has_sessions_for_agent(agent_id).await {
        Ok(false) => {
            if let Err(e) = state.repository.delete_agent(agent_id).await {
                tracing::warn!(agent_id = %agent_id, "failed to cleanup orphaned agent: {e}");
            }
        }
        Ok(true) => {} // has sessions — skip
        Err(e) => tracing::warn!(agent_id = %agent_id, "failed to check orphan status: {e}"),
    }
}
