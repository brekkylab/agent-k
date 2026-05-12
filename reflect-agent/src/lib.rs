//! `reflect-agent` — single lead agent built via `ailoy::agent::AgentBuilder`,
//! with a deterministic post-hoc verify pass and an optional LLM-driven
//! reflect gate that escalates low-confidence stops to a stronger model.

mod agent;
mod provider;
mod reflect;
mod verify;

pub use agent::{
    DEFAULT_MODEL, ForcedReflectOutcome, build_agent, build_agent_with_mode,
    run_with_forced_reflect, run_with_verify,
};
pub use provider::register_provider_from_env;
pub use reflect::{
    DEFAULT_ESCALATE_MODEL, DEFAULT_REFLECT_MODEL, LOW_CONFIDENCE_THRESHOLD, ReflectMode,
    ReflectVerdict, RETRY_BUDGET, reflect_call,
};
pub use verify::{BashFailureReason, Issue, VerifyConfig, VerifyReport, verify_run};
