//! Post-hoc verify reporter (Stage A).
//!
//! Inspects an agent's history *after* a `run` finishes and reports
//! deterministic signals that suggest something went wrong. No LLM calls.
//! No agent behaviour change — this stage only flags issues; the agent
//! has already responded by the time the report is built.
//!
//! Stage B (a future PR) is expected to add an ailoy-side hook so the
//! same signal logic can run between turns and actually intercept tool
//! results before the next LLM call. The signal functions in this module
//! are written to be reusable from that callback unchanged.
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

use ailoy::{
    datatype::Value,
    message::{Message, Part, Role},
};
use serde::Serialize;

/// Knobs for the verify pass. Only the loop threshold is exposed today;
/// future signals (e.g. a `bash_stderr_dominant` ratio) will land here too,
/// keeping the policy surface in one place.
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
    /// or `null`). The follow-up Stage B will be able to substitute a
    /// fallback hint here; Stage A only reports.
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
        if self.issues.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for issue in &self.issues {
            out.push_str("- ");
            out.push_str(&format_issue(issue));
            out.push('\n');
        }
        out
    }
}

fn format_issue(issue: &Issue) -> String {
    match issue {
        Issue::EmptyResult { tool } => format!("empty result from `{tool}`"),
        Issue::LoopDetected {
            tool,
            count,
            threshold,
        } => format!("`{tool}` invoked {count} times with identical args (threshold {threshold})"),
        Issue::UnverifiedCitation { citation } => {
            format!("citation not found in tool log: `{citation}`")
        }
        Issue::BashFailure { reason } => match reason {
            BashFailureReason::NonZeroExit { exit_code } => {
                format!("bash exited with code {exit_code}")
            }
            BashFailureReason::TimedOut => "bash timed out".to_string(),
            BashFailureReason::ValidationError => "bash received invalid arguments".to_string(),
        },
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
        .filter(|c| !haystack.contains(c.as_str()))
        .map(|citation| Issue::UnverifiedCitation { citation })
        .collect()
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

/// Pick out citation candidates from assistant prose. Three patterns,
/// chosen for low false-positive rate at the cost of missing weirder
/// citation forms (those can be added once we see them in real runs):
///
/// - HTTP(S) URLs
/// - Absolute or `./` / `~/` file paths
/// - ISO-8601 timestamps (the demo task's main citation form: experiment
///   logs carry per-event timestamps that should round-trip to the plot)
fn extract_citations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(scan_pattern(text, is_url_char, "http://"));
    out.extend(scan_pattern(text, is_url_char, "https://"));
    out.extend(scan_paths(text));
    out.extend(scan_iso_timestamps(text));
    // Dedup while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.clone()));
    out
}

fn scan_pattern(text: &str, allowed: fn(char) -> bool, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(found) = text[i..].find(prefix) {
        let start = i + found;
        let mut end = start + prefix.len();
        while end < bytes.len() {
            let c = text[end..].chars().next().unwrap_or(' ');
            if allowed(c) {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        // Trim a trailing punctuation we don't want to be part of the citation.
        let candidate = trim_trailing_punct(&text[start..end]);
        if candidate.len() > prefix.len() {
            out.push(candidate.to_string());
        }
        i = end.max(start + prefix.len());
    }
    out
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && c != '<' && c != '>' && c != '"' && c != '\'' && c != ',' && c != ')'
}

fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | ']' | '?' | '!'))
}

fn scan_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let token = trim_trailing_punct(token).trim_matches(|c| matches!(c, '`' | '"' | '\''));
        let starts_path = token.starts_with('/') || token.starts_with("./") || token.starts_with("~/");
        if starts_path && token.len() > 1 && token.contains(|c: char| c == '/' || c == '.') {
            out.push(token.to_string());
        }
    }
    out
}

/// ISO-8601 / RFC 3339 style timestamps. We accept the common subsets:
///
/// - `YYYY-MM-DD`
/// - `YYYY-MM-DDTHH:MM:SS`
/// - `YYYY-MM-DDTHH:MM:SSZ` or with `+HH:MM` / `-HH:MM` offsets
///
/// Conservative on purpose: anything fancier (fractional seconds, week
/// numbers, etc.) is left for follow-ups when we see them in real runs.
fn scan_iso_timestamps(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 10 <= n {
        if is_date_at(bytes, i) {
            // Greedy: try to extend with T HH:MM:SS and optional zone.
            let mut end = i + 10;
            if end < n && bytes[end] == b'T' && end + 9 <= n && is_time_at(bytes, end + 1) {
                end += 9;
                if end < n && bytes[end] == b'Z' {
                    end += 1;
                } else if end + 6 <= n
                    && (bytes[end] == b'+' || bytes[end] == b'-')
                    && bytes[end + 3] == b':'
                    && is_digit(bytes[end + 1])
                    && is_digit(bytes[end + 2])
                    && is_digit(bytes[end + 4])
                    && is_digit(bytes[end + 5])
                {
                    end += 6;
                }
            }
            out.push(std::str::from_utf8(&bytes[i..end]).unwrap_or("").to_string());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn is_digit(b: u8) -> bool {
    b.is_ascii_digit()
}

fn is_date_at(b: &[u8], i: usize) -> bool {
    // YYYY-MM-DD
    is_digit(b[i])
        && is_digit(b[i + 1])
        && is_digit(b[i + 2])
        && is_digit(b[i + 3])
        && b[i + 4] == b'-'
        && is_digit(b[i + 5])
        && is_digit(b[i + 6])
        && b[i + 7] == b'-'
        && is_digit(b[i + 8])
        && is_digit(b[i + 9])
}

fn is_time_at(b: &[u8], i: usize) -> bool {
    // HH:MM:SS  (8 chars), called via is_time_at(b, end+1) where end+1+8 ≤ n
    i + 8 <= b.len()
        && is_digit(b[i])
        && is_digit(b[i + 1])
        && b[i + 2] == b':'
        && is_digit(b[i + 3])
        && is_digit(b[i + 4])
        && b[i + 5] == b':'
        && is_digit(b[i + 6])
        && is_digit(b[i + 7])
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
