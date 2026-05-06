mod calculate;
mod find;
mod read;
mod search;

use std::sync::Arc;

use ailoy::tool::{ToolFactory, ToolProvider};
pub use calculate::*;
pub use find::*;
pub use read::*;
pub use search::*;

use crate::store::Store;

pub fn build_tool_provider(store: Arc<Store>) -> ToolProvider {
    let mut provider = ToolProvider::new();

    let (desc, func) = make_search_document_tool(store.clone());
    provider = provider.custom(ToolFactory::simple(desc, func));
    let (desc, func) = build_find_in_document_tool(store.clone());
    provider = provider.custom(ToolFactory::simple(desc, func));
    let (desc, func) = build_read_document_tool(store.clone());
    provider = provider.custom(ToolFactory::simple(desc, func));
    let (desc, func) = build_calculate_tool();
    provider = provider.custom(ToolFactory::simple(desc, func));
    provider
}
