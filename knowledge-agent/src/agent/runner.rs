use std::sync::LazyLock;

use ailoy::{
    TurnEvent,
    agent::AgentRuntime,
    message::{Part, Role},
};
use anyhow::Result;
use futures::StreamExt;
use regex::Regex;

use super::tracer::{self, Step};

const MAX_RETRIES: usize = 5;
const MAX_TOOL_CALLS: usize = 20;

static RE_RETRY_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"try again in (\d+\.?\d*)s").unwrap());

/// Parse "Please try again in 6.464s" from rate limit error messages.
fn parse_retry_after(err: &str) -> u64 {
    if let Some(caps) = RE_RETRY_AFTER.captures(err) {
        if let Ok(secs) = caps[1].parse::<f64>() {
            return (secs * 1000.0) as u64 + 2000;
        }
    }
    15000
}

/// Infer tool name from output structure when position-based matching may be wrong.
fn infer_tool_name(output: &serde_json::Value, positional: Option<&String>) -> String {
    // Each tool has a distinct output shape — use that as ground truth
    if output.get("matches").is_some() && output.get("pattern").is_some() {
        return "glob_document".to_string();
    }
    if output.get("matches").is_some() && output.get("filepath").is_some() {
        return "find_in_document".to_string();
    }
    if output.get("results").is_some() && output.get("query").is_some() {
        return "search_document".to_string();
    }
    if output.get("content").is_some() && output.get("start_line").is_some() {
        return "open_document".to_string();
    }
    // calculate: has "result" + "expression" or "error" + "expression"
    if output.get("expression").is_some()
        && (output.get("result").is_some() || output.get("error").is_some())
    {
        return "calculate".to_string();
    }
    positional.cloned().unwrap_or_else(|| "unknown".to_string())
}

/// Run an agent with step tracing and rate-limit retry.
///
/// - Streams `agent.stream_turn()` and collects each step (Thinking/Reasoning/Answer/ToolCall/ToolResult).
/// - On 429 rate limit: waits, then retries with a continuation prompt so the LLM
///   can use tool results already in its history.
/// - Steps are accumulated across retries.
///
/// Returns `(final_answer_text, all_steps)`.
pub async fn run_with_trace(agent: &mut AgentRuntime, prompt: &str) -> Result<(String, Vec<Step>)> {
    let mut steps: Vec<Step> = Vec::new();
    let mut final_answer = String::new();
    let mut step_num = 0usize;
    let mut pending_tool_calls: Vec<String> = Vec::new();
    let mut tool_call_count = 0usize;

    for attempt in 0..MAX_RETRIES {
        let turn_prompt = if attempt == 0 {
            prompt.to_string()
        } else {
            let wait = 15_000 * (1 << (attempt - 1) as u64);
            println!(
                "    [retry {}/{}] waiting {:.0}s...",
                attempt + 1,
                MAX_RETRIES,
                wait as f64 / 1000.0
            );
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            "Continue from where you left off. Use the tool results already in the conversation to answer the question.".to_string()
        };

        let query =
            ailoy::message::Message::new(Role::User).with_contents([Part::text(&turn_prompt)]);
        let mut stream = agent.stream_turn(query);
        let mut had_error = false;

        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(o) => o,
                Err(e) => {
                    let err_str = e.to_string();
                    eprintln!("[ERROR] {}", err_str);
                    if err_str.contains("429") && attempt < MAX_RETRIES - 1 {
                        let wait = parse_retry_after(&err_str);
                        println!(
                            "    [rate limit] waiting {:.0}s before retry...",
                            wait as f64 / 1000.0
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                        had_error = true;
                        break;
                    }
                    anyhow::bail!(err_str);
                }
            };

            match event {
                TurnEvent::AssistantMessage(output) => {
                    let msg = &output.message;
                    if let Some(ref thinking) = msg.thinking {
                        if !thinking.is_empty() {
                            step_num += 1;
                            let step = Step::Thinking {
                                content: thinking.clone(),
                            };
                            tracer::print_step(step_num, &step);
                            steps.push(step);
                        }
                    }
                    let has_tool_calls = msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                    if has_tool_calls {
                        pending_tool_calls.clear();
                    }
                    for part in &msg.contents {
                        if let Some(text) = part.as_text() {
                            if !text.is_empty() {
                                step_num += 1;
                                let step = if has_tool_calls {
                                    Step::Reasoning {
                                        content: text.to_string(),
                                    }
                                } else {
                                    final_answer = text.to_string();
                                    Step::Answer {
                                        content: text.to_string(),
                                    }
                                };
                                tracer::print_step(step_num, &step);
                                steps.push(step);
                            }
                        }
                    }
                }
                TurnEvent::ToolCall {
                    id: _,
                    name,
                    arguments,
                } => {
                    tool_call_count += 1;
                    let input = serde_json::to_value(arguments).unwrap_or_default();
                    step_num += 1;
                    let step = Step::ToolCall {
                        name: name.clone(),
                        input,
                    };
                    tracer::print_step(step_num, &step);
                    steps.push(step);
                    pending_tool_calls.push(name);
                    if tool_call_count >= MAX_TOOL_CALLS {
                        println!(
                            "    [limit] max tool calls ({}) reached, stopping",
                            MAX_TOOL_CALLS
                        );
                        return Ok((final_answer, steps));
                    }
                }
                TurnEvent::ToolResult(msg) => {
                    let mut call_idx = 0;
                    for part in &msg.contents {
                        let output_val = if let Some(val) = part.as_value() {
                            serde_json::to_value(val).unwrap_or_default()
                        } else if let Some(text) = part.as_text() {
                            serde_json::from_str(text)
                                .unwrap_or(serde_json::Value::String(text.to_string()))
                        } else {
                            serde_json::Value::Null
                        };
                        let name = infer_tool_name(&output_val, pending_tool_calls.get(call_idx));
                        call_idx += 1;
                        let summary = tracer::summarize_tool_result(&name, &output_val);
                        step_num += 1;
                        let step = Step::ToolResult {
                            name,
                            summary,
                            output: output_val,
                        };
                        tracer::print_step(step_num, &step);
                        steps.push(step);
                    }
                }
                TurnEvent::ToolDelta(_) => {}
            }
        }

        if had_error {
            continue;
        }
        return Ok((final_answer, steps));
    }
    anyhow::bail!("max retries exceeded")
}

