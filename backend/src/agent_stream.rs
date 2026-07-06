//! Reassembles ailoy's raw streaming output into discrete, actionable items.
//!
//! `Agent::run_stream` yields a uniform stream of [`MessageDeltaOutput`]s.
//! ailoy's stream contract guarantees every message ends with a delta carrying a
//! `finish_reason` (the LangModel layer synthesizes a terminal Stop delta when a
//! provider ends the stream without one), so a message boundary is exactly
//! "`finish_reason` present". [`MessageAssembler`] folds deltas to that boundary
//! while also passing the assistant's text through for live rendering —
//! recreating the shape of ailoy's old `AgentEvent` (Delta | Message) for
//! consumers that want both views. A non-conforming stream (role change with no
//! intervening `finish_reason`) fails loudly via `accumulate` rather than being
//! silently healed.

use ailoy::message::{
    Delta as _, FinishReason, MessageDeltaOutput, MessageOutput, PartDelta, Role,
};

/// One actionable item derived from the raw delta stream.
#[derive(Debug)]
pub enum AgentStreamItem {
    /// Newly streamed top-level assistant text for live rendering. Empty
    /// fragments are never emitted, and tool results and sub-agent (depth ≥ 1)
    /// output never appear here — they surface only as
    /// [`AgentStreamItem::Completed`], matching the pre-streaming-API semantics.
    Delta(String),
    /// A message finalized at a boundary (equivalent to the old
    /// `AgentEvent::Message`): a completed assistant turn or a tool result. Boxed
    /// so this variant doesn't bloat the small `Delta` case.
    Completed(Box<MessageOutput>),
}

/// Accumulates streamed [`MessageDeltaOutput`]s and emits [`AgentStreamItem`]s at
/// message boundaries. Feed each delta to [`push`](Self::push); call
/// [`finish`](Self::finish) once the stream ends to flush any trailing message.
#[derive(Default)]
pub struct MessageAssembler {
    /// The message currently being streamed, accumulated across deltas.
    acc: MessageDeltaOutput,
}

impl MessageAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one streamed delta. Returns the items to act on, in order: an
    /// optional `Delta` (assistant text from this delta), then an optional
    /// `Completed` (when this delta carries the message's `finish_reason`).
    /// `Err` carries a finalization/accumulation failure message; a role change
    /// with no intervening `finish_reason` (contract violation) lands here via
    /// `accumulate`'s role-mismatch rejection.
    pub fn push(&mut self, delta: MessageDeltaOutput) -> Result<Vec<AgentStreamItem>, String> {
        let mut items = Vec::new();

        // 1. Live assistant text — top-level only. A sub-agent's answer is
        //    re-emitted on this same stream as a role=Assistant one-shot at
        //    depth ≥ 1; streaming it here would splice the sub-agent's internal
        //    text into the top-level answer (and a stop would persist that mix).
        //    A continuation delta carries no role/depth, so fall back to the
        //    in-progress message's; exclude Tool rather than require Assistant
        //    so text streamed before the role marker isn't dropped.
        let effective_role = delta.delta.role.as_ref().or(self.acc.delta.role.as_ref());
        let is_top_level = matches!(delta.depth.or(self.acc.depth), None | Some(0));
        if is_top_level && !matches!(effective_role, Some(Role::Tool)) {
            let mut fragment = String::new();
            for part in &delta.delta.contents {
                if let PartDelta::Text { text } = part {
                    fragment.push_str(text);
                }
            }
            if !fragment.is_empty() {
                items.push(AgentStreamItem::Delta(fragment));
            }
        }

        // 2. Fold this delta into the running message.
        let acc = std::mem::take(&mut self.acc);
        self.acc = acc.accumulate(delta).map_err(|e| e.to_string())?;

        // 3. A delta carrying finish_reason finalizes the message — per ailoy's
        //    stream contract, this is the message boundary.
        if self.acc.finish_reason.is_some() {
            let done = std::mem::take(&mut self.acc);
            items.push(AgentStreamItem::Completed(Box::new(
                done.finish().map_err(|e| e.to_string())?,
            )));
        }

        Ok(items)
    }

    /// Flush a message still accumulating when the stream ends. Defensive:
    /// ailoy's contract (every message ends with a finish_reason delta) makes
    /// this a no-op for conforming streams — `Some` here means an upstream
    /// producer broke the contract, so the caller should log it. The message is
    /// still returned (finalized as Stop) rather than dropped, because losing a
    /// completed answer is strictly worse than surfacing a contract bug quietly.
    /// Returns `None` if nothing pending.
    pub fn finish(&mut self) -> Result<Option<MessageOutput>, String> {
        if self.acc.delta.role.is_none() {
            return Ok(None);
        }
        let mut done = std::mem::take(&mut self.acc);
        done.finish_reason.get_or_insert(FinishReason::Stop {});
        Ok(Some(done.finish().map_err(|e| e.to_string())?))
    }
}

#[cfg(test)]
mod tests {
    use ailoy::message::MessageDelta;

    use super::*;

