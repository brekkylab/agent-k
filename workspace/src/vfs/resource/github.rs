//! GitHub: repositories under one or more owners, each in three views.
//!
//! ```text
//! <owner>/<repo>/code/<path>                 the git tree at a pinned commit
//! <owner>/<repo>/issues/<n>__<title>.json    an issue with its conversation
//! <owner>/<repo>/pulls/<n>__<title>.json     a pull request with its conversation and diff
//! ```
//!
//! **Why `<owner>` is a path segment.** A fine-grained token names exactly one *resource
//! owner* when it is created, so one credential cannot reach two owners' private
//! repositories. A mount spanning owners therefore carries one token per owner
//! ([`GithubSource`](crate::vfs::accessor::GithubSource)), and each owner becomes a
//! directory. The alternative — a single credential wide enough for all of them — is a
//! classic token with write access to everything the account can see, which is a great deal
//! of authority to hold open for a read-only mount.
//!
//! **Why the three views are split.** They behave nothing alike, and one flat namespace
//! would have to serve all of them on the weakest of their guarantees.
//!
//! `code/` is content-addressed, and that makes it the cheap view. One recursive tree call
//! names every path in a repository along with each blob's SHA and exact byte length, so
//! `ls`, `find` and `stat` are answered from memory afterwards, a listing is complete by
//! construction, and a blob SHA is a perfect cache validator — the bytes behind one can
//! never change. What it cannot do is serve part of a file: the blobs endpoint has no
//! range, so every file is declared [`serves_whole`](FileStat::serves_whole) and fetched
//! once in full rather than re-downloaded per window.
//!
//! `issues/` and `pulls/` are the opposite on every count. They are unbounded, so a listing
//! keeps the most recently updated
//! [`index_cap`](crate::vfs::accessor::GithubConfig::index_cap) rows — as does the
//! repository listing itself — and the mount reports
//! [`listings_complete`](Resource::listings_complete) as `false`. That is what lets a `stat`
//! of something outside a cap still find it: an old issue by number, a repository by name.
//! Their contents are assembled from several endpoints on read, so a listing cannot know a
//! length and reports [`UNKNOWN_LENGTH_SIZE`]; they carry a real `updated_at` to validate
//! against.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::vfs::{
    accessor::{EntryKind, GithubAccessor, GithubConfig, IssueRow, RepoRow, TreeEntry},
    error::{ResourceError, ResourceResult},
    path::MountPath,
    resource::{DirEntry, FileKind, FileStat, Resource},
};

const CODE_DIR: &str = "code";
const ISSUES_DIR: &str = "issues";
const PULLS_DIR: &str = "pulls";

/// How long a resolved branch stays pinned.
///
/// A branch moves, so a snapshot has to be rebuilt eventually; within one window every
/// listing and read of a repository comes from a single commit, so a traversal cannot see
/// half of one tree and half of the next. Staleness past it is harmless rather than wrong:
/// the blob SHAs an expired listing handed out still name the same immutable bytes. A
/// source pinned to a commit id skips this entirely.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

/// How long an owner's repository listing is reused.
///
/// A separate concern from [`SNAPSHOT_TTL`] despite the matching value: that one bounds how
/// stale a *branch* may be, this one how long a newly created or renamed repository stays
/// invisible. Retuning one must not move the other.
const REPO_LIST_TTL: Duration = Duration::from_secs(300);

/// What an unread issue or pull reports for its length.
///
/// A placeholder, not a measurement: the JSON is assembled from two to four endpoints, so
/// knowing the real length means building it, and `ls -l` in a directory of them would
/// build every one. A 0 would be worse than a guess — it reads as "empty", and search tools
/// skip empty files — so this is a generous upper bound on a rendered conversation, marked
/// [`FileStat::size_is_estimate`] and made exact as soon as anything reads the file. See
/// [`GithubResource::resolve_size_on_stat`].
const UNKNOWN_LENGTH_SIZE: u64 = 64 * 1024;

