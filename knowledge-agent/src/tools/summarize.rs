use std::sync::Arc;

use ailoy::{
    LangModelInferConfig, Message, Part, ToolDescBuilder, ToolRuntime, Value,
    agent::{LangModelProvider, ToolFunc},
};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    common::{extract_optional_i64, extract_required_str, result_to_value},
    search::SearchIndex,
};

const DEFAULT_MAX_LENGTH: usize = 500;
const CHUNK_SIZE_LINES: usize = 1000;
/// Rough estimate: 1 line ≈ 10 tokens. Documents under this threshold
/// are sent to the LLM in a single call instead of chunk-and-reduce.
const SINGLE_PASS_MAX_LINES: usize = 4000;
/// Cap on the combined chunk summaries fed into the reduce step.
/// Prevents context overflow when there are many large chunks.
const MAX_REDUCE_INPUT_CHARS: usize = 50_000;

#[derive(Debug, Clone, Serialize)]
pub struct SummarizeResult {
    pub summary: String,
    pub filepath: String,
    pub original_lines: usize,
    pub chunks_processed: usize,
    /// Number of chunks that failed to summarize (best-effort: partial results still returned).
    pub chunks_failed: usize,
    /// True when the combined chunk summaries exceeded MAX_REDUCE_INPUT_CHARS
    /// and were truncated before the final reduce step.
    pub reduce_truncated: bool,
}

/// Summarize configuration needed to call the LLM for chunk summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeConfig {
    pub model_name: String,
    pub model_provider: LangModelProvider,
}

impl SummarizeConfig {
    pub fn new(model_name: String, provider: LangModelProvider) -> Self {
        Self {
            model_name,
            model_provider: provider,
        }
    }
}

/// Call LLM to summarize a chunk of text.
async fn summarize_chunk(
    config: &SummarizeConfig,
    text: &str,
    focus: Option<&str>,
    max_length: usize,
) -> anyhow::Result<String> {
    let focus_instruction = match focus {
        Some(f) => format!(" Focus on: {}.", f),
        None => String::new(),
    };

    let prompt = format!(
        "Summarize the following text in at most {} characters.{}\n\n{}",
        max_length, focus_instruction, text
    );

    // ~4 chars per token; give 2x safety margin (model may use punctuation/spacing).
    // Clamp between 64 and 8192 tokens.
    let max_tokens = ((max_length / 2).max(64)).min(8192) as u64;

    let runtime = ailoy::agent::LangModelRuntime::new(
        config.model_name.clone(),
        config.model_provider.clone(),
    );

    let resp = runtime
        .run(
            &vec![Message::new(ailoy::Role::User).with_contents(vec![Part::text(prompt)])],
            &vec![],
            &LangModelInferConfig {
                max_tokens: Some(max_tokens),
            },
        )
        .await;
    match resp {
        Ok(output) => {
            let content = output.message.contents.first().unwrap();
            let text = content.as_text().unwrap().to_string();
            Ok(text)
        }
        Err(e) => {
            eprintln!("LangModel request failed: {}", e);
            anyhow::bail!("LangModel request failed: {}", e)
        }
    }
}

