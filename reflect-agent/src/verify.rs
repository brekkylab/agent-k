//! Post-hoc verify reporter.
//!
//! Inspects an agent's history *after* a `run` finishes and reports
//! deterministic signals that suggest something went wrong. No LLM calls.
//! No agent behaviour change — the gate only flags issues; the agent
//! has already responded by the time the report is built. The signal
//! functions are pure over `&[Message]`, so they remain reusable from
//! any cut of history.
//!
//! ## Signals
//!
//! | Signal | What it detects |
//! |---|---|
//! | `EmptyResult` | Tool returned an empty object / array / string. |
//! | `LoopDetected` | Same `(tool_name, args)` invoked ≥ `loop_threshold` times. |
//! | `UnverifiedCitation` | Final assistant text cites a source (URL, file path, ISO timestamp) that never appears in the tool log. |
//! | `BashFailure` | The bash tool reported `exit_code != 0`, `timed_out == true`, or a validation error. |
//!
//! See the PR body for the per-signal inspirations from leaked agent
//! system prompts (Cowork, Devin, Claude Code).

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use ailoy::{
    datatype::Value,
    message::{Message, Part, Role},
};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use regex::Regex;
use serde::Serialize;

/// Knobs for the verify pass.
#[derive(Clone, Debug)]
pub struct VerifyConfig {
    /// Number of identical `(tool_name, args)` calls that triggers
    /// [`Issue::LoopDetected`]. Defaults to 3, mirroring Devin's CI rule
    /// ("ask the user for help if CI does not pass after the third attempt").
    pub loop_threshold: usize,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self { loop_threshold: 3 }
    }
}

/// One issue detected by [`verify_run`].
///
/// Each variant is a deterministic finding (no LLM judgement). Variants are
/// `Serialize` so they can be logged as JSON or rendered to stderr.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Issue {
    /// A tool call returned an empty value (empty object / array / string,
    /// or `null`).
    EmptyResult { tool: String },

    /// The same `(tool_name, args)` was invoked at least `count` times in
    /// this run, exceeding [`VerifyConfig::loop_threshold`].
    LoopDetected {
        tool: String,
        count: usize,
        threshold: usize,
    },

    /// The final assistant text cited a source (URL, file path, or ISO
    /// timestamp) that never appears anywhere in the tool log. The cited
    /// substring may have been hallucinated.
    UnverifiedCitation { citation: String },

    /// The `bash` tool returned a structured failure: a non-zero exit
    /// code, a timeout, or a validation error from missing arguments.
    BashFailure { reason: BashFailureReason },
}

/// Why the bash tool failed. Mirrors the failure modes encoded in ailoy's
/// `bash` tool result (exit_code, timed_out, phase=="validation").
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BashFailureReason {
    NonZeroExit { exit_code: i64 },
    TimedOut,
    ValidationError,
}

/// Aggregate report for a single agent run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct VerifyReport {
    pub issues: Vec<Issue>,
}

impl VerifyReport {
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// Render the report as a short multi-line string suitable for stderr.
    /// Returns an empty string when no issues were found.
    pub fn format(&self) -> String {
        self.issues.iter().map(|i| format!("- {i}\n")).collect()
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::EmptyResult { tool } => write!(f, "empty result from `{tool}`"),
            Issue::LoopDetected {
                tool,
                count,
                threshold,
            } => write!(
                f,
                "`{tool}` invoked {count} times with identical args (threshold {threshold})"
            ),
            Issue::UnverifiedCitation { citation } => {
                write!(f, "citation not found in tool log: `{citation}`")
            }
            Issue::BashFailure { reason } => match reason {
                BashFailureReason::NonZeroExit { exit_code } => {
                    write!(f, "bash exited with code {exit_code}")
                }
                BashFailureReason::TimedOut => write!(f, "bash timed out"),
                BashFailureReason::ValidationError => write!(f, "bash received invalid arguments"),
            },
        }
    }
}

