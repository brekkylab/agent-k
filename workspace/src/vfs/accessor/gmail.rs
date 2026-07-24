use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1";

/// `(api_base, token_url)` for a config: the real Google hosts by default, or
/// both rooted at `base_url` when set — the enterprise-mock/gateway layout
/// (`{base}/gmail/v1`, `{base}/oauth2/token`).
fn endpoints(base_url: Option<&str>) -> (String, String) {
    match base_url {
        Some(b) => {
            let b = b.trim_end_matches('/');
            (format!("{b}/gmail/v1"), format!("{b}/oauth2/token"))
        }
        None => (GMAIL_API_BASE.to_string(), OAUTH_TOKEN_URL.to_string()),
    }
}

/// Messages per batch request. Google's hard cap is 100, but 50 halves the
/// worst-case response body (a `format=full` message can run to ~1 MB, and the
/// whole multipart response must land within the client's 30s timeout) and
/// keeps one chunk's quota burst (50 × 5 units) near Gmail's ~250 units/sec
/// per-user rate instead of double it.
const BATCH_CHUNK: usize = 50;
/// Concurrent batch requests ([`BATCH_CHUNK`] messages each). Kept low because
/// too many overshoot Gmail's per-user rate limit and the resulting 429
/// backoff makes it slower than serial; 3 measured fastest on a real mailbox.
const BATCH_CONCURRENCY: usize = 3;
/// Max passes over the id set, each re-requesting only ids that didn't come back
/// (failed chunk, or an errored sub-response inside a 200 batch). Bounded: an id
/// trashed between `list` and `get` 404s forever and would loop otherwise.
const MAX_BATCH_ROUNDS: usize = 3;

/// Retry budget for a rate-limited/5xx request. A directory listing fans out
/// many per-message fetches, so a burst can trip Gmail's per-user rate limit; a
/// bounded retry keeps a transient 429/5xx from failing the whole read.
const MAX_RETRIES: u32 = 5;
/// Cap on a single retry wait. Google's guidance suggests a `maximum_backoff` of
/// 32–64s, but this call sits behind a FUSE op the agent blocks on, so we cap
/// lower. 16s (with `MAX_RETRIES`=5, ~31s worst-case total) rides out the
/// ~25–30s throttle stalls seen while indexing a large mailbox, without hanging
/// the guest for a minute.
const MAX_BACKOFF: Duration = Duration::from_secs(16);
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

/// Result of the mount-create code exchange: the long-lived refresh token plus
/// the account's email address (`users.getProfile`). The email is what
/// identifies the *account* — shown in mount info and used as the cache key,
/// since a refresh token changes on every re-consent while the email doesn't.
pub struct GmailExchange {
    pub refresh_token: String,
    pub account_email: String,
}

/// Exchange an OAuth authorization `code` for a refresh token (confidential
/// client, server-side). Run at mount-create so the browser never handles the
/// client secret. Google only returns a refresh token when the consent used
/// `access_type=offline` + `prompt=consent`. `base_url` overrides the Google
/// hosts (mock/gateway deployments — see [`GmailConfig::base_url`]); `None` =
/// production.
pub async fn exchange_gmail_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    base_url: Option<&str>,
) -> anyhow::Result<GmailExchange> {
    let (api_base, token_url) = endpoints(base_url);
    // Bounded: this runs inside the create_mount HTTP handler, and a bare
    // reqwest client has NO default timeout — a hung upstream would hang the
    // mount creation indefinitely.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let resp = client
        .post(&token_url)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("google code exchange {status}: {body}");
    }
    let v: Value = serde_json::from_str(&body)?;
    let refresh_token = v
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(String::from)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "token response had no refresh_token (consent must use \
                 access_type=offline + prompt=consent)"
            )
        })?;
    // The exchange response carries a live access token — resolve the account
    // email while we have it. Required: the email is the mount's identity
    // (display + cache key), so a transient profile failure fails the create
    // with a clear message rather than minting a half-identified mount.
    let account_email = match v.get("access_token").and_then(|t| t.as_str()) {
        Some(at) => fetch_profile_email(at, &api_base).await,
        None => None,
    }
    .ok_or_else(|| {
        anyhow::anyhow!("code exchange succeeded but users.getProfile failed; retry the mount")
    })?;
    Ok(GmailExchange {
        refresh_token,
        account_email,
    })
}

