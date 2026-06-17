use ailoy::agent::{Agent, AgentBuilder, AgentCard, AgentProvider, AgentSpec, default_provider};

use super::tool::{
    build_tools, get_calculate_tool_desc, get_find_in_document_tool_desc,
    get_read_document_tool_desc, get_search_document_tool_desc,
};
use crate::knowledge_base::SharedStore;

pub const SYSTEM_PROMPT: &str = r#"You are {{NAME}}. Your primary role is to answer the user's questions from a project's document corpus, searching the web only when the corpus is not enough.

## Corpus tools
- **search_document(query)** — find candidate documents ranked by relevance. Start here for any question about the project's documents.
- **find_in_document(id, pattern)** — find where a term appears in one document. Each match reports the byte `start`/`end` of the hit and surrounding `context`.
- **read_document(id, offset, len)** — read `len` bytes starting at byte `offset`. Use a match's `start` as the `offset` to read around it; keep `len` tight (a few hundred to a couple thousand bytes).
- **calculate(expression)** — evaluate one arithmetic expression, e.g. `"1577 * 1.08"`, `"sqrt(2) * pi"`. For a figure derived from corpus data, read the raw numbers first, then calculate.

## How to work
- Chain the corpus tools: `search_document` to find the document, `find_in_document` to locate the term, `read_document` to read the bytes around a match. One find followed by one read is usually enough to confirm a fact.
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
- Lead with the direct answer in one or two sentences. Keep it concise.
- Cite every fact you assert with a numbered footnote marker `[^1]`, `[^2]`, … placed right after the sentence it supports. Reuse the same number when you cite the same source again.
- End the answer with a `## Sources` section listing each footnote's definition, one per line:
  - Corpus fact: `[^1]: <document title>` — use the title from `search_document`.
  - Web fact: `[^2]: <page title> — <url>` — use the title and URL from `web_search`.
- Markers and definitions must match exactly: every `[^N]` you write in the body must have one `[^N]:` line in `## Sources`, and every `[^N]:` line must be cited by a body marker. Never leave a marker undefined or a definition uncited.
- Cite only sources you actually used. When an answer combines corpus and web, give each its own footnote.
- A footnote is a citation of *support*. When you report that something is absent — the corpus or the web does not contain the answer, a document does not mention a topic — that is not a supported fact: say it plainly with no `[^N]` marker, and skip the `## Sources` section entirely. Do not cite a document as the source of what it fails to say.

## Others
- Current time: {{TIME}}
- Always respond in the language the user used."#;

/// Builder for a Speedwagon agent spec — corpus question-answering over a
/// document [`SharedStore`](crate::knowledge_base::SharedStore).
///
/// The default tool set is the four corpus tools (`search_document`,
/// `find_in_document`, `read_document`, `calculate`), a `web_search` fallback,
/// and a `shell` tool. Use [`with_shell(false)`](SpeedwagonSpec::with_shell) for
/// a corpus-only agent (the sub-agent spec does this).
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

    /// Toggle the `shell` tool, exposed as a secondary tool. Enabled by default;
    /// the corpus sub-agent turns it off to stay corpus-only.
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

/// Delegation note for a parent (like Coworker) that has no citation system of
/// its own: it should preserve the sub-agent's footnotes verbatim.
pub const SPEEDWAGON_DELEGATION_NOTE: &str = r#"

## Project documents
This project has a document corpus (the files in its knowledge folder), reachable through the `subagent_speedwagon` tool.
- For any question whose answer should come from the project's own documents — facts, figures, or quotes that live in the uploaded files — delegate to `subagent_speedwagon` with the full question, and answer from what it returns.
- Prefer the corpus over web search and over your own prior knowledge for anything project-specific. Only fall back to other sources when the sub-agent reports the corpus does not cover it.
- The sub-agent answers with `[^N]` footnote markers and a `## Sources` section. Carry those through verbatim: keep each `[^N]` marker where it sits in the sentence it supports, and reproduce the matching `## Sources` lines unchanged. Do not renumber, paraphrase, or drop a source."#;

/// Delegation note for Deep Research, which already runs its own citation system
/// (`citations.json` + `[^N]`). Corpus sources from the sub-agent are folded
/// into that single numbering space rather than kept as a separate block.
pub const SPEEDWAGON_DELEGATION_NOTE_DEEP_RESEARCH: &str = r#"

## Project documents
This project has a document corpus (the files in its knowledge folder), reachable through the `subagent_speedwagon` tool.
- When a section needs a fact from the project's own documents, delegate that question to `subagent_speedwagon` and use what it returns alongside your web research.
- The sub-agent cites corpus facts as `<document title>`. Fold each such source into your own `artifacts/citations.json` as a new entry with the next `[^N]` number, recording `{"title": ..., "source": "corpus"}` and no `url`. Cite it in `report.md` with that `[^N]` like any other source.
- In the verify phase, a corpus citation is exempt from the URL/fetch check: it is valid when the sub-agent returned that document. Web citations still require a URL you fetched this session."#;

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
            .model("anthropic/claude-haiku-4-5")
            .into_spec();
        assert_eq!(spec.model, "anthropic/claude-haiku-4-5");
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
