use std::sync::Arc;

use ailoy::{ToolAsyncFunc, ToolDescBuilder, ToolRuntime, Value};
use futures::future::BoxFuture;
use regex::Regex;
use serde::Serialize;
use serde_json::json;

use super::{
    common::{extract_optional_i64, extract_required_str, result_to_value},
    search::SearchIndex,
};

#[derive(Debug, Clone, Serialize)]
pub struct FindPosition {
    pub line: usize, // 1-based
    pub col: usize,  // 1-based character offset
}

impl std::fmt::Display for FindPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FindResult {
    pub filepath: String,
    pub total_lines: usize,
    pub total_found: usize,
    pub returned: usize,
    pub next_cursor: Option<usize>,
    pub matches: Vec<FindMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindMatch {
    pub keyword: String,
    pub start: FindPosition,
    pub end: FindPosition,
    pub line_content: String,
    pub context: Vec<ContextLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextLine {
    pub line: usize, // 1-based
    pub content: String,
}

/// Grep within document content for pattern matches.
///
/// Each keyword in `query` is treated as a regex pattern (case-insensitive).
/// If a pattern fails to compile, that keyword falls back to substring matching.
///
/// Matching strategy (AND with progressive fallback):
/// 1. Try ALL keywords present in a line (AND).
/// 2. If 0 results, require at least half the keywords.
/// 3. If still 0, require any single keyword (OR).
///
/// `cursor`: pass `next_cursor` from the previous response to get the next page.
pub fn find_in_document(
    filepath: &str,
    content: &str,
    query: &str,
    cursor: usize,
    max_matches: usize,
) -> FindResult {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // If query contains '|', treat as single regex (phrase-level OR).
    // e.g. "cost of goods sold|COGS" → regex matching either phrase.
    if query.contains('|') {
        return find_regex_mode(filepath, &lines, total_lines, query, cursor, max_matches);
    }

    // Otherwise, split by whitespace into keywords (AND with progressive fallback).
    let keywords: Vec<String> = query
        .split_whitespace()
        .map(|k| k.to_lowercase())
        .filter(|k| !k.is_empty())
        .collect();

    if keywords.is_empty() {
        return FindResult {
            filepath: filepath.to_string(),
            total_lines,
            total_found: 0,
            returned: 0,
            next_cursor: None,
            matches: vec![],
        };
    }

    // Compile each keyword as regex (case-insensitive). Falls back to None on compile error.
    let patterns: Vec<Option<Regex>> = keywords
        .iter()
        .map(|k| Regex::new(&format!("(?i){}", k)).ok())
        .collect();

    let matcher = |line: &str, kw_idx: usize| -> bool {
        if let Some(ref re) = patterns[kw_idx] {
            re.is_match(line)
        } else {
            line.to_lowercase().contains(keywords[kw_idx].as_str())
        }
    };

    let threshold_all = keywords.len();
    let threshold_half = (keywords.len() + 1) / 2;

    let mut hit_lines = find_lines_with_threshold(&lines, &keywords, threshold_all, &matcher);

    if hit_lines.is_empty() && threshold_half < threshold_all {
        hit_lines = find_lines_with_threshold(&lines, &keywords, threshold_half, &matcher);
    }

    if hit_lines.is_empty() {
        hit_lines = find_lines_with_threshold(&lines, &keywords, 1, &matcher);
    }

    // Collect individual keyword occurrences with char positions
    let mut occurrences: Vec<(String, usize, usize, usize)> = Vec::new();
    for &line_idx in &hit_lines {
        let lower = lines[line_idx].to_lowercase();
        for (ki, kw) in keywords.iter().enumerate() {
            if let Some(ref re) = patterns[ki] {
                for m in re.find_iter(lines[line_idx]) {
                    occurrences.push((kw.clone(), line_idx, m.start(), m.end()));
                }
            } else {
                let mut search_from = 0;
                while let Some(pos) = lower[search_from..].find(kw.as_str()) {
                    let col_start = search_from + pos;
                    let col_end = col_start + kw.len();
                    occurrences.push((kw.clone(), line_idx, col_start, col_end));
                    search_from = col_end;
                }
            }
        }
    }

    occurrences.sort_by_key(|o| (o.1, o.2));
    occurrences.dedup_by(|a, b| a.1 == b.1 && a.0 == b.0);

    let (page, total_found) = build_match_groups_paged(&lines, &occurrences, cursor, max_matches);

    let returned = page.len();
    let next_cursor = if cursor + returned < total_found {
        Some(cursor + returned)
    } else {
        None
    };

    FindResult {
        filepath: filepath.to_string(),
        total_lines,
        total_found,
        returned,
        next_cursor,
        matches: page,
    }
}

/// Regex mode: treat entire query as a single case-insensitive regex pattern.
/// Used when query contains '|' (e.g. "cost of goods sold|COGS|cost of revenue").
fn find_regex_mode(
    filepath: &str,
    lines: &[&str],
    total_lines: usize,
    query: &str,
    cursor: usize,
    max_matches: usize,
) -> FindResult {
    let re = match Regex::new(&format!("(?i){}", query)) {
        Ok(r) => r,
        Err(_) => {
            // Fallback: try plain substring matching on each '|' alternative
            return find_pipe_substring_mode(
                filepath,
                lines,
                total_lines,
                query,
                cursor,
                max_matches,
            );
        }
    };

    let mut occurrences: Vec<(String, usize, usize, usize)> = Vec::new();
    for (line_idx, line) in lines.iter().enumerate() {
        if let Some(m) = re.find(line) {
            occurrences.push((query.to_string(), line_idx, m.start(), m.end()));
        }
    }
    occurrences.sort_by_key(|o| (o.1, o.2));

    let (page, total_found) = build_match_groups_paged(lines, &occurrences, cursor, max_matches);
    let returned = page.len();
    let next_cursor = if cursor + returned < total_found {
        Some(cursor + returned)
    } else {
        None
    };

    FindResult {
        filepath: filepath.to_string(),
        total_lines,
        total_found,
        returned,
        next_cursor,
        matches: page,
    }
}

/// Fallback for regex mode when pattern is invalid: split by '|' and substring match.
fn find_pipe_substring_mode(
    filepath: &str,
    lines: &[&str],
    total_lines: usize,
    query: &str,
    cursor: usize,
    max_matches: usize,
) -> FindResult {
    let alternatives: Vec<String> = query
        .split('|')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let mut occurrences: Vec<(String, usize, usize, usize)> = Vec::new();
    for (line_idx, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        for alt in &alternatives {
            if let Some(pos) = lower.find(alt.as_str()) {
                occurrences.push((alt.clone(), line_idx, pos, pos + alt.len()));
                break; // one match per line
            }
        }
    }
    occurrences.sort_by_key(|o| (o.1, o.2));

    let (page, total_found) = build_match_groups_paged(lines, &occurrences, cursor, max_matches);
    let returned = page.len();
    let next_cursor = if cursor + returned < total_found {
        Some(cursor + returned)
    } else {
        None
    };

    FindResult {
        filepath: filepath.to_string(),
        total_lines,
        total_found,
        returned,
        next_cursor,
        matches: page,
    }
}

fn find_lines_with_threshold(
    lines: &[&str],
    keywords: &[String],
    threshold: usize,
    matcher: &dyn Fn(&str, usize) -> bool,
) -> Vec<usize> {
    let mut matched = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let count = (0..keywords.len()).filter(|&ki| matcher(line, ki)).count();
        if count >= threshold {
            matched.push(i);
        }
    }
    matched
}

const CONTEXT_LINES: usize = 3;
const MAX_LINE_CHARS: usize = 500;
const MAX_OUTPUT_CHARS: usize = 8000;

fn truncate_line(s: &str) -> String {
    if s.len() <= MAX_LINE_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(MAX_LINE_CHARS).collect();
        format!("{}…", truncated)
    }
}

