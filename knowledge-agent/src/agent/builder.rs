use std::{path::PathBuf, sync::Arc};

use ailoy::agent::{AgentProvider, AgentRuntime, AgentSpec};

use super::config::AgentConfig;
use crate::tools::{SearchIndex, ToolConfig, build_tool_set};

pub fn build_agent(
    agent_config: &AgentConfig,
    tools_config: &ToolConfig,
    search_index: &Arc<SearchIndex>,
    target_dirs: Vec<PathBuf>,
) -> AgentRuntime {
    let tool_set = build_tool_set(search_index.clone(), tools_config, target_dirs);

    let spec = AgentSpec::new(&agent_config.model_name)
        .with_instruction(agent_config.system_prompt.clone())
        .with_tools(tool_set.names());

    let provider = AgentProvider {
        lm: agent_config.provider.clone(),
        tools: vec![],
    };

    AgentRuntime::new(spec, provider, tool_set)
}
