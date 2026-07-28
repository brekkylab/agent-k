//! Per-mount snapshot store for external-source polling. The poller (worker.rs)
//! scans a subscribed provider mount, diffs the fresh listing against the stored
//! snapshot to synthesize `s3.*`/`notion.*` events, then overwrites the snapshot.

use std::collections::HashMap;

use sqlx::{Row as _, SqlitePool};
use uuid::Uuid;

use super::StateResult;

/// A path → change-signature map for one mount (signature is `modified:size`).
pub type Snapshot = HashMap<String, String>;

#[derive(Clone)]
pub struct SourcePollState {
    db: SqlitePool,
}

impl SourcePollState {
    pub fn new(db: SqlitePool) -> Self {
        Self { db }
    }

    /// The last stored snapshot for `(workspace, prefix)`, or `None` if the mount
    /// has never been polled (the first poll seeds a baseline without emitting).
    pub async fn get(&self, workspace_id: Uuid, prefix: &str) -> StateResult<Option<Snapshot>> {
        let row = sqlx::query(
            "SELECT snapshot FROM source_poll_state WHERE workspace_id = ? AND prefix = ?",
        )
        .bind(workspace_id.to_string())
        .bind(prefix)
        .fetch_optional(&self.db)
        .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(&r.get::<String, _>("snapshot"))?)),
            None => Ok(None),
        }
    }

    /// Overwrite the snapshot for `(workspace, prefix)`.
    pub async fn put(&self, workspace_id: Uuid, prefix: &str, snap: &Snapshot) -> StateResult<()> {
        let snapshot = serde_json::to_string(snap)?;
        sqlx::query(
            "INSERT INTO source_poll_state (workspace_id, prefix, snapshot, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(workspace_id, prefix) DO UPDATE SET \
                 snapshot = excluded.snapshot, updated_at = excluded.updated_at",
        )
        .bind(workspace_id.to_string())
        .bind(prefix)
        .bind(&snapshot)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.db)
        .await?;
        Ok(())
    }
}

/// One change discovered by diffing a fresh scan against the prior snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Created(String),
    Modified(String),
    Removed(String),
}

/// Diff `prev` → `next` into created/modified/removed changes (path-keyed).
pub fn diff_snapshots(prev: &Snapshot, next: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    for (path, sig) in next {
        match prev.get(path) {
            None => out.push(Change::Created(path.clone())),
            Some(old) if old != sig => out.push(Change::Modified(path.clone())),
            Some(_) => {}
        }
    }
    for path in prev.keys() {
        if !next.contains_key(path) {
            out.push(Change::Removed(path.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(pairs: &[(&str, &str)]) -> Snapshot {
        pairs.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect()
    }

    #[test]
    fn diff_detects_add_change_remove() {
        let prev = snap(&[("/m/a", "1"), ("/m/b", "1")]);
        let next = snap(&[("/m/a", "1"), ("/m/b", "2"), ("/m/c", "1")]);
        let mut got = diff_snapshots(&prev, &next);
        got.sort_by_key(|c| match c {
            Change::Created(p) | Change::Modified(p) | Change::Removed(p) => p.clone(),
        });
        assert_eq!(
            got,
            vec![
                Change::Modified("/m/b".into()),
                Change::Created("/m/c".into()),
            ]
        );
        // /m/a unchanged → no event; nothing removed.
    }

    #[test]
    fn diff_detects_removal() {
        let prev = snap(&[("/m/a", "1")]);
        let next = snap(&[]);
        assert_eq!(diff_snapshots(&prev, &next), vec![Change::Removed("/m/a".into())]);
    }
}
