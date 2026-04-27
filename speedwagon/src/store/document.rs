use std::fmt;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub title: String,
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
            "{{ id: {}, title: {}, len: {}, content: {} }}",
            self.id, self.title, self.len, content
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FindResult {
    pub id: String,
    pub next_cursor: Option<usize>,
    /// `Some("all"|"half"|"any")` when bare-word fallback was applied to
    /// produce these matches; `None` otherwise (no relaxation, or the query
    /// used explicit operators). Lets the caller (typically an LLM) know
    /// the match set was loosened from a strict AND-of-keywords intent.
    pub relaxation: Option<&'static str>,
    pub matches: Vec<FindMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindMatch {
    pub keyword: String,
    pub start: usize,
    pub end: usize,
    pub context: String,
}

/// Search a document for occurrences of `pattern`. Matching is case-insensitive
/// and line-oriented: at most one match is reported per matching line.
///
/// Query syntax (subset of structured query syntax):
/// - `term`              bare word
/// - `"phrase"`          exact phrase (single literal)
/// - `+term` / `-term`   required / excluded
/// - `term1 AND term2`   conjunction (operators are case-sensitive)
/// - `term1 OR term2`    disjunction
/// - `NOT term`          negation (same as `-term`)
/// - `(group)`           grouping
/// - `/regex/`           regex literal (case-insensitive)
///
/// When the input is purely bare words (no operators at all), a progressive
/// fallback is applied:
/// 1. require ALL keywords on the same line; if no hits,
/// 2. require at least HALF; if still no hits,
/// 3. require ANY single keyword.
///
/// Any explicit operator opts out of fallback so caller intent is preserved.
///
/// Internally we work in line/column space (matching is line-oriented and
/// the fallback threshold is computed per line); only byte offsets are
/// surfaced in the returned `FindMatch`.
pub fn find_in_document(
    id: &str,
    content: &str,
    pattern: &str,
    cursor: usize,
    k: usize,
    context_bytes: usize,
) -> FindResult {
    let empty_result = || FindResult {
        id: id.to_string(),
        next_cursor: None,
        relaxation: None,
        matches: vec![],
    };

    if pattern.trim().is_empty() {
        return empty_result();
    }

    let parsed = parse_query(pattern);
    let Some(expr) = parsed.expr else {
        return empty_result();
    };

    let line_spans = compute_line_spans(content);
    let lines: Vec<&str> = line_spans.iter().map(|&(s, e)| &content[s..e]).collect();
    let lines_lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();

    let mut compiler = Compiler::default();
    compiler.compile(&expr);

    let (matched, relaxation) = if let Some(keywords) = parsed.bare_keywords.as_ref() {
        select_with_fallback(keywords, &compiler, &lines, &lines_lower)
    } else {
        let m: Vec<usize> = (0..lines.len())
            .filter(|&i| eval_expr(&expr, &compiler, lines[i], &lines_lower[i]))
            .collect();
        (m, None)
    };

    let occurrences =
        collect_occurrences(&expr, &compiler, &line_spans, &lines, &lines_lower, &matched);

    let total_found = occurrences.len();
    let mut matches: Vec<FindMatch> = Vec::new();
    let mut output_chars: usize = 0;
    for (i, occ) in occurrences.iter().enumerate() {
        if i < cursor {
            continue;
        }
        if matches.len() >= k || output_chars >= MAX_OUTPUT_CHARS {
            break;
        }
        let ctx_start = floor_char_boundary(content, occ.start_byte.saturating_sub(context_bytes));
        let ctx_end = ceil_char_boundary(
            content,
            (occ.end_byte + context_bytes).min(content.len()),
        );
        let context = content[ctx_start..ctx_end].to_string();
        output_chars += context.len();
        matches.push(FindMatch {
            keyword: occ.keyword.clone(),
            start: occ.start_byte,
            end: occ.end_byte,
            context,
        });
    }

    let next_cursor = (cursor + matches.len() < total_found).then_some(cursor + matches.len());
    FindResult {
        id: id.to_string(),
        next_cursor,
        relaxation,
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

// ---------------------------------------------------------------------------
// Query AST + parser
// ---------------------------------------------------------------------------

/// Identifier for a unique term in the parsed expression. Used to key into
/// the compiled regex / lowercased-literal pools so we don't recompile or
/// reallocate during line scanning.
type TermId = usize;

#[derive(Debug, Clone)]
enum Expr {
    /// Plain literal substring (case-insensitive). `display` keeps the
    /// original-case spelling for surfacing in `FindMatch.keyword`.
    Literal { id: TermId, display: String },
    /// Regex pattern (case-insensitive).
    Regex { id: TermId, display: String },
    /// All children must match.
    And(Vec<Expr>),
    /// At least one child must match.
    Or(Vec<Expr>),
    /// Child must NOT match.
    Not(Box<Expr>),
}

#[derive(Debug)]
struct ParsedQuery {
    expr: Option<Expr>,
    /// `Some(keywords)` when the original input was just bare words and
    /// fallback should apply. Each keyword is lowercased.
    bare_keywords: Option<Vec<String>>,
}

fn parse_query(input: &str) -> ParsedQuery {
    let trimmed = input.trim();
    let bare_keywords = extract_bare_keywords(trimmed);

    let tokens = tokenize(trimmed);
    let mut p = TokenParser::new(&tokens);
    let mut next_id: TermId = 0;
    let expr = parse_or(&mut p, &mut next_id);

    ParsedQuery {
        expr,
        bare_keywords,
    }
}

fn extract_bare_keywords(input: &str) -> Option<Vec<String>> {
    if input.is_empty() {
        return None;
    }
    for ch in input.chars() {
        if matches!(ch, '"' | '+' | '-' | '(' | ')' | '/') {
            return None;
        }
    }
    let mut words = Vec::new();
    for w in input.split_whitespace() {
        if matches!(w, "AND" | "OR" | "NOT") {
            return None;
        }
        words.push(w.to_lowercase());
    }
    if words.is_empty() {
        None
    } else {
        Some(words)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    Plus,
    Minus,
    And,
    Or,
    Not,
    Phrase(String),
    Regex(String),
    Word(String),
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '"' => {
                let (lit, consumed) = read_quoted(&input[i..]);
                out.push(Token::Phrase(lit));
                i += consumed;
            }
            '/' => {
                let rest = &input[i + 1..];
                if let Some(end) = find_unescaped(rest, '/') {
                    let pat = unescape_slash(&rest[..end]);
                    out.push(Token::Regex(pat));
                    i += 1 + end + 1;
                } else {
                    let (word, consumed) = read_word(&input[i..]);
                    if word.is_empty() {
                        i += 1;
                    } else {
                        out.push(classify_word(&word));
                        i += consumed;
                    }
                }
            }
            _ => {
                let (word, consumed) = read_word(&input[i..]);
                if word.is_empty() {
                    i += 1;
                } else {
                    out.push(classify_word(&word));
                    i += consumed;
                }
            }
        }
    }
    out
}

fn classify_word(w: &str) -> Token {
    match w {
        "AND" => Token::And,
        "OR" => Token::Or,
        "NOT" => Token::Not,
        _ => Token::Word(w.to_string()),
    }
}

fn read_quoted(s: &str) -> (String, usize) {
    debug_assert!(s.starts_with('"'));
    let bytes = s.as_bytes();
    let mut buf = String::new();
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '\\' && i + 1 < bytes.len() {
            buf.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if c == '"' {
            return (buf, i + 1);
        }
        buf.push(c);
        i += 1;
    }
    (buf, bytes.len())
}

fn read_word(s: &str) -> (String, usize) {
    let mut buf = String::new();
    for (idx, ch) in s.char_indices() {
        if ch.is_whitespace() || matches!(ch, '(' | ')' | '"') {
            return (buf, idx);
        }
        buf.push(ch);
    }
    (buf, s.len())
}

fn find_unescaped(s: &str, target: char) -> Option<usize> {
    let mut prev_backslash = false;
    for (i, ch) in s.char_indices() {
        if prev_backslash {
            prev_backslash = false;
            continue;
        }
        if ch == '\\' {
            prev_backslash = true;
            continue;
        }
        if ch == target {
            return Some(i);
        }
    }
    None
}

fn unescape_slash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_backslash = false;
    for ch in s.chars() {
        if prev_backslash {
            // `\/` becomes `/`; other escapes pass through so regex meta
            // sequences (\b, \d, …) keep working.
            if ch != '/' {
                out.push('\\');
            }
            out.push(ch);
            prev_backslash = false;
            continue;
        }
        if ch == '\\' {
            prev_backslash = true;
            continue;
        }
        out.push(ch);
    }
    if prev_backslash {
        out.push('\\');
    }
    out
}

struct TokenParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> TokenParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
}