/// Run all verify checks on the slice of history produced by one agent
/// turn (everything appended since the user message that opened the run).
///
/// `history_slice` should be `&agent.get_history()[history_before..]` where
/// `history_before` was captured before `agent.run(...)` was awaited.
pub fn verify_run(history_slice: &[Message], config: &VerifyConfig) -> VerifyReport {
    let tool_log = collect_tool_log(history_slice);
    let mut issues = Vec::new();

    issues.extend(check_empty_results(&tool_log));
    issues.extend(check_loops(&tool_log, config.loop_threshold));
    issues.extend(check_bash_failures(&tool_log));
    issues.extend(check_citations(history_slice, &tool_log));

    VerifyReport { issues }
}

// ── tool log extraction ───────────────────────────────────────────────────

/// One resolved tool invocation: the assistant's call paired with the
/// tool's response. Identified by `call_id` from the assistant message's
/// tool_calls part and the matching `Role::Tool` message's `id`.
#[derive(Clone, Debug)]
struct ToolCall {
    name: String,
    args: Value,
    result: Option<Value>,
}

fn collect_tool_log(history: &[Message]) -> Vec<ToolCall> {
    // First pass: assistant tool_calls produce ToolCall entries keyed by call_id.
    let mut by_id: HashMap<String, ToolCall> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for msg in history {
        if msg.role != Role::Assistant {
            continue;
        }
        let Some(calls) = &msg.tool_calls else {
            continue;
        };
        for part in calls {
            let Some((call_id, name, args)) = part.as_function() else {
                continue;
            };
            order.push(call_id.to_string());
            by_id.insert(
                call_id.to_string(),
                ToolCall {
                    name: name.to_string(),
                    args: args.clone(),
                    result: None,
                },
            );
        }
    }
    // Second pass: Role::Tool messages' first value Part attaches as result
    // to the entry whose call_id matches the message's `id`.
    for msg in history {
        if msg.role != Role::Tool {
            continue;
        }
        let Some(call_id) = &msg.id else { continue };
        let Some(value) = msg.contents.iter().find_map(Part::as_value) else {
            continue;
        };
        if let Some(entry) = by_id.get_mut(call_id) {
            entry.result = Some(value.clone());
        }
    }
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

// ── signal: empty result ──────────────────────────────────────────────────

fn check_empty_results(tool_log: &[ToolCall]) -> Vec<Issue> {
    tool_log
        .iter()
        .filter(|c| c.result.as_ref().is_some_and(is_empty_value))
        .map(|c| Issue::EmptyResult {
            tool: c.name.clone(),
        })
        .collect()
}

/// "Empty" tool result: nothing meaningful for the LLM to read.
///
/// Direct cases — `null`, an empty string, an empty array, or an empty
/// object — are all empty.
///
/// Compound case — a non-empty object is *also* empty when all of its
/// string / array / nested-object fields are themselves empty and at
/// least one such field exists. Numeric / boolean fields (e.g. an
/// `exit_code: 0` or `timed_out: false`) are ignored: they're metadata,
/// not content. This catches the common shape where a tool always
/// returns a fixed schema (like ailoy's bash `{stdout, stderr, exit_code,
/// timed_out}`) but produced no actual output.
fn is_empty_value(v: &Value) -> bool {
    if v.is_null() {
        return true;
    }
    if let Some(s) = v.as_str() {
        return s.trim().is_empty();
    }
    if let Some(arr) = v.as_array() {
        return arr.is_empty();
    }
    if let Some(obj) = v.as_object() {
        if obj.is_empty() {
            return true;
        }
        let mut saw_content_field = false;
        for (_, field) in obj {
            // Skip numeric / boolean metadata; we only care about content.
            if field.as_str().is_none()
                && field.as_array().is_none()
                && !field.is_object()
                && !field.is_null()
            {
                continue;
            }
            saw_content_field = true;
            if !is_empty_value(field) {
                return false;
            }
        }
        return saw_content_field;
    }
    false
}

// ── signal: loop guard ────────────────────────────────────────────────────

fn check_loops(tool_log: &[ToolCall], threshold: usize) -> Vec<Issue> {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for c in tool_log {
        // Serialize args to a canonical string for grouping. Two identical
        // call payloads serialize to identical JSON; differences in field
        // order are stable thanks to ailoy's Value being object-ordered.
        let args_key = serde_json::to_string(&c.args).unwrap_or_default();
        *counts.entry((c.name.clone(), args_key)).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|((tool, _), count)| Issue::LoopDetected {
            tool,
            count,
            threshold,
        })
        .collect()
}

// ── signal: bash failure ──────────────────────────────────────────────────

fn check_bash_failures(tool_log: &[ToolCall]) -> Vec<Issue> {
    tool_log
        .iter()
        .filter(|c| c.name == "bash")
        .filter_map(|c| {
            let result = c.result.as_ref()?;
            bash_failure_reason(result).map(|reason| Issue::BashFailure { reason })
        })
        .collect()
}

fn bash_failure_reason(result: &Value) -> Option<BashFailureReason> {
    // ailoy's bash tool result shape:
    //   { "stdout": str, "stderr": str, "exit_code": i64, "timed_out": bool }
    // or the validation variant:
    //   { "stdout": "", "stderr": "...", "exit_code": -1, "phase": "validation" }
    if result
        .pointer("/phase")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p == "validation")
    {
        return Some(BashFailureReason::ValidationError);
    }
    if result
        .pointer("/timed_out")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Some(BashFailureReason::TimedOut);
    }
    if let Some(code) = result.pointer("/exit_code").and_then(|v| v.as_integer()) {
        if code != 0 {
            return Some(BashFailureReason::NonZeroExit { exit_code: code });
        }
    }
    None
}

