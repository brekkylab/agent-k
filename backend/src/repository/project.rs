use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::SqliteRepository;
use crate::repository::{RepositoryError, RepositoryResult};

#[derive(Debug, Clone)]
pub struct DbProject {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbProjectMember {
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub added_at: DateTime<Utc>,
}

impl SqliteRepository {
    fn row_to_db_project(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbProject> {
        Ok(DbProject {
            id: Self::parse_uuid(row.get::<String, _>("id"), "projects.id")?,
            name: row.get("name"),
            description: row.get("description"),
            owner_id: Self::parse_uuid(row.get::<String, _>("owner_id"), "projects.owner_id")?,
            created_at: Self::parse_timestamp(
                row.get::<String, _>("created_at"),
                "projects.created_at",
            )?,
            updated_at: Self::parse_timestamp(
                row.get::<String, _>("updated_at"),
                "projects.updated_at",
            )?,
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

        sqlx::query("UPDATE projects SET name = ?, description = ?, updated_at = ? WHERE id = ?")
            .bind(&new_name)
            .bind(&new_desc)
            .bind(&now)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        self.get_project(id)
            .await?
            .ok_or_else(|| RepositoryError::InvalidData("project disappeared after update".into()))
    }

    pub async fn delete_project(&self, id: Uuid) -> RepositoryResult<bool> {
        let result = sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn add_project_member(
        &self,
        project_id: Uuid,
        user_id: Uuid,
    ) -> RepositoryResult<()> {
        let now = Self::now_string();
        sqlx::query("INSERT INTO project_members (project_id, user_id, added_at) VALUES (?, ?, ?)")
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
        let result =
            sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
                .bind(project_id.to_string())
                .bind(user_id.to_string())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_project_members(
        &self,
        project_id: Uuid,
    ) -> RepositoryResult<Vec<(crate::repository::DbUser, chrono::DateTime<chrono::Utc>)>> {
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
                let added_at =
                    Self::parse_timestamp(r.get::<String, _>("added_at"), "pm.added_at")?;
                let user = Self::row_to_db_user(&r)?;
                Ok((user, added_at))
            })
            .collect()
    }

    pub async fn user_in_project(&self, user_id: Uuid, project_id: Uuid) -> RepositoryResult<bool> {
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
}
