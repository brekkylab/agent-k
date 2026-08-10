//! Drive behind the wrapper production installs.
//!
//! Drives `CachedResource::new(GdriveResource)` — the pair `build_mounts` assembles —
//! against a mock of the Drive and Docs endpoints on a loopback socket, and counts what
//! the server was asked for: the `Range` of every request and the bytes it handed over.
//! Which is the only way to tell a window from a whole object, and so the only way to
//! see the difference these tests exist to hold.
//!
//! No credentials and no new dependency: the mock is a `tokio::net::TcpListener` and the
//! provider is pointed at it with `base_url`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::vfs::{
    accessor::{GdriveConfig, Origins},
    cache::CachedResource,
    path::MountPath,
    resource::{GdriveResource, Resource},
};

/// One request as the mock saw it: enough to tell a window from a whole object.
#[derive(Clone)]
struct Seen {
    target: String,
    range: Option<String>,
}

struct Mock {
    addr: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    body_bytes: Arc<Mutex<u64>>,
}

impl Mock {
    fn config(&self) -> GdriveConfig {
        GdriveConfig {
            client_id: "cid".into(),
            client_secret: "cs".into(),
            refresh_token: "rt".into(),
            origins: Origins::behind(&self.addr),
        }
    }

    /// Range headers of the `alt=media` requests, in order. `None` means the whole
    /// object was asked for.
    fn media_ranges(&self) -> Vec<Option<String>> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.target.contains("alt=media"))
            .map(|s| s.range.clone())
            .collect()
    }

    fn bytes_sent(&self) -> u64 {
        *self.body_bytes.lock().unwrap()
    }

    fn reset(&self) {
        self.seen.lock().unwrap().clear();
        *self.body_bytes.lock().unwrap() = 0;
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// `bytes=start-end` or `bytes=start-`.
fn parse_range(h: &str) -> Option<(u64, Option<u64>)> {
    let (s, e) = h.trim().strip_prefix("bytes=")?.split_once('-')?;
    Some((s.parse().ok()?, e.parse().ok()))
}

/// Serve a token, one folder listing, and one blob with `Range` support. A document
/// route answers with `pad` bytes of JSON and no `Content-Length`, the way the real
/// Docs API does.
async fn start_with_document(
    listing: Value,
    blobs: HashMap<String, Vec<u8>>,
    document_pad: Option<usize>,
) -> Mock {
    start_inner(listing, blobs, document_pad).await
}

/// Serve a token, one folder listing, and one blob with `Range` support.
async fn start(listing: Value, blobs: HashMap<String, Vec<u8>>) -> Mock {
    start_inner(listing, blobs, None).await
}

async fn start_inner(
    listing: Value,
    blobs: HashMap<String, Vec<u8>>,
    document_pad: Option<usize>,
) -> Mock {
    start_full(listing, blobs, document_pad, None).await
}

/// `drives_status` answers `/drives` with that status instead of a listing.
async fn start_full(
    listing: Value,
    blobs: HashMap<String, Vec<u8>>,
    document_pad: Option<usize>,
    drives_status: Option<u16>,
) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let body_bytes = Arc::new(Mutex::new(0u64));
    let (log, written, blobs) = (seen.clone(), body_bytes.clone(), Arc::new(blobs));
    let listing = Arc::new(listing);
    let document_pad = Arc::new(document_pad);
    let drives_status = Arc::new(drives_status);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (log, written, blobs, listing, document_pad, drives_status) = (
                log.clone(),
                written.clone(),
                blobs.clone(),
                listing.clone(),
                document_pad.clone(),
                drives_status.clone(),
            );
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
                let mut lines = head.lines();
                let start_line = lines.next().unwrap_or("").to_string();
                let mut parts = start_line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                // Drain an announced body (the token POST) so the client isn't left
                // writing into a socket nobody reads.
                if let Some(cl) =
                    header(&headers, "content-length").and_then(|v| v.parse::<usize>().ok())
                {
                    while buf.len() < head_end + cl {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                }
                let range = header(&headers, "range").map(str::to_string);
                log.lock().unwrap().push(Seen {
                    target: target.clone(),
                    range: range.clone(),
                });

                let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                let reply = |status: u16, body: Vec<u8>, extra: Option<String>| {
                    let mut h = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n",
                        body.len()
                    );
                    if let Some(e) = extra {
                        h.push_str(&e);
                        h.push_str("\r\n");
                    }
                    h.push_str("\r\n");
                    let mut out = h.into_bytes();
                    out.extend_from_slice(&body);
                    (out, body.len() as u64)
                };

                // A document, streamed without a length: what the Docs API does.
                if let Some(pad) = *document_pad
                    && path.contains("/documents/")
                {
                    let body = json!({ "body": "x".repeat(pad) }).to_string().into_bytes();
                    let head = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                                 Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        .to_vec();
                    let _ = sock.write_all(&head).await;
                    let mut sent = 0u64;
                    for c in body.chunks(64 * 1024) {
                        let framed = format!("{:x}\r\n", c.len()).into_bytes();
                        if sock.write_all(&framed).await.is_err()
                            || sock.write_all(c).await.is_err()
                            || sock.write_all(b"\r\n").await.is_err()
                        {
                            break;
                        }
                        sent += c.len() as u64;
                    }
                    let _ = sock.write_all(b"0\r\n\r\n").await;
                    *written.lock().unwrap() += sent;
                    let _ = sock.shutdown().await;
                    return;
                }
                let (out, body_len) = if method == "POST" && path.ends_with("/oauth2/token") {
                    reply(
                        200,
                        json!({"access_token": "at", "expires_in": 3600})
                            .to_string()
                            .into_bytes(),
                        None,
                    )
                } else if path.ends_with("/drives") {
                    match *drives_status {
                        Some(st) => reply(st, br#"{"error":"scripted"}"#.to_vec(), None),
                        None => reply(200, json!({"drives": []}).to_string().into_bytes(), None),
                    }
                } else if path.ends_with("/drive/v3/files") {
                    reply(
                        200,
                        json!({ "files": *listing }).to_string().into_bytes(),
                        None,
                    )
                } else if query.contains("alt=media") {
                    let id = path.rsplit('/').next().unwrap_or("").to_string();
                    let blob = blobs.get(&id).cloned().unwrap_or_default();
                    match range.as_deref().and_then(parse_range) {
                        Some((start, _)) if start >= blob.len() as u64 => {
                            reply(416, Vec::new(), None)
                        }
                        Some((start, end)) => {
                            let last = end
                                .unwrap_or(blob.len() as u64 - 1)
                                .min(blob.len() as u64 - 1);
                            let window = blob[start as usize..=last as usize].to_vec();
                            let cr = format!("Content-Range: bytes {start}-{last}/{}", blob.len());
                            reply(206, window, Some(cr))
                        }
                        None => reply(200, blob, None),
                    }
                } else {
                    reply(404, br#"{"error":"no route"}"#.to_vec(), None)
                };
                if sock.write_all(&out).await.is_ok() {
                    *written.lock().unwrap() += body_len;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock {
        addr,
        seen,
        body_bytes,
    }
}

/// The provider as `build_mounts` assembles it.
fn mounted(cfg: &GdriveConfig) -> CachedResource {
    CachedResource::new(Arc::new(GdriveResource::new(cfg).unwrap()))
}

fn row(name: &str, id: &str, mime: &str, size: Option<&str>) -> Value {
    let mut v = json!({
        "id": id,
        "name": name,
        "mimeType": mime,
        "modifiedTime": "2026-01-30T09:00:00Z",
    });
    if let Some(s) = size {
        v.as_object_mut().unwrap().insert("size".into(), json!(s));
    }
    v
}

/// A file Drive listed without a `size`, read the way every caller reads: list the
/// folder, then read windows of it.
///
/// The listing can only offer a placeholder, and the danger is that the placeholder
/// is believed. It is under the cacheable limit, so a reader that trusts it fetches
/// the whole object for each window and then throws it away, because the object is
/// over that limit once its real length is known: measured at 80 MB to deliver 1 MB
/// before the length was marked as the estimate it is.
#[tokio::test]
async fn an_unsized_file_is_measured_then_read_by_the_window() {
    const REAL: usize = 20 * 1024 * 1024;
    const CHUNK: u64 = 256 * 1024;
    let mock = start(
        json!([row("big.pdf", "P1", "application/pdf", None)]),
        HashMap::from([("P1".to_string(), vec![b'a'; REAL])]),
    )
    .await;
    let fs = mounted(&mock.config());
    let dir = MountPath::new("/My Drive");
    let file = MountPath::new("/My Drive/big.pdf");

    let listed = fs.readdir(&dir).await.unwrap();
    assert!(
        listed[0].size_is_estimate,
        "a listing that cannot know the length must say so"
    );

    // The stat behind the listing resolves it — one ranged byte, not a download.
    mock.reset();
    let st = fs.stat(&file).await.unwrap();
    assert_eq!(st.size, REAL as u64, "stat answers the real length");
    assert!(!st.size_is_estimate, "and marks it exact once measured");
    assert_eq!(
        mock.media_ranges(),
        vec![Some("bytes=0-0".to_string())],
        "measured by asking for one byte"
    );

    // Windows stay windows.
    mock.reset();
    for i in 0..4u64 {
        let got = fs
            .read_bytes_pinned(&file, Some(i * CHUNK..(i + 1) * CHUNK), &st)
            .await
            .unwrap();
        assert_eq!(got.len() as u64, CHUNK);
    }
    assert_eq!(
        mock.media_ranges(),
        (0..4)
            .map(|i| Some(format!("bytes={}-{}", i * CHUNK, (i + 1) * CHUNK - 1)))
            .collect::<Vec<_>>(),
        "each window asks for itself"
    );
    assert_eq!(
        mock.bytes_sent(),
        4 * CHUNK,
        "and the server sends only those windows"
    );
}

/// The same row as `ls -l` and a WebDAV `GET` see it: a handle's metadata has to be
/// the length, since that is what fills `Content-Length` and where `seek(End)` lands.
#[tokio::test]
async fn an_open_handle_reports_the_real_length_of_an_unsized_file() {
    const REAL: usize = 20 * 1024 * 1024;
    let mock = start(
        json!([row("big.pdf", "P1", "application/pdf", None)]),
        HashMap::from([("P1".to_string(), vec![b'a'; REAL])]),
    )
    .await;
    let fs = crate::WorkspaceFs::from_config(crate::vfs::FsConfig {
        local_root: None,
        mirror_root: None,
        mounts: vec![crate::vfs::MountSpec {
            prefix: "/gdrive".into(),
            provider: crate::vfs::ProviderConfig::Gdrive(mock.config()),
        }],
    })
    .unwrap();

    let mut f = fs
        .open(
            "/gdrive/My Drive/big.pdf",
            crate::OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        f.metadata().await.unwrap().len,
        REAL as u64,
        "a handle must not advertise the placeholder"
    );
}

/// A document is the other kind of placeholder: no ranged read exists for it, so the
/// wrapper has to fetch it whole and keep it, or every window rebuilds it.
#[tokio::test]
async fn a_document_is_built_once_and_served_from_the_cache() {
    const CHUNK: u64 = 256 * 1024;
    let mock = start(
        json!([row(
            "notes",
            "D1",
            "application/vnd.google-apps.document",
            None
        )]),
        HashMap::new(),
    )
    .await;
    let fs = mounted(&mock.config());
    let listed = fs.readdir(&MountPath::new("/My Drive")).await.unwrap();
    assert_eq!(listed[0].name, "notes.gdoc.json");
    assert!(listed[0].size_is_estimate);

    // The mock has no Docs endpoint, so a read fails — what matters is that the
    // listing already refused to call the placeholder a length, which is what keeps
    // `whole_or_small` from treating it as a small, cacheable object.
    let st = fs
        .stat(&MountPath::new("/My Drive/notes.gdoc.json"))
        .await
        .unwrap();
    assert!(st.serves_whole, "a document has no windows");
    assert!(st.size_is_estimate, "and no known length until it is built");
    assert!(
        fs.read_bytes_pinned(
            &MountPath::new("/My Drive/notes.gdoc.json"),
            Some(0..CHUNK),
            &st
        )
        .await
        .is_err(),
        "the mock serves no document API"
    );
}

/// A document over the ceiling must stop being read, not be read and then refused.
///
/// None of these endpoints declares a length — Docs, Slides and Sheets all answer as
/// an HTTP/2 stream — so a guard that checks `Content-Length` first and buffers second
/// never fires, and the memory it exists to bound is already spent by the time it
/// looks. Measured here by counting what the server managed to write.
#[tokio::test]
async fn an_oversized_document_stops_being_read() {
    const PAD: usize = 96 * 1024 * 1024; // over the 64 MiB document ceiling
    let mock = start_with_document(
        json!([row(
            "huge",
            "D1",
            "application/vnd.google-apps.document",
            None
        )]),
        HashMap::new(),
        Some(PAD),
    )
    .await;
    let fs = mounted(&mock.config());
    let path = MountPath::new("/My Drive/huge.gdoc.json");
    let st = fs.stat(&path).await.unwrap();

    assert!(
        fs.read_bytes_pinned(&path, Some(0..256 * 1024), &st)
            .await
            .is_err(),
        "a document past the ceiling is refused"
    );
    let sent = mock.bytes_sent();
    assert!(
        sent < PAD as u64 / 2,
        "the server should have been cut off early, but wrote {} MB of {} MB",
        sent / (1 << 20),
        PAD / (1 << 20)
    );
}

/// A shared-drive listing that fails must not become "this account has none", cached.
///
/// The call is best-effort, so its failure was swallowed: the root came back with two
/// sections, nothing said why, and the reduced root was cached for the listing TTL, so
/// a retry inside five minutes made no attempt at all. It also sat on the full retry
/// ladder, which put half a minute of backoff in front of the first `ls` of a mount.
#[tokio::test]
async fn a_failed_shared_drive_listing_is_not_cached_as_an_answer() {
    let mock = start_full(
        json!([row("a.txt", "F1", "text/plain", Some("3"))]),
        HashMap::new(),
        None,
        Some(500),
    )
    .await;
    let fs = mounted(&mock.config());
    let root = MountPath::new("/");

    let t0 = std::time::Instant::now();
    let first = fs.readdir(&root).await.unwrap();
    let elapsed = t0.elapsed();
    let drives_attempts = |m: &Mock| {
        m.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.target.contains("/drives"))
            .count()
    };
    assert_eq!(
        first.iter().map(|e| e.name.clone()).collect::<Vec<_>>(),
        vec!["My Drive".to_string(), "Shared with me".to_string()]
    );
    assert_eq!(
        drives_attempts(&mock),
        1,
        "one attempt: a best-effort listing does not walk the retry ladder"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "the first listing of a mount must not sit through backoff: {elapsed:?}"
    );

    // The reduced root is not kept by the provider, so a fresh mount tries again
    // rather than inheriting the failure. Within one mount the wrapper's own listing
    // cache still answers for its TTL — that layer has no per-listing way to say "this
    // one is incomplete", which is what it would take to recover sooner.
    let fresh = mounted(&mock.config());
    mock.reset();
    let _ = fresh.readdir(&root).await.unwrap();
    assert_eq!(
        drives_attempts(&mock),
        1,
        "a new mount asks again instead of inheriting a degraded root"
    );
}

/// A zero-length window asks the server for nothing.
///
/// It used to fall through to the arm that sends no `Range` at all, so `read_bytes(0)`
/// pulled the whole object and returned none of it — 20 MB to answer with an empty
/// vector.
#[tokio::test]
async fn an_empty_window_does_not_fetch_the_object() {
    const REAL: usize = 20 * 1024 * 1024;
    let mock = start(
        json!([row("big.pdf", "P1", "application/pdf", Some("20971520"))]),
        HashMap::from([("P1".to_string(), vec![b'a'; REAL])]),
    )
    .await;
    let fs = mounted(&mock.config());
    let file = MountPath::new("/My Drive/big.pdf");
    let st = fs.stat(&file).await.unwrap();

    mock.reset();
    let got = fs
        .read_bytes_pinned(&file, Some(1024..1024), &st)
        .await
        .unwrap();
    assert!(got.is_empty());
    assert_eq!(
        mock.media_ranges(),
        Vec::<Option<String>>::new(),
        "no request at all"
    );
    assert_eq!(mock.bytes_sent(), 0);
}

/// The provider's own caches are high-water marks unless something prunes them.
///
/// Nothing did: one listing per folder ever visited, held for the life of the mount
/// long after its TTL made it unusable, and the corpus has a folder that lists 10,000
/// entries. The wrapper above bounds both of its equivalents.
#[tokio::test]
async fn a_listing_cache_does_not_grow_without_bound() {
    let mock = start(
        json!([row("a.txt", "F1", "text/plain", Some("3"))]),
        HashMap::new(),
    )
    .await;
    let inner = Arc::new(GdriveResource::new(&mock.config()).unwrap());
    let fs = CachedResource::new(inner.clone());

    // The root plus one folder listing.
    let _ = fs.readdir(&MountPath::new("/")).await.unwrap();
    let _ = fs.readdir(&MountPath::new("/My Drive")).await.unwrap();
    assert!(inner.listings_retained().await >= 2);

    // Age them out. The next listing drops what expired instead of keeping it for the
    // life of the mount — two survive, because resolving `/Shared with me` re-lists the
    // root on the way to it, and both of those are fresh.
    inner.age_listings_for_test().await;
    let _ = fs
        .readdir(&MountPath::new("/Shared with me"))
        .await
        .unwrap();
    assert_eq!(
        inner.listings_retained().await,
        2,
        "expired listings are dropped, the fresh ones kept"
    );
}

/// Each service is reached at its own origin, and only its own.
///
/// A single `base_url` could not express this: it stood in for every host, with the
/// intermediate path (`/drive`, `/sheets`) chosen by the client rather than the
/// deployment, so a gateway serving one service somewhere else could not be pointed
/// at. Two listeners here, on paths neither Google nor our own mock uses.
#[tokio::test]
async fn one_service_can_move_without_moving_the_others() {
    let sheet = row(
        "budget",
        "S1",
        "application/vnd.google-apps.spreadsheet",
        None,
    );
    // Gateway A: Drive and the token endpoint, Drive on a path of its own choosing.
    let a = start(json!([sheet]), HashMap::new()).await;
    // Gateway B: Sheets only, on a different port.
    let b = start(json!([]), HashMap::new()).await;

    let fs = mounted(&GdriveConfig {
        client_id: "cid".into(),
        client_secret: "cs".into(),
        refresh_token: "rt".into(),
        origins: Origins {
            drive: Some(format!("{}/drive", a.addr)),
            oauth: Some(format!("{}/oauth2", a.addr)),
            sheets: Some(format!("{}/sheets", b.addr)),
            ..Default::default()
        },
    });

    let listed = fs.readdir(&MountPath::new("/My Drive")).await.unwrap();
    assert_eq!(listed[0].name, "budget.gsheet.json");

    // The listing came from A; the workbook has to come from B.
    let path = MountPath::new("/My Drive/budget.gsheet.json");
    let st = fs.stat(&path).await.unwrap();
    let _ = fs.read_bytes_pinned(&path, None, &st).await;

    let hit = |m: &Mock, needle: &str| {
        m.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.target.contains(needle))
            .count()
    };
    assert!(hit(&a, "/drive/v3/files") > 0, "A served the listing");
    assert!(hit(&a, "/oauth2/token") > 0, "A served the token");
    assert!(
        hit(&b, "/sheets/v4/spreadsheets/S1") > 0,
        "B served the workbook, so the override reached it"
    );
    assert_eq!(hit(&b, "/drive/v3"), 0, "and B was never asked for Drive");
    assert_eq!(
        hit(&a, "/sheets/v4"),
        0,
        "nor A for Sheets: one origin moving does not move the rest"
    );
}

/// The same window, asked backwards, through the pair a mount actually uses.
///
/// It has to be a large file: a small one is cached whole and sliced by the wrapper,
/// whose own slicing clamps. Past the cacheable limit the range goes down to the
/// provider, where the arithmetic deciding whether to slice ran `end - start` on it
/// (underflow) and the slice indexed `data[4..2]`. Neither errors; both abort the
/// process, and a filesystem read is where a client's range arrives.
#[tokio::test]
async fn a_backwards_window_does_not_take_the_process_down() {
    const REAL: usize = 10 * 1024 * 1024; // over the wrapper's cacheable limit
    let mock = start(
        json!([row(
            "big.bin",
            "B1",
            "application/octet-stream",
            Some("10485760")
        )]),
        HashMap::from([("B1".to_string(), vec![b'z'; REAL])]),
    )
    .await;
    let fs = mounted(&mock.config());
    let file = MountPath::new("/My Drive/big.bin");
    let st = fs.stat(&file).await.unwrap();
    assert!(
        !whole_or_small_enough(&st),
        "the range must reach the provider"
    );

    // Built from values, since a literal backwards range is a lint — but one arriving
    // from a client is just two numbers.
    for (start, end) in [(3_000_000u64, 1_000_000u64), (99_999_999, 10), (4096, 4096)] {
        let got = fs
            .read_bytes_pinned(&file, Some(start..end), &st)
            .await
            .expect("a backwards window answers instead of panicking");
        assert!(got.is_empty(), "{start}..{end} should read nothing");
    }
    // And a forwards one still works.
    let got = fs
        .read_bytes_pinned(&file, Some(0..512), &st)
        .await
        .unwrap();
    assert_eq!(got.len(), 512);
}

/// Mirrors the wrapper's own rule, so the test above can assert it is *not* the one
/// doing the slicing.
fn whole_or_small_enough(st: &crate::vfs::resource::FileStat) -> bool {
    st.serves_whole || st.size == 0 || (!st.size_is_estimate && st.size <= 8 << 20)
}
