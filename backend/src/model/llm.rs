//! Model catalog, per-agent recommendation chains, and model resolution.
//!
//! Model ids follow the ailoy `provider/model-id` convention (e.g.
//! `"anthropic/claude-sonnet-4-6"`). A provider is *available* when its API key
//! env var is set, which ailoy reflects by registering a `provider/*` glob on
//! the process-wide [`default_provider`]. We treat "the glob resolves" as the
//! availability signal — it is synchronous and makes no network call.
//!
//! Tiers (`light` / `standard` / `max`) only group models in the picker UI;
//! a tier is never selected directly. Each agent type has an ordered, provider
//! -diverse recommendation chain; the first entry's tier is its "recommended
//! tier", and every entry is highlighted as a recommended model.

use std::collections::BTreeMap;

use ailoy::agent::default_provider;
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
/// recommendation chain and which agent is dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Coworker,
    Speedwagon,
    DeepResearch,
    Buddy,
}

impl AgentType {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentType::Coworker => "coworker",
            AgentType::Speedwagon => "speedwagon",
            AgentType::DeepResearch => "deep-research",
            AgentType::Buddy => "buddy",
        }
    }

    /// Parse a stored/request value; unknown values → `None`.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "coworker" => Some(AgentType::Coworker),
            "speedwagon" => Some(AgentType::Speedwagon),
            "deep-research" => Some(AgentType::DeepResearch),
            "buddy" => Some(AgentType::Buddy),
            _ => None,
        }
    }

    pub const ALL: [AgentType; 4] = [
        AgentType::Coworker,
        AgentType::Speedwagon,
        AgentType::DeepResearch,
        AgentType::Buddy,
    ];

    /// Selectable but not recommended for this agent — a soft picker hint, not a
    /// block. Speedwagon discourages gemini-3.5-flash (slow in the corpus loop).
    pub fn discourages_model(self, model_id: &str) -> bool {
        matches!((self, model_id), (AgentType::Speedwagon, "google/gemini-3.5-flash"))
    }

    /// Ordered, provider-diverse recommendation chain. The last entry is the
    /// terminal default (returned even if its provider is unavailable).
    pub fn chain(self) -> &'static [&'static str] {
        match self {
            // Coworker: a balanced "standard" general-assistant chain.
            AgentType::Coworker => &[
                "openai/gpt-5.4-mini",
                "anthropic/claude-sonnet-4-6",
                "google/gemini-3-flash",
                "moonshotai/kimi-k2.6",
            ],
            // Speedwagon (corpus QA): its own chain. Gemini slot is 3.1 Flash-Lite
            // — fast with good accuracy in the corpus loop.
            AgentType::Speedwagon => &[
                "openai/gpt-5.4-mini",
                "anthropic/claude-sonnet-4-6",
                "google/gemini-3.1-flash-lite",
                "moonshotai/kimi-k2.6",
            ],
            AgentType::DeepResearch => &[
                "openai/gpt-5.5",
                "google/gemini-3.5-flash",
                "anthropic/claude-opus-4-8",
                "moonshotai/kimi-k2.6",
            ],
            AgentType::Buddy => &[
                "openai/gpt-5.4-nano",
                "anthropic/claude-haiku-4-5",
                "google/gemini-3.1-flash-lite",
                "moonshotai/kimi-k2.6",
            ],
        }
    }
}

/// A catalogued model. `id` is the full ailoy `provider/model-id`
/// (e.g. `"anthropic/claude-sonnet-4-6"`).
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
        id: "openai/gpt-5.4-nano",
        label: "GPT-5.4 nano",
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
    ModelInfo {
        id: "google/gemini-2.5-flash",
        label: "Gemini 2.5 Flash",
        tier: ModelTier::Light,
    },
    // ── standard ─────────────────────────────────────────────────────────────
    ModelInfo {
        id: "openai/gpt-5.4-mini",
        label: "GPT-5.4 mini",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "anthropic/claude-sonnet-4-6",
        label: "Claude Sonnet 4.6",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "google/gemini-3-flash",
        label: "Gemini 3 Flash",
        tier: ModelTier::Standard,
    },
    ModelInfo {
        id: "moonshotai/kimi-k2.6",
        label: "Kimi K2.6",
        tier: ModelTier::Standard,
    },
    // ── max ────────────────────────────────────────────────────────────────
    ModelInfo {
        id: "openai/gpt-5.5",
        label: "GPT-5.5",
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
];

