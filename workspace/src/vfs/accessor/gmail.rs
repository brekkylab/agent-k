use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

/// Retry budget for a rate-limited/5xx request. A directory listing fans out
/// many per-message fetches, so a burst can trip Gmail's per-user rate limit; a
/// bounded retry keeps a transient 429/5xx from failing the whole read.
const MAX_RETRIES: u32 = 5;
/// Cap on a single retry wait. Google's exponential-backoff guidance suggests a
/// `maximum_backoff` of 32–64s, but this call sits behind a FUSE op the agent
/// blocks on, so we cap far lower — a heavily throttled read still resolves in
/// bounded time rather than hanging the guest for a minute.
const MAX_BACKOFF: Duration = Duration::from_secs(8);
/// Jitter ceiling (Google's algorithm: `+ random_number_milliseconds ≤ 1000ms`),
/// recomputed per retry to de-synchronise clients that would otherwise retry in
/// lockstep.
const JITTER_MAX_MS: u64 = 1000;

/// Exponential backoff with jitter for retry `n` (0-based), per Google's Gmail
/// API guidance: `min(2^n s + rand(0..=1000ms), maximum_backoff)`.
fn backoff_delay(n: u32) -> Duration {
    let base = Duration::from_secs(1u64 << n.min(16));
    let jitter = Duration::from_millis(fastrand::u64(0..=JITTER_MAX_MS));
    (base + jitter).min(MAX_BACKOFF)
}

