//! Slack Web API client for the Slack mount.
//!
//! Three things make this unlike the other accessors in this module:
//!
//! - **Slack signals failure inside a 200.** Every method answers
//!   `{"ok": true, …}` or `{"ok": false, "error": "…"}` with HTTP 200 either way,
//!   so a status-only check reads every failure as an empty success.
//!   [`SlackAccessor::call`] is the single gate on the API (file downloads aside,
//!   which are plain HTTP) and it is what enforces the `ok` check.
//! - **It reads as a person, not as an app.** The mount is one person's own view
//!   of their Slack, so the credential is that person's user token — see
//!   [`SlackConfig::user_token`] for why a bot token cannot express the same
//!   thing.
//! - **Tokens don't expire.** A user/bot token is long-lived *unless the app opts
//!   into token rotation*, which this client does not implement: there is no
//!   refresh loop, so a rotating app's mount would break when its token expires.
//!   The `code` exchange at mount-create ([`exchange_slack_code`]) is the only
//!   OAuth step.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLACK_API: &str = "https://slack.com/api";

/// The API origin for a config: real Slack by default, or `{base_url}/slack/api`
/// when set — the enterprise-mock/gateway layout.
fn api_base(base_url: Option<&str>) -> String {
    match base_url {
        Some(b) => format!("{}/slack/api", b.trim_end_matches('/')),
        None => SLACK_API.to_string(),
    }
}

/// Retry budget for a rate-limited/5xx request. Slack rate-limits per method per
/// workspace, and a listing fans out per-conversation calls, so a burst can trip
/// it; a bounded retry keeps a transient limit from failing the whole read.
const MAX_RETRIES: u32 = 5;
/// Cap on a single retry wait, deliberately shorter than a `Retry-After` Slack may
/// send: these calls sit behind a FUSE op the agent blocks on, so a throttled read
/// fails rather than wedging the guest for a minute. Raising this means moving the
/// wait off the FUSE path, not just raising the number.
const MAX_BACKOFF: Duration = Duration::from_secs(16);
/// Jitter ceiling, recomputed per retry so concurrent readers don't retry in
/// lockstep.
const JITTER_MAX_MS: u64 = 1000;

/// Exponential backoff with jitter for retry `n` (0-based).
fn backoff_delay(n: u32) -> Duration {
    let base = Duration::from_secs(1u64 << n.min(16));
    let jitter = Duration::from_millis(fastrand::u64(0..=JITTER_MAX_MS));
    (base + jitter).min(MAX_BACKOFF)
}

/// The `Retry-After` delay, if present. Slack sends delta-seconds on a 429 (and
/// alongside `error: "ratelimited"`); the HTTP-date form is not honored (treated
/// as absent → falls back to [`backoff_delay`]).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Items per page requested. Slack's ceiling is 1000 but it recommends no more
/// than 200 — and it is only a request: a rate-limited app gets 15 objects per
/// `conversations.history` response regardless, so a busy day pages.
const PAGE_LIMIT: usize = 200;
/// Pages one listing will walk before truncating (logged). A backstop against an
/// unbounded cursor loop, not a budget.
const MAX_PAGES: usize = 50;

/// A Slack API error carried out of [`SlackAccessor::call`], preserving the
/// machine-readable `error` code so callers can classify it — which is what
/// makes the resource's soft-fail set (`not_in_channel`, `missing_scope`, …)
/// possible without string-matching a formatted message.
#[derive(Debug, Clone)]
pub struct SlackApiError {
    /// The Slack method that failed, e.g. `conversations.history`.
    pub method: String,
    /// Slack's `error` code, e.g. `not_in_channel`.
    pub code: String,
    /// `needed`/`provided` scopes, when Slack reports them (`missing_scope`).
    pub scopes: Option<String>,
}

