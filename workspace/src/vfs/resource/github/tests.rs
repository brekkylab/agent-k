//! GitHub resource tests: the pure layers.
//!
//! Tree indexing, entry metadata, filename round-tripping and payload normalization, all
//! without a network. The provider driven behind the wrapper it ships with — where the
//! request counts live — is in `crate::vfs::resource::tests::github_mounted`.

use super::*;
// Not named by the module itself — it builds from `config.sources` — but every test here
// has to construct one.
use crate::vfs::accessor::GithubSource;

fn entry(path: &str, kind: EntryKind, size: Option<u64>) -> TreeEntry {
    TreeEntry {
        path: path.to_string(),
        kind,
        sha: "a".repeat(40),
        size,
    }
}

/// A recursive listing becomes a directory index, and a directory whose own row
/// arrives *after* its children still gets a key — otherwise reaching it would
/// depend on GitHub's row order.
#[test]
fn recursive_listing_indexes_every_directory() {
    let entries = vec![
        entry("src/vfs/mod.rs", EntryKind::Blob, Some(10)),
        entry("src", EntryKind::Tree, None),
        entry("src/vfs", EntryKind::Tree, None),
        entry("README.md", EntryKind::Blob, Some(3)),
        entry("vendor", EntryKind::Submodule, None),
    ];
    let dirs = index_tree(&entries);

    let root: Vec<&str> = dirs[""].iter().map(|c| c.name.as_str()).collect();
    assert_eq!(root, vec!["README.md", "src", "vendor"], "sorted by name");
    assert_eq!(dirs["src"].len(), 1);
    assert_eq!(dirs["src/vfs"][0].name, "mod.rs");
    // A submodule is a directory with nothing in it, not a missing path.
    assert!(dirs["vendor"].is_empty());
    assert!(!dirs.contains_key("nope"));
}

/// A blob's SHA becomes the etag and its tree size the exact length; a directory
/// gets neither. `serves_whole` marks the file, because the blobs endpoint cannot
/// serve a range and the wrapper has to know that before it plans a read.
#[test]
fn code_entries_carry_sha_as_etag_and_exact_size() {
    let blob = Child {
        name: "main.rs".into(),
        kind: EntryKind::Blob,
        sha: "b".repeat(40),
        size: Some(4096),
    };
    let e = code_dir_entry(&blob);
    assert_eq!(e.kind, FileKind::File);
    assert_eq!(e.size, 4096);
    assert_eq!(e.etag.as_deref(), Some("b".repeat(40).as_str()));
    assert!(!e.size_is_estimate, "a tree measures its blobs exactly");
    assert!(e.serves_whole, "no range on the blobs endpoint");

    let st = code_stat(&blob);
    assert_eq!(st.size, 4096);
    assert_eq!(st.etag, e.etag);
    assert!(st.serves_whole);

    let tree = Child {
        name: "src".into(),
        kind: EntryKind::Tree,
        sha: "c".repeat(40),
        size: None,
    };
    let d = code_dir_entry(&tree);
    assert_eq!(d.kind, FileKind::Dir);
    assert_eq!(d.size, 0);
    assert!(d.etag.is_none());
    assert!(!d.serves_whole);
}

/// A submodule lists as a directory, matching what `index_tree` does with it.
#[test]
fn submodule_lists_as_a_directory() {
    let sub = Child {
        name: "vendor".into(),
        kind: EntryKind::Submodule,
        sha: "deadbeef".into(),
        size: None,
    };
    assert_eq!(code_dir_entry(&sub).kind, FileKind::Dir);
    assert_eq!(code_stat(&sub).kind, FileKind::Dir);
}

#[test]
fn split_path_separates_parent_from_name() {
    assert_eq!(split_path("a"), ("", "a"));
    assert_eq!(split_path("a/b"), ("a", "b"));
    assert_eq!(split_path("a/b/c.rs"), ("a/b", "c.rs"));
}

