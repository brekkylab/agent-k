//! Agent scope (Layer B): which capabilities a given agent may use.
//!
//! Always a subset of what the acting user can do — the user-level resolver
//! ([`crate::authz::PermissionResolver`], Layer A) intersects on top in
//! [`super::context::AppToolContext::authorize`]. The live policy is computed
//! per (user, project) via [`AgentPolicy::effective`] from the project ceiling
//! and the member's grant.

use std::collections::HashSet;

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

    /// Read-only preset: grants reads only — no create/update/run/delete/
    /// member-manage/admin. Used as a sensible fallback/default; the live policy
    /// is normally `effective(member, ceiling)` from per-(user, project) settings.
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

    /// Parse a stored JSON array of capability names into a set. Unknown names
    /// are dropped. `None` (unset) yields `None` so callers can apply their own
    /// default. A malformed string is treated as an empty grant (logged).
    pub fn from_stored(json: Option<&str>) -> Option<Self> {
        let json = json?;
        match serde_json::from_str::<Vec<String>>(json) {
            Ok(names) => Some(Self::new(
                names.iter().filter_map(|n| Capability::from_name(n)),
            )),
            Err(e) => {
                tracing::warn!("invalid agent_capabilities JSON, treating as empty: {e}");
                Some(Self::new([]))
            }
        }
    }

    /// The effective agent policy = member grant ∩ project ceiling (Layer B).
    /// `member` / `ceiling` are the stored JSON arrays (`None` = unset):
    ///   ceiling unset → no project limit (all capabilities)
    ///   member unset  → inherit the (possibly all) ceiling
    pub fn effective(member: Option<&str>, ceiling: Option<&str>) -> Self {
        let ceiling = Self::from_stored(ceiling).unwrap_or_else(Self::all);
        let member = Self::from_stored(member).unwrap_or_else(|| ceiling.clone());
        Self::new(member.granted.intersection(&ceiling.granted).copied())
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
