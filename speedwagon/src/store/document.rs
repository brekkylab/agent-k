use std::fmt;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub content: Option<String>,
    pub len: usize,
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
            "{{ id: {}, title: {}, purpose: {}, len: {}, content: {} }}",
            self.id, self.title, self.purpose, self.len, content
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FindResult {
    pub id: String,
    pub next_cursor: Option<usize>,
    pub matches: Vec<FindMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindMatch {
    pub keyword: String,
    pub start: usize,
    pub end: usize,
    pub context: String,
}

pub fn find_in_document(
    id: &str,
    content: &str,
    pattern: &str,
    cursor: usize,
    k: usize,
    context_bytes: usize,
) -> FindResult {
    if pattern.is_empty() {
        return FindResult {
            id: id.to_string(),
            next_cursor: None,
            matches: vec![],
        };
    }

    // Always match case-insensitively; (?i) can be overridden inline via (?-i) if needed
    let Ok(re) = Regex::new(&format!("(?i){pattern}")) else {
        return FindResult {
            id: id.to_string(),
            next_cursor: None,
            matches: vec![],
        };
    };

    let all_matches: Vec<(usize, usize)> = re
        .find_iter(content)
        .map(|m| (m.start(), m.end()))
        .collect();
    let total_found = all_matches.len();

    let mut matches: Vec<FindMatch> = Vec::new();
    let mut output_chars: usize = 0;
    for (i, &(match_start, match_end)) in all_matches.iter().enumerate() {
        if i < cursor {
            continue;
        }
        if matches.len() >= k || output_chars >= MAX_OUTPUT_CHARS {
            break;
        }

        let ctx_start = floor_char_boundary(content, match_start.saturating_sub(context_bytes));
        let ctx_end = ceil_char_boundary(content, (match_end + context_bytes).min(content.len()));
        let context = content[ctx_start..ctx_end].to_string();
        output_chars += context.len();

        matches.push(FindMatch {
            keyword: pattern.to_string(),
            start: match_start,
            end: match_end,
            context,
        });
    }

    let next_cursor = (cursor + matches.len() < total_found).then_some(cursor + matches.len());
    FindResult {
        id: id.to_string(),
        next_cursor,
        matches,
    }
}

pub fn read_in_document(content: &str, offset: usize, len: usize) -> String {
    let start = floor_char_boundary(content, offset.min(content.len()));
    let end = ceil_char_boundary(content, (start + len).min(content.len()));
    content[start..end].to_string()
}

const MAX_OUTPUT_CHARS: usize = 8000;

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn ceil_char_boundary(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    while p < s.len() && !s.is_char_boundary(p) {
        p += 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_K: usize = 20;
    const DEFAULT_CTX: usize = 240;

    #[test]
    fn test_basic_match() {
        let content = "hello world\nfoo bar revenue baz\nnothing here";
        let result = find_in_document("test", content, "revenue", 0, DEFAULT_K, DEFAULT_CTX);

        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("revenue"));
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn test_regex_match() {
        let content = "total revenue was high\nnet sales increased\nnothing here";
        let result = find_in_document("test", content, "revenue|sales", 0, DEFAULT_K, DEFAULT_CTX);

        assert_eq!(result.matches.len(), 2);
    }

    #[test]
    fn test_cursor_pagination() {
        let k = 3;
        let content: String = (0..20)
            .map(|i| format!("line {} keyword here", i))
            .collect::<Vec<_>>()
            .join("\n");

        let r1 = find_in_document("test", &content, "keyword", 0, k, DEFAULT_CTX);
        assert_eq!(r1.matches.len(), 3);
        assert_eq!(r1.next_cursor, Some(3));

        let r2 = find_in_document(
            "test",
            &content,
            "keyword",
            r1.next_cursor.unwrap(),
            k,
            DEFAULT_CTX,
        );
        assert_eq!(r2.matches.len(), 3);
        assert_ne!(r1.matches[0].context, r2.matches[0].context);
    }

    #[test]
    fn test_cursor_exhausted() {
        let content = "one keyword\ntwo keyword\nthree keyword";
        let result = find_in_document("test", content, "keyword", 0, 100, DEFAULT_CTX);

        assert!(result.next_cursor.is_none());
        assert_eq!(result.matches.len(), 3);
    }

    #[test]
    fn test_case_insensitive() {
        let content = "Total Revenue was HIGH\nnet Sales increased\nNOTHING here";
        let result = find_in_document("test", content, "revenue|sales", 0, DEFAULT_K, DEFAULT_CTX);

        assert_eq!(result.matches.len(), 2);
        assert!(result.matches[0].context.contains("Revenue"));
        assert!(result.matches[1].context.contains("Sales"));
    }

    #[test]
    fn test_case_insensitive_mixed() {
        let content = "Apple apple APPLE aPpLe";
        let result = find_in_document("test", content, "apple", 0, DEFAULT_K, DEFAULT_CTX);

        assert_eq!(result.matches.len(), 4);
    }
}