/// Recursive-descent: or → and → unary → atom.
/// Adjacent terms without an explicit connector default to AND.
fn parse_or(p: &mut TokenParser, next_id: &mut TermId) -> Option<Expr> {
    let mut alts: Vec<Expr> = Vec::new();
    if let Some(e) = parse_and(p, next_id) {
        alts.push(e);
    }
    while let Some(Token::Or) = p.peek() {
        p.bump();
        if let Some(e) = parse_and(p, next_id) {
            alts.push(e);
        }
    }
    match alts.len() {
        0 => None,
        1 => Some(alts.pop().unwrap()),
        _ => Some(Expr::Or(alts)),
    }
}

fn parse_and(p: &mut TokenParser, next_id: &mut TermId) -> Option<Expr> {
    let mut conj: Vec<Expr> = Vec::new();
    loop {
        match p.peek() {
            None | Some(Token::RParen) | Some(Token::Or) => break,
            Some(Token::And) => {
                p.bump();
                continue;
            }
            _ => {}
        }
        if let Some(e) = parse_unary(p, next_id) {
            conj.push(e);
        } else {
            // Unparseable token; drop to make progress.
            p.bump();
        }
    }
    match conj.len() {
        0 => None,
        1 => Some(conj.pop().unwrap()),
        _ => Some(Expr::And(conj)),
    }
}

