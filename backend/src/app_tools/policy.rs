//! Agent scope (Layer B): which capabilities a given agent may use.
//!
//! Always a subset of what the acting user can do — the user-level resolver
//! ([`crate::authz::PermissionResolver`], Layer A) intersects on top in
//! [`super::context::AppToolContext::authorize`]. The future home of this config
//! is per-user settings.

use std::collections::HashSet;

use uuid::Uuid;

use crate::authz::Capability;

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
