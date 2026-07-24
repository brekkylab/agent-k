//! Model catalog, per-agent recommendation chains, and model resolution.
//!
//! Model ids follow the ailoy `provider/model-id` convention (e.g.
//! `"anthropic/claude-sonnet-5"`). A provider is *available* when its API key
//! env var is set, which ailoy reflects by registering a `provider/*` glob on
//! the process-wide `"default"` lang-model provider. We treat "the glob
//! resolves" as the availability signal — it is synchronous and makes no
//! network call.
//!
//! Because ailoy resolves any `provider/*` id, a model does not have to be
//! catalogued to run; the catalog only drives what the picker shows and what
//! each agent recommends.
//!
//! Tiers (`light` / `standard` / `max`) only group models in the picker UI;
//! a tier is never selected directly. Each agent type has an ordered, provider
//! -diverse recommendation chain; the first entry's tier is its "recommended
//! tier", and every entry is highlighted as a recommended model.

use ailoy::lang_model::get_lm_providers;
use schemars::JsonSchema;
use serde::Serialize;

/// Capability tier — a display-only grouping in the model picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Light,
    Standard,
    Max,
}

/// Product-level agent surface that drives a session — selects the
/// recommendation chain shown in the picker.
///
/// Mirrors the session-creation `AgentType` in [`crate::router::session`]; keep
/// the two variant sets in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Coworker,
    DeepResearch,
}

impl AgentType {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentType::Coworker => "coworker",
            AgentType::DeepResearch => "deep-research",
        }
    }

    /// Parse a stored/request value; unknown values → `None`.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "coworker" => Some(AgentType::Coworker),
            "deep-research" => Some(AgentType::DeepResearch),
            _ => None,
        }
    }

    pub const ALL: [AgentType; 2] = [AgentType::Coworker, AgentType::DeepResearch];

    /// Ordered, provider-diverse recommendation chain. The last entry is the
    /// terminal default (returned even if its provider is unavailable).
    pub fn chain(self) -> &'static [&'static str] {
        match self {
            // Coworker: a balanced "standard" general-assistant chain.
            AgentType::Coworker => &[
                "openai/gpt-5.6-terra",
                "anthropic/claude-sonnet-5",
                "google/gemini-3.5-flash",
                "moonshotai/kimi-k3",
            ],
            // Deep Research: a "max" chain for multi-hop research.
            AgentType::DeepResearch => &[
                "openai/gpt-5.6-sol",
                "google/gemini-3.5-flash",
                "anthropic/claude-fable-5",
                "moonshotai/kimi-k3",
            ],
        }
    }
}

/// A catalogued model. `id` is the full ailoy `provider/model-id`
/// (e.g. `"anthropic/claude-sonnet-5"`).
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub tier: ModelTier,
}

/// The full advertised catalog. `GET /models` filters/annotates this by
/// runtime provider availability.
pub const CATALOG: &[ModelInfo] = &[
    // ── light ──────────────────────────────────────────────────────────────
    ModelInfo {
        id: "openai/gpt-5.6-luna",
        label: "GPT-5.6 Luna",
        tier: ModelTier::Light,
    },
    ModelInfo {
        id: "anthropic/claude-haiku-4-5",
        label: "Claude Haiku 4.5",
        tier: ModelTier::Light,
    },
    ModelInfo {
        id: "google/gemini-3.1-flash-lite",
        label: "Gemini 3.1 Flash-Lite",
        tier: ModelTier::Light,
    },
    // ── standard ─────────────────────────────────────────────────────────────
    ModelInfo {
        id: "openai/gpt-5.6-terra",
        label: "GPT-5.6 Terra",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "anthropic/claude-sonnet-5",
        label: "Claude Sonnet 5",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "google/gemini-3-flash-preview",
        label: "Gemini 3 Flash",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "moonshotai/kimi-k3",
        label: "Kimi K3",
        tier: ModelTier::Standard,
    },
    // ── max ────────────────────────────────────────────────────────────────
    ModelInfo {
        id: "openai/gpt-5.6-sol",
        label: "GPT-5.6 Sol",
        tier: ModelTier::Max,
    },
    ModelInfo {
        id: "google/gemini-3.5-flash",
        label: "Gemini 3.5 Flash",
        tier: ModelTier::Max,
    },
    ModelInfo {
        id: "anthropic/claude-opus-4-8",
        label: "Claude Opus 4.8",
        tier: ModelTier::Max,
    },
    ModelInfo {
        id: "anthropic/claude-fable-5",
        label: "Claude Fable 5",
        tier: ModelTier::Max,
    },
];

/// Whether the model's provider is registered (API key env var set).
/// Synchronous, no network — the same lookup `Agent` construction performs.
/// Works for any id, catalogued or not, so callers can judge an arbitrary pin.
pub fn provider_available(model_id: &str) -> bool {
    get_lm_providers()
        .get("default")
        .is_some_and(|p| p.get(model_id).is_some())
}

