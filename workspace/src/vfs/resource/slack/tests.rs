//! Slack mount tests.
//!
//! The pure layers — path resolution, naming, the date range — are unit-tested
//! here without a network. The tree itself is exercised against enterprise-mock
//! (`slack_mock`) and a real workspace (`slack_live`), both `#[ignore]`d.

use super::*;

// ---- path resolution ------------------------------------------------------

/// Every id the tree needs is in the entry names, so a path resolves with no
/// cached parent and no request. This is what lets a cold deep path (a guest
/// re-mount, a WebDAV GET after a restart) work at all.
#[test]
fn a_deep_path_resolves_from_its_segments_alone() {
    let r = |p: &str| resolve(&MountPath::new(p));
    assert_eq!(r("/"), Some(Node::Root));
    assert_eq!(r("/channels"), Some(Node::Convs { dms: false }));
    assert_eq!(r("/dms"), Some(Node::Convs { dms: true }));
    assert_eq!(r("/users"), Some(Node::Users));
    assert_eq!(
        r("/users/kim__U0456.json"),
        Some(Node::User { id: "U0456".into() })
    );
    assert_eq!(
        r("/channels/general__C0123"),
        Some(Node::Conv { id: "C0123".into() })
    );
    // A day, and the two children it holds.
    let day = Scope {
        id: "C0123".into(),
        date: "2026-08-03".into(),
        ts: None,
    };
    let base = "/channels/general__C0123/2026-08-03";
    assert_eq!(r(base), Some(Node::Convo(day.clone())));
    assert_eq!(
        r(&format!("{base}/chat.jsonl")),
        Some(Node::Chat(day.clone()))
    );
    assert_eq!(r(&format!("{base}/files")), Some(Node::Files(day.clone())));
    assert_eq!(
        r(&format!("{base}/files/spec__F0456.pdf")),
        Some(Node::File {
            scope: day.clone(),
            name: "spec__F0456.pdf".into()
        })
    );
    assert_eq!(
        r(&format!("{base}/threads")),
        Some(Node::Threads {
            id: "C0123".into(),
            date: "2026-08-03".into()
        })
    );

    // A thread is the SAME shape as a day — that is the point of `Scope`, and it
    // is why the tree needs one explanation rather than two.
    let thread = Scope {
        ts: Some("1754210000.123456".into()),
        ..day
    };
    let tbase = format!("{base}/threads/1754210000.123456");
    assert_eq!(r(&tbase), Some(Node::Convo(thread.clone())));
    assert_eq!(
        r(&format!("{tbase}/chat.jsonl")),
        Some(Node::Chat(thread.clone()))
    );
    assert_eq!(
        r(&format!("{tbase}/files")),
        Some(Node::Files(thread.clone()))
    );
    assert_eq!(
        r(&format!("{tbase}/files/deck__F0456.pptx")),
        Some(Node::File {
            scope: thread,
            name: "deck__F0456.pptx".into()
        })
    );

    // DMs route through the same shapes.
    assert_eq!(
        r("/dms/kim__D0789/2026-08-03/threads"),
        Some(Node::Threads {
            id: "D0789".into(),
            date: "2026-08-03".into()
        })
    );
}

/// `resolve` accepts any well-formed date, because it works from segments alone —
/// which means the existence check has to happen later, uniformly. It used to
/// happen in `stat` only, so a date outside a conversation's range was `NotFound`
/// to `stat` while `readdir` listed its three children and `cat chat.jsonl`
/// returned 0 bytes; worse, that read was a real `conversations.history` call, so
/// any date at all could spend a request. Covered against a server in
/// `slack_mock_tree_walk`; this pins the resolution half.
#[test]
fn a_well_formed_date_resolves_and_is_checked_later() {
    // 1999 predates Slack, but the path is still well-formed…
    let far_past = resolve(&MountPath::new(
        "/channels/general__C0123/1999-01-01/chat.jsonl",
    ));
    assert!(
        far_past.is_some(),
        "resolution is by shape, not by existence"
    );
    // …so every node under a day must be one `require_scope` gates. If a variant
    // is added here that skips it, this is the reminder.
    let base = "/channels/general__C0123/1999-01-01";
    for (p, gated) in [
        (base.to_string(), true),
        (format!("{base}/chat.jsonl"), true),
        (format!("{base}/files"), true),
        (format!("{base}/files/a__F1.pdf"), true),
        (format!("{base}/threads"), true),
        (format!("{base}/threads/1.2"), true),
        (format!("{base}/threads/1.2/chat.jsonl"), true),
    ] {
        let node = resolve(&MountPath::new(&p)).unwrap_or_else(|| panic!("{p} should resolve"));
        let has_scope = matches!(
            node,
            Node::Convo(_)
                | Node::Chat(_)
                | Node::Files(_)
                | Node::File { .. }
                | Node::Threads { .. }
        );
        assert_eq!(has_scope, gated, "{p}");
    }
}

