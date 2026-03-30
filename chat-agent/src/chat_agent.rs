use std::path::PathBuf;

use ailoy::{
    AgentProvider, AgentRuntime, AgentSpec, Message, Part, Role, ToolSet, Value,
};
use futures::StreamExt as _;

use crate::speedwagon::{self, KbEntry, SubAgentProvider};
use crate::tools::{self, DEFAULT_TOOL_ADD_INTEGERS, DEFAULT_TOOL_UTC_NOW};

/// A record of a tool interaction: the LLM's call and the tool's response.
#[derive(Debug, Clone)]
pub struct ToolCallEntry {
    /// Name of the tool (e.g. `"ask_knowledge"`)
    pub tool: String,
    /// Arguments passed by the LLM, preserving original structure.
    pub args: serde_json::Value,
    /// Result returned by the tool, or `None` if the tool hasn't responded yet.
    pub result: Option<serde_json::Value>,
}

/// Lightweight owner of an `AgentRuntime` for chat use cases.
pub struct ChatAgent {
    runtime: AgentRuntime,
    tool_log: Vec<ToolCallEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChatAgentRunError {
    #[error("failed to run language model")]
    Runtime {
        #[source]
        source: anyhow::Error,
    },
    #[error("model response did not include text content")]
    NoTextContent,
}

impl ChatAgent {
    pub fn new(
        mut spec: AgentSpec,
        provider: AgentProvider,
        kb_entries: Vec<KbEntry>,
        session_source_paths: Vec<(String, String, PathBuf)>,
    ) -> Self {
        // Extract API credentials and model name from the parent provider to pass to speedwagon sub-agents
        let sub_provider = SubAgentProvider::from_provider(&provider, &spec.lm);
        let (tool_names, tool_set) = build_tool_set(&kb_entries, sub_provider, session_source_paths);
        for name in tool_names {
            if !spec.tools.iter().any(|n| n == &name) {
                spec.tools.push(name);
            }
        }
        let runtime = AgentRuntime::new(spec, provider, tool_set);
        Self {
            runtime,
            tool_log: Vec::new(),
        }
    }

    /// Returns a log of tool calls collected during `run_user_text`.
    ///
    /// Each entry contains the tool name, its arguments as `serde_json::Value`,
    /// and the tool's result. This provides routing observability
    /// (e.g. which `kb_id` was chosen) without exposing ailoy internals.
    pub fn tool_call_log(&self) -> &[ToolCallEntry] {
        &self.tool_log
    }

    /// Sends a user message and returns the final assistant text.
    ///
    /// Uses `stream_turn()` to observe each step and collect tool call/result
    /// pairs into `tool_log` as they happen.
    pub async fn run_user_text(
        &mut self,
        content: impl Into<String>,
    ) -> Result<String, ChatAgentRunError> {
        // Clear previous turn's tool log so only the current query's calls are visible
        self.tool_log.clear();
        let query = Message::new(Role::User).with_contents([Part::text(content.into())]);
        let mut strm = self.runtime.stream_turn(query);

        let mut last_assistant: Option<Message> = None;
        while let Some(output) = strm.next().await {
            let output = output.map_err(|source| ChatAgentRunError::Runtime { source })?;
            let msg = &output.message;

            // Collect tool calls from assistant messages
            if msg.role == Role::Assistant {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        if let Some((_id, name, args)) = tc.as_function() {
                            self.tool_log.push(ToolCallEntry {
                                tool: name.to_string(),
                                args: ailoy_to_json(args),
                                result: None,
                            });
                        }
                    }
                }
                last_assistant = Some(msg.clone());
            }

            // Attach tool result to the earliest entry still awaiting a response.
            // ailoy's stream_turn yields results in the same order as the calls,
            // so sequential matching is correct even with parallel tool calls.
            if msg.role == Role::Tool {
                for part in &msg.contents {
                    if let Some(value) = part.as_value() {
                        if let Some(entry) = self.tool_log.iter_mut().find(|e| e.result.is_none()) {
                            entry.result = Some(ailoy_to_json(value));
                        }
                    }
                }
            }
        }

        extract_assistant_text(
            &last_assistant.ok_or(ChatAgentRunError::NoTextContent)?,
        )
        .ok_or(ChatAgentRunError::NoTextContent)
    }
}

/// Convert ailoy `Value` to `serde_json::Value` without exposing ailoy types.
fn ailoy_to_json(v: &Value) -> serde_json::Value {
    v.clone().into()
}

/// Build all tools and return their names alongside the ToolSet.
/// Tool names are derived from the same source as their runtimes, ensuring consistency.
fn build_tool_set(
    kb_entries: &[KbEntry],
    sub_provider: SubAgentProvider,
    session_source_paths: Vec<(String, String, PathBuf)>,
) -> (Vec<String>, ToolSet) {
    let mut tool_set = tools::build_default_tool_set();
    let mut tool_names = vec![
        DEFAULT_TOOL_UTC_NOW.to_string(),
        DEFAULT_TOOL_ADD_INTEGERS.to_string(),
    ];
    if let Some((name, runtime)) = speedwagon::build_speedwagon_tool(kb_entries, sub_provider) {
        tool_names.push(name.clone());
        tool_set.insert(name, runtime);
    }
    if let Some((name, runtime)) = tools::build_read_source_tool(session_source_paths) {
        tool_names.push(name.clone());
        tool_set.insert(name, runtime);
    }
    (tool_names, tool_set)
}


