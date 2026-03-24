use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ailoy::agent::ToolFunc;
use ailoy::{
    AgentProvider, AgentRuntime, AgentSpec, Message, Part, Role, ToolDescBuilder, ToolRuntime,
    ToolSet, Value,
};

const DEFAULT_TOOL_UTC_NOW: &str = "utc_now";
const DEFAULT_TOOL_ADD_INTEGERS: &str = "add_integers";

/// Lightweight owner of an `AgentRuntime` for chat use cases.
pub struct ChatAgent {
    runtime: AgentRuntime,
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
    pub fn new(mut spec: AgentSpec, provider: AgentProvider) -> Self {
        ensure_default_tool_names(&mut spec);
        let runtime = AgentRuntime::new(spec, provider, build_default_tool_set());
        Self { runtime }
    }

    pub async fn run_user_text(
        &mut self,
        content: impl Into<String>,
    ) -> Result<String, ChatAgentRunError> {
        let query = Message::new(Role::User).with_contents([Part::text(content.into())]);
        let message = self
            .runtime
            .run(query)
            .await
            .map_err(|source| ChatAgentRunError::Runtime { source })?;

        extract_assistant_text(&message).ok_or(ChatAgentRunError::NoTextContent)
    }
}

fn ensure_default_tool_names(spec: &mut AgentSpec) {
    for tool_name in [DEFAULT_TOOL_UTC_NOW, DEFAULT_TOOL_ADD_INTEGERS] {
        if !spec.tools.iter().any(|name| name == tool_name) {
            spec.tools.push(tool_name.to_string());
        }
    }
}

fn build_default_tool_set() -> ToolSet {
    let mut tool_set = ToolSet::new();
    tool_set.insert(
        DEFAULT_TOOL_UTC_NOW.to_string(),
        ToolRuntime::new(utc_now_tool_desc(), utc_now_tool()),
    );
    tool_set.insert(
        DEFAULT_TOOL_ADD_INTEGERS.to_string(),
        ToolRuntime::new(add_integers_tool_desc(), add_integers_tool()),
    );
    tool_set
}

fn utc_now_tool_desc() -> ailoy::ToolDesc {
    ToolDescBuilder::new(DEFAULT_TOOL_UTC_NOW)
        .description("Return the current UTC Unix timestamp in seconds.")
        .parameters(Value::object([
            ("type", Value::string("object")),
            ("properties", Value::object_empty()),
        ]))
        .returns(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([(
                    "unix_seconds",
                    Value::object([("type", Value::string("number"))]),
                )]),
            ),
        ]))
        .build()
}

fn add_integers_tool_desc() -> ailoy::ToolDesc {
    ToolDescBuilder::new(DEFAULT_TOOL_ADD_INTEGERS)
        .description("Add two integer values and return their sum.")
        .parameters(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "a",
                        Value::object([("type", Value::string("number"))]),
                    ),
                    (
                        "b",
                        Value::object([("type", Value::string("number"))]),
                    ),
                ]),
            ),
            (
                "required",
                Value::array([Value::string("a"), Value::string("b")]),
            ),
        ]))
        .returns(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "sum",
                        Value::object([("type", Value::string("number"))]),
                    ),
                    (
                        "error",
                        Value::object([("type", Value::string("string"))]),
                    ),
                ]),
            ),
        ]))
        .build()
}

fn utc_now_tool() -> Arc<ToolFunc> {
    Arc::new(|_args: Value| {
        Box::pin(async move {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => Value::object([("unix_seconds", Value::unsigned(duration.as_secs()))]),
                Err(_) => Value::object([("error", Value::string("time_before_unix_epoch"))]),
            }
        })
    })
}

fn add_integers_tool() -> Arc<ToolFunc> {
    Arc::new(|args: Value| Box::pin(async move { add_integers_result(args) }))
}

fn add_integers_result(args: Value) -> Value {
    let Some(args_map) = args.as_object() else {
        return invalid_arguments_value();
    };

    let Some(a) = args_map.get("a").and_then(Value::as_integer) else {
        return invalid_arguments_value();
    };
    let Some(b) = args_map.get("b").and_then(Value::as_integer) else {
        return invalid_arguments_value();
    };

    match a.checked_add(b) {
        Some(sum) => Value::object([("sum", Value::integer(sum))]),
        None => Value::object([("error", Value::string("overflow"))]),
    }
}

fn invalid_arguments_value() -> Value {
    Value::object([("error", Value::string("invalid_arguments"))])
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
    use super::{
        ChatAgent, DEFAULT_TOOL_ADD_INTEGERS, DEFAULT_TOOL_UTC_NOW, add_integers_result,
        build_default_tool_set, ensure_default_tool_names, extract_assistant_text,
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
        let _agent = ChatAgent::new(sample_spec(), sample_provider());
    }

    #[test]
    fn new_injects_default_tool_names_when_spec_empty() {
        let mut spec = sample_spec();
        ensure_default_tool_names(&mut spec);
        assert_eq!(
            spec.tools,
            vec![
                DEFAULT_TOOL_UTC_NOW.to_string(),
                DEFAULT_TOOL_ADD_INTEGERS.to_string()
            ]
        );
    }

    #[test]
    fn new_does_not_duplicate_default_tool_names_when_already_present() {
        let mut spec = sample_spec();
        spec.tools = vec![
            "custom_tool".to_string(),
            DEFAULT_TOOL_UTC_NOW.to_string(),
            DEFAULT_TOOL_ADD_INTEGERS.to_string(),
        ];

        ensure_default_tool_names(&mut spec);

        assert_eq!(
            spec.tools,
            vec![
                "custom_tool".to_string(),
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
        let mut agent = ChatAgent::new(sample_spec(), sample_provider());
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