/// Slack has no nested threads, so a thread has no `threads/` of its own — and a
/// path claiming one must not resolve to the day's.
#[test]
fn a_thread_has_no_threads_of_its_own() {
    let base = "/channels/general__C0123/2026-08-03/threads/1754210000.123456";
    assert_eq!(resolve(&MountPath::new(format!("{base}/threads"))), None);
    assert_eq!(
        resolve(&MountPath::new(format!("{base}/threads/1754210001.1"))),
        None
    );
}

/// A `ts` becomes a `conversations.replies` argument, so an arbitrary segment must
/// not reach one — the same reasoning as [`valid_slack_id`] for channel ids.
#[test]
fn only_timestamp_shaped_segments_become_a_thread() {
    for good in ["1754210000.123456", "1754210000", "0.1"] {
        assert!(valid_ts(good).is_some(), "{good} should pass");
    }
    for bad in [
        "",
        "1754210000.123456.789", // two dots
        "abc",
        "1754210000?x=1",
        "1754210000/../y",
        ".123456",                             // must start with a digit
        "12345678901234567890123456789012345", // over the length bound
    ] {
        assert!(valid_ts(bad).is_none(), "{bad:?} should be rejected");
    }
    // And a rejected ts makes the whole path unresolvable, at every depth below.
    let base = "/channels/general__C0123/2026-08-03/threads";
    for bad in [
        format!("{base}/abc"),
        format!("{base}/abc/chat.jsonl"),
        format!("{base}/abc/files"),
        format!("{base}/abc/files/f__F1.pdf"),
    ] {
        assert_eq!(resolve(&MountPath::new(&bad)), None, "should reject {bad}");
    }
}

/// A path that names nothing must resolve to `None` rather than to some
/// plausible node — a bad segment reaching a Slack request is how a path becomes
/// an unintended API call.
#[test]
fn unresolvable_paths_are_rejected() {
    for bad in [
        "/nope",                                             // not a section
        "/channels/no-id-here",                              // no `__<id>` tail
        "/channels/general__C0123/not-a-date",               // date level isn't a date
        "/channels/general__C0123/2026-13-45",               // not a real date
        "/channels/general__C0123/2026-08-03/other",         // unknown leaf
        "/channels/general__C0123/2026-08-03/threads/x.txt", // wrong suffix
        "/channels/general__C0123/2026-08-03/nope/x",        // unknown subdirectory
        "/users/kim__U0456",                                 // profile needs .json
        "/channels/general__C0123/2026-08-03/files/a/b",     // too deep
    ] {
        assert_eq!(resolve(&MountPath::new(bad)), None, "should reject {bad:?}");
    }
}

/// An id goes into a request URL, so a path must not be able to smuggle a query
/// or a traversal into one through its `__<id>` tail.
#[test]
fn only_slack_shaped_ids_are_accepted() {
    for good in ["C0123ABC", "U04ABC123", "D0789"] {
        assert!(valid_slack_id(good), "{good} should pass");
    }
    for bad in [
        "",
        "c0123", // Slack ids are uppercase
        "C0123?query=1",
        "C0123/../other",
        "C 123",
        "C0123ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789", // over the length bound
    ] {
        assert!(!valid_slack_id(bad), "{bad:?} should be rejected");
    }
    // And a rejected id makes the whole path unresolvable.
    assert_eq!(resolve(&MountPath::new("/channels/x__c0123")), None);
}

/// The `__<id>` split takes the LAST separator, so a display name containing
/// `__` still yields the id.
#[test]
fn id_extraction_takes_the_last_separator() {
    assert_eq!(id_from_name("a__b__C0123").as_deref(), Some("C0123"));
    assert_eq!(id_from_name("general__C0123").as_deref(), Some("C0123"));
    // A collision suffix belongs to the entry name, not to the id.
    assert_eq!(id_from_name("general__C0123(2)").as_deref(), Some("C0123"));
    assert_eq!(id_from_name("no-separator"), None);
    assert_eq!(id_from_name("trailing__"), None);
}

// ---- naming ---------------------------------------------------------------

#[test]
fn sanitize_collapses_path_breaking_characters() {
    // `/` would otherwise become an extra path segment.
    assert_eq!(sanitize("team/eng"), "team_eng");
    assert_eq!(sanitize("proj: q3 review"), "proj_q3_review");
    // Repeats squeeze, edges trim.
    assert_eq!(sanitize("  a   b  "), "a_b");
    assert_eq!(sanitize("--x--"), "--x--");
    // Non-ASCII is kept: it is a valid filename and the agent reads these.
    assert_eq!(sanitize("공지사항"), "공지사항");
    assert_eq!(sanitize("설계 리뷰"), "설계_리뷰");
    // An all-punctuation name would sanitize to nothing.
    assert_eq!(sanitize("///"), "unnamed");
    assert_eq!(sanitize("   "), "unnamed");
}

