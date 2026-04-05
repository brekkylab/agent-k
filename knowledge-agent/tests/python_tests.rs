use knowledge_agent::validate_python_code;

// ── Allowed modules ────────────────────────────────────────────────────

#[test]
fn allowed_math() {
    assert!(validate_python_code("import math\nprint(math.sqrt(2))").is_ok());
}

#[test]
fn allowed_json() {
    assert!(validate_python_code("import json\nprint(json.dumps({'a': 1}))").is_ok());
}

#[test]
fn allowed_datetime() {
    assert!(validate_python_code("from datetime import datetime\nprint(datetime.now())").is_ok());
}

#[test]
fn allowed_collections() {
    assert!(validate_python_code("from collections import Counter\nprint(Counter([1,2,2,3]))").is_ok());
}

#[test]
fn allowed_re() {
    assert!(validate_python_code("import re\nprint(re.findall(r'\\d+', 'abc123'))").is_ok());
}

#[test]
fn allowed_no_imports() {
    assert!(validate_python_code("print(1 + 2)").is_ok());
}

// ── Blocked modules ────────────────────────────────────────────────────

#[test]
fn blocked_os() {
    let result = validate_python_code("import os\nos.system('ls')");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("os"));
}

#[test]
fn blocked_subprocess() {
    assert!(validate_python_code("import subprocess").is_err());
}

#[test]
fn blocked_sys() {
    assert!(validate_python_code("import sys").is_err());
}

#[test]
fn blocked_shutil() {
    assert!(validate_python_code("import shutil").is_err());
}

#[test]
fn blocked_socket() {
    assert!(validate_python_code("import socket").is_err());
}

#[test]
fn blocked_requests() {
    assert!(validate_python_code("import requests").is_err());
}

#[test]
fn blocked_pathlib() {
    assert!(validate_python_code("from pathlib import Path").is_err());
}

#[test]
fn blocked_http() {
    assert!(validate_python_code("from http.server import HTTPServer").is_err());
}

#[test]
fn blocked_urllib() {
    assert!(validate_python_code("from urllib.request import urlopen").is_err());
}

#[test]
fn blocked_not_in_allowed() {
    let result = validate_python_code("import numpy");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not in the allowed list"));
}

#[test]
fn blocked_removed_modules() {
    // These were removed: not needed for document analysis
    assert!(validate_python_code("import random").is_err());
    assert!(validate_python_code("import calendar").is_err());
    assert!(validate_python_code("import enum").is_err());
    assert!(validate_python_code("import dataclasses").is_err());
    assert!(validate_python_code("import copy").is_err());
}

// ── Blocked builtins ───────────────────────────────────────────────────

#[test]
fn blocked_dunder_import() {
    assert!(validate_python_code("__import__('os')").is_err());
}

#[test]
fn blocked_exec() {
    assert!(validate_python_code("exec('print(1)')").is_err());
}

#[test]
fn blocked_open() {
    assert!(validate_python_code("f = open('file.txt')").is_err());
}

// ── Dynamic execution bypass ───────────────────────────────────────────

#[test]
fn blocked_eval() {
    assert!(validate_python_code("eval('1+1')").is_err());
}

