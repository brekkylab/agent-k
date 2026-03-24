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
        lm: LangModelProvider::API {
            schema: LangModelAPISchema::ChatCompletion,
            url: "https://api.openai.com/v1/chat/completions".parse()?,
            api_key: Some("YOUR_API_KEY".to_string()),
        },
        tools: vec![],
    };

    let mut agent = ChatAgent::new(spec, provider);
    let answer = agent.run_user_text("Hello").await?;
    println!("{answer}");

    Ok(())
}
```

## Current Scope

- Runtime creation and one-turn execution wrapper only
- Session storage, cache policy, and HTTP API mapping are out of scope

## Development

```bash
cargo check --manifest-path chat-agent/Cargo.toml
cargo test --manifest-path chat-agent/Cargo.toml
```
