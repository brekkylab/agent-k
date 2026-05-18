use ailoy::{
    agent::AgentBuilder,
    lang_model::ResponseFormat,
    message::{Message, Part, Role},
    to_value,
};
use serde::Deserialize;
use tokio_stream::StreamExt;

const ROUTER_INSTRUCTION: &str = r#"Your objective is to choose the most appropriate agent(s) to answer the user's query.

## Candidate agents
- `trivial`: A simple agent that has no additional features, just LLM.
- `rag`: Specialized in answering users' questions. It can use external sources or internal knowledge if needed.
- `deep_research`: Can create structured reports for given topics.
- `cowork`: Plans and executes tasks, including exploring/editing files, running code, and producing downloadable artifacts.

## Rules
Route to `trivial` only for trivial tasks: greetings, pure noise, or refusals to act.
Route to `rag` for direct questions that can be answered in a single turn.
If you think searching for materials or references is necessary, always use `rag` instead of `trivial`.
Do not hesitate to route to `deep_research` when the user expects:
 - a structured report,
 - extensive comparison,
 - literature or research synthesis,
 - or investigation across many sources or topics.
`cowork` is the only agent that can control the local file system. Therefore, if access to local files is required, route to `cowork`.
We believe `cowork` has the most powerful capabilities, including other agent's capabilities. Therefore, tasks that may be difficult for other agents to solve should be routed to `cowork`.

## Pipelining
If the user's query involves more than one objective, decompose it into sub-queries and pipeline one agent's result to other agents.
When pipelining is applied, the downstream agent treats the generated sub-query as the user's input. The preceding agent's input and output are appended to that input as prior context.
You have to pipeline them only if a pipelining path is available. If none exists, you may find the closest agent.
Do not pipeline a query that has a single intent, even when it is list-like, lengthy, or asks about multiple items. Assign it to one agent as-is.
Each sub-query is the corresponding part of the user's request, kept close to the original wording.

Available pipelines are:
- `rag` → `cowork`
- `deep_research` → `cowork`

Prefer a single agent whenever possible.
Pipelines should be rare and only used when the request clearly contains multiple separable objectives that map naturally to different agents.

## Response format
The response should be a single JSON, following below schema.
If a pipeline is not applied, return an object containing agent and reason.

```
{"agent": "<selected agent name>"}
```

If a pipeline is applied, return a list of steps.

```
{"steps": [{"agent": "<selected agent name>", "query": "..."}, ...]}
```
"#;

#[derive(Debug, Deserialize)]
pub struct Step {
    pub agent: String,
    #[serde(rename = "query")]
    pub input: String,

    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Plan {
    pub steps: Vec<Step>,
}

/// Matches the two shapes documented in `ROUTER_INSTRUCTION` → `## Response
/// format`: a single-agent object, or a `{ "steps": [...] }` pipeline.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RouterResponse {
    Pipeline {
        steps: Vec<Step>,
    },
    Single {
        agent: String,
        #[serde(default)]
        reason: Option<String>,
    },
}

impl RouterResponse {
    fn into_plan(self, original_input: &str) -> Plan {
        match self {
            RouterResponse::Pipeline { steps } => Plan { steps },
            RouterResponse::Single { agent, reason } => Plan {
                steps: vec![Step {
                    agent,
                    input: original_input.to_string(),
                    reason,
                }],
            },
        }
    }
}

const ROUTER_MAX_RETRIES: usize = 2;

pub async fn run_gpt_router_agent(user_input: impl Into<String>) -> anyhow::Result<Plan> {
    run_router_agent("openai/gpt-5.4-nano", user_input).await
}

pub async fn run_claude_router_agent(user_input: impl Into<String>) -> anyhow::Result<Plan> {
    run_router_agent("anthropic/claude-haiku-4-5", user_input).await
}

async fn run_router_agent(model: &str, user_input: impl Into<String>) -> anyhow::Result<Plan> {
    let user_input: String = user_input.into();
    let schema = to_value!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Selected agent name" },
                    "reason": { "type": "string", "description": "Short reason why this agent was selected" }
                },
                "required": ["agent", "reason"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "agent": { "type": "string", "description": "Agent assigned to this step" },
                                "query": { "type": "string", "description": "Sub-request for the agent" },
                                "reason": { "type": "string", "description": "Short reason for assigning this step to the agent" }
                            },
                            "required": ["agent", "query", "reason"],
                            "additionalProperties": false
                        },
                        "minItems": 1
                    }
                },
                "required": ["steps"],
                "additionalProperties": false
            }
        ]
    });
    let mut agent = AgentBuilder::new(model)
        .instruction(ROUTER_INSTRUCTION)
        .response_format(ResponseFormat::json_schema(schema)?)
        .build()?;

    let mut next_message =
        Some(Message::new(Role::User).with_contents([Part::text(user_input.clone())]));
    let mut last_err = String::from("no attempts made");

    for _ in 0..ROUTER_MAX_RETRIES {
        let msg = next_message
            .take()
            .expect("next_message set before each iteration");
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

        let mut it = serde_json::Deserializer::from_str(&raw).into_iter::<RouterResponse>();
        last_err = match it.next() {
            Some(Ok(resp)) => {
                let plan = resp.into_plan(&user_input);
                if plan.steps.is_empty() {
                    "empty steps array".to_string()
                } else {
                    let invalid = plan.steps.iter().enumerate().find_map(|(i, s)| {
                        if !matches!(s.agent.as_str(), "trivial" | "rag" | "deep_research" | "cowork") {
                            Some(format!(
                                "invalid agent '{}' at step {}; must be one of trivial/rag/deep_research/cowork",
                                s.agent, i
                            ))
                        } else {
                            None
                        }
                    });
                    if let Some(msg) = invalid {
                        msg
                    } else {
                        return Ok(plan);
                    }
                }
            }
            Some(Err(e)) => format!("JSON parse failed: {e}"),
            None => "response had no JSON object".to_string(),
        };

        next_message = Some(Message::new(Role::User).with_contents([Part::text(format!(
            "Your previous response is not valid JSON. Respond again.\n\n{last_err}."
        ))]));
    }
    Ok(Plan {
        steps: vec![Step {
            agent: "cowork".to_string(),
            input: user_input,
            reason: Some(format!("fallback: {last_err}")),
        }],
    })
}
