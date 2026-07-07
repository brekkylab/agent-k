use std::{net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Handle,
    task::JoinHandle,
};

use crate::vfs::{Vfs, path::VPath, resource::FileKind};

/// Host-side forward server exposing a [`Vfs`] over a tiny HTTP/1.1 API for the
/// in-guest FUSE forwarder. Bound to an OS-assigned ephemeral port; requests
/// must carry the session token in the `x-vfs-token` header. Aborts on drop.
///
/// Routes: `GET /readdir|/stat|/read?path=…[&offset=&size=]`, `PUT /write?path=…`,
/// `DELETE /unlink?path=…`, `POST /mkdir?path=…`, `DELETE /rmdir?path=…`,
/// `POST /rename?path=<from>&to=<to>`.
///
/// Vendored from ailoy's `src/vfs/sandbox/forward.rs` (61c4c43). Only change:
/// the per-run token is a `uuid` v4 (agent-k already depends on `uuid`) instead
/// of `getrandom` + `hex`, avoiding two new deps.
pub struct VfsForward {
    addr: SocketAddr,
    token: String,
    task: JoinHandle<()>,
}

impl Drop for VfsForward {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl VfsForward {
    pub fn spawn(vfs: Arc<Vfs>, rt: &Handle) -> anyhow::Result<Self> {
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;

        // Per-run bearer token for the loopback/guest link. A v4 UUID (128 bits
        // of randomness, rendered as 32 hex chars) is ample for this.
        let token = uuid::Uuid::new_v4().as_simple().to_string();

        let task_token = token.clone();
        let task = rt.spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("vfs forward: listener init failed: {e}");
                    return;
                }
            };
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let vfs = vfs.clone();
                        let token = task_token.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(stream, vfs, token).await {
                                tracing::debug!("vfs forward: connection error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::debug!("vfs forward: accept error: {e}");
                    }
                }
            }
        });

        Ok(Self { addr, token, task })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

struct Req {
    method: String,
    path: String,
    query: String,
    token: Option<String>,
    content_length: usize,
    body_prefix: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> anyhow::Result<Req> {
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            anyhow::bail!("connection closed before headers");
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 64 * 1024 {
            anyhow::bail!("request header too large");
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };

    let mut token = None;
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            match key.as_str() {
                "x-vfs-token" => token = Some(val.to_string()),
                "content-length" => content_length = val.parse().unwrap_or(0),
                _ => {}
            }
        }
    }

    let body_prefix = buf[header_end + 4..].to_vec();
    Ok(Req {
        method,
        path,
        query,
        token,
        content_length,
        body_prefix,
    })
}

async fn handle_conn(mut stream: TcpStream, vfs: Arc<Vfs>, token: String) -> anyhow::Result<()> {
    let req = read_request(&mut stream).await?;

    if req.token.as_deref() != Some(token.as_str()) {
        return respond(&mut stream, 403, "text/plain", b"forbidden".to_vec()).await;
    }

    let params = parse_query(&req.query);
    let path = params
        .get("path")
        .cloned()
        .unwrap_or_else(|| "/".to_string());

    let result = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/readdir") => readdir_json(&vfs, &path).await,
        ("GET", "/stat") => stat_json(&vfs, &path).await,
        ("GET", "/read") => {
            let offset = params.get("offset").and_then(|s| s.parse::<u64>().ok());
            let size = params.get("size").and_then(|s| s.parse::<u64>().ok());
            return match read_bytes(&vfs, &path, offset, size).await {
                Ok(data) => respond(&mut stream, 200, "application/octet-stream", data).await,
                Err(e) => respond(&mut stream, 500, "text/plain", e.to_string().into_bytes()).await,
            };
        }
        ("PUT", "/write") => {
            let body = read_body(&mut stream, &req).await?;
            // For a `.cmd/<op>` path this returns the command's JSON result (C4);
            // for a normal write it returns `{"ok":true}`.
            write_bytes(&vfs, &path, body).await
        }
        ("DELETE", "/unlink") => unlink_path(&vfs, &path)
            .await
            .map(|_| b"{\"ok\":true}".to_vec()),
        ("POST", "/mkdir") => mkdir_path(&vfs, &path)
            .await
            .map(|_| b"{\"ok\":true}".to_vec()),
        ("DELETE", "/rmdir") => rmdir_path(&vfs, &path)
            .await
            .map(|_| b"{\"ok\":true}".to_vec()),
        ("POST", "/rename") => {
            let to = params.get("to").cloned().unwrap_or_default();
            rename_path(&vfs, &path, &to)
                .await
                .map(|_| b"{\"ok\":true}".to_vec())
        }
        _ => return respond(&mut stream, 404, "text/plain", b"not found".to_vec()).await,
    };

    match result {
        Ok(json) => respond(&mut stream, 200, "application/json", json).await,
        Err(e) => respond(&mut stream, 500, "text/plain", e.to_string().into_bytes()).await,
    }
}