// ── signal: unverified citation ───────────────────────────────────────────

fn check_citations(history: &[Message], tool_log: &[ToolCall]) -> Vec<Issue> {
    let final_text = match last_assistant_text(history) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let citations = extract_citations(&final_text);
    if citations.is_empty() {
        return Vec::new();
    }
    let haystack = build_tool_log_haystack(tool_log);
    citations
        .into_iter()
        .filter(|c| !appears_in_haystack(c, &haystack))
        .map(|citation| Issue::UnverifiedCitation { citation })
        .collect()
}

/// Is `citation` justified by `haystack` under any of the tolerated
/// representational variants? Tolerated variants are intentionally
/// narrow — each is a paraphrase the same canonical artifact tends to
/// take in real LLM output, not an open-ended fuzzy match.
///
/// Order matters: cheaper / stronger checks first, the broader ones
/// (e.g. timestamp prefix) last.
fn appears_in_haystack(citation: &str, haystack: &str) -> bool {
    // 1. Exact substring — what PR #53 originally shipped.
    if haystack.contains(citation) {
        return true;
    }

    // 2. URL: tolerate trailing-slash and http/https swaps.
    if is_url_citation(citation) {
        for alt in url_variants(citation) {
            if haystack.contains(&alt) {
                return true;
            }
        }
    }

    // 3. File path: strip leading `./` / `~/` and trailing `/`,
    //    so `/tmp/log.txt` and `./log.txt` line up if the haystack
    //    contains one shape but the assistant cited the other.
    if is_path_citation(citation) {
        let cit_norm = normalize_path(citation);
        if !cit_norm.is_empty() && haystack.contains(&cit_norm) {
            return true;
        }
    }

    // 4. ISO-style timestamp: walk back to the date prefix. The agent
    //    might cite a precise instant (`2024-01-15T10:30:00`) when the
    //    tool log only echoed the date. Walking the *citation* (not the
    //    haystack) keeps this asymmetric: vague tool output can satisfy
    //    a precise quote, but a precise tool output never licenses a
    //    vague quote.
    if is_iso_timestamp(citation) {
        for prefix in timestamp_prefixes(citation) {
            if !prefix.is_empty() && haystack.contains(prefix) {
                return true;
            }
        }
    }

    false
}

fn is_url_citation(c: &str) -> bool {
    c.starts_with("http://") || c.starts_with("https://")
}

fn is_path_citation(c: &str) -> bool {
    c.starts_with('/') || c.starts_with("./") || c.starts_with("~/")
}

fn is_iso_timestamp(c: &str) -> bool {
    // chrono-validated calendar instant — same gate as extraction, so a
    // citation that survived `extract_citations` always answers true here.
    is_valid_iso_timestamp(c)
}

