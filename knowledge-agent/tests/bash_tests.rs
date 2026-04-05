use knowledge_agent::validate_command;

// ── Whitelist passes ───────────────────────────────────────────────────

#[test]
fn whitelist_cat() {
    assert!(validate_command("cat foo.txt").is_ok());
}

#[test]
fn whitelist_grep() {
    assert!(validate_command("grep -r 'pattern' .").is_ok());
}

#[test]
fn whitelist_ls() {
    assert!(validate_command("ls -la").is_ok());
}

#[test]
fn whitelist_jq() {
    assert!(validate_command("cat data.json | jq '.name'").is_ok());
}

#[test]
fn whitelist_nl() {
    assert!(validate_command("nl -ba file.txt").is_ok());
}

#[test]
fn whitelist_iconv() {
    assert!(validate_command("iconv -f EUC-KR -t UTF-8 file.txt").is_ok());
}

#[test]
fn whitelist_strings() {
    assert!(validate_command("strings file.bin").is_ok());
}

#[test]
fn whitelist_md5sum() {
    assert!(validate_command("md5sum file.txt").is_ok());
}

#[test]
fn whitelist_sha256sum() {
    assert!(validate_command("sha256sum file.txt").is_ok());
}

#[test]
fn whitelist_pipe_chain() {
    assert!(validate_command("cat file.txt | grep pattern | sort | uniq").is_ok());
}

#[test]
fn whitelist_wc() {
    assert!(validate_command("wc -l *.md").is_ok());
}

#[test]
fn whitelist_diff() {
    assert!(validate_command("diff a.txt b.txt").is_ok());
}

#[test]
fn whitelist_date() {
    assert!(validate_command("date +%Y-%m-%d").is_ok());
}

// ── Blocked commands ───────────────────────────────────────────────────

#[test]
fn blocked_rm() {
    let result = validate_command("rm -rf /");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("rm"));
}

#[test]
fn blocked_mv() {
    assert!(validate_command("mv a.txt b.txt").is_err());
}

#[test]
fn blocked_cp() {
    assert!(validate_command("cp a.txt b.txt").is_err());
}

#[test]
fn blocked_chmod() {
    assert!(validate_command("chmod 777 file").is_err());
}

#[test]
fn blocked_sudo() {
    assert!(validate_command("sudo ls").is_err());
}

#[test]
fn blocked_shell_exec() {
    assert!(validate_command("bash -c 'echo hi'").is_err());
    assert!(validate_command("sh script.sh").is_err());
}

#[test]
fn blocked_pip() {
    assert!(validate_command("pip install requests").is_err());
}

