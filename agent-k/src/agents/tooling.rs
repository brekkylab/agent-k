//! Injection hook for host-application tools.
//!
//! Agents in this crate are embedded in-process by the backend. The backend
//! wants to hand an agent a set of tools that drive the app itself (read
//! automations, list sessions, …). Those tools depend on the backend's
//! repository/types, which this crate must not know about — so the backend
//! passes them in as [`ExtraTools`]: a list of [`ToolDesc`]s to advertise on
//! the agent spec, plus a `register` closure that installs the matching
//! [`ToolFunc`]s onto the agent's [`ToolProvider`].
//!
//! This mirrors the corpus-tool pattern (`register_corpus_tools` +
//! `try_with_provider_and_runenv`): a tool must be declared on the spec **and**
//! registered on the provider, or `ToolProvider::provide` fails to resolve it.

use std::sync::Arc;

use ailoy::tool::{ToolDesc, ToolProvider};

/// Extra, host-supplied tools to attach to an agent at build time.
#[derive(Clone)]
pub struct ExtraTools {
    /// Tool descriptions to advertise on the agent's spec/builder. The model
    /// only sees (and can call) tools listed here.
    pub descs: Vec<ToolDesc>,
    /// Installs the [`ToolFunc`](ailoy::tool::ToolFunc)s backing `descs` onto a
    /// provider's tool registry. Called against a clone of the default provider
    /// so built-ins (web search, etc.) remain available.
    pub register: Arc<dyn Fn(&mut ToolProvider) + Send + Sync>,
}

impl ExtraTools {
    pub fn new(
        descs: Vec<ToolDesc>,
        register: impl Fn(&mut ToolProvider) + Send + Sync + 'static,
    ) -> Self {
        Self {
            descs,
            register: Arc::new(register),
        }
    }

    /// Whether there is anything to inject (empty descs = no-op).
    pub fn is_empty(&self) -> bool {
        self.descs.is_empty()
    }
}
