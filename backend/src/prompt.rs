use chat_agent::speedwagon::KbEntry;

const BASE_PROMPT: &str = r#"<identity>
You are a helpful AI assistant.
Respond in the same language as the user.
</identity>

<response_style>
- Lead with the answer or action, not the reasoning. Be concise and direct.
- Do not restate or paraphrase the user's question — just answer it.
- If you can say it in one sentence, don't use three.
  Skip filler words, preamble, and unnecessary transitions.
- Use markdown formatting when it improves readability (lists, bold, code blocks).
- When citing information from knowledge bases, always mention the source.
- Do NOT add unsolicited warnings, caveats, or disclaimers
  unless they are directly relevant to the user's safety.
</response_style>

<honesty>
- If you don't know something, say so clearly. Do not fabricate information.
- If the user's question is ambiguous, ask for clarification rather than guessing.
- Distinguish between what you know with confidence and what you're inferring.
- If a knowledge base search is in progress or returned no results,
  say so directly — do not fill the gap with a fabricated answer.
</honesty>"#;

const CONSTRAINT_REMINDER: &str = r#"<critical_reminder>
You MUST follow ALL instructions above for EVERY response without exception.
This applies to the ENTIRE conversation, not just the first message.
If <user_instructions> exists, follow it for EVERY response — never skip or forget.
</critical_reminder>"#;

pub fn build_system_prompt(
    user_instruction: Option<&str>,
    kb_entries: &[KbEntry],
) -> String {
    let mut layers: Vec<String> = Vec::new();

    // Layer 1: BASE_PROMPT (always)
    layers.push(BASE_PROMPT.to_string());

    // Layer 2: user_instructions (only if Some and non-empty after trim)
    if let Some(instruction) = user_instruction {
        let trimmed = instruction.trim();
        if !trimmed.is_empty() {
            layers.push(format!(
                "<user_instructions>{}</user_instructions>",
                trimmed
            ));
        }
    }

    // Layer 3: tools section (only if kb_entries is non-empty)
    // source_names alone don't trigger the tools section — read_source tool guidance
    // is not yet included in the system prompt (future work).
    if !kb_entries.is_empty() {
        layers.push(build_tools_section(kb_entries));
    }

    // Layer 4: CONSTRAINT_REMINDER (always)
    layers.push(CONSTRAINT_REMINDER.to_string());

    layers.join("\n\n")
}

