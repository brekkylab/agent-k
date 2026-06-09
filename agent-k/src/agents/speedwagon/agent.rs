use ailoy::agent::{Agent, AgentCard, AgentProvider, AgentSpec, default_provider};

use super::tool::{
    build_tools, get_calculate_tool_desc, get_find_in_document_tool_desc,
    get_read_document_tool_desc, get_search_document_tool_desc,
};
use crate::knowledge_base::SharedStore;

pub const SYSTEM_PROMPT: &str = r#"You are a research assistant that answers questions from a document corpus using the provided tools.

# Tools

- **search_document(query)** — find candidate documents ranked by relevance. Start here.
- **find_in_document(id, query)** — locate the lines where a term appears in one document. Returns line numbers.
- **read_document(id, start, end)** — read a line range. Keep ranges tight (20-40 lines around a match).
- **calculate(expression)** — evaluate one arithmetic expression, e.g. `"1577 * 1.08"`, `"sqrt(2) * pi"`.

# How to work

Chain the tools: `search_document` to find the document, `find_in_document` to locate the term, `read_document` to read just that range. One find followed by one read is usually enough to confirm a fact.

- Do not re-read a range you have already read, and do not re-run a query that already answered the question. Each call should add information you do not yet have.
- If a search returns nothing useful, try different terms or synonyms before concluding — but two or three distinct queries is the limit, not a dozen.
- For a number derived from corpus data (a growth rate, a ratio), read the raw figures first, then call `calculate`.

# When to stop

Stop searching as soon as you have the facts the question asks for, and answer. Reading on after the answer is in hand wastes effort and risks contradicting yourself. If you have tried the reasonable queries and the corpus does not hold the answer, say so and state what you searched — do not keep retrying the same approach.

# Answer

Lead with the direct answer in one or two sentences, then cite the source: the document title or filepath and the line numbers you read it from. Keep it concise."#;

/// Builder for a Speedwagon agent spec — corpus question-answering over a
/// document [`SharedStore`](crate::knowledge_base::SharedStore).
///
/// The default tool set is the four corpus tools (`search_document`,
/// `find_in_document`, `read_document`, `calculate`) plus a `web_search`
/// fallback. [`with_shell`](SpeedwagonSpec::with_shell) adds a `shell` tool
/// (off by default — shell access is not part of the corpus-QA loop and is
/// opt-in for callers that need it).
#[derive(Debug, Clone)]
pub struct SpeedwagonSpec {
    model: String,
    card: Option<AgentCard>,
    web_search: bool,
    shell: bool,
}

impl SpeedwagonSpec {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn card(mut self, card: AgentCard) -> Self {
        self.card = Some(card);
        self
    }

    /// Add the `web_search` tool as a fallback for questions the corpus does
    /// not cover. Enabled by default.
    pub fn with_web_search(mut self, enabled: bool) -> Self {
        self.web_search = enabled;
        self
    }

    /// Add the `shell` tool. Disabled by default; corpus QA does not need
    /// shell access, and enabling it tends to add latency without improving
    /// answer accuracy.
    pub fn with_shell(mut self, enabled: bool) -> Self {
        self.shell = enabled;
        self
    }

    /// Build the [`AgentSpec`] with the configured tool descriptions. Tool
    /// functions are resolved at agent-construction time against the
    /// [`AgentProvider`] (see [`get_speedwagon_agent`]).
    pub fn into_spec(self) -> AgentSpec {
        let mut spec = AgentSpec::new(self.model)
            .instruction(SYSTEM_PROMPT)
            .tools([
                get_search_document_tool_desc(),
                get_find_in_document_tool_desc(),
                get_read_document_tool_desc(),
                get_calculate_tool_desc(),
            ]);
        spec.card = self.card;
        if self.web_search {
            spec = spec.web_search_tool(vec![]);
        }
        if self.shell {
            spec = spec.shell_tool();
        }
        spec
    }
}

impl Default for SpeedwagonSpec {
    fn default() -> Self {
        Self {
            model: "openai/gpt-5.4-mini".into(),
            card: None,
            web_search: true,
            shell: false,
        }
    }
}

impl From<SpeedwagonSpec> for AgentSpec {
    fn from(value: SpeedwagonSpec) -> Self {
        value.into_spec()
    }
}

/// Build a ready-to-run Speedwagon agent bound to `store`.
///
/// The corpus tools (`search_document` / `find_in_document` / `read_document`)
/// read from `store`; `calculate`, `web_search`, and the optional `shell` come
/// from the built-in tool registry. Language models are taken from the global
/// [`default_provider`] (populated from the environment at startup), so only
/// the tool registry is swapped for the store-bound one.
///
/// `with_shell` enables the `shell` tool (default off; see
/// [`SpeedwagonSpec::with_shell`]). Tools run on a local [`RunEnv`].
///
/// [`RunEnv`]: ailoy::runenv::RunEnv
pub async fn get_speedwagon_agent(
    model: impl AsRef<str>,
    store: SharedStore,
    with_shell: bool,
) -> anyhow::Result<Agent> {
    let spec = SpeedwagonSpec::new()
        .model(model.as_ref())
        .with_shell(with_shell)
        .into_spec();

    // Models come from the global provider (env-populated); the tool registry
    // is the store-bound one. `web_search` / `shell` resolve against the
    // built-in factories that `build_tools` (via `ToolProvider::new`)
    // pre-registers, so no extra wiring is needed here.
    let provider = AgentProvider {
        models: default_provider().models.clone(),
        tools: build_tools(store),
    };

    Agent::try_with_provider(spec, &provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names(spec: AgentSpec) -> Vec<String> {
        spec.tools.into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn default_spec_has_corpus_tools_and_web_search_no_shell() {
        let names = tool_names(SpeedwagonSpec::new().into_spec());
        for t in [
            "search_document",
            "find_in_document",
            "read_document",
            "calculate",
            "web_search",
        ] {
            assert!(names.iter().any(|n| n == t), "missing tool: {t} in {names:?}");
        }
        assert!(
            !names.iter().any(|n| n == "shell"),
            "shell should be off by default"
        );
    }

    #[test]
    fn with_shell_adds_shell_tool() {
        let names = tool_names(SpeedwagonSpec::new().with_shell(true).into_spec());
        // canonical built-in name is "shell" — the provider resolves it against
        // the pre-registered builtin factory of the same name.
        assert!(names.iter().any(|n| n == "shell"), "shell missing in {names:?}");
    }

    #[test]
    fn with_web_search_off_drops_web_search() {
        let names = tool_names(SpeedwagonSpec::new().with_web_search(false).into_spec());
        assert!(!names.iter().any(|n| n == "web_search"));
        // corpus tools remain
        assert!(names.iter().any(|n| n == "search_document"));
    }

    #[test]
    fn model_override_is_applied() {
        let spec = SpeedwagonSpec::new()
            .model("anthropic/claude-haiku-4-5-20251001")
            .into_spec();
        assert_eq!(spec.model, "anthropic/claude-haiku-4-5-20251001");
    }
}
