//! GitHub: one repository's git objects, read through a fine-grained token.
//!
//! Git is content-addressed, which decides most of this file. A tree listing names
//! every child's blob SHA, and a blob SHA is a hash of the bytes — so a listing
//! already carries an exact length and an exact version tag for everything in it, and
//! nothing here ever has to ask "has this changed?". What a listing cannot do is serve
//! *part* of a blob: the API has no range, so [`GithubAccessor::blob`] is whole-object
//! only and the resource says so through
//! [`FileStat::serves_whole`](crate::vfs::resource::FileStat::serves_whole).
//!
//! Authentication is a fine-grained personal access token, scoped by the user to the
//! repositories and permissions they choose (`Contents: read-only` is enough to mount).
//! That is narrower than an OAuth app can be — `repo` is the only scope covering private
//! repositories and it grants write to *all* of them — so the token the user pastes is
//! the least-privilege option short of a GitHub App. The cost is that it expires: every
//! request goes through [`GithubAccessor::auth`], which is also the seam a GitHub App
//! would replace (JWT → installation token, cached by expiry) without touching a caller.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const API: &str = "https://api.github.com";
/// REST API version to pin. Sent on every request so a later default can't silently
/// reshape a response: `2026-03-10` split submodules out of the contents listing and
/// dropped `rate` from `/rate_limit`, neither of which this code reads, and the git
/// tree/blob endpoints it does read were unchanged.
const API_VERSION: &str = "2026-03-10";
/// GitHub rejects a request with no `User-Agent`.
const UA: &str = "agent-k-workspace";
/// Raw bytes rather than the default base64-in-JSON envelope — a blob is served as the
/// file it is, so there is nothing to decode and no 1.33× inflation on the wire.
const RAW_MEDIA: &str = "application/vnd.github.raw+json";

const MAX_RETRIES: u32 = 5;
/// Ceiling on one backoff sleep, so a transient failure (429, 5xx, a secondary limit)
/// climbs 1 → 2 → 4 → 8 → 16s and stops there instead of growing without bound.
const MAX_BACKOFF: Duration = Duration::from_secs(16);
/// Longest this will hold a caller while an exhausted hourly window turns over.
///
/// A threshold, not a cap — and the distinction is the whole reason it is named
/// separately. A backoff escalates because it does not know when the condition clears; a
/// spent quota returns at one instant GitHub already told us, so there is nothing to
/// escalate toward and the only question is whether that instant is near enough to wait
/// for (see [`reset_wait`]). It equals [`MAX_BACKOFF`] today because both are bounded by
/// the same caller's patience, but they answer different questions and retuning one must
/// not silently move the other.
const MAX_QUOTA_WAIT: Duration = Duration::from_secs(16);
const JITTER_MAX_MS: u64 = 1000;

/// Blob ceiling for the git blobs endpoint. A larger file is not readable through this
/// API at all, so the resource reports the size the tree gave and the read fails with
/// this named limit rather than an opaque 403.
pub const MAX_BLOB_BYTES: u64 = 100 << 20;

/// Rows per page for the issue and pull listings — GitHub's maximum, so a capped
/// listing costs as few requests as it can.
const PER_PAGE: usize = 100;

/// How many issues (and pulls) a listing keeps by default.
///
/// Unlike a git tree, these have no bound: a repository with 20k issues would cost 200
/// requests to enumerate, on a listing an agent reads to orient itself. So the listing
/// keeps the most recently *updated* N — the ones an agent asking "what is going on
/// here" wants — and anything outside it stays reachable by number, because
/// [`Resource::listings_complete`](crate::vfs::resource::Resource::listings_complete)
/// is `false` and a `stat` may therefore probe.
pub const DEFAULT_INDEX_CAP: usize = 200;

