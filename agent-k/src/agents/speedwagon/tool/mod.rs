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

/// Register the four corpus tool functions on `provider`, bound to `store`.
/// Used both to build Speedwagon's own provider and to add the corpus tools to
/// a parent agent's provider so a Speedwagon sub-agent can resolve them.
pub fn register_corpus_tools(provider: &mut ToolProvider, store: SharedStore) {
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
}

pub fn build_tools(store: SharedStore) -> ToolProvider {
    let mut provider = ToolProvider::new();
    register_corpus_tools(&mut provider, store);
    provider
}