/// Generate URL variants we accept as equivalent. Kept tiny on purpose:
/// trailing slash on/off, and (only when http/https swap is otherwise a
/// no-op string) the other scheme. Anything broader is a false-positive
/// risk.
fn url_variants(c: &str) -> Vec<String> {
    let mut out = Vec::new();
    let trimmed = c.trim_end_matches('/');
    if trimmed != c {
        out.push(trimmed.to_string());
    } else {
        out.push(format!("{c}/"));
    }
    if let Some(rest) = c.strip_prefix("https://") {
        out.push(format!("http://{rest}"));
    } else if let Some(rest) = c.strip_prefix("http://") {
        out.push(format!("https://{rest}"));
    }
    out
}

fn normalize_path(c: &str) -> &str {
    let trimmed = c
        .trim_start_matches("./")
        .trim_start_matches("~/")
        .trim_end_matches('/');
    trimmed
}

/// Successive shorter prefixes of a timestamp citation, longest first.
/// `2024-01-15T10:30:00Z` → `2024-01-15T10:30:00`, `2024-01-15`. The exact
/// citation (full string) is already covered by step 1 in
/// [`appears_in_haystack`], so we don't repeat it here.
///
/// Each candidate is gated through chrono so we never hand back a half-
/// token like `2024-01-15T1` even if the cut happens to land mid-field.
fn timestamp_prefixes(c: &str) -> Vec<&str> {
    let mut out = Vec::new();
    // Strip any zone suffix first — Z or ±HH:MM — then keep the result
    // only if chrono accepts it as a real `YYYY-MM-DDTHH:MM:SS`.
    let stripped = c
        .trim_end_matches(|ch: char| ch.is_ascii_digit() || ch == ':')
        .trim_end_matches(|ch: char| ch == '+' || ch == '-' || ch == 'Z');
    if stripped.len() < c.len() && is_valid_iso_timestamp(stripped) {
        out.push(stripped);
    }
    // Date-only fallback: take the first 10 chars and ask chrono whether
    // they form a real calendar date. This drops `2024-13-15` style
    // shape-only strings even if step 1 above let them through.
    if c.len() > 10 {
        let date_prefix = &c[..10];
        if is_valid_iso_timestamp(date_prefix) && !out.iter().any(|p: &&str| *p == date_prefix) {
            out.push(date_prefix);
        }
    }
    out
}

fn last_assistant_text(history: &[Message]) -> Option<String> {
    let msg = history
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.contents.is_empty())?;
    let mut text = String::new();
    for part in &msg.contents {
        if let Some(t) = part.as_text() {
            text.push_str(t);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

/// Concatenate every tool call's args + result into one string we can do
/// substring lookups against. Cheap and good enough for citation grep:
/// any URL / path / timestamp the agent learned from a tool will appear
/// somewhere here verbatim.
fn build_tool_log_haystack(tool_log: &[ToolCall]) -> String {
    let mut haystack = String::new();
    for c in tool_log {
        haystack.push_str(&serde_json::to_string(&c.args).unwrap_or_default());
        haystack.push('\n');
        if let Some(result) = &c.result {
            haystack.push_str(&serde_json::to_string(result).unwrap_or_default());
            haystack.push('\n');
        }
    }
    haystack
}

/// One regex, three named alternatives — single pass over the text covers
/// every citation shape we recognise:
///
/// - `url` — HTTP(S) URLs, stopping at whitespace / quote / bracket / comma
/// - `path` — absolute or `./` / `~/` file paths, stopping at whitespace /
///   quote / comma / semicolon
/// - `ts` — ISO-8601 / RFC 3339 timestamp candidates (just the *shape*;
///   chrono validates them in [`extract_citations`])
///
/// Trailing sentence punctuation (`.`, `,`, `;`, `:`, `)`, `]`, `?`, `!`)
/// is stripped after the match so prose like "see https://foo.com." yields
/// `https://foo.com`, not the period.
static CITATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
          (?P<url>https?://[^\s<>"',\)]+)
        | (?P<path>(?:/|\./|~/)[^\s,;"'`]+)
        | (?P<ts>\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2})?)?)
        "#,
    )
    .expect("citation regex compiles")
});

