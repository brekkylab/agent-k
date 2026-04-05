use std::{collections::HashSet, path::Path, sync::Arc, time::Instant};

use ailoy::{ToolDescBuilder, ToolRuntime, Value, agent::ToolFunc};
use anyhow::Result;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tantivy::{
    Index, IndexReader, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    query::{QueryParser, TermQuery},
    schema::{IndexRecordOption, OwnedValue},
};

use super::common::{extract_optional_i64, extract_required_str, result_to_value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub filepath: String,
    pub score: f32,
    #[serde(skip_serializing)]
    pub content_preview: String,
    #[serde(skip_serializing)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOutput {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub elapsed_us: u64,
}

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
}

impl SearchIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Self { index, reader })
    }

    pub fn indexed_filepaths(&self) -> Result<HashSet<String>> {
        self.collect_field_terms("filepath")
    }

    fn collect_field_terms(&self, field_name: &str) -> Result<HashSet<String>> {
        let searcher = self.reader.searcher();
        let field = self.index.schema().get_field(field_name)?;

        let mut ids = HashSet::new();
        for segment_reader in searcher.segment_readers() {
            let inverted_index = segment_reader.inverted_index(field)?;
            let mut terms = inverted_index.terms().stream()?;
            while terms.advance() {
                if let Ok(s) = std::str::from_utf8(terms.key()) {
                    if !s.is_empty() {
                        ids.insert(s.to_string());
                    }
                }
            }
        }
        Ok(ids)
    }

    fn get_str(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
        match doc.get_first(field) {
            Some(OwnedValue::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }

    fn doc_to_result(&self, doc: &TantivyDocument, score: f32) -> SearchResult {
        let schema = self.index.schema();
        let content = Self::get_str(doc, schema.get_field("content").unwrap());
        let preview = content.chars().take(2000).collect();
        SearchResult {
            filepath: Self::get_str(doc, schema.get_field("filepath").unwrap()),
            score,
            content_preview: preview,
            content,
        }
    }

    /// Retrieve a single document by filepath.
    pub fn get_document(&self, filepath: &str) -> Result<SearchResult> {
        let searcher = self.reader.searcher();
        let schema = self.index.schema();

        let field = schema.get_field("filepath")?;
        let term = tantivy::Term::from_field_text(field, filepath);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let top_docs = searcher.search(&query, &TopDocs::with_limit(1))?;
        if let Some((score, addr)) = top_docs.into_iter().next() {
            let doc: TantivyDocument = searcher.doc(addr)?;
            return Ok(self.doc_to_result(&doc, score));
        }

        anyhow::bail!("document not found: {}", filepath)
    }

    /// Full-corpus BM25 search.
    pub fn search_raw(&self, query_str: &str, top_k: usize) -> Result<SearchOutput> {
        self.search_filtered(query_str, top_k)
    }

    /// BM25 full-text search.
    pub fn search_filtered(&self, query_str: &str, top_k: usize) -> Result<SearchOutput> {
        let start = Instant::now();
        let searcher = self.reader.searcher();

        let schema = self.index.schema();
        let content_f = schema.get_field("content").unwrap();
        let qp = QueryParser::for_index(&self.index, vec![content_f]);
        let (text_query, _) = qp.parse_query_lenient(query_str);

        let top_docs = searcher.search(&text_query, &TopDocs::with_limit(top_k))?;

        let mut results: Vec<SearchResult> = Vec::new();
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr)?;
            results.push(self.doc_to_result(&doc, score));
        }

        let elapsed_us = start.elapsed().as_micros() as u64;
        Ok(SearchOutput {
            query: query_str.into(),
            results,
            elapsed_us,
        })
    }

    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
        let output = self.search_raw(query, top_k)?;
        Ok(output.results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_serialization_excludes_content() {
        let result = SearchResult {
            filepath: "foo/bar.pdf".to_string(),
            score: 0.95,
            content_preview: "this is a preview".to_string(),
            content: "this is the full document content".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("filepath").is_some());
        assert!(json.get("score").is_some());
        assert!(
            json.get("content").is_none(),
            "content must not be serialized"
        );
        assert!(
            json.get("content_preview").is_none(),
            "content_preview must not be serialized"
        );
    }
}

pub fn build_search_document_tool(index: Arc<SearchIndex>, top_k_max: i64) -> ToolRuntime {
    let desc = ToolDescBuilder::new("search_document")
            .description(
                "Search for relevant documents using BM25 full-text search. \
                 Returns top results ranked by relevance with filepath and score. \
                 Use the returned filepath with find_in_document or open_document for detailed content."
            )
            .parameters(json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "BM25 search query" },
                    "top_k": { "type": "integer", "description": "Number of results to return (default 3)" }
                },
                "required": ["query"]
            }))
            .build();

    let idx = index.clone();
    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let idx = idx.clone();
        Box::pin(async move {
            let query = match extract_required_str(&args, "query") {
                Ok(q) => q,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let top_k = extract_optional_i64(&args, "top_k")
                .unwrap_or(3)
                .clamp(1, top_k_max) as usize;

            match idx.search_filtered(&query, top_k) {
                Ok(output) => result_to_value(&output),
                Err(e) => json!({ "error": e.to_string() }).into(),
            }
        })
    });

    ToolRuntime::new(desc, f)
}
