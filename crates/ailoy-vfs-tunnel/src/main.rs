// In-guest raw-FUSE tunnel pump.
//
// This binary does NO protocol conversion. It mounts /dev/fuse and then relays
// the raw FUSE wire protocol, byte for byte, to the host over a single
// persistent TCP connection. The host runs the actual FUSE engine
// (fuser::Session::from_fd) against the workspace filesystem. See
// backend/src/sandbox_tunnel.rs.
//
// It is self-contained: given a mountpoint (arg) and VFS_HOST/VFS_TOKEN (env) it
// creates the mountpoint, clears any stale mount, connects to the host, mounts
// /dev/fuse, then DAEMONIZES — the foreground process exits 0 only once the
// mount is up, so the launcher's single `exec` returns exactly when the
// workspace is mounted (non-zero + a diagnostic on failure). No init script,
// no readiness polling. The backgrounded child relays until the VM stops.
//
// Framing is free: every FUSE message begins with a u32 LE `len` (fuse_in_header
// / fuse_out_header) covering the whole message, so a reader on the stream side
// just reads 4 bytes, then `len - 4` more. A reader on a message-boundary fd
// (/dev/fuse) gets one whole message per read().
//
// After daemonizing, stdout/stderr are redirected to /tmp/ailoy-vfs-tunnel.log;
// prints before that go to the launcher's captured output (setup diagnostics).
//
// Cross-compiled to <arch>-unknown-linux-musl; runs as root in the guest.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::fd::RawFd;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const FUSE_DEV: &str = "/dev/fuse\0";
const LOG_PATH: &str = "/tmp/ailoy-vfs-tunnel.log\0";
// Bounded by the host's negotiated max_write (128 KiB) + headers; 2 MiB is ample.
const BUF: usize = 2 * 1024 * 1024;

/// Log + flush. stdout is redirected to a file (block-buffered), and the pump
/// can wedge without exiting, so flush every line or the log looks empty.
macro_rules! logln {
    ($($a:tt)*) => {{
        println!($($a)*);
        let _ = std::io::stdout().flush();
    }};
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn cstr(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

/// Resolve `host:port` with a watchdog, since `to_socket_addrs` on the flaky
/// guest→host resolver can hang indefinitely (see the HTTP forwarder's long
/// note on the same hazard). Returns the first address or None on timeout.
fn resolve(hostport: &str) -> Option<SocketAddr> {
    for attempt in 0..10 {
        let target = hostport.to_string();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(target.to_socket_addrs().ok().and_then(|mut it| it.next()));
        });
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Some(a)) => return Some(a),
            _ => logln!("resolve {hostport} attempt {attempt} failed/timed out"),
        }
        thread::sleep(Duration::from_millis(300));
    }
    None
}

/// Connect to the host tunnel server, bounded at every step.
fn connect_host(hostport: &str) -> TcpStream {
    let addr = resolve(hostport).unwrap_or_else(|| panic!("resolve {hostport} timed out"));
    logln!("resolved {hostport} -> {addr}, connecting");
    let mut last = None;
    for attempt in 0..10 {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(s) => {
                logln!("connected to {addr} (attempt {attempt})");
                return s;
            }
            Err(e) => {
                logln!("connect {addr} attempt {attempt} failed: {e}");
                last = Some(e);
                thread::sleep(Duration::from_millis(300));
            }
        }
    }
    panic!("connect {addr} failed: {last:?}");
}

/// `mkdir -p` for the mountpoint (mount(2) needs it to exist).
fn mkdir_p(path: &str) {
    let mut cur = String::new();
    for comp in path.split('/').filter(|c| !c.is_empty()) {
        cur.push('/');
        cur.push_str(comp);
        // SAFETY: NUL-terminated path; EEXIST and other errors are ignored (the
        // subsequent mount(2) is the real check).
        unsafe { libc::mkdir(cstr(&cur).as_ptr(), 0o755) };
    }
}