/// `users.getProfile` → lowercased email address, if the call succeeds.
async fn fetch_profile_email(access_token: &str, api_base: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let v: Value = client
        .get(format!("{api_base}/users/me/profile"))
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    v.get("emailAddress")
        .and_then(|e| e.as_str())
        .map(|s| s.trim().to_lowercase())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GmailConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// The account's email address, resolved at mount-create
    /// ([`exchange_gmail_code`]). Identifies the account across re-consents
    /// (a refresh token changes each consent; the email doesn't), so it keys
    /// the disk cache and is shown in mount info.
    pub account_email: String,
    /// Alternative API origin (an enterprise mock or gateway): requests go to
    /// `{base_url}/gmail/v1` and `{base_url}/oauth2/token` instead of the real
    /// Google hosts. `None` = production Google. Deployment-level only — the
    /// token URL receives the app's client secret, so this must never be
    /// user-suppliable: it is NOT part of the mount-create API; the backend
    /// injects it from its own config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Holds Google OAuth credentials (one refresh token) and a cached access
/// token. A Gmail mount is read-only, so the token needs
/// `https://www.googleapis.com/auth/gmail.readonly`.
pub struct GmailAccessor {
    client: reqwest::Client,
    config: GmailConfig,
    /// Resolved API origin (`…/gmail/v1`) — real Google or the config's
    /// `base_url` (see [`endpoints`]).
    api_base: String,
    /// Resolved OAuth token endpoint, same override rule.
    token_url: String,
    /// Cached OAuth access token + its expiry. Refreshed proactively before
    /// expiry and on a 401 (see [`Self::send_with_refresh`]).
    access_token: Mutex<Option<(String, Instant)>>,
}

impl GmailAccessor {
    pub fn new(config: &GmailConfig) -> anyhow::Result<Self> {
        let (api_base, token_url) = endpoints(config.base_url.as_deref());
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
            api_base,
            token_url,
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
            .post(&self.token_url)
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
        self.send_with_refresh_inner(build, true).await
    }

    /// [`Self::send_with_refresh`] minus the 5xx retry, for non-idempotent
    /// calls (`messages.send`): a 5xx can arrive after Gmail has already
    /// accepted the send, so a blind retry risks duplicating the email. A 429
    /// still retries (a rate-limited request is rejected before it executes),
    /// as does the 401 refresh-once (a failed-auth request never executed).
    async fn send_nonidempotent(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> anyhow::Result<reqwest::Response> {
        self.send_with_refresh_inner(build, false).await
    }

    async fn send_with_refresh_inner(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
        retry_5xx: bool,
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
            let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || (retry_5xx && status.is_server_error());
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
        let url = format!("{}/users/me/labels", self.api_base);
        let v = self.get_json(&url).await?;
        Ok(v.get("labels")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Message ids under a label, paginating (500/page, `nextPageToken`) up to
    /// `limit` — a safety ceiling that stops a pathologically large label from
    /// costing thousands of pages and an unbounded index build. `messages.list`
    /// is newest-first, so hitting the cap keeps the newest `limit` ids.
    pub async fn list_all_message_ids(
        &self,
        label_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<String>> {
        self.list_message_ids(("labelIds", label_id), limit).await
    }

    /// Message ids matching a Gmail search query (`q=` — server-side, so a
    /// content search costs one `messages.list` instead of fetching every
    /// body). Newest-first; capped at `limit`.
    pub async fn search_message_ids(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<String>> {
        self.list_message_ids(("q", query), limit).await
    }

    /// Shared `messages.list` pagination behind the label/search entry points:
    /// one `filter` query pair, newest-first, truncated at `limit`.
    async fn list_message_ids(
        &self,
        filter: (&str, &str),
        limit: usize,
    ) -> anyhow::Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut params: Vec<(&str, String)> = vec![
                ("maxResults", "500".to_string()),
                (filter.0, filter.1.to_string()),
                // We only read message ids here; drop threadId/resultSizeEstimate.
                ("fields", "messages/id,nextPageToken".to_string()),
            ];
            if let Some(t) = &page_token {
                params.push(("pageToken", t.clone()));
            }
            let url = reqwest::Url::parse_with_params(
                &format!("{}/users/me/messages", self.api_base),
                &params,
            )?;
            let text = self
                .send_with_refresh(|t| self.client.get(url.clone()).bearer_auth(t))
                .await?
                .error_for_status()?
                .text()
                .await?;
            // An empty label returns 204 No Content with an empty body (e.g.
            // CHAT/TRASH); there's nothing to parse or page.
            if text.trim().is_empty() {
                break;
            }
            let v: Value = serde_json::from_str(&text)?;
            if let Some(arr) = v.get("messages").and_then(|m| m.as_array()) {
                ids.extend(
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from)),
                );
            }
            // Safety ceiling: newest-first, so truncating keeps the newest `limit`
            // ids and stops us paginating (and later fetching internalDate for) a
            // pathologically large label.
            if ids.len() >= limit {
                ids.truncate(limit);
                tracing::warn!(
                    "gmail list ({}={}): reached cap {limit}; older messages omitted",
                    filter.0,
                    filter.1
                );
                break;
            }
            match v.get("nextPageToken").and_then(|t| t.as_str()) {
                Some(t) => page_token = Some(t.to_string()),
                None => break,
            }
        }
        Ok(ids)
    }

    /// The message-resource query for `format`. `minimal` is only ever used to
    /// date-bucket a label listing, which reads just `id` + `internalDate`, so we
    /// add a `fields` mask (partial response) and let Gmail drop the labels /
    /// `sizeEstimate` / `historyId` we'd otherwise receive and discard. `full`
    /// carries the whole message (we warm the body cache from it), so no mask.
    fn msg_query(format: &str) -> String {
        match format {
            "minimal" => "format=minimal&fields=id,internalDate".to_string(),
            other => format!("format={other}"),
        }
    }

    /// Full message resource (`format=full`): headers, payload parts, labels,
    /// `internalDate`, `sizeEstimate`.
    pub async fn get_message_full(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{}/users/me/messages/{id}?format=full", self.api_base);
        self.get_json(&url).await
    }

    /// Minimal message resource (`format=minimal`, masked to `id` + `internalDate`
    /// — see [`Self::msg_query`]). Used to date-bucket a label listing cheaply.
    pub async fn get_message_minimal(&self, id: &str) -> anyhow::Result<Value> {
        let q = Self::msg_query("minimal");
        let url = format!("{}/users/me/messages/{id}?{q}", self.api_base);
        self.get_json(&url).await
    }

    /// Fetch many messages via Gmail **batch** requests (`/batch/gmail/v1`,
    /// multipart/mixed), collapsing per-message round-trips into
    /// [`BATCH_CHUNK`]-message chunks run [`BATCH_CONCURRENCY`] at a time. Returns the parsed message JSON
    /// keyed by `id` (order not guaranteed — callers key on `id`).
    ///
    /// Completeness over speed: after each parallel pass it reconciles by id and
    /// re-requests only what didn't come back — a failed chunk *or* an errored
    /// sub-response inside an otherwise-200 batch (which [`parse_batch_bodies`]
    /// silently omits). Up to [`MAX_BATCH_ROUNDS`] passes; ids still unresolved
    /// after that (e.g. trashed between `list` and `get`) are logged, not hidden.
    /// This is the fan-out path a directory listing takes.
    pub async fn get_messages_batch(
        &self,
        ids: &[String],
        format: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let mut got: HashMap<String, Value> = HashMap::with_capacity(ids.len());
        for _ in 0..MAX_BATCH_ROUNDS {
            let missing: Vec<String> = ids
                .iter()
                .filter(|id| !got.contains_key(*id))
                .cloned()
                .collect();
            if missing.is_empty() {
                break;
            }
            // A chunk that errors resolves to an empty Vec, so its ids simply stay
            // missing and are retried next round rather than failing the batch.
            let chunks: Vec<Vec<String>> = missing
                .chunks(BATCH_CHUNK)
                .map(<[String]>::to_vec)
                .collect();
            let passes: Vec<Vec<Value>> =
                stream::iter(chunks)
                    .map(|chunk| async move {
                        self.batch_chunk(&chunk, format).await.unwrap_or_default()
                    })
                    .buffer_unordered(BATCH_CONCURRENCY)
                    .collect()
                    .await;
            let before = got.len();
            for v in passes.into_iter().flatten() {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    // Key on id: order-independent and dup-safe across retries.
                    got.insert(id.to_string(), v);
                }
            }
            // No progress → the remaining ids will not resolve by repeating
            // the same request (permanently 404'd, e.g. trashed between `list`
            // and `get`); stop instead of burning the remaining rounds.
            if got.len() == before {
                break;
            }
        }
        let unresolved = ids.len() - got.len();
        if unresolved > 0 {
            tracing::warn!(
                "gmail batch: {unresolved}/{} message(s) unresolved after {MAX_BATCH_ROUNDS} rounds (likely trashed)",
                ids.len()
            );
        }
        Ok(got.into_values().collect())
    }

    async fn batch_chunk(&self, ids: &[String], format: &str) -> anyhow::Result<Vec<Value>> {
        const BOUNDARY: &str = "agentk_gmail_batch_boundary";
        let mut body = String::new();
        for id in ids {
            body.push_str(&format!("--{BOUNDARY}\r\n"));
            body.push_str("Content-Type: application/http\r\n\r\n");
            let q = Self::msg_query(format);
            body.push_str(&format!("GET /gmail/v1/users/me/messages/{id}?{q}\r\n\r\n"));
        }
        body.push_str(&format!("--{BOUNDARY}--\r\n"));

        // Batch lives at the API origin (…/batch/gmail/v1), not under /gmail/v1.
        let origin = self
            .api_base
            .strip_suffix("/gmail/v1")
            .unwrap_or(&self.api_base);
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
            anyhow::bail!("gmail batch {status}: {}", truncate_chars(&text, 300));
        }
        Ok(parse_batch_bodies(&text))
    }

    /// Fetch and base64url-decode an attachment's bytes.
    pub async fn get_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let url = format!(
            "{}/users/me/messages/{message_id}/attachments/{attachment_id}",
            self.api_base
        );
        let v = self.get_json(&url).await?;
        let data = v.get("data").and_then(|d| d.as_str()).unwrap_or("");
        Ok(decode_b64url(data))
    }

    /// Move a message to Trash (the `rm` of a `.gmail.json`).
    pub async fn trash(&self, message_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/users/me/messages/{message_id}/trash", self.api_base);
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
    /// Non-idempotent — `messages.send` may have queued the mail even when the
    /// response is a 5xx — so this rides [`Self::send_nonidempotent`], never
    /// the 5xx-retrying path (a blind retry could duplicate the email).
    pub async fn send_raw(&self, raw_b64: &str, thread_id: Option<&str>) -> anyhow::Result<Value> {
        let mut body = serde_json::json!({ "raw": raw_b64 });
        if let Some(tid) = thread_id {
            body["threadId"] = Value::from(tid);
        }
        let url = format!("{}/users/me/messages/send", self.api_base);
        let resp = self
            .send_nonidempotent(|t| self.client.post(&url).bearer_auth(t).json(&body))
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("gmail send {status}: {text}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

/// First `max` bytes of `s`, cut back to a char boundary — a plain byte slice
/// would panic when the cut lands mid-UTF-8 (error bodies can carry non-ASCII).
fn truncate_chars(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
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
    fn endpoints_default_to_google_and_reroot_on_base_url() {
        let (api, tok) = endpoints(None);
        assert_eq!(api, "https://gmail.googleapis.com/gmail/v1");
        assert_eq!(tok, "https://oauth2.googleapis.com/token");
        // A base_url reroots both (mock/gateway layout); trailing '/' tolerated.
        let (api, tok) = endpoints(Some("https://enterprise-mock.brekkylab.com/"));
        assert_eq!(api, "https://enterprise-mock.brekkylab.com/gmail/v1");
        assert_eq!(tok, "https://enterprise-mock.brekkylab.com/oauth2/token");
        // The batch origin derives from api_base by stripping /gmail/v1, so a
        // rerooted base keeps batch under the same origin.
        assert_eq!(
            api.strip_suffix("/gmail/v1").unwrap(),
            "https://enterprise-mock.brekkylab.com"
        );
    }

    #[test]
    fn truncate_chars_respects_utf8_boundaries() {
        assert_eq!(truncate_chars("hello", 300), "hello");
        assert_eq!(truncate_chars("hello", 3), "hel");
        // '한' is 3 bytes: a cut at byte 4 lands mid-char and must back up.
        assert_eq!(truncate_chars("한글메시지", 4), "한");
        assert_eq!(truncate_chars("한글메시지", 0), "");
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
