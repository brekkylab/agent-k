//! Background driver for the Gmail mailbox mirror.
//!
//! The sync engine itself lives in the workspace crate
//! ([`sync_gmail_incremental`] resolves what kind of run is needed: no state →
//! initial full sync, incomplete → resumed full sync, complete → history.list
//! journal replay). This module decides only *when* it runs:
//!
//! - **mount creation** spawns the initial full sync in the background,
//! - **the frontend** triggers later refreshes explicitly
//!   (`POST /workspaces/{wid}/mounts/{mount_id}/sync`); there is no timer, and
//! - **mount removal** garbage-collects the account's mirror once no mount
//!   anywhere references the account (aborting its running sync first).
//!
//! Runs go through a per-account single-flight guard, so concurrent triggers
//! (a double-clicked refresh, two mounts of one account, refresh-during-
//! initial-sync) never run two engines over one mirror directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ::workspace::{
    GmailConfig, GmailSyncState, ProviderConfig, account_mirror_dir, sync_gmail_incremental,
};
use sqlx::Row as _;

use super::WorkspacesState;
use crate::state::StateResult;

/// Per-account single-flight registry for mirror sync runs, keyed by the
/// account mirror directory (the resource every run exclusively owns). Holds
/// each run's [`tokio::task::JoinHandle`] so mirror GC can abort it.
#[derive(Clone, Default)]
pub struct GmailSyncRunner {
    active: Arc<Mutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
}

impl GmailSyncRunner {
    /// Whether a sync run is currently active for the account dir.
    pub fn is_running(&self, acct: &Path) -> bool {
        self.active
            .lock()
            .unwrap()
            .get(acct)
            .is_some_and(|h| !h.is_finished())
    }

    /// Start a background sync for the account unless one is already running.
    /// Returns whether this call started a new run. Progress is observable via
    /// the mirror's `state.json` ([`GmailSyncState::load`]); the outcome is
    /// logged, not returned — callers poll status instead of awaiting.
    pub fn spawn(&self, config: GmailConfig, mirror_root: &Path) -> bool {
        let acct = account_mirror_dir(mirror_root, &config.account_email);
        let mut active = self.active.lock().unwrap();
        if active.get(&acct).is_some_and(|h| !h.is_finished()) {
            return false;
        }
        let dir = acct.clone();
        let handle = tokio::spawn(async move {
            let email = config.account_email.clone();
            match sync_gmail_incremental(&config, &dir).await {
                Ok(d) => tracing::info!(
                    "gmail sync [{email}]: +{} -{} ~{} (full_resync={})",
                    d.added,
                    d.deleted,
                    d.relabeled,
                    d.full_resync
                ),
                Err(e) => tracing::warn!("gmail sync [{email}] failed: {e:#}"),
            }
        });
        active.insert(acct, handle);
        true
    }

    /// Abort the account's sync run, if one is active. Used before deleting
    /// the mirror dir — a live engine would otherwise recreate it mid-removal.
    fn abort(&self, acct: &Path) {
        if let Some(h) = self.active.lock().unwrap().remove(acct) {
            h.abort();
        }
    }
}

impl WorkspacesState {
    /// Kick a background mirror sync for a Gmail mount's account (initial full
    /// sync or incremental refresh — the engine picks). Returns whether a new
    /// run started (`false` = one is already in flight; that run's result is
    /// equivalent, so callers just report it).
    pub fn spawn_gmail_sync(&self, config: &GmailConfig) -> bool {
        self.gmail_sync.spawn(config.clone(), &self.mirror_root())
    }

    /// `(persisted sync state, run currently active)` for a Gmail mount's
    /// account mirror. State is `None` until the first run writes it.
    pub fn gmail_sync_status(&self, config: &GmailConfig) -> (Option<GmailSyncState>, bool) {
        let acct = account_mirror_dir(&self.mirror_root(), &config.account_email);
        (
            GmailSyncState::load(&acct),
            self.gmail_sync.is_running(&acct),
        )
    }

    /// Delete the account's on-disk mirror unless some other mount (any
    /// workspace) still references the account. Called after a Gmail mount row
    /// is removed: aborts the account's running sync first, then removes the
    /// whole account dir (tree, staging, state, done log). Filesystem errors
    /// are logged, not returned — the mount row is already gone, and a
    /// leftover dir is re-adopted (resumed) if the account is ever re-mounted.
    pub(super) async fn gc_gmail_mirror(&self, account_email: &str) -> StateResult<()> {
        let acct = account_mirror_dir(&self.mirror_root(), account_email);

        // Still referenced? Compare via the mirror dir each config maps to,
        // the same identity the sync and the resource use.
        let rows = sqlx::query("SELECT config FROM workspace_mounts WHERE provider = 'gmail'")
            .fetch_all(&self.db)
            .await?;
        for row in &rows {
            let cfg: GmailConfig = serde_json::from_str(&row.get::<String, _>("config"))?;
            if account_mirror_dir(&self.mirror_root(), &cfg.account_email) == acct {
                return Ok(());
            }
        }

        self.gmail_sync.abort(&acct);
        match tokio::fs::remove_dir_all(&acct).await {
            Ok(()) => tracing::info!("gmail mirror gc: removed {}", acct.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("gmail mirror gc: {}: {e}", acct.display()),
        }
        Ok(())
    }

    /// Provider-dispatch hook for mount removal side effects.
    pub(super) async fn on_mount_removed(&self, provider: &ProviderConfig) {
        if let ProviderConfig::Gmail(c) = provider
            && let Err(e) = self.gc_gmail_mirror(&c.account_email).await
        {
            tracing::warn!("gmail mirror gc for {}: {e}", c.account_email);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_flight_and_abort() {
        let runner = GmailSyncRunner::default();
        let a = Path::new("/mirror/gmail/a@x.com");

        // Simulate an in-flight run.
        runner
            .active
            .lock()
            .unwrap()
            .insert(a.to_path_buf(), tokio::spawn(std::future::pending()));
        assert!(runner.is_running(a));
        assert!(!runner.is_running(Path::new("/mirror/gmail/b@x.com")));

        // Abort frees the account.
        runner.abort(a);
        assert!(!runner.is_running(a));
    }
}
