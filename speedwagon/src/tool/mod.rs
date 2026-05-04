mod calculate;
mod find;
mod read;
mod search;

use ailoy::tool::ToolProvider;
pub use calculate::*;
pub use find::*;
pub use read::*;
pub use search::*;

use crate::store::SharedStore;

pub fn build_tools(store: SharedStore) -> ToolProvider {
    let mut provider = ToolProvider::new().bash().python_repl().web_search();
    provider = provider.custom(make_search_document_tool(store.clone()));
    provider = provider.custom(build_find_in_document_tool(store.clone()));
    provider = provider.custom(build_read_document_tool(store.clone()));
    provider = provider.custom(build_calculate_tool());
    provider
}
