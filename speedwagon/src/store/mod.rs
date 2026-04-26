mod document;
mod indexer;
mod parser;
#[cfg(feature = "internal")]
pub mod preset;
mod searcher;
mod translator;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use tantivy::Index;
use uuid::Uuid;

pub use document::{Document, FindResult};
pub use searcher::{SearchPage, SearchResult};

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
    MD,
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

        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &bytes);

        let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
        if !corpus_path.exists() {
            match filetype {
                FileType::MD => {
                    fs::write(&corpus_path, &bytes)?;
                }
                _ => {
                    let ext = filetype.to_string();
                    let origin_path = self.root.join("origin").join(format!("{id}.{ext}"));
                    if !origin_path.exists() {
                        fs::write(&origin_path, &bytes)?;
                    }
                    translator::translate(&origin_path, &corpus_path)?;
                }
            }
        }

        if !indexer::document_exists(&self.index, &id.to_string())? {
            let content = fs::read_to_string(&corpus_path)
                .with_context(|| format!("failed to read corpus: {corpus_path:?}"))?;
            let (title, purpose) = tokio::try_join!(
                parser::get_title(&content),
                parser::get_purpose(&content),
            )?;

            indexer::add_document(
                &self.index,
                &id.to_string(),
                &title,
                &purpose,
                &content,
            )?;
        }

        Ok(id)
    }

    /// Adds multiple files in one batch: translates each to corpus, resolves titles, then
    /// commits them all to the index in a single write.
    pub async fn ingest_many(
        &mut self,
        items: impl IntoIterator<Item = (impl IntoIterator<Item = u8>, FileType)>,
    ) -> Result<Vec<Uuid>> {
        let items = items
            .into_iter()
            .map(|v| (v.0.into_iter().collect::<Vec<_>>(), v.1))
            .collect::<Vec<_>>();
        let mut all_ids = Vec::with_capacity(items.len());
        let mut to_index: Vec<(Uuid, String)> = Vec::new(); // (id, content)

        for (bytes, filetype) in &items {
            let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, bytes);
            all_ids.push(id);

            let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
            if !corpus_path.exists() {
                match filetype {
                    FileType::MD => {
                        fs::write(&corpus_path, bytes)?;
                    }
                    _ => {
                        let ext = filetype.to_string();
                        let origin_path = self.root.join("origin").join(format!("{id}.{ext}"));
                        if !origin_path.exists() {
                            fs::write(&origin_path, bytes)?;
                        }
                        translator::translate(&origin_path, &corpus_path)?;
                    }
                }
            }

            if !indexer::document_exists(&self.index, &id.to_string())? {
                let content = fs::read_to_string(&corpus_path)
                    .with_context(|| format!("failed to read corpus: {corpus_path:?}"))?;
                to_index.push((id, content));
            }
        }

        if to_index.is_empty() {
            return Ok(all_ids);
        }

        let mut docs: Vec<(String, String, String, String)> = Vec::with_capacity(to_index.len());
        for (id, content) in to_index {
            let (title, purpose) = tokio::try_join!(
                parser::get_title(&content),
                parser::get_purpose(&content),
            )?;
            docs.push((id.to_string(), title, purpose, content));
        }

        let refs: Vec<(&str, &str, &str, &str)> = docs
            .iter()
            .map(|(id, title, purpose, content)| {
                (id.as_str(), title.as_str(), purpose.as_str(), content.as_str())
            })
            .collect();
        indexer::add_documents(&self.index, &refs)?;

        Ok(all_ids)
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

    /// Get # of documents
    pub fn count(&self) -> u32 {
        indexer::num_documents(&self.index).unwrap_or(0) as u32
    }

    /// Returns all documents stored in the index.
    pub fn list(&self, include_content: bool, page: u32, page_size: u32) -> Result<Vec<Document>> {
        let all = indexer::list_documents(&self.index, include_content)?;
        let start = (page * page_size) as usize;
        Ok(all
            .into_iter()
            .skip(start)
            .take(page_size as usize)
            .collect())
    }

    pub fn get(&self, id: impl Into<Uuid>) -> Option<Document> {
        let id = id.into();
        indexer::get_document(&self.index, &id.to_string())
            .ok()
            .flatten()
    }

    pub fn get_many(&self, ids: &[Uuid]) -> Result<Vec<Document>> {
        let id_strs: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        let id_refs: Vec<&str> = id_strs.iter().map(String::as_str).collect();
        indexer::get_documents(&self.index, &id_refs)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn search(&self, query: impl AsRef<str>, page: u32, page_size: u32) -> Result<SearchPage> {
        searcher::search_page(&self.index, query.as_ref(), page, page_size)
    }

    /// Reads a byte slice of a document's content.
    /// Returns `None` if the document does not exist or has no content.
    pub fn read(&self, id: Uuid, offset: usize, len: usize) -> Option<String> {
        let doc = self.get(id)?;
        let content = doc.content?;
        Some(document::read_in_document(&content, offset, len))
    }

    /// Searches within a single document's content for pattern matches.
    /// Returns `None` if the document does not exist or has no content.
    pub fn find(
        &self,
        id: Uuid,
        keyword: impl AsRef<str>,
        cursor: usize,
        k: usize,
        context_bytes: usize,
    ) -> Option<FindResult> {
        let doc = self.get(id)?;
        let content = doc.content?;
        Some(document::find_in_document(
            &id.to_string(),
            &content,
            keyword.as_ref(),
            cursor,
            k,
            context_bytes,
        ))
    }
}

#[cfg(test)]
#[cfg(feature = "internal")]
mod tests {
    use knowledge_base_examples::{Cached, DocSet as _, FinanceBench};

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
            let name = kb.filename(i).await.unwrap_or_else(|| format!("doc-{i}"));
            let bytes: Vec<u8> = kb
                .read_origin(i)
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

        let docs = store
            .list(true, 0, u32::MAX)
            .expect("failed to list documents");
        for doc in &docs {
            println!("{doc}");
        }
    }
}