const GITHUB_PROMPT: &str = "\
GitHub (read-only). A directory per owner, then per repository, then three views:
  <owner>/<repo>/code/<path>                   — the repository tree at a pinned commit;
                                                 real files, so ls/find/grep/cat all work
  <owner>/<repo>/issues/<number>__<title>.json — issue metadata, body, comments
  <owner>/<repo>/pulls/<number>__<title>.json  — pull metadata, body, comments, review
                                                 comments, changed files with patches
The <title> in an issue/pull filename is decoration: `cat issues/1234.json` works too, and
an issue older than the listing cap is still readable by its number. Archived repositories
are left out of a listing but still open by name.";

/// One entry of a directory in a repository's `code/`.
#[derive(Clone, Debug)]
struct Child {
    name: String,
    kind: EntryKind,
    sha: String,
    /// Exact, for a blob — a git tree measures what it points at.
    size: Option<u64>,
}

/// How a repository's tree is held.
enum Listing {
    /// The whole tree from one recursive call, keyed by directory path (`""` = the root of
    /// `code/`). Every directory has a key, so a lookup that misses is a real miss.
    Whole(HashMap<String, Vec<Child>>),
    /// The tree was larger than one recursive call may return (GitHub caps it at 100k
    /// entries / 7 MB), so directories are listed as they are visited and remembered. A
    /// shallow listing has no such cap, so each one is still complete on its own.
    PerDir {
        root_tree: String,
        dirs: Mutex<HashMap<String, Vec<Child>>>,
    },
}

/// One repository at one commit.
struct Snapshot {
    commit: String,
    listing: Listing,
}

/// One owner, one credential, and what it is scoped to.
struct OwnerSource {
    /// As configured — the casing a listing shows. Lookups are case-insensitive, because
    /// GitHub owner names are.
    owner: String,
    accessor: GithubAccessor,
    /// Serve only this repository under the owner, from config.
    only_repo: Option<String>,
    /// The source pinned a commit id, so its snapshot can never go stale.
    immutable_ref: bool,
}

/// Something fetched, kept alongside when it was fetched so a TTL can be applied to it.
type Cached<T> = (Arc<T>, Instant);

/// A keyed cache of such values, guarded across the `await` that fills it — which is what
/// collapses concurrent first requests for one key into a single fetch.
type Cache<T> = tokio::sync::Mutex<HashMap<String, Cached<T>>>;

pub struct GithubResource {
    /// In config order, which is the order the root lists them.
    sources: Vec<Arc<OwnerSource>>,
    /// Tree snapshots keyed `owner/repo`, lowercased.
    snapshots: Cache<Snapshot>,
    /// Repository listings keyed by lowercased owner.
    repo_lists: Cache<Vec<RepoRow>>,
}