/// Create the mountpoint, detach any stale mount, then mount /dev/fuse there and
/// return the device fd. The kernel delegates all FUSE ops for that mountpoint
/// to this fd — which we relay verbatim. No libfuse/fusermount: a direct
/// mount(2) as root.
fn mount_fuse(mountpoint: &str) -> RawFd {
    mkdir_p(mountpoint);
    // Detach a stale mount left by a dead pump (mountpoint stuck in "Transport
    // endpoint is not connected"). Best-effort; ignore errors.
    // SAFETY: NUL-terminated path.
    unsafe { libc::umount2(cstr(mountpoint).as_ptr(), libc::MNT_DETACH) };

    // SAFETY: standard libc FFI; strings are NUL-terminated below.
    let fd = unsafe { libc::open(FUSE_DEV.as_ptr() as *const libc::c_char, libc::O_RDWR) };
    if fd < 0 {
        panic!("open /dev/fuse failed (errno {})", errno());
    }
    let target = cstr(mountpoint);
    let fstype = cstr("fuse");
    let source = cstr("ailoyvfs");
    // rootmode is the octal st_mode of the root inode: S_IFDIR = 0o040000.
    let data = cstr(&format!("fd={fd},rootmode=40000,user_id=0,group_id=0"));
    let flags = (libc::MS_NOSUID | libc::MS_NODEV) as libc::c_ulong;
    // SAFETY: all pointers are valid NUL-terminated C strings for the call.
    let rc = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            flags,
            data.as_ptr() as *const libc::c_void,
        )
    };
    if rc != 0 {
        panic!("mount(2) fuse at {mountpoint} failed (errno {})", errno());
    }
    logln!("mounted fuse at {mountpoint} (fd={fd})");
    fd
}

/// Fork into the background. Must be called while single-threaded (fork-safe).
///
/// In the PARENT this never returns: it waits for the child to report readiness
/// on a pipe, then `_exit`s — 0 if the child signalled the mount is up, non-zero
/// if the child died during setup (its diagnostics already went to the launcher's
/// captured output). So the launching `exec` returns exactly when mounted.
///
/// In the CHILD it returns a "ready fd"; call [`detach_and_signal_ready`] once
/// the mount is up. The child keeps running (the relay loop).
fn daemonize() -> RawFd {
    let mut fds = [0i32; 2];
    // SAFETY: pipe writes two fds into `fds`.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        panic!("pipe failed (errno {})", errno());
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    // SAFETY: fork; both branches handled.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        panic!("fork failed (errno {})", errno());
    }
    if pid > 0 {
        // Parent: block until the child reports readiness, then exit.
        // SAFETY: close our unused write end, read one byte, _exit.
        unsafe {
            libc::close(write_fd);
            let mut b = [0u8; 1];
            let n = libc::read(read_fd, b.as_mut_ptr() as *mut libc::c_void, 1);
            let code = if n == 1 && b[0] == b'R' { 0 } else { 1 };
            // _exit: the child shares our address space's file buffers via fork;
            // don't run atexit/flush in the parent.
            libc::_exit(code);
        }
    }
    // Child: new session so it outlives the launching shell.
    // SAFETY: close the read end we don't use; detach from the controlling tty.
    unsafe {
        libc::close(read_fd);
        libc::setsid();
    }
    write_fd
}

/// Redirect stdio to the log (releasing the launcher's captured pipes), then
/// tell the parent the mount is up so its `exec` returns 0.
fn detach_and_signal_ready(ready_fd: RawFd) {
    // SAFETY: open the log and dup2 it over stdout/stderr, stdin from /dev/null;
    // then write the ready byte and close the pipe. Best-effort on the redirects.
    unsafe {
        let logfd = libc::open(
            LOG_PATH.as_ptr() as *const libc::c_char,
            libc::O_CREAT | libc::O_WRONLY | libc::O_APPEND,
            0o644,
        );
        if logfd >= 0 {
            let devnull = libc::open(b"/dev/null\0".as_ptr() as *const libc::c_char, libc::O_RDONLY);
            if devnull >= 0 {
                libc::dup2(devnull, 0);
                libc::close(devnull);
            }
            libc::dup2(logfd, 1);
            libc::dup2(logfd, 2);
            if logfd > 2 {
                libc::close(logfd);
            }
        }
        let _ = libc::write(ready_fd, b"R".as_ptr() as *const libc::c_void, 1);
        libc::close(ready_fd);
    }
}

