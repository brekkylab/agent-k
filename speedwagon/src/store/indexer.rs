use std::{fs, path::Path};

use anyhow::Result;
use tantivy::{
    Index, IndexWriter, TantivyDocument, Term,
    collector::TopDocs,
    query::{AllQuery, BooleanQuery, Occur, Query, TermQuery},
    schema::{IndexRecordOption, OwnedValue, STORED, STRING, Schema, TEXT},
};

use super::Document;

fn build_schema() -> Schema {
    let mut sb = Schema::builder();
    sb.add_text_field("id", STRING | STORED);
    sb.add_text_field("title", TEXT | STORED);
    sb.add_text_field("purpose", TEXT | STORED);
    sb.add_text_field("content", TEXT | STORED);
    sb.build()
}

fn schema_matches(existing: &Schema, expected: &Schema) -> bool {
    for (_, expected_entry) in expected.fields() {
        let name = expected_entry.name();
        match existing.get_field(name) {
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
    true
}

pub fn open_or_create(index_dir: &Path) -> Result<Index> {
    let expected = build_schema();

    if index_dir.exists() {
        match Index::open_in_dir(index_dir) {
            Ok(index) if schema_matches(&index.schema(), &expected) => return Ok(index),
            Ok(_) => {
                log::info!(
                    "speedwagon index schema is outdated at {index_dir:?}; removing and rebuilding (corpus is preserved)"
                );
                fs::remove_dir_all(index_dir)?;
            }
            Err(e) => {
                log::warn!(
                    "failed to open existing index at {index_dir:?} ({e}); removing and rebuilding"
                );
                fs::remove_dir_all(index_dir)?;
            }
        }
    }

    fs::create_dir_all(index_dir)?;
    let index = Index::create_in_dir(index_dir, expected)?;
    Ok(index)
}

pub fn add_document(
    index: &Index,
    id: &str,
    title: &str,
    purpose: &str,
    content: &str,
) -> Result<Document> {
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let purpose_f = schema.get_field("purpose")?;
    let content_f = schema.get_field("content")?;

    let mut writer: IndexWriter = index.writer(128_000_000)?;
    let mut doc = TantivyDocument::default();
    doc.add_text(id_f, id);
    doc.add_text(title_f, title);
    doc.add_text(purpose_f, purpose);
    doc.add_text(content_f, content);
    writer.add_document(doc)?;
    writer.commit()?;

    Ok(Document {
        id: id.to_string(),
        title: title.to_string(),
        purpose: purpose.to_string(),
        len: content.len(),
        content: Some(content.to_string()),
    })
}

pub fn add_documents(
    index: &Index,
    docs: &[(&str, &str, &str, &str)],
) -> Result<Vec<Document>> {
    if docs.is_empty() {
        return Ok(vec![]);
    }

    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let purpose_f = schema.get_field("purpose")?;
    let content_f = schema.get_field("content")?;

    let mut writer: IndexWriter = index.writer(128_000_000)?;
    let mut result = Vec::with_capacity(docs.len());

    for &(id, title, purpose, content) in docs {
        let mut doc = TantivyDocument::default();
        doc.add_text(id_f, id);
        doc.add_text(title_f, title);
        doc.add_text(purpose_f, purpose);
        doc.add_text(content_f, content);
        writer.add_document(doc)?;
        result.push(Document {
            id: id.to_string(),
            title: title.to_string(),
            purpose: purpose.to_string(),
            len: content.len(),
            content: Some(content.to_string()),
        });
    }

    writer.commit()?;
    Ok(result)
}

pub fn document_exists(index: &Index, id: &str) -> Result<bool> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let term = Term::from_field_text(id_f, id);
    let query = TermQuery::new(term, IndexRecordOption::Basic);
    let top_docs = searcher.search(&query, &TopDocs::with_limit(1))?;
    Ok(!top_docs.is_empty())
}

pub fn delete_document(index: &Index, id: &str) -> Result<Option<Document>> {
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let purpose_f = schema.get_field("purpose")?;
    let content_f = schema.get_field("content")?;

    let doc = {
        let reader = index.reader()?;
        let searcher = reader.searcher();
        let term = Term::from_field_text(id_f, id);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        let top_docs = searcher.search(&query, &TopDocs::with_limit(1))?;

        if top_docs.is_empty() {
            return Ok(None);
        }

        let (_, addr) = top_docs.into_iter().next().unwrap();
        let tdoc: TantivyDocument = searcher.doc(addr)?;
        let content = get_str(&tdoc, content_f);
        Document {
            id: id.to_string(),
            title: get_str(&tdoc, title_f),
            purpose: get_str(&tdoc, purpose_f),
            len: content.len(),
            content: Some(content),
        }
    };

    let mut writer: IndexWriter = index.writer(128_000_000)?;
    writer.delete_term(Term::from_field_text(id_f, id));
    writer.commit()?;

    Ok(Some(doc))
}

pub fn list_documents(index: &Index, include_content: bool) -> Result<Vec<Document>> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let id_f = schema.get_field("id").unwrap();
    let title_f = schema.get_field("title").unwrap();
    let purpose_f = schema.get_field("purpose").unwrap();
    let content_f = schema.get_field("content").unwrap();

    let total = searcher.num_docs() as usize;
    if total == 0 {
        return Ok(vec![]);
    }

    let top_docs = searcher.search(&AllQuery, &TopDocs::with_limit(total))?;
    Ok(top_docs
        .into_iter()
        .map(|(_, addr)| {
            let doc: TantivyDocument = searcher.doc(addr).unwrap();
            let content = get_str(&doc, content_f);
            Document {
                id: get_str(&doc, id_f),
                title: get_str(&doc, title_f),
                purpose: get_str(&doc, purpose_f),
                len: content.len(),
                content: if include_content { Some(content) } else { None },
            }
        })
        .collect())
}

