use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    auth::Role,
    repository::{RepositoryError, RepositoryResult},
};

pub struct SqliteRepository {
    pub(crate) pool: SqlitePool,
}

impl SqliteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) fn now_string() -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    pub(crate) fn parse_uuid(s: String, field: &str) -> RepositoryResult<Uuid> {
        Uuid::parse_str(&s)
            .map_err(|_| RepositoryError::InvalidData(format!("invalid uuid in {field}")))
    }

    pub(crate) fn parse_timestamp(s: String, field: &str) -> RepositoryResult<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| RepositoryError::InvalidData(format!("invalid timestamp in {field}")))
    }

    pub(crate) fn parse_role(s: String, field: &str) -> RepositoryResult<Role> {
        match s.as_str() {
            "user" => Ok(Role::User),
            "admin" => Ok(Role::Admin),
            _ => Err(RepositoryError::InvalidData(format!(
                "invalid role '{s}' in {field}"
            ))),
        }
    }

    pub(crate) fn map_db_error(e: sqlx::Error, unique_field: &str) -> RepositoryError {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.message().contains("UNIQUE constraint failed") {
                return RepositoryError::UniqueViolation(unique_field.to_string());
            }
        }
        RepositoryError::Database(e)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ailoy::message::{Message, Part, Role};
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::SqliteRepository;
    use crate::{
        auth::Role as UserRole,
        repository::{NewUser, UpdateUser},
    };

    async fn make_project(pool: &sqlx::SqlitePool, owner_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        let now = SqliteRepository::now_string();
        sqlx::query("INSERT INTO projects (id, name, description, owner_id, created_at, updated_at) VALUES (?, 'Test Project', NULL, ?, ?, ?)")
            .bind(id.to_string())
            .bind(owner_id.to_string())
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn make_user(repo: &SqliteRepository, username: &str) -> Uuid {
        let u = new_user(username, UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();
        id
    }

    async fn make_repo(db_url: &str) -> SqliteRepository {
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        SqliteRepository::new(pool)
    }

    fn new_user(username: &str, role: UserRole) -> NewUser {
        NewUser {
            id: Uuid::new_v4(),
            username: username.to_string(),
            password_hash: "hash".to_string(),
            role,
            display_name: None,
            is_active: true,
        }
    }

    #[tokio::test]
    async fn session_and_messages_survive_repository_restart() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let session_id;

        {
            let repo = make_repo(&db_url).await;
            let user_id = make_user(&repo, "testuser_restart").await;
            let project_id = make_project(&repo.pool, user_id).await;
            let session = repo.create_session(project_id, user_id).await.unwrap();
            session_id = session.id;

            let msgs = vec![
                Message::new(Role::User).with_contents([Part::text("What is 1+1?")]),
                Message::new(Role::Assistant).with_contents([Part::text("1+1 equals 2.")]),
            ];
            repo.append_messages(session_id, &msgs).await.unwrap();

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
        }

        {
            let repo = make_repo(&db_url).await;

            let session = repo.get_session(session_id).await.unwrap();
            assert!(session.is_some(), "session must survive restart");

            let fetched = repo.get_messages(session_id).await.unwrap();
            assert_eq!(fetched.len(), 2);
            assert!(matches!(fetched[0].role, Role::User));
            assert!(matches!(fetched[1].role, Role::Assistant));

            let user_text = fetched[0]
                .contents
                .iter()
                .find_map(|p| p.as_text())
                .unwrap_or("");
            assert_eq!(user_text, "What is 1+1?");
        }
    }

    #[tokio::test]
    async fn delete_session_cascades_messages() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let user_id = make_user(&repo, "testuser_delete").await;
        let project_id = make_project(&repo.pool, user_id).await;
        let session = repo.create_session(project_id, user_id).await.unwrap();
        let session_id = session.id;

        repo.append_messages(
            session_id,
            &[Message::new(Role::User).with_contents([Part::text("hello")])],
        )
        .await
        .unwrap();

        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 1);

        let deleted = repo.delete_session(session_id).await.unwrap();
        assert!(deleted);

        assert_eq!(repo.get_messages(session_id).await.unwrap().len(), 0);
        assert!(repo.get_session(session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_messages_preserves_insertion_order() {
        let dir = tempdir().unwrap();
        let db_url = format!("sqlite://{}", dir.path().join("test.db").display());

        let repo = make_repo(&db_url).await;
        let user_id = make_user(&repo, "testuser_order").await;
        let project_id = make_project(&repo.pool, user_id).await;
        let session = repo.create_session(project_id, user_id).await.unwrap();
        let sid = session.id;

        let batch1 = vec![
            Message::new(Role::User).with_contents([Part::text("turn1 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn1 assistant")]),
        ];
        repo.append_messages(sid, &batch1).await.unwrap();

        let batch2 = vec![
            Message::new(Role::User).with_contents([Part::text("turn2 user")]),
            Message::new(Role::Assistant).with_contents([Part::text("turn2 assistant")]),
        ];
        repo.append_messages(sid, &batch2).await.unwrap();

        let all = repo.get_messages(sid).await.unwrap();
        assert_eq!(all.len(), 4);

        let texts: Vec<&str> = all
            .iter()
            .flat_map(|m| m.contents.iter().filter_map(|p| p.as_text()))
            .collect();

        assert_eq!(
            texts,
            [
                "turn1 user",
                "turn1 assistant",
                "turn2 user",
                "turn2 assistant"
            ]
        );
    }

    #[tokio::test]
    async fn create_and_get_user() {
        let repo = make_repo("sqlite::memory:").await;

        let u = new_user("alice", UserRole::User);
        let id = u.id;
        let created = repo.create_user(u).await.unwrap();

        assert_eq!(created.username, "alice");
        assert!(matches!(created.role, UserRole::User));
        assert!(created.is_active);

        let fetched = repo.get_user_by_id(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);

        let by_name = repo.get_user_by_username("alice").await.unwrap().unwrap();
        assert_eq!(by_name.id, id);
    }

    #[tokio::test]
    async fn duplicate_username_returns_unique_violation() {
        let repo = make_repo("sqlite::memory:").await;

        repo.create_user(new_user("bob", UserRole::User))
            .await
            .unwrap();

        let err = repo
            .create_user(new_user("bob", UserRole::Admin))
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::repository::RepositoryError::UniqueViolation(_)),
            "expected UniqueViolation, got {err}"
        );
    }

    #[tokio::test]
    async fn update_user_and_count_admins() {
        let repo = make_repo("sqlite::memory:").await;

        assert_eq!(repo.count_admins().await.unwrap(), 0);

        let u = new_user("carol", UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();

        repo.update_user(
            id,
            UpdateUser {
                role: Some(UserRole::Admin),
                display_name: Some("Carol".to_string()),
                password_hash: None,
                is_active: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(repo.count_admins().await.unwrap(), 1);

        let updated = repo.get_user_by_id(id).await.unwrap().unwrap();
        assert!(matches!(updated.role, UserRole::Admin));
        assert_eq!(updated.display_name.as_deref(), Some("Carol"));
    }

    #[tokio::test]
    async fn list_users_pagination() {
        let repo = make_repo("sqlite::memory:").await;

        for i in 0..5 {
            repo.create_user(new_user(&format!("user{i}"), UserRole::User))
                .await
                .unwrap();
        }

        let (page1, total) = repo.list_users(1, 3).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 3);

        let (page2, _) = repo.list_users(2, 3).await.unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn delete_user() {
        let repo = make_repo("sqlite::memory:").await;

        let u = new_user("dave", UserRole::User);
        let id = u.id;
        repo.create_user(u).await.unwrap();

        assert!(repo.delete_user(id).await.unwrap());
        assert!(repo.get_user_by_id(id).await.unwrap().is_none());
        assert!(!repo.delete_user(id).await.unwrap());
    }

    // -----------------------------------------------------------------------
    // automation suite
    // -----------------------------------------------------------------------

    mod automation_tests {
        use std::sync::Arc;

        use chrono::{Duration as ChronoDuration, Utc};
        use uuid::Uuid;

        use super::{make_project, make_repo, make_user};
        use crate::{
            model::{EventKind, RunStatus, TriggerKind, TriggerSpec},
            repository::SessionOrigin,
        };

        async fn fixtures(repo: &crate::repository::SqliteRepository) -> (Uuid, Uuid) {
            let user_id = make_user(repo, "automation_user").await;
            let project_id = make_project(&repo.pool, user_id).await;
            (project_id, user_id)
        }

        #[tokio::test]
        async fn automation_crud_roundtrip() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;

            let created = repo
                .create_automation(
                    project_id,
                    "demo".to_string(),
                    Some("desc".to_string()),
                    vec!["first prompt".to_string(), "second prompt".to_string()],
                    user_id,
                )
                .await
                .unwrap();

            assert_eq!(created.name, "demo");
            assert_eq!(created.prompts.len(), 2);

            let fetched = repo.get_automation(created.id).await.unwrap().unwrap();
            assert_eq!(fetched.prompts[1], "second prompt");

            let updated = repo
                .update_automation(
                    created.id,
                    Some("renamed".to_string()),
                    None,
                    Some(vec!["only one".to_string()]),
                )
                .await
                .unwrap();
            assert_eq!(updated.name, "renamed");
            assert_eq!(updated.prompts, vec!["only one"]);

            let listed = repo.list_automations_in_project(project_id).await.unwrap();
            assert_eq!(listed.len(), 1);

            assert!(repo.delete_automation(created.id).await.unwrap());
            assert!(repo.get_automation(created.id).await.unwrap().is_none());
        }

        #[tokio::test]
        async fn trigger_create_and_find_by_webhook_token_hash() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();

            let spec = TriggerSpec::Webhook {
                dedupe: Some("payload_hash".into()),
            };
            let token_hash = "abcd1234".repeat(8); // 64 chars, plausible sha256 hex
            let trigger = repo
                .create_trigger(auto.id, &spec, Some(token_hash.clone()), None)
                .await
                .unwrap();
            assert_eq!(trigger.kind, TriggerKind::Webhook);
            assert!(trigger.enabled);

            let found = repo
                .find_trigger_by_webhook_token_hash(&token_hash)
                .await
                .unwrap()
                .expect("should find by token hash");
            assert_eq!(found.id, trigger.id);

            assert!(
                repo.find_trigger_by_webhook_token_hash("nope")
                    .await
                    .unwrap()
                    .is_none()
            );

            // duplicate hash → unique violation
            let err = repo
                .create_trigger(auto.id, &spec, Some(token_hash.clone()), None)
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                crate::repository::RepositoryError::UniqueViolation(_)
            ));
        }

        #[tokio::test]
        async fn list_due_cron_triggers_filters_by_time_and_kind() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();

            let now = Utc::now();
            // Due cron trigger
            let due_spec = TriggerSpec::Cron {
                expr: "* * * * *".into(),
                tz: None,
            };
            let due = repo
                .create_trigger(auto.id, &due_spec, None, Some(now - ChronoDuration::seconds(10)))
                .await
                .unwrap();

            // Future cron trigger — not due yet
            repo.create_trigger(
                auto.id,
                &due_spec,
                None,
                Some(now + ChronoDuration::hours(1)),
            )
            .await
            .unwrap();

            // Webhook trigger should be excluded
            repo.create_trigger(
                auto.id,
                &TriggerSpec::Webhook { dedupe: None },
                Some("hash_aaa".repeat(8)),
                None,
            )
            .await
            .unwrap();

            let due_list = repo.list_due_cron_triggers(now).await.unwrap();
            assert_eq!(due_list.len(), 1);
            assert_eq!(due_list[0].id, due.id);
        }

        #[tokio::test]
        async fn claim_due_run_picks_one_at_a_time() {
            let repo = Arc::new(make_repo("sqlite::memory:").await);
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let now = Utc::now();
            let run = repo
                .create_run(auto.id, None, session.id, now - ChronoDuration::seconds(1), None)
                .await
                .unwrap();

            // Two concurrent claim attempts — only one should win.
            let lease_until = now + ChronoDuration::minutes(5);
            let r1 = repo.clone();
            let r2 = repo.clone();
            let h1 = tokio::spawn(async move { r1.claim_due_run(now, lease_until).await });
            let h2 = tokio::spawn(async move { r2.claim_due_run(now, lease_until).await });

            let a = h1.await.unwrap().unwrap();
            let b = h2.await.unwrap().unwrap();

            let winners = [a.is_some(), b.is_some()].iter().filter(|x| **x).count();
            assert_eq!(winners, 1, "exactly one claim must succeed");

            let claimed = repo.get_run(run.id).await.unwrap().unwrap();
            assert_eq!(claimed.status, RunStatus::Running);
            assert!(claimed.lease_until.is_some());

            // No further claims should pick it up.
            let none_left = repo.claim_due_run(now, lease_until).await.unwrap();
            assert!(none_left.is_none());
        }

        #[tokio::test]
        async fn claim_skips_future_scheduled_runs() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let now = Utc::now();
            let future = now + ChronoDuration::minutes(10);
            repo.create_run(auto.id, None, session.id, future, None)
                .await
                .unwrap();

            assert!(
                repo.claim_due_run(now, now + ChronoDuration::minutes(1))
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        #[tokio::test]
        async fn renew_lease_and_reap_expired() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let claim_time = Utc::now();
            let scheduled_for = claim_time - ChronoDuration::seconds(1);
            repo.create_run(auto.id, None, session.id, scheduled_for, None)
                .await
                .unwrap();

            // Claim with a short lease.
            let short_lease = claim_time + ChronoDuration::seconds(1);
            let claimed = repo
                .claim_due_run(claim_time, short_lease)
                .await
                .unwrap()
                .unwrap();

            // Renew while still running.
            let new_lease = claim_time + ChronoDuration::minutes(5);
            assert!(repo.renew_lease(claimed.id, new_lease).await.unwrap());

            // Mark as failed → lease no longer renewable.
            repo.update_run_status(claimed.id, RunStatus::Failed, true)
                .await
                .unwrap();
            assert!(!repo.renew_lease(claimed.id, new_lease).await.unwrap());

            // Now set up another expired-lease run for reaping.
            let session2 = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            repo.create_run(auto.id, None, session2.id, scheduled_for, None)
                .await
                .unwrap();
            let expired_lease = claim_time + ChronoDuration::seconds(1);
            let run2 = repo
                .claim_due_run(claim_time, expired_lease)
                .await
                .unwrap()
                .unwrap();

            // Reaper: now is past expired_lease, so this run should be reaped.
            let after_expiry = expired_lease + ChronoDuration::seconds(5);
            let reaped = repo.reap_expired_leases(after_expiry).await.unwrap();
            assert_eq!(reaped, vec![run2.id]);

            let reset = repo.get_run(run2.id).await.unwrap().unwrap();
            assert_eq!(reset.status, RunStatus::Queued);
            assert!(reset.lease_until.is_none());

            // Reaper must record a lease_lost event in the same transaction.
            let events = repo.list_events_for_run(run2.id).await.unwrap();
            assert!(
                events.iter().any(|e| e.kind == EventKind::LeaseLost),
                "expected lease_lost event after reap, got {:?}",
                events.iter().map(|e| e.kind).collect::<Vec<_>>()
            );
        }

        #[tokio::test]
        async fn reap_all_running_requeues_unconditionally() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let scheduled_for = Utc::now() - ChronoDuration::seconds(1);
            repo.create_run(auto.id, None, session.id, scheduled_for, None)
                .await
                .unwrap();

            // Claim with a lease far in the future — would not be reaped by
            // reap_expired_leases yet.
            let claimed = repo
                .claim_due_run(Utc::now(), Utc::now() + ChronoDuration::hours(1))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(claimed.status, RunStatus::Running);

            // Boot reap should still pick it up.
            let reaped = repo.reap_all_running().await.unwrap();
            assert_eq!(reaped, vec![claimed.id]);
            let reset = repo.get_run(claimed.id).await.unwrap().unwrap();
            assert_eq!(reset.status, RunStatus::Queued);
            assert!(reset.lease_until.is_none());

            // Same-tx lease_lost event must be present.
            let events = repo.list_events_for_run(claimed.id).await.unwrap();
            assert!(events.iter().any(|e| e.kind == EventKind::LeaseLost));

            // Second call is a no-op.
            assert!(repo.reap_all_running().await.unwrap().is_empty());
        }

        #[tokio::test]
        async fn finalize_run_writes_event_and_status_atomically() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let run = repo
                .create_run(auto.id, None, session.id, Utc::now(), None)
                .await
                .unwrap();
            let claimed = repo
                .claim_due_run(Utc::now(), Utc::now() + ChronoDuration::minutes(3))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(claimed.id, run.id);

            let written = repo
                .finalize_run(run.id, RunStatus::Succeeded, EventKind::Succeeded, 1, None)
                .await
                .unwrap();
            assert!(written, "expected finalize_run to write on running row");

            // Status flipped + lease cleared + succeeded event present.
            let finalized = repo.get_run(run.id).await.unwrap().unwrap();
            assert_eq!(finalized.status, RunStatus::Succeeded);
            assert!(finalized.lease_until.is_none());
            let events = repo.list_events_for_run(run.id).await.unwrap();
            assert!(events.iter().any(|e| e.kind == EventKind::Succeeded));

            // Second call (status no longer 'running') is a no-op: returns false
            // and does not add another event.
            let written_again = repo
                .finalize_run(run.id, RunStatus::Failed, EventKind::Failed, 1, None)
                .await
                .unwrap();
            assert!(!written_again);
            let events_after = repo.list_events_for_run(run.id).await.unwrap();
            assert_eq!(events.len(), events_after.len());
            let still = repo.get_run(run.id).await.unwrap().unwrap();
            assert_eq!(still.status, RunStatus::Succeeded);
        }

        #[tokio::test]
        async fn events_append_and_list_in_order() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let session = repo
                .create_session_with_origin(project_id, user_id, SessionOrigin::Automation)
                .await
                .unwrap();
            let run = repo
                .create_run(auto.id, None, session.id, Utc::now(), None)
                .await
                .unwrap();

            repo.append_event(run.id, EventKind::Triggered, 1, None)
                .await
                .unwrap();
            repo.append_event(run.id, EventKind::Queued, 1, None)
                .await
                .unwrap();
            repo.append_event(
                run.id,
                EventKind::StepStarted,
                1,
                Some(&serde_json::json!({ "step_index": 0 })),
            )
            .await
            .unwrap();

            let events = repo.list_events_for_run(run.id).await.unwrap();
            assert_eq!(events.len(), 3);
            assert_eq!(events[0].kind, EventKind::Triggered);
            assert_eq!(events[2].kind, EventKind::StepStarted);
            assert_eq!(
                events[2].payload.as_ref().unwrap()["step_index"]
                    .as_i64()
                    .unwrap(),
                0
            );
        }

        #[tokio::test]
        async fn trigger_spec_roundtrip_through_db() {
            let repo = make_repo("sqlite::memory:").await;
            let (project_id, user_id) = fixtures(&repo).await;
            let auto = repo
                .create_automation(project_id, "a".into(), None, vec!["p".into()], user_id)
                .await
                .unwrap();
            let original = TriggerSpec::Cron {
                expr: "*/15 * * * *".into(),
                tz: Some("UTC".into()),
            };
            let t = repo
                .create_trigger(auto.id, &original, None, None)
                .await
                .unwrap();

            let reloaded = TriggerSpec::from_db(t.kind, &t.spec_json).unwrap();
            match reloaded {
                TriggerSpec::Cron { expr, tz } => {
                    assert_eq!(expr, "*/15 * * * *");
                    assert_eq!(tz.as_deref(), Some("UTC"));
                }
                _ => panic!("expected Cron variant"),
            }
        }
    }
}