/// A filename is capped at 255 **bytes** on ext4/XFS but 255 *characters* on
/// macOS, so a name that works on a dev machine can fail in deployment. The byte
/// bound is what catches it, and the cut must land on a char boundary.
#[test]
fn long_names_are_bounded_in_bytes_not_just_characters() {
    let cjk = "한".repeat(300); // 3 bytes each
    let out = sanitize(&cjk);
    assert!(out.len() <= NAME_MAX_BYTES, "{} bytes", out.len());
    assert!(out.chars().count() <= NAME_MAX);
    // Not corrupted mid-character (this would panic on a bad cut).
    assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    assert!(out.ends_with("..."), "{out}");

    // An ASCII name is bounded by the character cap.
    let long = "a".repeat(300);
    assert_eq!(sanitize(&long).chars().count(), NAME_MAX);
}

/// The extension survives sanitization, so tools that dispatch on it (`docling`,
/// a `*.pdf` glob) still work on a downloaded file.
#[test]
fn file_names_keep_their_extension() {
    assert_eq!(
        file_blob_name("Q3 Report.pdf", "F0456"),
        "Q3_Report__F0456.pdf"
    );
    // Mixed case normalizes so a glob doesn't miss it.
    assert_eq!(file_blob_name("a.PDF", "F1"), "a__F1.pdf");
    // A dotted stem must not have its tail read as an extension.
    assert_eq!(file_blob_name("v1.2 notes", "F2"), "v1.2_notes__F2");
    // No extension at all.
    assert_eq!(file_blob_name("Makefile", "F3"), "Makefile__F3");
    // A path-breaking name still becomes one segment.
    assert_eq!(
        file_blob_name("../../etc/passwd", "F4"),
        ".._.._etc_passwd__F4"
    );
}

/// Two files of one name in a day must stay distinct entries, and the number goes
/// BEFORE the extension — numbered after it, the name is unique but drops out of
/// every glob that would find it.
#[test]
fn duplicate_names_are_numbered_before_the_extension() {
    let mut v = vec![
        ("a__F1.pdf".to_string(),),
        ("a__F1.pdf".to_string(),),
        ("b__F2".to_string(),),
        ("b__F2".to_string(),),
    ];
    dedup_names(&mut v, |t| &mut t.0);
    assert_eq!(v[0].0, "a__F1.pdf");
    assert_eq!(v[1].0, "a__F1(2).pdf");
    assert_eq!(v[2].0, "b__F2");
    assert_eq!(v[3].0, "b__F2(2)");
    // A numbered name still resolves to its id.
    assert_eq!(id_from_name("a__F1(2)").as_deref(), Some("F1"));
}

/// A DM has no name of its own — Slack names it after the other person.
#[test]
fn a_dm_is_named_after_the_other_person() {
    let mut names = HashMap::new();
    names.insert("U0456".to_string(), "kim jihoon".to_string());
    let dm = serde_json::json!({"id": "D0789", "user": "U0456"});
    assert_eq!(conv_label(&dm, &names), "kim jihoon");
    // A partner the member list didn't cover: `convs` resolves those with
    // `users.info` before getting here (measured against a real workspace, where
    // a DM with Slack's own `USLACK` account was absent from `users.list`). If
    // even that fails, the id is a poor name but a stable, non-empty one.
    let unknown = serde_json::json!({"id": "D0790", "user": "U9999"});
    assert_eq!(conv_label(&unknown, &names), "U9999");
    // A channel uses its own name.
    let ch = serde_json::json!({"id": "C1", "name": "general"});
    assert_eq!(conv_label(&ch, &names), "general");
}

// ---- message rendering ----------------------------------------------------

fn one_name() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("U0456".to_string(), "kim jihoon".to_string());
    m
}

/// A message carries only ids, which a reader cannot resolve line by line. The
/// name is *added* rather than substituted: the id is the stable identity (a
/// display name can change or repeat, and `users/<name>__<id>.json` is found by
/// id), so both must survive.
#[test]
fn a_message_line_gains_a_name_and_keeps_its_id() {
    let m = serde_json::json!({"user": "U0456", "text": "hi", "ts": "1.2"});
    let line = message_line(&m, &one_name());
    assert!(line.ends_with(b"\n"), "one JSON object per line");
    let v: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(v["user"], "U0456", "the id must survive");
    assert_eq!(v["user_name"], "kim jihoon");
    // Everything else passes through untouched.
    assert_eq!(v["ts"], "1.2");
}

