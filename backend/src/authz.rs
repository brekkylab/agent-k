//! User-level authorization (Layer A) — shared across handlers and agent tools.
//!
//! [`PermissionResolver`] answers "can this user do X to this resource?" using
//! the project membership / ownership model. It is deliberately independent of
//! agent tooling — the agent layer ([`crate::app_tools::AgentPolicy`], Layer B)
//! narrows on top of it. This is the single seam where user-level RBAC will plug
//! in: swap the implementation, keep the `(actor, capability, resource)`
//! signature.
//!
//! [`Capability`] is the stable permission vocabulary (resource + action);
//! [`Resource`] is the access target (`Some(id)` = one object, `None` = the
//! collection, for `list_*`). The resolver's `(cap, resource)` match only
//! accepts valid pairings — a mismatched pair (e.g. an automation capability
//! with a session resource) falls through to `false`.

use async_trait::async_trait;
use uuid::Uuid;

use crate::repository::AppRepository;

/// The concrete target of a permission check.
///
/// `Some(id)` is object-level (one specific entity); `None` is
/// collection/type-level (e.g. "list automations the user can see").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resource {
    Project(Option<Uuid>),
    Automation(Option<Uuid>),
    Session(Option<Uuid>),
    User(Option<Uuid>),
}

/// Stable permission vocabulary: one variant per (resource, action). Membership
/// operations are project-scoped, so they pair with `Resource::Project`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Capability {
    // Automation configuration
    AutomationRead,
    AutomationCreate,
    AutomationUpdate,
    AutomationDelete,
    // Automation execution
    AutomationRun,
    AutomationRunRead,
    // Session
    SessionRead,
    // Project & members
    ProjectRead,
    MemberRead,
    MemberManage,
    // User
    UserReadSelf,
    UserLookup,
    UserAdmin,
}

impl Capability {
    /// Every capability, in display order. Keep in sync with the enum so
    /// `AgentPolicy::all` and any future "grant everything" path can't miss a
    /// variant.
    pub const ALL: [Capability; 13] = [
        Capability::AutomationRead,
        Capability::AutomationCreate,
        Capability::AutomationUpdate,
        Capability::AutomationDelete,
        Capability::AutomationRun,
        Capability::AutomationRunRead,
        Capability::SessionRead,
        Capability::ProjectRead,
        Capability::MemberRead,
        Capability::MemberManage,
        Capability::UserReadSelf,
        Capability::UserLookup,
        Capability::UserAdmin,
    ];

    /// Stable wire/display name (also what the settings UI shows). Keep these
    /// strings stable — they may be persisted as per-user grants later.
    pub fn name(&self) -> &'static str {
        use Capability::*;
        match self {
            AutomationRead => "automation.read",
            AutomationCreate => "automation.create",
            AutomationUpdate => "automation.update",
            AutomationDelete => "automation.delete",
            AutomationRun => "automation.run",
            AutomationRunRead => "automation.run_read",
            SessionRead => "session.read",
            ProjectRead => "project.read",
            MemberRead => "member.read",
            MemberManage => "member.manage",
            UserReadSelf => "user.read_self",
            UserLookup => "user.lookup",
            UserAdmin => "user.admin",
        }
    }
}

/// Layer A: resolves whether the acting user is permitted, independent of any
/// agent. The seam for future user-level RBAC — swap the implementation, keep
/// the `(actor, capability, resource)` signature.
#[async_trait]
pub trait PermissionResolver: Send + Sync {
    async fn user_can(&self, actor: Uuid, cap: Capability, resource: &Resource) -> bool;
}

/// Default resolver backed by the current project membership / ownership model.
pub struct RepoPermissionResolver {
    repository: AppRepository,
}

impl RepoPermissionResolver {
    pub fn new(repository: AppRepository) -> Self {
        Self { repository }
    }

    async fn is_member(&self, actor: Uuid, project_id: Uuid) -> bool {
        self.repository
            .user_in_project(actor, project_id)
            .await
            .unwrap_or(false)
    }

    async fn is_owner(&self, actor: Uuid, project_id: Uuid) -> bool {
        self.repository
            .user_is_project_owner(actor, project_id)
            .await
            .unwrap_or(false)
    }

    /// Resolve an automation to its project, then check membership.
    async fn member_via_automation(&self, actor: Uuid, automation_id: Uuid) -> bool {
        match self.repository.get_automation(automation_id).await {
            Ok(Some(a)) => self.is_member(actor, a.project_id).await,
            _ => false,
        }
    }
}

#[async_trait]
impl PermissionResolver for RepoPermissionResolver {
    async fn user_can(&self, actor: Uuid, cap: Capability, resource: &Resource) -> bool {
        use Capability::*;
        match (cap, resource) {
            // ── Project / members ───────────────────────────────────────────
            // Collection reads are scoped at query time to the user's own
            // projects, so the gate itself is permissive (`None` => true).
            (ProjectRead | MemberRead, Resource::Project(None)) => true,
            (ProjectRead | MemberRead, Resource::Project(Some(pid))) => {
                self.is_member(actor, *pid).await
            }
            (MemberManage, Resource::Project(Some(pid))) => self.is_owner(actor, *pid).await,
            (MemberManage, Resource::Project(None)) => false,

            // ── Automation ──────────────────────────────────────────────────
            (AutomationRead | AutomationRunRead, Resource::Automation(None)) => true,
            (AutomationRead | AutomationRunRead, Resource::Automation(Some(id))) => {
                self.member_via_automation(actor, *id).await
            }
            // Creating an automation is a collection-level op; it always targets
            // the agent's own project (where the actor is a member by
            // construction), so the collection form is permitted here and the
            // tool scopes it to that project.
            (AutomationCreate, Resource::Automation(None)) => true,
            // Update/delete/run need a specific automation; the collection form
            // is meaningless and denied.
            (
                AutomationUpdate | AutomationDelete | AutomationRun,
                Resource::Automation(None),
            ) => false,
            (
                AutomationCreate | AutomationUpdate | AutomationDelete | AutomationRun,
                Resource::Automation(Some(id)),
            ) => self.member_via_automation(actor, *id).await,

            // ── Session (read-only) ─────────────────────────────────────────
            (SessionRead, Resource::Session(None)) => true,
            (SessionRead, Resource::Session(Some(id))) => self
                .repository
                .get_session_with_authz(*id, actor)
                .await
                .map(|o| o.is_some())
                .unwrap_or(false),

            // ── User ────────────────────────────────────────────────────────
            (UserReadSelf, Resource::User(Some(id))) => *id == actor,
            // Directory lookup of a basic profile; mirrors how add_member lets a
            // user resolve another by username.
            (UserLookup, Resource::User(_)) => true,
            // Admin user management is never permitted by the default resolver.
            (UserAdmin, _) => false,

            // Any mismatched / unhandled pairing is denied.
            _ => false,
        }
    }
}
