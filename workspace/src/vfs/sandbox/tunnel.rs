//! Raw-FUSE tunnel host engine — runs the FUSE protocol engine on the host and
//! serves a [`ForwardFs`] (the unified `WorkspaceFs`) to an in-guest FUSE
//! client. Counterpart of the guest pump (`crates/ailoy-vfs-tunnel`), which
//! mounts `/dev/fuse` and relays the raw FUSE wire protocol over one persistent
//! TCP connection — no protocol conversion. The backend injects that pump and
//! points it at a [`TunnelServer`] started here.
//!
//! The engine runs `fuser::Session::from_fd` (no mount; the `macos-no-mount`
//! feature makes that compile+run on a macOS host) against the socketpair below.
//!
//! Transport bridge: fuser insists on a message-boundary fd (one message per
//! read/write, like `/dev/fuse`). TCP is a byte stream, so per connection we
//! open a `socketpair(AF_UNIX, SOCK_DGRAM)` — `SOCK_SEQPACKET` is unsupported on
//! macOS — and a shim relays between the stream (reframed via each FUSE header's
//! `len`) and the datagram fd handed to fuser. Socket buffers are enlarged so
//! the ~128 KiB read replies fit a single datagram (macOS caps small by default).
//!
//! Read-path only for now (lookup/getattr/readdir/open/read); mutating ops fall
//! through to the trait's ENOSYS defaults.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo,
    KernelConfig, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, Request,
    SessionACL,
};
use subtle::ConstantTimeEq;
use tokio::runtime::Handle;

use super::{ForwardFs, FwdStat};

const TTL: Duration = Duration::from_secs(1);
/// Cap the negotiated max_write so every FUSE message stays well under the
/// enlarged datagram-socket buffer.
const MAX_WRITE: u32 = 128 * 1024;
/// Datagram socket buffer (both directions) — must exceed the largest single
/// FUSE message (≈ MAX_WRITE + headers, and read replies of the same order).
const SOCK_BUF: libc::c_int = 2 * 1024 * 1024;
/// Guest→host relay buffer.
const RELAY_BUF: usize = 2 * 1024 * 1024;
/// Requested readahead window (bytes); the kernel clamps to its own max. Larger
/// → more concurrent in-flight READ requests, so reads pipeline over the tunnel
/// instead of paying one round-trip's latency each. Measured ~3.4× faster than
/// serial direct_io on S3, and no worse on Notion (the workspace VFS resolves
/// render-on-read sizes, so page-cache reads are never clamped short).
const MAX_READAHEAD: u32 = 8 * 1024 * 1024;
/// How long a connection may take to present its token, in total rather than
/// per read. Every connection costs a thread, so a client that connects and then
/// stalls — saying nothing, or dribbling bytes to keep a per-read timer alive —
/// would otherwise hold one. Generous for the only legitimate client: the guest
/// pump writes its token as the first thing it does after connecting, over a
/// loopback socket.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Host-side raw-FUSE tunnel server. Bound to an ephemeral loopback port; the
/// guest pump connects, sends a one-line token, then streams raw FUSE bytes.
/// Aborts its accept loop on drop.
///
/// Loopback is enough because microsandbox runs a user-space network stack: the
/// guest dials `host.microsandbox.internal:<port>`, and the host side of that
/// connection is opened by the microsandbox process itself, so this server sees
/// a peer of `127.0.0.1`. Measured with a loopback-only listener and a guest
/// `nc` against the gateway address: the guest connects and the host observes
/// `127.0.0.1`. Binding the wildcard address instead would additionally expose
/// the whole workspace tree — every file this server can read — to any peer on
/// the same LAN that learns the token.
pub struct TunnelServer {
    addr: SocketAddr,
    token: String,
    shutdown: Arc<AtomicBool>,
}