async fn read_body(stream: &mut TcpStream, req: &Req) -> anyhow::Result<Vec<u8>> {
    let mut body = req.body_prefix.clone();
    while body.len() < req.content_length {
        let mut tmp = [0u8; 8192];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(req.content_length);
    Ok(body)
}

async fn readdir_json(vfs: &Vfs, path: &str) -> anyhow::Result<Vec<u8>> {
    let entries: Vec<(String, FileKind, u64)> = if path == "/" {
        vfs.mount_names()
            .into_iter()
            .map(|n| (n, FileKind::Dir, 0))
            .collect()
    } else {
        let (res, vp) = vfs
            .route(path)
            .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
        res.readdir(&vp)
            .await?
            .into_iter()
            .map(|e| (e.name, e.kind, e.size))
            .collect()
    };
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|(name, kind, size)| {
            serde_json::json!({"name": name, "is_dir": matches!(kind, FileKind::Dir), "size": size})
        })
        .collect();
    Ok(serde_json::to_vec(
        &serde_json::json!({ "entries": items }),
    )?)
}

async fn stat_json(vfs: &Vfs, path: &str) -> anyhow::Result<Vec<u8>> {
    if path == "/" {
        return Ok(serde_json::to_vec(
            &serde_json::json!({"exists": true, "is_dir": true, "size": 0}),
        )?);
    }
    let Some((res, vp)) = vfs.route(path) else {
        return Ok(serde_json::to_vec(&serde_json::json!({"exists": false}))?);
    };
    match res.stat(&vp).await {
        Ok(s) => {
            let mtime = s
                .mtime
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            Ok(serde_json::to_vec(&serde_json::json!({
                "exists": true,
                "is_dir": matches!(s.kind, FileKind::Dir),
                "size": s.size,
                "mtime": mtime,
            }))?)
        }
        Err(_) => Ok(serde_json::to_vec(&serde_json::json!({"exists": false}))?),
    }
}

async fn read_bytes(
    vfs: &Vfs,
    path: &str,
    offset: Option<u64>,
    size: Option<u64>,
) -> anyhow::Result<Vec<u8>> {
    let (res, vp) = vfs
        .route(path)
        .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
    let range = match (offset, size) {
        (Some(o), Some(s)) => Some(o..o + s),
        _ => None,
    };
    // The forward server speaks to the vendored core, whose `Resource` returns a
    // typed `VfsError`; collapse it into `anyhow` for the HTTP layer.
    res.read_bytes(&vp, range).await.map_err(|e| anyhow::anyhow!("{e}"))
}

