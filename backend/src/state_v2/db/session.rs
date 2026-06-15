use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{DbError, DbResult, now_string, parse_ts, parse_uuid};

mod query {
    pub mod session {
        pub const INSERT: &str = "\
            INSERT INTO sessions \
                (id, project_id, creator_id, share_mode, origin, agent_type, model, created_at, updated_at) \
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

        pub const SELECT_BY_ID: &str = "\
            SELECT id, project_id, creator_id, share_mode, origin, title, \
                   last_message_at, last_message_snippet, agent_type, model, created_at, updated_at \
            FROM sessions WHERE id = ?";

        pub const LIST_BY_PROJECT: &str = "\
            SELECT id, project_id, creator_id, share_mode, origin, title, \
                   last_message_at, last_message_snippet, agent_type, model, created_at, updated_at \
            FROM sessions WHERE project_id = ? \
            ORDER BY COALESCE(last_message_at, created_at) DESC";

        pub const UPDATE: &str = "\
            UPDATE sessions \
            SET title = ?, share_mode = ?, agent_type = ?, model = ?, updated_at = ? \
            WHERE id = ?";

        /// UPDATE on `sessions` triggered by message append.
        pub const TOUCH_AFTER_MESSAGE: &str = "\
            UPDATE sessions \
            SET updated_at = ?, \
                last_message_at = (SELECT MAX(created_at) FROM session_messages WHERE session_id = ?), \
                last_message_snippet = ? \
            WHERE id = ?";

        pub const DELETE: &str = "DELETE FROM sessions WHERE id = ?";
    }

    pub mod message {
        pub const INSERT: &str = "\
            INSERT INTO session_messages \
                (session_id, message_json, created_at, sender_kind, sender_name, sender_user_id, attachments, artifacts) \
            VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
            RETURNING seq";

        pub const LIST_BY_SESSION: &str = "\
            SELECT seq, session_id, message_json, created_at, sender_kind, sender_name, sender_user_id, attachments, artifacts \
            FROM session_messages WHERE session_id = ? ORDER BY seq ASC";