impl std::fmt::Display for SlackApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "slack {}: {}", self.method, self.code)?;
        if let Some(s) = &self.scopes {
            write!(f, " ({s})")?;
        }
        Ok(())
    }
}

impl std::error::Error for SlackApiError {}

/// Errors that mean "this conversation is not readable by this token", as
/// opposed to "the request was wrong" or "Slack is down". A bot that was never
/// invited to one channel must not break the whole tree, so the resource treats
/// these as an empty result for that one conversation.
///
/// `not_authed`/`invalid_auth` are deliberately **not** here: those are about the
/// token itself, so every conversation would fail the same way and swallowing
/// them would present an empty workspace as a complete one.
const READ_DENIED: &[&str] = &[
    "not_in_channel",
    "channel_not_found",
    "missing_scope",
    "is_archived",
    "restricted_action",
    "no_permission",
];

/// Whether this error means the caller can't read that conversation (see
/// [`READ_DENIED`]).
pub fn is_read_denied(e: &anyhow::Error) -> bool {
    e.downcast_ref::<SlackApiError>()
        .is_some_and(|s| READ_DENIED.contains(&s.code.as_str()))
}

/// Result of the mount-create code exchange. `team_name`/`team_id` are the
/// mount's stable identity (a token can be re-issued; the workspace it points at
/// doesn't change), shown in mount info so the UI can tell mounts apart.
pub struct SlackExchange {
    /// The installing user's token — the mount's primary credential (see
    /// [`SlackConfig::user_token`]). Present when the consent requested user
    /// scopes.
    pub user_token: Option<String>,
    /// The bot token, when the install also requested bot scopes. A fallback.
    pub bot_token: Option<String>,
    pub team_id: String,
    pub team_name: String,
}

