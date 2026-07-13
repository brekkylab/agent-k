use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{StateError, StateResult, parse_ts, parse_uuid};
use crate::auth::Role;

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub preferred_language: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct NewUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub preferred_language: String,
}

#[derive(Default)]
pub struct UpdateUser {
    pub display_name: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<Role>,
    pub is_active: Option<bool>,
    pub preferred_language: Option<String>,
}

fn parse_role(raw: String) -> StateResult<Role> {
    match raw.as_str() {
        "user" => Ok(Role::User),
        "admin" => Ok(Role::Admin),
        other => Err(StateError::InvalidData(format!("users.role: {other}"))),
    }
}

impl User {
    fn from_sqlite_row(row: &SqliteRow) -> StateResult<Self> {
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "users.id")?,
            username: row.get("username"),
            password_hash: row.get("password_hash"),
            role: parse_role(row.get::<String, _>("role"))?,
            display_name: row.get("display_name"),
            is_active: row.get::<i64, _>("is_active") != 0,
            preferred_language: row.get("preferred_language"),
            created_at: parse_ts(&row.get::<String, _>("created_at"), "users.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "users.updated_at")?,
        })
    }
}

pub struct UsersState {
    db: SqlitePool,
}

impl UsersState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    pub async fn create(&self, user: NewUser) -> StateResult<User> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, display_name, is_active, preferred_language, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(user.role.as_str())
        .bind(&user.display_name)
        .bind(if user.is_active { 1i64 } else { 0i64 })
        .bind(&user.preferred_language)
        .bind(&now_str)
        .bind(&now_str)
        .execute(&self.db)
        .await
        .map_err(map_sqlx_error)?;

        Ok(User {
            id: user.id,
            username: user.username,
            password_hash: user.password_hash,
            role: user.role,
            display_name: user.display_name,
            is_active: user.is_active,
            preferred_language: user.preferred_language,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn get(&self, id: Uuid) -> StateResult<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, preferred_language, created_at, updated_at \
             FROM users WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.db)
        .await?;
        row.as_ref().map(User::from_sqlite_row).transpose()
    }

    pub async fn get_by_username(&self, username: &str) -> StateResult<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, preferred_language, created_at, updated_at \
             FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.db)
        .await?;
        row.as_ref().map(User::from_sqlite_row).transpose()
    }

    pub async fn list(&self, page: u32, size: u32) -> StateResult<(Vec<User>, i64)> {
        let size = size.min(100) as i64;
        let offset = ((page.saturating_sub(1)) as i64) * size;

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.db)
            .await?;

        let rows = sqlx::query(
            "SELECT id, username, password_hash, role, display_name, is_active, preferred_language, created_at, updated_at \
             FROM users ORDER BY created_at ASC LIMIT ? OFFSET ?",
        )
        .bind(size)
        .bind(offset)
        .fetch_all(&self.db)
        .await?;

        let users = rows
            .iter()
            .map(User::from_sqlite_row)
            .collect::<StateResult<Vec<_>>>()?;
        Ok((users, total))
    }

    pub async fn update(&self, id: Uuid, update: UpdateUser) -> StateResult<Option<User>> {
        let now = Utc::now().to_rfc3339();

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
        if let Some(ref lang) = update.preferred_language {
            builder.push(", preferred_language = ").push_bind(lang);
        }

        builder.push(" WHERE id = ").push_bind(id.to_string());

        let result = builder.build().execute(&self.db).await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get(id).await
    }

    /// Atomically delete the user and their default workspace (which shares the
    /// user's id), cascading to that workspace's agents and sessions via foreign
    /// keys. The caller must remove the workspace's on-disk files first (see
    /// [`WorkspacesState::remove_files`](super::WorkspacesState::remove_files)).
    /// Returns whether a user row existed.
    pub async fn delete_with_default_workspace(&self, id: Uuid) -> StateResult<bool> {
        let mut tx = self.db.begin().await?;
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workspaces WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn count_admins(&self) -> StateResult<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = 1",
        )
        .fetch_one(&self.db)
        .await?;
        Ok(count)
    }
}

fn map_sqlx_error(e: sqlx::Error) -> StateError {
    if let sqlx::Error::Database(ref db_err) = e {
        if db_err
            .code()
            .map(|c| c == "2067" || c == "1555")
            .unwrap_or(false)
            || db_err.message().contains("UNIQUE")
        {
            return StateError::UniqueViolation("username".to_string());
        }
    }
    StateError::Sqlx(e)
}