pub fn catalog_entry(id: &str) -> Option<&'static ModelInfo> {
    CATALOG.iter().find(|m| m.id == id)
}

/// Whether the model's provider is registered (API key env var set).
/// Synchronous, no network — the same lookup `Agent` construction performs.
/// Works for any id, catalogued or not, so callers can judge an arbitrary pin.
pub fn provider_available(model_id: &str) -> bool {
    default_provider().models.get(model_id).is_some()
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

/// Per-project recommendation-chain overrides, parsed from the
/// `projects.recommended_chains` JSON column.
#[derive(Debug, Default)]
pub struct ProjectChains {
    by_agent: BTreeMap<AgentType, Vec<String>>,
}

impl ProjectChains {
    /// Tolerant parse: unknown keys, empty lists, and malformed JSON are dropped.
    /// `None`/parse-failure → empty (all defaults).
    pub fn parse(json: Option<&str>) -> Self {
        let mut by_agent = BTreeMap::new();
        if let Some(raw) = json
            && let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(raw)
        {
            for (k, v) in map {
                if let Some(agent) = AgentType::from_str(&k)
                    && !v.is_empty()
                {
                    by_agent.insert(agent, v);
                }
            }
        }
        Self { by_agent }
    }

    /// Effective chain for an agent: the project override (honored as-is), else
    /// the built-in default. No per-agent model is blocked.
    pub fn chain_for(&self, agent: AgentType) -> Vec<String> {
        match self.by_agent.get(&agent) {
            Some(ids) => ids.clone(),
            None => agent.chain().iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Validate project chain overrides for storage: known agent_type keys and
/// non-empty lists of catalogued model ids. Provider availability and per-agent
/// recommendation are not enforced — any catalogued model may be chosen.
pub fn validate_chains(chains: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
    for (key, ids) in chains {
        if AgentType::from_str(key).is_none() {
            return Err(format!("unknown agent_type: {key}"));
        }
        if ids.is_empty() {
            return Err(format!("chain for '{key}' must not be empty"));
        }
        for id in ids {
            if catalog_entry(id).is_none() {
                return Err(format!("unknown model in '{key}' chain: {id}"));
            }
        }
    }
    Ok(())
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
    /// Catalog ids this agent discourages (see [`AgentType::discourages_model`]).
    /// The picker tags these "(Not Recommended)" but still lets them be chosen.
    pub not_recommended: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelCatalogResponse {
    pub models: Vec<ModelEntry>,
    pub agents: Vec<AgentRecommendation>,
}

/// Build the catalog response, annotating availability from the live provider.
/// `chains` carries any per-project overrides.
pub fn catalog_response(chains: &ProjectChains) -> ModelCatalogResponse {
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
            let chain = chains.chain_for(agent);
            AgentRecommendation {
                agent_type: agent,
                resolved_model: resolve_model_in(&chain, None),
                chain,
                not_recommended: CATALOG
                    .iter()
                    .filter(|m| agent.discourages_model(m.id))
                    .map(|m| m.id.to_string())
                    .collect(),
            }
        })
        .collect();

    ModelCatalogResponse { models, agents }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_chain_entry_is_catalogued() {
        for agent in AgentType::ALL {
            for id in agent.chain() {
                assert!(
                    catalog_entry(id).is_some(),
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
        assert_eq!(AgentType::from_str("speedwagon"), Some(AgentType::Speedwagon));
        assert_eq!(AgentType::from_str("coworker"), Some(AgentType::Coworker));
        // 'rag' is no longer an accepted alias.
        assert_eq!(AgentType::from_str("rag"), None);
        assert_eq!(AgentType::from_str("nope"), None);
    }

    #[test]
    fn speedwagon_chain_uses_gemini_3_1_flash_lite() {
        // Speedwagon's chain is independent of Coworker's: its Gemini slot is
        // 3.1 Flash-Lite, Coworker keeps 3 Flash. Guards the split.
        let sw = AgentType::Speedwagon.chain();
        let cw = AgentType::Coworker.chain();
        assert!(
            sw.contains(&"google/gemini-3.1-flash-lite"),
            "speedwagon chain should use gemini-3.1-flash-lite: {sw:?}"
        );
        assert!(
            !sw.contains(&"google/gemini-3-flash"),
            "speedwagon chain should not use gemini-3-flash: {sw:?}"
        );
        assert!(
            cw.contains(&"google/gemini-3-flash"),
            "coworker chain should keep gemini-3-flash: {cw:?}"
        );
        assert!(
            !cw.contains(&"google/gemini-3.1-flash-lite"),
            "coworker chain should not use gemini-3.1-flash-lite: {cw:?}"
        );
    }

    #[test]
    fn speedwagon_discourages_gemini_3_5_flash() {
        let sw = AgentType::Speedwagon;
        // Discouraged (hint only, still selectable).
        assert!(sw.discourages_model("google/gemini-3.5-flash"));
        // Recommended / fine — not discouraged.
        assert!(!sw.discourages_model("google/gemini-2.5-flash"));
        assert!(!sw.discourages_model("google/gemini-3.1-flash-lite"));
        assert!(!sw.discourages_model("openai/gpt-5.4-mini"));
        // Other agents discourage nothing.
        for m in CATALOG {
            assert!(!AgentType::Coworker.discourages_model(m.id), "coworker discouraged {}", m.id);
        }
    }

    #[test]
    fn validate_chains_accepts_any_catalogued_model() {
        // No per-agent blocking: a model that is merely discouraged for
        // Speedwagon is still accepted in an override.
        let mut ok = BTreeMap::new();
        ok.insert("speedwagon".to_string(), vec!["google/gemini-3.5-flash".to_string()]);
        assert!(validate_chains(&ok).is_ok());

        // Unknown agent_type and uncatalogued model are still rejected.
        let mut bad_agent = BTreeMap::new();
        bad_agent.insert("nope".to_string(), vec!["openai/gpt-5.4-mini".to_string()]);
        assert!(validate_chains(&bad_agent).is_err());
        let mut bad_model = BTreeMap::new();
        bad_model.insert("speedwagon".to_string(), vec!["openai/does-not-exist".to_string()]);
        assert!(validate_chains(&bad_model).is_err());
    }

    #[test]
    fn chain_for_returns_override_verbatim() {
        // A stored project override (projects.recommended_chains JSON) is returned
        // as-is — a now-discouraged model is no longer filtered out.
        let json = r#"{"speedwagon":["google/gemini-3.5-flash","openai/gpt-5.4-mini"]}"#;
        let chains = ProjectChains::parse(Some(json));
        assert_eq!(
            chains.chain_for(AgentType::Speedwagon),
            ["google/gemini-3.5-flash", "openai/gpt-5.4-mini"]
        );
        // No override → built-in chain.
        assert_eq!(
            ProjectChains::default().chain_for(AgentType::Speedwagon),
            AgentType::Speedwagon.chain()
        );
    }

    #[test]
    fn resolve_always_returns_a_catalogued_model() {
        // Env-independent: resolution always yields a real catalog entry
        // (chain hit, pin, or last-resort) — never an empty/garbage id.
        for agent in AgentType::ALL {
            let r = resolve_model(Some(agent.as_str()), None);
            assert!(catalog_entry(&r).is_some(), "resolved {r} not in catalog");
        }
        let pinned = resolve_model(Some("coworker"), Some("anthropic/claude-opus-4-8"));
        assert!(catalog_entry(&pinned).is_some());
        // An unknown/empty pin is ignored, falling through to the chain.
        let bogus = resolve_model(Some("buddy"), Some("openai/does-not-exist"));
        assert!(catalog_entry(&bogus).is_some());
    }
}