/// Resolve a session's model via the agent_type's built-in chain (see
/// [`resolve_model_in`]). `agent_type` defaults to Coworker when absent/unknown.
pub fn resolve_model(agent_type: Option<&str>, pin: Option<&str>) -> String {
    let agent = agent_type
        .and_then(AgentType::from_str)
        .unwrap_or(AgentType::Coworker);
    resolve_model_in(agent.chain(), pin)
}

/// Resolve within an explicit chain: an available pin, else the first available
/// chain entry, else the chain's last entry (terminal default, even if unavailable).
pub fn resolve_model_in<S: AsRef<str>>(chain: &[S], pin: Option<&str>) -> String {
    if let Some(pin) = pin.filter(|&p| !p.is_empty() && provider_available(p)) {
        return pin.to_string();
    }
    chain
        .iter()
        .map(AsRef::as_ref)
        .find(|m| provider_available(m))
        .map(str::to_string)
        .unwrap_or_else(|| {
            chain
                .last()
                .map(|s| s.as_ref().to_string())
                .unwrap_or_default()
        })
}

// ── API DTOs (GET /models) ───────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelEntry {
    pub id: String,
    pub label: String,
    pub tier: ModelTier,
    /// True when the provider's API key is configured on this server.
    pub available: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentRecommendation {
    pub agent_type: AgentType,
    /// Full provider-priority chain (catalog ids). `chain[0]` is the primary
    /// recommendation; every entry is highlighted in the picker.
    pub chain: Vec<String>,
    /// The model that resolution would pick right now given provider
    /// availability — the value the composer pre-selects for "recommended".
    pub resolved_model: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelCatalogResponse {
    pub models: Vec<ModelEntry>,
    pub agents: Vec<AgentRecommendation>,
}

/// Build the catalog response, annotating availability from the live provider.
pub fn catalog_response() -> ModelCatalogResponse {
    let models = CATALOG
        .iter()
        .map(|m| ModelEntry {
            id: m.id.to_string(),
            label: m.label.to_string(),
            tier: m.tier,
            available: provider_available(m.id),
        })
        .collect();

    let agents = AgentType::ALL
        .iter()
        .map(|&agent| {
            let chain: Vec<String> = agent.chain().iter().map(|s| s.to_string()).collect();
            AgentRecommendation {
                agent_type: agent,
                resolved_model: resolve_model_in(&chain, None),
                chain,
            }
        })
        .collect();

    ModelCatalogResponse { models, agents }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_catalogued(id: &str) -> bool {
        CATALOG.iter().any(|m| m.id == id)
    }

    #[test]
    fn every_chain_entry_is_catalogued() {
        for agent in AgentType::ALL {
            for id in agent.chain() {
                assert!(
                    is_catalogued(id),
                    "{} chain references uncatalogued model {id}",
                    agent.as_str()
                );
            }
        }
    }

    #[test]
    fn chains_are_provider_diverse() {
        // Each recommendation chain should span distinct providers so a single
        // missing API key never empties the chain.
        for agent in AgentType::ALL {
            let mut providers: Vec<&str> = agent
                .chain()
                .iter()
                .map(|&id| id.split('/').next().unwrap_or(id))
                .collect();
            providers.sort_unstable();
            let len = providers.len();
            providers.dedup();
            assert_eq!(
                len,
                providers.len(),
                "{} chain repeats a provider",
                agent.as_str()
            );
        }
    }

    #[test]
    fn agent_type_parses_canonical_values() {
        assert_eq!(AgentType::from_str("coworker"), Some(AgentType::Coworker));
        assert_eq!(
            AgentType::from_str("deep-research"),
            Some(AgentType::DeepResearch)
        );
        assert_eq!(AgentType::from_str("speedwagon"), None);
        assert_eq!(AgentType::from_str("nope"), None);
    }

    #[test]
    fn resolve_always_returns_a_catalogued_model() {
        // Env-independent: resolution always yields a real catalog entry
        // (chain hit, pin, or last-resort) — never an empty/garbage id.
        for agent in AgentType::ALL {
            let r = resolve_model(Some(agent.as_str()), None);
            assert!(is_catalogued(&r), "resolved {r} not in catalog");
        }
        let pinned = resolve_model(Some("coworker"), Some("anthropic/claude-opus-4-8"));
        assert!(is_catalogued(&pinned));
        // A pin whose provider is not configured is ignored, falling through to
        // the chain. An unregistered provider holds regardless of which API keys
        // are present.
        let bogus = resolve_model(Some("deep-research"), Some("nonexistent/model"));
        assert!(is_catalogued(&bogus));
    }

    #[test]
    fn catalog_response_annotates_and_recommends() {
        let resp = catalog_response();
        // Every catalog entry is surfaced.
        assert_eq!(resp.models.len(), CATALOG.len());
        // One recommendation per agent surface, each resolving to a catalogued id.
        assert_eq!(resp.agents.len(), AgentType::ALL.len());
        for rec in &resp.agents {
            assert!(!rec.chain.is_empty());
            assert!(
                is_catalogued(&rec.resolved_model),
                "{} resolved to uncatalogued {}",
                rec.agent_type.as_str(),
                rec.resolved_model
            );
        }
    }
}
