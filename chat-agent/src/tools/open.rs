//! `open_file` tool — read a range of lines from a local `.md` / `.txt` file.
//!
//! Intended pairing: `convert_pdf_to_md` returns `{ md_path, size_chars }`;
//! the LLM then calls `open_file(filepath=md_path, start_line, end_line)`
//! to read slices of the converted markdown.
//!
//! The extension whitelist alone can't stop symlink/traversal reads, so paths
//! are canonicalized and must stay under `allowed_root()` (the system temp
//! dir, where `convert_pdf_to_md` writes its output).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ailoy::{ToolAsyncFunc, ToolDesc, ToolDescBuilder, ToolRuntime, Value};
use serde::Serialize;

use crate::error_value;

pub const OPEN_FILE_TOOL: &str = "open_file";

const MAX_CONTENT_CHARS: usize = 40000;
const MAX_LINES_PER_OPEN: usize = 200;
const DEFAULT_WINDOW_LINES: usize = 150;
const ALLOWED_EXTENSIONS: &[&str] = &["md", "txt"];

#[derive(Debug, Clone, Serialize)]
pub struct OpenResult {
    pub filepath: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub truncated: bool,
    pub content: String,
}

// Slice `content` into a line-numbered window, truncating at `MAX_CONTENT_CHARS`.
pub fn open_file(
    filepath: &str,
    content: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> OpenResult {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let start = start_line.unwrap_or(1).max(1);
    let end = end_line
        .unwrap_or(start + DEFAULT_WINDOW_LINES)
        .min(total_lines);
    let end = end.min(start + MAX_LINES_PER_OPEN - 1);

    let start_idx = (start - 1).min(total_lines);
    let end_idx = end.min(total_lines);

    let mut out = String::new();
    let mut truncated = false;
    let mut actual_end = start_idx;

    for (i, line) in lines[start_idx..end_idx].iter().enumerate() {
        let formatted = format!("{}: {}\n", start + i, line);
        if out.len() + formatted.len() > MAX_CONTENT_CHARS {
            truncated = true;
            break;
        }
        out.push_str(&formatted);
        actual_end = start_idx + i + 1;
    }

    if truncated {
        out.push_str(&format!(
            "\n[truncated at {} chars — use a smaller line range]",
            MAX_CONTENT_CHARS
        ));
    }

    OpenResult {
        filepath: filepath.to_string(),
        start_line: start,
        end_line: actual_end,
        total_lines,
        truncated,
        content: out,
    }
}

fn has_allowed_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            ALLOWED_EXTENSIONS.iter().any(|&a| a == lower)
        })
        .unwrap_or(false)
}

// Directory that canonicalized paths must stay under.
// `convert_pdf_to_md` writes via Python `tempfile`, which uses the system temp dir.
fn allowed_root() -> PathBuf {
    let tmp = std::env::temp_dir();
    tmp.canonicalize().unwrap_or(tmp)
}

fn result_to_value(result: &OpenResult) -> Value {
    let json = serde_json::to_value(result).unwrap_or(serde_json::Value::Null);
    serde_json::from_value::<Value>(json).unwrap_or(Value::Null)
}

pub fn build_open_file_tool() -> Option<(String, ToolRuntime)> {
    let desc = open_file_desc();
    let func = open_file_func();
    Some((
        OPEN_FILE_TOOL.to_string(),
        ToolRuntime::new_async(desc, func),
    ))
}

fn open_file_desc() -> ToolDesc {
    ToolDescBuilder::new(OPEN_FILE_TOOL)
        .description(format!(
            "Read a range of lines from a local .md or .txt file. Returns line-numbered content. \
             Use this to read the markdown produced by `convert_pdf_to_md` (pass its `md_path` as filepath). \
             Keep ranges small (20-40 lines) to be efficient. \
             Requests larger than {MAX_LINES_PER_OPEN} lines are capped. \
             Allowed extensions: {}.",
            ALLOWED_EXTENSIONS.join(", ")
        ))
        .parameters(Value::object([
            ("type", Value::string("object")),
            (
                "properties",
                Value::object([
                    (
                        "filepath",
                        Value::object([
                            ("type", Value::string("string")),
                            (
                                "description",
                                Value::string("Path to a .md or .txt file"),
                            ),
                        ]),
                    ),
                    (
                        "start_line",
                        Value::object([
                            ("type", Value::string("integer")),
                            (
                                "description",
                                Value::string("Start line (1-based, default 1)"),
                            ),
                        ]),
                    ),
                    (
                        "end_line",
                        Value::object([
                            ("type", Value::string("integer")),
                            (
                                "description",
                                Value::string(format!(
                                    "End line (default start+{DEFAULT_WINDOW_LINES})"
                                )),
                            ),
                        ]),
                    ),
                ]),
            ),
            ("required", Value::array([Value::string("filepath")])),
        ]))
        .build()
}

