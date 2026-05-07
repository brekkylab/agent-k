use ailoy::message::Message;
use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    auth::Role,
    repository::{DbProject, DbSession, DbUser, NewUser, RepositoryError, RepositoryResult, UpdateUser},
};

pub struct SqliteRepository {
    pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn now_string() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn parse_uuid(s: String, field: &str) -> RepositoryResult<Uuid> {
        Uuid::parse_str(&s)
            .map_err(|_| RepositoryError::InvalidData(format!("invalid uuid in {field}")))
    }

    fn parse_timestamp(s: String, field: &str) -> RepositoryResult<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| RepositoryError::InvalidData(format!("invalid timestamp in {field}")))
    }

    fn parse_role(s: String, field: &str) -> RepositoryResult<Role> {
        match s.as_str() {
            "user" => Ok(Role::User),
            "admin" => Ok(Role::Admin),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid role '{s}' in {field}"
            ))),
        }
    }

    fn map_db_error(e: sqlx::Error, unique_field: &str) -> RepositoryError {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.message().contains("UNIQUE constraint failed") {
                return RepositoryError::UniqueViolation(unique_field.to_string());
            }
        }
        RepositoryError::Database(e)
    }

    // ── Sessions ──────────────────────────────────────────────────────────────

    fn row_to_db_session(row: sqlx::sqlite::SqliteRow) -> RepositoryResult<DbSession> {
        let share_mode_str: String = row.get("share_mode");
        let share_mode = crate::repository::ShareMode::from_str(&share_mode_str)
            .ok_or_else(|| crate::repository::RepositoryError::InvalidData(
                format!("invalid share_mode: {share_mode_str}"),
            ))?;
        Ok(DbSession {
            id: Self::parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            project_id: Self::parse_uuid(row.get::<String, _>("project_id"), "sessions.project_id")?,
            creator_id: Self::parse_uuid(row.get::<String, _>("creator_id"), "sessions.creator_id")?,
            share_mode,
            created_at: Self::parse_timestamp(row.get::<String, _>("created_at"), "sessions.created_at")?,
            updated_at: Self::parse_timestamp(row.get::<String, _>("updated_at"), "sessions.updated_at")?,
        })
    }

    pub async fn create_session(
        &self,
        project_id: Uuid,
        creator_id: Uuid,
    ) -> RepositoryResult<DbSession> {
        let id = Uuid::new_v4();
        let now = Self::now_string();
        sqlx::query(
            "INSERT INTO sessions (id, project_id, creator_id, share_mode, created_at, updated_at) \
             VALUES (?, ?, ?, 'private', ?, ?);",
        )
        .bind(id.to_string())
        .bind(project_id.to_string())
        .bind(creator_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(DbSession {
            id,
            project_id,
            creator_id,
            share_mode: crate::repository::ShareMode::Private,
            created_at: Self::parse_timestamp(now.clone(), "sessions.created_at")?,
            updated_at: Self::parse_timestamp(now, "sessions.updated_at")?,
        })
    }

    pub async fn get_session(&self, id: Uuid) -> RepositoryResult<Option<DbSession>> {
        let row = sqlx::query(
            "SELECT id, project_id, creator_id, share_mode, created_at, updated_at \
             FROM sessions WHERE id = ?;",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else { return Ok(None) };
        Ok(Some(Self::row_to_db_session(row)?))
    }

    /// Returns (DbSession, SessionAccess) for the requesting user, or None if no access.
    /// Private sessions of other users return None (403 semantics handled by caller).
    pub async fn get_session_with_authz(
        &self,
        session_id: Uuid,
        requesting_user_id: Uuid,
    ) -> RepositoryResult<Option<(DbSession, crate::repository::SessionAccess)>> {
        let uid = requesting_user_id.to_string();
        let sid = session_id.to_string();

        let row = sqlx::query(
            "SELECT s.id, s.project_id, s.creator_id, s.share_mode, s.created_at, s.updated_at,
                    CASE
                        WHEN s.creator_id = ?1 THEN 'creator'
                        WHEN (p.owner_id = ?1 OR pm.user_id IS NOT NULL)
                             AND s.share_mode = 'shared_chat' THEN 'chat_member'
                        WHEN (p.owner_id = ?1 OR pm.user_id IS NOT NULL)
                             AND s.share_mode = 'shared_readonly' THEN 'readonly_member'
                        ELSE NULL
                    END AS access_level
             FROM sessions s
             JOIN projects p ON p.id = s.project_id
             LEFT JOIN project_members pm
                   ON pm.project_id = s.project_id AND pm.user_id = ?1
             WHERE s.id = ?2",
        )
        .bind(&uid)
        .bind(&sid)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else { return Ok(None) };

        let access_level: Option<String> = row.get("access_level");
        let access = match access_level.as_deref() {
            Some("creator") => crate::repository::SessionAccess::Creator,
            Some("chat_member") => crate::repository::SessionAccess::ChatMember,
            Some("readonly_member") => crate::repository::SessionAccess::ReadOnlyMember,
            _ => return Ok(None),
        };

        Ok(Some((Self::row_to_db_session(row)?, access)))
    }

    pub async fn list_sessions_in_project(
        &self,
        project_id: Uuid,
        requesting_user_id: Uuid,
    ) -> RepositoryResult<Vec<DbSession>> {
        let pid = project_id.to_string();
        let uid = requesting_user_id.to_string();

        let rows = sqlx::query(
            "SELECT s.id, s.project_id, s.creator_id, s.share_mode, s.created_at, s.updated_at
             FROM sessions s
             JOIN projects p ON p.id = s.project_id
             WHERE s.project_id = ?1
               AND (
                   s.creator_id = ?2
                   OR (
                       s.share_mode != 'private'
                       AND (
                           p.owner_id = ?2
                           OR EXISTS (SELECT 1 FROM project_members pm
                                      WHERE pm.project_id = ?1 AND pm.user_id = ?2)
                       )
                   )
               )
             ORDER BY s.created_at DESC",
        )
        .bind(&pid)
        .bind(&uid)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(Self::row_to_db_session)
            .collect()
    }

    pub async fn update_session_share_mode(
        &self,
        session_id: Uuid,
        share_mode: &crate::repository::ShareMode,
    ) -> RepositoryResult<DbSession> {
        let now = Self::now_string();
        let result = sqlx::query(
            "UPDATE sessions SET share_mode = ?, updated_at = ? WHERE id = ?",
        )
        .bind(share_mode.as_str())
        .bind(&now)
        .bind(session_id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(crate::repository::RepositoryError::InvalidData(
                format!("session {session_id} not found"),
            ));
        }

        self.get_session(session_id).await?.ok_or_else(|| {
            crate::repository::RepositoryError::InvalidData("session disappeared after update".into())
        })
    }

    pub async fn delete_session(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?;")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn append_messages(
        &self,
        session_id: Uuid,
        messages: &[Message],
    ) -> RepositoryResult<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let now = Self::now_string();
        let sid = session_id.to_string();

        for msg in messages {
            let msg_json = serde_json::to_string(msg)?;
            sqlx::query(
                "INSERT INTO session_messages (session_id, message_json, created_at) \
                 VALUES (?, ?, ?);",
            )
            .bind(&sid)
            .bind(&msg_json)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        }

        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?;")
            .bind(&now)
            .bind(&sid)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn clear_messages(&self, session_id: Uuid) -> RepositoryResult<()> {
        sqlx::query("DELETE FROM session_messages WHERE session_id = ?;")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_messages(&self, session_id: Uuid) -> RepositoryResult<Vec<Message>> {
        let rows = sqlx::query(
            "SELECT message_json FROM session_messages \
             WHERE session_id = ? ORDER BY seq ASC;",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let json = row.get::<String, _>("message_json");
                serde_json::from_str::<Message>(&json).map_err(RepositoryError::Serialization)
            })
            .collect()
    }

    // ── Users ─────────────────────────────────────────────────────────────────

    fn row_to_db_user(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbUser> {
        Ok(DbUser {
            id: Self::parse_uuid(row.get::<String, _>("id"), "users.id")?,
            username: row.get::<String, _>("username"),
            password_hash: row.get::<String, _>("password_hash"),
            role: Self::parse_role(row.get::<String, _>("role"), "users.role")?,
            display_name: row.get::<Option<String>, _>("display_name"),
            is_active: row.get::<i64, _>("is_active") != 0,
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "users.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "users.updated_at",
            )?,
        })
    }

    pub async fn create_user(&self, user: NewUser) -> RepositoryResult<DbUser> {
        let now = Self::now_string();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, display_name, is_active, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?);",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(user.role.as_str())
        .bind(&user.display_name)
        .bind(if user.is_active { 1i64 } else { 0i64 })
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| Self::map_db_error(e, "username"))?;

        Ok(DbUser {
            id: user.id,
            username: user.username,
            password_hash: user.password_hash,
            role: user.role,
            display_name: user.display_name,
            is_active: user.is_active,
            created_at: Self::parse_timestamp(now.clone(), "users.created_at")?,
            updated_at: Self::parse_timestamp(now, "users.updated_at")?,
        })
    }

    pub async fn get_user_by_id(&self, id: Uuid) -> RepositoryResult<Option<DbUser>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, created_at, updated_at \
             FROM users WHERE id = ?;",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_db_user).transpose()
    }

    pub async fn get_user_by_username(&self, username: &str) -> RepositoryResult<Option<DbUser>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, created_at, updated_at \
             FROM users WHERE username = ?;",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_db_user).transpose()
    }

    pub async fn list_users(&self, page: u32, size: u32) -> RepositoryResult<(Vec<DbUser>, i64)> {
        let size = size.min(100) as i64;
        let offset = ((page.saturating_sub(1)) as i64) * size;

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users;")
            .fetch_one(&self.pool)
            .await?;

        let rows = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, created_at, updated_at \
             FROM users ORDER BY created_at ASC LIMIT ? OFFSET ?;",
        )
        .bind(size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let users = rows
            .iter()
            .map(Self::row_to_db_user)
            .collect::<RepositoryResult<Vec<_>>>()?;

        Ok((users, total))
    }

    pub async fn update_user(
        &self,
        id: Uuid,
        update: UpdateUser,
    ) -> RepositoryResult<Option<DbUser>> {
        let now = Self::now_string();

        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE users SET updated_at = ");
        builder.push_bind(&now);

        if let Some(ref dn) = update.display_name {
            builder.push(", display_name = ").push_bind(dn);
        }
        if let Some(ref ph) = update.password_hash {
            builder.push(", password_hash = ").push_bind(ph);
        }
        if let Some(ref role) = update.role {
            builder.push(", role = ").push_bind(role.as_str());
        }
        if let Some(active) = update.is_active {
            builder
                .push(", is_active = ")
                .push_bind(if active { 1i64 } else { 0i64 });
        }

        builder.push(" WHERE id = ").push_bind(id.to_string());

        let result = builder.build().execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_user_by_id(id).await
    }

    pub async fn delete_user(&self, id: Uuid) -> RepositoryResult<bool> {
        let uid = id.to_string();
        // Delete sessions created by the user in projects they do not own
        // (project-owned sessions cascade when the project is deleted below).
        sqlx::query("DELETE FROM sessions WHERE creator_id = ?;")
            .bind(&uid)
            .execute(&self.pool)
            .await?;
        // Delete projects owned by the user (FK is RESTRICT, not CASCADE).
        // project_members and sessions in those projects cascade automatically.
        sqlx::query("DELETE FROM projects WHERE owner_id = ?;")
            .bind(&uid)
            .execute(&self.pool)
            .await?;

        let result = sqlx::query("DELETE FROM users WHERE id = ?;")
            .bind(&uid)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn count_admins(&self) -> RepositoryResult<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = 1;",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    // ── Projects ──────────────────────────────────────────────────────────────────

    fn row_to_db_project(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbProject> {
        Ok(DbProject {
            id: Self::parse_uuid(row.get::<String, _>("id"), "projects.id")?,
            name: row.get("name"),
            description: row.get("description"),
            owner_id: Self::parse_uuid(row.get::<String, _>("owner_id"), "projects.owner_id")?,
            created_at: Self::parse_timestamp(row.get::<String, _>("created_at"), "projects.created_at")?,
            updated_at: Self::parse_timestamp(row.get::<String, _>("updated_at"), "projects.updated_at")?,
        })
    }

    pub async fn create_project(
        &self,
        name: String,
        description: Option<String>,
        owner_id: Uuid,
    ) -> RepositoryResult<DbProject> {
        let id = Uuid::new_v4();
        let now = Self::now_string();
        sqlx::query(
            "INSERT INTO projects (id, name, description, owner_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&name)
        .bind(&description)
        .bind(owner_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(DbProject {
            id,
            name,
            description,
            owner_id,
            created_at: Self::parse_timestamp(now.clone(), "projects.created_at")?,
            updated_at: Self::parse_timestamp(now, "projects.updated_at")?,
        })
    }

    pub async fn get_project(&self, id: Uuid) -> RepositoryResult<Option<DbProject>> {
        let row = sqlx::query(
            "SELECT id, name, description, owner_id, created_at, updated_at \
             FROM projects WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_db_project).transpose()
    }

    pub async fn list_projects_for_user(&self, user_id: Uuid) -> RepositoryResult<Vec<DbProject>> {
        let uid = user_id.to_string();
        let rows = sqlx::query(
            "SELECT DISTINCT p.id, p.name, p.description, p.owner_id, p.created_at, p.updated_at
             FROM projects p
             LEFT JOIN project_members pm ON pm.project_id = p.id AND pm.user_id = ?1
             WHERE p.owner_id = ?1 OR pm.user_id IS NOT NULL
             ORDER BY p.created_at ASC",
        )
        .bind(&uid)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_db_project).collect()
    }

    pub async fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        description: Option<Option<String>>,
    ) -> RepositoryResult<DbProject> {
        let now = Self::now_string();
        let current = self
            .get_project(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData(format!("project {id} not found")))?;

        let new_name = name.unwrap_or(current.name);
        let new_desc = description.unwrap_or(current.description);

        sqlx::query(
            "UPDATE projects SET name = ?, description = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&new_name)
        .bind(&new_desc)
        .bind(&now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        self.get_project(id).await?.ok_or_else(|| {
            RepositoryError::InvalidData("project disappeared after update".into())
        })
    }

    pub async fn delete_project(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Project Members ───────────────────────────────────────────────────────────

    pub async fn add_project_member(
        &self,
        project_id: Uuid,
        user_id: Uuid,
    ) -> RepositoryResult<()> {
        let now = Self::now_string();
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, added_at) VALUES (?, ?, ?)",
        )
        .bind(project_id.to_string())
        .bind(user_id.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| Self::map_db_error(e, "project_members.user_id"))?;
        Ok(())
    }

    pub async fn remove_project_member(
        &self,
        project_id: Uuid,
        user_id: Uuid,
    ) -> RepositoryResult<bool> {
        let result = sqlx::query(
            "DELETE FROM project_members WHERE project_id = ? AND user_id = ?",
        )
        .bind(project_id.to_string())
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_project_members(
        &self,
        project_id: Uuid,
    ) -> RepositoryResult<Vec<(DbUser, chrono::DateTime<chrono::Utc>)>> {
        let rows = sqlx::query(
            "SELECT u.id, u.username, u.password_hash, u.role, u.display_name, u.is_active,
                    u.created_at, u.updated_at, pm.added_at
             FROM project_members pm
             JOIN users u ON u.id = pm.user_id
             WHERE pm.project_id = ?
             ORDER BY pm.added_at ASC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|r| {
                let added_at = Self::parse_timestamp(r.get::<String, _>("added_at"), "pm.added_at")?;
                let user = Self::row_to_db_user(&r)?;
                Ok((user, added_at))
            })
            .collect()
    }

    pub async fn user_in_project(
        &self,
        user_id: Uuid,
        project_id: Uuid,
    ) -> RepositoryResult<bool> {
        let row = sqlx::query(
            "SELECT 1 FROM projects WHERE id = ? AND owner_id = ?
             UNION ALL
             SELECT 1 FROM project_members WHERE project_id = ? AND user_id = ?
             LIMIT 1",
        )
        .bind(project_id.to_string())
        .bind(user_id.to_string())
        .bind(project_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn user_is_project_owner(
        &self,
        user_id: Uuid,
        project_id: Uuid,
    ) -> RepositoryResult<bool> {
        let row = sqlx::query("SELECT 1 FROM projects WHERE id = ? AND owner_id = ?")
            .bind(project_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn create_user_with_personal_project(
        &self,
        new_user: NewUser,
    ) -> RepositoryResult<(DbUser, DbProject)> {
        let user = self.create_user(new_user).await?;
        let project = self.create_project("Personal".to_string(), None, user.id).await?;
        Ok((user, project))
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