fn parse_unary(p: &mut TokenParser, next_id: &mut TermId) -> Option<Expr> {
    match p.peek()? {
        Token::Plus => {
            p.bump();
            // `+term` is a no-op semantically (the term must already match in
            // its surrounding AND group). We keep it for readability — it
            // signals "this is the required anchor" and disables fallback.
            parse_atom(p, next_id)
        }
        Token::Minus | Token::Not => {
            p.bump();
            let inner = parse_atom(p, next_id)?;
            Some(Expr::Not(Box::new(inner)))
        }
        _ => parse_atom(p, next_id),
    }
}

fn parse_atom(p: &mut TokenParser, next_id: &mut TermId) -> Option<Expr> {
    match p.bump()? {
        Token::LParen => {
            let inner = parse_or(p, next_id);
            if let Some(Token::RParen) = p.peek() {
                p.bump();
            }
            inner
        }
        Token::Phrase(s) => Some(Expr::Literal {
            id: take_id(next_id),
            display: s,
        }),
        Token::Regex(s) => Some(Expr::Regex {
            id: take_id(next_id),
            display: s,
        }),
        Token::Word(s) => Some(Expr::Literal {
            id: take_id(next_id),
            display: s,
        }),
        Token::RParen | Token::And | Token::Or | Token::Plus | Token::Minus | Token::Not => None,
    }
}

fn take_id(next_id: &mut TermId) -> TermId {
    let id = *next_id;
    *next_id += 1;
    id
}

// ---------------------------------------------------------------------------
// Compilation + per-line evaluation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Compiler {
    /// One slot per `TermId`. Literals get a lowercased copy in `lits` and
    /// `None` in `regexes`; regex terms get `None` in `lits` and a compiled
    /// `Some(Regex)` in `regexes`. A regex compile failure leaves both slots
    /// effectively absent so the term never matches.
    lits: Vec<Option<String>>,
    regexes: Vec<Option<Regex>>,
}