/// Like `run_with_trace` but sends each `Step` to a channel instead of printing.
/// Useful for TUI integration where the caller consumes steps asynchronously.
pub async fn run_with_trace_channel(
    agent: &mut AgentRuntime,
    prompt: &str,
    tx: tokio::sync::mpsc::UnboundedSender<Step>,
) -> Result<(String, Vec<Step>)> {
    let mut steps: Vec<Step> = Vec::new();
    let mut final_answer = String::new();
    let mut pending_tool_calls: Vec<String> = Vec::new();
    let mut tool_call_count = 0usize;

    for attempt in 0..MAX_RETRIES {
        let turn_prompt = if attempt == 0 {
            prompt.to_string()
        } else {
            let wait = 15_000 * (1 << (attempt - 1) as u64);
            let _ = tx.send(Step::Reasoning {
                content: format!(
                    "[retry {}/{}] waiting {:.0}s...",
                    attempt + 1,
                    MAX_RETRIES,
                    wait as f64 / 1000.0
                ),
            });
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            "Continue from where you left off. Use the tool results already in the conversation to answer the question.".to_string()
        };

        let query =
            ailoy::message::Message::new(Role::User).with_contents([Part::text(&turn_prompt)]);
        let mut stream = agent.stream_turn(query);
        let mut had_error = false;

        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(o) => o,
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("429") && attempt < MAX_RETRIES - 1 {
                        let wait = parse_retry_after(&err_str);
                        let _ = tx.send(Step::Reasoning {
                            content: format!(
                                "[rate limit] waiting {:.0}s before retry...",
                                wait as f64 / 1000.0
                            ),
                        });
                        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                        had_error = true;
                        break;
                    }
                    anyhow::bail!(err_str);
                }
            };

            match event {
                TurnEvent::AssistantMessage(output) => {
                    let msg = &output.message;
                    if let Some(ref thinking) = msg.thinking {
                        if !thinking.is_empty() {
                            let step = Step::Thinking {
                                content: thinking.clone(),
                            };
                            let _ = tx.send(step.clone());
                            steps.push(step);
                        }
                    }
                    let has_tool_calls = msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                    if has_tool_calls {
                        pending_tool_calls.clear();
                    }
                    for part in &msg.contents {
                        if let Some(text) = part.as_text() {
                            if !text.is_empty() {
                                let step = if has_tool_calls {
                                    Step::Reasoning {
                                        content: text.to_string(),
                                    }
                                } else {
                                    final_answer = text.to_string();
                                    Step::Answer {
                                        content: text.to_string(),
                                    }
                                };
                                let _ = tx.send(step.clone());
                                steps.push(step);
                            }
                        }
                    }
                }
                TurnEvent::ToolCall {
                    id: _,
                    name,
                    arguments,
                } => {
                    tool_call_count += 1;
                    let input = serde_json::to_value(arguments).unwrap_or_default();
                    let step = Step::ToolCall {
                        name: name.clone(),
                        input,
                    };
                    let _ = tx.send(step.clone());
                    steps.push(step);
                    pending_tool_calls.push(name);
                    if tool_call_count >= MAX_TOOL_CALLS {
                        let _ = tx.send(Step::Reasoning {
                            content: format!(
                                "[limit] max tool calls ({}) reached, stopping",
                                MAX_TOOL_CALLS
                            ),
                        });
                        return Ok((final_answer, steps));
                    }
                }
                TurnEvent::ToolResult(msg) => {
                    let mut call_idx = 0;
                    for part in &msg.contents {
                        let output_val = if let Some(val) = part.as_value() {
                            serde_json::to_value(val).unwrap_or_default()
                        } else if let Some(text) = part.as_text() {
                            serde_json::from_str(text)
                                .unwrap_or(serde_json::Value::String(text.to_string()))
                        } else {
                            serde_json::Value::Null
                        };
                        let name = infer_tool_name(&output_val, pending_tool_calls.get(call_idx));
                        call_idx += 1;
                        let summary = tracer::summarize_tool_result(&name, &output_val);
                        let step = Step::ToolResult {
                            name,
                            summary,
                            output: output_val,
                        };
                        let _ = tx.send(step.clone());
                        steps.push(step);
                    }
                }
                TurnEvent::ToolDelta(_) => {}
            }
        }

        if had_error {
            continue;
        }
        return Ok((final_answer, steps));
    }
    anyhow::bail!("max retries exceeded")
}