/// Exchange an OAuth authorization `code` for the workspace's tokens
/// (`oauth.v2.access`, confidential client, server-side). Run at mount-create so
/// the browser never handles the client secret. `base_url` overrides the Slack
/// host (mock/gateway deployments — see [`SlackConfig::base_url`]); `None` =
/// production.
pub async fn exchange_slack_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    base_url: Option<&str>,
) -> anyhow::Result<SlackExchange> {
    // Bounded: this runs inside the create_mount HTTP handler, and a bare
    // reqwest client has NO default timeout — a hung upstream would hang the
    // mount creation indefinitely.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let resp = client
        .post(format!("{}/oauth.v2.access", api_base(base_url)))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("slack code exchange {status}: {body}");
    }
    let v: Value = serde_json::from_str(&body)?;
    // Same 200-with-ok:false convention as the rest of the API.
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        anyhow::bail!(
            "slack code exchange failed: {}",
            v.get("error").and_then(Value::as_str).unwrap_or("unknown")
        );
    }
    // The user token is what the mount wants (see `SlackConfig::user_token`); the
    // bot token is whatever the install also happened to grant. Either alone is a
    // usable mount, so neither is required here — but both absent is not.
    let user_token = v
        .get("authed_user")
        .and_then(|u| u.get("access_token"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(String::from);
    let bot_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(String::from);
    if user_token.is_none() && bot_token.is_none() {
        anyhow::bail!(
            "oauth.v2.access returned no token; the app must request user scopes \
             (recommended: the mount reads as the installing user) or bot scopes"
        );
    }
    if user_token.is_none() {
        // Worth saying out loud: the mount will be limited to the bot's own
        // channel memberships, with no DMs and no search.
        tracing::warn!(
            "slack install granted no user token; the mount will see only the bot's \
             conversations (no DMs, no search)"
        );
    }
    let team = v.get("team");
    let team_id = team
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // The workspace name is the mount's display identity, so a missing one is a
    // half-identified mount — fail the create with a clear message instead.
    let team_name = team
        .and_then(|t| t.get("name"))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("oauth.v2.access returned no team name; retry the mount"))?;
    Ok(SlackExchange {
        user_token,
        bot_token,
        team_id,
        team_name,
    })
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    /// User token (`xoxp-…`) — the mount's primary credential, because the mount
    /// is one person's own view of their Slack.
    ///
    /// A workspace VFS is a personal space (one owner per workspace row), so what
    /// belongs in it is what that person sees in their own Slack client:
    /// every conversation they are in, their DMs, and search. A bot token cannot
    /// express that — it is a separate member with its own membership, blind to
    /// anyone's DMs, and Slack does not offer it search at all.
    ///
    /// `None` falls back to [`Self::bot_token`], which still serves the tree for
    /// the conversations the bot was invited to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_token: Option<String>,
    /// Bot token (`xoxb-…`), when the install requested bot scopes. Only used as
    /// a fallback when there is no user token: it sees only conversations the bot
    /// is a member of, no DMs, and cannot search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    /// The Slack workspace's own id and name, resolved at mount-create
    /// ([`exchange_slack_code`]). Identity across token re-issues; shown in
    /// mount info.
    pub team_id: String,
    pub team_name: String,
    /// Alternative API origin (an enterprise mock or gateway): requests go to
    /// `{base_url}/slack/api` instead of `slack.com/api`. `None` = production.
    /// Deployment-level only — the exchange endpoint receives the app's client
    /// secret, so this must never be user-suppliable: it is NOT part of the
    /// mount-create API; the backend injects it from its own config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Holds one Slack workspace's tokens. Read-only: the mount serves history,
/// profiles and file bytes, and nothing here posts.
pub struct SlackAccessor {
    client: reqwest::Client,
    config: SlackConfig,
    /// Resolved API origin (`…/slack/api`) — real Slack or the config's
    /// `base_url` (see [`api_base`]).
    api_base: String,
}

impl SlackAccessor {
    pub fn new(config: &SlackConfig) -> anyhow::Result<Self> {
        // One credential is the minimum: with neither, every call would 401 and
        // the mount would present an empty workspace as a complete one. Rejecting
        // here makes it a mount-create failure with a clear cause instead.
        if token(&config.user_token).is_none() && token(&config.bot_token).is_none() {
            anyhow::bail!("slack mount has neither a user token nor a bot token");
        }
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
            api_base: api_base(config.base_url.as_deref()),
        })
    }

    /// Whether this mount could search at all. Slack refuses `search.messages` to
    /// bot tokens, so a bot-only mount has no search — callers ask this rather
    /// than discovering it from an error.
    ///
    /// A user token makes search *possible*, not certain: whether the install
    /// actually granted `search:read` is not visible in the token, so a call can
    /// still come back `missing_scope`.
    pub fn search_available(&self) -> bool {
        token(&self.config.user_token).is_some()
    }

    /// The token every call uses: the user's own, falling back to the bot's.
    ///
    /// Deliberately not per-method. The mount is one person's view of their Slack,
    /// so reading it as that person is the point — a bot token would silently
    /// narrow the tree to the bot's own memberships and drop DMs entirely. The
    /// fallback exists only for a bot-only install.
    fn token(&self) -> &str {
        token(&self.config.user_token)
            .or_else(|| token(&self.config.bot_token))
            // `new` rejects a config with neither, so one is always present.
            .unwrap_or_default()
    }

    /// Call one Slack method and return its (`ok: true`) body.
    ///
    /// The single gate on the API. Slack answers HTTP 200 with `{"ok": false,
    /// "error": …}` for application errors, so this checks `ok` and turns a
    /// failure into a typed [`SlackApiError`] — without it, every permission
    /// error and rate limit would read as a successful empty response. A 429 or
    /// 5xx (transport-level) and `error: "ratelimited"` (application-level) both
    /// retry with bounded backoff.
    async fn call(&self, method: &str, params: &[(&str, String)]) -> anyhow::Result<Value> {
        let url = reqwest::Url::parse_with_params(
            &format!("{}/{method}", self.api_base),
            params.iter().map(|(k, v)| (*k, v.as_str())),
        )?;
        let token = self.token();
        let mut retries = 0u32;
        loop {
            let resp = self
                .client
                .get(url.clone())
                .bearer_auth(token)
                .send()
                .await?;
            let status = resp.status();
            if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                && retries < MAX_RETRIES
            {
                let wait = match retry_after(&resp) {
                    Some(d) => d.min(MAX_BACKOFF),
                    None => backoff_delay(retries),
                };
                retries += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            let v: Value = resp.error_for_status()?.json().await?;
            if v.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(v);
            }
            let code = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error")
                .to_string();
            // Slack also reports a rate limit in-band (200 + ok:false); retry it
            // on the same budget as the 429.
            if code == "ratelimited" && retries < MAX_RETRIES {
                let wait = backoff_delay(retries);
                retries += 1;
                tokio::time::sleep(wait).await;
                continue;
            }
            // `missing_scope` carries what was needed vs granted — the difference
            // between a mystery and an actionable message.
            let scopes = match (v.get("needed"), v.get("provided")) {
                (Some(n), p) => Some(format!(
                    "needed: {}; provided: {}",
                    n.as_str().unwrap_or("?"),
                    p.and_then(Value::as_str).unwrap_or("(none)")
                )),
                _ => None,
            };
            return Err(SlackApiError {
                method: method.to_string(),
                code,
                scopes,
            }
            .into());
        }
    }

    /// Walk a cursor-paginated method, collecting `items_key` across pages.
    /// Stops at [`MAX_PAGES`] (logged) so a pathological workspace can't
    /// paginate without bound.
    async fn paginate(
        &self,
        method: &str,
        params: &[(&str, String)],
        items_key: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for page in 0..MAX_PAGES {
            let mut p = params.to_vec();
            p.push(("limit", PAGE_LIMIT.to_string()));
            if let Some(c) = &cursor {
                p.push(("cursor", c.clone()));
            }
            let v = self.call(method, &p).await?;
            if let Some(arr) = v.get(items_key).and_then(Value::as_array) {
                out.extend(arr.iter().cloned());
            }
            cursor = v
                .get("response_metadata")
                .and_then(|m| m.get("next_cursor"))
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(String::from);
            if cursor.is_none() {
                return Ok(out);
            }
            if page + 1 == MAX_PAGES {
                tracing::warn!(
                    "slack {method}: stopped at {MAX_PAGES} pages ({} items); rest omitted",
                    out.len()
                );
            }
        }
        Ok(out)
    }

    /// Conversations of `types` the token can see (`conversations.list`).
    /// Archived channels are excluded — they list fine but their history is
    /// denied, so they would only ever show as empty date trees.
    pub async fn list_conversations(&self, types: &str) -> anyhow::Result<Vec<Value>> {
        self.paginate(
            "conversations.list",
            &[
                ("types", types.to_string()),
                ("exclude_archived", "true".to_string()),
            ],
            "channels",
        )
        .await
    }

    /// One conversation's metadata (`conversations.info`) — used to learn
    /// `created` when a deep path is resolved without its parent listing.
    pub async fn conversation_info(&self, channel: &str) -> anyhow::Result<Value> {
        let v = self
            .call("conversations.info", &[("channel", channel.to_string())])
            .await?;
        Ok(v.get("channel").cloned().unwrap_or(Value::Null))
    }

    /// Top-level messages in `channel` between `oldest` and `latest` (unix
    /// seconds as Slack's string ts), oldest-first.
    ///
    /// `conversations.history` returns thread **roots** and standalone messages
    /// only; a root's replies need [`Self::conversation_replies`]. That split is
    /// why the mount serves them as separate files.
    pub async fn conversation_history(
        &self,
        channel: &str,
        oldest: &str,
        latest: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let mut msgs = self
            .paginate(
                "conversations.history",
                &[
                    ("channel", channel.to_string()),
                    ("oldest", oldest.to_string()),
                    ("latest", latest.to_string()),
                    ("inclusive", "true".to_string()),
                ],
                "messages",
            )
            .await?;
        msgs.sort_by(|a, b| ts_of(a).total_cmp(&ts_of(b)));
        Ok(msgs)
    }

    /// The newest message's ts in `channel`, or `None` when it has none. One
    /// call (`limit=1`) — the upper bound of the channel's date range.
    pub async fn latest_message_ts(&self, channel: &str) -> anyhow::Result<Option<f64>> {
        let v = self
            .call(
                "conversations.history",
                &[("channel", channel.to_string()), ("limit", "1".to_string())],
            )
            .await?;
        Ok(v.get("messages")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .map(ts_of))
    }

    /// A thread: its root followed by every reply, oldest-first.
    pub async fn conversation_replies(
        &self,
        channel: &str,
        ts: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let mut msgs = self
            .paginate(
                "conversations.replies",
                &[("channel", channel.to_string()), ("ts", ts.to_string())],
                "messages",
            )
            .await?;
        msgs.sort_by(|a, b| ts_of(a).total_cmp(&ts_of(b)));
        Ok(msgs)
    }

    /// Workspace members (`users.list`), bots/deleted/Slackbot included — the
    /// resource decides what to list, and a message's author may well be a bot
    /// whose profile the reader wants to resolve.
    ///
    /// One call answers both naming and the `users/` profiles: the response
    /// carries each member's full profile, not just their id. `profile.email` is
    /// among those fields only when the install granted `users:read.email`;
    /// Slack simply omits it otherwise, so nothing here has to ask.
    pub async fn list_users(&self) -> anyhow::Result<Vec<Value>> {
        self.paginate("users.list", &[], "members").await
    }

    /// One user (`users.info`), for an id the member list didn't cover — a DM
    /// partner outside the workspace's own member list (measured: Slack's own
    /// `USLACK` account).
    pub async fn user_info(&self, user: &str) -> anyhow::Result<Value> {
        let v = self
            .call("users.info", &[("user", user.to_string())])
            .await?;
        Ok(v.get("user").cloned().unwrap_or(Value::Null))
    }

    /// Download a Slack-hosted file. `url` comes from a message's `files[]`
    /// (`url_private_download`) and **requires** the bearer token — unauthorized
    /// it answers a 200 HTML login page rather than an error, so an unchecked
    /// download silently yields a web page instead of the file.
    ///
    /// `range` maps to an HTTP `Range` header; Slack's file CDN honors it, and a
    /// server that ignores it answers 200 with the whole body, which the caller
    /// (the resource) slices.
    pub async fn download_file(
        &self,
        url: &str,
        range: Option<std::ops::Range<u64>>,
    ) -> anyhow::Result<(Vec<u8>, bool)> {
        // Only ever follow Slack's own file hosts: `url_private_download` comes
        // from API data, and sending the mount's token to whatever host a
        // message's file metadata names would leak it.
        let parsed = reqwest::Url::parse(url)?;
        let allowed = self.config.base_url.as_deref().and_then(|b| {
            reqwest::Url::parse(b)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
        });
        let host = parsed.host_str().unwrap_or_default().to_string();
        let ok_host = match &allowed {
            // Mock/gateway deployment: the file host is the configured origin.
            Some(h) => &host == h,
            None => host == "slack.com" || host.ends_with(".slack.com"),
        };
        if !ok_host {
            anyhow::bail!(
                "slack file url host {host:?} is not a Slack host; refusing to send token"
            );
        }
        // Same credential as the API calls: a file the person can see in Slack is
        // one their own token can fetch.
        let mut req = self.client.get(parsed).bearer_auth(self.token());
        let ranged = range.is_some();
        if let Some(r) = &range {
            // Inclusive-end, per HTTP: a 0..10 read asks for bytes 0-9.
            req = req.header(
                reqwest::header::RANGE,
                format!("bytes={}-{}", r.start, r.end.saturating_sub(1).max(r.start)),
            );
        }
        let resp = req.send().await?.error_for_status()?;
        // 206 means the range was applied; a 200 to a ranged request means it
        // wasn't, and the caller must slice the full body itself.
        let served_range = ranged && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        Ok((resp.bytes().await?.to_vec(), served_range))
    }

    /// Message search over Slack's own index (`search.messages`), which reaches
    /// inside files Slack has indexed. Needs the user token — see
    /// [`Self::search_available`].
    pub async fn search_messages(&self, query: &str, count: usize) -> anyhow::Result<Value> {
        if !self.search_available() {
            anyhow::bail!(
                "slack search needs a user token (xoxp-); this mount was installed with a bot token only"
            );
        }
        self.call(
            "search.messages",
            &[
                ("query", query.to_string()),
                ("count", count.to_string()),
                ("sort", "timestamp".to_string()),
            ],
        )
        .await
    }
}