impl GithubResource {
    pub fn new(config: &GithubConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !config.sources.is_empty(),
            "github mount has no sources: at least one owner + token is required"
        );
        let mut sources: Vec<Arc<OwnerSource>> = Vec::with_capacity(config.sources.len());
        for source in &config.sources {
            // Two credentials for one owner would leave the second unreachable, since the
            // owner directory can only route to one of them.
            anyhow::ensure!(
                !sources
                    .iter()
                    .any(|s| s.owner.eq_ignore_ascii_case(&source.owner)),
                "github mount lists owner {:?} twice",
                source.owner
            );
            let immutable_ref = source.git_ref.as_deref().map(is_commit_id).unwrap_or(false);
            sources.push(Arc::new(OwnerSource {
                owner: source.owner.clone(),
                accessor: GithubAccessor::new(
                    source,
                    config.index_cap,
                    config.api_base.as_deref(),
                )?,
                only_repo: source.repo.clone(),
                immutable_ref,
            }));
        }
        Ok(Self {
            sources,
            snapshots: tokio::sync::Mutex::new(HashMap::new()),
            repo_lists: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// The source serving `owner`, matched the way GitHub treats owner names.
    fn source(&self, owner: &str) -> ResourceResult<Arc<OwnerSource>> {
        self.sources
            .iter()
            .find(|s| s.owner.eq_ignore_ascii_case(owner))
            .cloned()
            .ok_or(ResourceError::NotFound)
    }

    /// An owner's repositories, cached for [`REPO_LIST_TTL`]. A source restricted to one
    /// repository answers without a request at all.
    async fn repo_list(&self, src: &OwnerSource) -> ResourceResult<Arc<Vec<RepoRow>>> {
        if let Some(only) = &src.only_repo {
            return Ok(Arc::new(vec![RepoRow {
                name: only.clone(),
                pushed_at: None,
                archived: false,
            }]));
        }
        let key = src.owner.to_ascii_lowercase();
        let mut slot = self.repo_lists.lock().await;
        if let Some((rows, fetched)) = slot.get(&key)
            && fetched.elapsed() < REPO_LIST_TTL
        {
            return Ok(rows.clone());
        }
        let rows = Arc::new(src.accessor.list_repos().await?);
        slot.insert(key, (rows.clone(), Instant::now()));
        Ok(rows)
    }

    /// Whether `repo` exists under this source, probing when a capped listing doesn't
    /// mention it — an archived repository, or one past the cap.
    async fn repo_exists(&self, src: &OwnerSource, repo: &str) -> ResourceResult<bool> {
        if let Some(only) = &src.only_repo {
            return Ok(only.eq_ignore_ascii_case(repo));
        }
        let rows = self.repo_list(src).await?;
        if rows.iter().any(|r| r.name.eq_ignore_ascii_case(repo)) {
            return Ok(true);
        }
        Ok(src.accessor.repo_meta(repo).await.is_ok())
    }

    /// The current snapshot of one repository, building or rebuilding it if needed.
    async fn snapshot(&self, src: &OwnerSource, repo: &str) -> ResourceResult<Arc<Snapshot>> {
        let key = format!(
            "{}/{}",
            src.owner.to_ascii_lowercase(),
            repo.to_ascii_lowercase()
        );
        let mut slot = self.snapshots.lock().await;
        if let Some((snap, built)) = slot.get(&key)
            && (src.immutable_ref || built.elapsed() < SNAPSHOT_TTL)
        {
            return Ok(snap.clone());
        }
        let git_ref = match src.accessor.configured_ref() {
            Some(r) => r.to_string(),
            None => src.accessor.default_branch(repo).await?,
        };
        let commit = src.accessor.resolve_commit(repo, &git_ref).await?;
        let tree = src.accessor.tree(repo, &commit, true).await?;
        let listing = if tree.truncated {
            Listing::PerDir {
                root_tree: tree.sha,
                dirs: Mutex::new(HashMap::new()),
            }
        } else {
            Listing::Whole(index_tree(&tree.entries))
        };
        let snap = Arc::new(Snapshot { commit, listing });
        slot.insert(key, (snap.clone(), Instant::now()));
        Ok(snap)
    }

    /// Children of a directory in a repository's `code/` (`""` = its root).
    ///
    /// Recursive in [`Listing::PerDir`]: a directory's tree id lives in its parent's
    /// listing, so reaching a deep path walks down from the root — once, because each level
    /// is remembered on the way.
    fn code_children<'a>(
        &'a self,
        src: &'a OwnerSource,
        repo: &'a str,
        snap: &'a Snapshot,
        dir: &'a str,
    ) -> Pin<Box<dyn Future<Output = ResourceResult<Vec<Child>>> + Send + 'a>> {
        Box::pin(async move {
            match &snap.listing {
                Listing::Whole(dirs) => dirs.get(dir).cloned().ok_or(ResourceError::NotFound),
                Listing::PerDir { root_tree, dirs } => {
                    if let Some(hit) = dirs.lock().unwrap().get(dir).cloned() {
                        return Ok(hit);
                    }
                    let tree_sha = if dir.is_empty() {
                        root_tree.clone()
                    } else {
                        let (parent, name) = split_path(dir);
                        let siblings = self.code_children(src, repo, snap, parent).await?;
                        let entry = siblings
                            .iter()
                            .find(|c| c.name == name)
                            .ok_or(ResourceError::NotFound)?;
                        match entry.kind {
                            EntryKind::Tree => entry.sha.clone(),
                            // Another repository's commit: an empty directory here, the
                            // same as a checkout that didn't recurse into it.
                            EntryKind::Submodule => return Ok(Vec::new()),
                            EntryKind::Blob => return Err(ResourceError::NotFound),
                        }
                    };
                    let tree = src.accessor.tree(repo, &tree_sha, false).await?;
                    // Serving a truncated listing would make `listings_complete` a lie in
                    // the other direction, and the cache would answer a file that exists
                    // with NotFound.
                    if tree.truncated {
                        return Err(ResourceError::Backend(anyhow::anyhow!(
                            "github: the listing of {dir:?} was truncated by the API, so it \
                             cannot be served as complete"
                        )));
                    }
                    let children = index_children(&tree.entries);
                    dirs.lock()
                        .unwrap()
                        .insert(dir.to_string(), children.clone());
                    Ok(children)
                }
            }
        })
    }