/// The title in a filename is decoration, so a bare number addresses the same issue —
/// which is what makes an issue outside the listing cap reachable.
#[test]
fn filenames_round_trip_to_a_number() {
    let row = IssueRow {
        number: 1234,
        title: "Fix the thing!".into(),
        updated_at: None,
    };
    let name = conversation_filename(&row);
    assert_eq!(name, "1234__Fix_the_thing.json");
    assert_eq!(entry_number(&name), Some(1234));
    assert_eq!(entry_number("1234.json"), Some(1234));
    assert_eq!(entry_number("1234__whatever-else.json"), Some(1234));

    for bad in ["1234", "abc.json", ".json", "", "__x.json"] {
        assert_eq!(entry_number(bad), None, "should reject {bad:?}");
    }
}

/// A title that sanitizes to nothing still yields a usable filename.
#[test]
fn untitled_issues_still_get_a_filename() {
    let row = IssueRow {
        number: 7,
        title: "???".into(),
        updated_at: None,
    };
    assert_eq!(conversation_filename(&row), "7.json");
    assert_eq!(entry_number("7.json"), Some(7));
}

#[test]
fn titles_fold_to_one_safe_segment() {
    assert_eq!(sanitize_title("a/b"), "a_b");
    assert_eq!(sanitize_title("a  //  b"), "a_b");
    assert_eq!(sanitize_title("../../etc/passwd"), ".._.._etc_passwd");
    assert_eq!(sanitize_title("  spaced  "), "spaced");
    assert!(!sanitize_title(&"x".repeat(200)).is_empty());
    assert!(sanitize_title(&"x".repeat(200)).chars().count() <= 60);
    // Nothing survives, and the caller has to cope with an empty slug.
    assert_eq!(sanitize_title("!!!"), "");
}

/// An issue listing row dates itself from `updated_at` — a real timestamp, unlike
/// anything in `code/` — and reports a placeholder length it marks as an estimate.
#[test]
fn conversation_rows_carry_real_times_and_an_estimated_size() {
    let row = IssueRow {
        number: 9,
        title: "t".into(),
        updated_at: Some("2026-03-10T12:00:00Z".into()),
    };
    let e = conversation_dir_entry(&row);
    assert!(e.mtime.is_some(), "issues have honest timestamps");
    assert_eq!(e.size, UNKNOWN_LENGTH_SIZE);
    assert!(e.size_is_estimate);
    assert!(e.serves_whole, "assembled from several endpoints");

    let undated = IssueRow {
        updated_at: None,
        ..row
    };
    assert!(conversation_dir_entry(&undated).mtime.is_none());
}

#[test]
fn issue_normalization_keeps_the_fields_an_agent_reads() {
    let issue = json!({
        "number": 42,
        "title": "Broken",
        "state": "open",
        "html_url": "https://github.com/o/r/issues/42",
        "user": {"login": "alice"},
        "labels": [{"name": "bug"}, {"name": "p1"}],
        "assignees": [{"login": "bob"}],
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-02-01T00:00:00Z",
        "closed_at": null,
        "body": "it broke",
    });
    let n = normalize_issue(&issue);
    assert_eq!(n["number"], json!(42));
    assert_eq!(n["url"], json!("https://github.com/o/r/issues/42"));
    assert_eq!(n["user"], json!("alice"));
    assert_eq!(n["labels"], json!(["bug", "p1"]));
    assert_eq!(n["assignees"], json!(["bob"]));
    assert_eq!(n["body"], json!("it broke"));
    assert_eq!(n["closed_at"], Value::Null);
}

/// A pull's head/base name their repository, so a fork's branch is not confused for
/// one in the target repository.
#[test]
fn branch_labels_name_the_fork() {
    let pull = json!({
        "head": {"ref": "feature", "repo": {"full_name": "fork/r"}},
        "base": {"ref": "main", "repo": {"full_name": "o/r"}},
    });
    assert_eq!(branch_label(&pull, "head"), "fork/r:feature");
    assert_eq!(branch_label(&pull, "base"), "o/r:main");
    // A deleted fork leaves no repo object; the branch name still comes through.
    let orphan = json!({"head": {"ref": "gone"}});
    assert_eq!(branch_label(&orphan, "head"), "gone");
}

