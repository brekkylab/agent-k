use ailoy::{
    agent::AgentBuilder,
    lang_model::ResponseFormat,
    message::{Message, Part, Role},
    to_value,
};
use serde::Deserialize;
use tokio_stream::StreamExt;

const ROUTER_INSTRUCTION: &str = r#"Your objective is, based on the conversation history so far, to choose the most appropriate agent to answer the user's latest query.

## Candidate Agents
- `reception`: A simple agent that has no additional features, just an LLM.
- `speedwagon`: A RAG-enabled agent specialized in answering users' questions. It can use external sources or internal knowledge if needed.
- `vegapunk`: A deep-research agent that can create structured reports, literature or research synthesis for given topics as markdown-formatted artifacts.
- `minerva`: An execution-oriented agent that can plan and perform tasks autonomously, including exploring/editing files, running code, and producing downloadable artifacts.

## Aggregation
- Some agents hold other agents as sub-agents.
  - `minerva` holds `speedwagon` and `vegapunk`.
  - `vegapunk` holds `speedwagon`.
- Calling a parent agent implies calling its sub-agents as well, meaning it can do everything a sub-agent can do. We refer to this as being "more capable."

## Selecting Rules
- Even when the information cannot be found via internet search, the user may have provided their own documents, so consider `speedwagon` first.
- `reception` can be selected only for trivial tasks: greetings, nonsensical input, or refusals to act.
- The expected output format is the first consideration.
  - Brief answer is sufficient → `speedwagon`
  - Markdown-formatted report is the better presentation → `vegapunk`
  - If other file types need to be produced as artifacts → `minerva`
- Analyze the work required to fulfill the user's request. Even when the intent is clear, if the task is beyond what a less-capable agent can handle, do not choose that agent.
  - If external information retrieval alone is enough to answer → `speedwagon`
  - If the answer requires cross-checking and reasoning across multiple sources → `vegapunk`
  - If analyzing or processing the information requires external tools or scripts → `minerva`
- Prefer to call less-capable agent whenever confidence is sufficient.
- Consider the actions required to fulfill the request, and choose `minerva` only when truly necessary:
  - If script generation and execution is needeed to satisfy user's query
  - If the request requires iterative loop (action / observe / reflection) → minerva
- If the user's query is obviously conjunction of two or more requests, consider each request and pick the agent that can handle them all.

## Response Format
```
{"choice": "<selected agent name>", "reason": "..."}
```
- The `reason` must be written in the language the user used in the query.
"#;

// - The user's intent is the primary consideration when selecting an agent. If the user's intent is clear, route in that direction.
//   - Makes an informational request → `speedwagon`
//   - Asks for an analysis report → `vegapunk`
//   - Wants a task to be performed → `minerva`

#[derive(Debug, Deserialize)]
pub struct Route {
    pub choice: String,

    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn run_gpt_router_agent(user_input: impl Into<String>) -> anyhow::Result<Route> {
    run_router_agent("openai/gpt-5.4-mini", user_input).await
}

pub async fn run_claude_router_agent(user_input: impl Into<String>) -> anyhow::Result<Route> {
    run_router_agent("anthropic/claude-haiku-4-5", user_input).await
}

async fn run_router_agent(model: &str, user_input: impl Into<String>) -> anyhow::Result<Route> {
    let user_input: String = user_input.into();
    let schema = to_value!({
        "type": "object",
        "properties": {
            "choice": {
                "type": "string",
                "enum": ["reception", "speedwagon", "vegapunk", "minerva"],
                "description": "Agent assigned to this step"
            },
            "reason": { "type": "string", "description": "Short reason for assigning this step to the agent" }
        },
        "required": ["choice", "reason"],
        "additionalProperties": false
    });
    let mut agent = AgentBuilder::new(model)
        .instruction(ROUTER_INSTRUCTION)
        .response_format(ResponseFormat::json_schema(schema)?)
        .build()?;

    let msg = Message::new(Role::User).with_contents([Part::text(user_input)]);
    let mut stream = agent.run(msg);
    while let Some(event) = stream.next().await {
        let _ = event?;
    }
    drop(stream);

    let last = agent
        .get_history()
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .ok_or_else(|| anyhow::anyhow!("router produced no assistant message"))?;

    let raw = last
        .contents
        .iter()
        .filter_map(|p| p.as_text())
        .collect::<Vec<_>>()
        .join("");

    let route: Route = serde_json::from_str(&raw)?;
    Ok(route)
}