/// An id the member list doesn't cover gets no name at all — a wrong name is worse
/// than a raw id.
#[test]
fn an_unresolvable_author_gets_no_name() {
    let m = serde_json::json!({"user": "U9999", "text": "hi"});
    let v: Value = serde_json::from_slice(&message_line(&m, &one_name())).unwrap();
    assert_eq!(v["user"], "U9999");
    assert!(v.get("user_name").is_none(), "must not invent a name");
    // A message with no author at all (some subtypes) is passed through.
    let m = serde_json::json!({"subtype": "channel_join", "text": "x"});
    let v: Value = serde_json::from_slice(&message_line(&m, &one_name())).unwrap();
    assert!(v.get("user_name").is_none());
}

/// A mention is `<@U0456>` in the raw text, which reads as noise. The body is what
/// a person actually reads, so it is rewritten in place.
#[test]
fn mentions_in_the_body_become_names() {
    let names = one_name();
    assert_eq!(resolve_mentions("<@U0456> hi", &names), "@kim jihoon hi");
    // Slack's labelled form.
    assert_eq!(resolve_mentions("<@U0456|kim>!", &names), "@kim jihoon!");
    // Several in one line, and text on both sides.
    assert_eq!(
        resolve_mentions("cc <@U0456> and <@U0456> ok", &names),
        "cc @kim jihoon and @kim jihoon ok"
    );
    // Untouched: no mention, an unknown id, an unterminated token.
    assert_eq!(resolve_mentions("plain text", &names), "plain text");
    assert_eq!(resolve_mentions("<@U9999> hi", &names), "<@U9999> hi");
    assert_eq!(resolve_mentions("a <@U0456 b", &names), "a <@U0456 b");
    // A channel link is not a user mention and must not be mangled.
    assert_eq!(
        resolve_mentions("<#C123|general>", &names),
        "<#C123|general>"
    );
    // And the rewrite reaches the serialized line.
    let m = serde_json::json!({"user": "U0456", "text": "<@U0456> ping"});
    let v: Value = serde_json::from_slice(&message_line(&m, &names)).unwrap();
    assert_eq!(v["text"], "@kim jihoon ping");
}

/// `users.info` returns the same shape `users.list` does, so one resolved partner
/// names its DM exactly as a listed member would.
#[test]
fn a_partner_resolved_by_users_info_names_its_dm() {
    // The real `users.info?user=USLACK` response, reduced to the naming fields.
    let resolved = serde_json::json!({
        "id": "USLACK", "name": "slack", "real_name": "Slack",
        "profile": {"display_name": "Slack", "real_name": "Slack"}
    });
    assert_eq!(display_name(&resolved), "Slack");
    let mut names = HashMap::new();
    names.insert("USLACK".to_string(), display_name(&resolved).to_string());
    let dm = serde_json::json!({"id": "D0BND485ETS", "user": "USLACK"});
    assert_eq!(conv_label(&dm, &names), "Slack");
    assert_eq!(sanitize(&conv_label(&dm, &names)), "Slack");
}

#[test]
fn user_display_name_prefers_the_human_facing_fields() {
    let u = serde_json::json!({
        "id": "U1", "name": "kim.jihoon",
        "profile": {"display_name": "지훈", "real_name": "Kim Jihoon"}
    });
    assert_eq!(display_name(&u), "지훈");
    assert_eq!(user_filename(&u, "U1"), "지훈__U1.json");
    // Empty display_name falls through to real_name, then to name.
    let u2 = serde_json::json!({
        "id": "U2", "name": "bot", "profile": {"display_name": "", "real_name": "Deploy Bot"}
    });
    assert_eq!(display_name(&u2), "Deploy Bot");
    let u3 = serde_json::json!({"id": "U3", "name": "plain"});
    assert_eq!(display_name(&u3), "plain");
}

/// A file with no id or no URL has nothing to serve — a deleted-file tombstone
/// must not become an entry that errors on read.
#[test]
fn files_without_bytes_are_not_listed() {
    assert!(file_meta(&serde_json::json!({"name": "a.pdf"})).is_none());
    assert!(file_meta(&serde_json::json!({"id": "F1", "name": "a.pdf"})).is_none());
    let ok = file_meta(&serde_json::json!({
        "id": "F1", "name": "a.pdf", "size": 42,
        "url_private_download": "https://files.slack.com/f/F1/a.pdf",
        "timestamp": 1754210000i64,
    }))
    .expect("a complete file listing");
    assert_eq!(ok.vfs_name, "a__F1.pdf");
    // The size comes from the listing, so it is exact with nothing downloaded.
    assert_eq!(ok.size, 42);
    assert!(ok.mtime.is_some());
}

// ---- dates ----------------------------------------------------------------

/// 2026-08-03T12:00:00Z — midday, so the fixture doesn't sit on a day boundary
/// where an off-by-one in either direction would still look right.
const AUG_3_NOON: f64 = 1_785_758_400.0;

