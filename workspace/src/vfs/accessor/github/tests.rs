//! GitHub accessor tests: the pure layers, none of which need a network.
//!
//! What is checked here is everything that decides a URL or a retry before any request is
//! built — identifier validation, tree-row parsing, and the backoff ladder. The endpoints
//! themselves are exercised through the mounted provider in
//! `crate::vfs::resource::tests::github_mounted`.

use super::*;

#[test]
fn repo_segments_reject_url_escapes() {
    for ok in ["rust-lang", "agent_k", "repo.rs", "a1"] {
        assert!(valid_repo_segment(ok), "should accept {ok:?}");
    }
    for bad in [
        "", "..", ".", "a/b", "a?b", "a#b", "../repos", "a b", "a%2Fb",
    ] {
        assert!(!valid_repo_segment(bad), "should reject {bad:?}");
    }
}

/// A branch keeps its slashes — that is the whole reason it is validated separately
/// from an owner or a repo — while everything git forbids in a ref stays rejected.
#[test]
fn branches_keep_slashes_but_cannot_climb_out() {
    for ok in ["main", "release/2026-03", "feat/a/b", "v1.2.x"] {
        assert!(valid_branch(ok), "should accept {ok:?}");
    }
    for bad in [
        "",
        "/main",
        "main/",
        "-main",
        "a..b",
        "a//b",
        "main.lock",
        "a b",
        "a?b",
        "a#b",
        "a~b",
        "a^b",
        "a:b",
        "a[b",
        "a*b",
        "a%2Fb",
        "HEAD@{1}",
    ] {
        assert!(!valid_branch(bad), "should reject {bad:?}");
    }
}

#[test]
fn shas_are_hex_of_git_object_id_length() {
    assert!(valid_sha(&"a".repeat(40)));
    assert!(valid_sha(&"0".repeat(64)));
    assert!(valid_sha("2222222222222222222222222222222222222222"));
    for bad in ["", "abc", &"g".repeat(40), &"a".repeat(39), &"a".repeat(41)] {
        assert!(!valid_sha(bad), "should reject {bad:?}");
    }
}

#[test]
fn backoff_is_exponential_jittered_and_capped() {
    for _ in 0..50 {
        assert!(backoff_delay(0) >= Duration::from_secs(1));
        assert!(backoff_delay(0) <= Duration::from_millis(2000));
        assert!(backoff_delay(1) >= Duration::from_secs(2));
        // 2^4 = 16s already at the cap, so anything above it stays there.
        assert_eq!(backoff_delay(4), MAX_BACKOFF);
        assert_eq!(backoff_delay(30), MAX_BACKOFF);
    }
}