/// Owner and repository names: GitHub allows letters, digits, `-`, `_` and `.`, so
/// anything else is rejected before it reaches a URL. The `url` crate honors `../`,
/// `?` and `#`, so an unchecked segment could address a different endpoint entirely.
fn valid_repo_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A branch name, which unlike an owner or repo legitimately contains `/`
/// (`release/2026-03`). Everything git itself forbids in a ref is rejected here too, so
/// what remains cannot climb out of the `git/ref/heads/` prefix it gets appended to.
fn valid_branch(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.ends_with(".lock")
        && !s.contains("..")
        && !s.contains("//")
        && !s.contains("@{")
        && !s.chars().any(|c| {
            c.is_ascii_control()
                || c.is_whitespace()
                || matches!(c, '?' | '#' | '~' | '^' | ':' | '[' | '*' | '\\' | '%')
        })
}

/// A git object id: hex, either SHA-1 (40) or SHA-256 (64, for repositories on git's
/// newer object format).
fn valid_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Exponential backoff with jitter for retry `n` (0-based): `min(2^n s + rand(0..=1s),
/// MAX_BACKOFF)`.
fn backoff_delay(n: u32) -> Duration {
    let base = Duration::from_secs(1u64 << n.min(16));
    let jitter = Duration::from_millis(fastrand::u64(0..=JITTER_MAX_MS));
    (base + jitter).min(MAX_BACKOFF)
}

/// The `Retry-After` delay, if present (delta-seconds only; the HTTP-date form is
/// treated as absent and falls back to [`backoff_delay`]).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?;
    parse_delta_seconds(raw)
}

fn parse_delta_seconds(raw: &str) -> Option<Duration> {
    raw.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Whether the hourly quota is spent (`x-ratelimit-remaining: 0`).
///
/// This is the one rate limit that is not retried on a ladder. The others are retried
/// because nobody knows when they clear; this one clears at a time GitHub states outright,
/// so the response to it is a single wait or none at all — [`reset_wait`] decides which.
fn quota_exhausted(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "0")
        .unwrap_or(false)
}

/// How long to wait out a spent hourly quota, or `None` if waiting is not worth it.
///
/// The window resets *wholesale* at `x-ratelimit-reset` rather than trickling back, so
/// there is exactly one useful moment to retry and no reason to back off toward it. And
/// that moment is the end of the **current** window, not an hour from this request: a
/// window that opened 59 minutes ago ends in seconds, so a caller that gives up on sight
/// of `remaining: 0` often throws away a request it could have had for a one-second wait.
/// Past [`MAX_QUOTA_WAIT`] there is nothing a caller behind a FUSE operation can do with
/// the answer, so it fails instead, with the reset time named.
fn reset_wait(resp: &reqwest::Response) -> Option<Duration> {
    // A second of margin: our clock and GitHub's need not agree, and retrying a hair
    // early just spends a request to be told the same thing.
    let wait = Duration::from_secs(reset_in(resp)?) + Duration::from_secs(1);
    (wait <= MAX_QUOTA_WAIT).then_some(wait)
}

/// Seconds until the hourly quota resets, for the message on a spent quota.
fn reset_in(resp: &reqwest::Response) -> Option<u64> {
    let epoch = resp
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(epoch.saturating_sub(now))
}

fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// One credential and the scope it reaches.
///
/// An owner and a token arrive together because GitHub binds them together: a
/// fine-grained token names exactly one *resource owner* when it is created, so no single
/// one of them can serve two owners' private repositories. A mount that spans owners
/// therefore holds one of these per owner, and that is what keeps each least-privilege —
/// the alternative, one credential broad enough to reach every owner, is a classic token
/// carrying write access to everything the account can see.
#[derive(Clone, Serialize, Deserialize)]
pub struct GithubSource {
    /// The user or organization that owns the repositories, and the token's resource
    /// owner: the first path segment of a repository URL.
    pub owner: String,
    /// Fine-grained personal access token scoped to `owner`, `Contents: read-only` at
    /// minimum. Long-lived but *expiring* — see the module docs for why a 401 is reported
    /// rather than retried.
    pub token: String,
    /// Serve only this repository rather than every one the token reaches under `owner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Branch to serve, or a full commit SHA to pin to. Meaningful only alongside `repo`:
    /// one ref cannot describe a set of repositories, and a commit id means nothing
    /// across them. `None` = each repository's own default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
}

