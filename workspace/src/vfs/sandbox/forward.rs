use std::{net::SocketAddr, sync::Arc};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    runtime::Handle,
    task::JoinHandle,
};

use crate::vfs::sandbox::ForwardFs;

/// Host-side forward server exposing a [`ForwardFs`] over a tiny HTTP/1.1 API
/// for the in-guest FUSE forwarder. Bound to an OS-assigned ephemeral port;
/// requests must carry the session token in the `x-vfs-token` header. Aborts on
/// drop.
///
/// The served filesystem is any [`ForwardFs`]: a provider-only
/// [`Vfs`](crate::vfs::Vfs) or the unified `WorkspaceFs` (local files + provider
/// mounts). The server itself is filesystem-agnostic — it just adapts the HTTP
/// routes onto the trait.
///
/// Routes: `GET /readdir|/stat|/read?path=…[&offset=&size=]`, `PUT /write?path=…`,
/// `DELETE /unlink?path=…`, `POST /mkdir?path=…`, `DELETE /rmdir?path=…`,
/// `POST /rename?path=<from>&to=<to>`.
///
/// Vendored from ailoy's `src/vfs/sandbox/forward.rs` (61c4c43). Changes: the
/// per-run token is a `uuid` v4 (agent-k already depends on `uuid`) instead of
/// `getrandom` + `hex`; and the server is generic over [`ForwardFs`] rather than
/// bound to `Vfs`, so it can also serve the unified workspace tree.
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
    pub fn spawn(fs: Arc<dyn ForwardFs>, rt: &Handle) -> anyhow::Result<Self> {
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
                        let fs = fs.clone();
                        let token = task_token.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(stream, fs, token).await {
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

async fn handle_conn(
    mut stream: TcpStream,
    fs: Arc<dyn ForwardFs>,
    token: String,
) -> anyhow::Result<()> {
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
        ("GET", "/readdir") => readdir_json(&*fs, &path).await,
        ("GET", "/stat") => stat_json(&*fs, &path).await,
        ("GET", "/read") => {
            let offset = params.get("offset").and_then(|s| s.parse::<u64>().ok());
            let size = params.get("size").and_then(|s| s.parse::<u64>().ok());
            return match fs.read(&path, offset, size).await {
                Ok(data) => respond(&mut stream, 200, "application/octet-stream", data).await,
                Err(e) => respond(&mut stream, 500, "text/plain", e.to_string().into_bytes()).await,
            };
        }
        ("PUT", "/write") => {
            let body = read_body(&mut stream, &req).await?;
            // For a `.cmd/<op>` path this returns the command's JSON result (C4);
            // for a normal write it returns `{"ok":true}`; a read-only frontend
            // rejects it.
            fs.write(&path, body).await
        }
        ("DELETE", "/unlink") => fs.unlink(&path).await.map(|_| b"{\"ok\":true}".to_vec()),
        ("POST", "/mkdir") => fs.mkdir(&path).await.map(|_| b"{\"ok\":true}".to_vec()),
        ("DELETE", "/rmdir") => fs.rmdir(&path).await.map(|_| b"{\"ok\":true}".to_vec()),
        ("POST", "/rename") => {
            let to = params.get("to").cloned().unwrap_or_default();
            fs.rename(&path, &to)
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

async fn readdir_json(fs: &dyn ForwardFs, path: &str) -> anyhow::Result<Vec<u8>> {
    let items: Vec<serde_json::Value> = fs
        .readdir(path)
        .await?
        .into_iter()
        .map(|e| serde_json::json!({"name": e.name, "is_dir": e.is_dir, "size": e.size}))
        .collect();
    Ok(serde_json::to_vec(
        &serde_json::json!({ "entries": items }),
    )?)
}

async fn stat_json(fs: &dyn ForwardFs, path: &str) -> anyhow::Result<Vec<u8>> {
    let s = fs.stat(path).await?;
    if !s.exists {
        return Ok(serde_json::to_vec(&serde_json::json!({"exists": false}))?);
    }
    Ok(serde_json::to_vec(&serde_json::json!({
        "exists": true,
        "is_dir": s.is_dir,
        "size": s.size,
        "mtime": s.mtime,
        "atime": s.atime,
        "ctime": s.ctime,
    }))?)
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
