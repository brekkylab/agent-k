use std::sync::{Arc, LazyLock};
use std::time::Duration;

use ailoy::{ToolDescBuilder, ToolRuntime, Value, agent::ToolFunc};
use futures::future::BoxFuture;
use regex::Regex;
use serde::Serialize;
use serde_json::json;

use super::common::{extract_optional_i64, extract_required_str, result_to_value};

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_OUTPUT_CHARS: usize = 8000;

// ── Static regexes (compiled once) ─────────────────────────────────────

static RE_CHAINING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r";|&&|\|\|").unwrap());
static RE_SUBSHELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\(|`").unwrap());
// Block all > and >> redirects. 2>&1 is allowed via separate check.
static RE_REDIRECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r">>?").unwrap());
// Allow only 2>&1 (stderr to stdout merge)
static RE_REDIRECT_SAFE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"2>&1").unwrap());
static RE_PIPE_SHELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\|\s*(sh|bash|zsh|dash|exec)\b").unwrap());
static RE_CMD_BOUNDARY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\|)\s*(\S+)").unwrap());
static RE_SED_INPLACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsed\b.*\s-i\b").unwrap());
// Match tar with extract/create: handles both bundled flags (-txf, -xvf)
// and long options (--extract, --create).
static RE_TAR_XC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\btar\b.*(\s-[a-zA-Z]*[xc][a-zA-Z]*|--(extract|create))").unwrap());
static RE_UNZIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bunzip\b").unwrap());
static RE_UNZIP_LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bunzip\b.*\s-l\b").unwrap());
static RE_TEE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\btee\b").unwrap());

const BLOCKED_CMDS: &[&str] = &[
    "rm", "mv", "cp", "mkdir", "touch", "chmod", "chown", "ln",
    "kill", "pkill", "dd", "mkfs", "mount", "sudo", "su",
    "pip", "npm", "apt", "brew", "cargo",
    "sh", "bash", "zsh", "dash", "exec", "eval", "source",
    "env",
];

// Block `find -delete` and `find -execdir` which bypass the -exec check
static RE_FIND_DANGER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfind\b.*(-delete\b|-execdir\b)").unwrap());

// Block sed's /e flag which executes the substitution result as a shell command.
// Matches `/<flags>e<flags>` followed by a closing quote/whitespace at the end of a substitution.
static RE_SED_EXEC_FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bsed\b.*\/[gipqI]*e[gipqI]*['"\s;]"#).unwrap());

static RE_XARGS_DANGER: LazyLock<Regex> = LazyLock::new(|| {
    let pattern = BLOCKED_CMDS.join("|");
    Regex::new(&format!(r"\bxargs\b.*\b({})\b", pattern)).unwrap()
});
static RE_FIND_EXEC_DANGER: LazyLock<Regex> = LazyLock::new(|| {
    let pattern = BLOCKED_CMDS.join("|");
    Regex::new(&format!(r"-exec\s+.*\b({})\b", pattern)).unwrap()
});

#[derive(Debug, Clone, Serialize)]
pub struct BashResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

// ── Whitelist ──────────────────────────────────────────────────────────

const WHITELIST: &[&str] = &[
    // File read
    "cat", "head", "tail", "wc", "file", "stat", "nl",
    // Search
    "grep", "rg", "find",
    // Directory
    "ls", "tree", "pwd", "du",
    // Text processing — awk excluded: awk scripts can call system() to execute arbitrary commands
    "sed", "cut", "sort", "uniq", "tr", "paste", "column", "fmt", "fold", "rev",
    // Compare
    "diff", "comm", "cmp",
    // Encoding / binary
    "iconv", "strings",
    // JSON/Data
    "jq", "yq", "csvtool", "xmllint",
    // Hash (file dedup / integrity)
    "md5sum", "sha256sum",
    // Utility
    "echo", "printf", "bc", "expr", "seq", "date", "true", "false", "test", "xargs",
    // Archive (list-only — extract/create blocked via RE_TAR_XC)
    "tar", "zcat", "zgrep", "unzip",
];

// ── Quoted-string stripping ─────────────────────────────────────────────
// Strip content inside single/double quotes so that quoted arguments
// (e.g. echo "hello && world") don't trigger false-positive checks.
fn strip_quoted_strings(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                while let Some(c2) = chars.next() {
                    if c2 == '"' {
                        break;
                    }
                    if c2 == '\\' {
                        chars.next(); // skip escaped char
                    }
                }
            }
            '\'' => {
                while let Some(c2) = chars.next() {
                    if c2 == '\'' {
                        break;
                    }
                }
            }
            _ => result.push(c),
        }
    }
    result
}

// ── Validation ─────────────────────────────────────────────────────────

pub fn validate_command(cmd: &str) -> Result<(), String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err("empty command".to_string());
    }

    // Strip quoted strings for metacharacter checks to avoid false positives
    // (e.g. echo "hello && world" should not trigger the chaining check).
    let unquoted = strip_quoted_strings(trimmed);

    // 1. Block command chaining: ;, &&, ||
    if RE_CHAINING.is_match(&unquoted) {
        return Err("command chaining (;, &&, ||) not allowed".to_string());
    }

    // 2. Block subshell execution: $(...) and backticks
    if RE_SUBSHELL.is_match(&unquoted) {
        return Err("subshell execution ($() or backticks) not allowed".to_string());
    }

    // 3. Block file redirects: > and >> (but allow 2>&1)
    // Strip out all safe 2>&1 patterns, then check if any > remains
    let without_safe = RE_REDIRECT_SAFE.replace_all(&unquoted, "");
    if RE_REDIRECT.is_match(&without_safe) {
        return Err("file redirect (> or >>) not allowed".to_string());
    }

    // 4. Block pipe to shell
    if RE_PIPE_SHELL.is_match(&unquoted) {
        return Err("pipe to shell not allowed".to_string());
    }

    // 5. Block dangerous base commands (use unquoted so quoted args don't shadow real commands)
    for cap in RE_CMD_BOUNDARY.captures_iter(&unquoted) {
        let base = cap[1].split('/').last().unwrap_or(&cap[1]);
        if BLOCKED_CMDS.contains(&base) {
            return Err(format!("command '{}' is not allowed", base));
        }
        if !WHITELIST.contains(&base) {
            return Err(format!("command '{}' is not in the whitelist", base));
        }
    }

    // 6. Block dangerous flags
    if RE_SED_INPLACE.is_match(trimmed) {
        return Err("sed in-place edit (-i) not allowed".to_string());
    }
    if RE_SED_EXEC_FLAG.is_match(trimmed) {
        return Err("sed /e flag (execute substitution result) not allowed".to_string());
    }
    if RE_TAR_XC.is_match(trimmed) {
        return Err("tar extract/create not allowed (only tar -t for listing)".to_string());
    }
    if RE_UNZIP.is_match(trimmed) && !RE_UNZIP_LIST.is_match(trimmed) {
        return Err("unzip only allowed with -l (list)".to_string());
    }

    // 7. Block dangerous compositions: xargs/find -exec/-delete/-execdir with blocked commands
    if RE_XARGS_DANGER.is_match(trimmed) {
        return Err("dangerous xargs composition not allowed".to_string());
    }
    if RE_FIND_EXEC_DANGER.is_match(trimmed) {
        return Err("dangerous find -exec composition not allowed".to_string());
    }
    if RE_FIND_DANGER.is_match(trimmed) {
        return Err("find -delete and -execdir not allowed".to_string());
    }

    // 8. Block tee (writes to files)
    if RE_TEE.is_match(trimmed) {
        return Err("tee not allowed (writes to files)".to_string());
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

pub async fn run_bash(command: &str, timeout_ms: u64, working_dir: Option<&std::path::Path>) -> BashResult {
    if let Err(reason) = validate_command(command) {
        return BashResult {
            stdout: String::new(),
            stderr: format!("BLOCKED: {}", reason),
            exit_code: -1,
            timed_out: false,
        };
    }

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    let mut child = match cmd.spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return BashResult {
                stdout: String::new(),
                stderr: format!("execution error: {}", e),
                exit_code: -1,
                timed_out: false,
            };
        }
    };

    // Read stdout/stderr in background tasks so `child.wait()` (by &mut) remains
    // accessible for kill() on timeout. `wait_with_output()` takes ownership and
    // would prevent kill() from being called.
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
            BashResult {
                stdout: truncate_output(&String::from_utf8_lossy(&stdout_bytes)),
                stderr: truncate_output(&String::from_utf8_lossy(&stderr_bytes)),
                exit_code: status.code().unwrap_or(-1),
                timed_out: false,
            }
        }
        Ok(Err(e)) => {
            out_task.abort();
            err_task.abort();
            BashResult {
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
            BashResult {
                stdout: String::new(),
                stderr: format!("command timed out after {}ms", timeout_ms),
                exit_code: -1,
                timed_out: true,
            }
        }
    }
}

pub fn build_run_bash_tool(working_dir: Option<std::path::PathBuf>) -> ToolRuntime {
    let desc = ToolDescBuilder::new("run_bash")
        .description(
            "Execute a read-only shell command in the corpus directory. \
             Only whitelisted commands are allowed (cat, grep, ls, jq, etc.). \
             File writes, redirects, and destructive operations are blocked.",
        )
        .parameters(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute (read-only operations only)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default: 10000)"
                }
            },
            "required": ["command"]
        }))
        .build();

    let f: Arc<ToolFunc> = Arc::new(move |args: Value| -> BoxFuture<'static, Value> {
        let wd = working_dir.clone();
        Box::pin(async move {
            let command = match extract_required_str(&args, "command") {
                Ok(c) => c,
                Err(e) => return json!({ "error": e.to_string() }).into(),
            };
            let timeout = extract_optional_i64(&args, "timeout_ms")
                .map(|v| v.max(1000) as u64)
                .unwrap_or(DEFAULT_TIMEOUT_MS);

            let result = run_bash(&command, timeout, wd.as_deref()).await;
            result_to_value(&result)
        })
    });

    ToolRuntime::new(desc, f)
}