impl Drop for TunnelServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl TunnelServer {
    pub fn spawn(fs: Arc<dyn ForwardFs>, rt: Handle) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let token = uuid::Uuid::new_v4().as_simple().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));

        let task_token = token.clone();
        let task_shutdown = shutdown.clone();
        thread::spawn(move || {
            loop {
                if task_shutdown.load(Ordering::Relaxed) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, peer)) => {
                        let fs = fs.clone();
                        let rt = rt.clone();
                        let token = task_token.clone();
                        thread::spawn(move || {
                            if let Err(e) = serve_conn(stream, fs, rt, token, peer) {
                                tracing::debug!("vfs tunnel: connection from {peer} ended: {e}");
                            }
                        });
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(e) => tracing::debug!("vfs tunnel: accept error: {e}"),
                }
            }
        });

        Ok(Self {
            addr,
            token,
            shutdown,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// Compare a presented token against the expected one without branching on
/// content, so the time a rejection takes carries no information about how many
/// leading bytes were right. Length is compared first and in the clear: the
/// token is a fixed-length UUID, so its length is not a secret.
fn token_matches(expected: &str, presented: &str) -> bool {
    let expected = expected.as_bytes();
    let presented = presented.as_bytes();
    expected.len() == presented.len() && bool::from(expected.ct_eq(presented))
}

/// Read the leading one-line token (bytes up to `\n`), one byte at a time so we
/// never consume any following raw FUSE frame bytes.
///
/// `deadline` bounds the whole handshake, not each read. `set_read_timeout` sets
/// `SO_RCVTIMEO`, which applies per `recv` call and so resets on every byte: a
/// client dribbling one byte just under the timeout holds its thread for the
/// line limit times the timeout rather than the timeout. Re-deriving the
/// remaining budget before each byte is what makes the bound the one intended.
fn read_token_line(stream: &mut TcpStream, deadline: Instant) -> io::Result<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // Also the guard for `set_read_timeout`, which rejects a zero duration.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "token handshake did not complete in time",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        stream.read_exact(&mut b)?;
        if b[0] == b'\n' {
            break;
        }
        out.push(b[0]);
        if out.len() > 128 {
            return Err(io::Error::other("token line too long"));
        }
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

/// Read one length-prefixed FUSE message off the TCP stream (the first u32 LE of
/// every FUSE header is the total message length).
fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr)?;
    let len = u32::from_le_bytes(hdr) as usize;
    if len < 4 {
        return Err(io::Error::other("frame len < 4"));
    }
    let mut msg = vec![0u8; len];
    msg[..4].copy_from_slice(&hdr);
    stream.read_exact(&mut msg[4..])?;
    Ok(msg)
}

fn set_sock_buf(fd: RawFd) {
    for opt in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        // SAFETY: standard setsockopt on a valid fd; ignore failure (best-effort).
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                &SOCK_BUF as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }
}