/// Pick out citation candidates from assistant prose. Three patterns,
/// chosen for low false-positive rate at the cost of missing weirder
/// citation forms (those can be added once we see them in real runs):
///
/// - HTTP(S) URLs
/// - Absolute or `./` / `~/` file paths
/// - ISO-8601 timestamps (the demo task's main citation form: experiment
///   logs carry per-event timestamps that should round-trip to the plot).
///   Timestamps go through chrono so that shape-only fakes like
///   `2024-13-45` or `2024-02-30` are dropped at extraction rather than
///   leaking into the haystack-miss path as false UnverifiedCitation hits.
fn extract_citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for caps in CITATION_RE.captures_iter(text) {
        let raw = if let Some(m) = caps.name("url") {
            trim_trailing_punct(m.as_str())
        } else if let Some(m) = caps.name("path") {
            let trimmed = trim_trailing_punct(m.as_str());
            // Keep the original gate: paths must contain `/` or `.` past
            // the leading marker, so a bare "/" or "~/" doesn't qualify.
            if trimmed.len() <= 1 || !trimmed[1..].contains(['/', '.']) {
                continue;
            }
            trimmed
        } else if let Some(m) = caps.name("ts") {
            let candidate = m.as_str();
            if !is_valid_iso_timestamp(candidate) {
                continue;
            }
            candidate
        } else {
            continue;
        };
        if !raw.is_empty() && seen.insert(raw.to_string()) {
            out.push(raw.to_string());
        }
    }
    out
}

fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | ']' | '?' | '!'))
}

