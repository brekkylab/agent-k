use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};

#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn new(title: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_updated_at(mut self) -> Self {
        self.updated_at = Utc::now();
        self
    }

    fn from_sqlite_row(row: &SqliteRow) -> StateResult<Self> {
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "projects.id")?,
            title: row.get("title"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "projects.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "projects.updated_at")?,
        })
    }
}

pub struct ProjectsState {
    db: SqlitePool,
}

impl ProjectsState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> StateResult<Vec<Project>> {
        let rows = sqlx::query(
            "SELECT id, title, created_at, updated_at FROM projects ORDER BY created_at ASC",
        )
        .fetch_all(&self.db)
        .await?;
        rows.iter().map(Project::from_sqlite_row).collect()
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<Project>> {
        let row =
            sqlx::query("SELECT id, title, created_at, updated_at FROM projects WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.db)
                .await?;
        row.as_ref().map(Project::from_sqlite_row).transpose()
    }

    /// Returns the prior row if one was overwritten, `None` if freshly inserted.
    pub async fn upsert(&self, item: Project) -> StateResult<Option<Project>> {
        let prior = self.get(item.id).await?;
        sqlx::query(
            "INSERT INTO projects (id, title, created_at, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                 title = excluded.title, \
                 updated_at = excluded.updated_at",
        )
        .bind(item.id.to_string())
        .bind(&item.title)
        .bind(item.created_at.to_rfc3339())
        .bind(item.updated_at.to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(prior)
    }

    pub async fn remove(&self, id: Uuid) -> StateResult<Project> {
        let existing = self.get(id).await?.ok_or(StateError::NotFound)?;
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.db)
            .await?;
        Ok(existing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_db() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn project_crud_round_trip() {
        let pool = fresh_db().await;
        let state = ProjectsState::new(pool);

        let project = Project::new("Alpha".into());
        let id = project.id;

        assert!(state.upsert(project.clone()).await.unwrap().is_none());

        let fetched = state.get(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.title, "Alpha");

        let bumped = fetched.clone().with_title("Alpha v2");
        let prior = state.upsert(bumped).await.unwrap().expect("prior row");
        assert_eq!(prior.title, "Alpha");

        let after = state.get(id).await.unwrap().unwrap();
        assert_eq!(after.title, "Alpha v2");

        assert_eq!(state.list().await.unwrap().len(), 1);

        let removed = state.remove(id).await.unwrap();
        assert_eq!(removed.id, id);
        assert!(state.get(id).await.unwrap().is_none());
        assert!(matches!(state.remove(id).await, Err(StateError::NotFound)));
    }
}
