use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";

/// `(api_base, token_url)` for a config: the real Google hosts by default, or
/// both rooted at `base_url` when set — the enterprise-mock/gateway layout
/// (`{base}/drive/v3`, `{base}/oauth2/token`).
fn endpoints(base_url: Option<&str>) -> (String, String) {
    match base_url {
        Some(b) => {
            let b = b.trim_end_matches('/');
            (format!("{b}/drive/v3"), format!("{b}/oauth2/token"))
        }
        None => (DRIVE_API_BASE.to_string(), OAUTH_TOKEN_URL.to_string()),
    }
}

/// Per-file fields requested from every listing — exactly what the mount puts
/// in a file's metadata card. One entry per field, joined at request time: the
/// separators aren't hand-maintained, so a mask can't grow a stray space or a
/// missing comma the way a single hand-written literal can.
const FILE_FIELDS: &[&str] = &[
    "id",
    "name",
    "mimeType",
    // Shared-drive scoping: children of a shared drive must be listed with it.
    "driveId",
    // Drive's own size: populated for binary files *and* Docs-editors files,
    // absent for folders and shortcuts. (enterprise-mock omits it on native
    // docs — a divergence from Google, so don't rely on either shape.)
    // Informational only: the mount never transfers content, so this is never
    // the entry's own size.
    "size",
    "modifiedTime",
    "createdTime",
    "webViewLink",
    // Nested sub-selection, so this one entry carries its own punctuation.
    "owners(displayName,emailAddress)",
];

/// Hard cap on listing pages (1000 files/page, 100 drives/page) so a
/// duplicate/looping `nextPageToken` (a known Drive API pathology with some
/// query/corpora combos) can't spin forever.
const MAX_PAGES: usize = 50;

/// Retry budget for a rate-limited/5xx request; same reasoning as the gmail
/// accessor's (these calls sit behind a FUSE/WebDAV op the agent blocks on, so
/// the worst-case total stays low).
const MAX_RETRIES: u32 = 5;
const MAX_BACKOFF: Duration = Duration::from_secs(16);
const JITTER_MAX_MS: u64 = 1000;

/// Exponential backoff with jitter for retry `n` (0-based), per Google's API
/// guidance: `min(2^n s + rand(0..=1000ms), maximum_backoff)`.
fn backoff_delay(n: u32) -> Duration {
    let base = Duration::from_secs(1u64 << n.min(16));
    let jitter = Duration::from_millis(fastrand::u64(0..=JITTER_MAX_MS));
    (base + jitter).min(MAX_BACKOFF)
}

/// The `Retry-After` delay, if present (delta-seconds only; the HTTP-date form
/// is treated as absent and falls back to [`backoff_delay`]).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Result of the mount-create code exchange: the long-lived refresh token plus
/// the account's email address (`about.get`). The email identifies the
/// *account* — shown in mount info — since a refresh token changes on every
/// re-consent while the email doesn't.
pub struct GdriveExchange {
    pub refresh_token: String,
    pub account_email: String,
}

/// Exchange an OAuth authorization `code` for a refresh token (confidential
/// client, server-side). Run at mount-create so the browser never handles the
/// client secret. Google only returns a refresh token when the consent used
/// `access_type=offline` + `prompt=consent`. `base_url` overrides the Google
/// hosts (mock/gateway deployments — see [`GdriveConfig::base_url`]); `None` =
/// production.
pub async fn exchange_gdrive_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    base_url: Option<&str>,
) -> anyhow::Result<GdriveExchange> {
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
    // email while we have it. Required: the email is the mount's identity, so a
    // transient failure fails the create with a clear message rather than
    // minting a half-identified mount.
    let account_email = match v.get("access_token").and_then(|t| t.as_str()) {
        Some(at) => fetch_about_email(at, &api_base).await,
        None => None,
    }
    .ok_or_else(|| {
        anyhow::anyhow!("code exchange succeeded but about.get failed; retry the mount")
    })?;
    Ok(GdriveExchange {
        refresh_token,
        account_email,
    })
}

