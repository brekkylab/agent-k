use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    auth::Role,
    repository::{RepositoryError, RepositoryResult},
};

pub struct SqliteRepository {
    pub(crate) pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) fn now_string() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    pub(crate) fn parse_uuid(s: String, field: &str) -> RepositoryResult<Uuid> {
        Uuid::parse_str(&s)
            .map_err(|_| RepositoryError::InvalidData(format!("invalid uuid in {field}")))
    }

    pub(crate) fn parse_timestamp(s: String, field: &str) -> RepositoryResult<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| RepositoryError::InvalidData(format!("invalid timestamp in {field}")))
    }

    pub(crate) fn parse_role(s: String, field: &str) -> RepositoryResult<Role> {
        match s.as_str() {
            "user" => Ok(Role::User),
            "admin" => Ok(Role::Admin),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid role '{s}' in {field}"
            ))),
        }
    }

    pub(crate) fn map_db_error(e: sqlx::Error, unique_field: &str) -> RepositoryError {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.message().contains("UNIQUE constraint failed") {
                return RepositoryError::UniqueViolation(unique_field.to_string());
            }
        }
        RepositoryError::Database(e)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ailoy::message::{Message, Part, Role};
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::SqliteRepository;
    use crate::{
        auth::Role as UserRole,
        repository::{NewUser, UpdateUser},
    };

    async fn make_project(pool: &sqlx::SqlitePool, owner_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        let now = SqliteRepository::now_string();
        sqlx::query("INSERT INTO projects (id, name, description, owner_id, created_at, updated_at) VALUES (?, 'Test Project', NULL, ?, ?, ?)")
            .bind(id.to_string())
            .bind(owner_id.to_string())
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn make_user(repo: &SqliteRepository, username: &str) -> Uuid {
        let u = new_user(username, UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();
        id
    }

    async fn make_repo(db_url: &str) -> SqliteRepository {
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        SqliteRepository::new(pool)
    }

    fn new_user(username: &str, role: UserRole) -> NewUser {
        NewUser {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash: "hash".to_string(),
            role,
            display_name: None,
            is_active: true,
        }
    }

    #[tokio::test]
    async fn session_and_messages_survive_repository_restart() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let session_id;

        {
            let repo = make_repo(&db_url).await;
            let user_id = make_user(&repo, "testuser_restart").await;
            let project_id = make_project(&repo.pool, user_id).await;
            let session = repo.create_session(project_id, user_id).await.unwrap();
            session_id = session.id;

            let msgs = vec![
                Message::new(Role::User).with_contents([Part::text("What is 1+1?")]),
                Message::new(Role::Assistant).with_contents([Part::text("1+1 equals 2.")]),
            ];
            repo.append_messages(session_id, &msgs).await.unwrap();

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
        }

        {
            let repo = make_repo(&db_url).await;

            let session = repo.get_session(session_id).await.unwrap();
            assert!(session.is_some(), "session must survive restart");

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
            assert!(matches!(fetched[0].role, Role::User));
            assert!(matches!(fetched[1].role, Role::Assistant));

            let user_text = fetched[0]
                .contents
                .iter()
                .find_map(|p| p.as_text())
                .unwrap_or("");
            assert_eq!(user_text, "What is 1+1?");
        }
    }

    #[tokio::test]
    async fn delete_session_cascades_messages() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let user_id = make_user(&repo, "testuser_delete").await;
        let project_id = make_project(&repo.pool, user_id).await;
        let session = repo.create_session(project_id, user_id).await.unwrap();
        let session_id = session.id;

        repo.append_messages(
            session_id,
            &[Message::new(Role::User).with_contents([Part::text("hello")])],
        )
        .await
        .unwrap();

        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 1);

        let deleted = repo.delete_session(session_id).await.unwrap();
        assert!(deleted);

        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 0);
        assert!(repo.get_session(session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_messages_preserves_insertion_order() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let user_id = make_user(&repo, "testuser_order").await;
        let project_id = make_project(&repo.pool, user_id).await;
        let session = repo.create_session(project_id, user_id).await.unwrap();
        let sid = session.id;

        let batch1 = vec![
            Message::new(Role::User).with_contents([Part::text("turn1 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn1 assistant")]),
        ];
        repo.append_messages(sid, &batch1).await.unwrap();

        let batch2 = vec![
            Message::new(Role::User).with_contents([Part::text("turn2 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn2 assistant")]),
        ];
        repo.append_messages(sid, &batch2).await.unwrap();

        let all = repo.get_messages(sid).await.unwrap();
        assert_eq!(all.len(), 4);

        let texts: Vec<&str> = all
            .iter()
            .flat_map(|m| m.contents.iter().filter_map(|p| p.as_text()))
            .collect();

        assert_eq!(
            texts,
            [
                "turn1 user",
                "turn1 assistant",
                "turn2 user",
                "turn2 assistant"
            ]
        );
    }

    #[tokio::test]
    async fn create_and_get_user() {
        let repo = make_repo("sqlite::memory:").await;

        let u = new_user("alice", UserRole::User);
        let id = u.id;
        let created = repo.create_user(u).await.unwrap();

        assert_eq!(created.username, "alice");
        assert!(matches!(created.role, UserRole::User));
        assert!(created.is_active);

        let fetched = repo.get_user_by_id(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);

        let by_name = repo.get_user_by_username("alice").await.unwrap().unwrap();
        assert_eq!(by_name.id, id);
    }

    #[tokio::test]
    async fn duplicate_username_returns_unique_violation() {
        let repo = make_repo("sqlite::memory:").await;

        repo.create_user(new_user("bob", UserRole::User))
            .await
            .unwrap();

        let err = repo
            .create_user(new_user("bob", UserRole::Admin))
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::repository::RepositoryError::UniqueViolation(_)),
            "expected UniqueViolation, got {err}"
        );
    }

    #[tokio::test]
    async fn update_user_and_count_admins() {
        let repo = make_repo("sqlite::memory:").await;

        assert_eq!(repo.count_admins().await.unwrap(), 0);

        let u = new_user("carol", UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();

        repo.update_user(
            id,
            UpdateUser {
                role: Some(UserRole::Admin),
                display_name: Some("Carol".to_string()),
                password_hash: None,
                is_active: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(repo.count_admins().await.unwrap(), 1);

        let updated = repo.get_user_by_id(id).await.unwrap().unwrap();
        assert!(matches!(updated.role, UserRole::Admin));
        assert_eq!(updated.display_name.as_deref(), Some("Carol"));
    }

    #[tokio::test]
    async fn list_users_pagination() {
        let repo = make_repo("sqlite::memory:").await;

        for i in 0..5 {
            repo.create_user(new_user(&format!("user{i}"), UserRole::User))
                .await
                .unwrap();
        }

        let (page1, total) = repo.list_users(1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 3);

        let (page2, _) = repo.list_users(2, 3).await.unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn delete_user() {
        let repo = make_repo("sqlite::memory:").await;

        let u = new_user("dave", UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();

        assert!(repo.delete_user(id).await.unwrap());
        assert!(repo.get_user_by_id(id).await.unwrap().is_none());
        assert!(!repo.delete_user(id).await.unwrap());
    }
}
