use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::SqliteRepository;
use crate::{
    model::IngestStatus,
    repository::{RepositoryError, RepositoryResult},
};

#[derive(Debug, Clone)]
pub struct DbIngestJob {
    pub id: Uuid,
    pub project_id: Uuid,
    pub source_path: String,
    pub status: IngestStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub document_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DbProjectDocument {
    pub project_id: Uuid,
    pub source_path: String,
    pub document_id: String,
    pub ingested_at: DateTime<Utc>,
}

impl SqliteRepository {
    fn row_to_ingest_job(row: &sqlx::sqlite::SqliteRow) -> RepositoryResult<DbIngestJob> {
        let id: String = row.try_get("id")?;
        let project_id: String = row.try_get("project_id")?;
        let source_path: String = row.try_get("source_path")?;
        let status: String = row.try_get("status")?;
        let attempts: i64 = row.try_get("attempts")?;
        let last_error: Option<String> = row.try_get("last_error")?;
        let lease_until: Option<String> = row.try_get("lease_until")?;
        let document_id: Option<String> = row.try_get("document_id")?;
        let created_at: String = row.try_get("created_at")?;
        let updated_at: String = row.try_get("updated_at")?;

        Ok(DbIngestJob {
            id: Uuid::parse_str(&id)
                .map_err(|e| RepositoryError::InvalidData(format!("ingest_jobs.id: {e}")))?,
            project_id: Uuid::parse_str(&project_id).map_err(|e| {
                RepositoryError::InvalidData(format!("ingest_jobs.project_id: {e}"))
            })?,
            source_path,
            status: IngestStatus::from_db(&status).map_err(RepositoryError::InvalidData)?,
            attempts,
            last_error,
            lease_until: lease_until
                .as_deref()
                .map(parse_ts)
                .transpose()?,
            document_id,
            created_at: parse_ts(&created_at)?,
            updated_at: parse_ts(&updated_at)?,
        })
    }

    fn row_to_project_document(
        row: &sqlx::sqlite::SqliteRow,
    ) -> RepositoryResult<DbProjectDocument> {
        let project_id: String = row.try_get("project_id")?;
        let source_path: String = row.try_get("source_path")?;
        let document_id: String = row.try_get("document_id")?;
        let ingested_at: String = row.try_get("ingested_at")?;
        Ok(DbProjectDocument {
            project_id: Uuid::parse_str(&project_id).map_err(|e| {
                RepositoryError::InvalidData(format!("project_documents.project_id: {e}"))
            })?,
            source_path,
            document_id,
            ingested_at: parse_ts(&ingested_at)?,
        })
    }

    /// Idempotent enqueue. If a row already exists for (project_id, source_path),
    /// reset it to `queued` (attempts=0, last_error=NULL) so a re-uploaded file
    /// goes back through the worker. The returned row is the post-upsert state.
    pub async fn enqueue_ingest_job(
        &self,
        project_id: Uuid,
        source_path: &str,
    ) -> RepositoryResult<DbIngestJob> {
        let now = Self::now_string();
        let new_id = Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO ingest_jobs (id, project_id, source_path, status, attempts, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'queued', 0, ?4, ?4) \
             ON CONFLICT(project_id, source_path) DO UPDATE SET \
                status = 'queued', attempts = 0, last_error = NULL, \
                lease_until = NULL, document_id = NULL, updated_at = ?4",
        )
        .bind(&new_id)
        .bind(project_id.to_string())
        .bind(source_path)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "SELECT id, project_id, source_path, status, attempts, last_error, lease_until, document_id, created_at, updated_at \
             FROM ingest_jobs WHERE project_id = ? AND source_path = ?",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .fetch_one(&self.pool)
        .await?;

        Self::row_to_ingest_job(&row)
    }

    /// Atomic queued→running pickup, oldest first. Mirrors `claim_due_run`.
    pub async fn claim_ingest_job(
        &self,
        lease_until: DateTime<Utc>,
    ) -> RepositoryResult<Option<DbIngestJob>> {
        let lease_s = ts_string(lease_until);
        let updated = Self::now_string();
        let row = sqlx::query(
            "UPDATE ingest_jobs SET status='running', lease_until = ?1, attempts = attempts + 1, updated_at = ?2 \
             WHERE id = ( \
                SELECT id FROM ingest_jobs WHERE status = 'queued' \
                ORDER BY created_at ASC LIMIT 1 \
             ) \
             RETURNING id, project_id, source_path, status, attempts, last_error, lease_until, document_id, created_at, updated_at",
        )
        .bind(&lease_s)
        .bind(&updated)
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(Self::row_to_ingest_job).transpose()
    }

