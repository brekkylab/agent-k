//! Cross-cutting helpers that don't belong to a single piece of application
//! state — e.g. one-off LLM calls that support a feature (session titling)
//! rather than a persistent agent run.

pub mod session_title;
