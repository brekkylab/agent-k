//! Background driver for the Gmail mailbox mirror.
//!
//! The sync engine itself lives in the workspace crate
//! ([`sync_gmail_incremental`] resolves what kind of run is needed: no state →
//! initial full sync, incomplete → resumed full sync, complete → history.list
//! journal replay). This module decides only *when* it runs:
//!
//! - **mount creation** spawns the initial full sync in the background, and
//! - **the frontend** triggers later refreshes explicitly
//!   (`POST /workspaces/{wid}/mounts/{mount_id}/sync`); there is no timer.
//!
//! Both go through a per-account single-flight guard, so concurrent triggers
//! (a double-clicked refresh, two mounts of one account, refresh-during-
//! initial-sync) never run two engines over one mirror directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ::workspace::{GmailConfig, GmailSyncState, account_mirror_dir, sync_gmail_incremental};

use super::WorkspacesState;

/// Per-account single-flight registry for mirror sync runs, keyed by the
/// account mirror directory (the resource every run exclusively owns).
#[derive(Clone, Default)]
pub struct GmailSyncRunner {
    active: Arc<Mutex<HashSet<PathBuf>>>,
}

/// An acquired run slot; releases the account on drop, so the slot is freed
/// however the sync task ends (success, error, panic unwind).
struct SyncSlot {
    active: Arc<Mutex<HashSet<PathBuf>>>,
    acct: PathBuf,
}

impl Drop for SyncSlot {
    fn drop(&mut self) {
        self.active.lock().unwrap().remove(&self.acct);
    }
}

impl GmailSyncRunner {
    /// Claim the account's run slot, or `None` if a run is already active.
    fn try_begin(&self, acct: &Path) -> Option<SyncSlot> {
        if self.active.lock().unwrap().insert(acct.to_path_buf()) {
            Some(SyncSlot {
                active: self.active.clone(),
                acct: acct.to_path_buf(),
            })
        } else {
            None
        }
    }

    /// Whether a sync run is currently active for the account dir.
    pub fn is_running(&self, acct: &Path) -> bool {
        self.active.lock().unwrap().contains(acct)
    }

    /// Start a background sync for the account unless one is already running.
    /// Returns whether this call started a new run. Progress is observable via
    /// the mirror's `state.json` ([`GmailSyncState::load`]); the outcome is
    /// logged, not returned — callers poll status instead of awaiting.
    pub fn spawn(&self, config: GmailConfig, mirror_root: &Path) -> bool {
        let acct = account_mirror_dir(mirror_root, &config.account_email);
        let Some(slot) = self.try_begin(&acct) else {
            return false;
        };
        tokio::spawn(async move {
            let email = config.account_email.clone();
            match sync_gmail_incremental(&config, &acct).await {
                Ok(d) => tracing::info!(
                    "gmail sync [{email}]: +{} -{} ~{} (full_resync={})",
                    d.added,
                    d.deleted,
                    d.relabeled,
                    d.full_resync
                ),
                Err(e) => tracing::warn!("gmail sync [{email}] failed: {e:#}"),
            }
            drop(slot);
        });
        true
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_flight_per_account() {
        let runner = GmailSyncRunner::default();
        let a = Path::new("/mirror/gmail/a@x.com");
        let b = Path::new("/mirror/gmail/b@x.com");

        let slot = runner.try_begin(a).expect("first claim wins");
        assert!(runner.is_running(a));
        assert!(runner.try_begin(a).is_none(), "second claim blocked");
        // A different account is unaffected.
        assert!(runner.try_begin(b).is_some());

        // Dropping the slot frees the account for the next run.
        drop(slot);
        assert!(!runner.is_running(a));
        assert!(runner.try_begin(a).is_some());
    }
}
