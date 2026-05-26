use ailoy::{
    agent::{Agent, AgentCard, AgentProvider, AgentSpec},
    runenv::RunEnv,
};

use super::tool::{
    build_tools, get_calculate_tool_desc, get_find_in_document_tool_desc,
    get_read_document_tool_desc, get_search_document_tool_desc,
};
use crate::knowledge_base::SharedStore;

pub const SYSTEM_PROMPT: &str = r#"You are {{NAME}}, an expert research assistant. Your task is to answer questions by systematically searching through a document corpus using the provided tools. Think step by step.

# Strategy

Follow this ReAct (Reason + Act) approach:

1. **Thought**: Analyze the question. Identify key entities and decide the best tool.
2. **Act**: Call the chosen tool.
3. **Observe**: Examine the result. Decide next step.

Repeat until you can confidently answer.

## Finding information

- Start with **search_document** to locate candidate documents.
- Use **find_in_document** to pinpoint specific keywords within a candidate document.
- Use **read_document** to read surrounding context around a match. Keep ranges small (20-40 lines); multiple small reads are better than one large read.
- If results are poor, try different query terms or synonyms before giving up. Try at least 2 different queries.

## Computation

- Use `calculate` for single arithmetic expressions (percentages, ratios, unit conversions). Examples: `"1577 * 1.08"`, `"sqrt(2) * pi"`.

## Web fallback

- If `web_search` is available and the corpus search yields no usable matches after at least two distinct queries, or if the question is clearly outside the corpus (current events, public facts not in any uploaded document), use `web_search` to gather candidate answers.
- Treat the corpus as the primary, trusted source. Use the web only when the corpus cannot answer.
- When the corpus disagrees with the web, prefer the corpus and note the discrepancy.

# Choosing the right approach

- **Document questions** (facts, quotes, data from the corpus): Use `search_document` first, then `find_in_document` and `read_document` to inspect. ALWAYS cite filepath and line numbers.
- **Computation questions** (single expressions): Use `calculate` directly.
- **Mixed questions** (e.g. "what is 3M's revenue growth rate?"): Find the raw data in documents first, then use `calculate` to compute.
- **Public-knowledge questions** the corpus does not cover: Try `search_document` once to confirm, then use `web_search`.

If unsure whether the answer is in the corpus, try a quick search first.

# Rules

- If `find_in_document` returns no matches, try synonym keywords or a broader term.
- For document-based answers: ALWAYS cite the specific document (filepath) and line numbers.
- For web-based answers: ALWAYS cite the source URL.
- **NEVER give up after a single tool call.** Try alternative tools and keywords before concluding. If the corpus is empty or off-topic and `web_search` is available, fall back to it before giving up.
- If you cannot find the answer after exhausting all approaches, say so and explain what you tried.
- Be concise in your final answer. Lead with the direct answer, then provide the source reference.
- Current time: {{TIME}}."#;

#[derive(Debug, Clone)]
pub struct SpeedwagonSpec {
    spec: AgentSpec,
}

impl SpeedwagonSpec {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.spec.model = model.into();
        self
    }

    pub fn card(mut self, card: AgentCard) -> Self {
        self.spec.card = Some(card);
        self
    }

    /// Advertise `web_search` as an available tool. Use when the agent should
    /// fall back to the public web for questions the corpus cannot answer.
    /// The runtime `ToolFunc` for `web_search` is provided by ailoy's default
    /// `ToolProvider`, so no extra wiring is required at registration time.
    pub fn with_web_search(mut self) -> Self {
        self.spec = self.spec.web_search_tool(vec![]);
        self
    }

    pub fn into_spec(self) -> AgentSpec {
        self.into()
    }
}

impl Default for SpeedwagonSpec {
    fn default() -> Self {
        Self {
            spec: AgentSpec::new("openai/gpt-5.4-mini")
                .instruction(SYSTEM_PROMPT)
                .tools([
                    get_search_document_tool_desc(),
                    get_find_in_document_tool_desc(),
                    get_read_document_tool_desc(),
                    get_calculate_tool_desc(),
                ]),
        }
    }
}

