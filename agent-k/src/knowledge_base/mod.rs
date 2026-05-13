mod description;
mod document;
mod helper;
mod indexer;
mod parser;
#[cfg(feature = "internal")]
pub mod preset;
mod searcher;
mod translator;

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::RwLock;

use anyhow::{Context as _, Result};
use tantivy::Index;
use uuid::Uuid;

pub use document::{Document, FindResult};
pub use searcher::{SearchPage, SearchResult};

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub succeeded: Vec<Uuid>,
    pub failed: Vec<IngestFailure>,
}

#[derive(Debug, Clone)]
pub struct IngestFailure {
    pub index: usize,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct PurgeResult {
    pub purged: Vec<Uuid>,
    pub failed: Vec<PurgeFailure>,
}

#[derive(Debug, Clone)]
pub struct PurgeFailure {
    pub id: Uuid,
    pub error: String,
}

pub type SharedStore = Arc<RwLock<Store>>;

fn remove_ingest_artifact(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("failed to clean up ingest artifact {:?}: {e}", path),
    }
}

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
    HTML,
}

impl FileType {
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => Some(Self::PDF),
            "md" => Some(Self::MD),
            "html" | "htm" => Some(Self::HTML),
            _ => None,
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        Self::from_extension(ext)
    }

    pub fn canonical_extension(&self) -> &'static str {
        match self {
            Self::PDF => "pdf",
            Self::MD => "md",
            Self::HTML => "html",
        }
    }

    pub fn supported_extensions() -> &'static [&'static str] {
        &["pdf", "html", "htm", "md"]
    }
}

impl Store {
    /// Opens an existing store or creates a new one at `root`.
    /// LLM-backed metadata (ingest's title/purpose, describe) reads from
    /// ailoy's process-global default provider — populate it once at app
    /// boot via `ailoy::agent::default_provider_mut`.
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
                    let ext = filetype.canonical_extension();
                    let origin_path = self.root.join("origin").join(format!("{id}.{ext}"));
                    if !origin_path.exists() {
                        fs::write(&origin_path, &bytes)?;
                    }
                    translator::translate(filetype, &origin_path, &corpus_path).await?;
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

    /// Adds multiple files in one batch with partial-success semantics.
    /// Successfully processed files are batched into a single index write.
    /// Files that fail at any stage (translation, reading, title extraction) are
    /// recorded in `IngestResult::failed` and their intermediate files are cleaned up.
    pub async fn ingest_many(
        &mut self,
        items: impl IntoIterator<Item = (impl IntoIterator<Item = u8>, FileType)>,
    ) -> Result<IngestResult> {
        let items: Vec<(Vec<u8>, FileType)> = items
            .into_iter()
            .map(|v| (v.0.into_iter().collect::<Vec<_>>(), v.1))
            .collect();

        let mut succeeded = Vec::with_capacity(items.len());
        let mut failed = Vec::new();
        let mut to_index: Vec<(usize, Uuid, String, bool)> = Vec::new(); // (input index, id, content, new_corpus)

        for (idx, (bytes, filetype)) in items.iter().enumerate() {
            let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, bytes);
            let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
            let new_corpus = !corpus_path.exists();

            if new_corpus {
                let mut new_origin: Option<PathBuf> = None;

                let ok = match filetype {
                    FileType::MD => fs::write(&corpus_path, bytes).map_err(|e| e.to_string()),
                    _ => {
                        let ext = filetype.canonical_extension();
                        let origin_path = self.root.join("origin").join(format!("{id}.{ext}"));
                        if !origin_path.exists() {
                            if let Err(e) = fs::write(&origin_path, bytes) {
                                failed.push(IngestFailure {
                                    index: idx,
                                    error: e.to_string(),
                                });
                                continue;
                            }
                            new_origin = Some(origin_path.clone());
                        }
                        translator::translate(filetype.clone(), &origin_path, &corpus_path)
                            .await
                            .map_err(|e| e.to_string())
                    }
                };

                if let Err(e) = ok {
                    if new_corpus {
                        remove_ingest_artifact(&corpus_path);
                    }
                    if let Some(origin) = &new_origin {
                        remove_ingest_artifact(origin);
                    }
                    failed.push(IngestFailure {
                        index: idx,
                        error: e,
                    });
                    continue;
                }
            }

            match indexer::document_exists(&self.index, &id.to_string()) {
                Ok(true) => {
                    succeeded.push(id);
                }
                Ok(false) => match fs::read_to_string(&corpus_path) {
                    Ok(content) => {
                        to_index.push((idx, id, content, new_corpus));
                    }
                    Err(e) => {
                        if new_corpus {
                            remove_ingest_artifact(&corpus_path);
                        }
                        failed.push(IngestFailure {
                            index: idx,
                            error: e.to_string(),
                        });
                    }
                },
                Err(e) => {
                    failed.push(IngestFailure {
                        index: idx,
                        error: e.to_string(),
                    });
                }
            }
        }

