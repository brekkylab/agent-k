use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tantivy::{Index, TantivyDocument, collector::TopDocs, schema::OwnedValue};

use crate::store::Document;

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
    let title_f = schema.get_field("title").unwrap();
    let purpose_f = schema.get_field("purpose").unwrap();
    let content_f = schema.get_field("content").unwrap();

    let qp = tantivy::query::QueryParser::for_index(index, vec![title_f, purpose_f, content_f]);
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
            purpose: get_str(doc, schema.get_field("purpose").unwrap()),
            len: content.len(),
            content: Some(content),
        },
        score,
        content_preview: preview,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantivy::{
        Index, TantivyDocument,
        schema::{STORED, STRING, TEXT},
    };

    fn make_index(docs: &[(&str, &str, &str, &str)]) -> Index {
        let mut sb = tantivy::schema::Schema::builder();
        sb.add_text_field("id", STRING | STORED);
        sb.add_text_field("title", TEXT | STORED);
        sb.add_text_field("purpose", TEXT | STORED);
        sb.add_text_field("content", TEXT | STORED);
        let schema = sb.build();
        let index = Index::create_in_ram(schema);
        {
            let mut writer = index.writer(15_000_000).unwrap();
            let schema = index.schema();
            let id_f = schema.get_field("id").unwrap();
            let title_f = schema.get_field("title").unwrap();
            let purpose_f = schema.get_field("purpose").unwrap();
            let content_f = schema.get_field("content").unwrap();
            for &(id, title, purpose, content) in docs {
                let mut doc = TantivyDocument::default();
                doc.add_text(id_f, id);
                doc.add_text(title_f, title);
                doc.add_text(purpose_f, purpose);
                doc.add_text(content_f, content);
                writer.add_document(doc).unwrap();
            }
            writer.commit().unwrap();
        }
        index
    }

    #[test]
    fn test_search_finds_matching_document() {
        let index = make_index(&[
            (
                "doc1",
                "Revenue Report",
                "",
                "quarterly revenue increased by 20%",
            ),
            ("doc2", "Product Launch", "", "new product features announced"),
        ]);
        let page = search_page(&index, "revenue", 0, 10).unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].document.id, "doc1");
        assert!(!page.has_more);
    }

    #[test]
    fn test_search_no_results() {
        let index = make_index(&[("doc1", "Title", "", "some content here")]);
        let page = search_page(&index, "xyzzy", 0, 10).unwrap();
        assert!(page.results.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn test_search_has_more() {
        let index = make_index(&[
            ("a", "Alpha", "", "rust programming language"),
            ("b", "Beta", "", "rust memory safety"),
            ("c", "Gamma", "", "rust async runtime"),
        ]);
        let page = search_page(&index, "rust", 0, 2).unwrap();
        assert_eq!(page.results.len(), 2);
        assert!(page.has_more);
    }

    #[test]
    fn test_search_page_offset() {
        let index = make_index(&[
            ("a", "Alpha", "", "rust programming language"),
            ("b", "Beta", "", "rust memory safety"),
            ("c", "Gamma", "", "rust async runtime"),
        ]);
        let page0_ids: Vec<_> = search_page(&index, "rust", 0, 2)
            .unwrap()
            .results
            .into_iter()
            .map(|r| r.document.id)
            .collect();
        let page1 = search_page(&index, "rust", 1, 2).unwrap();
        assert_eq!(page1.results.len(), 1);
        assert!(!page1.has_more);
        assert!(!page0_ids.contains(&page1.results[0].document.id));
    }

    #[test]
    fn test_content_preview_capped_at_2000_chars() {
        let content = "documentation ".repeat(200); // 2800 chars
        let index = make_index(&[("doc1", "Long Doc", "", &content)]);
        let page = search_page(&index, "documentation", 0, 10).unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].content_preview.chars().count(), 2000);
        assert!(page.results[0].document.len > 2000);
    }

    #[test]
    fn test_search_matches_purpose_field() {
        let index = make_index(&[
            (
                "doc1",
                "Filing Cover",
                "3M Company FY2018 10-K Annual Report — revenue, healthcare, safety industrial",
                "boilerplate cover page text",
            ),
            (
                "doc2",
                "Other",
                "Costco Wholesale 2023 Q1 earnings — net sales, comparable sales",
                "different filler",
            ),
        ]);
        let page = search_page(&index, "healthcare", 0, 10).unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].document.id, "doc1");
    }

    #[test]
    fn test_serialization_skips_content_preview() {
        let result = SearchResult {
            document: Document {
                id: "id1".into(),
                title: "T".into(),
                purpose: "P".into(),
                content: None,
                len: 0,
            },
            score: 1.0,
            content_preview: "preview".into(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("document").is_some());
        assert!(json.get("score").is_some());
        assert!(
            json.get("content_preview").is_none(),
            "content_preview must not be serialized"
        );
    }
}
