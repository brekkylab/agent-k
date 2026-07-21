use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const API: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2022-06-28";
/// Recursion ceiling for `list_block_tree`.
const MAX_BLOCK_DEPTH: usize = 10;
/// Waits before the 1st and 2nd retry of a rate-limited/5xx request (2 retries,
/// 3 attempts total).
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_millis(500), Duration::from_secs(2)];
/// Upper bound on a single retry wait (honoring Retry-After), so a large value
/// can't wedge the FUSE op behind this call.
const MAX_BACKOFF: Duration = Duration::from_secs(3);

/// Reject non-UUID ids before they reach a request URL: the `url` crate honors
/// `../`/`?`/`#`, so an unchecked id could rewrite the path to another endpoint.
/// Length guard excludes `try_parse`'s braced/urn forms (Notion emits only 32/36).
fn valid_notion_id(s: &str) -> bool {
    matches!(s.len(), 32 | 36) && uuid::Uuid::try_parse(s).is_ok()
}

/// Turn a finished response into JSON, or bail on a non-2xx status.
async fn finish(resp: reqwest::Response) -> anyhow::Result<Value> {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("notion API {status}: {body}");
    }
    Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// The `Retry-After` delay, if present. Notion sends delta-seconds; the HTTP-date
/// form is not honored (treated as absent → falls back to exponential backoff).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    parse_retry_after(raw)
}