    /// The entry at a non-empty `code/` path, found through its parent's listing.
    async fn code_entry(
        &self,
        src: &OwnerSource,
        repo: &str,
        snap: &Snapshot,
        path: &str,
    ) -> ResourceResult<Child> {
        let (parent, name) = split_path(path);
        let children = self.code_children(src, repo, snap, parent).await?;
        children
            .into_iter()
            .find(|c| c.name == name)
            .ok_or(ResourceError::NotFound)
    }

    /// An issue's metadata, body and comments.
    async fn render_issue(
        &self,
        src: &OwnerSource,
        repo: &str,
        number: u64,
    ) -> ResourceResult<Vec<u8>> {
        let issue = src.accessor.issue(repo, number).await?;
        let comments = src.accessor.comments(repo, number).await?;
        let mut out = normalize_issue(&issue);
        out["comments"] = Value::Array(comments.iter().map(normalize_comment).collect());
        Ok(serde_json::to_vec_pretty(&out)?)
    }

    /// A pull's metadata, body, both kinds of comment, and its changed files.
    async fn render_pull(
        &self,
        src: &OwnerSource,
        repo: &str,
        number: u64,
    ) -> ResourceResult<Vec<u8>> {
        let pull = src.accessor.pull(repo, number).await?;
        let comments = src.accessor.comments(repo, number).await?;
        let review = src.accessor.review_comments(repo, number).await?;
        let files = src.accessor.pull_files(repo, number).await?;
        let mut out = normalize_issue(&pull);
        out["draft"] = json!(pull.get("draft").and_then(|d| d.as_bool()).unwrap_or(false));
        out["merged"] = json!(
            pull.get("merged")
                .and_then(|m| m.as_bool())
                .unwrap_or(false)
        );
        out["head_ref"] = json!(branch_label(&pull, "head"));
        out["base_ref"] = json!(branch_label(&pull, "base"));
        out["additions"] = json!(pull.get("additions").and_then(|a| a.as_u64()));
        out["deletions"] = json!(pull.get("deletions").and_then(|d| d.as_u64()));
        out["comments"] = Value::Array(comments.iter().map(normalize_comment).collect());
        out["review_comments"] =
            Value::Array(review.iter().map(normalize_review_comment).collect());
        out["files"] = Value::Array(files.iter().map(normalize_file).collect());
        Ok(serde_json::to_vec_pretty(&out)?)
    }

    /// Listing rows for one repository's `issues/` or `pulls/`.
    async fn conversation_entries(
        &self,
        src: &OwnerSource,
        repo: &str,
        pulls: bool,
    ) -> ResourceResult<Vec<DirEntry>> {
        let rows = if pulls {
            src.accessor.list_pulls(repo).await?
        } else {
            src.accessor.list_issues(repo).await?
        };
        Ok(rows.iter().map(conversation_dir_entry).collect())
    }

