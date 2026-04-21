use std::{fmt, fs, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tantivy::{
    Index, IndexWriter, TantivyDocument, Term,
    collector::TopDocs,
    query::{AllQuery, TermQuery},
    schema::{IndexRecordOption, OwnedValue, STORED, STRING, TEXT},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub content: Option<String>,
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const TRIM: usize = 50;
        let content = match &self.content {
            None => "<no content>".to_string(),
            Some(c) => {
                let raw = if c.len() > TRIM {
                    format!("{}...", &c[..TRIM])
                } else {
                    c.clone()
                };
                raw.replace('\n', "\\n")
            }
        };
        write!(
            f,
            "{{ id: {}, title: {}, content: {} }}",
            self.id, self.title, content
        )
    }
}

pub fn open_or_create(index_dir: &Path) -> Result<Index> {
    let index = if index_dir.exists() {
        Index::open_in_dir(index_dir)?
    } else {
        fs::create_dir_all(index_dir)?;

        let mut sb = tantivy::schema::Schema::builder();
        sb.add_text_field("id", STRING | STORED);
        sb.add_text_field("title", TEXT | STORED);
        sb.add_text_field("content", TEXT | STORED);
        let schema = sb.build();

        Index::create_in_dir(index_dir, schema)?
    };

    // let reader = index
    //     .reader_builder()
    //     .reload_policy(ReloadPolicy::OnCommitWithDelay)
    //     .try_into()?;

    Ok(index)
}

pub fn add_document(index: &Index, id: &str, title: &str, content: &str) -> Result<Document> {
    let schema = index.schema();
    let id_f = schema.get_field("id")?;
    let title_f = schema.get_field("title")?;
    let content_f = schema.get_field("content")?;

    let mut writer: IndexWriter = index.writer(128_000_000)?;
    let mut doc = TantivyDocument::default();
    doc.add_text(id_f, id);
    doc.add_text(title_f, title);
    doc.add_text(content_f, content);
    writer.add_document(doc)?;
    writer.commit()?;

    Ok(Document {
        id: id.to_string(),
        title: title.to_string(),
        content: Some(content.to_string()),
    })
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
        Document {
            id: id.to_string(),
            title: get_str(&tdoc, title_f),
            content: Some(get_str(&tdoc, content_f)),
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
            Document {
                id: get_str(&doc, id_f),
                title: get_str(&doc, title_f),
                content: if include_content {
                    Some(get_str(&doc, content_f))
                } else {
                    None
                },
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