fn parse_retry_after(raw: &str) -> Option<Duration> {
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NotionConfig {
    pub api_key: String,
}

/// Holds the Notion API client (one token).
pub struct NotionAccessor {
    client: reqwest::Client,
    api_key: String,
}

impl NotionAccessor {
    pub fn new(config: &NotionConfig) -> anyhow::Result<Self> {
        Ok(Self {
            // Bound every request: a hung upstream call would otherwise never
            // return, and since these run behind the FUSE forward server, that
            // wedges the guest FUSE op (and any process touching the mount)
            // indefinitely. A timeout turns that into a recoverable error.
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            api_key: config.api_key.clone(),
        })
    }

    /// Add the auth + version headers. Content-Type is set by `.json(...)` on the
    /// builder; don't add a second one (a duplicate makes Notion ignore the body).
    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header("Authorization", format!("Bearer {}", self.api_key))
            .header("Notion-Version", NOTION_VERSION)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Value> {
        // Rendering one page fans out into many sequential calls (get_page +
        // list_children per block, to depth 10), so a medium page can trip
        // Notion's ~3 req/s limit. Retry 429/5xx a bounded number of times,
        // honoring Retry-After, so a transient limit doesn't fail the whole read.
        for attempt in 0..=RETRY_BACKOFF.len() {
            let Some(this) = req.try_clone() else {
                // Non-cloneable body (not used by our calls): send once, no retry.
                return finish(self.authed(req).send().await?).await;
            };
            let resp = self.authed(this).send().await?;
            let status = resp.status();
            let retryable =
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if retryable && attempt < RETRY_BACKOFF.len() {
                let wait = retry_after(&resp)
                    .unwrap_or(RETRY_BACKOFF[attempt])
                    .min(MAX_BACKOFF);
                tokio::time::sleep(wait).await;
                continue;
            }
            return finish(resp).await;
        }
        unreachable!("the final attempt returns instead of retrying")
    }

    /// Pages shared with the integration (search, filtered to pages), paging
    /// through every result.
    pub async fn search_pages(&self) -> anyhow::Result<Vec<Value>> {
        let mut results = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut body = json!({
                "filter": {"property": "object", "value": "page"},
                "page_size": 100,
            });
            if let Some(c) = &cursor {
                body["start_cursor"] = json!(c);
            }
            let v = self
                .send(self.client.post(format!("{API}/search")).json(&body))
                .await?;
            if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
                results.extend(arr.iter().cloned());
            }
            if !v.get("has_more").and_then(|h| h.as_bool()).unwrap_or(false) {
                break;
            }
            match v.get("next_cursor").and_then(|c| c.as_str()) {
                Some(c) => cursor = Some(c.to_string()),
                None => break,
            }
        }
        Ok(results)
    }

    pub async fn get_page(&self, id: &str) -> anyhow::Result<Value> {
        anyhow::ensure!(valid_notion_id(id), "invalid notion page id: {id:?}");
        self.send(self.client.get(format!("{API}/pages/{id}")))
            .await
    }

    /// All immediate block children of `id`, paging through every result.
    pub async fn list_children(&self, id: &str) -> anyhow::Result<Vec<Value>> {
        anyhow::ensure!(valid_notion_id(id), "invalid notion block id: {id:?}");
        let mut results = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut url = format!("{API}/blocks/{id}/children?page_size=100");
            if let Some(c) = &cursor {
                url.push_str("&start_cursor=");
                url.push_str(c);
            }
            let v = self.send(self.client.get(url)).await?;
            if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
                results.extend(arr.iter().cloned());
            }
            if !v.get("has_more").and_then(|h| h.as_bool()).unwrap_or(false) {
                break;
            }
            match v.get("next_cursor").and_then(|c| c.as_str()) {
                Some(c) => cursor = Some(c.to_string()),
                None => break,
            }
        }
        Ok(results)
    }

    /// List block children recursively, embedding nested blocks under a
    /// `children` key. Blocks of type `child_page`/`child_database` are not
    /// descended into (their children belong to a different page). Recursion
    /// stops at [`MAX_BLOCK_DEPTH`].
    pub async fn list_block_tree(&self, id: &str) -> anyhow::Result<Vec<Value>> {
        self.list_block_tree_depth(id.to_string(), 0).await
    }

    fn list_block_tree_depth(
        &self,
        id: String,
        depth: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<Value>>> + Send + '_>>
    {
        Box::pin(async move {
            let mut blocks = self.list_children(&id).await?;
            if depth >= MAX_BLOCK_DEPTH {
                return Ok(blocks);
            }
            for block in &mut blocks {
                let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if btype == "child_page" || btype == "child_database" {
                    continue;
                }
                let has_children = block
                    .get("has_children")
                    .and_then(|h| h.as_bool())
                    .unwrap_or(false);
                if has_children {
                    let child_id = block
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let children = self.list_block_tree_depth(child_id, depth + 1).await?;
                    block["children"] = Value::Array(children);
                }
            }
            Ok(blocks)
        })
    }

    pub async fn create_page(&self, body: Value) -> anyhow::Result<Value> {
        self.send(self.client.post(format!("{API}/pages")).json(&body))
            .await
    }

    pub async fn append_blocks(&self, block_id: &str, children: Value) -> anyhow::Result<Value> {
        anyhow::ensure!(valid_notion_id(block_id), "invalid notion block id: {block_id:?}");
        let body = json!({ "children": children });
        self.send(
            self.client
                .patch(format!("{API}/blocks/{block_id}/children"))
                .json(&body),
        )
        .await
    }

    pub async fn add_comment(&self, body: Value) -> anyhow::Result<Value> {
        self.send(self.client.post(format!("{API}/comments")).json(&body))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, parse_retry_after, valid_notion_id};

    #[test]
    fn notion_id_validation_rejects_url_escapes() {
        assert!(valid_notion_id("22222222222222222222222222222222"));
        assert!(valid_notion_id("22222222-2222-2222-2222-222222222222"));
        for bad in ["../pages/2222?", "..%2Fpages", "abc/def", "x?y", "x#y", ""] {
            assert!(!valid_notion_id(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn retry_after_parses_delta_seconds_only() {
        assert_eq!(parse_retry_after("1"), Some(Duration::from_secs(1)));
        assert_eq!(parse_retry_after("  30 "), Some(Duration::from_secs(30)));
        // HTTP-date form and garbage are not honored.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after(""), None);
    }
}
