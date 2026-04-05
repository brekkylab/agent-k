use std::sync::{Arc, LazyLock};
use std::time::Duration;

use ailoy::{ToolDescBuilder, ToolRuntime, Value, agent::ToolFunc};
use futures::future::BoxFuture;
use regex::Regex;
use serde::Serialize;
use serde_json::json;

use super::common::{extract_optional_i64, extract_required_str, result_to_value};

// ── Static regexes (compiled once) ─────────────────────────────────────

// Match `import X` and `import X, Y` but not `from X import Y`.
// We strip from-imports first, then scan for remaining `import` statements.
static RE_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|;|\s)import\s+([\w.]+(?:\s*,\s*[\w.]+)*)").unwrap());
// Used to strip `from X import Y` before RE_IMPORT scan
static RE_FROM_IMPORT_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfrom\s+[\w.]+\s+import\b[^\n;]*").unwrap());
static RE_FROM_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfrom\s+([\w.]+)\s+import\b").unwrap());

static RE_BLOCKED_BUILTINS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (Regex::new(r"__import__\s*\(").unwrap(), "__import__()"),
        (Regex::new(r"\bexec\s*\(").unwrap(), "exec()"),
        (Regex::new(r"\beval\s*\(").unwrap(), "eval()"),
        (Regex::new(r"(?:^|[^.\w])compile\s*\(").unwrap(), "compile()"),
        (Regex::new(r"\bopen\s*\(").unwrap(), "open() (no file access)"),
        (Regex::new(r"\bglobals\s*\(").unwrap(), "globals()"),
        (Regex::new(r"\bvars\s*\(").unwrap(), "vars()"),
        (Regex::new(r"\bsetattr\s*\(").unwrap(), "setattr()"),
        (Regex::new(r"\bdelattr\s*\(").unwrap(), "delattr()"),
        (Regex::new(r"\b__builtins__\b").unwrap(), "__builtins__ access"),
        (Regex::new(r"\bbreakpoint\s*\(").unwrap(), "breakpoint()"),
        // Sandbox escape via class hierarchy traversal
        (Regex::new(r"__subclasses__").unwrap(), "__subclasses__() (sandbox escape)"),
        (Regex::new(r"__class__").unwrap(), "__class__ access"),
        (Regex::new(r"__base__").unwrap(), "__base__ access"),
        (Regex::new(r"__mro__").unwrap(), "__mro__ access"),
        // io file access — StringIO/BytesIO allowed, but file-based I/O classes are not
        (Regex::new(r"\bio\.(open|FileIO|RawIOBase|BufferedReader|BufferedWriter|BufferedRandom|BufferedIOBase|IOBase|TextIOWrapper)\b").unwrap(),
         "io file I/O classes are not allowed (use io.StringIO or io.BytesIO)"),
        // time.sleep wastes resources (timeout catches it eventually, but block upfront)
        (Regex::new(r"\btime\.sleep\s*\(").unwrap(), "time.sleep()"),
    ]
});

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_OUTPUT_CHARS: usize = 8000;

/// Injected before user code to set a virtual-memory limit (512 MB).
/// Uses a private name (_r) and deletes it so user code cannot access the resource module.
/// Wrapped in try/except because RLIMIT_AS is advisory-only on macOS.
const MEMORY_LIMIT_PREAMBLE: &str = r#"try:
    import resource as _r
    _r.setrlimit(_r.RLIMIT_AS, (536_870_912, 536_870_912))
    del _r
except Exception:
    pass
"#;

#[derive(Debug, Clone, Serialize)]
pub struct PythonResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

// ── Module validation ──────────────────────────────────────────────────

const ALLOWED_MODULES: &[&str] = &[
    // Math
    "math", "statistics", "decimal", "fractions",
    // Text
    "re", "string", "textwrap", "difflib",
    // Data
    "json", "csv", "collections", "itertools", "functools",
    // Date/Time
    "datetime", "time",
    // Utility
    "hashlib", "base64", "pprint", "operator",
    // io: StringIO/BytesIO allowed; file I/O classes are blocked via RE_BLOCKED_BUILTINS
    "io",
];

const BLOCKED_MODULES: &[&str] = &[
    "os", "sys", "subprocess", "shutil", "pathlib",
    "socket", "http", "requests",
    "urllib", "ftplib", "smtplib", "imaplib",
    "ctypes", "importlib", "runpy", "code", "codeop",
    "signal", "multiprocessing", "threading",
    "pickle", "shelve", "marshal",
    "builtins", "__builtin__",
];