#[test]
fn blocked_not_in_whitelist() {
    let result = validate_command("python3 script.py");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

// ── Grey zone: redirects ───────────────────────────────────────────────

#[test]
fn blocked_redirect_overwrite() {
    assert!(validate_command("echo hi > file.txt").is_err());
}

#[test]
fn blocked_redirect_append() {
    assert!(validate_command("echo hi >> file.txt").is_err());
}

#[test]
fn allowed_stderr_to_stdout() {
    // 2>&1 is the only allowed redirect form (merge stderr into stdout)
    assert!(validate_command("grep foo file.txt 2>&1").is_ok());
}

// ── Grey zone: pipe to shell ───────────────────────────────────────────

#[test]
fn blocked_pipe_to_sh() {
    assert!(validate_command("echo 'rm -rf /' | sh").is_err());
}

#[test]
fn blocked_pipe_to_bash() {
    assert!(validate_command("cat script.sh | bash").is_err());
}

// ── awk blocked (system() execution) ──────────────────────────────────

#[test]
fn blocked_awk() {
    // awk can call system() to execute arbitrary commands
    let result = validate_command("awk '{print $1}' file.txt");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

// ── Removed commands (not relevant to document analysis) ──────────────

#[test]
fn blocked_curl() {
    let result = validate_command("curl https://example.com");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

#[test]
fn blocked_ping() {
    assert!(validate_command("ping example.com").is_err());
}

#[test]
fn blocked_network_diagnostics() {
    assert!(validate_command("dig example.com").is_err());
    assert!(validate_command("nslookup example.com").is_err());
    assert!(validate_command("host example.com").is_err());
}

#[test]
fn blocked_system_discovery() {
    assert!(validate_command("locate foo").is_err());
    assert!(validate_command("which python").is_err());
    assert!(validate_command("whereis bash").is_err());
}

#[test]
fn blocked_system_info() {
    assert!(validate_command("whoami").is_err());
    assert!(validate_command("hostname").is_err());
    assert!(validate_command("uptime").is_err());
    assert!(validate_command("uname -a").is_err());
    assert!(validate_command("cal").is_err());
}

#[test]
fn blocked_less() {
    assert!(validate_command("less file.txt").is_err());
}

#[test]
fn blocked_df() {
    assert!(validate_command("df -h").is_err());
}

// ── find -delete / -execdir ────────────────────────────────────────────

#[test]
fn blocked_find_delete() {
    assert!(validate_command("find . -name '*.tmp' -delete").is_err());
}

#[test]
fn blocked_find_execdir() {
    assert!(validate_command("find . -type f -execdir cat {} \\;").is_err());
}

// ── sed /e flag ────────────────────────────────────────────────────────

#[test]
fn blocked_sed_exec_flag() {
    assert!(validate_command("sed 's/foo/ls/e' file.txt").is_err());
    assert!(validate_command("sed 's/foo/ls/ge' file.txt").is_err());
}

#[test]
fn allowed_sed_without_exec_flag() {
    assert!(validate_command("sed 's/foo/e/g' file.txt").is_ok());
    assert!(validate_command("sed 's/foo/bar/g' file.txt").is_ok());
}

// ── Grey zone: dangerous flags ─────────────────────────────────────────

#[test]
fn blocked_sed_in_place() {
    assert!(validate_command("sed -i 's/foo/bar/' file.txt").is_err());
}

#[test]
fn allowed_sed_stdout() {
    assert!(validate_command("sed 's/foo/bar/' file.txt").is_ok());
}

#[test]
fn blocked_tar_extract() {
    assert!(validate_command("tar -xf archive.tar").is_err());
}

#[test]
fn blocked_tar_bundled_extract() {
    // Bundled flags containing x must also be blocked
    assert!(validate_command("tar -txf archive.tar").is_err());
    assert!(validate_command("tar -xvf archive.tar").is_err());
}

#[test]
fn allowed_tar_list() {
    // -t (list) and -f (file) without x or c should be allowed
    assert!(validate_command("tar -tf archive.tar").is_ok());
    assert!(validate_command("tar -tvf archive.tar").is_ok());
}

#[test]
fn blocked_tar_create() {
    assert!(validate_command("tar -cf archive.tar dir/").is_err());
}

#[test]
fn blocked_unzip_extract() {
    assert!(validate_command("unzip archive.zip").is_err());
}

#[test]
fn allowed_unzip_list() {
    assert!(validate_command("unzip -l archive.zip").is_ok());
}


// ── Grey zone: dangerous compositions ──────────────────────────────────

#[test]
fn blocked_xargs_rm() {
    assert!(validate_command("find . -name '*.tmp' | xargs rm").is_err());
}

#[test]
fn blocked_find_exec_rm() {
    assert!(validate_command("find . -name '*.tmp' -exec rm {} \\;").is_err());
}

#[test]
fn blocked_tee() {
    assert!(validate_command("echo hi | tee file.txt").is_err());
}

// ── Command chaining bypass ─────────────────────────────────────────────

#[test]
fn blocked_semicolon_chaining() {
    assert!(validate_command("ls; rm -rf /").is_err());
}

#[test]
fn blocked_and_chaining() {
    assert!(validate_command("echo hi && rm foo").is_err());
}

#[test]
fn blocked_or_chaining() {
    assert!(validate_command("ls || rm foo").is_err());
}

// ── Subshell bypass ────────────────────────────────────────────────────

#[test]
fn blocked_subshell_dollar() {
    assert!(validate_command("echo $(rm foo)").is_err());
}

#[test]
fn blocked_subshell_backtick() {
    assert!(validate_command("echo `rm foo`").is_err());
}

// ── env bypass ─────────────────────────────────────────────────────────

#[test]
fn blocked_env_command() {
    assert!(validate_command("env rm -rf /").is_err());
}

// ── stderr redirect bypass ──────────────────────────────────────────────

#[test]
fn blocked_stderr_redirect() {
    assert!(validate_command("echo foo 2>/tmp/leak").is_err());
}

#[test]
fn blocked_stderr_append() {
    assert!(validate_command("echo foo 2>>/tmp/leak").is_err());
}

// ── wget/printenv/sleep removed ────────────────────────────────────────

#[test]
fn blocked_wget() {
    let result = validate_command("wget https://example.com");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

#[test]
fn blocked_sleep() {
    let result = validate_command("sleep 100");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

#[test]
fn blocked_printenv() {
    let result = validate_command("printenv");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the whitelist"));
}

// ── Edge cases ─────────────────────────────────────────────────────────

#[test]
fn empty_command() {
    assert!(validate_command("").is_err());
    assert!(validate_command("   ").is_err());
}

// ── Quoted-string false-positive tests ────────────────────────────────

#[test]
fn quoted_semicolon_allowed() {
    // Semicolon inside a quoted argument should not be treated as chaining
    assert!(validate_command(r#"echo "hello; world""#).is_ok());
}

#[test]
fn quoted_ampersand_allowed() {
    // && inside a quoted argument should not be treated as chaining
    assert!(validate_command(r#"grep "foo && bar" file.txt"#).is_ok());
}

#[test]
fn quoted_redirect_allowed() {
    // > inside a quoted argument should not be treated as a redirect
    assert!(validate_command(r#"echo "a > b""#).is_ok());
}

#[test]
fn quoted_pipe_to_shell_allowed() {
    // "| bash" inside a quoted argument should not trigger pipe-to-shell block
    assert!(validate_command(r#"grep "x | bash" file.txt"#).is_ok());
}

#[test]
fn unquoted_semicolon_blocked() {
    // Semicolon outside quotes must still be blocked
    assert!(validate_command("echo hello; echo world").is_err());
}

// ── Async execution tests ──────────────────────────────────────────────

#[tokio::test]
async fn run_echo() {
    let result = knowledge_agent::run_bash("echo hello", 5000, None).await;
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim(), "hello");
    assert!(!result.timed_out);
}

#[tokio::test]
async fn run_blocked_returns_error() {
    let result = knowledge_agent::run_bash("rm -rf /", 5000, None).await;
    assert_eq!(result.exit_code, -1);
    assert!(result.stderr.contains("BLOCKED"));
}

#[tokio::test]
async fn run_timeout() {
    // `tail -f /dev/null` blocks indefinitely and is in the whitelist
    let result = knowledge_agent::run_bash("tail -f /dev/null", 1000, None).await;
    assert!(result.timed_out);
}

#[tokio::test]
async fn run_nonzero_exit_code() {
    let result = knowledge_agent::run_bash("ls /nonexistent_path_xyz", 5000, None).await;
    assert_ne!(result.exit_code, 0);
    assert!(!result.timed_out);
}

#[tokio::test]
async fn run_output_truncation() {
    // Generate output larger than MAX_OUTPUT_CHARS (8000)
    let result = knowledge_agent::run_bash("seq 1 10000", 10000, None).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("[truncated at 8000 chars]"));
}