#[test]
fn retry_after_parses_delta_seconds_only() {
    assert_eq!(parse_delta_seconds("60"), Some(Duration::from_secs(60)));
    assert_eq!(parse_delta_seconds(" 5 "), Some(Duration::from_secs(5)));
    assert_eq!(parse_delta_seconds("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    assert_eq!(parse_delta_seconds(""), None);
}

/// A source naming a bad owner, repo or ref fails at construction, before any of it can
/// reach a URL.
#[test]
fn construction_rejects_bad_identifiers() {
    let src = |owner: &str, repo: Option<&str>, git_ref: Option<&str>| GithubSource {
        owner: owner.into(),
        token: "t".into(),
        repo: repo.map(str::to_string),
        git_ref: git_ref.map(str::to_string),
    };
    let build = |s: &GithubSource| GithubAccessor::new(s, None, None);

    assert!(build(&src("o", None, None)).is_ok());
    assert!(build(&src("o", Some("r"), None)).is_ok());
    assert!(build(&src("o", Some("r"), Some("release/1"))).is_ok());
    assert!(build(&src("../o", None, None)).is_err());
    assert!(build(&src("o", Some("r?x"), None)).is_err());
    assert!(build(&src("o", Some("r"), Some("a b"))).is_err());

    // A ref without a repo is rejected rather than silently applied to every repository
    // the token reaches: one ref cannot describe a set of them.
    assert!(build(&src("o", None, Some("main"))).is_err());

    let mut empty_token = src("o", None, None);
    empty_token.token = "  ".into();
    assert!(build(&empty_token).is_err());
}

/// `repo` now arrives from a path segment rather than from config, so every URL built with
/// one revalidates it — otherwise a crafted segment could address another endpoint.
#[test]
fn a_repo_from_a_path_segment_is_revalidated() {
    let acc = GithubAccessor::new(
        &GithubSource {
            owner: "octo".into(),
            token: "t".into(),
            repo: None,
            git_ref: None,
        },
        None,
        Some("https://example.test"),
    )
    .expect("build accessor");

    let url = acc.repo_url("demo", "/git/trees/x").expect("valid repo");
    assert_eq!(url, "https://example.test/repos/octo/demo/git/trees/x");

    for bad in ["../../other", "a/b", "x?y", "x#y", "..", ""] {
        assert!(
            acc.repo_url(bad, "").is_err(),
            "should reject repo segment {bad:?}"
        );
    }
}

/// A listing row is kept only when it belongs to this source's owner — the account may
/// reach several — and only when its name is one this code could put back into a URL.
#[test]
fn repo_rows_are_filtered_to_the_sources_owner() {
    let acc = GithubAccessor::new(
        &GithubSource {
            owner: "Octo".into(),
            token: "t".into(),
            repo: None,
            git_ref: None,
        },
        None,
        None,
    )
    .unwrap();
    let row = |owner: &str, name: &str| {
        serde_json::json!({
            "name": name,
            "owner": {"login": owner},
            "pushed_at": "2026-03-01T00:00:00Z",
            "archived": false,
        })
    };

    // Owner names are case-insensitive on GitHub, so the comparison is too.
    let mine = acc.repo_row(&row("octo", "demo")).expect("same owner");
    assert_eq!(mine.name, "demo");
    assert!(mine.pushed_at.is_some());
    assert!(!mine.archived);

    assert!(acc.repo_row(&row("someone-else", "demo")).is_none());
    assert!(acc.repo_row(&row("octo", "bad/name")).is_none());
    assert!(acc.repo_row(&serde_json::json!({"name": "x"})).is_none());

    let archived = acc
        .repo_row(&serde_json::json!({
            "name": "old", "owner": {"login": "octo"}, "archived": true,
        }))
        .expect("archived rows still parse");
    assert!(archived.archived, "the caller decides whether to show it");
}

/// The tree rows this code keeps, and the ones it drops rather than guess at.
#[test]
fn tree_rows_are_parsed_by_type() {
    let sha = "a".repeat(40);
    let blob = serde_json::json!({"path": "a.rs", "type": "blob", "sha": sha, "size": 12});
    let parsed = parse_entry(&blob).expect("blob row");
    assert_eq!(parsed.kind, EntryKind::Blob);
    assert_eq!(parsed.size, Some(12));

    let tree = serde_json::json!({"path": "src", "type": "tree", "sha": sha});
    assert_eq!(parse_entry(&tree).unwrap().kind, EntryKind::Tree);

    // A submodule's sha names a commit elsewhere, so it is kept without being
    // required to be a blob id this repository can serve.
    let sub = serde_json::json!({"path": "vendor", "type": "commit", "sha": "x"});
    assert_eq!(parse_entry(&sub).unwrap().kind, EntryKind::Submodule);

    for bad in [
        serde_json::json!({"type": "blob", "sha": sha}),
        serde_json::json!({"path": "a", "type": "wat", "sha": sha}),
        serde_json::json!({"path": "a", "type": "blob", "sha": "nothex"}),
    ] {
        assert!(parse_entry(&bad).is_none(), "should drop {bad}");
    }
}