/// Summarize a full document by splitting into chunks, summarizing each,
/// then combining into a final summary.
pub async fn summarize_document(
    index: &SearchIndex,
    filepath: &str,
    max_length: usize,
    focus: Option<&str>,
    config: &SummarizeConfig,
) -> anyhow::Result<SummarizeResult> {
    let doc = index.get_document(filepath)?;
    let lines: Vec<&str> = doc.content.lines().collect();
    let original_lines = lines.len();

    if original_lines == 0 {
        return Ok(SummarizeResult {
            summary: "(empty document)".to_string(),
            filepath: filepath.to_string(),
            original_lines: 0,
            chunks_processed: 0,
            chunks_failed: 0,
            reduce_truncated: false,
        });
    }

    // Single-pass: if the document fits in the model's context, send it all at once.
    // This is cheaper (no repeated system prompts, no reduce step).
    if original_lines <= SINGLE_PASS_MAX_LINES {
        let summary = summarize_chunk(config, &doc.content, focus, max_length).await?;
        return Ok(SummarizeResult {
            summary,
            filepath: filepath.to_string(),
            original_lines,
            chunks_processed: 1,
            chunks_failed: 0,
            reduce_truncated: false,
        });
    }

    // Map-Reduce: for very large documents that exceed single-pass threshold.
    let chunks: Vec<String> = lines
        .chunks(CHUNK_SIZE_LINES)
        .map(|chunk| chunk.join("\n"))
        .collect();
    let num_chunks = chunks.len();

    // Parallelize chunk summarization with a concurrency cap to respect API rate limits.
    // Collect futures eagerly first (each owns cloned data) to avoid HRTB lifetime issues,
    // then buffer_unordered(N) polls at most N concurrently.
    const MAX_CONCURRENT_CHUNKS: usize = 5;
    let focus_owned: Option<String> = focus.map(|s| s.to_string());
    let chunk_futs: Vec<_> = chunks
        .iter()
        .map(|chunk| {
            let cfg = config.clone();
            let text = chunk.clone();
            let foc = focus_owned.clone();
            async move { summarize_chunk(&cfg, &text, foc.as_deref(), max_length).await }
        })
        .collect();
    let results: Vec<anyhow::Result<String>> = stream::iter(chunk_futs)
        .buffer_unordered(MAX_CONCURRENT_CHUNKS)
        .collect()
        .await;

    // Best-effort: keep successful summaries, count failures.
    // Only return an error if every single chunk failed.
    let mut chunk_summaries = Vec::with_capacity(num_chunks);
    let mut chunks_failed = 0usize;
    for result in results {
        match result {
            Ok(summary) => chunk_summaries.push(summary),
            Err(_) => chunks_failed += 1,
        }
    }
    if chunk_summaries.is_empty() {
        anyhow::bail!("all {} chunks failed to summarize", num_chunks);
    }

    // Cap combined input to the reduce step to prevent context overflow.
    let combined_raw = chunk_summaries.join("\n\n---\n\n");
    let (combined, reduce_truncated) = if combined_raw.len() > MAX_REDUCE_INPUT_CHARS {
        let mut end = MAX_REDUCE_INPUT_CHARS;
        while end > 0 && !combined_raw.is_char_boundary(end) {
            end -= 1;
        }
        (format!("{}\n[...truncated]", &combined_raw[..end]), true)
    } else {
        (combined_raw, false)
    };
    let final_summary = summarize_chunk(config, &combined, focus, max_length).await?;

    Ok(SummarizeResult {
        summary: final_summary,
        filepath: filepath.to_string(),
        original_lines,
        chunks_processed: num_chunks,
        chunks_failed,
        reduce_truncated,
    })
}

pub fn build_summarize_document_tool(
    index: Arc<SearchIndex>,
    config: SummarizeConfig,
) -> ToolRuntime {
    let desc = ToolDescBuilder::new("summarize_document")
        .description(
            "Summarize a document from the knowledge base. \
             Splits long documents into chunks and produces a concise summary. \
             Optionally focus the summary on a specific topic.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "filepath": {
                    "type": "string",
                    "description": "File path of the document to summarize"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum summary length in characters (default: 500)"
                },
                "focus": {
                    "type": "string",
                    "description": "Optional topic/keyword to focus the summary on"
                }
            },
            "required": ["filepath"]
        }))
        .build();

    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let idx = index.clone();
        let cfg = config.clone();
        Box::pin(async move {
            let filepath = match extract_required_str(&args, "filepath") {
                Ok(f) => f,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let max_length = extract_optional_i64(&args, "max_length")
                .map(|v| v.max(50) as usize)
                .unwrap_or(DEFAULT_MAX_LENGTH);
            let focus = args
                .pointer("/focus")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            match summarize_document(&idx, &filepath, max_length, focus.as_deref(), &cfg).await {
                Ok(result) => result_to_value(&result),
                Err(e) => json!({ "error": e.to_string() }).into(),
            }
        })
    });

    ToolRuntime::new(desc, f)
}