    /// Build a streamed delta with an optional role, text, and finish_reason.
    fn delta(role: Option<Role>, text: &str, finish: bool) -> MessageDeltaOutput {
        let mut md = MessageDelta::new();
        if let Some(r) = role {
            md = md.with_role(r);
        }
        if !text.is_empty() {
            md = md.with_contents([PartDelta::Text { text: text.into() }]);
        }
        let mut out = MessageDeltaOutput::new();
        out.delta = md;
        out.depth = Some(0);
        if finish {
            out.finish_reason = Some(FinishReason::Stop {});
        }
        out
    }

    fn text_of(items: &[AgentStreamItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                AgentStreamItem::Delta(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn completed(items: Vec<AgentStreamItem>) -> Vec<MessageOutput> {
        items
            .into_iter()
            .filter_map(|i| match i {
                AgentStreamItem::Completed(o) => Some(*o),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn streams_text_then_completes_on_finish_reason() {
        let mut a = MessageAssembler::new();

        let first = a.push(delta(Some(Role::Assistant), "Hel", false)).unwrap();
        assert_eq!(text_of(&first), vec!["Hel"]);
        assert!(completed(first).is_empty());

        let second = a.push(delta(None, "lo", true)).unwrap();
        assert_eq!(text_of(&second), vec!["lo"]);
        let done = completed(second);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].message.role, Role::Assistant);
        assert_eq!(done[0].message.contents[0].as_text().unwrap(), "Hello");
        assert!(matches!(done[0].finish_reason, FinishReason::Stop {}));

        // Nothing pending after a clean finish.
        assert!(a.finish().unwrap().is_none());
    }

    #[test]
    fn flushes_trailing_message_when_stream_ends_without_finish_reason() {
        // Defensive path: ailoy's contract means a conforming stream never ends
        // mid-message, but if one does (upstream bug), the trailing message must
        // be recovered — losing a completed answer is worse than the anomaly.
        let mut a = MessageAssembler::new();

        let items = a
            .push(delta(Some(Role::Assistant), "answer", false))
            .unwrap();
        assert_eq!(text_of(&items), vec!["answer"]);
        assert!(completed(items).is_empty());

        let trailing = a.finish().unwrap().expect("trailing message must flush");
        assert_eq!(trailing.message.role, Role::Assistant);
        assert_eq!(trailing.message.contents[0].as_text().unwrap(), "answer");
        assert!(matches!(trailing.finish_reason, FinishReason::Stop {}));
    }

    #[test]
    fn tool_result_surfaces_only_as_completed_never_as_delta() {
        let mut a = MessageAssembler::new();
        // Finish an assistant turn first.
        a.push(delta(Some(Role::Assistant), "calling", true))
            .unwrap();

        // A tool result arrives as its own role=Tool one-shot delta.
        let items = a
            .push(delta(Some(Role::Tool), "tool output", true))
            .unwrap();
        assert!(
            text_of(&items).is_empty(),
            "tool result must not stream as assistant text"
        );
        let done = completed(items);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].message.role, Role::Tool);
    }

    #[test]
    fn role_change_without_finish_reason_errors_loudly() {
        // ailoy's contract guarantees a finish_reason delta closes every message
        // before the role can change; a violation must surface as an error (via
        // accumulate's role-mismatch rejection), not be silently healed into a
        // split — silent healing would mask a broken producer.
        let mut a = MessageAssembler::new();
        let first = a
            .push(delta(Some(Role::Assistant), "thinking", false))
            .unwrap();
        assert_eq!(text_of(&first), vec!["thinking"]);
        assert!(completed(first).is_empty());

        let err = a
            .push(delta(Some(Role::Tool), "result", true))
            .expect_err("role change without finish_reason must be rejected");
        assert!(
            err.contains("role"),
            "error should mention the role mismatch: {err}"
        );
    }

    #[test]
    fn subagent_output_never_streams_as_live_delta() {
        // A sub-agent's answer is re-emitted on the stream as a one-shot
        // role=Assistant delta at depth >= 1. It must not surface as live
        // top-level text (that would splice the sub-agent's internal answer
        // into the main bubble, and a stop would persist the mix) — only as a
        // Completed message, which the client routes to the sub-agent UI.
        let mut a = MessageAssembler::new();
        let mut d = delta(Some(Role::Assistant), "subagent internal answer", true);
        d.depth = Some(1);
        let items = a.push(d).unwrap();
        assert!(
            text_of(&items).is_empty(),
            "sub-agent text must not stream as the top-level answer"
        );
        let done = completed(items);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].depth, Some(1));
        assert_eq!(
            done[0].message.contents[0].as_text().unwrap(),
            "subagent internal answer"
        );
    }

    #[test]
    fn empty_fragments_are_not_emitted() {
        let mut a = MessageAssembler::new();
        // A role-only opener (e.g. message_start) carries no text.
        let items = a.push(delta(Some(Role::Assistant), "", false)).unwrap();
        assert!(text_of(&items).is_empty());
        assert!(completed(items).is_empty());
    }

    #[test]
    fn finish_on_empty_assembler_yields_nothing() {
        let mut a = MessageAssembler::new();
        assert!(a.finish().unwrap().is_none());
    }
}
