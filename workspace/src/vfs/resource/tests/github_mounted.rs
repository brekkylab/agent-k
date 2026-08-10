//! GitHub behind the wrapper production installs.
//!
//! Drives `CachedResource::new(GithubResource)` — the pair `build_mounts` assembles —
//! against a mock of the git, issues and pulls endpoints on a loopback socket, counting
//! what the server was asked for. The counts are the point: this provider's two claims
//! are that a whole traversal costs *one* tree request and that a file read in windows
//! costs *one* blob request, and neither is visible from a test that only checks the
//! bytes that come back.
//!
//! No credentials and no new dependency: the mock is a `tokio::net::TcpListener` and the
//! provider is pointed at it with `api_base`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::vfs::{
    accessor::{GithubConfig, GithubSource},
    cache::CachedResource,
    error::ResourceError,
    path::MountPath,
    resource::{GithubResource, Resource},
};

const OWNER: &str = "octo";
const REPO: &str = "demo";
/// The commit the mount pins to, so a test's request count isn't padded by ref
/// resolution. 40 hex characters, as a real one is.
const COMMIT: &str = "1111111111111111111111111111111111111111";
const ROOT_TREE: &str = "2222222222222222222222222222222222222222";
const SRC_TREE: &str = "3333333333333333333333333333333333333333";
const MAIN_BLOB: &str = "4444444444444444444444444444444444444444";
const README_BLOB: &str = "5555555555555555555555555555555555555555";

/// What the mock serves. Everything is optional so a test scripts only the endpoints it
/// exercises; anything else answers 404.
#[derive(Default)]
struct Script {
    /// `git/trees/{sha}?recursive=1`.
    tree_recursive: Option<Value>,
    /// `git/trees/{sha}` without `recursive`, keyed by tree sha.
    trees: HashMap<String, Value>,
    /// `git/blobs/{sha}`, keyed by blob sha.
    blobs: HashMap<String, Vec<u8>>,
    default_branch: Option<String>,
    /// `git/ref/heads/{branch}` → this commit sha.
    branch_commit: Option<String>,
    issues_list: Option<Value>,
    pulls_list: Option<Value>,
    /// Single issues and pulls by number.
    issues: HashMap<u64, Value>,
    pulls: HashMap<u64, Value>,
    comments: Option<Value>,
    review_comments: Option<Value>,
    pull_files: Option<Value>,
    /// Answer *every* request with this status instead, for the failure paths.
    force_status: Option<u16>,
    /// `/user/repos` — what the token can see, across owners.
    user_repos: Option<Value>,
    /// Answer the first `.0` requests with a spent-quota 403 whose window resets `.1`
    /// seconds from now, then serve normally. `(1, 0)` is a window about to turn over;
    /// `(1, 3600)` is one that just opened.
    quota_block: Option<(usize, u64)>,
    /// How many requests `quota_block` has already refused.
    blocked: std::sync::atomic::AtomicUsize,
}