#[test]
fn ranges_are_clamped_not_panicking() {
    let data = b"0123456789".to_vec();
    assert_eq!(slice(data.clone(), None), data);
    assert_eq!(slice(data.clone(), Some(0..4)), b"0123".to_vec());
    assert_eq!(slice(data.clone(), Some(8..99)), b"89".to_vec());
    assert!(slice(data.clone(), Some(99..200)).is_empty());
    // A reversed range clamps to empty rather than panicking on the slice. Built from
    // bindings because the literal form is a compile-time lint, and the point here is
    // what happens at runtime when a caller hands one over.
    let (high, low) = (9u64, 2u64);
    assert!(slice(data, Some(high..low)).is_empty());
}

/// A source, with an api_base that would refuse a connection if anything reached for one.
fn offline_source(owner: &str, repo: Option<&str>, git_ref: Option<&str>) -> GithubSource {
    GithubSource {
        owner: owner.into(),
        token: "t".into(),
        repo: repo.map(str::to_string),
        git_ref: git_ref.map(str::to_string),
    }
}

fn offline_config(sources: Vec<GithubSource>) -> GithubConfig {
    GithubConfig {
        sources,
        index_cap: None,
        api_base: Some("http://127.0.0.1:1/unused".into()),
    }
}

/// A commit id pins forever; a branch has to be re-resolved.
#[test]
fn a_commit_ref_is_recognized_as_immutable() {
    let sha = "a".repeat(40);
    let pinned = |git_ref: Option<&str>| {
        GithubResource::new(&offline_config(vec![offline_source(
            "o",
            Some("r"),
            git_ref,
        )]))
        .unwrap()
        .sources[0]
            .immutable_ref
    };
    assert!(pinned(Some(&sha)));
    assert!(!pinned(Some("main")));
    assert!(!pinned(None));
}

/// A mount must have at least one credential, and two credentials for one owner is a
/// configuration error rather than a silent preference for the first: the owner directory
/// can only route to one of them.
#[test]
fn sources_are_required_and_one_per_owner() {
    assert!(GithubResource::new(&offline_config(vec![])).is_err());
    assert!(
        GithubResource::new(&offline_config(vec![
            offline_source("octo", None, None),
            offline_source("other", None, None),
        ]))
        .is_ok()
    );
    // Owner names are case-insensitive on GitHub, so the duplicate check is too.
    assert!(
        GithubResource::new(&offline_config(vec![
            offline_source("octo", None, None),
            offline_source("Octo", Some("just-this-one"), None),
        ]))
        .is_err()
    );
}

/// Writes are refused, and the mount says so through `Unsupported` rather than a
/// silent success.
#[tokio::test]
async fn the_mount_is_read_only() {
    let res =
        GithubResource::new(&offline_config(vec![offline_source("o", Some("r"), None)])).unwrap();
    let path = MountPath::new("/o/r/code/a.rs");
    assert!(matches!(
        res.write_bytes(&path, vec![1]).await,
        Err(ResourceError::Unsupported)
    ));
    assert!(matches!(
        res.unlink(&path).await,
        Err(ResourceError::Unsupported)
    ));
}

