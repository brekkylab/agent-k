//! Permission model for agent-invokable app-control tools.
//!
//! Two orthogonal layers gate every tool call (see [`super::context::AppToolContext::authorize`]):
//!
//! * **Layer A — user permissions** ([`PermissionResolver`]): can the acting
//!   user (the session creator) do this at all? Today this wraps the existing
//!   project membership / ownership checks. When user-level RBAC lands, only
//!   this layer's implementation changes — the tool code is untouched.
//! * **Layer B — agent scope** ([`AgentPolicy`]): of the things the user could
//!   do, which is this agent allowed to do? Always a subset of the user's
//!   permissions. v1 grants read-only; the future home of this config is
//!   per-user settings.
//!
//! [`Capability`] is the stable permission vocabulary (resource + action).
//! [`Resource`] is the access target; `Some(id)` is one object, `None` is the
//! whole collection (for `list_*`). The two must agree on [`ResourceKind`],
//! which `authorize` asserts as a programming invariant.

use std::collections::HashSet;

use async_trait::async_trait;
use uuid::Uuid;

use crate::repository::AppRepository;

/// The kind of entity a capability or resource refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ResourceKind {
    Project,
    Automation,
    Session,
    User,
}

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

impl Resource {
    pub fn kind(&self) -> ResourceKind {
        match self {
            Resource::Project(_) => ResourceKind::Project,
            Resource::Automation(_) => ResourceKind::Automation,
            Resource::Session(_) => ResourceKind::Session,
            Resource::User(_) => ResourceKind::User,
        }
    }
}

/// Stable permission vocabulary: one variant per (resource, action). Membership
/// operations are project-scoped, so they map to [`ResourceKind::Project`].
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
    /// [`AgentPolicy::all`] and any future "grant everything" path can't miss a
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

    /// The resource kind this capability applies to. Used by `authorize` to
    /// assert the passed [`Resource`] matches.
    pub fn resource_kind(&self) -> ResourceKind {
        use Capability::*;
        match self {
            AutomationRead | AutomationCreate | AutomationUpdate | AutomationDelete
            | AutomationRun | AutomationRunRead => ResourceKind::Automation,
            SessionRead => ResourceKind::Session,
            // Members live on a project, not a standalone entity.
            ProjectRead | MemberRead | MemberManage => ResourceKind::Project,
            UserReadSelf | UserLookup | UserAdmin => ResourceKind::User,
        }
    }
}

/// Layer B: the set of capabilities granted to a particular agent. Always a
/// subset of what the acting user can do (Layer A intersects on top).
#[derive(Clone, Debug, Default)]
pub struct AgentPolicy {
    granted: HashSet<Capability>,
}

impl AgentPolicy {
    pub fn new(granted: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }

    /// v1 default: read-only. No create/update/run/delete/member-manage/admin.
    /// This is the fixed policy until per-user agent settings exist.
    pub fn read_only() -> Self {
        use Capability::*;
        Self::new([
            AutomationRead,
            AutomationRunRead,
            SessionRead,
            ProjectRead,
            MemberRead,
            UserReadSelf,
            UserLookup,
        ])
    }

    /// Grant every capability. Note this is the *agent scope* (Layer B) only —
    /// the resolver (Layer A) still gates each call against the user's actual
    /// permissions, so e.g. `UserAdmin` stays denied for non-admins.
    pub fn all() -> Self {
        Self::new(Capability::ALL)
    }

    /// The policy granted to `_user_id`'s agents. The single source of truth for
    /// both agent construction and the settings UI. Defaults to the full
    /// capability set for everyone; this is the seam where per-user settings
    /// (and later RBAC) will narrow it.
    pub fn for_user(_user_id: Uuid) -> Self {
        Self::all()
    }

    pub fn grants(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }

    pub fn granted(&self) -> &HashSet<Capability> {
        &self.granted
    }

    /// Granted capability names, sorted for stable presentation (settings UI).
    pub fn granted_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.granted.iter().map(|c| c.name().to_string()).collect();
        names.sort();
        names
    }
}

/// Layer A: resolves whether the acting user is permitted, independent of the
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
