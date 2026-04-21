mod indexer;
mod parser;
mod searcher;
mod translator;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use tantivy::Index;
use uuid::Uuid;

pub use indexer::Document;
pub use parser::TitleAgent;
pub use searcher::SearchPage;

/// Speedwagon store layout:
///
/// ```text
/// {root}/
/// ├── origin/  ← original source files (pdf, docx, …)
/// ├── corpus/  ← converted markdown files ({uuid}.md)
/// └── index/   ← live Tantivy index
/// ```
pub struct Store {
    root: PathBuf,
    index: Index,
}

#[derive(Debug, Clone, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum FileType {
    PDF,
}

impl Store {
    /// Opens an existing store or creates a new one at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("origin"))?;
        fs::create_dir_all(root.join("corpus"))?;
        let index = indexer::open_or_create(&root.join("index"))?;

        Ok(Self { root, index })
    }

    /// Adds a new file to the store, translates it to corpus, indexes it, and returns its UUID.
    pub async fn ingest(
        &mut self,
        contents: impl IntoIterator<Item = impl Into<u8>>,
        filetype: FileType,
    ) -> Result<Uuid> {
        let bytes: Vec<u8> = contents.into_iter().map(|b| b.into()).collect();
        let ext = filetype.to_string();

        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &bytes);

        let origin_path = self.root.join("origin").join(format!("{id}.{ext}"));
        if !origin_path.exists() {
            fs::write(&origin_path, &bytes)?;
        }

        let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
        if !corpus_path.exists() {
            translator::translate(&origin_path, &corpus_path)?;
        }

        if !indexer::document_exists(&self.index, &id.to_string())? {
            let content = fs::read_to_string(&corpus_path)
                .with_context(|| format!("failed to read corpus: {corpus_path:?}"))?;
            let title = parser::get_title(&content).await?;

            indexer::add_document(&self.index, &id.to_string(), &title, &content)?;
        }

        Ok(id)
    }

    /// Removes a document from the index and deletes its origin and corpus files.
    /// Returns the deleted document, or `None` if no document with that ID exists.
    pub fn purge(&mut self, id: Uuid) -> Result<Option<Document>> {
        let doc = indexer::delete_document(&self.index, &id.to_string())?;

        if doc.is_some() {
            let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
            if corpus_path.exists() {
                fs::remove_file(&corpus_path)?;
            }

            // origin filename includes the original extension; find it by UUID prefix
            let origin_dir = self.root.join("origin");
            for entry in fs::read_dir(&origin_dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(&id.to_string()) {
                    fs::remove_file(entry.path())?;
                    break;
                }
            }
        }

        Ok(doc)
    }

    /// Returns all documents stored in the index.
    pub fn list(&self, include_content: bool) -> Result<Vec<Document>> {
        indexer::list_documents(&self.index, include_content)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn search(&self, query: impl AsRef<str>, page: u32, page_size: u32) -> Result<SearchPage> {
        searcher::search_page(&self.index, query.as_ref(), page, page_size)
    }
}

#[cfg(test)]
mod tests {
    use knowledge_base_examples::{Cached, FinanceBench, KnowledgeBase as _};

    use super::*;

    #[tokio::test]
    #[ignore = "requires network access & docling"]
    async fn test_ingest_financebench_samples() {
        let kb = Cached::new(
            FinanceBench::new()
                .await
                .expect("failed to init FinanceBench"),
        )
        .expect("failed to create cache");

        let store_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".tests/finance-bench");
        let mut store = Store::new(&store_dir).expect("failed to create store");

        let mut ids = Vec::new();
        for i in 0..3 {
            let name = kb.name(i).await;
            let bytes: Vec<u8> = kb
                .contents(i)
                .await
                .unwrap_or_else(|| panic!("failed to fetch {name}"))
                .into();
            let id = store
                .ingest(bytes, FileType::PDF)
                .await
                .unwrap_or_else(|e| panic!("failed to ingest {name}: {e}"));
            println!("[{i}] {name} → {id}");
            ids.push(id);
        }

        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3, "expected 3 unique UUIDs");

        for id in &ids {
            let corpus_path = store_dir.join("corpus").join(format!("{id}.md"));
            assert!(corpus_path.exists(), "corpus file missing: {corpus_path:?}");
            assert!(
                corpus_path.metadata().unwrap().len() > 0,
                "corpus file is empty: {corpus_path:?}"
            );
        }

        let docs = store.list(true).expect("failed to list documents");
        for doc in &docs {
            println!("{doc}");
        }
    }
}