/// The root is the configured owners, served without any network call — so a mount whose
/// tokens are all wrong still lists, and the failure surfaces on descent. The casing is the
/// one from config, even though lookups ignore it.
#[tokio::test]
async fn the_root_lists_the_owners_offline() {
    let res = GithubResource::new(&offline_config(vec![
        offline_source("Octo", None, None),
        offline_source("brekkylab", Some("agent-k"), None),
    ]))
    .unwrap();

    let names: Vec<String> = res
        .readdir(&MountPath::root())
        .await
        .expect("root needs no API call")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(
        names,
        vec!["Octo", "brekkylab"],
        "config order, config casing"
    );

    assert_eq!(
        res.stat(&MountPath::root()).await.unwrap().kind,
        FileKind::Dir
    );
    // An owner directory resolves from config alone, whatever case it is asked for.
    for asked in ["/Octo", "/octo", "/OCTO"] {
        assert_eq!(
            res.stat(&MountPath::new(asked)).await.unwrap().kind,
            FileKind::Dir,
            "stat {asked}"
        );
    }
    // An owner this mount has no credential for is a miss, again without a request.
    assert!(matches!(
        res.stat(&MountPath::new("/someone-else")).await,
        Err(ResourceError::NotFound)
    ));
    assert!(matches!(
        res.readdir(&MountPath::new("/someone-else")).await,
        Err(ResourceError::NotFound)
    ));
}

/// A source restricted to one repository answers its owner's listing from config — no
/// enumeration request at all — and denies every other name under that owner.
#[tokio::test]
async fn a_single_repo_source_needs_no_enumeration() {
    let res = GithubResource::new(&offline_config(vec![offline_source(
        "octo",
        Some("demo"),
        None,
    )]))
    .unwrap();

    let names: Vec<String> = res
        .readdir(&MountPath::new("/octo"))
        .await
        .expect("listed from config, no request")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["demo"]);

    // The three views of the named repository resolve offline too.
    let views: Vec<String> = res
        .readdir(&MountPath::new("/octo/demo"))
        .await
        .expect("views need no request for a named repo")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(views, vec!["code", "issues", "pulls"]);

    // Any other repository under this owner is out of scope, and saying so costs nothing.
    assert!(matches!(
        res.readdir(&MountPath::new("/octo/other")).await,
        Err(ResourceError::NotFound)
    ));
}