    /// `stat` of one issue/pull file, which probes the API because the name may be for a row
    /// outside the listing cap.
    async fn conversation_stat(
        &self,
        src: &OwnerSource,
        repo: &str,
        name: &str,
        pulls: bool,
    ) -> ResourceResult<FileStat> {
        let number = entry_number(name).ok_or(ResourceError::NotFound)?;
        let row = if pulls {
            src.accessor.pull(repo, number).await
        } else {
            src.accessor.issue(repo, number).await
        }
        .map_err(|_| ResourceError::NotFound)?;
        Ok(FileStat {
            kind: FileKind::File,
            size: UNKNOWN_LENGTH_SIZE,
            mtime: json_time(&row, "updated_at"),
            ctime: json_time(&row, "created_at"),
            size_is_estimate: true,
            serves_whole: true,
            ..Default::default()
        })
    }
}

#[async_trait]
impl Resource for GithubResource {
    async fn read_bytes(
        &self,
        path: &MountPath,
        range: Option<std::ops::Range<u64>>,
    ) -> ResourceResult<Vec<u8>> {
        let segs = segments(path);
        let data = match segs.as_slice() {
            [owner, repo, view, rest @ ..] if view == CODE_DIR && !rest.is_empty() => {
                let src = self.source(owner)?;
                let snap = self.snapshot(&src, repo).await?;
                let entry = self.code_entry(&src, repo, &snap, &rest.join("/")).await?;
                if entry.kind != EntryKind::Blob {
                    return Err(ResourceError::NotFound);
                }
                src.accessor.blob(repo, &entry.sha, entry.size).await?
            }
            [owner, repo, view, name] if view == ISSUES_DIR || view == PULLS_DIR => {
                let src = self.source(owner)?;
                let number = entry_number(name).ok_or(ResourceError::NotFound)?;
                if view == PULLS_DIR {
                    self.render_pull(&src, repo, number).await?
                } else {
                    self.render_issue(&src, repo, number).await?
                }
            }
            _ => return Err(ResourceError::NotFound),
        };
        // The blobs endpoint has no range and a rendered conversation is built whole, so a
        // window is cut here. Every file reports `serves_whole`, so the wrapper asks for the
        // whole object and this only runs for an unwrapped caller.
        Ok(slice(data, range))
    }

    async fn write_bytes(&self, _path: &MountPath, _data: Vec<u8>) -> ResourceResult<()> {
        Err(ResourceError::Unsupported)
    }

    async fn readdir(&self, path: &MountPath) -> ResourceResult<Vec<DirEntry>> {
        let segs = segments(path);
        match segs.as_slice() {
            // The owners, straight from config — no request.
            [] => Ok(self.sources.iter().map(|s| dir(&s.owner)).collect()),
            [owner] => {
                let src = self.source(owner)?;
                let rows = self.repo_list(&src).await?;
                Ok(rows
                    .iter()
                    .filter(|r| !r.archived)
                    .map(repo_dir_entry)
                    .collect())
            }
            [owner, repo] => {
                let src = self.source(owner)?;
                // Verified rather than assumed: listing three views for a repository that
                // does not exist would defer the failure to a descent that looks unrelated.
                if !self.repo_exists(&src, repo).await? {
                    return Err(ResourceError::NotFound);
                }
                Ok(vec![dir(CODE_DIR), dir(ISSUES_DIR), dir(PULLS_DIR)])
            }
            [owner, repo, view] if view == ISSUES_DIR || view == PULLS_DIR => {
                let src = self.source(owner)?;
                self.conversation_entries(&src, repo, view == PULLS_DIR)
                    .await
            }
            [owner, repo, view, rest @ ..] if view == CODE_DIR => {
                let src = self.source(owner)?;
                let snap = self.snapshot(&src, repo).await?;
                let children = self
                    .code_children(&src, repo, &snap, &rest.join("/"))
                    .await?;
                Ok(children.iter().map(code_dir_entry).collect())
            }
            _ => Err(ResourceError::NotFound),
        }
    }