fn extract_assistant_text(message: &Message) -> Option<String> {
    let text = message
        .contents
        .iter()
        .filter_map(|part| part.as_text())
        .collect::<Vec<_>>()
        .join("");

    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::{ChatAgent, build_tool_set, extract_assistant_text};
    use crate::speedwagon::SubAgentProvider;
    use crate::tools::{
        DEFAULT_TOOL_ADD_INTEGERS, DEFAULT_TOOL_UTC_NOW, add_integers_result,
        build_default_tool_set,
    };
    use ailoy::{
        AgentProvider, AgentSpec, LangModelAPISchema, LangModelProvider, Message, Part, Role,
        Value,
    };

    fn sample_spec() -> AgentSpec {
        AgentSpec {
            lm: "gpt-4.1-mini".to_string(),
            instruction: None,
            tools: vec![],
        }
    }

    fn sample_provider() -> AgentProvider {
        AgentProvider {
            lm: LangModelProvider::API {
                schema: LangModelAPISchema::ChatCompletion,
                url: "https://example.com/v1/chat/completions"
                    .parse()
                    .expect("valid URL for test provider"),
                api_key: Some("test-key".to_string()),
            },
            tools: vec![],
        }
    }

    #[test]
    fn new_creates_chat_agent_with_runtime() {
        let _agent = ChatAgent::new(sample_spec(), sample_provider(), vec![], vec![]);
    }

    fn sample_sub_provider() -> SubAgentProvider {
        SubAgentProvider {
            api_key: "test-key".to_string(),
            api_url: "https://example.com".to_string(),
            model_name: "gpt-4.1-mini".to_string(),
        }
    }

    #[test]
    fn build_tool_set_returns_default_tool_names() {
        let (tool_names, _tool_set) = build_tool_set(&[], sample_sub_provider(), vec![]);
        assert_eq!(
            tool_names,
            vec![
                DEFAULT_TOOL_UTC_NOW.to_string(),
                DEFAULT_TOOL_ADD_INTEGERS.to_string()
            ]
        );
    }

    #[test]
    fn default_tool_set_contains_two_default_tools() {
        let tool_set = build_default_tool_set();
        assert!(tool_set.get(DEFAULT_TOOL_UTC_NOW).is_some());
        assert!(tool_set.get(DEFAULT_TOOL_ADD_INTEGERS).is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn utc_now_tool_returns_unix_seconds() {
        let tool_set = build_default_tool_set();
        let tool = tool_set
            .get(DEFAULT_TOOL_UTC_NOW)
            .cloned()
            .expect("utc_now tool should exist");

        let tool_message = tool
            .run(Part::function(DEFAULT_TOOL_UTC_NOW, Value::object_empty()))
            .await
            .expect("tool call should succeed");

        assert_eq!(tool_message.role, Role::Tool);
        let value = tool_message
            .contents
            .first()
            .and_then(Part::as_value)
            .expect("tool message should contain value");
        let unix_seconds = value
            .as_object()
            .and_then(|map| map.get("unix_seconds"))
            .and_then(Value::as_unsigned);
        assert!(unix_seconds.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn add_integers_tool_returns_sum_for_valid_args() {
        let tool_set = build_default_tool_set();
        let tool = tool_set
            .get(DEFAULT_TOOL_ADD_INTEGERS)
            .cloned()
            .expect("add_integers tool should exist");

        let tool_message = tool
            .run(Part::function(
                DEFAULT_TOOL_ADD_INTEGERS,
                Value::object([("a", Value::integer(2)), ("b", Value::integer(3))]),
            ))
            .await
            .expect("tool call should succeed");

        let value = tool_message
            .contents
            .first()
            .and_then(Part::as_value)
            .expect("tool message should contain value");
        let sum = value
            .as_object()
            .and_then(|map| map.get("sum"))
            .and_then(Value::as_integer);
        assert_eq!(sum, Some(5));
    }

    #[test]
    fn add_integers_tool_returns_error_for_invalid_or_overflow() {
        let invalid = add_integers_result(Value::object([
            ("a", Value::string("not-a-number")),
            ("b", Value::integer(3)),
        ]));
        let invalid_error = invalid
            .as_object()
            .and_then(|map| map.get("error"))
            .and_then(Value::as_str);
        assert_eq!(invalid_error, Some("invalid_arguments"));

        let overflow = add_integers_result(Value::object([
            ("a", Value::integer(i64::MAX)),
            ("b", Value::integer(1)),
        ]));
        let overflow_error = overflow
            .as_object()
            .and_then(|map| map.get("error"))
            .and_then(Value::as_str);
        assert_eq!(overflow_error, Some("overflow"));
    }

    #[test]
    fn run_user_text_method_is_available() {
        let mut agent = ChatAgent::new(sample_spec(), sample_provider(), vec![], vec![]);
        let fut = agent.run_user_text("hello");
        drop(fut);
    }

    #[test]
    fn extract_assistant_text_joins_text_parts() {
        let message = Message::new(Role::Assistant).with_contents([Part::text("a"), Part::text("b")]);
        assert_eq!(extract_assistant_text(&message), Some("ab".to_string()));
    }

    #[test]
    fn extract_assistant_text_returns_none_when_no_text() {
        let message = Message::new(Role::Assistant);
        assert_eq!(extract_assistant_text(&message), None);
    }
}
