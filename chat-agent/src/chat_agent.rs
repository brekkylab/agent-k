use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;

use ailoy::{AgentProvider, AgentRuntime, AgentSpec, Message, Part, Role, ToolSet, Value};
use futures::{Stream, StreamExt as _};

use crate::speedwagon::{self, KbEntry, SubAgentProvider};
use crate::tools;

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

/// Events emitted during streaming execution of a user message.
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// LLM call started
    Thinking,
    /// LLM decided to call a tool
    ToolCall {
        tool: String,
        args: serde_json::Value,
    },
    /// Tool execution completed
    ToolResult {
        tool: String,
        result: serde_json::Value,
    },
    /// Final assistant response with collected tool call history
    Message {
        content: String,
        tool_calls: Vec<ToolCallEntry>,
    },
}

impl ChatAgent {
    pub fn new(
        mut spec: AgentSpec,
        provider: AgentProvider,
        kb_entries: Vec<KbEntry>,
        kb_overrides: HashMap<String, SubAgentProvider>,
        session_source_paths: Vec<(String, String, PathBuf)>,
    ) -> Self {
        // Extract API credentials from the parent provider; model name passed separately as fallback
        let default_provider = SubAgentProvider::from_provider(&provider);
        let default_model_name = spec.lm.clone();
        let (tool_names, tool_set) = build_tool_set(
            &kb_entries,
            default_provider,
            default_model_name,
            kb_overrides,
            session_source_paths,
        );
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

        extract_assistant_text(&last_assistant.ok_or(ChatAgentRunError::NoTextContent)?)
            .ok_or(ChatAgentRunError::NoTextContent)
    }

    /// Returns the current conversation history (clone).
    pub fn get_history(&self) -> Vec<Message> {
        self.runtime.get_history()
    }