fn serve_conn(
    mut stream: TcpStream,
    fs: Arc<dyn ForwardFs>,
    rt: Handle,
    token: String,
    peer: SocketAddr,
) -> anyhow::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true).ok();

    // Warn rather than debug throughout the handshake: the only client that
    // should ever reach this port is the guest pump, which is handed the token
    // directly, so anything that fails here is something else dialing it.
    let presented = match read_token_line(&mut stream, Instant::now() + HANDSHAKE_TIMEOUT) {
        Ok(line) => line,
        Err(e) => {
            tracing::warn!("vfs tunnel: handshake from {peer} did not complete: {e}");
            return Err(e.into());
        }
    };
    if !token_matches(&token, &presented) {
        tracing::warn!("vfs tunnel: rejected connection from {peer}: bad token");
        anyhow::bail!("bad token");
    }
    // Clear it before the relay below. A mounted tunnel is request-driven and
    // legitimately idles between requests, and the clone taken for the read
    // direction shares this socket's options.
    stream.set_read_timeout(None)?;

    // socketpair: fuse_fd -> fuser, shim_fd -> our relay.
    let mut fds = [0i32; 2];
    // SAFETY: writes two fds into `fds`; checked below.
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, fds.as_mut_ptr()) };
    if rc != 0 {
        anyhow::bail!("socketpair failed (errno {})", io::Error::last_os_error());
    }
    let (fuse_fd, shim_fd) = (fds[0], fds[1]);
    set_sock_buf(fuse_fd);
    set_sock_buf(shim_fd);

    let mut tcp_rx = stream.try_clone()?;
    let mut tcp_tx = stream;

    // TCP (stream) -> shim (datagram): reframe each message (FUSE self-frames via
    // its header `len`), send as one datagram to the fuse end.
    let up = thread::spawn(move || {
        while let Ok(msg) = read_frame(&mut tcp_rx) {
            // SAFETY: sending `msg.len()` bytes from an owned buffer.
            let n =
                unsafe { libc::send(shim_fd, msg.as_ptr() as *const libc::c_void, msg.len(), 0) };
            if n != msg.len() as isize {
                break;
            }
        }
        // Closing the shim end makes fuser's fuse_fd see EOF → session ends.
        // SAFETY: closing our own datagram fd once, on this thread's exit.
        unsafe { libc::close(shim_fd) };
    });

    // shim (datagram) -> TCP: each recv is one full FUSE reply, already
    // self-framed by its out-header `len`; forward verbatim.
    let down_fd = shim_fd;
    let down = thread::spawn(move || {
        let mut buf = vec![0u8; RELAY_BUF];
        loop {
            // SAFETY: recv into an owned buffer of length RELAY_BUF.
            let n =
                unsafe { libc::recv(down_fd, buf.as_mut_ptr() as *mut libc::c_void, RELAY_BUF, 0) };
            if n <= 0 {
                break;
            }
            if tcp_tx.write_all(&buf[..n as usize]).is_err() {
                break;
            }
        }
    });

    // Hand the fuse end to fuser. from_fd blocks on the INIT handshake, which the
    // shim threads (started above) are already relaying. The forked fuser
    // advertises FUSE 7.40 and emits Linux wire structs even on this macOS host,
    // so the Linux guest kernel accepts the replies.
    // SAFETY: fuse_fd is a fresh fd owned solely by this Session now.
    let owned = unsafe { OwnedFd::from_raw_fd(fuse_fd) };
    let tunnel_fs = TunnelFs::new(fs, rt);
    let session = fuser::Session::from_fd(tunnel_fs, owned, SessionACL::All, tunnel_config())?;
    let bg = session.spawn()?;

    // Run until the guest disconnects (TCP closed → up thread exits).
    let _ = up.join();
    drop(bg); // ends the session loop
    let _ = down.join();
    Ok(())
}

fn tunnel_config() -> Config {
    // mount_options are irrelevant for from_fd (no mount); defaults are fine.
    Config::default()
}

// ---- FUSE filesystem backed by ForwardFs (read path) --------------------------

/// inode <-> path table, mirroring the guest forwarder's interning.
struct Inodes {
    to_path: Mutex<HashMap<u64, String>>,
    to_ino: Mutex<HashMap<String, u64>>,
    next: AtomicU64,
}
impl Inodes {
    fn new() -> Self {
        let mut to_path = HashMap::new();
        let mut to_ino = HashMap::new();
        to_path.insert(1, "/".to_string());
        to_ino.insert("/".to_string(), 1);
        Self {
            to_path: Mutex::new(to_path),
            to_ino: Mutex::new(to_ino),
            next: AtomicU64::new(2),
        }
    }
    fn path(&self, ino: u64) -> Option<String> {
        self.to_path.lock().unwrap().get(&ino).cloned()
    }
    fn intern(&self, path: &str) -> u64 {
        if let Some(&i) = self.to_ino.lock().unwrap().get(path) {
            return i;
        }
        let ino = self.next.fetch_add(1, Ordering::Relaxed);
        self.to_ino.lock().unwrap().insert(path.to_string(), ino);
        self.to_path.lock().unwrap().insert(ino, path.to_string());
        ino
    }
    fn child(&self, parent: &str, name: &str) -> String {
        if parent == "/" {
            format!("/{name}")
        } else {
            format!("{parent}/{name}")
        }
    }
}

