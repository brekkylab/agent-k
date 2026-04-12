use uuid::Uuid;

use crate::models::Agent;
use crate::repository::{RepositoryError, normalize_spec};
use crate::state::AppState;

/// Find an existing agent with the same normalized spec, or create a new one.
///
/// Returns `(agent, created)` where `created` is true if a new agent was inserted.
///
/// NOTE: Not atomic — concurrent identical requests may create duplicates.
/// Acceptable for dev stage single-user; revisit with UNIQUE constraint for production.
pub async fn find_or_create_agent(
    state: &AppState,
    spec: ailoy::AgentSpec,
) -> Result<(Agent, bool), RepositoryError> {
    let normalized = normalize_spec(&spec)?;
    if let Some(existing) = state.repository.find_agent_by_spec(&normalized).await? {
        return Ok((existing, false));
    }
    let agent = state.repository.create_agent(spec).await?;
    Ok((agent, true))
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
        _ => {} // has sessions or error — skip
    }
}
