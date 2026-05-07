//! LLM-driven reflect gate (Phase 2 of the reflect-agent design).
//!
//! Two modes are exposed:
//!
//! - **`Self_`** — the agent's `bash` / `python_repl` / `web_search` tool set
//!   gains a fourth tool, `verify`, and the system prompt instructs the LLM
//!   to call it before emitting the final answer. The LLM decides when to
//!   invoke the verify pass; if it doesn't, the gate is bypassed.
//!
//! - **`Forced`** — `Agent::run` is wrapped on the outside. After the agent
//!   finishes a turn, the wrapper unconditionally runs a separate LLM call
//!   on the draft answer and gets a [`ReflectVerdict`] back. On `Retry`,
//!   the wrapper re-invokes the agent with `next_query`. The retry budget
//!   is fixed at one re-attempt per user turn.
//!
//! Both modes share [`reflect_call`] — the JSON-shaped LLM critique that
//! emits a `stop` / `retry` verdict — and [`ReflectVerdict`] parsing.

use ailoy::{
    agent::AgentProvider,
    lang_model::LangModel,
    message::{Message, Part, Role},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default LLM used by the reflect call. Cheap enough that one extra call
/// per turn is acceptable; same model family as the main agent so that the
/// global provider already covers it.
pub const DEFAULT_REFLECT_MODEL: &str = "anthropic/claude-haiku-4-5-20251001";

/// Maximum number of `Retry` verdicts honoured per user turn. The second
/// `Retry` is silently coerced to `Stop`. Mirrors the budget the rest of
/// the verify-gate work uses.
pub const RETRY_BUDGET: usize = 1;

/// Which reflect strategy to apply to a turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReflectMode {
    /// No LLM-driven reflect. The deterministic [`crate::verify_run`] pass
    /// from PR #53 still runs; this mode is what the CLI defaults to.
    #[default]
    Off,
    /// `verify` tool registered on the agent + system prompt instructs the
    /// LLM to call it before final emission. LLM-self-discipline driven.
    Self_,
    /// Wrapper around `Agent::run` that unconditionally runs [`reflect_call`]
    /// on the draft answer after the turn finishes. Strict but blind to
    /// what the deterministic verify pass already noticed.
    Forced,
    /// Forced behaviour plus the deterministic verifier's findings get
    /// passed into the reflect call as hints, and a low-confidence `Stop`
    /// (below [`HYBRID_LOW_CONFIDENCE_THRESHOLD`]) is treated as a
    /// retry within the standard budget. The two layers (deterministic
    /// verify + LLM reflect) work together rather than in parallel.
    Hybrid,
}

impl ReflectMode {
    /// Lower-case CLI label.
    pub fn as_str(self) -> &'static str {
        match self {
            ReflectMode::Off => "off",
            ReflectMode::Self_ => "self",
            ReflectMode::Forced => "forced",
            ReflectMode::Hybrid => "hybrid",
        }
    }

    /// Parse the `--reflect-mode` CLI value.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "off" => Ok(ReflectMode::Off),
            "self" => Ok(ReflectMode::Self_),
            "forced" => Ok(ReflectMode::Forced),
            "hybrid" => Ok(ReflectMode::Hybrid),
            other => anyhow::bail!(
                "unknown reflect mode: {other:?} (expected off|self|forced|hybrid)"
            ),
        }
    }
}

/// Outcome of one reflect call. `Stop` means the draft can be returned to
/// the user; `Retry` carries a re-posed query for the agent to run again.
///
/// `confidence` is an `Option<f32>` in `[0.0, 1.0]`. The verifier LLM is
/// asked to emit it but is allowed to omit it — older verdicts produced
/// before the field was introduced still parse cleanly. Hybrid mode reads
/// it to decide whether a `Stop` verdict is strong enough to honour;
/// other modes treat low-confidence stops the same as high-confidence
/// ones.
#[derive(Clone, Debug, PartialEq)]
pub enum ReflectVerdict {
    Stop {
        rationale: String,
        confidence: Option<f32>,
    },
    Retry {
        rationale: String,
        next_query: String,
        confidence: Option<f32>,
    },
}

impl ReflectVerdict {
    pub fn is_retry(&self) -> bool {
        matches!(self, ReflectVerdict::Retry { .. })
    }