#[test]
fn the_date_range_runs_back_from_the_newest_message() {
    let newest = AUG_3_NOON;
    let created = newest as i64 - 2 * 86_400;
    let dates = date_range(newest, created);
    assert_eq!(dates.first().unwrap(), "2026-08-03", "newest first");
    assert_eq!(dates.len(), 3, "{dates:?}");
    assert_eq!(dates.last().unwrap(), "2026-08-01");
}

/// A years-old channel must not list thousands of date directories.
#[test]
fn the_date_range_is_capped() {
    let dates = date_range(AUG_3_NOON, 0); // created at the epoch
    assert_eq!(dates.len() as i64, MAX_DAYS);
    assert_eq!(dates.first().unwrap(), "2026-08-03");
}

/// A conversation created after its newest message (clock skew, or a `created` we
/// couldn't read) must still list the day that message is in — not an empty tree.
#[test]
fn a_created_after_the_newest_message_still_lists_that_day() {
    let dates = date_range(AUG_3_NOON, AUG_3_NOON as i64 + 10 * 86_400);
    assert_eq!(dates, vec!["2026-08-03".to_string()]);
}

#[test]
fn day_bounds_span_exactly_one_utc_day() {
    let (start, next) = day_bounds("2026-08-03").expect("a real date");
    assert_eq!(next - start, 86_400);
    assert_eq!(fmt_ts(start), format!("{start}.000000"));
    assert_eq!(date_mtime("2026-08-03"), epoch_secs(start));
    assert!(day_bounds("2026-13-45").is_none());
}

#[test]
fn slicing_a_read_clamps_to_the_data() {
    let d = b"hello world";
    assert_eq!(slice(d, &None), d);
    assert_eq!(slice(d, &Some(0..5)), b"hello");
    // Past EOF reads empty rather than panicking.
    assert!(slice(d, &Some(50..60)).is_empty());
    // An inverted range is empty, not a panic. Built from values so the literal
    // form doesn't trip the empty-range lint — the point is what `slice` does
    // with one, which a caller can hand us.
    let (hi, lo) = (8u64, 2u64);
    assert!(slice(d, &Some(hi..lo)).is_empty());
}

// ---- against a server -----------------------------------------------------

/// Tokens from the environment: `SLACK_USER_TOKEN` (`xoxp-`, what the mount
/// really wants) and/or `SLACK_BOT_TOKEN` (`xoxb-`, the fallback). `None` when
/// neither is set, which is how the ignored tests skip themselves.
fn env_tokens() -> Option<(Option<String>, Option<String>)> {
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let user = nonempty("SLACK_USER_TOKEN");
    let bot = nonempty("SLACK_BOT_TOKEN");
    (user.is_some() || bot.is_some()).then_some((user, bot))
}

/// Config for the enterprise-mock (`app/routers/slack.py`), which serves the
/// Slack API under `{base}/slack/api`.
fn mock_config() -> Option<SlackConfig> {
    let (user_token, bot_token) = env_tokens()?;
    Some(SlackConfig {
        user_token,
        bot_token,
        team_id: "T0000MOCK".into(),
        team_name: "mock".into(),
        base_url: Some(std::env::var("SLACK_BASE_URL").ok()?),
    })
}

fn live_config() -> Option<SlackConfig> {
    let (user_token, bot_token) = env_tokens()?;
    Some(SlackConfig {
        user_token,
        bot_token,
        team_id: std::env::var("SLACK_TEAM_ID").unwrap_or_else(|_| "T-live".into()),
        team_name: std::env::var("SLACK_TEAM_NAME").unwrap_or_else(|_| "live-test".into()),
        base_url: None,
    })
}

fn names(entries: &[DirEntry]) -> Vec<String> {
    entries.iter().map(|e| e.name.clone()).collect()
}