impl Compiler {
    fn compile(&mut self, expr: &Expr) {
        match expr {
            Expr::Literal { id, display } => {
                self.ensure(*id);
                self.lits[*id] = Some(display.to_lowercase());
            }
            Expr::Regex { id, display } => {
                self.ensure(*id);
                self.regexes[*id] = Regex::new(&format!("(?i){display}")).ok();
            }
            Expr::And(cs) | Expr::Or(cs) => {
                for c in cs {
                    self.compile(c);
                }
            }
            Expr::Not(inner) => self.compile(inner),
        }
    }

    fn ensure(&mut self, id: TermId) {
        while self.lits.len() <= id {
            self.lits.push(None);
            self.regexes.push(None);
        }
    }

    fn term_matches(&self, id: TermId, line: &str, line_lower: &str) -> bool {
        if let Some(lit) = self.lits.get(id).and_then(|x| x.as_ref())
            && !lit.is_empty()
            && line_lower.contains(lit.as_str())
        {
            return true;
        }
        if let Some(re) = self.regexes.get(id).and_then(|x| x.as_ref()) {
            return re.is_match(line);
        }
        false
    }
}

fn eval_expr(expr: &Expr, c: &Compiler, line: &str, line_lower: &str) -> bool {
    match expr {
        Expr::Literal { id, .. } | Expr::Regex { id, .. } => c.term_matches(*id, line, line_lower),
        Expr::And(cs) => {
            // Pure-negative AND (e.g. `-foo`) has no positive evidence and
            // shouldn't surface every non-`foo` line as a hit. Require at
            // least one non-Not child to evaluate true.
            let has_positive = cs.iter().any(|e| !matches!(e, Expr::Not(_)));
            if !has_positive {
                return false;
            }
            cs.iter().all(|e| eval_expr(e, c, line, line_lower))
        }
        Expr::Or(cs) => cs.iter().any(|e| eval_expr(e, c, line, line_lower)),
        Expr::Not(inner) => !eval_expr(inner, c, line, line_lower),
    }
}

// ---------------------------------------------------------------------------
// Bare-word fallback
// ---------------------------------------------------------------------------

fn select_with_fallback(
    keywords: &[String],
    compiler: &Compiler,
    lines: &[&str],
    lines_lower: &[String],
) -> (Vec<usize>, Option<&'static str>) {
    let n = keywords.len();
    if n == 0 {
        return (vec![], None);
    }
    // Each keyword's TermId equals its index in the parsed expression's
    // left-to-right enumeration. parse_query emits a flat And of literals
    // for bare-word inputs, so id == position in `keywords`.
    let count_hits = |i: usize| -> usize {
        (0..n)
            .filter(|&kid| compiler.term_matches(kid, lines[i], &lines_lower[i]))
            .count()
    };
    let collect = |threshold: usize| -> Vec<usize> {
        (0..lines.len())
            .filter(|&i| count_hits(i) >= threshold)
            .collect()
    };

    let threshold_all = n;
    let threshold_half = n.div_ceil(2);

    let all = collect(threshold_all);
    if !all.is_empty() {
        let tag = if threshold_all > 1 { Some("all") } else { None };
        return (all, tag);
    }
    if threshold_half < threshold_all {
        let half = collect(threshold_half);
        if !half.is_empty() {
            return (half, Some("half"));
        }
    }
    let any = collect(1);
    if any.is_empty() {
        (vec![], None)
    } else {
        (any, Some("any"))
    }
}

// ---------------------------------------------------------------------------
// Line spans + occurrence collection
// ---------------------------------------------------------------------------

fn compute_line_spans(content: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (i, &b) in content.as_bytes().iter().enumerate() {
        if b == b'\n' {
            spans.push((start, i));
            start = i + 1;
        }
    }
    spans.push((start, content.len()));
    spans
}