struct Mock {
    addr: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Mock {
    /// A mount scoped to the one demo repository, which is what most tests want: the repo
    /// comes from config, so no enumeration request pads the counts.
    fn config(&self, git_ref: Option<&str>) -> GithubConfig {
        self.config_with(vec![GithubSource {
            owner: OWNER.into(),
            token: "ghp_test".into(),
            repo: Some(REPO.into()),
            git_ref: git_ref.map(str::to_string),
        }])
    }

    fn config_with(&self, sources: Vec<GithubSource>) -> GithubConfig {
        GithubConfig {
            sources,
            index_cap: None,
            api_base: Some(self.addr.clone()),
        }
    }

    /// The mount as `build_mounts` assembles it.
    fn mounted(&self, git_ref: Option<&str>) -> CachedResource {
        CachedResource::new(Arc::new(
            GithubResource::new(&self.config(git_ref)).expect("build GithubResource"),
        ))
    }

    /// The same wrapper, over an arbitrary set of sources.
    fn mounted_with(&self, sources: Vec<GithubSource>) -> CachedResource {
        CachedResource::new(Arc::new(
            GithubResource::new(&self.config_with(sources)).expect("build GithubResource"),
        ))
    }

    fn targets(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// How many requests had `needle` in their target.
    fn count(&self, needle: &str) -> usize {
        self.targets().iter().filter(|t| t.contains(needle)).count()
    }

    fn reset(&self) {
        self.seen.lock().unwrap().clear();
    }
}

async fn start(script: Script) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (log, script) = (seen.clone(), Arc::new(script));
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (log, script) = (log.clone(), script.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let head_end = loop {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let start_line = head.lines().next().unwrap_or("").to_string();
                let target = start_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                log.lock().unwrap().push(target.clone());

                let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                let reply = |status: u16, body: Vec<u8>| {
                    let mut out = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    out
                };
                let json_reply = |v: &Value| reply(200, v.to_string().into_bytes());
                // A spent-quota refusal, carrying the two headers that tell a client the
                // window is empty and when it turns over.
                let quota_reply = |reset_in: u64| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let body = br#"{"message":"API rate limit exceeded"}"#.to_vec();
                    let mut out = format!(
                        "HTTP/1.1 403 X\r\nContent-Type: application/json\r\n\
                         x-ratelimit-remaining: 0\r\nx-ratelimit-limit: 5000\r\n\
                         x-ratelimit-reset: {}\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n",
                        now + reset_in,
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    out
                };

                // `/repos/{owner}/{repo}{rest}` for *any* owner and repo, so a mount holding
                // several sources routes. `rest` of `…/issues/12/comments` is
                // `/issues/12/comments`, whose segments are ("issues", 12, "comments").
                let rest = path
                    .strip_prefix("/repos/")
                    .and_then(|p| {
                        let mut parts = p.splitn(3, '/');
                        let _owner = parts.next()?;
                        let _repo = parts.next()?;
                        Some(match parts.next() {
                            Some(tail) => format!("/{tail}"),
                            None => String::new(),
                        })
                    })
                    .unwrap_or_default();
                let rest = rest.as_str();
                let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
                let number = segs.get(1).and_then(|n| n.parse::<u64>().ok());

                let quota_refusal = script.quota_block.and_then(|(n, reset_in)| {
                    use std::sync::atomic::Ordering;
                    (script.blocked.load(Ordering::SeqCst) < n).then(|| {
                        script.blocked.fetch_add(1, Ordering::SeqCst);
                        reset_in
                    })
                });

                let out = if let Some(reset_in) = quota_refusal {
                    quota_reply(reset_in)
                } else if let Some(st) = script.force_status {
                    reply(st, br#"{"message":"scripted"}"#.to_vec())
                } else if path == "/user/repos" {
                    // Page 2 onward is empty, so the walk stops rather than looping.
                    let empty = json!([]);
                    let body = if query.contains("page=1") {
                        script.user_repos.as_ref().unwrap_or(&empty)
                    } else {
                        &empty
                    };
                    json_reply(body)
                } else if segs.is_empty() {
                    match &script.default_branch {
                        Some(b) => json_reply(&json!({"default_branch": b})),
                        None => reply(404, br#"{"message":"no repo"}"#.to_vec()),
                    }
                } else if rest.starts_with("/git/ref/heads/") {
                    match &script.branch_commit {
                        Some(sha) => json_reply(&json!({"object": {"sha": sha, "type": "commit"}})),
                        None => reply(404, br#"{"message":"no ref"}"#.to_vec()),
                    }
                } else if rest.starts_with("/git/trees/") {
                    let sha = rest.trim_start_matches("/git/trees/");
                    let scripted = if query.contains("recursive=1") {
                        script.tree_recursive.clone()
                    } else {
                        script.trees.get(sha).cloned()
                    };
                    match scripted {
                        Some(v) => json_reply(&v),
                        None => reply(404, br#"{"message":"no tree"}"#.to_vec()),
                    }
                } else if rest.starts_with("/git/blobs/") {
                    let sha = rest.trim_start_matches("/git/blobs/");
                    match script.blobs.get(sha) {
                        Some(b) => reply(200, b.clone()),
                        None => reply(404, br#"{"message":"no blob"}"#.to_vec()),
                    }
                } else if segs == ["issues"] {
                    json_reply(script.issues_list.as_ref().unwrap_or(&json!([])))
                } else if segs == ["pulls"] {
                    json_reply(script.pulls_list.as_ref().unwrap_or(&json!([])))
                } else if segs.first() == Some(&"issues") && segs.get(2) == Some(&"comments") {
                    json_reply(script.comments.as_ref().unwrap_or(&json!([])))
                } else if segs.first() == Some(&"pulls") && segs.get(2) == Some(&"comments") {
                    json_reply(script.review_comments.as_ref().unwrap_or(&json!([])))
                } else if segs.first() == Some(&"pulls") && segs.get(2) == Some(&"files") {
                    json_reply(script.pull_files.as_ref().unwrap_or(&json!([])))
                } else if segs.first() == Some(&"issues") {
                    match number.and_then(|n| script.issues.get(&n)) {
                        Some(v) => json_reply(v),
                        None => reply(404, br#"{"message":"no issue"}"#.to_vec()),
                    }
                } else if segs.first() == Some(&"pulls") {
                    match number.and_then(|n| script.pulls.get(&n)) {
                        Some(v) => json_reply(v),
                        None => reply(404, br#"{"message":"no pull"}"#.to_vec()),
                    }
                } else {
                    reply(404, br#"{"message":"no route"}"#.to_vec())
                };
                let _ = sock.write_all(&out).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock { addr, seen }
}

/// A small repository: `README.md` and `src/main.rs`, plus a submodule.
fn demo_tree(truncated: bool) -> Value {
    json!({
        "sha": ROOT_TREE,
        "truncated": truncated,
        "tree": [
            {"path": "README.md", "type": "blob", "sha": README_BLOB, "size": 11},
            {"path": "src", "type": "tree", "sha": SRC_TREE},
            {"path": "src/main.rs", "type": "blob", "sha": MAIN_BLOB, "size": 4096},
            {"path": "vendor", "type": "commit", "sha": "abc123"},
        ]
    })
}

fn demo_blobs() -> HashMap<String, Vec<u8>> {
    HashMap::from([
        (README_BLOB.to_string(), b"hello world".to_vec()),
        (MAIN_BLOB.to_string(), vec![b'x'; 4096]),
    ])
}

async fn code_mock(truncated: bool) -> Mock {
    start(Script {
        tree_recursive: Some(demo_tree(truncated)),
        blobs: demo_blobs(),
        ..Default::default()
    })
    .await
}

/// The headline claim: one recursive tree call answers the whole traversal. Every
/// `readdir` and `stat` below the first is a map lookup, so a `find`-style walk of a
/// repository costs a single request no matter how deep it goes.
#[tokio::test]
async fn one_tree_call_serves_the_whole_traversal() {
    let mock = code_mock(false).await;
    let fs = mock.mounted(Some(COMMIT));

    let root: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect("readdir /code")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, vec!["README.md", "src", "vendor"]);

    let src: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/code/src"))
        .await
        .expect("readdir /code/src")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(src, vec!["main.rs"]);

    for p in [
        "/octo/demo/code/README.md",
        "/octo/demo/code/src/main.rs",
        "/octo/demo/code/src",
        "/octo/demo/code",
        "/octo/demo",
        "/octo",
    ] {
        fs.stat(&MountPath::new(p))
            .await
            .unwrap_or_else(|e| panic!("stat {p}: {e}"));
    }

    assert_eq!(
        mock.count("/git/trees/"),
        1,
        "one recursive tree, not one per directory: {:?}",
        mock.targets()
    );
    // Pinned to a commit id, so nothing had to resolve a ref either.
    assert_eq!(mock.count("/git/ref/"), 0);
}

/// A blob's length comes from the tree, exact, and survives the trip through the
/// wrapper's listing cache into a `stat` — the hand-off where a placeholder length once
/// got served as a measurement.
#[tokio::test]
async fn exact_sizes_survive_listing_to_stat() {
    let mock = code_mock(false).await;
    let fs = mock.mounted(Some(COMMIT));

    let listed = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .unwrap();
    let readme = listed.iter().find(|e| e.name == "README.md").unwrap();
    assert_eq!(readme.size, 11);
    assert!(!readme.size_is_estimate, "a git tree measures its blobs");
    assert_eq!(readme.etag.as_deref(), Some(README_BLOB));

    let st = fs
        .stat(&MountPath::new("/octo/demo/code/README.md"))
        .await
        .unwrap();
    assert_eq!(st.size, 11, "the listing's length, not a guess");
    assert!(!st.size_is_estimate);
    assert_eq!(st.etag.as_deref(), Some(README_BLOB), "sha as validator");
    assert!(st.serves_whole, "the blobs endpoint has no range");
}

/// The second claim: the blobs endpoint has no range, so a file read in windows must
/// fetch once and be served from cache thereafter. Without `serves_whole` each window
/// would re-download the whole blob.
#[tokio::test]
async fn a_windowed_read_fetches_the_blob_once() {
    let mock = code_mock(false).await;
    let fs = mock.mounted(Some(COMMIT));
    let path = MountPath::new("/octo/demo/code/src/main.rs");

    // Prime the tree so the count below is only about blobs.
    fs.readdir(&MountPath::new("/octo/demo/code/src"))
        .await
        .unwrap();
    mock.reset();

    let mut assembled = Vec::new();
    for start in (0..4096u64).step_by(512) {
        let chunk = fs
            .read_bytes(&path, Some(start..start + 512))
            .await
            .expect("windowed read");
        assert_eq!(chunk.len(), 512, "window at {start}");
        assembled.extend_from_slice(&chunk);
    }
    assert_eq!(assembled, vec![b'x'; 4096], "windows reassemble the file");
    assert_eq!(
        mock.count("/git/blobs/"),
        1,
        "eight windows, one download: {:?}",
        mock.targets()
    );

    // A whole-file read after them is still served from the cached bytes, because the
    // blob sha it validates against cannot change.
    let whole = fs.read_bytes(&path, None).await.unwrap();
    assert_eq!(whole.len(), 4096);
    assert_eq!(mock.count("/git/blobs/"), 1, "still cached");
}

/// A submodule is another repository's commit. It lists as an empty directory — what a
/// checkout that didn't recurse shows — rather than as a file that fails to read.
#[tokio::test]
async fn a_submodule_is_an_empty_directory() {
    let mock = code_mock(false).await;
    let fs = mock.mounted(Some(COMMIT));

    let st = fs
        .stat(&MountPath::new("/octo/demo/code/vendor"))
        .await
        .unwrap();
    assert_eq!(st.kind, crate::vfs::resource::FileKind::Dir);
    assert!(
        fs.readdir(&MountPath::new("/octo/demo/code/vendor"))
            .await
            .expect("a submodule dir lists")
            .is_empty()
    );
}

/// When the repository is too large for one recursive listing, GitHub sets `truncated`
/// and the provider must not serve the partial answer as complete. It falls back to
/// shallow per-directory listings, which have no such cap.
#[tokio::test]
async fn a_truncated_tree_falls_back_to_per_directory_listings() {
    let mock = start(Script {
        // The recursive attempt comes back cut short: only the flag and the root tree
        // sha are usable.
        tree_recursive: Some(json!({"sha": ROOT_TREE, "truncated": true, "tree": []})),
        trees: HashMap::from([
            (
                ROOT_TREE.to_string(),
                json!({"sha": ROOT_TREE, "truncated": false, "tree": [
                    {"path": "README.md", "type": "blob", "sha": README_BLOB, "size": 11},
                    {"path": "src", "type": "tree", "sha": SRC_TREE},
                ]}),
            ),
            (
                SRC_TREE.to_string(),
                json!({"sha": SRC_TREE, "truncated": false, "tree": [
                    {"path": "main.rs", "type": "blob", "sha": MAIN_BLOB, "size": 4096},
                ]}),
            ),
        ]),
        blobs: demo_blobs(),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let root: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect("root lists from the shallow tree")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(root, vec!["README.md", "src"]);

    // Reaching a nested directory walks down for its tree id, then remembers it.
    let src: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/code/src"))
        .await
        .expect("nested dir lists")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(src, vec!["main.rs"]);

    // Reads still work, and the file's exact size came through the shallow listing.
    let st = fs
        .stat(&MountPath::new("/octo/demo/code/src/main.rs"))
        .await
        .unwrap();
    assert_eq!(st.size, 4096);
    let bytes = fs
        .read_bytes(&MountPath::new("/octo/demo/code/src/main.rs"), None)
        .await
        .unwrap();
    assert_eq!(bytes.len(), 4096);

    // The walk cached each level, so revisiting costs no further tree calls.
    let before = mock.count("/git/trees/");
    fs.readdir(&MountPath::new("/octo/demo/code/src"))
        .await
        .unwrap();
    assert_eq!(mock.count("/git/trees/"), before, "levels are remembered");
}

/// A mount that named no ref resolves the default branch, then pins it — and does that
/// once, not per listing.
#[tokio::test]
async fn a_default_branch_mount_resolves_then_pins() {
    let mock = start(Script {
        default_branch: Some("main".into()),
        branch_commit: Some(COMMIT.to_string()),
        tree_recursive: Some(demo_tree(false)),
        blobs: demo_blobs(),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(None);

    fs.readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect("readdir");
    fs.readdir(&MountPath::new("/octo/demo/code/src"))
        .await
        .expect("readdir");
    fs.stat(&MountPath::new("/octo/demo/code/README.md"))
        .await
        .expect("stat");

    assert_eq!(mock.count("/git/ref/heads/main"), 1, "resolved once");
    assert_eq!(mock.count("/git/trees/"), 1, "then pinned");
    // The repository itself is read only to learn the default branch.
    let repo_hits = mock
        .targets()
        .iter()
        .filter(|t| t.ends_with(&format!("/repos/{OWNER}/{REPO}")))
        .count();
    assert_eq!(repo_hits, 1);
}

fn issue(number: u64, title: &str, body: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "state": "open",
        "html_url": format!("https://github.com/{OWNER}/{REPO}/issues/{number}"),
        "user": {"login": "alice"},
        "labels": [{"name": "bug"}],
        "assignees": [],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-02-01T00:00:00Z",
        "body": body,
    })
}

/// `issues/` lists the rows it was given, with the title in the filename, and reading one
/// assembles its conversation.
#[tokio::test]
async fn issues_list_and_read_with_their_comments() {
    let mock = start(Script {
        issues_list: Some(json!([
            issue(7, "Crash on startup", "it crashes"),
            // A pull request comes back from the issues endpoint too, and must not be
            // listed here — it belongs under `pulls/`.
            json!({"number": 8, "title": "A PR", "pull_request": {"url": "x"},
                   "updated_at": "2026-02-02T00:00:00Z"}),
        ])),
        issues: HashMap::from([(7, issue(7, "Crash on startup", "it crashes"))]),
        comments: Some(json!([
            {"user": {"login": "bob"}, "created_at": "2026-02-01T01:00:00Z", "body": "me too"},
        ])),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let names: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/issues"))
        .await
        .expect("readdir /issues")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(
        names,
        vec!["7__Crash_on_startup.json"],
        "a pull request is not an issue here"
    );

    let bytes = fs
        .read_bytes(
            &MountPath::new("/octo/demo/issues/7__Crash_on_startup.json"),
            None,
        )
        .await
        .expect("read the issue");
    let v: Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(v["number"], json!(7));
    assert_eq!(v["body"], json!("it crashes"));
    assert_eq!(v["labels"], json!(["bug"]));
    assert_eq!(v["comments"][0]["user"], json!("bob"));
    assert_eq!(v["comments"][0]["body"], json!("me too"));
}

/// The listing keeps only the newest rows, so the title in a filename is decoration and
/// an issue the listing never mentioned is still readable by number. That is what
/// `listings_complete() == false` buys, and it has to survive the wrapper, which would
/// otherwise answer an unlisted name with NotFound and never ask.
#[tokio::test]
async fn an_unlisted_issue_is_still_reachable_by_number() {
    let mock = start(Script {
        // The listing mentions 7 and nothing else.
        issues_list: Some(json!([issue(7, "Recent", "new")])),
        issues: HashMap::from([
            (7, issue(7, "Recent", "new")),
            (1, issue(1, "Ancient history", "old")),
        ]),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let listed: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/issues"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(listed, vec!["7__Recent.json"]);

    // Absent from that listing, but a stat is allowed to probe for it.
    let st = fs
        .stat(&MountPath::new("/octo/demo/issues/1.json"))
        .await
        .expect("an unlisted issue still stats");
    assert!(st.mtime.is_some(), "issues carry a real updated_at");
    assert!(st.size_is_estimate, "length unknown until it is built");

    let bytes = fs
        .read_bytes(&MountPath::new("/octo/demo/issues/1.json"), None)
        .await
        .expect("and reads");
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["body"], json!("old"));

    // An issue that really does not exist is still a miss.
    assert!(matches!(
        fs.stat(&MountPath::new("/octo/demo/issues/999.json")).await,
        Err(ResourceError::NotFound)
    ));
}

/// A pull carries what an issue does plus the things only it has: both kinds of comment,
/// its branches, and the diff.
#[tokio::test]
async fn pulls_carry_their_diff_and_review_comments() {
    let pull = json!({
        "number": 12,
        "title": "Add the thing",
        "state": "open",
        "html_url": "https://github.com/octo/demo/pull/12",
        "user": {"login": "carol"},
        "labels": [],
        "assignees": [],
        "created_at": "2026-03-01T00:00:00Z",
        "updated_at": "2026-03-02T00:00:00Z",
        "body": "please review",
        "draft": false,
        "merged": false,
        "additions": 10,
        "deletions": 2,
        "head": {"ref": "feature", "repo": {"full_name": "fork/demo"}},
        "base": {"ref": "main", "repo": {"full_name": "octo/demo"}},
    });
    let mock = start(Script {
        pulls_list: Some(json!([pull])),
        pulls: HashMap::from([(12, pull.clone())]),
        comments: Some(json!([
            {"user": {"login": "dan"}, "created_at": "2026-03-02T01:00:00Z", "body": "looks ok"},
        ])),
        review_comments: Some(json!([
            {"user": {"login": "erin"}, "created_at": "2026-03-02T02:00:00Z",
             "path": "src/main.rs", "line": 42, "diff_hunk": "@@ -1 +1 @@",
             "body": "rename this"},
        ])),
        pull_files: Some(json!([
            {"filename": "src/main.rs", "status": "modified", "additions": 10,
             "deletions": 2, "patch": "@@ -1 +1 @@\n-old\n+new"},
        ])),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let names: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/pulls"))
        .await
        .expect("readdir /pulls")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["12__Add_the_thing.json"]);

    let bytes = fs
        .read_bytes(&MountPath::new("/octo/demo/pulls/12.json"), None)
        .await
        .expect("read the pull");
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["number"], json!(12));
    assert_eq!(
        v["head_ref"],
        json!("fork/demo:feature"),
        "the fork is named"
    );
    assert_eq!(v["base_ref"], json!("octo/demo:main"));
    assert_eq!(v["additions"], json!(10));
    assert_eq!(v["comments"][0]["body"], json!("looks ok"));
    assert_eq!(v["review_comments"][0]["path"], json!("src/main.rs"));
    assert_eq!(v["review_comments"][0]["line"], json!(42));
    assert_eq!(v["files"][0]["path"], json!("src/main.rs"));
    assert!(
        v["files"][0]["patch"].as_str().unwrap().contains("+new"),
        "the diff is the point of reading a pull"
    );
}

/// The failure a fine-grained token is *expected* to reach eventually. It has to name
/// itself: nothing server-side can renew such a token, so an operator seeing this needs
/// to be told to supply a new one rather than to wait or retry.
#[tokio::test]
async fn an_expired_token_says_so() {
    let mock = start(Script {
        force_status: Some(401),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let err = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect_err("401 must not look like an empty repository");
    let msg = err.to_string();
    assert!(msg.contains("401"), "{msg}");
    assert!(msg.contains("expired") || msg.contains("revoked"), "{msg}");
    assert!(
        !matches!(err, ResourceError::NotFound),
        "a rejected token is not a missing path"
    );
    // Nothing here is retryable, so the failure is prompt rather than five backoffs deep.
    assert_eq!(mock.count("/git/trees/"), 1, "no retry ladder on a 401");
}

/// The hourly quota returns in full at one instant rather than trickling back, and that
/// instant is the end of the *current* window — which may be seconds away. So a spent
/// quota whose window is about to turn over is waited out, not surrendered to: giving up
/// on sight of `remaining: 0` would throw away a request available a second later.
#[tokio::test]
async fn a_quota_about_to_reset_is_waited_out() {
    let mock = start(Script {
        // One refusal, with a window that has already ended (a one-second margin wait).
        quota_block: Some((1, 0)),
        tree_recursive: Some(demo_tree(false)),
        blobs: demo_blobs(),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let names: Vec<String> = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect("the retry after the reset succeeds")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["README.md", "src", "vendor"]);
    assert_eq!(
        mock.count("/git/trees/"),
        2,
        "refused once, then retried once the window turned over"
    );
}

/// A quota whose window just opened is another matter: the reset is up to an hour out,
/// and nothing behind a FUSE operation can wait that long. It fails immediately — with
/// the reset time in the message, so whoever reads it knows this clears by itself — and
/// without walking the backoff ladder toward a wait that was never going to happen.
#[tokio::test]
async fn a_quota_that_just_opened_fails_with_its_reset_time() {
    let mock = start(Script {
        // Refuse everything, with the window resetting in 40 minutes.
        quota_block: Some((usize::MAX, 2400)),
        tree_recursive: Some(demo_tree(false)),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(Some(COMMIT));

    let err = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect_err("a spent quota is not an empty repository");
    let msg = err.to_string();
    assert!(msg.contains("rate limit"), "{msg}");
    assert!(msg.contains("quota spent"), "{msg}");
    assert!(
        msg.contains("resets in 24") || msg.contains("resets in 23"),
        "the message names when it clears: {msg}"
    );
    assert!(
        !matches!(err, ResourceError::NotFound),
        "a spent quota is not a missing path"
    );
    assert_eq!(
        mock.count("/git/trees/"),
        1,
        "no backoff ladder toward an hour-away reset"
    );
}

fn repo_row(owner: &str, name: &str, archived: bool) -> Value {
    json!({
        "name": name,
        "owner": {"login": owner},
        "pushed_at": "2026-03-01T00:00:00Z",
        "archived": archived,
    })
}

/// A source that names no repository enumerates the owner's, keeping only that owner's rows
/// — `/user/repos` reports everything the *account* reaches, which may span owners this
/// mount has no credential for — and leaving archived ones out of the listing.
#[tokio::test]
async fn an_owner_lists_its_repositories() {
    let mock = start(Script {
        user_repos: Some(json!([
            repo_row(OWNER, "demo", false),
            repo_row(OWNER, "infra", false),
            repo_row(OWNER, "retired", true),
            // Another owner entirely: reachable by the account, not by this source.
            repo_row("someone-else", "theirs", false),
        ])),
        tree_recursive: Some(demo_tree(false)),
        blobs: demo_blobs(),
        default_branch: Some("main".into()),
        branch_commit: Some(COMMIT.to_string()),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted_with(vec![GithubSource {
        owner: OWNER.into(),
        token: "ghp_test".into(),
        repo: None,
        git_ref: None,
    }]);

    let repos = fs
        .readdir(&MountPath::new("/octo"))
        .await
        .expect("readdir /octo");
    let names: Vec<&str> = repos.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["demo", "infra"],
        "another owner's repository is not this source's, and an archived one is noise"
    );
    assert!(
        repos[0].mtime.is_some(),
        "a repository is dated by its last push"
    );

    // Descending works, and the owner listing is reused rather than refetched.
    fs.readdir(&MountPath::new("/octo/demo"))
        .await
        .expect("views");
    fs.readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect("tree");
    assert_eq!(
        mock.count("/user/repos"),
        1,
        "the owner listing is cached: {:?}",
        mock.targets()
    );
}

/// An archived repository is left out of a listing but still opens by name — the same
/// bargain the issue cap makes, and what `listings_complete() == false` is for. A name that
/// is neither listed nor real stays a miss.
#[tokio::test]
async fn an_archived_repository_still_opens_by_name() {
    let mock = start(Script {
        user_repos: Some(json!([repo_row(OWNER, "demo", false)])),
        tree_recursive: Some(demo_tree(false)),
        blobs: demo_blobs(),
        default_branch: Some("main".into()),
        branch_commit: Some(COMMIT.to_string()),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted_with(vec![GithubSource {
        owner: OWNER.into(),
        token: "ghp_test".into(),
        repo: None,
        git_ref: None,
    }]);

    // `retired` is absent from the listing, but the repo endpoint answers for it, so the
    // probe finds it.
    let listed: Vec<String> = fs
        .readdir(&MountPath::new("/octo"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(listed, vec!["demo"]);

    let views: Vec<String> = fs
        .readdir(&MountPath::new("/octo/retired"))
        .await
        .expect("an unlisted repository still opens")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(views, vec!["code", "issues", "pulls"]);

    // A repository the API denies is still a miss. The mock answers `default_branch` for
    // any repo path, so this uses an owner with no source instead — the case that needs no
    // request at all.
    assert!(matches!(
        fs.readdir(&MountPath::new("/nobody/whatever")).await,
        Err(ResourceError::NotFound)
    ));
}

/// Two owners in one mount, each with its own credential, and each request carrying the
/// token belonging to the owner in the path. This is the whole reason `<owner>` is a path
/// segment: one fine-grained token cannot reach both.
#[tokio::test]
async fn each_owner_is_served_by_its_own_token() {
    let mock = start(Script {
        tree_recursive: Some(demo_tree(false)),
        blobs: demo_blobs(),
        ..Default::default()
    })
    .await;
    let fs = mock.mounted_with(vec![
        GithubSource {
            owner: OWNER.into(),
            token: "token-for-octo".into(),
            repo: Some(REPO.into()),
            git_ref: Some(COMMIT.into()),
        },
        GithubSource {
            owner: "brekkylab".into(),
            token: "token-for-brekkylab".into(),
            repo: Some("agent-k".into()),
            git_ref: Some(COMMIT.into()),
        },
    ]);

    let owners: Vec<String> = fs
        .readdir(&MountPath::root())
        .await
        .expect("root lists owners offline")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(owners, vec!["octo", "brekkylab"], "config order");

    // Each owner's repository resolves under its own path, and the mock sees the request
    // addressed to that owner.
    for (owner, repo) in [(OWNER, REPO), ("brekkylab", "agent-k")] {
        let dir = format!("/{owner}/{repo}/code");
        let entries = fs
            .readdir(&MountPath::new(&dir))
            .await
            .unwrap_or_else(|e| panic!("readdir {dir}: {e}"));
        assert!(!entries.is_empty(), "{dir} lists");
        assert!(
            mock.targets()
                .iter()
                .any(|t| t.contains(&format!("/repos/{owner}/{repo}/"))),
            "a request was addressed to {owner}/{repo}: {:?}",
            mock.targets()
        );
    }

    // An owner with no credential in this mount is a miss, and costs no request.
    let before = mock.targets().len();
    assert!(matches!(
        fs.readdir(&MountPath::new("/someone-else/repo")).await,
        Err(ResourceError::NotFound)
    ));
    assert_eq!(
        mock.targets().len(),
        before,
        "no request for an unknown owner"
    );
}

/// A 404 on a private repository the token cannot see is GitHub's way of not confirming
/// it exists, so the message has to offer both readings rather than assert the wrong one.
#[tokio::test]
async fn an_invisible_repository_explains_both_readings() {
    let mock = start(Script {
        // No `default_branch` scripted, so the repo lookup 404s.
        ..Default::default()
    })
    .await;
    let fs = mock.mounted(None);

    let err = fs
        .readdir(&MountPath::new("/octo/demo/code"))
        .await
        .expect_err("a repo that answers 404");
    let msg = err.to_string();
    assert!(msg.contains("404"), "{msg}");
    assert!(msg.contains("token cannot see"), "{msg}");
}