/// Walk the whole tree against enterprise-mock: the three sections, a channel's
/// dates, one day's three children, `chat.jsonl`'s contents, a thread if the day
/// has one, and the user profiles.
///
/// The mock has **no `oauth.v2.access`**, so the code exchange is only covered by
/// the live test; this starts from a token.
///
///   SLACK_BASE_URL=<mock origin> SLACK_USER_TOKEN=usr-… \
///     cargo test -p workspace slack_mock -- --ignored --nocapture
///
/// Tokens come from the mock's own `GET /_mock/users`. A big corpus is what makes
/// this worth running — measured against one with 36 channels and 327 users, a
/// listing pages and takes seconds, exercising what a 3-member workspace never
/// reaches. It is also close to the client's 30s timeout, so run the two mock
/// tests one at a time (`slack_mock_tree_walk`, then `slack_mock_search`) rather
/// than letting them contend.
#[tokio::test]
#[ignore = "requires a reachable enterprise-mock (SLACK_BASE_URL + a SLACK_*_TOKEN)"]
async fn slack_mock_tree_walk() {
    let Some(cfg) = mock_config() else {
        eprintln!("set SLACK_BASE_URL and SLACK_USER_TOKEN (or SLACK_BOT_TOKEN) to run");
        return;
    };
    let r = SlackResource::new(&cfg).unwrap();

    let root = names(&r.readdir(&MountPath::root()).await.expect("root"));
    assert_eq!(root, vec![CHANNELS, DMS, USERS]);

    let channels = r
        .readdir(&MountPath::new("/channels"))
        .await
        .expect("channels");
    println!("channels: {}", channels.len());
    assert!(!channels.is_empty(), "the mock corpus has channels");
    for c in channels.iter().take(5) {
        println!("  {}", c.name);
        // The listing name must round-trip to a usable id.
        let id = id_from_name(&c.name).expect("an id in the name");
        assert!(valid_slack_id(&id), "{id}");
    }

    // Users are files, not directories.
    let users = r.readdir(&MountPath::new("/users")).await.expect("users");
    assert!(!users.is_empty());
    assert!(users.iter().all(|u| matches!(u.kind, FileKind::File)));
    let profile = r
        .read_bytes(&MountPath::new(format!("/users/{}", users[0].name)), None)
        .await
        .expect("a profile");
    let v: Value = serde_json::from_slice(&profile).expect("valid JSON");
    assert!(v.get("id").is_some(), "{v}");

    // Find a channel with dates and read one day.
    let mut walked = 0;
    for c in channels.iter() {
        let conv = MountPath::new(format!("/channels/{}", c.name));
        let dates = r.readdir(&conv).await.expect("dates");
        if dates.is_empty() {
            continue;
        }
        // Newest first, and every entry is a date directory.
        assert!(
            dates.iter().all(|d| is_date(&d.name)),
            "{:?}",
            names(&dates)
        );
        assert!(dates.len() as i64 <= MAX_DAYS);

        // Descending a day lists exactly the three children.
        let day_path = conv.child(&dates[0].name);
        let children = names(&r.readdir(&day_path).await.expect("day"));
        assert_eq!(children, vec![CHAT_FILE, FILES_DIR, THREADS_DIR]);

        let chat = r
            .read_bytes(&day_path.child(CHAT_FILE), None)
            .await
            .expect("chat.jsonl");
        // Every line must be one JSON object — this is what `jq` relies on.
        for line in chat.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            serde_json::from_slice::<Value>(line).expect("one JSON object per line");
        }
        println!(
            "{}/{}: chat.jsonl {} bytes, {} lines",
            c.name,
            dates[0].name,
            chat.len(),
            chat.iter().filter(|b| **b == b'\n').count()
        );
        // stat must agree with what a read returns, or Content-Length is wrong.
        let st = r.stat(&day_path.child(CHAT_FILE)).await.expect("stat chat");
        assert_eq!(st.size, chat.len() as u64);

        let threads = r
            .readdir(&day_path.child(THREADS_DIR))
            .await
            .expect("threads");
        // One directory per thread, each shaped exactly like a day.
        assert!(
            threads.iter().all(|e| matches!(e.kind, FileKind::Dir)),
            "a thread is a directory: {:?}",
            names(&threads)
        );
        for t in threads.iter().take(2) {
            let tp = day_path.child(THREADS_DIR).child(&t.name);
            assert_eq!(
                names(&r.readdir(&tp).await.expect("thread")),
                vec![CHAT_FILE, FILES_DIR],
                "a thread holds the same two children a day does"
            );
            let bytes = r
                .read_bytes(&tp.child(CHAT_FILE), None)
                .await
                .expect("a thread");
            let lines = bytes
                .split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .count();
            // A listed thread has a root plus at least one reply.
            assert!(lines >= 2, "{} has {lines} line(s)", t.name);
            println!("  thread {} -> {lines} messages", t.name);
            // stat must agree with the read, as it does for a day.
            assert_eq!(
                r.stat(&tp.child(CHAT_FILE)).await.expect("stat").size,
                bytes.len() as u64
            );
            let in_thread = r.readdir(&tp.child(FILES_DIR)).await.expect("thread files");
            println!("    {} file(s) inside it", in_thread.len());
        }

        let files = r.readdir(&day_path.child(FILES_DIR)).await.expect("files");
        println!("  {} file(s)", files.len());
        walked += 1;
        if walked == 2 {
            break;
        }
    }
    assert!(walked > 0, "no channel in the mock had any dates");

    // A path that names nothing is a 404, not an error or an empty success.
    assert!(matches!(
        r.stat(&MountPath::new("/channels/nope__C9999999")).await,
        Err(ResourceError::NotFound)
    ));

    // A real conversation but a date outside its range: every operation must
    // agree it does not exist. `stat` alone used to say so while `readdir` listed
    // three children and `cat chat.jsonl` spent a real request to return 0 bytes.
    let first = names(&r.readdir(&MountPath::new("/channels")).await.unwrap())[0].clone();
    let unborn = MountPath::new(format!("/{CHANNELS}/{first}/1999-01-01"));
    for p in [
        unborn.clone(),
        unborn.child(CHAT_FILE),
        unborn.child(FILES_DIR),
        unborn.child(THREADS_DIR),
    ] {
        assert!(
            matches!(r.stat(&p).await, Err(ResourceError::NotFound)),
            "stat {} must be NotFound",
            p.as_str()
        );
        assert!(
            matches!(r.readdir(&p).await, Err(ResourceError::NotFound)),
            "readdir {} must be NotFound",
            p.as_str()
        );
        assert!(
            matches!(r.read_bytes(&p, None).await, Err(ResourceError::NotFound)),
            "read {} must be NotFound",
            p.as_str()
        );
    }

    // Read-only.
    assert!(matches!(
        r.write_bytes(&MountPath::new("/channels/x__C1/a"), vec![1])
            .await,
        Err(ResourceError::Unsupported)
    ));
}