#[derive(Debug, Clone)]
struct Occurrence {
    keyword: String,
    start_byte: usize,
    end_byte: usize,
}

fn collect_occurrences(
    expr: &Expr,
    compiler: &Compiler,
    line_spans: &[(usize, usize)],
    lines: &[&str],
    lines_lower: &[String],
    matched_line_indices: &[usize],
) -> Vec<Occurrence> {
    let mut out: Vec<Occurrence> = Vec::new();
    for &line_idx in matched_line_indices {
        let line = lines[line_idx];
        let line_lower = &lines_lower[line_idx];
        let (line_byte_start, _) = line_spans[line_idx];

        // Pick the earliest positive-evidence term hit on this line — that's
        // the anchor we report. Negative (Not) branches contribute no anchor.
        if let Some((kw, col_s, col_e)) = first_anchor(expr, compiler, line, line_lower) {
            out.push(Occurrence {
                keyword: kw,
                start_byte: line_byte_start + col_s,
                end_byte: line_byte_start + col_e,
            });
        }
    }
    out.sort_by_key(|o| o.start_byte);
    out
}

fn first_anchor(
    expr: &Expr,
    c: &Compiler,
    line: &str,
    line_lower: &str,
) -> Option<(String, usize, usize)> {
    match expr {
        Expr::Literal { id, display } => {
            let lit = c.lits.get(*id).and_then(|x| x.as_ref())?;
            if lit.is_empty() {
                return None;
            }
            let pos = line_lower.find(lit.as_str())?;
            Some((display.clone(), pos, pos + lit.len()))
        }
        Expr::Regex { id, display } => {
            let re = c.regexes.get(*id).and_then(|x| x.as_ref())?;
            let m = re.find(line)?;
            Some((display.clone(), m.start(), m.end()))
        }
        Expr::And(cs) | Expr::Or(cs) => {
            let mut best: Option<(String, usize, usize)> = None;
            for child in cs {
                if let Some(cand) = first_anchor(child, c, line, line_lower) {
                    best = Some(match best {
                        Some(prev) if prev.1 <= cand.1 => prev,
                        _ => cand,
                    });
                }
            }
            best
        }
        Expr::Not(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_K: usize = 20;
    const DEFAULT_CTX: usize = 240;

    fn find(content: &str, pattern: &str) -> FindResult {
        find_in_document("test", content, pattern, 0, DEFAULT_K, DEFAULT_CTX)
    }

    #[test]
    fn test_basic_match() {
        let content = "hello world\nfoo bar revenue baz\nnothing here";
        let result = find(content, "revenue");
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("revenue"));
        assert_eq!(result.relaxation, None);
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn test_basic_byte_offsets_round_trip() {
        let content = "hello world\nfoo bar revenue baz\nnothing here";
        let result = find(content, "revenue");
        let m = &result.matches[0];
        assert_eq!(&content[m.start..m.end], "revenue");
    }

    #[test]
    fn test_regex_via_slashes() {
        let content = "total revenue was high\nnet sales increased\nnothing here";
        let result = find(content, "/revenue|sales/");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_or_operator() {
        let content = "total revenue was high\nnet sales increased\nnothing here";
        let result = find(content, "revenue OR sales");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_phrase_quoted() {
        let content = "cost of goods sold was up\nrevenue cost goods\nNothing";
        let result = find(content, "\"cost of goods sold\"");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(
            &content[result.matches[0].start..result.matches[0].end],
            "cost of goods sold"
        );
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_required_and_excluded() {
        let content = "revenue and tax line\nrevenue without that word\ntax only";
        let result = find(content, "+revenue -tax");
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("revenue without that word"));
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_explicit_and_no_fallback() {
        // `alpha AND beta` must require both — no relaxation.
        let content = "alpha alone\nbeta alone\nnothing\nalpha and beta together";
        let result = find(content, "alpha AND beta");
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("together"));
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_basic_fallback_all() {
        let content = "alpha and beta together\nalpha alone\nbeta alone";
        let result = find(content, "alpha beta");
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.relaxation, Some("all"));
    }

    #[test]
    fn test_basic_fallback_half() {
        let content = "alpha beta gamma\nalpha beta\ngamma delta\nepsilon";
        let result = find(content, "alpha beta gamma delta");
        assert_eq!(result.matches.len(), 3);
        assert_eq!(result.relaxation, Some("half"));
    }

    #[test]
    fn test_basic_fallback_any_via_three_keywords() {
        // 3 keywords; half = 2. No line has 2; falls through to any.
        let content = "alpha solo\nbeta solo\ngamma solo";
        let result = find(content, "alpha beta gamma");
        assert_eq!(result.matches.len(), 3);
        assert_eq!(result.relaxation, Some("any"));
    }

    #[test]
    fn test_basic_two_keywords_half_equals_any() {
        // 2 keywords, half = 1 = any. We report `half` (it's the level we
        // actually used, and it's more specific than `any`).
        let content = "alpha alone\nbeta only\nnothing";
        let result = find(content, "alpha beta");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.relaxation, Some("half"));
    }

    #[test]
    fn test_single_keyword_no_relaxation_tag() {
        let content = "one keyword\ntwo keyword\nthree keyword";
        let result = find(content, "keyword");
        assert_eq!(result.matches.len(), 3);
        assert_eq!(result.relaxation, None);
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
    fn test_case_insensitive_via_regex() {
        let content = "Total Revenue was HIGH\nnet Sales increased\nNOTHING here";
        let result = find(content, "/revenue|sales/");
        assert_eq!(result.matches.len(), 2);
        assert!(result.matches[0].context.contains("Revenue"));
        assert!(result.matches[1].context.contains("Sales"));
    }

    #[test]
    fn test_case_insensitive_literal() {
        let content = "Apple apple APPLE aPpLe";
        let result = find(content, "apple");
        // All on same line — line-based matcher collapses to one match.
        assert_eq!(result.matches.len(), 1);
    }

    #[test]
    fn test_one_match_per_line() {
        let content = "revenue revenue revenue here";
        let result = find(content, "revenue");
        assert_eq!(result.matches.len(), 1);
        let m = &result.matches[0];
        assert_eq!(&content[m.start..m.end], "revenue");
    }

    #[test]
    fn test_unicode_byte_boundaries() {
        let content = "한국어 revenue 데이터\n다른 줄입니다";
        let result = find_in_document("test", content, "revenue", 0, 5, 4);
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("revenue"));
    }

    #[test]
    fn test_grouping_with_or_and_required() {
        let content = "revenue 2024 report\nsales 2024 report\nrevenue 2023 report\n2024 alone";
        let result = find(content, "(revenue OR sales) +2024");
        assert_eq!(result.matches.len(), 2);
        assert_eq!(result.relaxation, None);
    }

    #[test]
    fn test_not_excludes_lines() {
        let content = "alpha keep\nbeta drop\nalpha drop\nalpha";
        let result = find(content, "alpha NOT drop");
        assert_eq!(result.matches.len(), 2);
        for m in &result.matches {
            assert!(!m.context.contains("drop") || m.context.contains("alpha keep"));
        }
    }

    #[test]
    fn test_phrase_with_special_chars() {
        // Phrase content shouldn't be interpreted as regex.
        let content = "line with cost.of.goods\nrevenue cost ofF goods\nplain";
        let result = find(content, "\"cost.of.goods\"");
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].context.contains("cost.of.goods"));
    }

    #[test]
    fn test_empty_pattern() {
        let result = find("anything", "   ");
        assert!(result.matches.is_empty());
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn test_byte_offsets_in_content() {
        let content = "hello\nrevenue line\ntail";
        let result = find(content, "revenue");
        let m = &result.matches[0];
        assert_eq!(&content[m.start..m.end], "revenue");
        // byte offset of "revenue" in the content
        assert_eq!(m.start, 6);
    }
}