/// A GitHub mount: one credential per owner, plus the settings they share.
#[derive(Clone, Serialize, Deserialize)]
pub struct GithubConfig {
    /// One entry per owner. Two entries naming the same owner is a configuration error —
    /// the second credential would be unreachable.
    pub sources: Vec<GithubSource>,
    /// Newest-updated ceiling for the `issues/` and `pulls/` listings, and the ceiling on
    /// how many repositories an owner lists. `None` = [`DEFAULT_INDEX_CAP`]. A performance
    /// knob only — nothing is hidden by it, since anything outside a capped listing is
    /// still reachable by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_cap: Option<usize>,
    /// API origin override, for GitHub Enterprise Server and for pointing the provider
    /// at a loopback mock in tests. Deployment-level only: this is where the user's
    /// token gets sent, so the HTTP API never accepts it from a request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

/// What a tree entry's mode/type says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A file (including a symlink, whose content is its target path).
    Blob,
    /// A subdirectory.
    Tree,
    /// A submodule: a commit in another repository, which this mount cannot serve.
    Submodule,
}

/// One row of a git tree.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    /// Slash-separated path, relative to the tree that was asked for. A recursive
    /// listing gives full paths (`src/vfs/mod.rs`); a shallow one gives bare names.
    pub path: String,
    pub kind: EntryKind,
    pub sha: String,
    /// Exact byte length, present for blobs.
    pub size: Option<u64>,
}

/// A git tree as fetched: its own object id, its rows, and whether GitHub cut the
/// listing short.
#[derive(Clone, Debug)]
pub struct Tree {
    pub sha: String,
    /// GitHub caps a recursive listing at 100k entries / 7 MB. A caller must not treat
    /// a truncated listing as complete — see
    /// [`Resource::listings_complete`](crate::vfs::resource::Resource::listings_complete).
    pub truncated: bool,
    pub entries: Vec<TreeEntry>,
}

/// One row of a repository listing.
#[derive(Clone, Debug)]
pub struct RepoRow {
    pub name: String,
    /// `pushed_at`, RFC 3339 — the closest thing a repository has to an mtime.
    pub pushed_at: Option<String>,
    /// Archived repositories are read-only history. They stay reachable by name but are
    /// left out of a listing, because a mount whose directory is half dead repositories is
    /// mostly noise.
    pub archived: bool,
}

/// Holds the GitHub API client for **one owner and one credential** — the pairing GitHub
/// itself imposes (see [`GithubSource`]). A mount spanning owners holds several.
pub struct GithubAccessor {
    client: reqwest::Client,
    token: String,
    owner: String,
    /// From [`GithubSource::repo`]: serve only this repository under the owner.
    only_repo: Option<String>,
    git_ref: Option<String>,
    index_cap: usize,
    api_base: String,
}