        let mut docs: Vec<(String, String, String, String)> = Vec::with_capacity(to_index.len());
        for (idx, id, content, new_corpus) in to_index {
            match tokio::try_join!(parser::get_title(&content), parser::get_purpose(&content)) {
                Ok((title, purpose)) => docs.push((id.to_string(), title, purpose, content)),
                Err(e) => {
                    let corpus_path = self.root.join("corpus").join(format!("{id}.md"));
                    if new_corpus {
                        remove_ingest_artifact(&corpus_path);
                    }
                    failed.push(IngestFailure {
                        index: idx,
                        error: e.to_string(),
                    });
                }
            }
        }

        if !docs.is_empty() {
            let refs: Vec<(&str, &str, &str, &str)> = docs
                .iter()
                .map(|(id, title, purpose, content)| {
                    (id.as_str(), title.as_str(), purpose.as_str(), content.as_str())
                })
                .collect();
            indexer::add_documents(&self.index, &refs)?;

            for (id_str, _, _, _) in &docs {
                if let Ok(id) = id_str.parse::<Uuid>() {
                    succeeded.push(id);
                }
            }
        }

        Ok(IngestResult { succeeded, failed })
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

    /// Removes multiple documents with partial-success semantics.
    pub fn purge_many(&mut self, ids: impl IntoIterator<Item = Uuid>) -> PurgeResult {
        let mut purged = Vec::new();
        let mut failed = Vec::new();

        for id in ids {
            match self.purge(id) {
                Ok(Some(_)) => purged.push(id),
                Ok(None) => failed.push(PurgeFailure {
                    id,
                    error: "document not found".into(),
                }),
                Err(e) => failed.push(PurgeFailure {
                    id,
                    error: e.to_string(),
                }),
            }
        }

        PurgeResult { purged, failed }
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

    /// One LLM call over every doc's `(title, purpose)` in the index. Input
    /// is proportional to doc count (~24K chars at N=200), so don't run this
    /// synchronously on indexing hot paths — use a finalize hook or
    /// background job. Empty LLM body falls back to a deterministic string.
    pub async fn describe(
        &self,
        kb_name: &str,
        instruction: Option<&str>,
    ) -> Result<String> {
        let docs = indexer::list_documents(&self.index, false)?;
        if docs.is_empty() {
            return Ok(String::new());
        }
        let pairs: Vec<(&str, &str)> = docs
            .iter()
            .map(|d| (d.title.as_str(), d.purpose.as_str()))
            .collect();
        description::get_description(kb_name, instruction, &pairs).await
    }
}

#[cfg(test)]
mod filetype_tests {
    use super::FileType;
    use std::path::Path;

    #[test]
    fn filetype_from_extension_maps_supported_extensions() {
        assert!(matches!(
            FileType::from_extension("pdf"),
            Some(FileType::PDF)
        ));
        assert!(matches!(
            FileType::from_extension("PDF"),
            Some(FileType::PDF)
        ));
        assert!(matches!(
            FileType::from_extension("html"),
            Some(FileType::HTML)
        ));
        assert!(matches!(
            FileType::from_extension("htm"),
            Some(FileType::HTML)
        ));
        assert!(matches!(FileType::from_extension("md"), Some(FileType::MD)));
    }

    #[test]
    fn filetype_from_extension_rejects_unknown_extensions() {
        assert!(FileType::from_extension("txt").is_none());
        assert!(FileType::from_extension("").is_none());
    }

    #[test]
    fn filetype_from_path_and_canonical_extension() {
        assert!(matches!(
            FileType::from_path(Path::new("/tmp/a.PDF")),
            Some(FileType::PDF)
        ));
        assert!(matches!(
            FileType::from_path(Path::new("/tmp/a.htm")),
            Some(FileType::HTML)
        ));
        assert!(FileType::from_path(Path::new("/tmp/a")).is_none());

        assert_eq!(FileType::PDF.canonical_extension(), "pdf");
        assert_eq!(FileType::HTML.canonical_extension(), "html");
        assert_eq!(FileType::MD.canonical_extension(), "md");
    }
}

#[cfg(test)]
#[cfg(feature = "internal")]
mod tests {
    use knowledge_base_examples::{Cached, DocSet as _, FinanceBench};

    use super::*;

    #[tokio::test]
    async fn ingest_many_preserves_existing_corpus_when_corpus_read_fails() {
        let tempdir = tempfile::tempdir().expect("failed to create tempdir");
        let mut store = Store::new(tempdir.path()).expect("failed to create store");

        let bytes = b"same input bytes";
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, bytes);
        let corpus_path = tempdir.path().join("corpus").join(format!("{id}.md"));
        let invalid_utf8 = [0xff, 0xfe, 0xfd];
        fs::write(&corpus_path, invalid_utf8).expect("failed to seed existing corpus");

        let result = store
            .ingest_many([(bytes.to_vec(), FileType::MD)])
            .await
            .expect("ingest_many should report per-item failure");

        assert!(result.succeeded.is_empty());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(
            fs::read(&corpus_path).expect("existing corpus should remain"),
            invalid_utf8
        );
    }

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