/// Does `s` parse as a real calendar instant? Three accepted shapes match
/// the regex's `ts` alternative — chrono checks month / day / leap-year
/// validity so that `2024-13-45` (bad month) or `2024-02-30` (no Feb 30)
/// are rejected at extraction.
fn is_valid_iso_timestamp(s: &str) -> bool {
    DateTime::parse_from_rfc3339(s).is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
        || NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ailoy::{message::ToolDescBuilder, to_value};

    // ── helpers ───────────────────────────────────────────────────────────

    fn assistant_with_call(call_id: &str, name: &str, args: Value) -> Message {
        let _ = ToolDescBuilder::new(name); // touch the builder to keep import live
        Message::new(Role::Assistant).with_tool_calls([Part::function(
            call_id.to_string(),
            name.to_string(),
            args,
        )])
    }

    fn tool_message(call_id: &str, value: Value) -> Message {
        Message::new(Role::Tool)
            .with_contents([Part::value(value)])
            .with_id(call_id)
    }

    fn assistant_text(text: &str) -> Message {
        Message::new(Role::Assistant).with_contents([Part::text(text)])
    }

    // ── tool log extraction ───────────────────────────────────────────────

    #[test]
    fn collect_tool_log_pairs_calls_and_results() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "ls"})),
            tool_message("c1", to_value!({"stdout": "a\n", "stderr": "", "exit_code": 0})),
        ];
        let log = collect_tool_log(&history);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].name, "bash");
        assert!(log[0].result.is_some());
    }

    #[test]
    fn collect_tool_log_keeps_call_order() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "first"})),
            tool_message("c1", to_value!({"exit_code": 0})),
            assistant_with_call("c2", "python_repl", to_value!({"code": "x"})),
            tool_message("c2", to_value!({"output": "y"})),
        ];
        let log = collect_tool_log(&history);
        assert_eq!(log.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["bash", "python_repl"]);
    }

    // ── empty result ──────────────────────────────────────────────────────

    #[test]
    fn empty_string_array_object_null_all_flagged() {
        for v in [Value::null(), to_value!(""), to_value!([]), to_value!({})] {
            assert!(is_empty_value(&v), "expected empty: {v:?}");
        }
    }

    #[test]
    fn whitespace_string_is_empty() {
        assert!(is_empty_value(&to_value!("   \n\t")));
    }

    #[test]
    fn nonempty_values_not_flagged() {
        for v in [
            to_value!("ok"),
            to_value!([1, 2, 3]),
            to_value!({"k": "v"}),
            to_value!(0),
            to_value!(false),
        ] {
            assert!(!is_empty_value(&v), "expected non-empty: {v:?}");
        }
    }

    // ── loop guard ────────────────────────────────────────────────────────

    #[test]
    fn loop_threshold_is_inclusive() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "ls"})),
            tool_message("c1", to_value!({"stdout": "", "exit_code": 0})),
            assistant_with_call("c2", "bash", to_value!({"cmd": "ls"})),
            tool_message("c2", to_value!({"stdout": "", "exit_code": 0})),
            assistant_with_call("c3", "bash", to_value!({"cmd": "ls"})),
            tool_message("c3", to_value!({"stdout": "", "exit_code": 0})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(i, Issue::LoopDetected { count: 3, .. })),
            "expected loop detected, got: {:?}",
            report.issues
        );
    }

    #[test]
    fn distinct_args_do_not_count_as_loop() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "ls /a"})),
            tool_message("c1", to_value!({"stdout": "x", "exit_code": 0})),
            assistant_with_call("c2", "bash", to_value!({"cmd": "ls /b"})),
            tool_message("c2", to_value!({"stdout": "y", "exit_code": 0})),
            assistant_with_call("c3", "bash", to_value!({"cmd": "ls /c"})),
            tool_message("c3", to_value!({"stdout": "z", "exit_code": 0})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(!report.issues.iter().any(|i| matches!(i, Issue::LoopDetected { .. })));
    }

    #[test]
    fn custom_loop_threshold_is_respected() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "ls"})),
            tool_message("c1", to_value!({"stdout": "x", "exit_code": 0})),
            assistant_with_call("c2", "bash", to_value!({"cmd": "ls"})),
            tool_message("c2", to_value!({"stdout": "x", "exit_code": 0})),
        ];
        let cfg = VerifyConfig { loop_threshold: 2 };
        let report = verify_run(&history, &cfg);
        assert!(report.issues.iter().any(|i| matches!(i, Issue::LoopDetected { count: 2, .. })));
    }

    // ── bash failure ──────────────────────────────────────────────────────

    #[test]
    fn bash_nonzero_exit_is_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "false"})),
            tool_message("c1", to_value!({"stdout": "", "stderr": "", "exit_code": 1, "timed_out": false})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(report.issues.iter().any(|i| matches!(
            i,
            Issue::BashFailure { reason: BashFailureReason::NonZeroExit { exit_code: 1 } }
        )));
    }

    #[test]
    fn bash_timeout_is_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "sleep 999"})),
            tool_message("c1", to_value!({"stdout": "", "stderr": "", "exit_code": 0, "timed_out": true})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(report.issues.iter().any(|i| matches!(
            i,
            Issue::BashFailure { reason: BashFailureReason::TimedOut }
        )));
    }

    #[test]
    fn bash_validation_error_is_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({})),
            tool_message("c1", to_value!({"stdout": "", "stderr": "missing required parameter: cmd", "exit_code": -1, "phase": "validation"})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(report.issues.iter().any(|i| matches!(
            i,
            Issue::BashFailure { reason: BashFailureReason::ValidationError }
        )));
    }

    #[test]
    fn bash_success_is_not_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "echo ok"})),
            tool_message("c1", to_value!({"stdout": "ok\n", "stderr": "", "exit_code": 0, "timed_out": false})),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(!report.issues.iter().any(|i| matches!(i, Issue::BashFailure { .. })));
    }

    // ── citation grep ─────────────────────────────────────────────────────

    #[test]
    fn citation_appearing_in_tool_log_is_not_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "grep T 2024-01-15T10:30:00 log.txt"})),
            tool_message(
                "c1",
                to_value!({"stdout": "2024-01-15T10:30:00 metric=42", "exit_code": 0}),
            ),
            assistant_text("Found 2024-01-15T10:30:00 with metric=42."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(!report.issues.iter().any(|i| matches!(i, Issue::UnverifiedCitation { .. })));
    }

    #[test]
    fn citation_missing_from_tool_log_is_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "grep T log.txt"})),
            tool_message("c1", to_value!({"stdout": "", "exit_code": 0})),
            // Hallucinated timestamp — never appeared in any tool result.
            assistant_text("The event happened at 2026-12-31T23:59:59."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(
                i,
                Issue::UnverifiedCitation { citation } if citation == "2026-12-31T23:59:59"
            )),
            "got: {:?}",
            report.issues
        );
    }

    #[test]
    fn url_citations_are_extracted() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "curl example.com"})),
            tool_message("c1", to_value!({"stdout": "ok", "exit_code": 0})),
            assistant_text("See https://example.com/foo for details."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(
                i,
                Issue::UnverifiedCitation { citation } if citation == "https://example.com/foo"
            )),
            "got: {:?}",
            report.issues
        );
    }

    #[test]
    fn file_path_citations_are_extracted() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "echo hi"})),
            tool_message("c1", to_value!({"stdout": "hi", "exit_code": 0})),
            assistant_text("Result saved to /tmp/output.csv."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(
                i,
                Issue::UnverifiedCitation { citation } if citation == "/tmp/output.csv"
            )),
            "got: {:?}",
            report.issues
        );
    }

    #[test]
    fn extract_citations_handles_trailing_punctuation() {
        let cs = extract_citations("see https://example.com, and /tmp/file.txt.");
        assert!(cs.contains(&"https://example.com".to_string()));
        assert!(cs.contains(&"/tmp/file.txt".to_string()));
    }

    #[test]
    fn extract_iso_timestamps_dates_and_datetimes() {
        let cs = extract_citations("on 2024-01-15 and at 2024-01-15T10:30:00Z");
        assert!(cs.contains(&"2024-01-15".to_string()));
        assert!(cs.contains(&"2024-01-15T10:30:00Z".to_string()));
    }

    /// Shape-only timestamps (impossible months / days) get dropped at
    /// extraction. Without chrono validation these would have fallen
    /// through to the haystack lookup and surfaced as
    /// `UnverifiedCitation`; that's a false positive — the assistant
    /// never wrote a real date.
    #[test]
    fn extract_iso_rejects_invalid_calendar_dates() {
        let cs = extract_citations("bad month 2024-13-45 and bad day 2024-02-30");
        assert!(
            !cs.iter().any(|c| c == "2024-13-45"),
            "13/45 must not be picked up as a citation, got: {cs:?}"
        );
        assert!(
            !cs.iter().any(|c| c == "2024-02-30"),
            "Feb 30 must not be picked up as a citation, got: {cs:?}"
        );
    }

    /// A bogus timestamp in the assistant's text no longer becomes an
    /// `UnverifiedCitation` — it never enters the citation set in the
    /// first place. End-to-end check that chrono validation actually
    /// suppresses the false positive on the verify report.
    #[test]
    fn invalid_calendar_date_is_not_flagged_as_citation() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "head log.txt"})),
            tool_message("c1", to_value!({"stdout": "build at 2024-01-15", "exit_code": 0})),
            assistant_text("Saw the spike at 2024-13-45T10:30:00."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. })),
            "shape-only timestamp must not surface as a citation, got: {:?}",
            report.issues
        );
    }

    // ── fuzzy match: representational variants are accepted ──────────────

    /// A precise-time citation against a date-only haystack — the agent
    /// fabricated the time-of-day, but the date itself came from a tool
    /// result. Conservative call: license the citation, since the date is
    /// real.
    #[test]
    fn timestamp_precise_citation_matches_date_only_haystack() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "head log.txt"})),
            tool_message("c1", to_value!({"stdout": "build started 2024-01-15", "exit_code": 0})),
            assistant_text("The spike happened at 2024-01-15T10:30:00."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. })),
            "date prefix should license the precise timestamp citation, got: {:?}",
            report.issues
        );
    }

    /// Inverse asymmetry: vague citation against a precise haystack —
    /// `2024-01-15` is contained in `2024-01-15T10:30:00`, so the
    /// substring step (step 1) already accepts it. Pinned for clarity.
    #[test]
    fn timestamp_date_citation_matches_precise_haystack() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "head log.txt"})),
            tool_message("c1", to_value!({"stdout": "2024-01-15T10:30:00 metric=42", "exit_code": 0})),
            assistant_text("Found a record on 2024-01-15."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. }))
        );
    }

    /// Genuinely fabricated timestamp — date itself is not in the
    /// haystack, so even prefix walking can't license it.
    #[test]
    fn timestamp_fabricated_is_still_flagged() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "head log.txt"})),
            tool_message("c1", to_value!({"stdout": "build started 2024-01-15", "exit_code": 0})),
            // 2099 is not in the haystack at any granularity.
            assistant_text("The spike happened at 2099-12-31T23:59:59."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(
                i,
                Issue::UnverifiedCitation { citation } if citation == "2099-12-31T23:59:59"
            )),
            "fabricated future timestamp must still be flagged, got: {:?}",
            report.issues
        );
    }

    /// URL trailing slash is tolerated symmetrically: cited with slash,
    /// haystack without (or vice versa).
    #[test]
    fn url_trailing_slash_is_tolerated() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "curl https://example.com/foo"})),
            tool_message("c1", to_value!({"stdout": "ok", "exit_code": 0})),
            // Cited with trailing slash, tool-log uses bare form.
            assistant_text("See https://example.com/foo/ for details."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. })),
            "trailing-slash variant should match, got: {:?}",
            report.issues
        );
    }

    /// http vs https swap is also tolerated — the agent often re-quotes
    /// the more secure form even when the tool fetched plain HTTP.
    #[test]
    fn url_http_https_swap_is_tolerated() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "curl http://example.com/foo"})),
            tool_message("c1", to_value!({"stdout": "ok", "exit_code": 0})),
            assistant_text("See https://example.com/foo for details."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. })),
            "https/http swap should match, got: {:?}",
            report.issues
        );
    }

    /// `~/foo.txt` (cited) is licensed by `foo.txt` (haystack) thanks to
    /// the leading-`~/` strip; the absolute/relative gap is *not*
    /// crossed (we don't invent path prefixes).
    #[test]
    fn path_leading_tilde_is_normalized() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "cat foo.txt"})),
            tool_message("c1", to_value!({"stdout": "wrote to foo.txt", "exit_code": 0})),
            assistant_text("Result is in ~/foo.txt."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            !report
                .issues
                .iter()
                .any(|i| matches!(i, Issue::UnverifiedCitation { .. })),
            "leading ~/ should normalize away, got: {:?}",
            report.issues
        );
    }

    /// We deliberately do NOT cross the absolute-vs-relative gap:
    /// `/tmp/foo.txt` (cited) against `tmp/foo.txt` (haystack without
    /// the leading slash) is still flagged. The path normaliser only
    /// strips leads from the citation; it never invents a prefix to
    /// make a haystack token match.
    #[test]
    fn path_absolute_vs_bare_is_not_normalized() {
        let history = [
            assistant_with_call("c1", "bash", to_value!({"cmd": "echo hi"})),
            tool_message("c1", to_value!({"stdout": "see tmp/foo.txt", "exit_code": 0})),
            assistant_text("Result is in /tmp/foo.txt."),
        ];
        let report = verify_run(&history, &VerifyConfig::default());
        assert!(
            report.issues.iter().any(|i| matches!(
                i,
                Issue::UnverifiedCitation { citation } if citation == "/tmp/foo.txt"
            )),
            "absolute path with no matching absolute haystack token must stay flagged, got: {:?}",
            report.issues
        );
    }

    // ── report aggregation ────────────────────────────────────────────────

    #[test]
    fn empty_history_yields_empty_report() {
        let report = verify_run(&[], &VerifyConfig::default());
        assert!(report.is_empty());
    }

    #[test]
    fn format_renders_each_issue_on_its_own_line() {
        let report = VerifyReport {
            issues: vec![
                Issue::EmptyResult { tool: "bash".into() },
                Issue::BashFailure {
                    reason: BashFailureReason::NonZeroExit { exit_code: 1 },
                },
            ],
        };
        let s = report.format();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains("empty result"));
        assert!(s.contains("exited with code 1"));
    }
}