async fn unlink_path(vfs: &Vfs, path: &str) -> anyhow::Result<()> {
    let (res, vp) = vfs
        .route(path)
        .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
    res.unlink(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
}

async fn mkdir_path(vfs: &Vfs, path: &str) -> anyhow::Result<()> {
    let (res, vp) = vfs
        .route(path)
        .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
    res.mkdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
}

async fn rmdir_path(vfs: &Vfs, path: &str) -> anyhow::Result<()> {
    let (res, vp) = vfs
        .route(path)
        .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
    res.rmdir(&vp).await.map_err(|e| anyhow::anyhow!("{e}"))
}

async fn rename_path(vfs: &Vfs, from: &str, to: &str) -> anyhow::Result<()> {
    let (res, from_vp) = vfs
        .route(from)
        .ok_or_else(|| anyhow::anyhow!("no mount for {from}"))?;
    let (_, to_vp) = vfs
        .route(to)
        .ok_or_else(|| anyhow::anyhow!("no mount for {to}"))?;
    res.rename(&from_vp, &to_vp)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Returns the command result JSON for a `/<mount>/.cmd/<op>` path (C4 read-back),
/// or `{"ok":true}` for a normal byte write.
async fn write_bytes(vfs: &Vfs, path: &str, data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let (res, vp): (_, VPath) = vfs
        .route(path)
        .ok_or_else(|| anyhow::anyhow!("no mount for {path}"))?;
    if let Some(op) = vp.as_str().strip_prefix("/.cmd/") {
        res.command(op, &data)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    } else {
        res.write_bytes(&vp, data)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(b"{\"ok\":true}".to_vec())
    }
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn parse_query(query: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    map
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod live {
    //! Step 1 host smoke test: spawn the forward server over a Notion-mounted
    //! `Vfs` and prove its HTTP surface serves real Notion — no guest/sandbox.
    //! This exercises the full host chain HTTP → Vfs.route → NotionResource →
    //! Notion API. Ignored by default:
    //!
    //!   NOTION_API_KEY=ntn_… cargo test -p agent-k-backend forward_live_notion -- --ignored --nocapture
    use std::sync::Arc;

    use super::VfsForward;
    use crate::vfs::{MountSpec, NotionConfig, ProviderConfig, Vfs, VfsConfig};

    #[tokio::test]
    #[ignore = "requires NOTION_API_KEY + network"]
    async fn forward_live_notion() {
        let api_key =
            std::env::var("NOTION_API_KEY").expect("set NOTION_API_KEY to run this live check");
        let vfs = Vfs::from_config(VfsConfig {
            mounts: vec![MountSpec {
                prefix: "/notion".into(),
                provider: ProviderConfig::Notion(NotionConfig { api_key }),
            }],
        })
        .expect("build vfs");

        let fwd = VfsForward::spawn(Arc::new(vfs), &tokio::runtime::Handle::current())
            .expect("spawn forward server");
        let base = format!("http://127.0.0.1:{}", fwd.port());
        let client = reqwest::Client::new();
        let get = |url: String| {
            let c = client.clone();
            let token = fwd.token().to_string();
            async move { c.get(url).header("x-vfs-token", token).send().await.unwrap() }
        };

        // Wrong token → 403 (auth works).
        let forbidden = client
            .get(format!("{base}/readdir?path=/"))
            .header("x-vfs-token", "wrong")
            .send()
            .await
            .unwrap();
        assert_eq!(forbidden.status(), 403, "bad token must be rejected");

        // Root lists the mount name.
        let root = get(format!("{base}/readdir?path=/")).await.text().await.unwrap();
        println!("readdir / -> {root}");
        assert!(root.contains("notion"), "root should list the notion mount");

        // The mount's page list.
        let pages_json = get(format!("{base}/readdir?path=/notion/pages"))
            .await
            .text()
            .await
            .unwrap();
        println!("readdir /notion/pages -> {pages_json}");
        let pages: serde_json::Value = serde_json::from_str(&pages_json).unwrap();
        let entries = pages["entries"].as_array().expect("entries array");
        let Some(first) = entries.first() else {
            println!("no pages shared with this integration");
            return;
        };
        let name = first["name"].as_str().unwrap();

        // Read that page's rendered page.json over /read.
        let read_path = format!("/notion/pages/{name}/page.json");
        let bytes = get(format!("{base}/read?path={read_path}"))
            .await
            .bytes()
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        println!("--- GET /read {read_path} ({} bytes) ---", bytes.len());
        println!("{}", &text[..text.len().min(1200)]);
        assert!(bytes.len() > 0, "page.json should have content");
    }
}
