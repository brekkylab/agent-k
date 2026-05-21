use std::sync::Arc;
use std::time::Duration;

use ailoy::{
    datatype::Value,
    to_value,
    tool::{ToolDesc, ToolDescBuilder, ToolFunc},
    tool_func,
};
use reqwest::Client;

const BRAVE_API_BASE: &str = "https://api.search.brave.com/res/v1/web/search";
const BRAVE_API_KEY_ENV: &str = "BRAVE_SEARCH_API_KEY";
const REQUEST_TIMEOUT_SECS: u64 = 10;

struct BraveState {
    client: Client,
    api_key: Option<String>,
}

impl BraveState {
    fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("reqwest::Client builder cannot fail with these settings");
        let api_key = std::env::var(BRAVE_API_KEY_ENV).ok();
        Self { client, api_key }
    }
}

pub fn get_api_search_tool_desc() -> ToolDesc {
    ToolDescBuilder::new("api_search")
        .description(concat!(
            "Search the web via a paid search API (currently Brave Search). ",
            "Returns results ranked by the provider's own index, with the ",
            "same `{results, total}` shape as ailoy's meta-search builtin. ",
            "Use this to find current information, primary sources, ",
            "documentation, or any web-accessible content. The provider ",
            "and credentials are resolved from environment variables at ",
            "process start; see the deep-research agent's documentation."
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
    let state = Arc::new(BraveState::new());
    move |_| {
        let state = state.clone();
        tool_func!(async |args: Value| -> Value with [state = state.clone()] {
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
                .unwrap_or(10)
                .min(20);

            let Some(api_key) = state.api_key.as_deref() else {
                return to_value!({
                    "error": format!("missing required environment variable: {BRAVE_API_KEY_ENV}"),
                    "results": Value::array_empty(),
                    "total": 0i64
                });
            };

            let resp = match state
                .client
                .get(BRAVE_API_BASE)
                .header("X-Subscription-Token", api_key)
                .header("Accept", "application/json")
                .query(&[
                    ("q", query.as_str()),
                    ("count", &max_results.to_string()),
                ])
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("Brave Search request failed: {e}");
                    return to_value!({
                        "error": msg,
                        "results": Value::array_empty(),
                        "total": 0i64
                    });
                }
            };

            let status = resp.status();
            if !status.is_success() {
                let body: String = resp.text().await.unwrap_or_default();
                let msg = format!("Brave Search returned HTTP {}: {body}", status.as_u16());
                return to_value!({
                    "error": msg,
                    "results": Value::array_empty(),
                    "total": 0i64
                });
            }

            let json: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("Brave Search JSON parse failed: {e}");
                    return to_value!({
                        "error": msg,
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
        })
    }
}
