mod calculate;
mod find;
mod read;
mod search;

use ailoy::tool::ToolProvider;
pub use calculate::*;
pub use find::*;
pub use read::*;
pub use search::*;

use crate::knowledge_base::SharedStore;

pub fn build_tools(store: SharedStore) -> ToolProvider {
    let mut provider = ToolProvider::new();
    provider.insert_func(
        "search_document",
        get_search_document_tool_func(store.clone()),
    );
    provider.insert_func(
        "find_in_document",
        get_find_in_document_tool_func(store.clone()),
    );
    provider.insert_func("read_document", get_read_document_tool_func(store.clone()));
    provider.insert_func("calculate", get_calculate_tool_func());
    provider
}