/// `about.get?fields=user(emailAddress)` → lowercased email, if the call
/// succeeds.
async fn fetch_about_email(access_token: &str, api_base: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let v: Value = client
        .get(format!("{api_base}/about?fields=user(emailAddress)"))
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    v.get("user")
        .and_then(|u| u.get("emailAddress"))
        .and_then(|e| e.as_str())
        .map(|s| s.trim().to_lowercase())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GdriveConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// The account's email address, resolved at mount-create
    /// ([`exchange_gdrive_code`]). Identifies the account across re-consents
    /// (a refresh token changes each consent; the email doesn't); shown in
    /// mount info so the UI can tell mounts apart.
    pub account_email: String,
    /// Alternative API origin (an enterprise mock or gateway): requests go to
    /// `{base_url}/drive/v3` and `{base_url}/oauth2/token` instead of the real
    /// Google hosts. `None` = production Google. Deployment-level only — the
    /// token URL receives the app's client secret, so this must never be
    /// user-suppliable: it is NOT part of the mount-create API; the backend
    /// injects it from its own config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Holds Google OAuth credentials (one refresh token) and a cached access
/// token. The mount is read-only and never transfers file content, so the token
/// needs `https://www.googleapis.com/auth/drive.readonly` (metadata-only scopes
/// would also do for listing, but `drive.readonly` is the documented one).
pub struct GdriveAccessor {
    client: reqwest::Client,
    config: GdriveConfig,
    /// Drive API base, resolved once from [`GdriveConfig::base_url`].
    api_base: String,
    /// OAuth token endpoint, resolved once from [`GdriveConfig::base_url`].
    token_url: String,
    /// Cached OAuth access token + its expiry. Refreshed proactively before
    /// expiry and on a 401 (see [`Self::send_with_refresh`]).
    access_token: Mutex<Option<(String, Instant)>>,
}

impl GdriveAccessor {
    pub fn new(config: &GdriveConfig) -> anyhow::Result<Self> {
        let (api_base, token_url) = endpoints(config.base_url.as_deref());
        Ok(Self {
            // Bound every request: a hung upstream call behind a filesystem op
            // would otherwise wedge the op (and any process touching the mount)
            // forever. A timeout makes it recoverable.
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
    /// token, refreshes, and retries once; a 429/5xx retries a bounded number
    /// of times honoring `Retry-After`. Every call this accessor makes is an
    /// idempotent GET, so the 5xx retry is always safe. Non-retryable statuses
    /// are returned to the caller, which classifies them via `error_for_status`.
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
                // An explicit Retry-After wins (capped so the caller isn't
                // blocked too long); otherwise exponential backoff with jitter.
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

    async fn get_json(&self, url: reqwest::Url) -> anyhow::Result<Value> {
        let resp = self
            .send_with_refresh(|t| self.client.get(url.clone()).bearer_auth(t))
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    /// Shared `files.list` pagination: one `q`, optional shared-drive scoping,
    /// truncated at `limit` collected files.
    async fn list_files_q(
        &self,
        q: &str,
        drive_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let mut files = Vec::new();
        let mut page_token: Option<String> = None;
        let mut pages = 0usize;
        loop {
            pages += 1;
            if pages > MAX_PAGES {
                tracing::warn!(
                    "gdrive files.list: reached page cap {MAX_PAGES}; listing truncated"
                );
                break;
            }
            let mut params: Vec<(&str, String)> = vec![
                ("q", q.to_string()),
                (
                    "fields",
                    format!("nextPageToken,files({})", FILE_FIELDS.join(",")),
                ),
                ("pageSize", "1000".to_string()),
            ];
            if let Some(d) = drive_id {
                params.push(("corpora", "drive".to_string()));
                params.push(("driveId", d.to_string()));
                params.push(("includeItemsFromAllDrives", "true".to_string()));
                params.push(("supportsAllDrives", "true".to_string()));
            }
            if let Some(pt) = &page_token {
                params.push(("pageToken", pt.clone()));
            }
            let url =
                reqwest::Url::parse_with_params(&format!("{}/files", self.api_base), &params)?;
            let v = self.get_json(url).await?;
            if let Some(arr) = v.get("files").and_then(|f| f.as_array()) {
                files.extend(arr.iter().cloned());
            }
            if files.len() >= limit {
                files.truncate(limit);
                tracing::warn!("gdrive files.list: reached cap {limit}; listing truncated");
                break;
            }
            let next = v
                .get("nextPageToken")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());
            // Stop on no token, or a token identical to the one we just used
            // (would otherwise re-fetch the same page forever).
            if next.is_none() || next == page_token {
                break;
            }
            page_token = next;
        }
        Ok(files)
    }

    /// The immediate, non-trashed children of `folder_id` ("root" for the My
    /// Drive root). `drive_id` is set when listing inside a shared drive.
    pub async fn list_files(
        &self,
        folder_id: &str,
        drive_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        let q = format!("'{folder_id}' in parents and trashed=false");
        self.list_files_q(&q, drive_id, limit).await
    }

    /// Items shared with the account ("Shared with me"). They carry no
    /// `parents`, so they are unreachable through the folder tree — this is the
    /// only listing that surfaces them.
    pub async fn list_shared_with_me(&self, limit: usize) -> anyhow::Result<Vec<Value>> {
        self.list_files_q("sharedWithMe=true and trashed=false", None, limit)
            .await
    }

    /// A native Workspace doc converted to `mime` (`files.export` — Docs/Slides
    /// → `text/plain`, Sheets → `text/csv`). Google caps an export at 10 MB and
    /// fails the request past that, which bounds what one read can return.
    pub async fn export(&self, id: &str, mime: &str) -> anyhow::Result<Vec<u8>> {
        let url = reqwest::Url::parse_with_params(
            &format!("{}/files/{id}/export", self.api_base),
            &[("mimeType", mime)],
        )?;
        let resp = self
            .send_with_refresh(|t| self.client.get(url.clone()).bearer_auth(t))
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// A blob file's bytes (`files.get?alt=media`). Only used for files already
    /// judged small and text-shaped; a native doc 403s here and goes through
    /// [`Self::export`] instead.
    pub async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!(
            "{}/files/{id}?alt=media&supportsAllDrives=true",
            self.api_base
        );
        let resp = self
            .send_with_refresh(|t| self.client.get(&url).bearer_auth(t))
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// Shared drives visible to the account (best-effort; needs scope).
    pub async fn list_shared_drives(&self) -> anyhow::Result<Vec<Value>> {
        let mut drives = Vec::new();
        let mut page_token: Option<String> = None;
        let mut pages = 0usize;
        loop {
            pages += 1;
            if pages > MAX_PAGES {
                break;
            }
            let mut params: Vec<(&str, String)> = vec![
                ("fields", "nextPageToken,drives(id,name)".to_string()),
                ("pageSize", "100".to_string()),
            ];
            if let Some(pt) = &page_token {
                params.push(("pageToken", pt.clone()));
            }
            let url =
                reqwest::Url::parse_with_params(&format!("{}/drives", self.api_base), &params)?;
            let v = self.get_json(url).await?;
            if let Some(arr) = v.get("drives").and_then(|d| d.as_array()) {
                drives.extend(arr.iter().cloned());
            }
            let next = v
                .get("nextPageToken")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string());
            if next.is_none() || next == page_token {
                break;
            }
            page_token = next;
        }
        Ok(drives)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_default_to_google_and_rebase_on_base_url() {
        let (api, token) = endpoints(None);
        assert_eq!(api, "https://www.googleapis.com/drive/v3");
        assert_eq!(token, "https://oauth2.googleapis.com/token");
        // A trailing slash on the override is tolerated.
        let (api, token) = endpoints(Some("http://localhost:8000/"));
        assert_eq!(api, "http://localhost:8000/drive/v3");
        assert_eq!(token, "http://localhost:8000/oauth2/token");
    }

    #[test]
    fn backoff_is_exponential_jittered_and_capped() {
        for _ in 0..100 {
            let d0 = backoff_delay(0);
            assert!(
                d0 >= Duration::from_secs(1) && d0 <= Duration::from_millis(2000),
                "{d0:?}"
            );
            let d2 = backoff_delay(2);
            assert!(
                d2 >= Duration::from_secs(4) && d2 <= Duration::from_millis(5000),
                "{d2:?}"
            );
            // large n → capped at MAX_BACKOFF; 2^4=16s + jitter > cap
            assert_eq!(backoff_delay(4), MAX_BACKOFF);
            assert_eq!(backoff_delay(10), MAX_BACKOFF);
        }
    }
}