/// Search reaches Slack's own index (inside files it indexed) — the one thing
/// reading the tree cannot do. Needs a user token; skipped without one.
#[tokio::test]
#[ignore = "requires a running enterprise-mock + SLACK_USER_TOKEN"]
async fn slack_mock_search() {
    let Some(cfg) = mock_config() else {
        eprintln!("set SLACK_BASE_URL and SLACK_USER_TOKEN to run");
        return;
    };
    let r = SlackResource::new(&cfg).unwrap();
    if !r.accessor.search_available() {
        eprintln!("set SLACK_USER_TOKEN to exercise search (a bot token cannot search)");
        return;
    }
    // An empty query must not become a search for the whole workspace.
    assert!(r.command("search", b"").await.is_err());
    assert!(r.command("search", b"   ").await.is_err());
    // An unknown command is an error, not a silent no-op.
    assert!(r.command("post", b"{}").await.is_err());

    let out = r.command("search", b"the").await.expect("search");
    let v: Value = serde_json::from_slice(&out).expect("valid JSON");
    let total = v
        .get("messages")
        .and_then(|m| m.get("total"))
        .and_then(Value::as_u64);
    println!("search 'the': total={total:?}");
    assert!(v.get("ok").and_then(Value::as_bool) == Some(true), "{v}");
}

/// Live round-trip against a real Slack workspace, reporting the wall-clock of
/// each operation — this is where the app's rate-limit tier shows up, and the
/// numbers are what the mount prompt's warnings are based on. Run with:
///
///   SLACK_USER_TOKEN=xoxp-… cargo test -p workspace slack_live -- --ignored --nocapture
///
/// A `SLACK_BOT_TOKEN` works too, but exercises less: no DMs, no search, and only
/// the channels the bot was invited to.
#[tokio::test]
#[ignore = "requires SLACK_USER_TOKEN (or SLACK_BOT_TOKEN) + network"]
async fn slack_live_tree_and_reads() {
    let Some(cfg) = live_config() else {
        eprintln!("set SLACK_USER_TOKEN (or SLACK_BOT_TOKEN) to run");
        return;
    };
    let r = SlackResource::new(&cfg).unwrap();
    let timed = |label: &'static str| {
        let t = Instant::now();
        move |extra: String| println!("{label}: {:.2}s {extra}", t.elapsed().as_secs_f64())
    };

    let done = timed("channels");
    let channels = r
        .readdir(&MountPath::new("/channels"))
        .await
        .expect("channels");
    done(format!("{} entries", channels.len()));
    for c in channels.iter().take(10) {
        println!("  {}", c.name);
    }

    let done = timed("users");
    let users = r.readdir(&MountPath::new("/users")).await.expect("users");
    done(format!("{} members", users.len()));

    let done = timed("dms");
    // A bot token (or an install without im:read) lists no DMs; that is a scope
    // fact, not a failure of the mount.
    let dms = r
        .readdir(&MountPath::new("/dms"))
        .await
        .unwrap_or_else(|e| {
            println!("dms unavailable: {e}");
            Vec::new()
        });
    done(format!("{} conversations", dms.len()));
    for d in dms.iter().take(5) {
        println!("  {}", d.name);
    }

    // Channels and DMs are the same three-child shape, so exercise both — a DM's
    // name comes from the other person rather than a channel name, which is the
    // one naming rule only real data checks.
    match channels.first() {
        Some(c) => walk_newest_day(&r, &MountPath::new(format!("/channels/{}", c.name))).await,
        None => println!("no channels visible to this token"),
    }
    match dms.first() {
        Some(d) => walk_newest_day(&r, &MountPath::new(format!("/dms/{}", d.name))).await,
        None => println!("no DMs visible (a bot token never has any)"),
    }

    if r.accessor.search_available() {
        let done = timed("search");
        let out = r.command("search", b"the").await.expect("search");
        done(format!("{} bytes of JSON", out.len()));
    } else {
        println!("no SLACK_USER_TOKEN: search unavailable (bot tokens cannot search)");
    }
}

