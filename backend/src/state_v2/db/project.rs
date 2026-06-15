use chrono::{DateTime, Utc};
use sqlx::{Row as _, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use super::{DbError, DbResult, map_unique, now_string, parse_ts, parse_uuid};

mod query {
    pub const INSERT: &str = "\
        INSERT INTO projects (id, slug, name, description, owner_id, created_at, updated_at) \
        VALUES (?, ?, ?, ?, ?, ?, ?)";

    pub const SELECT_BY_ID: &str = "\
        SELECT id, slug, name, description, owner_id, created_at, updated_at \
        FROM projects WHERE id = ?";

    pub const SELECT_BY_SLUG: &str = "\
        SELECT id, slug, name, description, owner_id, created_at, updated_at \
        FROM projects WHERE slug = ?";

    pub const LIST: &str = "\
        SELECT id, slug, name, description, owner_id, created_at, updated_at \
        FROM projects ORDER BY created_at ASC";

    pub const UPDATE: &str = "\
        UPDATE projects \
        SET slug = ?, name = ?, description = ?, updated_at = ? \
        WHERE id = ?";

    pub const DELETE: &str = "DELETE FROM projects WHERE id = ?";
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    fn from_sqlite_row(row: &SqliteRow) -> DbResult<Self> {
        Ok(Self {
            id: parse_uuid(row.get::<String, _>("id"), "projects.id")?,
            slug: row.get("slug"),
            name: row.get("name"),
            description: row.get("description"),
            owner_id: parse_uuid(row.get::<String, _>("owner_id"), "projects.owner_id")?,
            created_at: parse_ts(&row.get::<String, _>("created_at"), "projects.created_at")?,
            updated_at: parse_ts(&row.get::<String, _>("updated_at"), "projects.updated_at")?,
        })
    }
}

/// `slug = None` derives one from `name` (`slug::slugify`, with a nanoid
/// fallback when `name` slugifies to empty). Caller is responsible for
/// collision retries — duplicates surface as `DbError::UniqueViolation`.
pub(super) async fn insert_project(
    pool: &SqlitePool,
    owner_id: Uuid,
    name: String,
    description: Option<String>,
    slug: Option<String>,
) -> DbResult<Project> {
    let id = Uuid::new_v4();
    let now = now_string();
    let slug = slug.unwrap_or_else(|| {
        let derived = slug::slugify(&name);
        if derived.is_empty() {
            format!("project-{}", nanoid::nanoid!(6))
        } else {
            derived
        }
    });
    sqlx::query(query::INSERT)
        .bind(id.to_string())
        .bind(&slug)
        .bind(&name)
        .bind(&description)
        .bind(owner_id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| map_unique(e, "projects.slug"))?;

    Ok(Project {
        id,
        slug,
        name,
        description,
        owner_id,
        created_at: parse_ts(&now, "projects.created_at")?,
        updated_at: parse_ts(&now, "projects.updated_at")?,
    })
}

pub(super) async fn get_project(pool: &SqlitePool, id: Uuid) -> DbResult<Option<Project>> {
    let row = sqlx::query(query::SELECT_BY_ID)
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(Project::from_sqlite_row).transpose()
}

pub(super) async fn get_project_by_slug(
    pool: &SqlitePool,
    slug: &str,
) -> DbResult<Option<Project>> {
    let row = sqlx::query(query::SELECT_BY_SLUG)
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(Project::from_sqlite_row).transpose()
}

pub(super) async fn list_projects(pool: &SqlitePool) -> DbResult<Vec<Project>> {
    let rows = sqlx::query(query::LIST).fetch_all(pool).await?;
    rows.iter().map(Project::from_sqlite_row).collect()
}

/// Each arg is `None` to leave the field unchanged, `Some(_)` to replace.
/// `description: Some(None)` explicitly clears the column to NULL.
pub(super) async fn update_project(
    pool: &SqlitePool,
    id: Uuid,
    name: Option<String>,
    description: Option<Option<String>>,
    slug: Option<String>,
) -> DbResult<Project> {
    let current = get_project(pool, id).await?.ok_or(DbError::NotFound)?;
    let slug = slug.unwrap_or(current.slug);
    let name = name.unwrap_or(current.name);
    let description = description.unwrap_or(current.description);
    let now = now_string();

    let res = sqlx::query(query::UPDATE)
        .bind(&slug)
        .bind(&name)
        .bind(&description)
        .bind(&now)
        .bind(id.to_string())
        .execute(pool)
        .await
        .map_err(|e| map_unique(e, "projects.slug"))?;

    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    get_project(pool, id).await?.ok_or(DbError::NotFound)
}

pub(super) async fn delete_project(pool: &SqlitePool, id: Uuid) -> DbResult<bool> {
    let res = sqlx::query(query::DELETE)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use super::*;

    #[tokio::test]
    async fn project_crud_round_trip() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;

        let created = insert_project(
            &pool,
            owner,
            "Alpha".into(),
            Some("first".into()),
            Some("alpha".into()),
        )
        .await
        .unwrap();
        assert_eq!(created.slug, "alpha");

        let fetched = get_project(&pool, created.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "Alpha");

        let by_slug = get_project_by_slug(&pool, "alpha").await.unwrap().unwrap();
        assert_eq!(by_slug.id, created.id);

        let updated = update_project(&pool, created.id, Some("Alpha v2".into()), Some(None), None)
            .await
            .unwrap();
        assert_eq!(updated.name, "Alpha v2");
        assert!(updated.description.is_none());

        assert_eq!(list_projects(&pool).await.unwrap().len(), 1);

        assert!(delete_project(&pool, created.id).await.unwrap());
        assert!(get_project(&pool, created.id).await.unwrap().is_none());
        assert!(!delete_project(&pool, created.id).await.unwrap());
    }

    #[tokio::test]
    async fn duplicate_slug_is_unique_violation() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;

        insert_project(&pool, owner, "A".into(), None, Some("dup".into()))
            .await
            .unwrap();

        let err = insert_project(&pool, owner, "B".into(), None, Some("dup".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation(_)));
    }

    #[tokio::test]
    async fn insert_derives_slug_from_name_when_omitted() {
        let pool = fresh_db().await;
        let owner = make_owner(&pool).await;

        let p = insert_project(&pool, owner, "My Cool Project".into(), None, None)
            .await
            .unwrap();
        assert_eq!(p.slug, "my-cool-project");
    }

    #[tokio::test]
    async fn update_missing_project_returns_not_found() {
        let pool = fresh_db().await;
        let err = update_project(&pool, Uuid::new_v4(), None, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound));
    }
}