    pub async fn renew_ingest_lease(
        &self,
        job_id: Uuid,
        new_lease_until: DateTime<Utc>,
    ) -> RepositoryResult<bool> {
        let lease_s = ts_string(new_lease_until);
        let updated = Self::now_string();
        let res = sqlx::query(
            "UPDATE ingest_jobs SET lease_until = ?, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(&lease_s)
        .bind(&updated)
        .bind(job_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Finalize a running job to `done`, recording the resulting document id
    /// and inserting the project_documents mapping in the same transaction.
    pub async fn finalize_ingest_done(
        &self,
        job_id: Uuid,
        project_id: Uuid,
        source_path: &str,
        document_id: &str,
    ) -> RepositoryResult<bool> {
        let mut tx = self.pool.begin().await?;
        let now = Self::now_string();

        let res = sqlx::query(
            "UPDATE ingest_jobs SET status='done', document_id = ?, lease_until = NULL, last_error = NULL, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(document_id)
        .bind(&now)
        .bind(job_id.to_string())
        .execute(&mut *tx)
        .await?;

        if res.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO project_documents (project_id, source_path, document_id, ingested_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(project_id, source_path) DO UPDATE SET \
                document_id = ?3, ingested_at = ?4",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .bind(document_id)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(true)
    }

    pub async fn finalize_ingest_failed(
        &self,
        job_id: Uuid,
        error: &str,
    ) -> RepositoryResult<bool> {
        let now = Self::now_string();
        let res = sqlx::query(
            "UPDATE ingest_jobs SET status='failed', last_error = ?, lease_until = NULL, updated_at = ? \
             WHERE id = ? AND status = 'running'",
        )
        .bind(error)
        .bind(&now)
        .bind(job_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Reap running jobs whose lease has expired — put them back into `queued`.
    /// Returns the number of rows reaped.
    pub async fn reap_expired_ingest_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> RepositoryResult<u64> {
        let now_s = ts_string(now);
        let updated = Self::now_string();
        let res = sqlx::query(
            "UPDATE ingest_jobs SET status='queued', lease_until = NULL, updated_at = ? \
             WHERE status='running' AND lease_until IS NOT NULL AND lease_until < ?",
        )
        .bind(&updated)
        .bind(&now_s)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn list_ingest_jobs_for_project(
        &self,
        project_id: Uuid,
    ) -> RepositoryResult<Vec<DbIngestJob>> {
        let rows = sqlx::query(
            "SELECT id, project_id, source_path, status, attempts, last_error, lease_until, document_id, created_at, updated_at \
             FROM ingest_jobs WHERE project_id = ? ORDER BY created_at DESC",
        )
        .bind(project_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::row_to_ingest_job).collect()
    }

    pub async fn get_project_document(
        &self,
        project_id: Uuid,
        source_path: &str,
    ) -> RepositoryResult<Option<DbProjectDocument>> {
        let row = sqlx::query(
            "SELECT project_id, source_path, document_id, ingested_at \
             FROM project_documents WHERE project_id = ? AND source_path = ?",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(Self::row_to_project_document).transpose()
    }

    /// Remove the mapping and its ingest_job in one transaction. The caller is
    /// responsible for removing the actual document from the agent-k Store
    /// — that side effect lives in handler code where the Store is in scope.
    pub async fn delete_project_document(
        &self,
        project_id: Uuid,
        source_path: &str,
    ) -> RepositoryResult<Option<String>> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            "SELECT document_id FROM project_documents \
             WHERE project_id = ? AND source_path = ?",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .fetch_optional(&mut *tx)
        .await?;

        let document_id: Option<String> = row.map(|r| r.get("document_id"));

        sqlx::query(
            "DELETE FROM project_documents WHERE project_id = ? AND source_path = ?",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "DELETE FROM ingest_jobs WHERE project_id = ? AND source_path = ?",
        )
        .bind(project_id.to_string())
        .bind(source_path)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(document_id)
    }
}

fn parse_ts(s: &str) -> RepositoryResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| RepositoryError::InvalidData(format!("invalid timestamp '{s}': {e}")))
}

fn ts_string(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}
