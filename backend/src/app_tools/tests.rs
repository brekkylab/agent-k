//! Permission-gate tests for app-control tools: the two layers (agent policy ∩
//! user permission) and the capability/resource kind guard.

use std::sync::Arc;

use uuid::Uuid;

use crate::app_tools::context::AppToolContext;
use crate::app_tools::policy::{
    AgentPolicy, Capability, PermissionResolver, RepoPermissionResolver, Resource,
};
use crate::auth::Role;
use crate::repository::{AppRepository, NewUser};

async fn repo() -> AppRepository {
    // Temp-file DB: a shared on-disk SQLite avoids the per-connection isolation
    // of `:memory:` pools, and create_repository runs migrations.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app_tools_test.db");
    // Leak the tempdir so the file outlives this call for the test's duration.
    std::mem::forget(dir);
    crate::repository::create_repository(&format!("sqlite://{}", path.display()))
        .await
        .unwrap()
}

async fn make_user(repo: &AppRepository, username: &str) -> Uuid {
    let new_user = NewUser {
        id: Uuid::new_v4(),
        username: username.to_string(),
        password_hash: "hash".into(),
        role: Role::User,
        display_name: None,
        is_active: true,
        preferred_language: "en".into(),
    };
    let (user, _project) = repo
        .create_user_with_personal_project(new_user)
        .await
        .unwrap();
    user.id
}

fn ctx(
    repo: &AppRepository,
    policy: AgentPolicy,
    actor: Uuid,
    project_id: Uuid,
) -> AppToolContext {
    let resolver: Arc<dyn PermissionResolver> =
        Arc::new(RepoPermissionResolver::new(repo.clone()));
    AppToolContext {
        repository: repo.clone(),
        resolver,
        agent_policy: policy,
        acting_user_id: actor,
        project_id,
        session_id: Uuid::new_v4(),
    }
}

/// Layer B: the read-only policy grants reads and denies writes regardless of
/// the underlying user permission.
#[tokio::test]
async fn read_only_policy_denies_writes() {
    let repo = repo().await;
    let owner = make_user(&repo, "owner").await;
    let project = repo.list_projects_for_user(owner).await.unwrap()[0].id;
    let c = ctx(&repo, AgentPolicy::read_only(), owner, project);

    // Granted read capability passes both layers.
    assert!(
        c.authorize(Capability::AutomationRead, Resource::Automation(None))
            .await
    );
    // Write capability is not in the read-only policy → denied at Layer B even
    // though the owner could otherwise do it.
    assert!(
        !c.authorize(Capability::AutomationCreate, Resource::Automation(None))
            .await
    );
    assert!(
        !c.authorize(Capability::MemberManage, Resource::Project(Some(project)))
            .await
    );
}

/// Layer A: a non-member is denied object-level reads even when the policy
/// grants the capability.
#[tokio::test]
async fn non_member_denied_object_reads() {
    let repo = repo().await;
    let owner = make_user(&repo, "owner2").await;
    let outsider = make_user(&repo, "outsider").await;
    let project = repo.list_projects_for_user(owner).await.unwrap()[0].id;

    let automation = repo
        .create_automation(
            project,
            "nightly".into(),
            None,
            vec!["do it".into()],
            None,
            None,
            owner,
        )
        .await
        .unwrap();

    let owner_ctx = ctx(&repo, AgentPolicy::read_only(), owner, project);
    let outsider_ctx = ctx(&repo, AgentPolicy::read_only(), outsider, project);

    // Owner (a member) can read the specific automation; outsider cannot.
    assert!(
        owner_ctx
            .authorize(Capability::AutomationRead, Resource::Automation(Some(automation.id)))
            .await
    );
    assert!(
        !outsider_ctx
            .authorize(Capability::AutomationRead, Resource::Automation(Some(automation.id)))
            .await
    );

    // Project membership reads mirror the same split.
    assert!(
        owner_ctx
            .authorize(Capability::ProjectRead, Resource::Project(Some(project)))
            .await
    );
    assert!(
        !outsider_ctx
            .authorize(Capability::ProjectRead, Resource::Project(Some(project)))
            .await
    );
}

