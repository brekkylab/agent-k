//! Agent-invokable tools that drive this app's own features.
//!
//! These are in-process [`ailoy`] tools (not HTTP calls) handed to an embedded
//! agent — currently Buddy, the app-control surface. Each tool runs the same
//! repository operations and authorization the REST handlers use, judged
//! against the session creator's identity. See [`policy`] for the two-layer
//! permission model and [`context::AppToolContext`] for the per-call gate.
//!
//! v1 exposes **read-only** tools. Write/run/manage tools are deferred until
//! per-user agent permission settings exist.

mod automation;
mod context;
mod policy;
mod project;
mod session;
mod user;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_k::agents::ExtraTools;
use ailoy::tool::{ToolDesc, ToolFunc};

pub use context::AppToolContext;
pub use policy::{
    AgentPolicy, Capability, PermissionResolver, RepoPermissionResolver, Resource, ResourceKind,
};

/// One registered app-control tool: its primary capability (for policy-gated
/// advertisement), its description (shown to the model), and its function.
pub(crate) struct AppTool {
    pub cap: Capability,
    pub desc: ToolDesc,
    pub func: ToolFunc,
}

impl AppTool {
    pub(crate) fn new(cap: Capability, desc: ToolDesc, func: ToolFunc) -> Self {
        Self { cap, desc, func }
    }
}

/// Build the read-only (v1) app-control toolset for `ctx`, filtered by the
/// agent's policy. Returns [`ExtraTools`] ready to inject into an agent builder:
/// only tools whose capability the policy grants are advertised and registered.
pub fn build_app_tools(ctx: Arc<AppToolContext>) -> ExtraTools {
    let mut all: Vec<AppTool> = Vec::new();
    all.extend(automation::tools(&ctx));
    all.extend(session::tools(&ctx));
    all.extend(project::tools(&ctx));
    all.extend(user::tools(&ctx));

    let granted: Vec<AppTool> = all
        .into_iter()
        .filter(|t| ctx.agent_policy.grants(t.cap))
        .collect();

    let descs: Vec<ToolDesc> = granted.iter().map(|t| t.desc.clone()).collect();
    let funcs: Vec<(String, ToolFunc)> = granted
        .iter()
        .map(|t| (t.desc.name.clone(), t.func.clone()))
        .collect();

    ExtraTools::new(descs, move |provider| {
        for (name, func) in &funcs {
            provider.insert_func(name.clone(), func.clone());
        }
    })
}
