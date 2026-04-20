//! Main-agent tool definitions.
//!
//! Contains the default tools (`web_search`, `convert_pdf_to_md`), the
//! `read_source` tool, and `open_document` (read line ranges from .md/.txt
//! files — pairs with `convert_pdf_to_md`). These are tools used directly by
//! the parent ChatAgent — **not** by speedwagon sub-agents (which live in
//! `speedwagon/`).
//!
//! ## Adding a new tool
//!
//! 1. Define `build_X_tool(...) -> Option<(String, ToolRuntime)>` in a
//!    submodule under `tools/`.
//! 2. Re-export its public items from this module.
//! 3. Add the call to `build_tool_set()` in `chat_agent.rs`.
//!    Names and runtimes are collected together, so no separate registration step.
//!
//! This convention is intentionally simple for the current scale.
//! A trait-based plugin system may replace it when dynamic tool loading is needed.

pub mod open;
pub mod read;

use ailoy::{ToolSet, agent::BuiltinToolProvider};

pub use open::{OPEN_FILE_TOOL, build_open_file_tool};
pub use read::{READ_SOURCE_TOOL, build_read_source_tool};

// ---------------------------------------------------------------------------
// Default tool set (web_search, convert_pdf_to_md)
// ---------------------------------------------------------------------------

pub async fn build_default_tool_set() -> anyhow::Result<ToolSet> {
    let tool_set = ToolSet::new()
        .with_builtin(&BuiltinToolProvider::WebSearch {})
        .await?;
    tool_set
        .with_builtin(&BuiltinToolProvider::ConvertPdfToMd {})
        .await
}
