use std::{path::PathBuf, sync::Arc};

use ailoy::{ToolDescBuilder, ToolRuntime, Value, agent::ToolFunc};
use futures::future::BoxFuture;
use globset::{GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use serde::Serialize;
use serde_json::json;

use super::common::{extract_optional_i64, extract_required_str, result_to_value};

const DEFAULT_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize)]
pub struct GlobMatch {
    pub filepath: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobResult {
    pub pattern: String,
    pub matches: Vec<GlobMatch>,
    pub total_found: usize,
    pub truncated: bool,
}

/// Glob search across multiple target directories.
///
/// Walks each directory respecting .gitignore, matches against the pattern,
/// and returns relative paths (relative to each dir).
pub fn glob_documents(
    target_dirs: &[PathBuf],
    pattern: &str,
    limit: Option<usize>,
) -> anyhow::Result<GlobResult> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let matcher = build_matcher(pattern)?;

    let mut results: Vec<GlobMatch> = Vec::new();
    let mut truncated = false;

    for dir in target_dirs {
        if !dir.exists() {
            continue;
        }

        let walker = WalkBuilder::new(dir).hidden(false).git_ignore(true).build();

        for entry in walker {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let relative = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();

            if matcher.is_match(&relative) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                results.push(GlobMatch {
                    filepath: relative,
                    size,
                });

                if results.len() >= limit {
                    truncated = true;
                    break;
                }
            }
        }

        if truncated {
            break;
        }
    }

    results.sort_by(|a, b| a.filepath.cmp(&b.filepath));
    let total_found = results.len();

    Ok(GlobResult {
        pattern: pattern.to_string(),
        matches: results,
        total_found,
        truncated,
    })
}

fn build_matcher(pattern: &str) -> anyhow::Result<GlobSet> {
    let normalized = pattern.replace(' ', "*").replace('\'', "*");
    let mut builder = GlobSetBuilder::new();
    let glob = globset::GlobBuilder::new(&normalized)
        .case_insensitive(true)
        .build()?;
    builder.add(glob);
    Ok(builder.build()?)
}

pub fn build_glob_document_tool(target_dirs: Vec<PathBuf>) -> ToolRuntime {
    let desc = ToolDescBuilder::new("glob_document")
            .description(
                "Find files by glob pattern across the target directories. \
                 Returns matching file paths and sizes. \
                 Use this to explore available documents before searching their content."
            )
            .parameters(json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern (e.g. '*.md', '3M_*', '**/*.txt')" },
                    "limit": { "type": "integer", "description": "Max results to return (default 100)" }
                },
                "required": ["pattern"]
            }))
            .build();

    let dirs = target_dirs.clone();
    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let dirs = dirs.clone();
        Box::pin(async move {
            let pattern = match extract_required_str(&args, "pattern") {
                Ok(p) => p,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let limit = extract_optional_i64(&args, "limit").map(|v| v.max(1) as usize);

            match glob_documents(&dirs, &pattern, limit) {
                Ok(result) => result_to_value(&result),
                Err(e) => json!({ "error": e.to_string() }).into(),
            }
        })
    });

    ToolRuntime::new(desc, f)
}
