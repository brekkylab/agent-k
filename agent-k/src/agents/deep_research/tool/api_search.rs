//! `api_search` — paid web-search tool with a pluggable provider.
//!
//! Result shape matches ailoy's `web_search` builtin
//! (`{results: [{title, url, description, sources}], total}`) so the
//! deep-research prompt does not need to know which provider answered.
//! Provider and credentials come from env vars, resolved once at factory
//! construction.

use std::sync::Arc;
use std::time::Duration;

use ailoy::{
    datatype::Value,
    to_value,
    tool::{ToolDesc, ToolDescBuilder, ToolFunc},
    tool_func,
};
use reqwest::Client;

const PROVIDER_ENV: &str = "API_SEARCH_PROVIDER";
const BRAVE_API_BASE: &str = "https://api.search.brave.com/res/v1/web/search";
const BRAVE_API_KEY_ENV: &str = "BRAVE_SEARCH_API_KEY";
const REQUEST_TIMEOUT_SECS: u64 = 10;
const DEFAULT_MAX_RESULTS: u64 = 10;
const RESULT_CAP: u64 = 20;

enum Provider {
    Brave {
        client: Client,
        api_key: Option<String>,
    },
}

impl Provider {
    fn from_env() -> Self {
        let name = std::env::var(PROVIDER_ENV)
            .unwrap_or_else(|_| "brave".into())
            .to_lowercase();
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("reqwest::Client builder cannot fail with these settings");
        match name.as_str() {
            "brave" => Self::Brave {
                client,
                api_key: std::env::var(BRAVE_API_KEY_ENV).ok(),
            },
            other => {
                log::warn!(
                    "{PROVIDER_ENV}=\"{other}\" not recognised, falling back to brave"
                );
                Self::Brave {
                    client,
                    api_key: std::env::var(BRAVE_API_KEY_ENV).ok(),
                }
            }
        }
    }

    async fn search(&self, query: &str, max_results: u64) -> Value {
        match self {
            Self::Brave { client, api_key } => {
                brave_search(client, api_key.as_deref(), query, max_results).await
            }
        }
    }
}

pub fn get_api_search_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("api_search")
        .description(concat!(
            "Search the web via a paid search API (currently Brave Search). ",
            "Returns results ranked by the provider's own index, with the ",
            "same `{results, total}` shape as ailoy's meta-search builtin. ",
            "Use this to find current information, primary sources, ",
            "documentation, or any web-accessible content. ",
            "When you need information about multiple independent entities ",
            "(different missions, different people, different products), ",
            "call this tool in a parallel batch of 2-5 in a single ",
            "tool_calls block — one query per entity — rather than ",
            "sequentially across multiple turns."
        ))
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query string"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Default: 10. Max: 20.",
                    "default": 10
                }
            },
            "required": ["query"]
        }))
        .build()
}

pub fn get_api_search_tool_factory() -> impl Fn(&ToolDesc) -> ToolFunc {
    let provider = Arc::new(Provider::from_env());
    move |_| {
        let provider = provider.clone();
        tool_func!(async |args: Value| -> Value with [provider = provider.clone()] {
            let query = args
                .pointer("/query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if query.is_empty() {
                return to_value!({ "results": Value::array_empty(), "total": 0i64 });
            }
            let max_results = args
                .pointer("/max_results")
                .and_then(|v| v.as_unsigned())
                .unwrap_or(DEFAULT_MAX_RESULTS)
                .min(RESULT_CAP);
            provider.search(&query, max_results).await
        })
    }
}

async fn brave_search(
    client: &Client,
    api_key: Option<&str>,
    query: &str,
    max_results: u64,
) -> Value {
    let Some(api_key) = api_key else {
        return to_value!({
            "error": format!("missing required environment variable: {BRAVE_API_KEY_ENV}"),
            "results": Value::array_empty(),
            "total": 0i64
        });
    };

    let resp = match client
        .get(BRAVE_API_BASE)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &max_results.to_string())])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return to_value!({
                "error": format!("Brave Search request failed: {e}"),
                "results": Value::array_empty(),
                "total": 0i64
            });
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let body: String = resp.text().await.unwrap_or_default();
        return to_value!({
            "error": format!("Brave Search returned HTTP {}: {body}", status.as_u16()),
            "results": Value::array_empty(),
            "total": 0i64
        });
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return to_value!({
                "error": format!("Brave Search JSON parse failed: {e}"),
                "results": Value::array_empty(),
                "total": 0i64
            });
        }
    };

    let items = json
        .pointer("/web/results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let results: Vec<Value> = items
        .into_iter()
        .map(|item| {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            to_value!({
                "title": title,
                "url": url,
                "description": description,
                "sources": Value::array([Value::string("Brave".to_string())])
            })
        })
        .collect();

    let total = results.len() as i64;
    to_value!({
        "results": Value::array(results),
        "total": total
    })
}