    pub fn rationale(&self) -> &str {
        match self {
            ReflectVerdict::Stop { rationale, .. } => rationale,
            ReflectVerdict::Retry { rationale, .. } => rationale,
        }
    }

    pub fn confidence(&self) -> Option<f32> {
        match self {
            ReflectVerdict::Stop { confidence, .. } => *confidence,
            ReflectVerdict::Retry { confidence, .. } => *confidence,
        }
    }
}

/// Threshold under which Hybrid mode treats a `Stop` verdict as "not
/// confident enough" and forces a retry (within the standard retry
/// budget). Picked at 0.7 to match the loose convention in Codex's
/// `review_prompt.md` evaluator: low-confidence assessments shouldn't
/// be allowed to silently end the turn. Other modes ignore this.
pub const HYBRID_LOW_CONFIDENCE_THRESHOLD: f32 = 0.7;

/// Wire-format expected from the reflect LLM. We keep the parsing tolerant —
/// trailing prose, mixed casing, missing `next_query` on a `Stop`
/// verdict, and missing/out-of-range `confidence` are all accepted. A
/// malformed payload coerces to `Stop` so the gate fails open rather
/// than erroring out the whole turn.
#[derive(Debug, Serialize, Deserialize)]
struct WireVerdict {
    verdict: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    next_query: String,
    /// Optional. Verifier's self-rated confidence in `[0.0, 1.0]`. Out-of
    /// -range values are rejected at parse time and become `None`.
    #[serde(default)]
    confidence: Option<f32>,
}

const REFLECT_SYSTEM_PROMPT: &str = "\
You are a verification assistant. Read the draft answer below and decide \
whether it can be sent to the user as-is.

Output a single JSON object with these fields:
  - verdict:    \"stop\" or \"retry\"
  - rationale:  one short sentence explaining the choice
  - next_query: only when verdict is \"retry\"; a re-posed task that fixes the issue
  - confidence: a number in [0.0, 1.0] expressing how sure you are of the verdict

Choose \"stop\" when the draft answers the user's question correctly and is \
self-consistent with the cited evidence. Choose \"retry\" when the draft \
contains errors, leaves the user's question unanswered, or cites sources \
that do not exist in the trajectory. Use a confidence below 0.7 only when \
you genuinely cannot tell whether the draft is right. Output JSON only — \
no prose around it.";

/// Run one reflect call. The LLM sees the draft answer plus, optionally,
/// a list of deterministic-verifier hints — typically the rendered
/// findings from a [`crate::VerifyReport`] when called from Hybrid mode.
/// Pass an empty slice in modes (Self / Forced) that don't have a verify
/// pass paired with the call.
pub async fn reflect_call(
    provider: &AgentProvider,
    model_id: &str,
    draft_answer: &str,
    verify_hints: &[String],
) -> Result<ReflectVerdict> {
    let model: LangModel = provider.models.make_runtime(model_id)?;
    let user_text = if verify_hints.is_empty() {
        format!("Draft answer to verify:\n\n{}", draft_answer)
    } else {
        let mut hints = String::new();
        for h in verify_hints {
            hints.push_str("- ");
            hints.push_str(h);
            hints.push('\n');
        }
        format!(
            "A deterministic pre-checker has already flagged the following \
             potential issues with this turn — weigh them when judging the \
             draft, but remember they can be false positives (e.g. when the \
             user explicitly asked for a failing command). Use them as \
             inputs, not verdicts.\n\nVerifier flags:\n{}\nDraft answer to \
             verify:\n\n{}",
            hints.trim_end(),
            draft_answer
        )
    };
    let messages = vec![
        Message::new(Role::System).with_contents([Part::text(REFLECT_SYSTEM_PROMPT)]),
        Message::new(Role::User).with_contents([Part::text(user_text)]),
    ];
    let output = model.run(&messages, &[]).await?;
    let raw = collect_assistant_text(&output.message);
    Ok(parse_verdict(&raw))
}

fn collect_assistant_text(msg: &Message) -> String {
    let mut out = String::new();
    for part in &msg.contents {
        if let Some(t) = part.as_text() {
            out.push_str(t);
        }
    }
    out
}