struct TunnelFs {
    fs: Arc<dyn ForwardFs>,
    rt: Handle,
    inodes: Arc<Inodes>,
}
impl TunnelFs {
    fn new(fs: Arc<dyn ForwardFs>, rt: Handle) -> Self {
        Self {
            fs,
            rt,
            inodes: Arc::new(Inodes::new()),
        }
    }
}

fn secs_or(mtime: u64, secs: Option<u64>) -> SystemTime {
    let s = secs.filter(|v| *v > 0).unwrap_or(mtime);
    if s > 0 {
        UNIX_EPOCH + Duration::from_secs(s)
    } else {
        UNIX_EPOCH
    }
}
fn mk(
    ino: u64,
    kind: FileType,
    size: u64,
    mtime: u64,
    atime: Option<u64>,
    ctime: Option<u64>,
) -> FileAttr {
    FileAttr {
        ino: INodeNo(ino),
        size,
        blocks: 1,
        atime: secs_or(mtime, atime),
        mtime: secs_or(mtime, Some(mtime)),
        ctime: secs_or(mtime, ctime),
        crtime: UNIX_EPOCH,
        kind,
        perm: if kind == FileType::Directory {
            0o755
        } else {
            0o644
        },
        nlink: if kind == FileType::Directory { 2 } else { 1 },
        uid: 0,
        gid: 0,
        rdev: 0,
        blksize: 65536,
        flags: 0,
    }
}
fn dir_attr(ino: u64) -> FileAttr {
    mk(ino, FileType::Directory, 0, 0, None, None)
}
fn attr_from_stat(ino: u64, s: &FwdStat) -> FileAttr {
    if s.is_dir {
        dir_attr(ino)
    } else {
        mk(
            ino,
            FileType::RegularFile,
            s.size,
            s.mtime.unwrap_or(0),
            s.atime,
            s.ctime,
        )
    }
}