/// A configured token, treating an empty string as absent (a round-tripped
/// config can carry `""` where a scope was never granted).
fn token(t: &Option<String>) -> Option<&str> {
    t.as_deref().filter(|t| !t.is_empty())
}

/// A message's `ts` as a float (0.0 when absent/unparseable), for ordering.
fn ts_of(m: &Value) -> f64 {
    m.get("ts")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_defaults_to_slack_and_honors_the_override() {
        assert_eq!(api_base(None), "https://slack.com/api");
        // The mock mounts Slack under /slack/api, and a trailing slash in the
        // configured origin must not double up.
        assert_eq!(
            api_base(Some("http://localhost:8000")),
            "http://localhost:8000/slack/api"
        );
        assert_eq!(
            api_base(Some("http://localhost:8000/")),
            "http://localhost:8000/slack/api"
        );
    }

    /// Retry `n` waits in `[2^n s, 2^n s + 1s]`, capped at [`MAX_BACKOFF`]. The
    /// jitter floor matters as much as the ceiling: without it, the concurrent
    /// readers that tripped this limit together would retry together and trip it
    /// again.
    #[test]
    fn backoff_is_exponential_jittered_and_capped() {
        for _ in 0..100 {
            for n in 0..3u32 {
                let base = Duration::from_secs(1u64 << n);
                let d = backoff_delay(n);
                assert!(
                    d >= base && d <= base + Duration::from_millis(JITTER_MAX_MS),
                    "n={n}: {d:?}"
                );
            }
            // 2^4 = 16s already meets the cap, so jitter cannot push past it.
            assert_eq!(backoff_delay(4), MAX_BACKOFF);
            // The shift is clamped, so a large n saturates instead of overflowing.
            assert_eq!(backoff_delay(64), MAX_BACKOFF);
        }
    }

    fn config(user: Option<&str>, bot: Option<&str>) -> SlackConfig {
        SlackConfig {
            user_token: user.map(String::from),
            bot_token: bot.map(String::from),
            team_id: "T1".into(),
            team_name: "acme".into(),
            base_url: None,
        }
    }

    /// The user token wins when both are present. It is one token for every call
    /// rather than a per-method choice: reading as the person is the point, and a
    /// bot token would narrow the tree to the bot's own memberships and drop DMs.
    #[test]
    fn the_user_token_wins_when_both_are_present() {
        let a = SlackAccessor::new(&config(Some("xoxp-user"), Some("xoxb-bot"))).unwrap();
        assert_eq!(a.token(), "xoxp-user");
        assert!(a.search_available());
    }

    /// A bot-only install still serves a tree (the bot's own conversations), so
    /// the bot token is a fallback rather than an error — but search is genuinely
    /// impossible, and must be reported rather than attempted.
    #[tokio::test]
    async fn a_bot_only_install_falls_back_and_cannot_search() {
        let a = SlackAccessor::new(&config(None, Some("xoxb-bot"))).unwrap();
        assert_eq!(a.token(), "xoxb-bot");
        assert!(!a.search_available());
        // No network: the guard rejects before a request is built.
        assert!(a.search_messages("anything", 20).await.is_err());
    }

    /// A user-only install is the intended shape: no bot scopes needed at all.
    #[test]
    fn a_user_only_install_is_enough() {
        let a = SlackAccessor::new(&config(Some("xoxp-user"), None)).unwrap();
        assert_eq!(a.token(), "xoxp-user");
        assert!(a.search_available());
    }

    /// With no credential every call would 401, and the mount would show an empty
    /// workspace as a complete one — so it must fail at construction instead.
    /// An empty string counts as absent (a round-tripped config can carry `""`).
    #[test]
    fn a_config_with_no_token_is_rejected() {
        assert!(SlackAccessor::new(&config(None, None)).is_err());
        assert!(SlackAccessor::new(&config(Some(""), Some(""))).is_err());
        // And an empty user token falls through to a real bot token.
        let a = SlackAccessor::new(&config(Some(""), Some("xoxb-bot"))).unwrap();
        assert_eq!(a.token(), "xoxb-bot");
        assert!(!a.search_available());
    }

    /// The soft-fail classification must key off Slack's `error` code, not a
    /// formatted string, and must NOT swallow token-level failures — those apply
    /// to every conversation, so treating them as "this channel is empty" would
    /// present an empty workspace as a complete one.
    #[test]
    fn read_denied_covers_per_channel_errors_only() {
        let err = |code: &str| -> anyhow::Error {
            SlackApiError {
                method: "conversations.history".into(),
                code: code.into(),
                scopes: None,
            }
            .into()
        };
        for code in ["not_in_channel", "missing_scope", "is_archived"] {
            assert!(is_read_denied(&err(code)), "{code} should be soft");
        }
        for code in ["not_authed", "invalid_auth", "fatal_error"] {
            assert!(!is_read_denied(&err(code)), "{code} must propagate");
        }
        // A non-Slack error is never a permission denial.
        assert!(!is_read_denied(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn missing_scope_error_names_what_was_needed() {
        let e = SlackApiError {
            method: "conversations.history".into(),
            code: "missing_scope".into(),
            scopes: Some("needed: channels:history; provided: channels:read".into()),
        };
        let s = e.to_string();
        assert!(s.contains("channels:history"), "{s}");
        assert!(s.contains("conversations.history"), "{s}");
    }

    #[test]
    fn messages_order_by_ts() {
        let m = |ts: &str| serde_json::json!({"ts": ts});
        let mut v = [m("1754210000.000200"), m("1754209000.000100")];
        v.sort_by(|a, b| ts_of(a).total_cmp(&ts_of(b)));
        assert_eq!(ts_of(&v[0]), 1_754_209_000.000_1);
        // A message without a ts sorts first rather than panicking.
        assert_eq!(ts_of(&serde_json::json!({})), 0.0);
    }

    /// A file download must refuse to send the mount's token to a host that isn't
    /// Slack's — `url_private_download` is API-supplied data, and the token it
    /// would leak is the person's own Slack access.
    #[tokio::test]
    async fn download_refuses_a_non_slack_host() {
        let a = SlackAccessor::new(&config(Some("xoxp-user"), None)).unwrap();
        let e = a
            .download_file("https://evil.example.com/f.pdf", None)
            .await
            .expect_err("must refuse");
        assert!(e.to_string().contains("not a Slack host"), "{e}");
        // Slack's own hosts pass the check (this one fails later, on connect).
        let e = a
            .download_file("https://files.slack.com/nope", None)
            .await
            .expect_err("no network in tests");
        assert!(!e.to_string().contains("not a Slack host"), "{e}");
    }
}