pub fn num_documents(index: &Index) -> Result<u64> {
    let reader = index.reader()?;
    Ok(reader.searcher().num_docs())
}

pub fn get_document(index: &Index, id: &str) -> Result<Option<Document>> {
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let purpose_f = schema.get_field("purpose")?;
    let content_f = schema.get_field("content")?;

    let term = Term::from_field_text(id_f, id);
    let query = TermQuery::new(term, IndexRecordOption::Basic);
    let top_docs = searcher.search(&query, &TopDocs::with_limit(1))?;

    Ok(top_docs.into_iter().next().map(|(_, addr)| {
        let doc: TantivyDocument = searcher.doc(addr).unwrap();
        let content = get_str(&doc, content_f);
        Document {
            id: id.to_string(),
            title: get_str(&doc, title_f),
            purpose: get_str(&doc, purpose_f),
            len: content.len(),
            content: Some(content),
        }
    }))
}

pub fn get_documents(index: &Index, ids: &[&str]) -> Result<Vec<Document>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let reader = index.reader()?;
    let searcher = reader.searcher();
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let purpose_f = schema.get_field("purpose")?;
    let content_f = schema.get_field("content")?;

    let subqueries: Vec<(Occur, Box<dyn Query>)> = ids
        .iter()
        .map(|id| {
            let term = Term::from_field_text(id_f, id);
            let q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::Basic));
            (Occur::Should, q)
        })
        .collect();

    let query = BooleanQuery::new(subqueries);
    let top_docs = searcher.search(&query, &TopDocs::with_limit(ids.len()))?;

    Ok(top_docs
        .into_iter()
        .map(|(_, addr)| {
            let doc: TantivyDocument = searcher.doc(addr).unwrap();
            let id = get_str(&doc, id_f);
            let content = get_str(&doc, content_f);
            Document {
                id,
                title: get_str(&doc, title_f),
                purpose: get_str(&doc, purpose_f),
                len: content.len(),
                content: Some(content),
            }
        })
        .collect())
}

fn get_str(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    match doc.get_first(field) {
        Some(OwnedValue::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn schema_includes_purpose() {
        let schema = build_schema();
        assert!(schema.get_field("id").is_ok());
        assert!(schema.get_field("title").is_ok());
        assert!(schema.get_field("purpose").is_ok());
        assert!(schema.get_field("content").is_ok());
    }

    #[test]
    fn open_or_create_creates_new_index() {
        let tmp = TempDir::new().unwrap();
        let index = open_or_create(&tmp.path().join("idx")).unwrap();
        assert!(index.schema().get_field("purpose").is_ok());
    }

    #[test]
    fn open_or_create_reopens_matching_index() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("idx");
        let _ = open_or_create(&path).unwrap();
        // second call should reopen, not recreate
        let index = open_or_create(&path).unwrap();
        assert!(index.schema().get_field("purpose").is_ok());
    }

    #[test]
    fn open_or_create_rebuilds_outdated_schema() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("idx");
        fs::create_dir_all(&path).unwrap();

        // build legacy schema (no purpose)
        let mut sb = Schema::builder();
        sb.add_text_field("id", STRING | STORED);
        sb.add_text_field("title", TEXT | STORED);
        sb.add_text_field("content", TEXT | STORED);
        let legacy = sb.build();
        Index::create_in_dir(&path, legacy).unwrap();

        let index = open_or_create(&path).unwrap();
        assert!(
            index.schema().get_field("purpose").is_ok(),
            "expected rebuilt schema to contain purpose field"
        );
    }

    #[test]
    fn add_document_round_trips_purpose() {
        let tmp = TempDir::new().unwrap();
        let index = open_or_create(&tmp.path().join("idx")).unwrap();

        let purpose = "3M Company FY2018 10-K Annual Report — revenue, safety industrial, healthcare";
        add_document(&index, "doc1", "3M 10-K", purpose, "body text").unwrap();

        let doc = get_document(&index, "doc1").unwrap().unwrap();
        assert_eq!(doc.purpose, purpose);
        assert_eq!(doc.title, "3M 10-K");
    }

    #[test]
    fn add_documents_batch_round_trips_purpose() {
        let tmp = TempDir::new().unwrap();
        let index = open_or_create(&tmp.path().join("idx")).unwrap();

        let docs = vec![
            ("a", "Title A", "purpose A", "content A"),
            ("b", "Title B", "purpose B", "content B"),
        ];
        add_documents(&index, &docs).unwrap();

        let result = get_documents(&index, &["a", "b"]).unwrap();
        let purposes: std::collections::HashSet<_> =
            result.iter().map(|d| d.purpose.clone()).collect();
        assert!(purposes.contains("purpose A"));
        assert!(purposes.contains("purpose B"));
    }

    #[test]
    fn list_documents_includes_purpose() {
        let tmp = TempDir::new().unwrap();
        let index = open_or_create(&tmp.path().join("idx")).unwrap();
        add_document(&index, "x", "T", "P-purpose", "body").unwrap();

        let docs = list_documents(&index, false).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].purpose, "P-purpose");
        assert!(docs[0].content.is_none());
    }

    #[test]
    fn delete_document_returns_purpose() {
        let tmp = TempDir::new().unwrap();
        let index = open_or_create(&tmp.path().join("idx")).unwrap();
        add_document(&index, "id1", "Title", "the-purpose", "body").unwrap();

        let removed = delete_document(&index, "id1").unwrap().unwrap();
        assert_eq!(removed.purpose, "the-purpose");
    }
}