/// Build match groups with surrounding context lines.
/// Returns (page of matches, total group count).
fn build_match_groups_paged(
    lines: &[&str],
    occurrences: &[(String, usize, usize, usize)],
    offset: usize,
    limit: usize,
) -> (Vec<FindMatch>, usize) {
    if occurrences.is_empty() {
        return (vec![], 0);
    }

    // Deduplicate by line index — one match per line.
    let mut seen_lines = std::collections::HashSet::new();
    let mut unique_occs: Vec<&(String, usize, usize, usize)> = Vec::new();
    for occ in occurrences {
        if seen_lines.insert(occ.1) {
            unique_occs.push(occ);
        }
    }

    let total_found = unique_occs.len();

    let mut results: Vec<FindMatch> = Vec::new();
    let mut output_chars: usize = 0;
    for (i, occ) in unique_occs.into_iter().enumerate() {
        if i < offset {
            continue;
        }
        if results.len() >= limit || output_chars >= MAX_OUTPUT_CHARS {
            break;
        }

        let line_idx = occ.1;
        // Positions are 1-based inclusive.
        // occ = (keyword, line_idx, byte_start, byte_end) where byte_* are 0-based
        // and byte_end is exclusive (from Regex::find). So:
        //   start.col = byte_start + 1          (0-based → 1-based)
        //   end.col   = byte_end                (0-based exclusive == 1-based inclusive)
        let ctx_start = line_idx.saturating_sub(CONTEXT_LINES);
        let ctx_end = (line_idx + CONTEXT_LINES + 1).min(lines.len());
        let context: Vec<ContextLine> = (ctx_start..ctx_end)
            .filter(|&i| i != line_idx)
            .map(|i| ContextLine {
                line: i + 1,
                content: truncate_line(lines[i]),
            })
            .collect();

        let line_content = truncate_line(lines[line_idx]);
        output_chars += line_content.len();
        for ctx_line in &context {
            output_chars += ctx_line.content.len();
        }

        results.push(FindMatch {
            keyword: occ.0.clone(),
            start: FindPosition {
                line: line_idx + 1,
                col: occ.2 + 1,
            },
            end: FindPosition {
                line: line_idx + 1,
                col: occ.3,
            },
            line_content,
            context,
        });
    }

    (results, total_found)
}

