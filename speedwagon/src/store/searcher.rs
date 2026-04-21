use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tantivy::{Index, TantivyDocument, collector::TopDocs, schema::OwnedValue};

use crate::store::indexer::Document;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub document: Document,
    pub score: f32,
    #[serde(skip_serializing, default)]
    pub content_preview: String,
}

#[derive(Debug, Serialize)]
pub struct SearchPage {
    pub query: String,
    pub page: u32,
    pub page_size: u32,
    pub results: Vec<SearchResult>,
    pub has_more: bool,
    pub elapsed_us: u64,
}

pub fn search_page(
    index: &Index,
    query_str: &str,
    page: u32,
    page_size: u32,
) -> Result<SearchPage> {
    let start = Instant::now();
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let content_f = schema.get_field("content").unwrap();

    let qp = tantivy::query::QueryParser::for_index(index, vec![content_f]);
    let (query, _) = qp.parse_query_lenient(query_str);

    let offset = (page * page_size) as usize;
    let fetch = page_size as usize + 1;
    let top_docs = searcher.search(&query, &TopDocs::with_limit(fetch).and_offset(offset))?;

    let has_more = top_docs.len() > page_size as usize;
    let results: Vec<SearchResult> = top_docs
        .into_iter()
        .take(page_size as usize)
        .map(|(score, addr)| {
            let doc: TantivyDocument = searcher.doc(addr).unwrap();
            doc_to_result(&schema, &doc, score)
        })
        .collect();

    Ok(SearchPage {
        query: query_str.into(),
        page,
        page_size,
        results,
        has_more,
        elapsed_us: start.elapsed().as_micros() as u64,
    })
}

fn get_str(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    match doc.get_first(field) {
        Some(OwnedValue::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn doc_to_result(
    schema: &tantivy::schema::Schema,
    doc: &TantivyDocument,
    score: f32,
) -> SearchResult {
    let content = get_str(doc, schema.get_field("content").unwrap());
    let preview = content.chars().take(2000).collect();
    SearchResult {
        document: Document {
            id: get_str(doc, schema.get_field("id").unwrap()),
            title: get_str(doc, schema.get_field("title").unwrap()),
            content: Some(content),
        },
        score,
        content_preview: preview,
    }
}