/// Read the newest non-empty day of one conversation: its three children,
/// `chat.jsonl`, a thread, and an attachment (whole + ranged), timing each. Shared
/// by the channel and DM legs of the live test.
#[cfg(test)]
async fn walk_newest_day(r: &SlackResource, conv: &MountPath) {
    let timed = |label: &'static str| {
        let t = Instant::now();
        move |extra: String| println!("{label}: {:.2}s {extra}", t.elapsed().as_secs_f64())
    };
    println!("--- {} ---", conv.as_str());

    let done = timed("dates");
    let dates = match r.readdir(conv).await {
        Ok(d) => d,
        Err(e) => {
            println!("dates unavailable: {e}");
            return;
        }
    };
    done(format!("{} day(s)", dates.len()));
    assert!(
        dates.iter().all(|d| is_date(&d.name)),
        "every entry at this level must be a date: {:?}",
        names(&dates)
    );

    for d in dates.iter().take(10) {
        let day = conv.child(&d.name);
        let done = timed("day listing");
        let children = names(&r.readdir(&day).await.expect("day"));
        done(format!("{children:?}"));
        assert_eq!(children, vec![CHAT_FILE, FILES_DIR, THREADS_DIR]);

        let done = timed("cat chat.jsonl");
        let chat = r
            .read_bytes(&day.child(CHAT_FILE), None)
            .await
            .expect("chat");
        let lines = chat
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .count();
        done(format!("{} bytes, {lines} message(s)", chat.len()));
        for line in chat.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            serde_json::from_slice::<Value>(line).expect("one JSON object per line");
        }
        // stat must agree with the read, or a WebDAV GET's Content-Length lies.
        assert_eq!(
            r.stat(&day.child(CHAT_FILE)).await.expect("stat chat").size,
            chat.len() as u64
        );
        if lines == 0 {
            continue; // a quiet day; try the next one
        }

        // One directory per thread, each shaped exactly like a day — so reading a
        // thread is the same three steps as reading a day.
        let threads = r.readdir(&day.child(THREADS_DIR)).await.expect("threads");
        println!("  {} thread(s) that day", threads.len());
        assert!(
            threads.iter().all(|e| matches!(e.kind, FileKind::Dir)),
            "a thread is a directory: {:?}",
            names(&threads)
        );
        if let Some(t) = threads.first() {
            let tp = day.child(THREADS_DIR).child(&t.name);
            let done = timed("thread listing");
            let children = names(&r.readdir(&tp).await.expect("thread"));
            done(format!("{children:?}"));
            assert_eq!(children, vec![CHAT_FILE, FILES_DIR]);

            let done = timed("cat a thread's chat.jsonl");
            let bytes = r
                .read_bytes(&tp.child(CHAT_FILE), None)
                .await
                .expect("thread");
            done(format!("{} bytes", bytes.len()));
            // stat agrees with the read, exactly as it does for a day.
            assert_eq!(
                r.stat(&tp.child(CHAT_FILE)).await.expect("stat").size,
                bytes.len() as u64
            );
            // A listed thread is a root plus at least one reply.
            let n = bytes
                .split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .count();
            assert!(n >= 2, "{} carried {n} message(s)", t.name);

            // Attachments posted INSIDE the thread live in ITS files/ — the day's
            // cannot hold them (history returns roots only).
            let tf = tp.child(FILES_DIR);
            let in_thread = r.readdir(&tf).await.expect("thread files");
            println!("  {} file(s) inside that thread", in_thread.len());
            if let Some(f) = in_thread.iter().find(|f| f.size > 0) {
                let done = timed("cat a thread file");
                let b = r
                    .read_bytes(&tf.child(&f.name), None)
                    .await
                    .expect("thread file bytes");
                done(format!("{} of {} bytes ({})", b.len(), f.size, f.name));
                assert_eq!(b.len() as u64, f.size, "listed size must be exact");
            }
        }

        let files = r.readdir(&day.child(FILES_DIR)).await.expect("files");
        println!("  {} file(s) that day", files.len());
        if let Some(f) = files.iter().find(|f| f.size > 0) {
            let p = day.child(FILES_DIR).child(&f.name);
            let done = timed("cat a file");
            let whole = r.read_bytes(&p, None).await.expect("file bytes");
            done(format!("{} of {} bytes ({})", whole.len(), f.size, f.name));
            // The listing's size is Slack's own, so it must be the real length.
            assert_eq!(whole.len() as u64, f.size, "listed size must be exact");
            // A ranged read moves only its window.
            let n = 16.min(whole.len() as u64);
            let head = r.read_bytes(&p, Some(0..n)).await.expect("ranged");
            assert_eq!(head, whole[..n as usize], "ranged read must match");
            println!("  ranged read of {n} bytes matched");
        }
        return;
    }
    println!("  the newest 10 days were all empty");
}