pub fn build_find_in_document_tool(index: Arc<SearchIndex>, max_matches: usize) -> ToolRuntime {
    let desc = ToolDescBuilder::new("find_in_document")
            .description(
                "Search within a specific document for pattern matches. \
                 Each keyword in query is treated as a regex pattern (case-insensitive). \
                 Supports: 'revenue|net sales' (OR), '\\brevenue\\b' (word boundary), 'kill(ed|ing)?' (morphology). \
                 If a pattern is invalid, that keyword falls back to plain substring matching. \
                 Returns matching positions (line:col) with the matched line. \
                 Use open_document to read surrounding context. \
                 Use filepath from search_document results. \
                 To paginate: pass the 'next_cursor' value from the previous response as 'cursor'. \
                 If 'next_cursor' is null, there are no more results."
            )
            .parameters(json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string", "description": "File path from search results" },
                    "query": { "type": "string", "description": "Search pattern (regex supported)" },
                    "cursor": { "type": "integer", "description": "Pagination cursor from previous next_cursor" }
                },
                "required": ["filepath", "query"]
            }))
            .build();

    let idx = index.clone();
    let f: Arc<ToolAsyncFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let idx = idx.clone();
        Box::pin(async move {
            let filepath = match extract_required_str(&args, "filepath") {
                Ok(d) => d,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let query = match extract_required_str(&args, "query") {
                Ok(q) => q,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let cursor = extract_optional_i64(&args, "cursor").unwrap_or(0).max(0) as usize;

            match idx.get_document(&filepath) {
                Ok(doc) => {
                    let result =
                        find_in_document(&filepath, &doc.content, &query, cursor, max_matches);
                    result_to_value(&result)
                }
                Err(e) => json!({ "error": e.to_string() }).into(),
            }
        })
    });

    ToolRuntime::new_async(desc, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_MAX_MATCHES: usize = 20;

    #[test]
    fn test_character_positions() {
        let content = "hello world\nfoo bar revenue baz\nnothing here";
        let result = find_in_document("test", content, "revenue", 0, DEFAULT_MAX_MATCHES);

        assert_eq!(result.matches.len(), 1);
        let m = &result.matches[0];
        assert_eq!(m.start.line, 2);
        assert_eq!(m.start.col, 9);
        assert_eq!(m.end.col, 15);
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn test_regex_or_pattern() {
        let content = "total revenue was high\nnet sales increased\nnothing here";
        let result = find_in_document("test", content, "revenue|sales", 0, DEFAULT_MAX_MATCHES);

        // "revenue|sales" is one keyword, regex matches both "revenue" and "sales"
        assert_eq!(result.total_found, 2);
    }

    #[test]
    fn test_regex_fallback_on_bad_pattern() {
        let content = "hello world\nfoo (bar baz\nnothing here";
        // Unclosed paren — regex compile fails, falls back to substring
        let result = find_in_document("test", content, "(bar", 0, DEFAULT_MAX_MATCHES);

        // Substring "(bar" matches line 2
        assert_eq!(result.total_found, 1);
    }

    #[test]
    fn test_cursor_pagination() {
        let max_matches = 3;
        let content: String = (0..20)
            .map(|i| format!("line {} keyword here", i))
            .collect::<Vec<_>>()
            .join("\n");

        let r1 = find_in_document("test", &content, "keyword", 0, max_matches);
        assert_eq!(r1.returned, 3);
        assert!(r1.next_cursor.is_some());

        let r2 = find_in_document(
            "test",
            &content,
            "keyword",
            r1.next_cursor.unwrap(),
            max_matches,
        );
        assert_eq!(r2.returned, 3);
        assert_ne!(r1.matches[0].start.line, r2.matches[0].start.line);
    }

    #[test]
    fn test_cursor_exhausted() {
        let max_matches = 100;
        let content = "one keyword\ntwo keyword\nthree keyword";
        let result = find_in_document("test", content, "keyword", 0, max_matches);

        assert!(result.next_cursor.is_none());
        assert_eq!(result.total_found, result.returned);
    }
}