        pub const DELETE_BY_SESSION: &str = "DELETE FROM session_messages WHERE session_id = ?";
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareMode {
    Private,
    SharedReadonly,
    SharedChat,
}

impl ShareMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ShareMode::Private => "private",
            ShareMode::SharedReadonly => "shared_readonly",
            ShareMode::SharedChat => "shared_chat",
        }
    }

    fn parse(s: &str) -> DbResult<Self> {
        match s {
            "private" => Ok(ShareMode::Private),
            "shared_readonly" => Ok(ShareMode::SharedReadonly),
            "shared_chat" => Ok(ShareMode::SharedChat),
            other => Err(DbError::InvalidData(format!("invalid share_mode: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOrigin {
    User,
    Automation,
}

impl SessionOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionOrigin::User => "user",
            SessionOrigin::Automation => "automation",
        }
    }

    fn parse(s: &str) -> DbResult<Self> {
        match s {
            "user" => Ok(SessionOrigin::User),
            "automation" => Ok(SessionOrigin::Automation),
            other => Err(DbError::InvalidData(format!("invalid origin: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderKind {
    User,
    Agent,
}

impl SenderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SenderKind::User => "user",
            SenderKind::Agent => "agent",
        }
    }

    fn parse(s: &str) -> DbResult<Self> {
        match s {
            "user" => Ok(SenderKind::User),
            "agent" => Ok(SenderKind::Agent),
            other => Err(DbError::InvalidData(format!(
                "invalid sender_kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub project_id: Uuid,
    pub creator_id: Uuid,
    pub share_mode: ShareMode,
    pub origin: SessionOrigin,
    pub title: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub last_message_snippet: Option<String>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    fn from_sqlite_row(row: &SqliteRow) -> DbResult<Self> {
        let share_mode_str: String = row.get("share_mode");
        let origin_str: String = row.get("origin");
        let last_message_at = row
            .get::<Option<String>, _>("last_message_at")
            .map(|s| parse_ts(&s, "sessions.last_message_at"))
            .transpose()?;
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "sessions.id")?,
            project_id: parse_uuid(row.get::<String, _>("project_id"), "sessions.project_id")?,
            creator_id: parse_uuid(row.get::<String, _>("creator_id"), "sessions.creator_id")?,
            share_mode: ShareMode::parse(&share_mode_str)?,
            origin: SessionOrigin::parse(&origin_str)?,
            title: row.get("title"),
            last_message_at,
            last_message_snippet: row.get("last_message_snippet"),
            agent_type: row.get("agent_type"),
            model: row.get("model"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "sessions.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "sessions.updated_at")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub seq: i64,
    pub session_id: Uuid,
    pub message_json: String,
    pub sender_kind: SenderKind,
    pub sender_name: Option<String>,
    pub sender_user_id: Option<Uuid>,
    pub attachments: Vec<String>,
    pub artifacts: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl Message {
    fn from_sqlite_row(row: &SqliteRow) -> DbResult<Self> {
        let kind_str: String = row.get("sender_kind");
        let sender_user_id = row
            .get::<Option<String>, _>("sender_user_id")
            .map(|s| parse_uuid(s, "session_messages.sender_user_id"))
            .transpose()?;
        let attachments_json: String = row.get("attachments");
        let artifacts_json: String = row.get("artifacts");
        Ok(Self {
            seq: row.get("seq"),
            session_id: parse_uuid(
                row.get::<String, _>("session_id"),
                "session_messages.session_id",
            )?,
            message_json: row.get("message_json"),
            sender_kind: SenderKind::parse(&kind_str)?,
            sender_name: row.get("sender_name"),
            sender_user_id,
            attachments: serde_json::from_str(&attachments_json)
                .map_err(|e| DbError::InvalidData(format!("attachments json: {e}")))?,
            artifacts: serde_json::from_str(&artifacts_json)
                .map_err(|e| DbError::InvalidData(format!("artifacts json: {e}")))?,
            created_at: parse_ts(
                &row.get::<String, _>("created_at"),
                "session_messages.created_at",
            )?,
        })
    }
}

pub async fn insert_session(
    pool: &SqlitePool,
    project_id: Uuid,
    creator_id: Uuid,
    origin: SessionOrigin,
    agent_type: Option<String>,
    model: Option<String>,
) -> DbResult<Session> {
    let id = Uuid::new_v4();
    let now = now_string();
    let share_mode = ShareMode::Private;
    sqlx::query(query::session::INSERT)
        .bind(id.to_string())
        .bind(project_id.to_string())
        .bind(creator_id.to_string())
        .bind(share_mode.as_str())
        .bind(origin.as_str())
        .bind(&agent_type)
        .bind(&model)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

    Ok(Session {
        id,
        project_id,
        creator_id,
        share_mode,
        origin,
        title: None,
        last_message_at: None,
        last_message_snippet: None,
        agent_type,
        model,
        created_at: parse_ts(&now, "sessions.created_at")?,
        updated_at: parse_ts(&now, "sessions.updated_at")?,
    })
}

pub async fn get_session(pool: &SqlitePool, id: Uuid) -> DbResult<Option<Session>> {
    let row = sqlx::query(query::session::SELECT_BY_ID)
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(Session::from_sqlite_row).transpose()
}

pub async fn list_sessions_in_project(
    pool: &SqlitePool,
    project_id: Uuid,
) -> DbResult<Vec<Session>> {
    let rows = sqlx::query(query::session::LIST_BY_PROJECT)
        .bind(project_id.to_string())
        .fetch_all(pool)
        .await?;
    rows.iter().map(Session::from_sqlite_row).collect()
}

/// Each arg is `None` to leave the field unchanged, `Some(_)` to replace.
/// `title`/`agent_type`/`model` can be cleared by passing `Some(None)`.
pub async fn update_session(
    pool: &SqlitePool,
    id: Uuid,
    title: Option<Option<String>>,
    share_mode: Option<ShareMode>,
    agent_type: Option<Option<String>>,
    model: Option<Option<String>>,
) -> DbResult<Session> {
    let current = get_session(pool, id).await?.ok_or(DbError::NotFound)?;
    let title = title.unwrap_or(current.title);
    let share_mode = share_mode.unwrap_or(current.share_mode);
    let agent_type = agent_type.unwrap_or(current.agent_type);
    let model = model.unwrap_or(current.model);
    let now = now_string();

    let res = sqlx::query(query::session::UPDATE)
        .bind(&title)
        .bind(share_mode.as_str())
        .bind(&agent_type)
        .bind(&model)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    get_session(pool, id).await?.ok_or(DbError::NotFound)
}

pub async fn delete_session(pool: &SqlitePool, id: Uuid) -> DbResult<bool> {
    let res = sqlx::query(query::session::DELETE)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Append a single message and refresh the parent session's
/// `last_message_at`/`last_message_snippet`/`updated_at` in one transaction.
pub async fn append_message(
    pool: &SqlitePool,
    session_id: Uuid,
    message_json: String,
    sender_kind: SenderKind,
    sender_name: Option<String>,
    sender_user_id: Option<Uuid>,
    attachments: Vec<String>,
    artifacts: Vec<String>,
    snippet: Option<String>,
) -> DbResult<Message> {
    let now = now_string();
    let attachments_json = serde_json::to_string(&attachments)
        .map_err(|e| DbError::InvalidData(format!("attachments json: {e}")))?;
    let artifacts_json = serde_json::to_string(&artifacts)
        .map_err(|e| DbError::InvalidData(format!("artifacts json: {e}")))?;

    let mut tx = pool.begin().await?;

    let seq: i64 = sqlx::query_scalar(query::message::INSERT)
        .bind(session_id.to_string())
        .bind(&message_json)
        .bind(&now)
        .bind(sender_kind.as_str())
        .bind(&sender_name)
        .bind(sender_user_id.map(|u| u.to_string()))
        .bind(&attachments_json)
        .bind(&artifacts_json)
        .fetch_one(&mut *tx)
        .await?;

    sqlx::query(query::session::TOUCH_AFTER_MESSAGE)
        .bind(&now)
        .bind(session_id.to_string())
        .bind(snippet.as_deref())
        .bind(session_id.to_string())
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Message {
        seq,
        session_id,
        message_json,
        sender_kind,
        sender_name,
        sender_user_id,
        attachments,
        artifacts,
        created_at: parse_ts(&now, "session_messages.created_at")?,
    })
}

pub async fn list_messages_by_session(
    pool: &SqlitePool,
    session_id: Uuid,
) -> DbResult<Vec<Message>> {
    let rows = sqlx::query(query::message::LIST_BY_SESSION)
        .bind(session_id.to_string())
        .fetch_all(pool)
        .await?;
    rows.iter().map(Message::from_sqlite_row).collect()
}

pub async fn clear_messages_by_session(pool: &SqlitePool, session_id: Uuid) -> DbResult<u64> {
    let res = sqlx::query(query::message::DELETE_BY_SESSION)
        .bind(session_id.to_string())
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::*;

    #[tokio::test]
    async fn session_crud_round_trip() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;
        let project = make_project(&pool, owner).await;

        let s = insert_session(
            &pool,
            project,
            owner,
            SessionOrigin::User,
            Some("coworker".into()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(s.share_mode, ShareMode::Private);
        assert_eq!(s.origin, SessionOrigin::User);
        assert_eq!(s.agent_type.as_deref(), Some("coworker"));

        let fetched = get_session(&pool, s.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, s.id);

        let updated = update_session(
            &pool,
            s.id,
            Some(Some("My session".into())),
            Some(ShareMode::SharedChat),
            None,
            Some(Some("anthropic/claude-sonnet-4-6".into())),
        )
        .await
        .unwrap();
        assert_eq!(updated.title.as_deref(), Some("My session"));
        assert_eq!(updated.share_mode, ShareMode::SharedChat);
        assert_eq!(updated.agent_type.as_deref(), Some("coworker"));
        assert_eq!(updated.model.as_deref(), Some("anthropic/claude-sonnet-4-6"));

        let listed = list_sessions_in_project(&pool, project).await.unwrap();
        assert_eq!(listed.len(), 1);

        assert!(delete_session(&pool, s.id).await.unwrap());
        assert!(get_session(&pool, s.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn append_message_updates_last_message_fields() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;
        let project = make_project(&pool, owner).await;
        let s = insert_session(&pool, project, owner, SessionOrigin::User, None, None)
            .await
            .unwrap();

        let m1 = append_message(
            &pool,
            s.id,
            r#"{"role":"user","content":"hi"}"#.into(),
            SenderKind::User,
            Some("alice".into()),
            Some(owner),
            vec!["att.txt".into()],
            vec![],
            Some("hi".into()),
        )
        .await
        .unwrap();
        assert_eq!(m1.seq, 1);

        let m2 = append_message(
            &pool,
            s.id,
            r#"{"role":"assistant","content":"hello"}"#.into(),
            SenderKind::Agent,
            None,
            None,
            vec![],
            vec!["report.md".into()],
            Some("hello".into()),
        )
        .await
        .unwrap();
        assert_eq!(m2.seq, 2);

        let after = get_session(&pool, s.id).await.unwrap().unwrap();
        assert_eq!(after.last_message_snippet.as_deref(), Some("hello"));
        assert!(after.last_message_at.is_some());

        let msgs = list_messages_by_session(&pool, s.id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].sender_kind, SenderKind::User);
        assert_eq!(msgs[0].attachments, vec!["att.txt".to_string()]);
        assert_eq!(msgs[1].sender_kind, SenderKind::Agent);
        assert_eq!(msgs[1].artifacts, vec!["report.md".to_string()]);
        assert!(msgs[1].sender_user_id.is_none());

        assert_eq!(clear_messages_by_session(&pool, s.id).await.unwrap(), 2);
        assert!(
            list_messages_by_session(&pool, s.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delete_session_cascades_to_messages() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;
        let project = make_project(&pool, owner).await;
        let s = insert_session(&pool, project, owner, SessionOrigin::User, None, None)
            .await
            .unwrap();
        append_message(
            &pool,
            s.id,
            "{}".into(),
            SenderKind::Agent,
            None,
            None,
            vec![],
            vec![],
            None,
        )
        .await
        .unwrap();
        assert!(delete_session(&pool, s.id).await.unwrap());
        assert!(
            list_messages_by_session(&pool, s.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