    async fn stat(&self, path: &MountPath) -> ResourceResult<FileStat> {
        let segs = segments(path);
        match segs.as_slice() {
            [] => Ok(dir_stat()),
            [owner] => {
                self.source(owner)?;
                Ok(dir_stat())
            }
            [owner, repo] => {
                let src = self.source(owner)?;
                if !self.repo_exists(&src, repo).await? {
                    return Err(ResourceError::NotFound);
                }
                Ok(dir_stat())
            }
            [owner, repo, view] if view == CODE_DIR || view == ISSUES_DIR || view == PULLS_DIR => {
                let src = self.source(owner)?;
                if !self.repo_exists(&src, repo).await? {
                    return Err(ResourceError::NotFound);
                }
                Ok(dir_stat())
            }
            [owner, repo, view, name] if view == ISSUES_DIR || view == PULLS_DIR => {
                let src = self.source(owner)?;
                self.conversation_stat(&src, repo, name, view == PULLS_DIR)
                    .await
            }
            [owner, repo, view, rest @ ..] if view == CODE_DIR => {
                let src = self.source(owner)?;
                let snap = self.snapshot(&src, repo).await?;
                let entry = self.code_entry(&src, repo, &snap, &rest.join("/")).await?;
                Ok(code_stat(&entry))
            }
            _ => Err(ResourceError::NotFound),
        }
    }

    fn prompt(&self) -> &str {
        GITHUB_PROMPT
    }

    /// `false`, because three of this mount's listings are capped: `issues/`, `pulls/`, and
    /// an owner's repositories. A name missing from one of those may still exist, so a
    /// `stat` has to be allowed to probe for it — an old issue by number, a repository past
    /// the cap or archived.
    ///
    /// A `code/` listing would qualify for `true`, being complete by construction, but the
    /// flag is per provider rather than per path. It costs that view nothing: the whole tree
    /// is already in memory, so the extra `stat` the cache now asks for is a map lookup
    /// rather than a request.
    fn listings_complete(&self) -> bool {
        false
    }

    /// `false`: nothing here has a length worth resolving on `stat`.
    ///
    /// A blob's length is exact in the tree listing already. An issue's is not knowable
    /// without assembling it from several endpoints, which is the one thing this must not do
    /// per entry of an `ls -l`. Leaving it at the default would also make the cache fetch
    /// every genuinely empty file in a repository to confirm it is empty.
    fn resolve_size_on_stat(&self) -> bool {
        false
    }
}

/// Whether a configured ref is a commit id rather than a branch name.
fn is_commit_id(git_ref: &str) -> bool {
    matches!(git_ref.len(), 40 | 64) && git_ref.chars().all(|c| c.is_ascii_hexdigit())
}

/// Index a recursive tree listing by directory.
///
/// Every directory gets a key, including one whose own row is seen after its children's, so
/// a later lookup that misses is a real miss rather than an artifact of ordering. Submodules
/// get an (empty) key for the same reason.
fn index_tree(entries: &[TreeEntry]) -> HashMap<String, Vec<Child>> {
    let mut dirs: HashMap<String, Vec<Child>> = HashMap::new();
    dirs.entry(String::new()).or_default();
    for e in entries {
        if matches!(e.kind, EntryKind::Tree | EntryKind::Submodule) {
            dirs.entry(e.path.clone()).or_default();
        }
        let (parent, name) = split_path(&e.path);
        dirs.entry(parent.to_string()).or_default().push(Child {
            name: name.to_string(),
            kind: e.kind,
            sha: e.sha.clone(),
            size: e.size,
        });
    }
    for children in dirs.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name));
    }
    dirs
}

/// Children of one shallow tree listing, whose paths are bare names.
fn index_children(entries: &[TreeEntry]) -> Vec<Child> {
    let mut children: Vec<Child> = entries
        .iter()
        .map(|e| Child {
            name: split_path(&e.path).1.to_string(),
            kind: e.kind,
            sha: e.sha.clone(),
            size: e.size,
        })
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    children
}