impl From<SpeedwagonSpec> for AgentSpec {
    fn from(value: SpeedwagonSpec) -> Self {
        value.spec
    }
}

/// UTC timestamp in ISO 8601 (`YYYY-MM-DDTHH:MM:SSZ`) using only stdlib.
/// Mirrors `coworker::now_utc_iso8601` so both prompts share a wall-clock token.
fn now_utc_iso8601() -> String {
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (y, m, d)
    }

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = sod / 3600;
    let mi = (sod % 3600) / 60;
    let s = sod % 60;
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Build a standalone Speedwagon `Agent` bound to `store`.
///
/// Mirrors `get_coworker_agent` in shape (`name`, `model`, plus the agent's
/// resource handle), but Speedwagon needs neither a sandbox nor mounted
/// directories — only a `SharedStore` for the corpus tools. The `RunEnv` is a
/// `Local` instance because the agent does not execute user code.
///
/// `web_search` is enabled by default; pass the returned `Agent` straight to
/// `agent.run(...)` or hand the underlying `SpeedwagonSpec` to a parent agent
/// via `.subagent(...)`.
pub async fn get_speedwagon_agent(
    name: impl AsRef<str>,
    model: impl AsRef<str>,
    store: SharedStore,
) -> anyhow::Result<Agent> {
    let inst = SYSTEM_PROMPT
        .replace("{{NAME}}", name.as_ref())
        .replace("{{TIME}}", &now_utc_iso8601());

    let spec = SpeedwagonSpec::new()
        .model(model.as_ref())
        .with_web_search()
        .into_spec()
        .instruction(inst);

    let mut provider = AgentProvider::new();
    provider.tools = build_tools(store);

    Agent::try_with_provider_and_runenv(spec, &provider, RunEnv::local())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names(spec: &AgentSpec) -> Vec<&str> {
        spec.tools.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn default_spec_has_four_corpus_tools_and_no_web_search() {
        let spec = SpeedwagonSpec::new().into_spec();
        let names = tool_names(&spec);
        assert!(names.contains(&"search_document"), "missing search_document: {names:?}");
        assert!(names.contains(&"find_in_document"), "missing find_in_document: {names:?}");
        assert!(names.contains(&"read_document"), "missing read_document: {names:?}");
        assert!(names.contains(&"calculate"), "missing calculate: {names:?}");
        assert!(!names.contains(&"web_search"), "web_search should be opt-in: {names:?}");
        assert_eq!(names.len(), 4, "expected exactly 4 default tools, got {names:?}");
    }

    #[test]
    fn with_web_search_adds_fifth_tool() {
        let spec = SpeedwagonSpec::new().with_web_search().into_spec();
        let names = tool_names(&spec);
        assert!(names.contains(&"web_search"), "web_search not advertised: {names:?}");
        assert_eq!(names.len(), 5, "expected 5 tools after with_web_search, got {names:?}");
    }

    #[test]
    fn system_prompt_describes_corpus_first_with_web_fallback() {
        // The prompt has to teach the model BOTH that corpus comes first AND
        // that web_search is the fallback. If either half is removed the
        // agent's behaviour drifts.
        assert!(
            SYSTEM_PROMPT.contains("search_document"),
            "prompt lost corpus-tool guidance"
        );
        assert!(
            SYSTEM_PROMPT.contains("web_search"),
            "prompt lost web_search fallback guidance"
        );
        assert!(
            SYSTEM_PROMPT.to_ascii_lowercase().contains("fallback"),
            "prompt lost explicit fallback wording"
        );
    }

    #[test]
    fn model_builder_overrides_default() {
        let spec = SpeedwagonSpec::new()
            .model("anthropic/claude-haiku-4-5")
            .into_spec();
        assert_eq!(spec.model, "anthropic/claude-haiku-4-5");
    }
}
