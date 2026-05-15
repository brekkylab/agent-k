use ailoy::message::{Message, Role};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use super::SqliteRepository;
use crate::repository::RepositoryResult;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShareMode {
    Private,
    SharedReadonly,
    SharedChat,
}

impl ShareMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShareMode::Private => "private",
            ShareMode::SharedReadonly => "shared_readonly",
            ShareMode::SharedChat => "shared_chat",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "private" => Some(ShareMode::Private),
            "shared_readonly" => Some(ShareMode::SharedReadonly),
            "shared_chat" => Some(ShareMode::SharedChat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionAccess {
    Admin, // Session Creator or Project Owner
    ChatMember,
    ReadOnlyMember,
}

#[derive(Debug, Clone)]
pub struct DbSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub creator_id: Uuid,
    pub share_mode: ShareMode,
    pub title: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SqliteRepository {
    fn row_to_db_session(row: sqlx::sqlite::SqliteRow) -> RepositoryResult<DbSession> {
        let share_mode_str: String = row.get("share_mode");
        let share_mode = ShareMode::from_str(&share_mode_str).ok_or_else(|| {
            crate::repository::RepositoryError::InvalidData(format!(
                "invalid share_mode: {share_mode_str}"
            ))
        })?;

        let last_message_at = row
            .try_get::<Option<String>, _>("last_message_at")
            .unwrap_or(None)
            .and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            });

        Ok(DbSession {
            id: Self::parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            project_id: Self::parse_uuid(
                row.get::<String, _>("project_id"),
                "sessions.project_id",
            )?,
            creator_id: Self::parse_uuid(
                row.get::<String, _>("creator_id"),
                "sessions.creator_id",
            )?,
            share_mode,
            title: row.try_get::<Option<String>, _>("title").unwrap_or(None),
            last_message_at,
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "sessions.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "sessions.updated_at",
            )?,
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
            share_mode: ShareMode::Private,
            title: None,
            last_message_at: None,
            created_at: Self::parse_timestamp(now.clone(), "sessions.created_at")?,
            updated_at: Self::parse_timestamp(now, "sessions.updated_at")?,
        })
    }

    pub async fn get_session(&self, id: Uuid) -> RepositoryResult<Option<DbSession>> {
        let row = sqlx::query(
            "SELECT id, project_id, creator_id, share_mode, title, last_message_at, \
                    created_at, updated_at \
             FROM sessions WHERE id = ?;",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else { return Ok(None) };
        Ok(Some(Self::row_to_db_session(row)?))
    }

    pub async fn get_session_with_authz(
        &self,
        session_id: Uuid,
        requesting_user_id: Uuid,
    ) -> RepositoryResult<Option<(DbSession, SessionAccess)>> {
        let uid = requesting_user_id.to_string();
        let sid = session_id.to_string();

        let row = sqlx::query(
            "SELECT s.id, s.project_id, s.creator_id, s.share_mode, s.title, \
                    s.last_message_at, s.created_at, s.updated_at,
                    CASE
                        WHEN p.owner_id = ?1 THEN 'admin'
                        WHEN s.creator_id = ?1 AND pm.user_id IS NOT NULL THEN 'admin'
                        WHEN pm.user_id IS NOT NULL
                             AND s.share_mode = 'shared_chat' THEN 'chat_member'
                        WHEN pm.user_id IS NOT NULL
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
            Some("admin") => SessionAccess::Admin,
            Some("chat_member") => SessionAccess::ChatMember,
            Some("readonly_member") => SessionAccess::ReadOnlyMember,
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
            "SELECT s.id, s.project_id, s.creator_id, s.share_mode, s.title, \
                    s.last_message_at, s.created_at, s.updated_at
             FROM sessions s
             JOIN projects p ON p.id = s.project_id
             WHERE s.project_id = ?1
               AND (
                   p.owner_id = ?2
                   OR (
                       EXISTS (SELECT 1 FROM project_members pm
                               WHERE pm.project_id = ?1 AND pm.user_id = ?2)
                       AND (s.creator_id = ?2 OR s.share_mode != 'private')
                   )
               )
             ORDER BY s.created_at DESC",
        )
        .bind(&pid)
        .bind(&uid)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_db_session).collect()
    }

    pub async fn update_session_share_mode(
        &self,
        session_id: Uuid,
        share_mode: &ShareMode,
    ) -> RepositoryResult<DbSession> {
        let now = Self::now_string();
        let result = sqlx::query("UPDATE sessions SET share_mode = ?, updated_at = ? WHERE id = ?")
            .bind(share_mode.as_str())
            .bind(&now)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(crate::repository::RepositoryError::InvalidData(format!(
                "session {session_id} not found"
            )));
        }

        self.get_session(session_id).await?.ok_or_else(|| {
            crate::repository::RepositoryError::InvalidData(
                "session disappeared after update".into(),
            )
        })
    }

    pub async fn list_all_sessions_in_project(
        &self,
        project_id: Uuid,
    ) -> RepositoryResult<Vec<DbSession>> {
        let rows = sqlx::query(
            "SELECT id, project_id, creator_id, share_mode, title, last_message_at, \
                    created_at, updated_at \
             FROM sessions WHERE project_id = ? \
             ORDER BY created_at DESC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(Self::row_to_db_session).collect()
    }

    pub async fn fork_session(
        &self,
        source_id: Uuid,
        new_id: Uuid,
        new_creator_id: Uuid,
    ) -> RepositoryResult<DbSession> {
        let source = self.get_session(source_id).await?.ok_or_else(|| {
            crate::repository::RepositoryError::InvalidData(format!(
                "source session {source_id} not found"
            ))
        })?;

        let now = Self::now_string();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO sessions (id, project_id, creator_id, share_mode, title, \
                                   created_at, updated_at) \
             VALUES (?, ?, ?, 'private', ?, ?, ?);",
        )
        .bind(new_id.to_string())
        .bind(source.project_id.to_string())
        .bind(new_creator_id.to_string())
        .bind(&source.title)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO session_messages (session_id, message_json, created_at) \
             SELECT ?, message_json, created_at \
             FROM session_messages WHERE session_id = ? ORDER BY seq ASC;",
        )
        .bind(new_id.to_string())
        .bind(source_id.to_string())
        .execute(&mut *tx)
        .await?;

        // Set last_message_at from the copied messages
        sqlx::query(
            "UPDATE sessions SET last_message_at = \
             (SELECT MAX(created_at) FROM session_messages WHERE session_id = ?) \
             WHERE id = ?",
        )
        .bind(new_id.to_string())
        .bind(new_id.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        self.get_session(new_id).await?.ok_or_else(|| {
            crate::repository::RepositoryError::InvalidData(
                "forked session disappeared after creation".into(),
            )
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

        sqlx::query("UPDATE sessions SET updated_at = ?, last_message_at = ? WHERE id = ?;")
            .bind(&now)
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
                serde_json::from_str::<Message>(&json)
                    .map_err(crate::repository::RepositoryError::Serialization)
            })
            .collect()
    }

    // ── Session metadata ────────────────────────────────────────────────────────

    /// Set title only if not already set (idempotent for concurrent title-gen spawns).
    pub async fn set_session_title(&self, session_id: Uuid, title: &str) -> RepositoryResult<()> {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ? AND title IS NULL")
            .bind(title)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Returns the text content of the first user-role message in the session.
    pub async fn get_first_user_message_text(
        &self,
        session_id: Uuid,
    ) -> RepositoryResult<Option<String>> {
        let rows = sqlx::query(
            "SELECT message_json FROM session_messages \
             WHERE session_id = ? ORDER BY seq ASC LIMIT 5",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let json: String = row.get("message_json");
            if let Ok(msg) = serde_json::from_str::<Message>(&json) {
                if matches!(msg.role, Role::User) {
                    let text = msg
                        .contents
                        .iter()
                        .find_map(|p| p.as_text())
                        .map(str::to_string);
                    return Ok(text);
                }
            }
        }
        Ok(None)
    }

    /// Mark all current messages as read for a user. Uses upsert semantics.
    pub async fn mark_session_read(&self, session_id: Uuid, user_id: Uuid) -> RepositoryResult<()> {
        let now = Self::now_string();
        sqlx::query(
            "INSERT INTO session_reads (session_id, user_id, last_read_seq, updated_at)
             VALUES (
                 ?,
                 ?,
                 (SELECT COALESCE(MAX(seq), 0) FROM session_messages WHERE session_id = ?),
                 ?
             )
             ON CONFLICT (session_id, user_id) DO UPDATE SET
                 last_read_seq = (SELECT COALESCE(MAX(seq), 0)
                                  FROM session_messages
                                  WHERE session_id = excluded.session_id),
                 updated_at = excluded.updated_at",
        )
        .bind(session_id.to_string())
        .bind(user_id.to_string())
        .bind(session_id.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Count messages the user has not yet read in a session.
    pub async fn count_session_unread(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> RepositoryResult<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM session_messages
             WHERE session_id = ?
               AND seq > COALESCE(
                   (SELECT last_read_seq FROM session_reads
                    WHERE session_id = ? AND user_id = ?),
                   0
               )",
        )
        .bind(session_id.to_string())
        .bind(session_id.to_string())
        .bind(user_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get::<i64, _>("cnt"))
    }
}