/// `(parent, name)` of a slash-separated path; a bare name has an empty parent.
fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", path),
    }
}

fn segments(path: &MountPath) -> Vec<String> {
    path.as_str()
        .trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The issue/pull number a filename encodes. The title after `__` is decoration, so
/// `1234.json` and `1234__anything.json` both name issue 1234.
fn entry_number(name: &str) -> Option<u64> {
    name.strip_suffix(".json")?.split("__").next()?.parse().ok()
}

/// `<number>__<title>.json`, or `<number>.json` when the title sanitizes to nothing.
fn conversation_filename(row: &IssueRow) -> String {
    let slug = sanitize_title(&row.title);
    if slug.is_empty() {
        format!("{}.json", row.number)
    } else {
        format!("{}__{}.json", row.number, slug)
    }
}

/// A title reduced to one safe path segment: word characters, `-` and `.` survive,
/// everything else folds into a single `_`, capped so the whole filename stays well inside
/// every filesystem's limit.
fn sanitize_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len().min(60));
    let mut pending_sep = false;
    for c in title.chars() {
        if c.is_alphanumeric() || c == '-' || c == '.' {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(c);
        } else {
            pending_sep = true;
        }
        if out.chars().count() >= 60 {
            break;
        }
    }
    out.trim_matches('_').chars().take(60).collect()
}

fn slice(data: Vec<u8>, range: Option<std::ops::Range<u64>>) -> Vec<u8> {
    match range {
        Some(r) => {
            let start = (r.start as usize).min(data.len());
            let end = (r.end as usize).min(data.len()).max(start);
            data[start..end].to_vec()
        }
        None => data,
    }
}

/// A directory entry with nothing to report but its name. `code/` and its subdirectories
/// carry no timestamps because a git tree has none — a commit date would be a *wrong* answer
/// for a file untouched for years, and a missing fact beats an invented one.
fn dir(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        kind: FileKind::Dir,
        size: 0,
        mtime: None,
        atime: None,
        ctime: None,
        created: None,
        etag: None,
        content_type: None,
        size_is_estimate: false,
        serves_whole: false,
    }
}

/// A repository directory, dated by its last push — the closest thing a repository has to an
/// mtime, and unlike anything inside `code/`, a real one.
fn repo_dir_entry(row: &RepoRow) -> DirEntry {
    DirEntry {
        mtime: row.pushed_at.as_deref().and_then(rfc3339_to_systemtime),
        ..dir(&row.name)
    }
}

fn dir_stat() -> FileStat {
    FileStat {
        kind: FileKind::Dir,
        ..Default::default()
    }
}

/// The listing row for one entry in `code/`.
///
/// A blob carries its SHA as the etag, which is the strongest validator any provider here
/// has: it is a hash of the content, so cached bytes are valid exactly while it matches and
/// no TTL is needed. Its size is exact, straight from the tree.
fn code_dir_entry(c: &Child) -> DirEntry {
    let is_dir = !matches!(c.kind, EntryKind::Blob);
    DirEntry {
        name: c.name.clone(),
        kind: if is_dir {
            FileKind::Dir
        } else {
            FileKind::File
        },
        size: if is_dir { 0 } else { c.size.unwrap_or(0) },
        mtime: None,
        atime: None,
        ctime: None,
        created: None,
        etag: (!is_dir).then(|| c.sha.clone()),
        content_type: None,
        size_is_estimate: false,
        // The blobs endpoint has no range, so a window costs a whole download. Saying so
        // makes the wrapper fetch once and keep it, instead of refetching per chunk.
        serves_whole: !is_dir,
    }
}

fn code_stat(c: &Child) -> FileStat {
    let is_dir = !matches!(c.kind, EntryKind::Blob);
    FileStat {
        kind: if is_dir {
            FileKind::Dir
        } else {
            FileKind::File
        },
        size: if is_dir { 0 } else { c.size.unwrap_or(0) },
        etag: (!is_dir).then(|| c.sha.clone()),
        serves_whole: !is_dir,
        ..Default::default()
    }
}

