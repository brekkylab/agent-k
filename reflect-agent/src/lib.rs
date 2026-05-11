//! `reflect-agent` — single lead agent built via `ailoy::agent::AgentBuilder`,
//! with a deterministic post-hoc verify pass over each turn's history slice.

mod agent;
mod provider;
mod verify;

pub use agent::{DEFAULT_MODEL, build_agent, run_with_verify};
pub use provider::register_provider_from_env;
pub use verify::{BashFailureReason, Issue, VerifyConfig, VerifyReport, verify_run};