/// The `Retry-After` delay, if present (an explicit server instruction; capped by
/// [`MAX_BACKOFF`] at the call site). Google sends delta-seconds; the HTTP-date
/// form is not honored (treated as absent → falls back to [`backoff_delay`]).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Extract the message JSON bodies from a Gmail batch (`multipart/mixed`)
/// response. Rather than parse the multipart/HTTP envelope, scan for each
/// top-level balanced `{…}` and keep the ones that are message objects (have an
/// `id`); error sub-responses (`{"error":…}`) are skipped. Tolerant of the
/// boundary/header noise between parts.
fn parse_batch_bodies(text: &str) -> Vec<Value> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        // Find the matching close brace, respecting JSON strings/escapes.
        let (mut depth, mut in_str, mut esc, mut j) = (0i32, false, false, i);
        while j < bytes.len() {
            let c = bytes[j];
            if in_str {
                match c {
                    _ if esc => esc = false,
                    b'\\' => esc = true,
                    b'"' => in_str = false,
                    _ => {}
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        if depth == 0 && j < bytes.len() {
            if let Ok(v) = serde_json::from_str::<Value>(&text[i..=j])
                && v.get("id").is_some()
            {
                out.push(v);
            }
            i = j + 1;
        } else {
            break; // unbalanced tail; stop
        }
    }
    out
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

/// Holds Google OAuth credentials (one refresh token) and a cached access
/// token. A Gmail mount needs a token whose scope covers Gmail (e.g.
/// `https://www.googleapis.com/auth/gmail.modify`).
pub struct GmailAccessor {
    client: reqwest::Client,
    config: GmailConfig,
    /// Cached OAuth access token + its expiry. Refreshed proactively before
    /// expiry and on a 401 (see [`Self::send_with_refresh`]).
    access_token: Mutex<Option<(String, Instant)>>,
}

impl GmailAccessor {
    pub fn new(config: &GmailConfig) -> anyhow::Result<Self> {
        Ok(Self {
            // Bound every request: a hung upstream call run behind the FUSE
            // forward server would otherwise wedge the guest FUSE op (and any
            // process touching the mount) forever. A timeout makes it recoverable.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            config: config.clone(),
            access_token: Mutex::new(None),
        })
    }

    async fn token(&self) -> anyhow::Result<String> {
        let mut guard = self.access_token.lock().await;
        // Reuse a cached token until it's within 60s of expiry (proactive refresh
        // avoids the "everything 401s after ~1h" failure).
        if let Some((t, exp)) = guard.as_ref()
            && *exp > Instant::now() + Duration::from_secs(60)
        {
            return Ok(t.clone());
        }
        let resp = self
            .client
            .post(OAUTH_TOKEN_URL)
            .form(&[
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("refresh_token", self.config.refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("google token exchange {status}: {body}");
        }
        let v: Value = serde_json::from_str(&body)?;
        let token = v
            .get("access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow::anyhow!("no access_token in response"))?
            .to_string();
        let expires_in = v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(3600);
        *guard = Some((
            token.clone(),
            Instant::now() + Duration::from_secs(expires_in),
        ));
        Ok(token)
    }

    /// Send a request built from the current access token, retrying transient
    /// failures. A 401 (expired/revoked despite proactive refresh) drops the
    /// token, refreshes, and retries once; a 429/5xx retries a bounded number of
    /// times honoring `Retry-After`, so a rate limit or upstream blip doesn't
    /// fail the whole read. Non-retryable statuses are returned to the caller,
    /// which classifies them via `error_for_status`.
    async fn send_with_refresh(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        let mut token = self.token().await?;
        let mut refreshed = false;
        let mut retries = 0u32;
        loop {
            let resp = build(&token).send().await?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && !refreshed {
                *self.access_token.lock().await = None;
                token = self.token().await?;
                refreshed = true;
                continue;
            }
            let retryable =
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if retryable && retries < MAX_RETRIES {
                // An explicit Retry-After wins (capped so the agent isn't blocked
                // too long); otherwise exponential backoff with jitter.
                let wait = match retry_after(&resp) {
                    Some(d) => d.min(MAX_BACKOFF),
                    None => backoff_delay(retries),
                };
                retries += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            return Ok(resp);
        }
    }

    async fn get_json(&self, url: &str) -> anyhow::Result<Value> {
        let resp = self
            .send_with_refresh(|t| self.client.get(url).bearer_auth(t))
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// All Gmail labels (system + user). Each is `{id, name, type}`.
    pub async fn list_labels(&self) -> anyhow::Result<Vec<Value>> {
        let url = format!("{GMAIL_API_BASE}/users/me/labels");
        let v = self.get_json(&url).await?;
        Ok(v.get("labels")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Every message id under a label, paginating through all pages (500/page,
    /// `nextPageToken`) with no cap — the caller wants a complete listing, so a
    /// large label costs several pages.
    pub async fn list_all_message_ids(&self, label_id: &str) -> anyhow::Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut params: Vec<(&str, String)> = vec![
                ("maxResults", "500".to_string()),
                ("labelIds", label_id.to_string()),
            ];
            if let Some(t) = &page_token {
                params.push(("pageToken", t.clone()));
            }
            let url = reqwest::Url::parse_with_params(
                &format!("{GMAIL_API_BASE}/users/me/messages"),
                &params,
            )?;
            let v = self
                .send_with_refresh(|t| self.client.get(url.clone()).bearer_auth(t))
                .await?
                .error_for_status()?
                .json::<Value>()
                .await?;
            if let Some(arr) = v.get("messages").and_then(|m| m.as_array()) {
                ids.extend(
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from)),
                );
            }
            match v.get("nextPageToken").and_then(|t| t.as_str()) {
                Some(t) => page_token = Some(t.to_string()),
                None => break,
            }
        }
        Ok(ids)
    }

    /// Full message resource (`format=full`): headers, payload parts, labels,
    /// `internalDate`, `sizeEstimate`.
    pub async fn get_message_full(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{GMAIL_API_BASE}/users/me/messages/{id}?format=full");
        self.get_json(&url).await
    }

    /// Minimal message resource (`format=minimal`): `internalDate` + labels but
    /// no headers/body. Used to date-bucket a label listing cheaply.
    pub async fn get_message_minimal(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{GMAIL_API_BASE}/users/me/messages/{id}?format=minimal");
        self.get_json(&url).await
    }

    /// Fetch many messages in a single Gmail **batch** request (`/batch/gmail/v1`,
    /// multipart/mixed), collapsing N per-message round-trips into one. Returns
    /// the parsed message JSON for each sub-request that succeeded (order not
    /// guaranteed — callers key on the `id` field); chunked at Gmail's 100-per-
    /// batch cap. This is the fan-out path a directory listing takes.
    pub async fn get_messages_batch(
        &self,
        ids: &[String],
        format: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(100) {
            out.extend(self.batch_chunk(chunk, format).await?);
        }
        Ok(out)
    }

    async fn batch_chunk(&self, ids: &[String], format: &str) -> anyhow::Result<Vec<Value>> {
        const BOUNDARY: &str = "agentk_gmail_batch_boundary";
        let mut body = String::new();
        for id in ids {
            body.push_str(&format!("--{BOUNDARY}\r\n"));
            body.push_str("Content-Type: application/http\r\n\r\n");
            body.push_str(&format!(
                "GET /gmail/v1/users/me/messages/{id}?format={format}\r\n\r\n"
            ));
        }
        body.push_str(&format!("--{BOUNDARY}--\r\n"));

        // Batch lives at the API origin (…/batch/gmail/v1), not under /gmail/v1.
        let origin = GMAIL_API_BASE
            .strip_suffix("/gmail/v1")
            .unwrap_or(GMAIL_API_BASE);
        let url = format!("{origin}/batch/gmail/v1");
        let content_type = format!("multipart/mixed; boundary={BOUNDARY}");

        let resp = self
            .send_with_refresh(|t| {
                self.client
                    .post(&url)
                    .bearer_auth(t)
                    .header(reqwest::header::CONTENT_TYPE, &content_type)
                    .body(body.clone())
            })
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("gmail batch {status}: {}", &text[..text.len().min(300)]);
        }
        Ok(parse_batch_bodies(&text))
    }

    /// Fetch and base64url-decode an attachment's bytes.
    pub async fn get_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let url =
            format!("{GMAIL_API_BASE}/users/me/messages/{message_id}/attachments/{attachment_id}");
        let v = self.get_json(&url).await?;
        let data = v.get("data").and_then(|d| d.as_str()).unwrap_or("");
        Ok(decode_b64url(data))
    }

    /// Move a message to Trash (the `rm` of a `.gmail.json`).
    pub async fn trash(&self, message_id: &str) -> anyhow::Result<()> {
        let url = format!("{GMAIL_API_BASE}/users/me/messages/{message_id}/trash");
        self.send_with_refresh(|t| {
            self.client
                .post(&url)
                .bearer_auth(t)
                .json(&serde_json::json!({}))
        })
        .await?
        .error_for_status()?;
        Ok(())
    }

    /// Send a raw (base64url) RFC-2822 message, optionally within a thread.
    pub async fn send_raw(&self, raw_b64: &str, thread_id: Option<&str>) -> anyhow::Result<Value> {
        let mut body = serde_json::json!({ "raw": raw_b64 });
        if let Some(tid) = thread_id {
            body["threadId"] = Value::from(tid);
        }
        let url = format!("{GMAIL_API_BASE}/users/me/messages/send");
        let resp = self
            .send_with_refresh(|t| self.client.post(&url).bearer_auth(t).json(&body))
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("gmail send {status}: {text}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

/// Decode Gmail's base64url payload data, tolerating missing padding.
fn decode_b64url(s: &str) -> Vec<u8> {
    let trimmed = s.trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .unwrap_or_default()
}

/// Base64url-encode raw MIME bytes for the Gmail `send` API.
pub fn encode_b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_batch_bodies_extracts_messages_and_skips_errors() {
        // A multipart/mixed batch response with two message parts and one error
        // part, mirroring Gmail's envelope (boundary + application/http + HTTP).
        let resp = "--b\r\nContent-Type: application/http\r\n\r\n\
            HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n\
            {\"id\":\"m1\",\"payload\":{\"headers\":[{\"name\":\"Subject\",\"value\":\"a{b}c\"}]}}\r\n\
            --b\r\nContent-Type: application/http\r\n\r\n\
            HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n\r\n\
            {\"error\":{\"code\":404,\"message\":\"not found\"}}\r\n\
            --b\r\nContent-Type: application/http\r\n\r\n\
            HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n\
            {\"id\":\"m2\",\"snippet\":\"hi\"}\r\n--b--\r\n";
        let msgs = parse_batch_bodies(resp);
        let ids: Vec<&str> = msgs
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec!["m1", "m2"],
            "should extract both messages, skip the error part"
        );
        // Nested braces inside a JSON string ("a{b}c") must not break parsing.
        assert_eq!(msgs[0]["payload"]["headers"][0]["value"], "a{b}c");
    }

    #[test]
    fn backoff_is_exponential_jittered_and_capped() {
        // Each retry n waits in [2^n s, 2^n s + 1s), never above MAX_BACKOFF.
        for _ in 0..100 {
            // n=0 → [1s, 2s)
            let d0 = backoff_delay(0);
            assert!(
                d0 >= Duration::from_secs(1) && d0 <= Duration::from_millis(2000),
                "{d0:?}"
            );
            // n=1 → [2s, 3s)
            let d1 = backoff_delay(1);
            assert!(
                d1 >= Duration::from_secs(2) && d1 <= Duration::from_millis(3000),
                "{d1:?}"
            );
            // n=2 → [4s, 5s)
            let d2 = backoff_delay(2);
            assert!(
                d2 >= Duration::from_secs(4) && d2 <= Duration::from_millis(5000),
                "{d2:?}"
            );
            // large n → capped at MAX_BACKOFF (8s here); 2^4=16s + jitter > cap
            assert_eq!(backoff_delay(4), MAX_BACKOFF);
            assert_eq!(backoff_delay(10), MAX_BACKOFF);
        }
    }
}
