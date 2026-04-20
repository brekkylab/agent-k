mod calculate;
mod common;
mod find;
mod glob;
mod open;
mod search;

use std::{path::PathBuf, sync::Arc};

use ailoy::agent::ToolSet;
pub use calculate::{CalculateResult, build_calculate_tool, calculate};
pub use find::{
    FindMatch, FindPosition, FindResult, build_find_in_document_tool, find_in_document,
};
pub use glob::{GlobMatch, GlobResult, build_glob_document_tool};
pub use open::{OpenResult, build_open_document_tool, open_document};
pub use search::{SearchIndex, SearchOutput, SearchResult, build_search_document_tool};
use serde::{Deserialize, Serialize};

use self::{
    calculate::build_calculate_tool as _calculate_builder,
    find::build_find_in_document_tool as _find_builder,
    glob::build_glob_document_tool as _glob_builder,
    open::build_open_document_tool as _open_builder,
    search::build_search_document_tool as _search_builder,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub top_k_max: usize,
    pub context_lines_max: usize,
    pub max_matches: usize,
    pub max_snippet_lines: usize,
    pub max_content_chars: usize,
    pub max_lines_per_open: usize,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            top_k_max: 5,
            context_lines_max: 10,
            max_matches: 20,
            max_snippet_lines: 40,
            max_content_chars: 8000,
            max_lines_per_open: 200,
        }
    }
}

pub fn build_tool_set(
    index: Arc<SearchIndex>,
    config: &ToolConfig,
    target_dirs: Vec<PathBuf>,
) -> ToolSet {
    let mut tool_set = ToolSet::new();

    let tools = vec![
        _search_builder(index.clone(), config.top_k_max as i64),
        _find_builder(index.clone(), config.max_matches),
        _open_builder(
            index.clone(),
            config.max_content_chars,
            config.max_lines_per_open,
        ),
        _glob_builder(target_dirs),
        _calculate_builder(),
    ];

    for tool in tools {
        tool_set.insert(tool.desc().name.clone(), tool);
    }

    tool_set
}
