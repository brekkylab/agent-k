# knowledge-agent

A RAG agent that indexes/searches documents and autonomously finds answers using an 8-tool chain with LLM.
Validated on two benchmarks: NovelQA (novels) and FinanceBench (SEC financial filings).

## Stack

- **Rust** + **Tantivy** (BM25 full-text search)
- **ailoy** — Tool/Agent framework (LLM integration)
- **globset** + **ignore** — filename pattern matching
- Search is fully local; E2E tests use OpenAI API

---

## Architecture

```
.txt / .md files → unified indexing (indexer) → BM25 search (SearchIndex)
                                                    ↓
                                             ailoy Tool (8 tools)
                                    ┌─ Discovery ──────────────────────────┐
                                    │  glob_document    ← filename glob    │
                                    │  search_document  ← BM25 search     │
                                    ├─ Inspection ─────────────────────────┤
                                    │  find_in_document ← pattern matching │
                                    │  open_document    ← line range read  │
                                    │  summarize_document ← chunk summary  │
                                    ├─ Computation ────────────────────────┤
                                    │  calculate        ← math expression │
                                    │  run_python       ← Python sandbox  │
                                    │  run_bash         ← shell (readonly)│
                                    └──────────────────────────────────────┘
                                                    ↓
                                        runner.rs: run_with_trace()
                                        stream_turn + step tracing + retry
                                                    ↓
                                             ReAct E2E Q&A
```

---

## Tools (8)

### Discovery tools

#### 1. `glob_document`
Filename glob pattern matching. Walks corpus directories and returns files matching the pattern.
- Spaces → `*`, apostrophes → `*` auto-substitution
- Case insensitive, respects `.gitignore`

#### 2. `search_document`
BM25 full-text search. Returns documents ranked by relevance score.

### Inspection tools

#### 3. `find_in_document`
In-document pattern matching. Two modes:
- **Regex mode**: when query contains `|`, treated as single regex (e.g. `"cost of goods sold|COGS"`)
- **Keyword mode**: whitespace split → AND→half→OR progressive fallback

Returns matched line with position (line:col) and context. Supports cursor pagination.

#### 4. `open_document`
Line range reading. Truncates when exceeding max_content_chars.

#### 5. `summarize_document`
Summarize a document via map-reduce: split into chunks, summarize each in parallel, then reduce.
- Parallel chunk processing with `buffer_unordered(5)` rate limiting
- Best-effort: partial results returned even if some chunks fail
- Single-pass for documents under 4000 lines; chunked for larger
- Optional `focus` parameter to guide the summary topic
- Configurable `max_length` (default 500 chars)

### Computation tools

#### 6. `calculate`
Pure Rust math expression evaluator. No external process.
- Operators: `+`, `-`, `*`, `/`, `%`, `^`
- Functions: `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `log` (1-arg=ln, 2-arg=custom base), `log2`, `log10`, `ln`, `exp`, `ceil`, `floor`, `round`, `trunc`, `sign`, `min`, `max`, `pow`, `hypot`, `gcd`, `lcm`, `factorial`, `degrees`, `radians`
- Constants: `pi`, `e`

#### 7. `run_python`
Write and execute Python code in a sandboxed tmpdir.
- **Allowed modules**: math, statistics, decimal, fractions, re, string, textwrap, difflib, json, csv, collections, itertools, functools, datetime, time, hashlib, base64, pprint, operator, io (StringIO/BytesIO only)
- **Blocked**: os, sys, subprocess, shutil, pathlib, socket, http, requests, urllib, and all network/file I/O
- **Blocked builtins**: `open()`, `exec()`, `eval()`, `compile()`, `__import__()`, `time.sleep()`, sandbox escape patterns (`__subclasses__`, `__class__`, `__mro__`, etc.)
- Memory limit: 512 MB via `resource.setrlimit(RLIMIT_AS)`
- Configurable timeout (default 30s)

#### 8. `run_bash`
Execute read-only shell commands with 3-layer security:

**Layer 1 — Command whitelist (45 commands)**: cat, head, tail, nl, wc, file, stat, grep, rg, find, ls, tree, pwd, du, sed, cut, sort, uniq, tr, paste, column, fmt, fold, rev, diff, comm, cmp, iconv, strings, jq, yq, csvtool, xmllint, md5sum, sha256sum, echo, printf, bc, expr, seq, date, true, false, test, xargs, tar, zcat, zgrep, unzip

**Layer 2 — Flag/composition validation**: blocks `sed -i`, `sed /e`, `tar -x/-c`, `unzip` (without `-l`), `> / >>` redirects, `| sh`, `xargs rm`, `find -exec rm`, `find -delete/-execdir`, `tee`, etc. Quoted strings are stripped before metachar checks to prevent false positives.

**Layer 3 — Runtime protection**: tmpdir execution, read-only filesystem permissions on knowledge base, child process kill on timeout.

### Security policy

All tools enforce **read-only access** to the knowledge base:
- Original source files, `.md` documents, and Tantivy indexes are never modified
- `run_bash` and `run_python` are sandboxed with whitelist validation
- `calculate` is a pure function with no I/O

---

## LLM Flow

```
Question: "What is 3M's 2018 capital expenditure?"