fn open_file_func() -> Arc<ToolAsyncFunc> {
    Arc::new(move |args: Value| {
        Box::pin(async move {
            let args_map = match args.as_object() {
                Some(m) => m,
                None => return error_value("invalid_arguments"),
            };

            let filepath = match args_map.get("filepath").and_then(Value::as_str) {
                Some(s) => s.to_string(),
                None => return error_value("missing filepath"),
            };

            let canonical = match tokio::fs::canonicalize(&filepath).await {
                Ok(p) => p,
                Err(_) => return error_value("file not found or inaccessible"),
            };

            if !canonical.starts_with(allowed_root()) {
                return error_value("path outside allowed directory");
            }

            // narrow to .md/.txt even inside allowed_root().
            if !has_allowed_extension(&canonical) {
                return error_value(&format!(
                    "disallowed file extension: only {} are allowed",
                    ALLOWED_EXTENSIONS.join(", ")
                ));
            }

            let start_line = args_map
                .get("start_line")
                .and_then(Value::as_integer)
                .map(|v| v.max(1) as usize);
            let end_line = args_map
                .get("end_line")
                .and_then(Value::as_integer)
                .map(|v| v.max(1) as usize);

            let content = match tokio::fs::read_to_string(&canonical).await {
                Ok(c) => c,
                Err(e) => return error_value(&format!("failed to read file: {e}")),
            };

            let result = open_file(&filepath, &content, start_line, end_line);
            result_to_value(&result)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_content() -> String {
        (1..=30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn open_file_returns_requested_range() {
        let c = sample_content();
        let r = open_file("x.md", &c, Some(5), Some(10));
        assert_eq!(r.start_line, 5);
        assert_eq!(r.end_line, 10);
        assert_eq!(r.total_lines, 30);
        assert!(r.content.contains("5: line 5"));
        assert!(r.content.contains("10: line 10"));
        assert!(!r.content.contains("4: line 4"));
        assert!(!r.content.contains("11: line 11"));
        assert!(!r.truncated);
    }

    #[test]
    fn open_file_defaults_to_first_window() {
        let c = sample_content();
        let r = open_file("x.md", &c, None, None);
        assert_eq!(r.start_line, 1);
        assert!(r.end_line <= 30);
        assert!(r.content.starts_with("1: line 1"));
    }

    #[test]
    fn open_file_clamps_end_to_total_lines() {
        let c = sample_content();
        let r = open_file("x.md", &c, Some(25), Some(999));
        assert_eq!(r.start_line, 25);
        assert_eq!(r.end_line, 30);
        assert!(r.content.contains("30: line 30"));
    }

    #[test]
    fn open_file_caps_window_at_max_lines_per_open() {
        let big: String = (1..=10_000)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let r = open_file("x.md", &big, Some(1), Some(10_000));
        assert!(r.end_line - r.start_line + 1 <= MAX_LINES_PER_OPEN);
    }

    #[test]
    fn open_file_truncates_on_char_limit() {
        let long_line = "a".repeat(MAX_CONTENT_CHARS);
        let c = format!("{long_line}\n{long_line}\n{long_line}");
        let r = open_file("x.md", &c, Some(1), Some(3));
        assert!(r.truncated);
        assert!(r.content.contains("[truncated at"));
    }

    #[test]
    fn allowed_extension_accepts_md_and_txt_case_insensitive() {
        assert!(has_allowed_extension(Path::new("/tmp/foo.md")));
        assert!(has_allowed_extension(Path::new("/tmp/foo.MD")));
        assert!(has_allowed_extension(Path::new("/tmp/foo.txt")));
        assert!(has_allowed_extension(Path::new("/tmp/foo.TXT")));
    }

    #[test]
    fn allowed_extension_rejects_others() {
        assert!(!has_allowed_extension(Path::new("/etc/passwd")));
        assert!(!has_allowed_extension(Path::new("/tmp/foo.pdf")));
        assert!(!has_allowed_extension(Path::new("/tmp/no_ext")));
    }

    /// Per-test scratch dir under `std::env::temp_dir()`. No external tempfile dep.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("chat_agent_open_{tag}_{nanos}"));
            std::fs::create_dir_all(&path).expect("create_dir_all");
            Self { path }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn args_value(pairs: &[(&str, Value)]) -> Value {
        Value::object(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())))
    }

    #[tokio::test]
    async fn tool_reads_md_file() {
        let scratch = Scratch::new("reads_md");
        let path = scratch.path.join("sample.md");
        tokio::fs::write(&path, sample_content().into_bytes())
            .await
            .unwrap();

        let (name, runtime) = build_open_file_tool().expect("tool");
        assert_eq!(name, OPEN_FILE_TOOL);

        let args = args_value(&[
            ("filepath", Value::string(path.to_string_lossy().to_string())),
            ("start_line", Value::integer(2)),
            ("end_line", Value::integer(4)),
        ]);
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, args);
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        assert_eq!(
            value.pointer("/start_line").and_then(|v| v.as_integer()),
            Some(2)
        );
        assert_eq!(
            value.pointer("/end_line").and_then(|v| v.as_integer()),
            Some(4)
        );
        let content = value
            .pointer("/content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(content.contains("2: line 2"));
        assert!(content.contains("4: line 4"));
    }

    #[tokio::test]
    async fn tool_rejects_disallowed_extension() {
        let scratch = Scratch::new("reject_ext");
        let path = scratch.path.join("secret.pdf");
        tokio::fs::write(&path, b"ignored".to_vec()).await.unwrap();

        let (_, runtime) = build_open_file_tool().expect("tool");
        let args = args_value(&[(
            "filepath",
            Value::string(path.to_string_lossy().to_string()),
        )]);
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, args);
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        let err = value
            .pointer("/error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(err.contains("disallowed file extension"), "got: {err}");
    }

    #[tokio::test]
    async fn tool_reports_missing_file() {
        let (_, runtime) = build_open_file_tool().expect("tool");
        let args = args_value(&[(
            "filepath",
            Value::string("/tmp/definitely_missing_xyz_chat_agent_open.md"),
        )]);
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, args);
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        let err = value
            .pointer("/error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(err.contains("file not found or inaccessible"), "got: {err}");
    }

    #[tokio::test]
    async fn tool_rejects_path_outside_allowed_root() {
        let (_, runtime) = build_open_file_tool().expect("tool");
        let args = args_value(&[("filepath", Value::string("/etc/hosts"))]);
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, args);
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        let err = value
            .pointer("/error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            err.contains("path outside allowed directory")
                || err.contains("file not found or inaccessible"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn tool_rejects_symlink_pointing_outside_allowed_root() {
        let scratch = Scratch::new("reject_symlink");
        let link = scratch.path.join("link.md");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", &link).expect("symlink");
        #[cfg(not(unix))]
        return;

        let (_, runtime) = build_open_file_tool().expect("tool");
        let args = args_value(&[(
            "filepath",
            Value::string(link.to_string_lossy().to_string()),
        )]);
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, args);
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        let err = value
            .pointer("/error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            err.contains("path outside allowed directory"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn tool_reports_missing_filepath_argument() {
        let (_, runtime) = build_open_file_tool().expect("tool");
        let call = ailoy::message::Part::function(OPEN_FILE_TOOL, Value::object_empty());
        let msg = runtime.run(call).await.expect("tool run");
        let value = msg.contents[0].as_value().expect("value");
        assert_eq!(
            value.pointer("/error").and_then(|v| v.as_str()),
            Some("missing filepath")
        );
    }
}
