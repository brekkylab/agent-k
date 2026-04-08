# chat-agent

`chat-agent` is an agent that engages in conversation with the user, makes decisions such as referencing various documents through a knowledge agent, and ultimately provides the response the user wants.
`chat-agent` is an agent and library built using ailoy, designed with the premise that it will be used in the backend.

## What It Does

- Creates a chat runtime from `ailoy::AgentSpec` and `ailoy::AgentProvider`
- Runs one user turn (`text` input) and returns assistant text output
- Includes two default built-in tools (`utc_now`, `add_integers`) that can be replaced later

## Public API

- `ChatAgent::new(spec, provider) -> ChatAgent`
- `ChatAgent::run_user_text(content) -> Result<String, ChatAgentRunError>`
- `ChatAgentRunError`
  - `Runtime { source }`: language model/runtime execution failed
  - `NoTextContent`: model response did not contain text parts

## Default Built-in Tools

`ChatAgent::new(spec, provider)` always injects and activates these tools:

1. `utc_now`
- Input: any object (arguments are ignored)
- Output: `{ "unix_seconds": <u64> }`

2. `add_integers`
- Input: `{ "a": integer, "b": integer }`
- Output (success): `{ "sum": <i64> }`
- Output (invalid args): `{ "error": "invalid_arguments" }`
- Output (overflow): `{ "error": "overflow" }`

Notes:
- If `spec.tools` does not include these names, `ChatAgent` appends them automatically.
- If a name already exists in `spec.tools`, it is not duplicated.

## Example

```rust
use chat_agent::ChatAgent;
use ailoy::{AgentProvider, AgentSpec, LangModelAPISchema, LangModelProvider};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let spec = AgentSpec {
        lm: "gpt-4.1-mini".to_string(),
        instruction: None,
        tools: vec![],
    };

    let provider = AgentProvider {
        lm: LangModelProvider::openai("YOUR_API_KEY".to_string()),
        tools: vec![],
    };

    let mut agent = ChatAgent::new(spec, provider);
    let answer = agent.run_user_text("Hello").await?;
    println!("{answer}");

    Ok(())
}
```

## Knowledge Sub-Agent

`ChatAgent` integrates a knowledge sub-agent via the `ask_knowledge` tool.
When the LLM calls this tool, a **short-lived sub-agent** is spawned per invocation:

```
ChatAgent (parent, LLM)
  └─ ask_knowledge(kb_id, question)
       └─ knowledge sub-agent (create → ReAct search loop → drop)
            └─ tantivy full-text search index
```

Each sub-agent is stateless — created, runs a single query, and immediately dropped.

### Setup

#### 1. Download data from S3

Corpus, indexes, and config are stored in Amazon S3. Run the setup script:

```bash
./scripts/setup-data.sh
```

This downloads into `backend/data/`:

```
backend/data/
  knowledge_agents.json      ← KB config
  corpus/
    finance/                 ← SEC 10-K filing .md files
    novel/                   ← NovelQA .txt files
  index/
    finance/                 ← tantivy index
    novel/                   ← tantivy index
```

Requires AWS CLI configured with access to `s3://ne-rag-dataset`.

#### 2. (Optional) Rebuild indexes

If you need to re-index from scratch, use the [agentmaker](https://github.com/brekkylab/agentmaker) `knowledge-agent` CLI:

```bash
cargo run --manifest-path knowledge-agent/Cargo.toml -- \
  --index-dir ./backend/data/index/finance \
  --reindex --index-only \
  --target-paths ./backend/data/corpus/finance

cargo run --manifest-path knowledge-agent/Cargo.toml -- \
  --index-dir ./backend/data/index/novel \
  --reindex --index-only \
  --target-paths ./backend/data/corpus/novel
```

#### 3. Configure knowledge bases

`backend/data/knowledge_agents.json` is downloaded by the setup script. Paths are resolved relative to the JSON file location.

Override the config path via environment variable:

```bash
KNOWLEDGE_AGENTS_CONFIG=/path/to/knowledge_agents.json
```

#### 4. Private repo authentication

`knowledge-agent` is in a private repo. `.cargo/config.toml` includes:

```toml
[net]
git-fetch-with-cli = true
```

Ensure your local git has GitHub authentication (SSH key or credential helper).

### Public API (Knowledge)

- `ChatAgent::tool_call_log() -> &[ToolCallEntry]`
  - Returns tool call/result pairs collected during `run_user_text()`
  - `ToolCallEntry { tool: String, args: serde_json::Value, result: Option<serde_json::Value> }`
  - Useful for verifying routing (e.g. which `kb_id` was selected)

### Routing Tests

Integration tests verify the LLM routes queries to the correct knowledge base:

```bash
# Unit tests only (no API call)
cargo test --manifest-path chat-agent/Cargo.toml

# Routing integration tests (requires OPENAI_API_KEY + indexes)
cargo test --manifest-path chat-agent/Cargo.toml --test routing_test -- --ignored --nocapture
```

| Test | Query | Expected kb_id |
|------|-------|---------------|
| `routes_revenue_question_to_finance` | "What was Apple's total revenue in 2022?" | finance |
| `routes_expense_question_to_finance` | "How much did Amazon spend on R&D in 2021?" | finance |
| `routes_profit_question_to_finance` | "What was Microsoft's operating profit margin in 2020?" | finance |
| `routes_character_question_to_novel` | "Who is the protagonist of Pride and Prejudice?" | novel |
| `routes_theme_question_to_novel` | "What is the main theme of Anna Karenina?" | novel |
| `routes_plot_question_to_novel` | "How does Wuthering Heights end?" | novel |

## Current Scope

- Runtime creation and one-turn execution wrapper only
- Session storage, cache policy, and HTTP API mapping are out of scope

## Development

```bash
cargo check --manifest-path chat-agent/Cargo.toml
cargo test --manifest-path chat-agent/Cargo.toml
```