/// Pull the first `{...}` block out of the raw LLM response and parse it.
/// On any failure we fall back to `Stop` with a synthetic rationale —
/// keeping the gate fail-open is preferable to erroring an entire user
/// turn just because the verifier produced loose JSON.
fn parse_verdict(raw: &str) -> ReflectVerdict {
    let trimmed = raw.trim();
    let json_slice = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(l), Some(r)) if l < r => &trimmed[l..=r],
        _ => return fallback_stop("verifier produced no JSON"),
    };
    let wire: WireVerdict = match serde_json::from_str(json_slice) {
        Ok(v) => v,
        Err(_) => return fallback_stop("verifier produced malformed JSON"),
    };
    let confidence = sanitize_confidence(wire.confidence);
    match wire.verdict.trim().to_ascii_lowercase().as_str() {
        "stop" => ReflectVerdict::Stop {
            rationale: trim_or_default(&wire.rationale, "ok"),
            confidence,
        },
        "retry" => {
            let next_query = wire.next_query.trim();
            if next_query.is_empty() {
                // No actionable retry — coerce to Stop with a rationale that
                // names the missing field, so the caller sees what happened.
                return ReflectVerdict::Stop {
                    rationale: format!(
                        "verifier asked for retry without next_query; coerced to stop ({})",
                        trim_or_default(&wire.rationale, "no rationale")
                    ),
                    confidence,
                };
            }
            ReflectVerdict::Retry {
                rationale: trim_or_default(&wire.rationale, "needs another attempt"),
                next_query: next_query.to_string(),
                confidence,
            }
        }
        other => ReflectVerdict::Stop {
            rationale: format!("unknown verdict {other:?}; coerced to stop"),
            confidence,
        },
    }
}