impl GithubAccessor {
    /// Build the accessor for one source. `index_cap` and `api_base` come from the mount
    /// as a whole, since they are settings rather than credentials.
    pub fn new(
        source: &GithubSource,
        index_cap: Option<usize>,
        api_base: Option<&str>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            valid_repo_segment(&source.owner),
            "invalid github owner: {:?}",
            source.owner
        );
        if let Some(r) = &source.repo {
            anyhow::ensure!(valid_repo_segment(r), "invalid github repo: {r:?}");
        }
        if let Some(r) = &source.git_ref {
            anyhow::ensure!(
                valid_sha(r) || valid_branch(r),
                "invalid github ref: {r:?} (expected a branch name or a commit SHA)"
            );
            anyhow::ensure!(
                source.repo.is_some(),
                "github ref {r:?} needs a repo: one ref cannot describe every repository \
                 under {:?}",
                source.owner
            );
        }
        anyhow::ensure!(
            !source.token.trim().is_empty(),
            "github token for {:?} is empty",
            source.owner
        );
        Ok(Self {
            // Bound every request: a bare reqwest client has NO default timeout, and
            // these run behind the FUSE forward server, so a hung upstream would wedge
            // the guest FUSE op (and any process touching the mount) indefinitely.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .user_agent(UA)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            token: source.token.clone(),
            owner: source.owner.clone(),
            only_repo: source.repo.clone(),
            git_ref: source.git_ref.clone(),
            index_cap: index_cap.unwrap_or(DEFAULT_INDEX_CAP).max(1),
            api_base: api_base.unwrap_or(API).trim_end_matches('/').to_string(),
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The single repository this source is restricted to, if any.
    pub fn only_repo(&self) -> Option<&str> {
        self.only_repo.as_deref()
    }

    /// The configured ref, or `None` to mean the repository's default branch.
    pub fn configured_ref(&self) -> Option<&str> {
        self.git_ref.as_deref()
    }

    /// `{api_base}/repos/{owner}/{repo}{suffix}`.
    ///
    /// `repo` now arrives from a *path segment* rather than from config, so it is
    /// validated on every call: the `url` crate honors `../`, `?` and `#`, and an
    /// unchecked segment could address a different endpoint entirely.
    fn repo_url(&self, repo: &str, suffix: &str) -> anyhow::Result<String> {
        anyhow::ensure!(valid_repo_segment(repo), "invalid github repo: {repo:?}");
        Ok(format!(
            "{}/repos/{}/{}{}",
            self.api_base, self.owner, repo, suffix
        ))
    }

    /// The credential for one request, and the only place a caller's token is read.
    /// A GitHub App would replace this with a JWT-signed installation token cached by
    /// expiry; because every request goes through here, nothing else would change.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.token)
            .header("X-GitHub-Api-Version", API_VERSION)
    }

    /// Send with the retry ladder, then classify the status.
    ///
    /// Every call this accessor makes is an idempotent GET, so retrying a 5xx is always
    /// safe. Beyond that, three failures need answers of their own. A 401 means the token
    /// was rejected — a fine-grained token expires, and nothing here can renew it, so
    /// waiting cannot help and the message has to say so. A 404 is what GitHub returns
    /// both for a missing path and for a private repository the token cannot see, so it
    /// says both. And a spent hourly quota is waited out only when its window is about to
    /// turn over (see [`reset_wait`]) — the quota returns all at once, so the choice is
    /// between one well-timed wait and giving up, never a backoff ladder.
    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<reqwest::Response> {
        let mut retries = 0u32;
        loop {
            let Some(attempt) = req.try_clone() else {
                anyhow::bail!("github: request body is not retryable");
            };
            let resp = self.auth(attempt).send().await?;
            let status = resp.status();

            if status.is_success() {
                return Ok(resp);
            }

            // A 403 is GitHub's secondary rate limit *and* its permission denial, and
            // only the headers tell them apart. Reading the body consumes the response,
            // which is fine: a 403 this loop does not retry has failed either way, and
            // GitHub's explanation beats the bare status.
            let secondary_limit = status == reqwest::StatusCode::FORBIDDEN
                && (retry_after(&resp).is_some() || quota_exhausted(&resp));
            let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
                || secondary_limit;

            if retryable && retries < MAX_RETRIES {
                let wait = if quota_exhausted(&resp) {
                    // The hourly window returns in full at one instant, so that instant
                    // is the only wait worth making — and only when it is close enough to
                    // wait for. Otherwise stop here rather than sleep toward an hour.
                    match reset_wait(&resp) {
                        Some(d) => d,
                        None => return Err(self.classify(resp).await),
                    }
                } else {
                    // An explicit Retry-After wins, capped so the caller isn't blocked
                    // longer than the FUSE op can wait; otherwise backoff with jitter.
                    match retry_after(&resp) {
                        Some(d) => d.min(MAX_BACKOFF),
                        None => backoff_delay(retries),
                    }
                };
                retries += 1;
                tokio::time::sleep(wait).await;
                continue;
            }

            return Err(self.classify(resp).await);
        }
    }

    /// Turn a non-2xx response into the error a caller should see.
    async fn classify(&self, resp: reqwest::Response) -> anyhow::Error {
        let status = resp.status();
        let exhausted = quota_exhausted(&resp);
        let resets_in = reset_in(&resp);
        let body = resp.text().await.unwrap_or_default();
        let detail = first_chars(&body, 300);

        if exhausted {
            let when = match resets_in {
                Some(s) => format!("resets in {s}s"),
                None => "reset time unknown".to_string(),
            };
            return anyhow::anyhow!("github rate limit: hourly quota spent ({when})");
        }
        match status {
            reqwest::StatusCode::UNAUTHORIZED => anyhow::anyhow!(
                "github rejected the token (401): it has expired or was revoked — \
                 a fine-grained token cannot be renewed automatically, so the mount \
                 needs a new one. {detail}"
            ),
            reqwest::StatusCode::FORBIDDEN => anyhow::anyhow!(
                "github refused the request (403): the token for {:?} may lack `Contents: \
                 read-only`, or the organization has not approved it. {detail}",
                self.owner
            ),
            reqwest::StatusCode::NOT_FOUND => anyhow::anyhow!(
                "github 404 under {:?}: the path does not exist, or this token cannot see \
                 it (GitHub answers 404 rather than 403 for a private repository it will \
                 not disclose — an organization token awaiting approval reads this way \
                 too). {detail}",
                self.owner
            ),
            other => anyhow::anyhow!("github API {other}: {detail}"),
        }
    }

    async fn get_json(&self, url: String) -> anyhow::Result<Value> {
        let resp = self.send(self.client.get(url)).await?;
        Ok(resp.json().await?)
    }

    /// Repositories this credential reaches under its owner, most recently pushed first.
    ///
    /// `/user/repos` rather than `/users/{owner}/repos` or `/orgs/{owner}/repos`: it
    /// returns what the *token* can actually see, private repositories included, without
    /// this code having to know whether the owner is a user or an organization — the two
    /// owner-specific routes differ, and the user one silently omits private repositories.
    /// The account may belong to several owners, so the rows are filtered to this one.
    ///
    /// Archived repositories are dropped from the listing but stay reachable by name, the
    /// same bargain the issue cap makes.
    pub async fn list_repos(&self) -> anyhow::Result<Vec<RepoRow>> {
        let mut out: Vec<RepoRow> = Vec::new();
        let mut page = 1usize;
        // The cap counts repositories *kept*, so a page mostly full of other owners'
        // repositories doesn't end the walk early.
        while out.len() < self.index_cap {
            let url = format!(
                "{}/user/repos?affiliation=owner,collaborator,organization_member\
                 &sort=pushed&direction=desc&per_page={PER_PAGE}&page={page}",
                self.api_base
            );
            let rows = match self.get_json(url).await? {
                Value::Array(rows) => rows,
                _ => break,
            };
            let got = rows.len();
            out.extend(rows.iter().filter_map(|r| self.repo_row(r)));
            if got < PER_PAGE {
                break;
            }
            page += 1;
        }
        out.truncate(self.index_cap);
        Ok(out)
    }

    /// One repository listing row, if it belongs to this source's owner and is usable.
    fn repo_row(&self, row: &Value) -> Option<RepoRow> {
        let owner = row.get("owner")?.get("login")?.as_str()?;
        if !owner.eq_ignore_ascii_case(&self.owner) {
            return None;
        }
        let name = row.get("name")?.as_str()?;
        // A name this code could not put in a URL is not one it can serve.
        if !valid_repo_segment(name) {
            return None;
        }
        Some(RepoRow {
            name: name.to_string(),
            pushed_at: row
                .get("pushed_at")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            archived: row
                .get("archived")
                .and_then(|a| a.as_bool())
                .unwrap_or(false),
        })
    }

    /// One repository's metadata, for probing a name a listing didn't mention and for
    /// learning its default branch.
    pub async fn repo_meta(&self, repo: &str) -> anyhow::Result<Value> {
        self.get_json(self.repo_url(repo, "")?).await
    }

    /// A repository's default branch, for a source that named no ref.
    pub async fn default_branch(&self, repo: &str) -> anyhow::Result<String> {
        let v = self.repo_meta(repo).await?;
        v.get("default_branch")
            .and_then(|b| b.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("github: {}/{repo} has no default_branch", self.owner))
    }

    /// Resolve the mount's ref to a commit SHA.
    ///
    /// A SHA is used as given. A branch goes through `git/ref/heads/…` rather than the
    /// commits endpoint because that route takes the ref name as its trailing path —
    /// which is what makes a branch like `release/2026-03` addressable at all — and
    /// because it answers with just the object, not the commit's full file list.
    pub async fn resolve_commit(&self, repo: &str, git_ref: &str) -> anyhow::Result<String> {
        if valid_sha(git_ref) {
            return Ok(git_ref.to_string());
        }
        anyhow::ensure!(valid_branch(git_ref), "invalid github branch: {git_ref:?}");
        let v = self
            .get_json(self.repo_url(repo, &format!("/git/ref/heads/{git_ref}"))?)
            .await?;
        let sha = v
            .get("object")
            .and_then(|o| o.get("sha"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow::anyhow!("github: branch {git_ref:?} has no commit"))?;
        anyhow::ensure!(
            valid_sha(sha),
            "github returned a malformed commit id for {git_ref:?}"
        );
        Ok(sha.to_string())
    }

    /// One git tree. `tree_ish` is a commit or tree SHA — a commit resolves to the tree
    /// it points at. `recursive` asks for the whole subtree in one call, which is the
    /// difference between one request per mount and one per directory walked.
    pub async fn tree(&self, repo: &str, tree_ish: &str, recursive: bool) -> anyhow::Result<Tree> {
        anyhow::ensure!(valid_sha(tree_ish), "invalid github tree id: {tree_ish:?}");
        let suffix = if recursive {
            format!("/git/trees/{tree_ish}?recursive=1")
        } else {
            format!("/git/trees/{tree_ish}")
        };
        let v = self.get_json(self.repo_url(repo, &suffix)?).await?;
        let sha = v
            .get("sha")
            .and_then(|s| s.as_str())
            .unwrap_or(tree_ish)
            .to_string();
        let truncated = v
            .get("truncated")
            .and_then(|t| t.as_bool())
            .unwrap_or(false);
        let rows = v
            .get("tree")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let entries = rows.iter().filter_map(parse_entry).collect();
        Ok(Tree {
            sha,
            truncated,
            entries,
        })
    }

    /// A blob's bytes, whole. The API has no range, so this is the only shape a read
    /// can take; [`MAX_BLOB_BYTES`] is checked first so an oversized file fails by name
    /// rather than as an opaque upstream error.
    pub async fn blob(&self, repo: &str, sha: &str, size: Option<u64>) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(valid_sha(sha), "invalid github blob id: {sha:?}");
        if let Some(n) = size {
            anyhow::ensure!(
                n <= MAX_BLOB_BYTES,
                "github blob is {n} bytes; the git blobs endpoint serves at most \
                 {MAX_BLOB_BYTES} ({} MiB)",
                MAX_BLOB_BYTES >> 20
            );
        }
        let resp = self
            .send(
                self.client
                    .get(self.repo_url(repo, &format!("/git/blobs/{sha}"))?)
                    .header(reqwest::header::ACCEPT, RAW_MEDIA),
            )
            .await?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// How many rows the issue and pull listings keep.
    pub fn index_cap(&self) -> usize {
        self.index_cap
    }

    /// Walk `path`'s pages, newest-updated first, until the cap is filled or GitHub runs
    /// out of rows. A short page means the last one, so this stops without a probe.
    async fn list_paged(&self, repo: &str, path: &str) -> anyhow::Result<Vec<Value>> {
        let mut out: Vec<Value> = Vec::new();
        let mut page = 1usize;
        while out.len() < self.index_cap {
            let url = self.repo_url(
                repo,
                &format!(
                    "{path}?state=all&sort=updated&direction=desc&per_page={PER_PAGE}&page={page}"
                ),
            )?;
            let rows = match self.get_json(url).await? {
                Value::Array(rows) => rows,
                // An object here is an error envelope the status didn't flag; treat it
                // as the end rather than looping on it.
                _ => break,
            };
            let got = rows.len();
            out.extend(rows);
            if got < PER_PAGE {
                break;
            }
            page += 1;
        }
        out.truncate(self.index_cap);
        Ok(out)
    }

    /// The repository's issues, most recently updated first.
    ///
    /// GitHub's issues endpoint returns pull requests too — a pull *is* an issue with a
    /// `pull_request` field — so those are dropped here and served under `pulls/`
    /// instead, where the extra endpoints that only apply to them are available.
    pub async fn list_issues(&self, repo: &str) -> anyhow::Result<Vec<IssueRow>> {
        let rows = self.list_paged(repo, "/issues").await?;
        Ok(rows
            .iter()
            .filter(|r| r.get("pull_request").is_none())
            .filter_map(IssueRow::parse)
            .collect())
    }

    /// The repository's pull requests, most recently updated first.
    pub async fn list_pulls(&self, repo: &str) -> anyhow::Result<Vec<IssueRow>> {
        let rows = self.list_paged(repo, "/pulls").await?;
        Ok(rows.iter().filter_map(IssueRow::parse).collect())
    }

    pub async fn issue(&self, repo: &str, number: u64) -> anyhow::Result<Value> {
        self.get_json(self.repo_url(repo, &format!("/issues/{number}"))?)
            .await
    }

    pub async fn pull(&self, repo: &str, number: u64) -> anyhow::Result<Value> {
        self.get_json(self.repo_url(repo, &format!("/pulls/{number}"))?)
            .await
    }

    /// Conversation comments. The same endpoint serves an issue and a pull, since a pull
    /// carries its discussion as issue comments.
    pub async fn comments(&self, repo: &str, number: u64) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get_json(self.repo_url(
                repo,
                &format!("/issues/{number}/comments?per_page={PER_PAGE}"),
            )?)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// Line-anchored review comments, which exist only on a pull.
    pub async fn review_comments(&self, repo: &str, number: u64) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get_json(self.repo_url(
                repo,
                &format!("/pulls/{number}/comments?per_page={PER_PAGE}"),
            )?)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// Files a pull touches, with their patches.
    pub async fn pull_files(&self, repo: &str, number: u64) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .get_json(self.repo_url(repo, &format!("/pulls/{number}/files?per_page={PER_PAGE}"))?)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
}

/// A listing row for one issue or pull: what a filename needs, and the timestamp that
/// dates it.
#[derive(Clone, Debug)]
pub struct IssueRow {
    pub number: u64,
    pub title: String,
    /// `updated_at`, RFC 3339. Real, unlike a git tree's absent timestamps, so these
    /// entries carry an honest mtime.
    pub updated_at: Option<String>,
}

impl IssueRow {
    fn parse(row: &Value) -> Option<Self> {
        Some(Self {
            number: row.get("number").and_then(|n| n.as_u64())?,
            title: row
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string(),
            updated_at: row
                .get("updated_at")
                .and_then(|u| u.as_str())
                .map(str::to_string),
        })
    }
}

/// One tree row, or `None` if it is malformed. A row whose mode/type this code doesn't
/// recognize is dropped rather than guessed at.
fn parse_entry(row: &Value) -> Option<TreeEntry> {
    let path = row.get("path").and_then(|p| p.as_str())?.to_string();
    let sha = row.get("sha").and_then(|s| s.as_str()).unwrap_or_default();
    let kind = match row.get("type").and_then(|t| t.as_str())? {
        "blob" => EntryKind::Blob,
        "tree" => EntryKind::Tree,
        "commit" => EntryKind::Submodule,
        _ => return None,
    };
    // A submodule's sha names a commit in another repository, so it is not a blob or
    // tree id here; every other kind must carry a usable object id.
    if kind != EntryKind::Submodule && !valid_sha(sha) {
        return None;
    }
    Some(TreeEntry {
        path,
        kind,
        sha: sha.to_string(),
        size: row.get("size").and_then(|s| s.as_u64()),
    })
}

#[cfg(test)]
mod tests;
