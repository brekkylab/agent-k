//! Durable event outbox. Producers append domain events; the automation
//! dispatcher (worker.rs) consumes undispatched rows, matches them to event
//! triggers, and fires runs. Separate from the ephemeral in-process
//! [`EventQueue`](crate::event::EventQueue), which only powers live WS streaming.

use chrono::{DateTime, Utc};
use sqlx::{Row as _, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateResult, parse_ts, parse_uuid};

// `workspace_id`/`created_at`/`dispatched_at` are stored audit fields not all
// consumed by the dispatcher yet (workspace scoping arrives with more sources).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Event {
    pub id: Uuid,
    /// Dotted domain type, e.g. `run.succeeded`, `file.added`, `knowledge.indexed`.
    pub kind: String,
    pub workspace_id: Option<Uuid>,
    pub payload: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub dispatched_at: Option<DateTime<Utc>>,
}

impl Event {
    fn from_row(row: &SqliteRow) -> StateResult<Self> {
        let payload = row
            .get::<Option<String>, _>("payload")
            .map(|s| serde_json::from_str(&s))
            .transpose()?;
        let workspace_id = row
            .get::<Option<String>, _>("workspace_id")
            .map(|s| parse_uuid(s, "events.workspace_id"))
            .transpose()?;
        let dispatched_at = row
            .get::<Option<String>, _>("dispatched_at")
            .map(|s| parse_ts(&s, "events.dispatched_at"))
            .transpose()?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "events.id")?,
            kind: row.get("type"),
            workspace_id,
            payload,
            created_at: parse_ts(&row.get::<String, _>("created_at"), "events.created_at")?,
            dispatched_at,
        })
    }
}

const EVENT_COLS: &str = "id, type, workspace_id, payload, created_at, dispatched_at";

/// A domain event to append.
pub struct NewEvent {
    pub kind: String,
    pub workspace_id: Option<Uuid>,
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct EventsState {
    db: SqlitePool,
}

impl EventsState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    /// Append an event. Best-effort producers use this; producers that mutate
    /// the DB in the same logical step should prefer [`Self::emit_tx`].
    pub async fn emit(&self, new: NewEvent) -> StateResult<Event> {
        let mut tx = self.db.begin().await?;
        let ev = Self::emit_tx(&mut tx, new).await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// Append an event inside an existing transaction (transactional outbox):
    /// the event is durable iff the surrounding domain change commits.
    pub async fn emit_tx(tx: &mut Transaction<'_, Sqlite>, new: NewEvent) -> StateResult<Event> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let payload_str = new.payload.as_ref().map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO events (id, type, workspace_id, payload, created_at, dispatched_at) \
             VALUES (?, ?, ?, ?, ?, NULL)",
        )
        .bind(id.to_string())
        .bind(&new.kind)
        .bind(new.workspace_id.map(|w| w.to_string()))
        .bind(&payload_str)
        .bind(&now)
        .execute(&mut **tx)
        .await?;
        Ok(Event {
            id,
            kind: new.kind,
            workspace_id: new.workspace_id,
            payload: new.payload,
            created_at: parse_ts(&now, "events.created_at")?,
            dispatched_at: None,
        })
    }

    /// Oldest undispatched events, up to `limit`.
    pub async fn list_undispatched(&self, limit: i64) -> StateResult<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLS} FROM events WHERE dispatched_at IS NULL ORDER BY created_at ASC LIMIT ?"
        );
        let rows = sqlx::query(&sql).bind(limit).fetch_all(&self.db).await?;
        rows.iter().map(Event::from_row).collect()
    }

    /// Mark an event dispatched (idempotent — only flips a still-NULL row).
    pub async fn mark_dispatched(&self, id: Uuid) -> StateResult<()> {
        sqlx::query("UPDATE events SET dispatched_at = ? WHERE id = ? AND dispatched_at IS NULL")
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> SqlitePool {
        let p = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&p).await.unwrap();
        p
    }

    #[tokio::test]
    async fn outbox_round_trip() {
        let store = EventsState::new(db().await);
        let ev = store
            .emit(NewEvent {
                kind: "run.succeeded".into(),
                workspace_id: None,
                payload: Some(serde_json::json!({ "x": 1 })),
            })
            .await
            .unwrap();
        assert_eq!(store.list_undispatched(10).await.unwrap().len(), 1);
        store.mark_dispatched(ev.id).await.unwrap();
        assert!(store.list_undispatched(10).await.unwrap().is_empty());
    }
}
