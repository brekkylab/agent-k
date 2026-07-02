use ailoy::agent::AgentSpec;
use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};

/// A reusable, workspace-scoped agent definition.
///
/// Where a [`Session`](super::Session) is a single running conversation, an
/// `Agent` is the persistent *template* it is built from: a named, editable
/// [`AgentSpec`] that lives inside a workspace and can be reused to start many
/// sessions. Deleting the owning workspace cascades to its agents.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: Uuid,

    pub workspace_id: Uuid,

    /// Human label, unique within the owning workspace.
    pub name: String,

    /// Optional free-text description shown in listings.
    pub description: Option<String>,

    /// Whether this agent is enabled for use (e.g. selectable when starting a
    /// session). Disabled agents are kept but hidden from the active picker.
    pub active: bool,

    /// The agent's logical identity — model, instruction, tools, sub-agents.
    pub spec: AgentSpec,

    /// Whether this agent should run with a backbone run environment
    /// (sandbox). Mirrors [`Session::runenv`](super::Session::runenv).
    pub runenv: bool,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

impl Agent {
    pub fn new(workspace_id: Uuid, name: impl Into<String>, spec: AgentSpec) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            description: None,
            active: true,
            spec,
            runenv: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_spec(mut self, spec: AgentSpec) -> Self {
        self.spec = spec;
        self
    }

    pub fn with_runenv(mut self, runenv: bool) -> Self {
        self.runenv = runenv;
        self
    }

    pub fn with_updated_at(mut self) -> Self {
        self.updated_at = Utc::now();
        self
    }

    fn from_sqlite_row(row: &SqliteRow) -> StateResult<Self> {
        let spec_raw: String = row.get("spec");
        let spec: AgentSpec = serde_json::from_str(&spec_raw)
            .map_err(|e| StateError::InvalidData(format!("agents.spec: {e}")))?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "agents.id")?,
            workspace_id: parse_uuid(
                row.get::<String, _>("workspace_id"),
                "agents.workspace_id",
            )?,
            name: row.get("name"),
            description: row.get("description"),
            active: row.get("active"),
            spec,
            runenv: row.get("runenv"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "agents.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "agents.updated_at")?,
        })
    }
}

const SELECT_COLUMNS: &str =
    "id, workspace_id, name, description, active, spec, runenv, created_at, updated_at";

pub struct AgentsState {
    db: SqlitePool,
}

impl AgentsState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn list_by_workspace(&self, workspace_id: Uuid) -> StateResult<Vec<Agent>> {
        let rows = sqlx::query(&format!(
            "SELECT {SELECT_COLUMNS} FROM agents WHERE workspace_id = ? ORDER BY created_at ASC"
        ))
        .bind(workspace_id.to_string())
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(Agent::from_sqlite_row).collect()
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<Agent>> {
        let row = sqlx::query(&format!("SELECT {SELECT_COLUMNS} FROM agents WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(&self.db)
            .await?;
        row.as_ref().map(Agent::from_sqlite_row).transpose()
    }

    /// Insert or update by `id`, persisting the spec as JSON. Returns the prior
    /// row if one was overwritten, `None` if freshly inserted. A name that
    /// collides with another agent in the same workspace surfaces as
    /// [`StateError::UniqueViolation`].
    pub async fn upsert(&self, item: Agent) -> StateResult<Option<Agent>> {
        let prior = self.get(item.id).await?;
        sqlx::query(
            "INSERT INTO agents \
                 (id, workspace_id, name, description, active, spec, runenv, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 name = excluded.name, \
                 description = excluded.description, \
                 active = excluded.active, \
                 spec = excluded.spec, \
                 runenv = excluded.runenv, \
                 updated_at = excluded.updated_at",
        )
        .bind(item.id.to_string())
        .bind(item.workspace_id.to_string())
        .bind(&item.name)
        .bind(&item.description)
        .bind(item.active)
        .bind(serde_json::to_string(&item.spec)?)
        .bind(item.runenv)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await
        .map_err(map_sqlx_error)?;
        Ok(prior)
    }

    pub async fn remove(&self, id: Uuid) -> StateResult<Agent> {
        let existing = self.get(id).await?.ok_or(StateError::NotFound)?;
        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(existing)
    }
}

/// Map a SQLite UNIQUE violation on `(workspace_id, name)` to a typed error so
/// the router can answer `409 Conflict`. Everything else passes through.
fn map_sqlx_error(e: sqlx::Error) -> StateError {
    if let sqlx::Error::Database(ref db_err) = e {
        if db_err
            .code()
            .map(|c| c == "2067" || c == "1555")
            .unwrap_or(false)
            || db_err.message().contains("UNIQUE")
        {
            return StateError::UniqueViolation("agents.name".to_string());
        }
    }
    StateError::Sqlx(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed_workspace(pool: &SqlitePool) -> Uuid {
        let wid = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO workspaces (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(wid.to_string())
            .bind("W")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        wid
    }

    #[tokio::test]
    async fn agent_crud_round_trip() {
        let pool = fresh_db().await;
        let workspace_id = seed_workspace(&pool).await;
        let state = AgentsState::new(pool);

        let spec = AgentSpec::new("anthropic/claude-sonnet-4-5");
        let agent = Agent::new(workspace_id, "researcher", spec).with_description("does research");
        let id = agent.id;

        assert!(state.upsert(agent.clone()).await.unwrap().is_none());

        let fetched = state.get(id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "researcher");
        assert!(fetched.active);
        assert_eq!(fetched.spec.model, "anthropic/claude-sonnet-4-5");

        let bumped = fetched.clone().with_name("researcher v2").with_updated_at();
        let prior = state.upsert(bumped).await.unwrap().expect("prior row");
        assert_eq!(prior.name, "researcher");
        assert_eq!(state.get(id).await.unwrap().unwrap().name, "researcher v2");

        assert_eq!(state.list_by_workspace(workspace_id).await.unwrap().len(), 1);

        let removed = state.remove(id).await.unwrap();
        assert_eq!(removed.id, id);
        assert!(state.get(id).await.unwrap().is_none());
        assert!(matches!(state.remove(id).await, Err(StateError::NotFound)));
    }

    #[tokio::test]
    async fn duplicate_name_in_workspace_conflicts() {
        let pool = fresh_db().await;
        let workspace_id = seed_workspace(&pool).await;
        let state = AgentsState::new(pool);

        let spec = AgentSpec::new("anthropic/claude-sonnet-4-5");
        state
            .upsert(Agent::new(workspace_id, "dup", spec.clone()))
            .await
            .unwrap();

        let clash = Agent::new(workspace_id, "dup", spec);
        assert!(matches!(
            state.upsert(clash).await,
            Err(StateError::UniqueViolation(_))
        ));
    }
}