pub fn validate_python_code(code: &str) -> Result<(), String> {
    if code.trim().is_empty() {
        return Err("empty code".to_string());
    }

    // Extract all import statements
    let mut imports: Vec<String> = Vec::new();

    // 1. from X import Y — validate X (the module)
    for cap in RE_FROM_IMPORT.captures_iter(code) {
        imports.push(cap[1].to_string());
    }

    // 2. import X — strip from-imports first to avoid matching `from X import Y`'s `import`
    let code_without_from = RE_FROM_IMPORT_FULL.replace_all(code, "");
    for cap in RE_IMPORT.captures_iter(&code_without_from) {
        // Handle `import X, Y` by splitting on comma
        for module in cap[1].split(',') {
            let module = module.trim();
            if !module.is_empty() {
                imports.push(module.to_string());
            }
        }
    }

    for import in &imports {
        let top_module = import.split('.').next().unwrap_or(import);

        if BLOCKED_MODULES.contains(&top_module) {
            return Err(format!("module '{}' is not allowed", top_module));
        }

        if !ALLOWED_MODULES.contains(&top_module) {
            return Err(format!(
                "module '{}' is not in the allowed list",
                top_module
            ));
        }
    }

    // Block dangerous builtins and dynamic code execution
    for (re, name) in RE_BLOCKED_BUILTINS.iter() {
        if re.is_match(code) {
            return Err(format!("{} is not allowed", name));
        }
    }

    Ok(())
}

fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        return s.to_string();
    }
    let mut end = MAX_OUTPUT_CHARS;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[truncated at {} chars]",
        &s[..end],
        MAX_OUTPUT_CHARS
    )
}

/// Execute Python code in a 3-layer sandbox:
///
/// 1. **Static validation** (pre-execution): ALLOWED_MODULES whitelist +
///    RE_BLOCKED_BUILTINS pattern matching reject dangerous code before it runs.
/// 2. **Runtime isolation**: code runs in a tmpdir (auto-deleted after execution).
///    `open()` is blocked so the script cannot read/write paths outside the tmpdir.
/// 3. **Resource limits**: RLIMIT_AS 512 MB memory cap, configurable timeout,
///    child process killed on timeout.
pub async fn run_python(code: &str, timeout_ms: u64) -> PythonResult {
    if let Err(reason) = validate_python_code(code) {
        return PythonResult {
            stdout: String::new(),
            stderr: format!("BLOCKED: {}", reason),
            exit_code: -1,
            timed_out: false,
        };
    }

    let tmpdir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return PythonResult {
                stdout: String::new(),
                stderr: format!("failed to create tmpdir: {}", e),
                exit_code: -1,
                timed_out: false,
            };
        }
    };

    // Write code to a temp file only. tmpdir is dropped after execution,
    // which deletes the directory automatically. The sandbox also blocks
    // open() inside the script, so no other paths can be written or read.
    // Prepend a memory-limit preamble (512 MB cap via resource module).
    let script_path = tmpdir.path().join("script.py");
    let full_script = format!("{}{}", MEMORY_LIMIT_PREAMBLE, code);
    if let Err(e) = tokio::fs::write(&script_path, full_script).await {
        return PythonResult {
            stdout: String::new(),
            stderr: format!("failed to write script: {}", e),
            exit_code: -1,
            timed_out: false,
        };
    }

    let mut child = match tokio::process::Command::new("python3")
        .arg(&script_path)
        .current_dir(tmpdir.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return PythonResult {
                stdout: String::new(),
                stderr: format!("execution error: {}", e),
                exit_code: -1,
                timed_out: false,
            };
        }
    };

    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let stderr_pipe = child.stderr.take().expect("stderr piped");

    let out_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        tokio::io::BufReader::new(stdout_pipe).read_to_end(&mut buf).await?;
        Ok::<_, std::io::Error>(buf)
    });
    let err_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        tokio::io::BufReader::new(stderr_pipe).read_to_end(&mut buf).await?;
        Ok::<_, std::io::Error>(buf)
    });

    match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => {
            let stdout_bytes = out_task.await.unwrap_or(Ok(vec![])).unwrap_or_default();
            let stderr_bytes = err_task.await.unwrap_or(Ok(vec![])).unwrap_or_default();
            PythonResult {
                stdout: truncate_output(&String::from_utf8_lossy(&stdout_bytes)),
                stderr: truncate_output(&String::from_utf8_lossy(&stderr_bytes)),
                exit_code: status.code().unwrap_or(-1),
                timed_out: false,
            }
        }
        Ok(Err(e)) => {
            out_task.abort();
            err_task.abort();
            PythonResult {
                stdout: String::new(),
                stderr: format!("execution error: {}", e),
                exit_code: -1,
                timed_out: false,
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            out_task.abort();
            err_task.abort();
            PythonResult {
                stdout: String::new(),
                stderr: format!("script timed out after {}ms", timeout_ms),
                exit_code: -1,
                timed_out: true,
            }
        }
    }
}

pub fn build_run_python_tool() -> ToolRuntime {
    let desc = ToolDescBuilder::new("run_python")
        .description(
            "Write and execute Python code for data processing, complex calculations, or multi-step logic. \
             Only safe modules are allowed (math, json, re, datetime, collections, etc.). \
             File I/O, network access, and system calls are blocked.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code to execute (multi-line supported)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 30000)"
                }
            },
            "required": ["code"]
        }))
        .build();

    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        Box::pin(async move {
            let code = match extract_required_str(&args, "code") {
                Ok(c) => c,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let timeout = extract_optional_i64(&args, "timeout_ms")
                .map(|v| v.max(1000) as u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS);

            let result = run_python(&code, timeout).await;
            result_to_value(&result)
        })
    });

    ToolRuntime::new(desc, f)
}
