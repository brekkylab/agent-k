//! Standalone e2e for the host-side forward server ([`VfsForward`]): spins it up
//! over a [`WorkspaceFs`] and drives the full HTTP/1.1 forward protocol
//! (readdir/stat/read/write/mkdir/rename/unlink/rmdir + token auth) over a real
//! loopback socket — the same wire the in-guest FUSE forwarder speaks, but with
//! no VM/ailoy. Uses a tiny raw-TCP client so the crate needs no HTTP-client dep.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;
use workspace::{ForwardFs, VfsForward, WorkspaceFs};

struct Resp {
    status: u16,
    body: Vec<u8>,
}

impl Resp {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// One HTTP/1.1 request over a fresh loopback connection (the server answers
/// `Connection: close`, so one request per socket). `target` is the raw
/// path+query, e.g. `/stat?path=/files/x`.
async fn call(port: u16, token: &str, method: &str, target: &str, body: &[u8]) -> Resp {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.expect("connect");
    let head = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nx-vfs-token: {token}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    s.flush().await.unwrap();

    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has no header terminator");
    let status_line = String::from_utf8_lossy(&raw[..sep]);
    let status = status_line
        .lines()
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .expect("status code");
    Resp {
        status,
        body: raw[sep + 4..].to_vec(),
    }
}

fn serve(root: std::path::PathBuf) -> VfsForward {
    let fs: Arc<dyn ForwardFs> = Arc::new(WorkspaceFs::new(root, Uuid::new_v4()));
    VfsForward::spawn(fs, &tokio::runtime::Handle::current()).expect("spawn forward server")
}

// Multi-thread so the server's accept loop runs on its own worker while the
// test awaits client round-trips.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forward_http_drives_workspace_over_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let fwd = serve(tmp.path().to_path_buf());
    let (port, token) = (fwd.port(), fwd.token().to_string());

    // write -> stat -> read (full + ranged) under the reserved local `files/`.
    assert_eq!(
        call(port, &token, "PUT", "/write?path=/files/note.txt", b"hello http")
            .await
            .status,
        200
    );

    let st = call(port, &token, "GET", "/stat?path=/files/note.txt", b"").await;
    assert_eq!(st.status, 200);
    let t = st.text();
    assert!(t.contains("\"exists\":true"), "{t}");
    assert!(t.contains("\"is_dir\":false"), "{t}");
    assert!(t.contains("\"size\":10"), "{t}");

    assert_eq!(
        call(port, &token, "GET", "/read?path=/files/note.txt", b"").await.body,
        b"hello http"
    );
    assert_eq!(
        call(port, &token, "GET", "/read?path=/files/note.txt&offset=6&size=4", b"")
            .await
            .body,
        b"http"
    );

    // readdir: root exposes the reserved `files` dir; `/files` lists the file.
    assert!(
        call(port, &token, "GET", "/readdir?path=/", b"")
            .await
            .text()
            .contains("\"name\":\"files\""),
    );
    assert!(
        call(port, &token, "GET", "/readdir?path=/files", b"")
            .await
            .text()
            .contains("\"name\":\"note.txt\""),
    );

    // mkdir -> nested write -> list.
    assert_eq!(call(port, &token, "POST", "/mkdir?path=/files/sub", b"").await.status, 200);
    assert_eq!(
        call(port, &token, "PUT", "/write?path=/files/sub/a.txt", b"x").await.status,
        200
    );
    assert!(
        call(port, &token, "GET", "/readdir?path=/files/sub", b"")
            .await
            .text()
            .contains("\"name\":\"a.txt\""),
    );

    // rename -> the source is gone, the destination exists.
    assert_eq!(
        call(port, &token, "POST", "/rename?path=/files/note.txt&to=/files/renamed.txt", b"")
            .await
            .status,
        200
    );
    assert!(
        call(port, &token, "GET", "/stat?path=/files/renamed.txt", b"")
            .await
            .text()
            .contains("\"exists\":true"),
    );
    assert!(
        call(port, &token, "GET", "/stat?path=/files/note.txt", b"")
            .await
            .text()
            .contains("\"exists\":false"),
    );

    // unlink + rmdir clean up.
    assert_eq!(
        call(port, &token, "DELETE", "/unlink?path=/files/renamed.txt", b"").await.status,
        200
    );
    assert_eq!(
        call(port, &token, "DELETE", "/unlink?path=/files/sub/a.txt", b"").await.status,
        200
    );
    assert_eq!(call(port, &token, "DELETE", "/rmdir?path=/files/sub", b"").await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forward_http_rejects_a_bad_token() {
    let tmp = tempfile::tempdir().unwrap();
    let fwd = serve(tmp.path().to_path_buf());
    let r = call(fwd.port(), "not-the-token", "GET", "/stat?path=/", b"").await;
    assert_eq!(r.status, 403);
}