fn main() {
    let mountpoint = std::env::args()
        .nth(1)
        .expect("usage: ailoy-vfs-tunnel <mountpoint>");
    let host = std::env::var("VFS_HOST").unwrap_or_default();
    let host = host
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    let token = std::env::var("VFS_TOKEN").unwrap_or_default();

    // Daemonize up front while still single-threaded (fork-safe). The parent
    // blocks until we signal readiness below and then exits, so the launcher's
    // `exec` returns exactly when the mount is up. Only the child continues here.
    let ready_fd = daemonize();

    logln!("tunnel pump starting: mountpoint={mountpoint} host={host}");
    // Connect + authenticate FIRST, so a hung/failed resolver never leaves a
    // zombie mount behind. (A panic here kills the child; the parent then exits
    // non-zero and the diagnostic is in the launcher's captured output.)
    let mut to_host = connect_host(&host);
    to_host.set_nodelay(true).ok();
    to_host
        .write_all(format!("{token}\n").as_bytes())
        .expect("send token");
    let mut from_host = to_host.try_clone().expect("clone tcp");
    logln!("token sent; mounting");

    let fuse_fd = mount_fuse(&mountpoint);

    // Mount is up and the host connection is live: tell the parent (its `exec`
    // returns 0) and redirect our output to the log for the relay loop.
    detach_and_signal_ready(ready_fd);
    logln!("mounted; relaying");

    // kernel -> host: each read() from /dev/fuse yields exactly one request,
    // already self-framed by its header `len`. Forward the bytes as-is.
    let up = thread::spawn(move || {
        let mut buf = vec![0u8; BUF];
        let mut n_msgs = 0u64;
        loop {
            // SAFETY: reading into an owned buffer of length BUF.
            let n = unsafe { libc::read(fuse_fd, buf.as_mut_ptr() as *mut libc::c_void, BUF) };
            if n <= 0 {
                logln!("up: /dev/fuse read ended n={n} errno={} after {n_msgs} msgs", errno());
                break;
            }
            if to_host.write_all(&buf[..n as usize]).is_err() {
                logln!("up: tcp write failed after {n_msgs} msgs");
                break;
            }
            n_msgs += 1;
        }
    });

    // host -> kernel: reassemble one message per FUSE `len` header, then write it
    // to /dev/fuse in a single write() (one reply per write).
    let mut hdr = [0u8; 4];
    let mut n_msgs = 0u64;
    loop {
        if let Err(e) = from_host.read_exact(&mut hdr) {
            logln!("down: tcp read ended after {n_msgs} msgs: {e}");
            break;
        }
        let len = u32::from_le_bytes(hdr) as usize;
        if len < 4 {
            logln!("down: bad frame len {len}");
            break;
        }
        let mut msg = vec![0u8; len];
        msg[..4].copy_from_slice(&hdr);
        if let Err(e) = from_host.read_exact(&mut msg[4..]) {
            logln!("down: tcp read body ended after {n_msgs} msgs: {e}");
            break;
        }
        // SAFETY: writing `len` bytes from an owned buffer to the fuse device fd.
        let w = unsafe { libc::write(fuse_fd, msg.as_ptr() as *const libc::c_void, len) };
        if w != len as isize {
            logln!("down: fuse write short w={w} want={len} errno={}", errno());
            break;
        }
        n_msgs += 1;
    }

    // The up-thread may be parked in a blocking /dev/fuse read; don't join (it
    // could deadlock). Log and exit so stdout is flushed and the fd is closed.
    logln!("tunnel pump exiting");
    let _ = up; // detach
    std::process::exit(0);
}
