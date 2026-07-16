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
    let raw = resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
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

    /// Message stubs (`{id, threadId}`) for a label and/or query.
    pub async fn list_messages(
        &self,
        label_id: Option<&str>,
        query: Option<&str>,
        max_results: u32,
    ) -> anyhow::Result<Vec<Value>> {
        let mut params: Vec<(&str, String)> = vec![("maxResults", max_results.to_string())];
        if let Some(l) = label_id {
            params.push(("labelIds", l.to_string()));
        }
        if let Some(q) = query {
            params.push(("q", q.to_string()));
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
        Ok(v.get("messages")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default())
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
    fn backoff_is_exponential_jittered_and_capped() {
        // Each retry n waits in [2^n s, 2^n s + 1s), never above MAX_BACKOFF.
        for _ in 0..100 {
            // n=0 → [1s, 2s)
            let d0 = backoff_delay(0);
            assert!(d0 >= Duration::from_secs(1) && d0 <= Duration::from_millis(2000), "{d0:?}");
            // n=1 → [2s, 3s)
            let d1 = backoff_delay(1);
            assert!(d1 >= Duration::from_secs(2) && d1 <= Duration::from_millis(3000), "{d1:?}");
            // n=2 → [4s, 5s)
            let d2 = backoff_delay(2);
            assert!(d2 >= Duration::from_secs(4) && d2 <= Duration::from_millis(5000), "{d2:?}");
            // large n → capped at MAX_BACKOFF (8s here); 2^4=16s + jitter > cap
            assert_eq!(backoff_delay(4), MAX_BACKOFF);
            assert_eq!(backoff_delay(10), MAX_BACKOFF);
        }
    }
}
