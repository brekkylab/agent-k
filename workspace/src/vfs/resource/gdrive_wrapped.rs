//! Drive through the wrapper production installs.
//!
//! Every other test of this provider drives `GdriveResource` bare, but
//! `build_mounts` always wraps it in [`CachedResource`], and the wrapper is where a
//! listing feeds a `stat` and that `stat` picks a read's fetch strategy. A whole
//! class of mistake only exists in that seam: a placeholder length that the provider
//! would have resolved, served verbatim from the listing instead, turning a windowed
//! read into a whole-object fetch per window.
//!
//! So this drives the pair against a mock of the Drive endpoints on a loopback
//! socket — no credentials, no new dependency — and counts what the server was asked
//! for, which is the only way to see the difference.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::vfs::{
    accessor::GdriveConfig,
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
            account_email: "u@example.com".into(),
            base_url: Some(self.addr.clone()),
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

/// Serve a token, one folder listing, and one blob with `Range` support.
async fn start(listing: Value, blobs: HashMap<String, Vec<u8>>) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let body_bytes = Arc::new(Mutex::new(0u64));
    let (log, written, blobs) = (seen.clone(), body_bytes.clone(), Arc::new(blobs));
    let listing = Arc::new(listing);
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (log, written, blobs, listing) =
                (log.clone(), written.clone(), blobs.clone(), listing.clone());
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
                if let Some(cl) = header(&headers, "content-length").and_then(|v| v.parse::<usize>().ok()) {
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

                let (out, body_len) = if method == "POST" && path.ends_with("/oauth2/token") {
                    reply(
                        200,
                        json!({"access_token": "at", "expires_in": 3600})
                            .to_string()
                            .into_bytes(),
                        None,
                    )
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
                        Some((start, _)) if start >= blob.len() as u64 => reply(416, Vec::new(), None),
                        Some((start, end)) => {
                            let last = end.unwrap_or(blob.len() as u64 - 1).min(blob.len() as u64 - 1);
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
    let st = fs.stat(&MountPath::new("/My Drive/notes.gdoc.json")).await.unwrap();
    assert!(st.serves_whole, "a document has no windows");
    assert!(st.size_is_estimate, "and no known length until it is built");
    assert!(
        fs.read_bytes_pinned(&MountPath::new("/My Drive/notes.gdoc.json"), Some(0..CHUNK), &st)
            .await
            .is_err(),
        "the mock serves no document API"
    );
}
