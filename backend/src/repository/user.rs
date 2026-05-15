use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::SqliteRepository;
use crate::{auth::Role, repository::RepositoryResult};

#[derive(Debug, Clone)]
pub struct DbUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: Role,
    pub display_name: Option<String>,
    pub is_active: bool,
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
}

pub struct UpdateUser {
    pub display_name: Option<String>,
    pub password_hash: Option<String>,
    pub role: Option<Role>,
    pub is_active: Option<bool>,
}

impl SqliteRepository {
    pub(crate) fn row_to_db_user(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbUser> {
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
        sqlx::query("DELETE FROM sessions WHERE creator_id = ?;")
            .bind(&uid)
            .execute(&self.pool)
            .await?;
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

    pub async fn create_user_with_personal_project(
        &self,
        new_user: NewUser,
    ) -> RepositoryResult<(DbUser, crate::repository::DbProject)> {
        let user = self.create_user(new_user).await?;
        let project = self
            .create_project("Personal".to_string(), None, user.id)
            .await?;
        Ok((user, project))
    }
}
