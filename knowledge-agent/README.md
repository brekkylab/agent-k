# knowledge-agent

A RAG agent that indexes/searches documents and autonomously finds answers using a 5-tool chain with LLM.
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
                                             ailoy Tool (5 tools)
                                    ┌─ Discovery ──────────────────────────┐
                                    │  glob_document    ← filename glob    │
                                    │  search_document  ← BM25 search     │
                                    ├─ Inspection ─────────────────────────┤
                                    │  find_in_document ← pattern matching │
                                    │  open_document    ← line range read  │
                                    ├─ Computation ────────────────────────┤
                                    │  calculate        ← math expression │
                                    └──────────────────────────────────────┘
                                                    ↓
                                        runner.rs: run_with_trace()
                                        stream_turn + step tracing + retry
                                                    ↓
                                             ReAct E2E Q&A
```

---

## Tools (5)

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

### Computation tools

#### 5. `calculate`
Pure Rust math expression evaluator. No external process.
- Operators: `+`, `-`, `*`, `/`, `%`, `^`
- Functions: `sqrt`, `abs`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `log` (1-arg=ln, 2-arg=custom base), `log2`, `log10`, `ln`, `exp`, `ceil`, `floor`, `round`, `trunc`, `sign`, `min`, `max`, `pow`, `hypot`, `gcd`, `lcm`, `factorial`, `degrees`, `radians`
- Constants: `pi`, `e`

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
│   │   ├── mod.rs               # ToolConfig, build_tool_set() (5 tools)
│   │   ├── common.rs            # parameter extraction helpers
│   │   ├── glob.rs              # glob_document
│   │   ├── search.rs            # search_document (BM25)
│   │   ├── find.rs              # find_in_document
│   │   ├── open.rs              # open_document
│   │   └── calculate.rs         # calculate
│   ├── tui/
│   │   ├── mod.rs               # REPL loop
│   │   └── app.rs               # AppConfig
│   └── main.rs                  # CLI entry point
└── tests/
    ├── calculator_tests.rs      # expression evaluation
    ├── find_open_tests.rs       # find/open unit + integration
    ├── find_comparison_test.rs  # find regex behavior
    ├── search_tests.rs          # SearchIndex + ailoy Tool
    └── e2e_react_test.rs        # ReAct E2E benchmark
```

---

## Build & Run

```bash
# Build (from repo root)
cargo build --manifest-path knowledge-agent/Cargo.toml

# Index only (from repo root)
cargo run --manifest-path knowledge-agent/Cargo.toml -- --index-only

# Unit tests (from repo root)
cargo test --manifest-path knowledge-agent/Cargo.toml --test calculator_tests -- --nocapture
cargo test --manifest-path knowledge-agent/Cargo.toml --test find_open_tests -- --nocapture
cargo test --manifest-path knowledge-agent/Cargo.toml --test search_tests -- --nocapture

# E2E ReAct tests (requires OPENAI_API_KEY, from repo root)
cargo test --manifest-path knowledge-agent/Cargo.toml --test e2e_react_test test_e2e_react_financebench -- --ignored --nocapture
cargo test --manifest-path knowledge-agent/Cargo.toml --test e2e_react_test test_e2e_react_novelqa -- --ignored --nocapture
```
