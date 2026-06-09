use ailoy::agent::{Agent, AgentBuilder, AgentCard, AgentProvider, AgentSpec, default_provider};

use super::tool::{
    build_tools, get_calculate_tool_desc, get_find_in_document_tool_desc,
    get_read_document_tool_desc, get_search_document_tool_desc,
};
use crate::knowledge_base::SharedStore;

pub const SYSTEM_PROMPT: &str = r#"You are {{NAME}}. Your primary role is to answer the user's questions from a project's document corpus, searching the web only when the corpus is not enough.

## Corpus tools
- **search_document(query)** — find candidate documents ranked by relevance. Start here for any question about the project's documents.
- **find_in_document(id, query)** — locate the lines where a term appears in one document. Returns line numbers.
- **read_document(id, start, end)** — read a line range. Keep ranges tight (20-40 lines around a match).
- **calculate(expression)** — evaluate one arithmetic expression, e.g. `"1577 * 1.08"`, `"sqrt(2) * pi"`. For a figure derived from corpus data, read the raw numbers first, then calculate.

## How to work
- Chain the corpus tools: `search_document` to find the document, `find_in_document` to locate the term, `read_document` to read just that range. One find followed by one read is usually enough to confirm a fact.
- Do not re-read a range you have already read, and do not re-run a query that already answered the question. Each call should add information you do not yet have.
- If a search returns nothing useful, try different terms or synonyms — but two or three distinct queries is the limit, not a dozen.

## Web search
- **web_search** is available as a fallback. Use it when the corpus does not contain the answer, or when the question needs current or external information the documents cannot have (recent events, live figures, definitions of outside terms).
- Some questions need both: find the fact in the corpus, then use the web to supplement it (e.g. a company named in a document plus its latest public information). Cover both halves.
- Treat the corpus as primary. Do not web-search a question the documents already answer.

## Shell
- **shell** is available as a secondary tool for things the corpus/web tools cannot do — inspecting an uploaded file's raw form, a quick local computation, light data wrangling.
- It is not a substitute for corpus search. For questions about the documents, use the corpus tools; reach for shell only when they cannot accomplish the step.

## When to stop
Stop as soon as you have the facts the question asks for, and answer. Continuing to search after the answer is in hand wastes effort and risks contradicting yourself. If you have tried the reasonable approaches and neither the corpus nor the web holds the answer, say so and state what you tried — do not keep retrying the same thing.

## Answer
- Lead with the direct answer in one or two sentences, then cite the source: the document title (and the line numbers you read) for corpus facts, or the URL for web facts. When you combine both, attribute each part.
- Keep it concise.

## Others
- Current time: {{TIME}}
- Always respond in the language the user used."#;

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
    name: String,
    model: String,
    card: Option<AgentCard>,
    web_search: bool,
    shell: bool,
}

impl SpeedwagonSpec {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
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
        let instruction = SYSTEM_PROMPT
            .replace("{{NAME}}", &self.name)
            .replace("{{TIME}}", &chrono::Utc::now().to_rfc3339());
        let mut spec = AgentSpec::new(self.model)
            .instruction(instruction)
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
            name: "agent-k".into(),
            model: "openai/gpt-5.4-mini".into(),
            card: None,
            web_search: true,
            shell: true,
        }
    }
}

/// Card name → the `subagent_speedwagon` tool a parent agent calls to delegate.
pub const SPEEDWAGON_SUBAGENT_NAME: &str = "speedwagon";

/// [`AgentCard`] a parent agent reads when deciding to delegate to Speedwagon.
pub fn subagent_card() -> AgentCard {
    AgentCard {
        name: SPEEDWAGON_SUBAGENT_NAME.into(),
        description: "Answers questions from this project's document corpus \
            (the files in the project's knowledge folder). Delegate the full \
            question for anything grounded in the uploaded files; it returns \
            the answer with its source, or says when the corpus lacks it."
            .into(),
        skills: vec![],
    }
}

/// Speedwagon [`AgentSpec`] for use as another agent's sub-agent: carries the
/// [`subagent_card`] and drops web_search/shell (the parent owns those). Corpus
/// tool functions resolve against the parent's provider, which must register
/// them via [`build_tools`](super::tool::build_tools).
pub fn speedwagon_subagent_spec(name: impl Into<String>, model: impl Into<String>) -> AgentSpec {
    SpeedwagonSpec::new()
        .name(name)
        .model(model)
        .card(subagent_card())
        .with_web_search(false)
        .with_shell(false)
        .into_spec()
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
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    store: SharedStore,
    with_shell: bool,
) -> anyhow::Result<Agent> {
    // Models come from the global provider (env-populated); the tool registry
    // is the store-bound one. `web_search` / `shell` resolve against the
    // built-in factories that `build_tools` (via `ToolProvider::new`)
    // pre-registers, so no extra wiring is needed here.
    let provider = AgentProvider {
        models: default_provider().models.clone(),
        tools: build_tools(store),
    };

    let instruction = SYSTEM_PROMPT
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &chrono::Utc::now().to_rfc3339());
    let mut builder = AgentBuilder::new(model.as_ref())
        .provider(provider)
        .instruction(instruction)
        .tools([
            get_search_document_tool_desc(),
            get_find_in_document_tool_desc(),
            get_read_document_tool_desc(),
            get_calculate_tool_desc(),
        ])
        .web_search_tool(vec![]);
    if with_shell {
        builder = builder.shell_tool();
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names(spec: AgentSpec) -> Vec<String> {
        spec.tools.into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn default_spec_has_corpus_tools_web_search_and_shell() {
        let names = tool_names(SpeedwagonSpec::new().into_spec());
        for t in [
            "search_document",
            "find_in_document",
            "read_document",
            "calculate",
            "web_search",
            "shell",
        ] {
            assert!(names.iter().any(|n| n == t), "missing tool: {t} in {names:?}");
        }
    }

    #[test]
    fn with_shell_off_drops_shell_tool() {
        let names = tool_names(SpeedwagonSpec::new().with_shell(false).into_spec());
        assert!(!names.iter().any(|n| n == "shell"), "shell should be droppable");
        // corpus tools remain
        assert!(names.iter().any(|n| n == "search_document"));
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

    #[test]
    fn instruction_renders_name_and_time() {
        let spec = SpeedwagonSpec::new().name("agent-k").into_spec();
        let inst = spec.instruction.expect("instruction set");
        assert!(inst.contains("You are agent-k."), "name not rendered: {inst}");
        assert!(!inst.contains("{{NAME}}"), "NAME placeholder left unrendered");
        assert!(!inst.contains("{{TIME}}"), "TIME placeholder left unrendered");
        assert!(inst.contains("Current time:"), "time line missing");
    }
}
