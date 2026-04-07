use serde::{Deserialize, Serialize};
use url::Url;
use utoipa::ToSchema;

/// Defines the logical identity of an agent as configured by the user.
///
/// `AgentSpec` captures what makes an agent distinct - the language model it uses,
/// the system instruction that shapes its behavior, and the set of tools it has access to.
/// Changing any of these fields changes the fundamental nature of the agent.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AgentSpec {
    /// Identifier of the language model (e.g. `"claude-sonnet-4-6"`)
    pub lm: String,

    /// Optional system prompt that guides the agent's behavior
    pub instruction: Option<String>,

    /// Names of tools available to the agent
    pub tools: Vec<String>,
}

#[allow(dead_code)]
impl AgentSpec {
    pub fn new(lm: impl Into<String>) -> Self {
        Self {
            lm: lm.into(),
            instruction: None,
            tools: vec![],
        }
    }

    pub fn with_instruction(mut self, inst: String) -> Self {
        self.instruction = Some(inst);
        self
    }

    pub fn with_tools(mut self, tools: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tools = tools.into_iter().map(|v| v.into()).collect();
        self
    }
}

/// Wire protocol used when calling a language model API.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LangModelAPISchema {
    /// OpenAI-compatible `/v1/chat/completions` format
    ChatCompletion,

    /// Anthropic Messages API format
    Anthropic,

    /// Google Gemini API format
    Gemini,

    /// OpenAI Responses API format
    OpenAI,
}

/// Describes the runtime endpoint used to invoke a language model.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LangModelProvider {
    /// Calls a remote HTTP API. Requires the wire `schema`, the `url` of the endpoint, and an optional `api_key` for authentication.
    API {
        schema: LangModelAPISchema,

        url: Url,

        api_key: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuiltinToolProvider {
    WebSearch {},
}

/// Transport configuration for an MCP (Model Context Protocol) tool server.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MCPToolProvider {
    /// Spawns a child process and communicates over its stdio
    Stdio { command: String },

    /// Connects to a remote MCP server over HTTP streaming
    StreamableHTTP { url: Url },
}

/// Identifies where a tool's implementation lives at runtime.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolProvider {
    /// A tool baked into the agent runtime, referenced by `name`
    Builtin(BuiltinToolProvider),

    /// A tool served by an external MCP server described by [`MCPToolProvider`]
    MCP(MCPToolProvider),
}

/// Supplies the runtime parameters needed to execute an agent.
///
/// `AgentProvider` is separate from [`AgentSpec`] because these settings describe *how*
/// to run an agent, not *what* the agent is. Swapping the API endpoint or key does not
/// change the agent's identity; swapping the model or instruction does.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AgentProvider {
    /// The concrete language model provider (API schema, endpoint URL, credentials)
    pub lm: LangModelProvider,

    /// Resolved tool providers that back each tool name declared in [`AgentSpec::tools`]
    pub tools: Vec<ToolProvider>,
}

impl From<AgentSpec> for ailoy::AgentSpec {
    fn from(value: AgentSpec) -> Self {
        Self {
            lm: value.lm,
            instruction: value.instruction,
            tools: value.tools,
        }
    }
}

impl From<ailoy::AgentSpec> for AgentSpec {
    fn from(value: ailoy::AgentSpec) -> Self {
        Self {
            lm: value.lm,
            instruction: value.instruction,
            tools: value.tools,
        }
    }
}

impl From<LangModelAPISchema> for ailoy::LangModelAPISchema {
    fn from(value: LangModelAPISchema) -> Self {
        match value {
            LangModelAPISchema::ChatCompletion => Self::ChatCompletion,
            LangModelAPISchema::Anthropic => Self::Anthropic,
            LangModelAPISchema::Gemini => Self::Gemini,
            LangModelAPISchema::OpenAI => Self::OpenAI,
        }
    }
}

impl From<ailoy::LangModelAPISchema> for LangModelAPISchema {
    fn from(value: ailoy::LangModelAPISchema) -> Self {
        match value {
            ailoy::LangModelAPISchema::ChatCompletion => Self::ChatCompletion,
            ailoy::LangModelAPISchema::Anthropic => Self::Anthropic,
            ailoy::LangModelAPISchema::Gemini => Self::Gemini,
            ailoy::LangModelAPISchema::OpenAI => Self::OpenAI,
        }
    }
}

impl From<LangModelProvider> for ailoy::LangModelProvider {
    fn from(value: LangModelProvider) -> Self {
        match value {
            LangModelProvider::API {
                schema,
                url,
                api_key,
            } => Self::API {
                schema: schema.into(),
                url,
                api_key,
            },
        }
    }
}

impl From<ailoy::LangModelProvider> for LangModelProvider {
    fn from(value: ailoy::LangModelProvider) -> Self {
        match value {
            ailoy::LangModelProvider::API {
                schema,
                url,
                api_key,
            } => Self::API {
                schema: schema.into(),
                url,
                api_key,
            },
        }
    }
}

impl From<BuiltinToolProvider> for ailoy::agent::BuiltinToolProvider {
    fn from(value: BuiltinToolProvider) -> Self {
        match value {
            BuiltinToolProvider::WebSearch {} => Self::WebSearch {},
        }
    }
}

impl From<ailoy::agent::BuiltinToolProvider> for BuiltinToolProvider {
    fn from(value: ailoy::agent::BuiltinToolProvider) -> Self {
        match value {
            ailoy::agent::BuiltinToolProvider::WebSearch {} => Self::WebSearch {},
        }
    }
}

impl From<MCPToolProvider> for ailoy::agent::MCPToolProvider {
    fn from(value: MCPToolProvider) -> Self {
        match value {
            MCPToolProvider::Stdio { command } => Self::Stdio { command },
            MCPToolProvider::StreamableHTTP { url } => Self::StreamableHTTP { url },
        }
    }
}

impl From<ailoy::agent::MCPToolProvider> for MCPToolProvider {
    fn from(value: ailoy::agent::MCPToolProvider) -> Self {
        match value {
            ailoy::agent::MCPToolProvider::Stdio { command } => Self::Stdio { command },
            ailoy::agent::MCPToolProvider::StreamableHTTP { url } => Self::StreamableHTTP { url },
        }
    }
}

impl From<ToolProvider> for ailoy::ToolProvider {
    fn from(value: ToolProvider) -> Self {
        match value {
            ToolProvider::Builtin(builtin) => Self::Builtin(builtin.into()),
            ToolProvider::MCP(mcp) => Self::MCP(mcp.into()),
        }
    }
}

impl From<ailoy::ToolProvider> for ToolProvider {
    fn from(value: ailoy::ToolProvider) -> Self {
        match value {
            ailoy::ToolProvider::Builtin(builtin) => Self::Builtin(builtin.into()),
            ailoy::ToolProvider::MCP(mcp) => Self::MCP(mcp.into()),
        }
    }
}

impl From<AgentProvider> for ailoy::AgentProvider {
    fn from(value: AgentProvider) -> Self {
        Self {
            lm: value.lm.into(),
            tools: value.tools.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ailoy::AgentProvider> for AgentProvider {
    fn from(value: ailoy::AgentProvider) -> Self {
        Self {
            lm: value.lm.into(),
            tools: value.tools.into_iter().map(Into::into).collect(),
        }
    }
}