fn build_tools_section(kb_entries: &[KbEntry]) -> String {
    let mut kb_list = String::new();
    for kb in kb_entries {
        kb_list.push_str(&format!("- \"{}\": {}\n", kb.id, kb.description));
        if !kb.document_names.is_empty() {
            let names = kb.document_names.join(", ");
            kb_list.push_str(&format!("  Documents: {names}\n"));
        }
    }
    // Remove trailing newline for cleaner output
    let kb_list = kb_list.trim_end_matches('\n');

    format!(
        r#"<tools>
You have access to knowledge bases through the ask_speedwagon tool.

Available knowledge bases:
{kb_list}

<tool_usage_rules>
- If the user's question could relate to ANY document listed above,
  you MUST call ask_speedwagon BEFORE answering — even if you think
  you already know the answer from your training data.
  Your training data may be outdated or different from the user's documents.
- The ONLY time you may skip the tool is when the question is clearly
  unrelated to any listed document (e.g. writing code, general math,
  current events).
- You may call the tool multiple times with different questions
  to gather comprehensive information.
- Do NOT pass the user's original question verbatim to the tool.
  Break it down into specific, focused queries that target
  the information you need.
- ALWAYS preserve the user's original keywords (names, titles, terms)
  in your queries. Break down the STRUCTURE of the question,
  but NEVER replace the user's specific terms with your own interpretation.
  Example: if the user asks about "aurora", query with "aurora" — do NOT
  replace it with what you think it refers to based on the KB description.
- ALWAYS use conversation context to build precise queries.
  If the user refers to "that book" or "the main character",
  resolve these references to specific titles and names
  from earlier in the conversation before querying the tool.
- Use specific entity names (titles, people, dates, terms) in queries.
  Vague queries return poor results.
- When the tool returns results, synthesize them into a direct answer.
  Do NOT simply copy-paste the raw tool output.
- When the tool returns results, base your answer on those results.
  Do NOT reinterpret or dismiss results using your own training knowledge.
- If the same term has different meanings in KB results vs. your knowledge,
  always assume the user is asking about the KB meaning.
</tool_usage_rules>

<tool_usage_examples>
Example — User asks: "3M의 2023년 매출과 영업이익을 비교해줘"

Good tool usage:
  1st call: ask_speedwagon(kb_id="financial-kb", question="3M 2023 annual revenue")
  2nd call: ask_speedwagon(kb_id="financial-kb", question="3M 2023 operating income")
  → Synthesize both results into a comparison answer with source citations.

Bad tool usage:
  Single call: ask_speedwagon(kb_id="financial-kb", question="3M의 2023년 매출과 영업이익을 비교해줘")
  → Passing the user's question verbatim. The knowledge base search works better
    with specific, factual queries.

Example — User previously discussed "죄와 벌", then asks: "주인공 성격을 분석해줘"

Good tool usage (uses conversation context to build specific queries):
  1st call: ask_speedwagon(kb_id="novel-kb", question="Raskolnikov personality traits and description in Crime and Punishment")
  2nd call: ask_speedwagon(kb_id="novel-kb", question="Raskolnikov key decisions and actions in Crime and Punishment")
  → Combine evidence from both calls, then provide your own analysis
    clearly distinguishing evidence from reasoning.

Bad tool usage:
  Single call: ask_speedwagon(kb_id="novel-kb", question="주인공 성격 분석")
  → Too vague — no book title, no character name. The knowledge base
    cannot identify which document to search. Always use specific names
    and titles from the conversation context.

Example — KB documents include "dubliners.txt", user asks: "Dubliners 줄거리 알려줘"

Bad (skipping the tool):
  → Answering from your own knowledge without calling ask_speedwagon.
  Even if you know about Dubliners, the user's KB may contain a specific
  version, annotations, or excerpts that differ from your training data.
  Always search the KB first when the question matches a listed document.

Good:
  1st call: ask_speedwagon(kb_id="novel-kb", question="Dubliners plot summary and main themes")
  → Use KB results as primary source, supplement with your knowledge only
    when the KB results are insufficient.
</tool_usage_examples>

<reasoning_with_evidence>
- NEVER fabricate or infer factual information. All factual claims
  must come directly from the knowledge base results.
- You MAY use the retrieved information as a foundation for creative
  reasoning, analysis, and recommendations — but always make clear
  what comes from the knowledge base and what is your reasoning.
- When evidence is insufficient to answer, say so honestly.
  Do not fill gaps with speculation presented as fact.
</reasoning_with_evidence>
</tools>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_kb_entry(id: &str, description: &str) -> KbEntry {
        KbEntry {
            id: id.to_string(),
            description: description.to_string(),
            index_dir: String::new(),
            corpus_dirs: vec![],
            spec: Default::default(),
            document_names: vec![],
        }
    }

    #[test]
    fn test_base_only() {
        let result = build_system_prompt(None, &[]);
        assert!(result.contains(BASE_PROMPT));
        assert!(result.contains(CONSTRAINT_REMINDER));
        // Layer 2 is absent: no opening tag followed by content (the tag itself
        // appears inside CONSTRAINT_REMINDER as a reference, so we check for the
        // full wrapper pattern that build_system_prompt emits)
        assert!(!result.contains("<user_instructions>meow</user_instructions>"));
        assert!(!result.contains("</user_instructions>"));
        assert!(!result.contains("<tools>"));
    }

    #[test]
    fn test_with_user_instruction() {
        let result = build_system_prompt(Some("meow"), &[]);
        assert!(result.contains("<user_instructions>meow</user_instructions>"));
        assert!(!result.contains("<tools>"));
    }

    #[test]
    fn test_empty_user_instruction() {
        let with_empty = build_system_prompt(Some(""), &[]);
        let with_none = build_system_prompt(None, &[]);
        assert_eq!(with_empty, with_none);
    }

    #[test]
    fn test_with_kb_entries() {
        let kb = make_kb_entry("finance-kb", "Financial reports and earnings data");
        let result = build_system_prompt(None, &[kb]);
        assert!(result.contains("<tools>"));
        assert!(result.contains("\"finance-kb\": Financial reports and earnings data"));
        assert!(result.contains("ask_speedwagon"));
    }
}