/// Live check against a real account. Ignored by default; needs a fine-grained token with
/// `Contents: read-only` (add `Issues`/`Pull requests` read to exercise those views):
///
///   GITHUB_TOKEN=github_pat_… GITHUB_OWNER=octocat \
///     cargo test -p workspace github_live -- --ignored --nocapture
///
/// `GITHUB_REPO` is optional. Set it and the owner serves that one repository; leave it and
/// the owner's listing is enumerated — which is the other thing only a real token can
/// settle, since whether `/user/repos` reports a fine-grained token's full scope is not
/// something the docs state outright.
///
/// The mounted tests cover the wiring against a mock. What only real data can settle is
/// whether the *claims* hold: that a tree's reported length is the byte count the blobs
/// endpoint then hands over, which is what `size_is_estimate: false` promises.
#[tokio::test]
#[ignore = "requires GITHUB_* env + network"]
async fn github_live_reads_a_real_repository() {
    let Ok(token) = std::env::var("GITHUB_TOKEN") else {
        println!("set GITHUB_TOKEN / GITHUB_OWNER (GITHUB_REPO optional) for this live check");
        return;
    };
    let owner = std::env::var("GITHUB_OWNER").expect("set GITHUB_OWNER");
    let only_repo = std::env::var("GITHUB_REPO").ok();
    let res = GithubResource::new(&GithubConfig {
        sources: vec![GithubSource {
            owner: owner.clone(),
            token,
            repo: only_repo.clone(),
            // Unset = each repository's default branch.
            git_ref: std::env::var("GITHUB_REF").ok(),
        }],
        index_cap: std::env::var("GITHUB_INDEX_CAP")
            .ok()
            .and_then(|v| v.parse().ok()),
        api_base: None,
    })
    .expect("build GithubResource");

    // The root is the configured owner, offline.
    let owners: Vec<String> = res
        .readdir(&MountPath::root())
        .await
        .expect("readdir /")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(owners, vec![owner.clone()]);

    let repos = res
        .readdir(&MountPath::new(format!("/{owner}")))
        .await
        .unwrap_or_else(|e| panic!("readdir /{owner}: {e}"));
    println!("/{owner} has {} repositories:", repos.len());
    for r in repos.iter().take(20) {
        println!("  {} (pushed {:?})", r.name, r.mtime);
    }
    assert!(
        !repos.is_empty(),
        "the token reaches no repository under {owner:?} — with a fine-grained token, check \
         that /user/repos reports its scope (this is the uncertainty this test exists to \
         settle)"
    );

    // Whichever repository was named, or the most recently pushed one.
    let repo = only_repo.unwrap_or_else(|| repos[0].name.clone());
    let base = format!("/{owner}/{repo}");
    println!("--- {base} ---");

    let views: Vec<String> = res
        .readdir(&MountPath::new(&base))
        .await
        .unwrap_or_else(|e| panic!("readdir {base}: {e}"))
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(views, vec!["code", "issues", "pulls"]);

    let code_dir = format!("{base}/code");
    let code = res
        .readdir(&MountPath::new(&code_dir))
        .await
        .unwrap_or_else(|e| panic!("readdir {code_dir}: {e}"));
    println!("{code_dir} has {} entries:", code.len());
    for e in code.iter().take(20) {
        println!("  {:?} {} {} bytes", e.kind, e.name, e.size);
    }
    assert!(!code.is_empty(), "a real repository has a root tree");

    // The claim worth a network round trip: the tree's length is exact, so the bytes the
    // blobs endpoint returns must match it — for the smallest file, to keep this cheap.
    let smallest = code
        .iter()
        .filter(|e| e.kind == FileKind::File && e.size > 0)
        .min_by_key(|e| e.size);
    if let Some(f) = smallest {
        let path = MountPath::new(format!("{code_dir}/{}", f.name));
        let st = res.stat(&path).await.expect("stat a real blob");
        assert_eq!(st.size, f.size, "listing and stat agree");
        assert!(!st.size_is_estimate, "a git tree measures exactly");
        let etag_len = st.etag.as_deref().map(str::len);
        assert!(
            matches!(etag_len, Some(40) | Some(64)),
            "the etag is a git object id (SHA-1 or SHA-256), got {etag_len:?}"
        );
        let bytes = res.read_bytes(&path, None).await.expect("read a real blob");
        println!(
            "read {} — {} bytes, tree said {}",
            f.name,
            bytes.len(),
            f.size
        );
        assert_eq!(
            bytes.len() as u64,
            f.size,
            "the tree's length is the real byte count, not an estimate"
        );
    }

    // The other two views. Both may legitimately be empty, so this reports rather than
    // asserts — except that reading a row must produce parseable JSON.
    for view in ["issues", "pulls"] {
        let dir = format!("{base}/{view}");
        match res.readdir(&MountPath::new(&dir)).await {
            Ok(rows) => {
                println!("{dir} has {} entries:", rows.len());
                for e in rows.iter().take(10) {
                    println!("  {}", e.name);
                }
                if let Some(first) = rows.first() {
                    let path = MountPath::new(format!("{dir}/{}", first.name));
                    let bytes = res
                        .read_bytes(&path, None)
                        .await
                        .unwrap_or_else(|e| panic!("read {}: {e}", path.as_str()));
                    let v: Value = serde_json::from_slice(&bytes)
                        .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.as_str()));
                    assert!(
                        v.get("number").and_then(|n| n.as_u64()).is_some(),
                        "a rendered row names its number"
                    );
                    println!(
                        "  read {} — {} bytes, title {:?}",
                        first.name,
                        bytes.len(),
                        v.get("title").and_then(|t| t.as_str()).unwrap_or("")
                    );
                }
            }
            // A token without Issues/Pull requests read scope: worth reporting, not a
            // failure of the code under test.
            Err(e) => println!("{dir} unavailable (token scope?): {e}"),
        }
    }
}