① glob("*3M*2018*")
   → { matches: [{ filepath: "3M_2018_10K.md" }] }

② find(filepath="3M_2018_10K.md", query="capital expenditures|purchases of property")
   → { matches: [{ start: {line:2032}, line_content: "Purchases of PP&E | (1,577)" }] }

③ open(filepath="3M_2018_10K.md", start_line=2025, end_line=2040)
   → { content: "2025: ...\n2026: ...\n..." }

④ calculate("1577 * 1.0")
   → { result: 1577.0 }

⑤ Answer: "$1,577 million"
```

---

## Input Data

### NovelQA (novels)

Paths configured in `settings.json`:

```json
{
  "data": {
    "txt_dir": "../novelqa_downloader/books",
    "qa_file": "../novelqa_downloader/novelqa_merged.json"
  }
}
```

### FinanceBench (financial filings)

368 SEC filing PDFs converted to Markdown via Docling.
Path: `data/financebench/`

QA data: [PatronusAI/financebench](https://huggingface.co/datasets/PatronusAI/financebench) (150 questions)

### Tantivy Schema

| Field | Type | Purpose |
|-------|------|---------|
| `filepath` | `STRING \| STORED` | relative file path |
| `content` | `TEXT \| STORED` | BM25 searchable body |

---

## Configuration

```json
{
  "data": {
    "corpus_dirs": ["../novelqa_downloader/books", "data/financebench"],
    "txt_dir": "../novelqa_downloader/books",
    "qa_file": "../novelqa_downloader/novelqa_merged.json",
    "index_dir": "./tantivy_index"
  },
  "tools": {
    "top_k_max": 10,
    "max_matches": 20,
    "max_content_chars": 8000,
    "max_lines_per_open": 200
  }
}
```

---

## File Structure

```
knowledge-agent/
├── Cargo.toml
├── settings.json
├── src/
│   ├── lib.rs
│   ├── indexer.rs               # unified indexer (.txt + .md)
│   ├── agent/
│   │   ├── builder.rs           # build_agent()
│   │   ├── config.rs            # AgentConfig, system prompt
│   │   ├── runner.rs            # run_with_trace() — execution + retry
│   │   └── tracer.rs            # Step enum, tool tracing, infer_tool_name()
│   ├── tools/
│   │   ├── mod.rs               # ToolConfig, build_tool_set() (8 tools)
│   │   ├── common.rs            # parameter extraction helpers
│   │   ├── glob.rs              # glob_document
│   │   ├── search.rs            # search_document (BM25)
│   │   ├── find.rs              # find_in_document
│   │   ├── open.rs              # open_document
│   │   ├── summarize.rs         # summarize_document
│   │   ├── calculate.rs         # calculate
│   │   ├── python.rs            # run_python
│   │   └── bash.rs              # run_bash
│   ├── tui/
│   │   ├── mod.rs               # REPL loop
│   │   └── app.rs               # AppConfig
│   └── main.rs                  # CLI entry point
└── tests/
    ├── bash_tests.rs            # whitelist/greyzone/timeout
    ├── python_tests.rs          # module whitelist/sandbox
    ├── calculator_tests.rs      # expression evaluation
    ├── summarize_tests.rs       # config smoke test
    ├── find_open_tests.rs       # find/open unit + integration
    ├── find_comparison_test.rs  # find regex behavior
    ├── search_tests.rs          # SearchIndex + ailoy Tool
    └── e2e_react_test.rs        # ReAct E2E benchmark
```

---

## Build & Run

```bash
# Build
cargo build -p knowledge-agent

# Index only
cargo run -p knowledge-agent -- --index-only

# Unit tests (new tools)
cargo test --test bash_tests -- --nocapture
cargo test --test python_tests -- --nocapture
cargo test --test calculator_tests -- --nocapture

# Unit tests (existing)
cargo test --test find_open_tests -- --nocapture
cargo test --test search_tests -- --nocapture

# E2E ReAct tests (requires OPENAI_API_KEY)
cargo test --test e2e_react_test test_e2e_react_financebench -- --ignored --nocapture
cargo test --test e2e_react_test test_e2e_react_novelqa -- --ignored --nocapture
```
