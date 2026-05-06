//! `reflect-agent` — single lead agent built via `ailoy::agent::AgentBuilder`,
//! intended as the home for the verify gate (Phase 1) and reflect gate (Phase 2)
//! described in the agent-loop patterns report.
//!
//! At this stage the crate provides only the agent construction path. Verify
//! and reflect gates will be added in follow-up commits.

mod agent;
mod provider;
mod verify;

pub use agent::{DEFAULT_MODEL, build_agent, run_with_verify};
pub use provider::register_provider_from_env;
pub use verify::{BashFailureReason, Issue, VerifyConfig, VerifyReport, verify_run};