#[test]
fn blocked_eval_import() {
    assert!(validate_python_code(r#"eval("__import__('os').system('ls')")"#).is_err());
}

#[test]
fn blocked_compile() {
    assert!(validate_python_code(r#"compile("import os", "", "exec")"#).is_err());
}

#[test]
fn blocked_builtins_via_getattr() {
    // Blocked due to __builtins__ (not getattr — getattr itself is now allowed)
    assert!(validate_python_code(r#"getattr(__builtins__, '__import__')('os')"#).is_err());
}

#[test]
fn allowed_getattr() {
    // Normal attribute access via getattr should be allowed
    assert!(validate_python_code("class Foo:\n    x = 1\nprint(getattr(Foo(), 'x'))").is_ok());
}

#[test]
fn allowed_dict_access() {
    // __dict__ is allowed; class hierarchy escape is still blocked via __class__, __base__, etc.
    assert!(validate_python_code("d = {'a': 1}\nprint(d.__class__.__name__)").is_err()); // __class__ still blocked
    assert!(validate_python_code("d = {'a': 1}\nprint(d)").is_ok());
}

#[test]
fn blocked_globals() {
    assert!(validate_python_code("globals()['__builtins__']").is_err());
}

#[test]
fn blocked_builtins_access() {
    assert!(validate_python_code("__builtins__.__import__('os')").is_err());
}

#[test]
fn blocked_setattr() {
    assert!(validate_python_code("setattr(obj, 'x', 1)").is_err());
}

#[test]
fn blocked_vars() {
    assert!(validate_python_code("vars()").is_err());
}

// ── Sandbox escape via class hierarchy ──────────────────────────────────

#[test]
fn blocked_subclasses() {
    assert!(validate_python_code("().__class__.__base__.__subclasses__()").is_err());
}

#[test]
fn blocked_class_access() {
    assert!(validate_python_code("x.__class__").is_err());
}

#[test]
fn blocked_mro() {
    assert!(validate_python_code("int.__mro__").is_err());
}

// ── re.compile() should be allowed ─────────────────────────────────────

#[test]
fn allowed_re_compile() {
    assert!(validate_python_code("import re\npattern = re.compile(r'\\d+')").is_ok());
}

// ── Semicolon import bypass ────────────────────────────────────────────

#[test]
fn blocked_semicolon_import() {
    assert!(validate_python_code("import json; import os").is_err());
}

// ── io module: StringIO/BytesIO allowed, file I/O blocked ──────────────

#[test]
fn allowed_io_stringio() {
    assert!(validate_python_code("import io\nbuf = io.StringIO()\nbuf.write('hello')").is_ok());
    assert!(validate_python_code("from io import StringIO\nf = StringIO('data')").is_ok());
}

#[test]
fn blocked_io_file_access() {
    assert!(validate_python_code("import io\nio.open('file.txt')").is_err());
    assert!(validate_python_code("import io\nio.FileIO('file.txt')").is_err());
}

// ── Edge cases ─────────────────────────────────────────────────────────

#[test]
fn empty_code() {
    assert!(validate_python_code("").is_err());
    assert!(validate_python_code("   ").is_err());
}

// ── Async execution tests ──────────────────────────────────────────────

#[tokio::test]
async fn run_simple_print() {
    let result = knowledge_agent::run_python("print('hello')", 5000).await;
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim(), "hello");
    assert!(!result.timed_out);
}

#[tokio::test]
async fn run_math_calculation() {
    let result = knowledge_agent::run_python("import math\nprint(math.factorial(10))", 5000).await;
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim(), "3628800");
}

#[tokio::test]
async fn run_blocked_module_returns_error() {
    let result = knowledge_agent::run_python("import os\nos.listdir('.')", 5000).await;
    assert_eq!(result.exit_code, -1);
    assert!(result.stderr.contains("BLOCKED"));
}

#[tokio::test]
async fn run_timeout() {
    let result = knowledge_agent::run_python("while True: pass", 1000).await;
    assert!(result.timed_out);
}

#[tokio::test]
async fn blocked_time_sleep() {
    let result = knowledge_agent::run_python("import time\ntime.sleep(10)", 1000).await;
    assert_eq!(result.exit_code, -1);
    assert!(result.stderr.contains("BLOCKED"));
}

#[tokio::test]
async fn run_syntax_error() {
    let result = knowledge_agent::run_python("def foo(\nprint('unclosed')", 5000).await;
    assert_ne!(result.exit_code, 0);
    assert!(!result.timed_out);
    assert!(result.stderr.contains("SyntaxError") || result.stderr.contains("Error"));
}

#[tokio::test]
async fn run_output_truncation() {
    // Generate output larger than MAX_OUTPUT_CHARS (8000)
    let code = "print('x' * 10000)";
    let result = knowledge_agent::run_python(code, 5000).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("[truncated at 8000 chars]"));
}

#[tokio::test]
async fn run_multiline_logic() {
    let code = r#"
data = [3, 1, 4, 1, 5, 9, 2, 6]
data.sort()
print(data)
print(sum(data))
"#;
    let result = knowledge_agent::run_python(code, 5000).await;
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("[1, 1, 2, 3, 4, 5, 6, 9]"));
    assert!(result.stdout.contains("31"));
}
