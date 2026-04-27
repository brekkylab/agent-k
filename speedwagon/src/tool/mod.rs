mod calculate;
mod find;
mod read;
mod search;

use std::sync::Arc;

use ailoy::tool::ToolSet;
pub use calculate::*;
pub use find::*;
pub use read::*;
pub use search::*;

use crate::store::Store;

pub fn build_toolset(store: Arc<Store>) -> ToolSet {
    let mut toolset = ToolSet::new();

    toolset.insert("search_document", make_search_document_tool(store.clone()));
    toolset.insert(
        "find_in_document",
        build_find_in_document_tool(store.clone()),
    );
    toolset.insert("read_document", build_read_document_tool(store.clone()));
    toolset.insert("calculate", build_calculate_tool());
    toolset
}