/// Self-profile reads are scoped to the acting user.
#[tokio::test]
async fn user_read_self_is_self_only() {
    let repo = repo().await;
    let me = make_user(&repo, "me").await;
    let other = make_user(&repo, "other").await;
    let project = repo.list_projects_for_user(me).await.unwrap()[0].id;
    let c = ctx(&repo, AgentPolicy::read_only(), me, project);

    assert!(
        c.authorize(Capability::UserReadSelf, Resource::User(Some(me)))
            .await
    );
    assert!(
        !c.authorize(Capability::UserReadSelf, Resource::User(Some(other)))
            .await
    );
}

/// The capability/resource kind guard rejects mismatched pairings (release-mode
/// backstop returns false; debug builds would assert).
#[tokio::test]
#[cfg(not(debug_assertions))]
async fn kind_mismatch_is_denied() {
    let repo = repo().await;
    let owner = make_user(&repo, "owner3").await;
    let project = repo.list_projects_for_user(owner).await.unwrap()[0].id;
    let c = ctx(&repo, AgentPolicy::new([Capability::AutomationRead]), owner, project);

    // AutomationRead applied to a Session resource: kinds disagree → denied.
    assert!(
        !c.authorize(Capability::AutomationRead, Resource::Session(None))
            .await
    );
}

/// Under the full default policy, write/run capabilities resolve to project
/// membership (create/update/run/delete) and member-manage to ownership.
#[tokio::test]
async fn full_policy_write_paths() {
    let repo = repo().await;
    let owner = make_user(&repo, "wowner").await;
    let member = make_user(&repo, "wmember").await;
    let project = repo.list_projects_for_user(owner).await.unwrap()[0].id;
    repo.add_project_member(project, member).await.unwrap();
    let automation = repo
        .create_automation(project, "a".into(), None, vec!["go".into()], None, None, owner)
        .await
        .unwrap();

    let owner_ctx = ctx(&repo, AgentPolicy::all(), owner, project);
    let member_ctx = ctx(&repo, AgentPolicy::all(), member, project);

    // Owner: collection-create, run/delete the automation, and manage members.
    assert!(owner_ctx.authorize(Capability::AutomationCreate, Resource::Automation(None)).await);
    assert!(owner_ctx.authorize(Capability::AutomationRun, Resource::Automation(Some(automation.id))).await);
    assert!(owner_ctx.authorize(Capability::AutomationDelete, Resource::Automation(Some(automation.id))).await);
    assert!(owner_ctx.authorize(Capability::MemberManage, Resource::Project(Some(project))).await);

    // Non-owner member: may run/delete automations in their project, but member
    // management is owner-only.
    assert!(member_ctx.authorize(Capability::AutomationDelete, Resource::Automation(Some(automation.id))).await);
    assert!(!member_ctx.authorize(Capability::MemberManage, Resource::Project(Some(project))).await);
}

#[test]
fn all_policy_grants_every_capability() {
    let p = AgentPolicy::all();
    for cap in Capability::ALL {
        assert!(p.grants(cap), "expected all() to grant {cap:?}");
    }
}

#[test]
fn read_only_policy_grants_expected_set() {
    let p = AgentPolicy::read_only();
    for cap in [
        Capability::AutomationRead,
        Capability::AutomationRunRead,
        Capability::SessionRead,
        Capability::ProjectRead,
        Capability::MemberRead,
        Capability::UserReadSelf,
        Capability::UserLookup,
    ] {
        assert!(p.grants(cap), "expected read-only policy to grant {cap:?}");
    }
    for cap in [
        Capability::AutomationCreate,
        Capability::AutomationUpdate,
        Capability::AutomationRun,
        Capability::AutomationDelete,
        Capability::MemberManage,
        Capability::UserAdmin,
    ] {
        assert!(!p.grants(cap), "expected read-only policy to deny {cap:?}");
    }
}
