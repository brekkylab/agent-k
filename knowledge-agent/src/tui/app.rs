use serde::{Deserialize, Serialize};

use crate::{agent::AgentConfig, tools::ToolConfig};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub tool: ToolConfig,
}

impl AppConfig {
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }
}
