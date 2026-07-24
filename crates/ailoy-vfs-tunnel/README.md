# ailoy-vfs-tunnel

A **static, dependency-free** in-guest raw-FUSE tunnel pump for the workspace VFS
sandbox path. It mounts `/dev/fuse` in the sandbox guest and relays the **raw
FUSE wire protocol**, byte for byte, to the host over one persistent TCP
connection. The host runs the actual FUSE engine (`fuser::Session::from_fd`)
against the unified workspace tree and serves provider files (S3 / Notion) to the
guest shell. See `backend/src/sandbox_tunnel.rs` for the host side.

## Why a raw tunnel (not an in-guest FUSE server)

The predecessor ran a full FUSE server *in the guest* that translated every op
into an HTTP call to a host forward server. That did the protocol conversion
twice (FUSE↔HTTP on both ends) and opened a new TCP connection per op. Measured
against this tunnel on the same S3 file, it was slower and unreliable (short /
failed reads); the tunnel reads reliably at ~110 MB/s (direct_io) / ~340 MB/s
(readahead).

This pump does **zero** protocol conversion: it only pumps `/dev/fuse` bytes
between the kernel and a socket. All FUSE semantics live on the host. It is
statically linked (musl) and mounts via the kernel's `/dev/fuse` directly as root
— no `fusermount3`, no `libfuse`, no `fuser`, no Python. FUSE is built into the
guest kernel, so the binary works on **any** guest image with **zero** setup.

Framing is free: every FUSE message begins with a u32-LE `len` (fuse_in/out
header) covering the whole message, so the stream side reframes by reading 4
bytes then `len - 4` more, and the `/dev/fuse` side gets one whole message per
`read()`.

## Runtime contract

```
ailoy-vfs-tunnel <mountpoint>
  env VFS_HOST=http://host.microsandbox.internal:<port>   # host tunnel server
  env VFS_TOKEN=<token>                                    # one-line auth handshake
```
Must run as **root** (the guest is). No other guest dependencies.

## Self-daemonization (one `exec`, no init script)

The pump is self-contained: it needs no setup script and no readiness polling.
Given the mountpoint and env above it, in order:

1. **daemonizes first** — `fork()` while still single-threaded (fork-safe: the
   relay threads are spawned only afterwards, in the child);
2. in the child: connects to the host + sends the token, `mkdir -p`s the
   mountpoint, detaches any stale mount (`umount2(MNT_DETACH)`), then mounts
   `/dev/fuse`;
3. **signals readiness** to the parent over a pipe once the mount is up; the
   parent then `_exit`s.

So the **foreground process returns exactly when the workspace is mounted** —
exit `0` on success, non-zero on setup failure (with the diagnostic already on
the inherited stdout/stderr, which the launcher captures). The launcher therefore
needs just a single `exec`:

```sh
chmod +x /opt/ailoy/vfs-tunnel && \
  VFS_HOST=… VFS_TOKEN=… /opt/ailoy/vfs-tunnel /mnt/workspace
```

no `setsid`, no `/proc/mounts` poll loop, no separate mountpoint/`umount`
prep. After signalling ready, the backgrounded child `setsid`s, redirects
stdin from `/dev/null` and stdout/stderr to `/tmp/ailoy-vfs-tunnel.log`
(the guest-side diagnostic log), and relays until the VM stops.

## Building (static musl, any host — no external toolchain)

The sandbox guest arch matches the host: aarch64 on Apple Silicon, x86_64 on
Intel/AMD Linux. The only requirement is the musl target; cross-linking from a
non-Linux host uses Rust's **bundled lld** (`-C linker-flavor=ld.lld`) — no zig,
no external linker.

```sh
rustup target add aarch64-unknown-linux-musl   # or x86_64-unknown-linux-musl
RUSTFLAGS="-C linker-flavor=ld.lld" \
  cargo build --release --target aarch64-unknown-linux-musl
# -> target/aarch64-unknown-linux-musl/release/ailoy-vfs-tunnel  (static ELF)
```

## Integration (built from source, no committed binary)

`backend/build.rs` compiles this crate for the target arch (the guest arch under
libkrun) with the recipe above and writes the ELF to `OUT_DIR`;
`backend/src/sandbox_tunnel.rs` embeds it via
`include_bytes!(concat!(env!("OUT_DIR"), "/ailoy-vfs-tunnel"))`, writes it into
the guest, and launches it with the single `exec` shown above — the pump
self-daemonizes, so that `exec` returns once the mount is up.

There is **no committed binary** and **no release step**: changing `src/main.rs`
here is picked up automatically on the next `cargo build` of the backend
(build.rs has `rerun-if-changed` on the pump sources). If the musl target is
missing, build.rs writes an empty stub and the in-VM mount errors clearly at
runtime.