/// The listing row for one issue or pull.
fn conversation_dir_entry(row: &IssueRow) -> DirEntry {
    let mtime = row.updated_at.as_deref().and_then(rfc3339_to_systemtime);
    DirEntry {
        name: conversation_filename(row),
        kind: FileKind::File,
        size: UNKNOWN_LENGTH_SIZE,
        mtime,
        atime: None,
        ctime: None,
        created: None,
        etag: None,
        content_type: None,
        size_is_estimate: true,
        serves_whole: true,
    }
}

/// Fields an issue and a pull share.
fn normalize_issue(v: &Value) -> Value {
    json!({
        "number": v.get("number").and_then(|n| n.as_u64()),
        "title": str_of(v, "title"),
        "state": str_of(v, "state"),
        "url": str_of(v, "html_url"),
        "user": login(v.get("user")),
        "labels": label_names(v),
        "assignees": v.get("assignees").and_then(|a| a.as_array()).map(|a| {
            a.iter().filter_map(|x| login(Some(x))).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "created_at": str_of(v, "created_at"),
        "updated_at": str_of(v, "updated_at"),
        "closed_at": v.get("closed_at").and_then(|c| c.as_str()),
        "body": str_of(v, "body"),
    })
}

fn normalize_comment(v: &Value) -> Value {
    json!({
        "user": login(v.get("user")),
        "created_at": str_of(v, "created_at"),
        "body": str_of(v, "body"),
    })
}

/// A review comment, which unlike a conversation comment is anchored to a line.
fn normalize_review_comment(v: &Value) -> Value {
    json!({
        "user": login(v.get("user")),
        "created_at": str_of(v, "created_at"),
        "path": str_of(v, "path"),
        "line": v.get("line").and_then(|l| l.as_u64()),
        "diff_hunk": str_of(v, "diff_hunk"),
        "body": str_of(v, "body"),
    })
}

/// One changed file of a pull, patch included — the diff is the point of reading a pull, and
/// `code/` already serves whole files for anyone who wants the surrounding context.
fn normalize_file(v: &Value) -> Value {
    json!({
        "path": str_of(v, "filename"),
        "status": str_of(v, "status"),
        "additions": v.get("additions").and_then(|a| a.as_u64()),
        "deletions": v.get("deletions").and_then(|d| d.as_u64()),
        "patch": v.get("patch").and_then(|p| p.as_str()),
    })
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn login(user: Option<&Value>) -> Option<String> {
    user?
        .get("login")
        .and_then(|l| l.as_str())
        .map(str::to_string)
}

fn label_names(v: &Value) -> Vec<String> {
    v.get("labels")
        .and_then(|l| l.as_array())
        .map(|labels| {
            labels
                .iter()
                .filter_map(|l| {
                    // A label is an object, or just its name on some payloads.
                    l.get("name")
                        .and_then(|n| n.as_str())
                        .or_else(|| l.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `owner:branch` for a pull's head or base, so a fork is distinguishable.
fn branch_label(pull: &Value, side: &str) -> String {
    let s = pull.get(side);
    let branch = s
        .and_then(|x| x.get("ref"))
        .and_then(|r| r.as_str())
        .unwrap_or_default();
    match s
        .and_then(|x| x.get("repo"))
        .and_then(|r| r.get("full_name"))
        .and_then(|f| f.as_str())
    {
        Some(repo) => format!("{repo}:{branch}"),
        None => branch.to_string(),
    }
}

fn json_time(v: &Value, key: &str) -> Option<std::time::SystemTime> {
    v.get(key)
        .and_then(|x| x.as_str())
        .and_then(rfc3339_to_systemtime)
}

/// Parse an RFC 3339 timestamp into a `SystemTime` (pre-epoch → `None`).
fn rfc3339_to_systemtime(s: &str) -> Option<std::time::SystemTime> {
    let secs = chrono::DateTime::parse_from_rfc3339(s).ok()?.timestamp();
    (secs >= 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests;