impl Filesystem for TunnelFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // Cap max_write so the guest kernel's /dev/fuse read-buffer minimum
        // (≈ max_write + headers) stays under the guest pump's 2 MiB buffer.
        // Without this it negotiates 16 MiB and the pump's reads fail EINVAL.
        let _ = config.set_max_write(MAX_WRITE);
        // Larger readahead → the kernel prefetches with more concurrent in-flight
        // READ requests (pipelined over the tunnel). Clamped by the kernel's max.
        let _ = config.set_max_readahead(MAX_READAHEAD);
        Ok(())
    }

    fn lookup(&self, _r: &Request, parent: INodeNo, name: &std::ffi::OsStr, reply: ReplyEntry) {
        let (fs, inodes) = (self.fs.clone(), self.inodes.clone());
        let Some(name) = name.to_str().map(str::to_string) else {
            return reply.error(Errno::EINVAL);
        };
        self.rt.spawn(async move {
            let Some(pp) = inodes.path(parent.0) else {
                return reply.error(Errno::ENOENT);
            };
            let path = inodes.child(&pp, &name);
            match fs.stat(&path).await {
                Ok(s) if s.exists => {
                    let ino = inodes.intern(&path);
                    reply.entry(&TTL, &attr_from_stat(ino, &s), Generation(0));
                }
                Ok(_) => reply.error(Errno::ENOENT),
                // Reachable-but-failing, the way `readdir` and `read` already report it. A
                // source whose credentials are missing must not look like an absent one.
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn getattr(&self, _r: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let (fs, inodes) = (self.fs.clone(), self.inodes.clone());
        self.rt.spawn(async move {
            let Some(path) = inodes.path(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            if path == "/" {
                return reply.attr(&TTL, &dir_attr(1));
            }
            match fs.stat(&path).await {
                Ok(s) if s.exists => reply.attr(&TTL, &attr_from_stat(ino.0, &s)),
                Ok(_) => reply.error(Errno::ENOENT),
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn open(&self, _r: &Request, _ino: INodeNo, _flags: fuser::OpenFlags, reply: ReplyOpen) {
        // Page cache (no FOPEN_DIRECT_IO): the kernel prefetches with concurrent
        // in-flight reads (pipelined over the tunnel) — measured ~3.4× faster on
        // S3 than serial direct_io reads. Safe because getattr returns accurate
        // sizes: the workspace VFS even resolves render-on-read sizes (Notion),
        // so cached reads are never clamped short.
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    #[allow(clippy::too_many_arguments)]
    fn read(
        &self,
        _r: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: fuser::OpenFlags,
        _lock: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        let (fs, inodes) = (self.fs.clone(), self.inodes.clone());
        self.rt.spawn(async move {
            let Some(path) = inodes.path(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            match fs.read(&path, Some(offset), Some(size as u64)).await {
                Ok(data) => reply.data(&data),
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn readdir(
        &self,
        _r: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let (fs, inodes) = (self.fs.clone(), self.inodes.clone());
        self.rt.spawn(async move {
            let Some(path) = inodes.path(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            let entries = match fs.readdir(&path).await {
                Ok(e) => e,
                Err(_) => return reply.error(Errno::EIO),
            };
            let mut listing: Vec<(u64, FileType, String)> = vec![
                (ino.0, FileType::Directory, ".".into()),
                (1, FileType::Directory, "..".into()),
            ];
            for e in entries {
                let cp = inodes.child(&path, &e.name);
                let cino = inodes.intern(&cp);
                let kind = if e.is_dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                listing.push((cino, kind, e.name));
            }
            for (i, (e_ino, kind, name)) in listing.iter().enumerate().skip(offset as usize) {
                if reply.add(INodeNo(*e_ino), (i + 1) as u64, *kind, name) {
                    break;
                }
            }
            reply.ok();
        });
    }

    fn flush(
        &self,
        _r: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock: fuser::LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &self,
        _r: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _flags: fuser::OpenFlags,
        _lock: Option<fuser::LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }
}

#[cfg(test)]
mod host_engine_test {
    //! In-process proof (no VM, no S3, milliseconds) that the host tunnel engine
    //! — TCP + dgram shim + framing + async reply bridge over a ForwardFs — is
    //! correct. A TCP client plays the guest pump: it sends the token line, then
    //! raw FUSE INIT / GETATTR / LOOKUP / READ frames and checks the replies.
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use async_trait::async_trait;

    use super::{HANDSHAKE_TIMEOUT, TunnelServer, token_matches};
    use crate::vfs::sandbox::{ForwardFs, FwdEntry, FwdStat};

    struct FakeFs;
    #[async_trait]
    impl ForwardFs for FakeFs {
        async fn readdir(&self, _p: &str) -> anyhow::Result<Vec<FwdEntry>> {
            Ok(vec![FwdEntry {
                name: "x".into(),
                is_dir: false,
                size: 5,
                mtime: None,
            }])
        }
        async fn stat(&self, p: &str) -> anyhow::Result<FwdStat> {
            Ok(match p {
                "/" => FwdStat {
                    exists: true,
                    is_dir: true,
                    size: 0,
                    mtime: None,
                    atime: None,
                    ctime: None,
                },
                "/x" => FwdStat {
                    exists: true,
                    is_dir: false,
                    size: 5,
                    mtime: None,
                    atime: None,
                    ctime: None,
                },
                // A source that could not be reached, as distinct from one that is not
                // there. `lookup`/`getattr` must not report these the same way.
                "/broken" => return Err(anyhow::anyhow!("provider unreachable")),
                _ => FwdStat::missing(),
            })
        }
        async fn read(
            &self,
            _p: &str,
            _o: Option<u64>,
            _s: Option<u64>,
        ) -> anyhow::Result<Vec<u8>> {
            Ok(b"hello".to_vec())
        }
    }

    fn req(opcode: u32, unique: u64, nodeid: u64, body: &[u8]) -> Vec<u8> {
        let len = (40 + body.len()) as u32;
        let mut v = Vec::with_capacity(len as usize);
        v.extend_from_slice(&len.to_le_bytes());
        v.extend_from_slice(&opcode.to_le_bytes());
        v.extend_from_slice(&unique.to_le_bytes());
        v.extend_from_slice(&nodeid.to_le_bytes());
        v.extend_from_slice(&[0u8; 16]); // uid,gid,pid,padding
        v.extend_from_slice(body);
        v
    }
    fn init_body() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&7u32.to_le_bytes());
        b.extend_from_slice(&31u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b
    }
    fn getattr_body() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0u64.to_le_bytes()); // flags,dummy
        b.extend_from_slice(&0u64.to_le_bytes()); // fh
        b
    }
    /// read one reply off the TCP stream; returns (error, payload_len).
    fn read_reply(c: &mut TcpStream) -> (i32, usize) {
        let mut hdr = [0u8; 4];
        c.read_exact(&mut hdr).expect("reply len");
        let len = u32::from_le_bytes(hdr) as usize;
        let mut rest = vec![0u8; len - 4];
        c.read_exact(&mut rest).expect("reply body");
        let err = i32::from_le_bytes(rest[0..4].try_into().unwrap());
        (err, len)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_engine_serves_over_tcp() {
        let fs: Arc<dyn ForwardFs> = Arc::new(FakeFs);
        let srv = TunnelServer::spawn(fs, tokio::runtime::Handle::current()).expect("spawn server");

        let mut c = TcpStream::connect(("127.0.0.1", srv.port())).expect("connect");
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(format!("{}\n", srv.token()).as_bytes())
            .unwrap();

        // INIT (opcode 26)
        c.write_all(&req(26, 1, 0, &init_body())).unwrap();
        let (err, _) = read_reply(&mut c);
        assert_eq!(err, 0, "INIT should succeed");

        // GETATTR (opcode 3) on root inode 1
        c.write_all(&req(3, 2, 1, &getattr_body())).unwrap();
        let (err, len) = read_reply(&mut c);
        assert_eq!(err, 0, "GETATTR root should succeed");
        assert!(len > 16, "GETATTR should carry an attr payload");

        // LOOKUP (opcode 1) of "x" under root -> stat "/x"
        c.write_all(&req(1, 3, 1, b"x\0")).unwrap();
        let (err, _) = read_reply(&mut c);
        assert_eq!(err, 0, "LOOKUP /x should succeed");

        // A path that is not there, and one the source could not answer for. These have to
        // arrive as different errnos: a mount whose credentials are missing fails every
        // stat, and reporting that as ENOENT is what let the guest read an unreachable
        // source as an empty one. Asserted here rather than only against `ForwardFs`,
        // because the mapping lives in these handlers.
        c.write_all(&req(1, 4, 1, b"nope\0")).unwrap();
        let (err, _) = read_reply(&mut c);
        assert_eq!(-err, libc::ENOENT, "LOOKUP of an absent name is ENOENT");

        c.write_all(&req(1, 5, 1, b"broken\0")).unwrap();
        let (err, _) = read_reply(&mut c);
        assert_eq!(
            -err,
            libc::EIO,
            "LOOKUP of an unreachable source is EIO, not ENOENT"
        );

        println!("host engine served INIT + GETATTR + LOOKUP over TCP tunnel — OK");
    }

    /// The listener must stay on loopback. Everything this server can read — the
    /// whole workspace tree, provider mounts included — is behind one token, so a
    /// wildcard bind would put it in reach of every peer on the LAN. The guest
    /// still reaches it because microsandbox opens the host side of the
    /// connection itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn listener_is_bound_to_loopback() {
        let fs: Arc<dyn ForwardFs> = Arc::new(FakeFs);
        let srv = TunnelServer::spawn(fs, tokio::runtime::Handle::current()).expect("spawn server");
        assert!(
            srv.addr.ip().is_loopback(),
            "tunnel listener must not be reachable off-host, bound to {}",
            srv.addr
        );
    }

    /// A wrong token is refused before any FUSE traffic is served. Paired with
    /// the successful handshake above, this is what makes the token the gate
    /// rather than decoration.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wrong_token_is_refused() {
        let fs: Arc<dyn ForwardFs> = Arc::new(FakeFs);
        let srv = TunnelServer::spawn(fs, tokio::runtime::Handle::current()).expect("spawn server");

        let mut c = TcpStream::connect(("127.0.0.1", srv.port())).expect("connect");
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Same length as the real token so the length check is not what refuses it.
        let wrong: String = srv.token().chars().rev().collect();
        assert_ne!(wrong, srv.token(), "reversed token must differ");
        c.write_all(format!("{wrong}\n").as_bytes()).unwrap();
        c.write_all(&req(26, 1, 0, &init_body())).unwrap();

        // The server drops the connection instead of answering INIT, so the read
        // ends without a reply.
        let mut hdr = [0u8; 4];
        assert!(
            c.read_exact(&mut hdr).is_err(),
            "a bad token must not get a FUSE reply"
        );
    }

    /// A client that connects and then says nothing must be let go rather than
    /// holding its thread. Observed from the client side: the server drops the
    /// connection, so the read returns EOF. Without the deadline the read would
    /// instead block until the client\'s own timeout, which is what fails here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_silent_client_is_let_go() {
        let fs: Arc<dyn ForwardFs> = Arc::new(FakeFs);
        let srv = TunnelServer::spawn(fs, tokio::runtime::Handle::current()).expect("spawn server");

        let mut c = TcpStream::connect(("127.0.0.1", srv.port())).expect("connect");
        // Comfortably past the server\'s own deadline, so a client-side timeout
        // here means the server never imposed one.
        c.set_read_timeout(Some(HANDSHAKE_TIMEOUT * 4)).unwrap();

        let mut b = [0u8; 1];
        let n = c.read(&mut b);
        assert!(
            matches!(n, Ok(0)),
            "the server must close a connection that never presents a token, got {n:?}"
        );
    }

    /// A client that keeps sending, but never finishes the line, must also be
    /// let go — and on the same budget as one that says nothing. This is the
    /// case a per-read timeout misses: `SO_RCVTIMEO` restarts on every byte, so
    /// dribbling under it once bought the line limit times the timeout.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dribbling_client_is_let_go_on_the_same_budget() {
        let fs: Arc<dyn ForwardFs> = Arc::new(FakeFs);
        let srv = TunnelServer::spawn(fs, tokio::runtime::Handle::current()).expect("spawn server");

        let mut c = TcpStream::connect(("127.0.0.1", srv.port())).expect("connect");
        c.set_read_timeout(Some(HANDSHAKE_TIMEOUT * 4)).unwrap();

        // A byte every fifth of the budget, never a newline. Under a per-read
        // timeout every one of these would refresh the timer.
        let started = Instant::now();
        let writer = thread::spawn(move || {
            for _ in 0..40 {
                if c.write_all(b"a").is_err() {
                    break;
                }
                thread::sleep(HANDSHAKE_TIMEOUT / 5);
            }
            let mut b = [0u8; 1];
            let _ = c.read(&mut b);
        });
        writer.join().expect("writer thread");

        let held = started.elapsed();
        assert!(
            held < HANDSHAKE_TIMEOUT * 2,
            "the deadline must bound the whole handshake, not each read: held {held:?} against a \
             {HANDSHAKE_TIMEOUT:?} budget"
        );
    }

    #[test]
    fn token_matches_only_on_exact_equality() {
        assert!(token_matches("abc123", "abc123"));
        assert!(!token_matches("abc123", "abc124"));
        assert!(!token_matches("abc123", "abc12"));
        assert!(!token_matches("abc123", "abc1234"));
        assert!(!token_matches("abc123", ""));
    }
}