    /// Restore conversation history from DB messages.
    /// Only user/assistant messages are included; system/tool roles are skipped.
    /// history[0] is always System(system_prompt).
    pub fn restore_history(
        &mut self,
        system_prompt: String,
        messages: Vec<(String, String)>, // (role_str, content) pairs
    ) {
        let mut history =
            vec![Message::new(Role::System).with_contents([Part::text(system_prompt)])];
        for (role_str, content) in messages {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => continue, // skip system/tool messages
            };
            history.push(Message::new(role).with_contents([Part::text(content)]));
        }
        self.runtime.set_history(history);
    }

    /// Trim conversation history to keep at most `MAX_TURNS` most recent turns.
    ///
    /// A "turn" is defined as starting at each User message. `history[0]` (System message)
    /// is always preserved. Tool call / tool result messages within a turn are never split
    /// because trimming only cuts at User message boundaries.
    pub fn trim_history(&mut self) {
        const MAX_TURNS: usize = 20;

        let history = self.get_history();

        // Collect indices of all User messages after history[0]
        let turn_starts: Vec<usize> = history
            .iter()
            .enumerate()
            .filter(|(i, m)| *i > 0 && m.role == Role::User)
            .map(|(i, _)| i)
            .collect();

        if turn_starts.len() <= MAX_TURNS {
            return; // No trimming needed
        }

        // Keep only the last MAX_TURNS turns
        let cutoff = turn_starts[turn_starts.len() - MAX_TURNS];
        let mut trimmed = vec![history[0].clone()]; // Always preserve System message
        trimmed.extend_from_slice(&history[cutoff..]);
        self.runtime.set_history(trimmed);
    }

    /// Replace history[0] with a new System message containing the given instruction.
    /// If history is empty or history[0] is not System, inserts at position 0.
    ///
    /// **Currently unused** — prepared for Phase 2.5 (per-turn system prompt refresh).
    /// Today, the system prompt is only assembled at runtime creation time in
    /// `state.rs::get_or_create_runtime_for_session()`. This method will be called
    /// from `run_user_text_streaming()` once mid-session Settings changes need to
    /// be reflected without cache invalidation.
    pub fn update_system_prompt(&mut self, instruction: String) {
        let mut history = self.get_history();
        let system_msg = Message::new(Role::System).with_contents([Part::text(instruction)]);
        if history.is_empty() || history[0].role != Role::System {
            history.insert(0, system_msg);
        } else {
            history[0] = system_msg;
        }
        self.runtime.set_history(history);
    }

    /// Streaming version of `run_user_text` that yields `ChatEvent`s as they happen.
    ///
    /// Partial borrow strategy: `self.runtime` and `self.tool_log` are destructured
    /// before entering the `async_stream::stream!` macro to avoid capturing all of `self`.
    pub fn run_user_text_streaming(
        &mut self,
        content: impl Into<String>,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatEvent, ChatAgentRunError>> + '_>> {
        self.tool_log.clear();
        let query = Message::new(Role::User).with_contents([Part::text(content.into())]);

        // Partial borrow: strm borrows self.runtime, tool_log borrows self.tool_log
        let mut strm = self.runtime.stream_turn(query);
        let tool_log = &mut self.tool_log;

        Box::pin(async_stream::stream! {
            yield Ok(ChatEvent::Thinking);

            let mut last_assistant_text: Option<String> = None;

            while let Some(output) = strm.next().await {
                let output = match output {
                    Ok(o) => o,
                    Err(e) => {
                        yield Err(ChatAgentRunError::Runtime { source: e });
                        return;
                    }
                };
                let msg = &output.message;

                if msg.role == Role::Assistant {
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            if let Some((_id, name, args)) = tc.as_function() {
                                tool_log.push(ToolCallEntry {
                                    tool: name.to_string(),
                                    args: ailoy_to_json(args),
                                    result: None,
                                });
                                yield Ok(ChatEvent::ToolCall {
                                    tool: name.to_string(),
                                    args: ailoy_to_json(args),
                                });
                            }
                        }
                    }
                    last_assistant_text = extract_assistant_text(msg);
                }

                if msg.role == Role::Tool {
                    for part in &msg.contents {
                        if let Some(value) = part.as_value() {
                            let json_value = ailoy_to_json(value);
                            let tool_name = tool_log.iter().rev()
                                .find(|e| e.result.is_none())
                                .map(|e| e.tool.clone())
                                .unwrap_or_default();
                            if let Some(entry) = tool_log.iter_mut().find(|e| e.result.is_none()) {
                                entry.result = Some(json_value.clone());
                            }
                            yield Ok(ChatEvent::ToolResult {
                                tool: tool_name,
                                result: json_value,
                            });
                        }
                    }
                }
            }

            if let Some(content) = last_assistant_text {
                yield Ok(ChatEvent::Message {
                    content,
                    tool_calls: tool_log.clone(),
                });
            } else {
                yield Err(ChatAgentRunError::NoTextContent);
            }
        })
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
    default_provider: SubAgentProvider,
    default_model_name: String,
    kb_overrides: HashMap<String, SubAgentProvider>,
    session_source_paths: Vec<(String, String, PathBuf)>,
) -> (Vec<String>, ToolSet) {
    let mut tool_set = tools::build_default_tool_set();
    let mut tool_names = tool_set.names();
    if let Some((name, runtime)) = speedwagon::build_speedwagon_tool(
        kb_entries,
        default_provider,
        default_model_name,
        kb_overrides,
    ) {
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
    use ailoy::{
        AgentProvider, AgentSpec, LangModelAPISchema, LangModelProvider, Message, Part, Role,
    };
    use std::collections::HashMap;

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
        let _agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
    }

    fn sample_default_provider() -> SubAgentProvider {
        SubAgentProvider {
            api_key: "test-key".to_string(),
            api_url: "https://example.com".parse().unwrap(),
            schema: LangModelAPISchema::ChatCompletion,
        }
    }

    #[test]
    fn build_tool_set_returns_default_tool_names() {
        let (tool_names, _tool_set) = build_tool_set(
            &[],
            sample_default_provider(),
            "gpt-4.1-mini".into(),
            HashMap::new(),
            vec![],
        );
        assert_eq!(tool_names, vec!["web_search"]);
    }

    #[test]
    fn run_user_text_method_is_available() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        let fut = agent.run_user_text("hello");
        drop(fut);
    }

    #[test]
    fn extract_assistant_text_joins_text_parts() {
        let message =
            Message::new(Role::Assistant).with_contents([Part::text("a"), Part::text("b")]);
        assert_eq!(extract_assistant_text(&message), Some("ab".to_string()));
    }

    #[test]
    fn extract_assistant_text_returns_none_when_no_text() {
        let message = Message::new(Role::Assistant);
        assert_eq!(extract_assistant_text(&message), None);
    }

    #[test]
    fn update_system_prompt_on_new_agent() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        agent.update_system_prompt("Hello system".to_string());
        let history = agent.get_history();
        assert!(!history.is_empty());
        assert_eq!(history[0].role, Role::System);
        let text = history[0]
            .contents
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text, "Hello system");
    }

    #[test]
    fn update_system_prompt_replaces_existing() {
        let spec = AgentSpec {
            lm: "gpt-4.1-mini".to_string(),
            instruction: Some("original".to_string()),
            tools: vec![],
        };
        let mut agent = ChatAgent::new(spec, sample_provider(), vec![], HashMap::new(), vec![]);
        let history_before = agent.get_history();
        assert_eq!(history_before.len(), 1); // System message from spec.instruction

        agent.update_system_prompt("replaced".to_string());
        let history_after = agent.get_history();
        assert_eq!(history_after.len(), 1); // Still 1 message, replaced
        let text = history_after[0]
            .contents
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(text, "replaced");
    }

    #[test]
    fn get_history_returns_clone() {
        let agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        let h1 = agent.get_history();
        let h2 = agent.get_history();
        // Both should be equal (same content)
        assert_eq!(h1.len(), h2.len());
    }

    #[test]
    fn restore_history_sets_system_and_messages() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        agent.restore_history(
            "System prompt".to_string(),
            vec![
                ("user".to_string(), "Hello".to_string()),
                ("assistant".to_string(), "Hi there".to_string()),
                ("user".to_string(), "How are you?".to_string()),
            ],
        );
        let history = agent.get_history();
        assert_eq!(history.len(), 4); // system + 3 messages
        assert_eq!(history[0].role, Role::System);
        assert_eq!(history[1].role, Role::User);
        assert_eq!(history[2].role, Role::Assistant);
        assert_eq!(history[3].role, Role::User);
    }

    #[test]
    fn trim_history_preserves_system_and_recent_turns() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        // Build 25 user/assistant pairs via restore_history (system + 25 turns = 51 messages)
        let messages: Vec<(String, String)> = (0..25)
            .flat_map(|i| {
                vec![
                    ("user".to_string(), format!("Q{i}")),
                    ("assistant".to_string(), format!("A{i}")),
                ]
            })
            .collect();
        agent.restore_history("System".to_string(), messages);

        agent.trim_history();

        let trimmed = agent.get_history();
        // System + 20 turns (40 messages) = 41
        assert_eq!(trimmed[0].role, Role::System);
        let user_count = trimmed.iter().filter(|m| m.role == Role::User).count();
        assert_eq!(user_count, 20);
        // Oldest kept turn is turn index 5 (Q5), since we dropped turns 0-4
        let first_user_text = trimmed[1]
            .contents
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(first_user_text, "Q5");
    }

    #[test]
    fn trim_history_no_op_when_under_limit() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        let messages: Vec<(String, String)> = (0..5)
            .flat_map(|i| {
                vec![
                    ("user".to_string(), format!("Q{i}")),
                    ("assistant".to_string(), format!("A{i}")),
                ]
            })
            .collect();
        agent.restore_history("System".to_string(), messages);

        let before_len = agent.get_history().len();
        agent.trim_history();
        let after_len = agent.get_history().len();

        assert_eq!(before_len, after_len); // No change
    }

    #[test]
    fn restore_history_skips_tool_and_system_roles() {
        let mut agent = ChatAgent::new(
            sample_spec(),
            sample_provider(),
            vec![],
            HashMap::new(),
            vec![],
        );
        agent.restore_history(
            "System prompt".to_string(),
            vec![
                ("user".to_string(), "Hello".to_string()),
                ("system".to_string(), "Should be skipped".to_string()),
                ("tool".to_string(), "Should be skipped too".to_string()),
                ("assistant".to_string(), "Response".to_string()),
            ],
        );
        let history = agent.get_history();
        assert_eq!(history.len(), 3); // system + user + assistant (system/tool skipped)
    }
}