/// Drop NaN, infinities, and out-of-range values; clamp anything else
/// inside [0.0, 1.0] to itself. Tolerant on purpose so a verifier emitting
/// `1.05` or `-0.1` is read as "no usable confidence" rather than poison.
fn sanitize_confidence(c: Option<f32>) -> Option<f32> {
    let v = c?;
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

fn fallback_stop(reason: &str) -> ReflectVerdict {
    // Fail-open verdicts have no claim to high confidence — leave it as
    // None so Hybrid mode doesn't silently honour a synthetic stop.
    ReflectVerdict::Stop {
        rationale: reason.to_string(),
        confidence: None,
    }
}

fn trim_or_default(s: &str, default_: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        default_.to_string()
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ReflectMode parsing ───────────────────────────────────────────────

    #[test]
    fn mode_round_trip() {
        for m in [ReflectMode::Off, ReflectMode::Self_, ReflectMode::Forced] {
            assert_eq!(ReflectMode::parse(m.as_str()).unwrap(), m);
        }
    }

    #[test]
    fn mode_parse_rejects_unknown() {
        assert!(ReflectMode::parse("nope").is_err());
        assert!(ReflectMode::parse("").is_err());
    }

    #[test]
    fn mode_default_is_off() {
        assert_eq!(ReflectMode::default(), ReflectMode::Off);
    }

    // ── verdict parsing — happy paths ─────────────────────────────────────

    #[test]
    fn stop_verdict_is_parsed() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": "looks good"}"#);
        assert_eq!(
            v,
            ReflectVerdict::Stop {
                rationale: "looks good".into(),
                confidence: None,
            }
        );
        assert!(!v.is_retry());
        assert_eq!(v.confidence(), None);
    }

    #[test]
    fn retry_verdict_is_parsed() {
        let v = parse_verdict(
            r#"{"verdict": "retry", "rationale": "missing source", "next_query": "look up X"}"#,
        );
        assert_eq!(
            v,
            ReflectVerdict::Retry {
                rationale: "missing source".into(),
                next_query: "look up X".into(),
                confidence: None,
            }
        );
        assert!(v.is_retry());
    }

    #[test]
    fn verdict_extracts_json_from_surrounding_prose() {
        let v = parse_verdict(
            "Sure, here's my verdict: {\"verdict\": \"stop\", \"rationale\": \"clean\"} done.",
        );
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
        assert_eq!(v.rationale(), "clean");
    }

    #[test]
    fn verdict_is_case_insensitive() {
        let v = parse_verdict(r#"{"verdict": "STOP", "rationale": "ok"}"#);
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
    }

    // ── verdict parsing — fail-open coercions ─────────────────────────────

    #[test]
    fn malformed_json_coerces_to_stop() {
        let v = parse_verdict("not json at all");
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
        assert!(v.rationale().contains("no JSON"));
    }

    #[test]
    fn invalid_json_inside_braces_coerces_to_stop() {
        let v = parse_verdict("{verdict: stop, no quotes}");
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
        assert!(v.rationale().contains("malformed JSON"));
    }

    #[test]
    fn unknown_verdict_value_coerces_to_stop() {
        let v = parse_verdict(r#"{"verdict": "maybe", "rationale": "?"}"#);
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
        assert!(v.rationale().contains("unknown verdict"));
    }

    #[test]
    fn retry_without_next_query_coerces_to_stop() {
        // No actionable retry — gate fails open rather than looping forever.
        let v = parse_verdict(r#"{"verdict": "retry", "rationale": "needs more"}"#);
        assert!(matches!(v, ReflectVerdict::Stop { .. }));
        assert!(v.rationale().contains("without next_query"));
    }

    #[test]
    fn empty_rationale_gets_default() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": ""}"#);
        assert_eq!(v.rationale(), "ok");
    }

    #[test]
    fn missing_rationale_gets_default() {
        // serde default = empty string; trim_or_default fills it in.
        let v = parse_verdict(r#"{"verdict": "stop"}"#);
        assert_eq!(v.rationale(), "ok");
    }

    // ── confidence parsing ────────────────────────────────────────────────

    #[test]
    fn confidence_is_parsed_when_in_range() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": "ok", "confidence": 0.83}"#);
        assert_eq!(v.confidence(), Some(0.83));
    }

    #[test]
    fn confidence_passes_through_on_retry() {
        let v = parse_verdict(
            r#"{"verdict": "retry", "rationale": "needs work", "next_query": "redo it", "confidence": 0.42}"#,
        );
        assert_eq!(v.confidence(), Some(0.42));
        assert!(v.is_retry());
    }

    #[test]
    fn confidence_missing_yields_none() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": "ok"}"#);
        assert_eq!(v.confidence(), None);
    }

    #[test]
    fn confidence_above_one_is_dropped() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": "ok", "confidence": 1.5}"#);
        assert_eq!(v.confidence(), None);
    }

    #[test]
    fn confidence_negative_is_dropped() {
        let v = parse_verdict(r#"{"verdict": "stop", "rationale": "ok", "confidence": -0.1}"#);
        assert_eq!(v.confidence(), None);
    }

    #[test]
    fn confidence_at_boundaries_is_kept() {
        assert_eq!(
            parse_verdict(r#"{"verdict": "stop", "rationale": "ok", "confidence": 0.0}"#)
                .confidence(),
            Some(0.0)
        );
        assert_eq!(
            parse_verdict(r#"{"verdict": "stop", "rationale": "ok", "confidence": 1.0}"#)
                .confidence(),
            Some(1.0)
        );
    }

    #[test]
    fn fallback_stop_has_no_confidence() {
        // Fail-open paths must not claim a confidence — Hybrid mode would
        // otherwise treat synthetic verdicts as decisively as real ones.
        let v = parse_verdict("not json at all");
        assert_eq!(v.confidence(), None);
    }

    // ── budget + threshold constants — referenced by run_with_*_reflect ───

    #[test]
    fn retry_budget_is_one() {
        // Pinned at 1 to avoid surprise looping. If this number ever needs
        // to change, do it deliberately.
        assert_eq!(RETRY_BUDGET, 1);
    }

    #[test]
    fn hybrid_threshold_is_seven_tenths() {
        // Hybrid mode's "low confidence → retry" threshold. Loose convention
        // from Codex's review_prompt.md evaluator. Change deliberately.
        assert!((HYBRID_LOW_CONFIDENCE_THRESHOLD - 0.7).abs() < f32::EPSILON);
    }

    // ── ReflectMode coverage ──────────────────────────────────────────────

    #[test]
    fn hybrid_mode_round_trips() {
        assert_eq!(ReflectMode::parse("hybrid").unwrap(), ReflectMode::Hybrid);
        assert_eq!(ReflectMode::Hybrid.as_str(), "hybrid");
    }
}
