use std::io::SeekFrom;
use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use dav_server::{
    DavHandler,
    davpath::DavPath,
    fakels::FakeLs,
    fs::{
        DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsFuture, FsStream, OpenOptions,
        ReadDirMeta,
    },
    localfs::LocalFs,
};
use uuid::Uuid;

use crate::state::AppState;

/// WebDAV workspace router. Mounted by [`super::get_router`] at
/// `/projects/{pid}/workspace[/…]`; exposes `data_root/{pid}/workspace` as a
/// per-project filesystem.
///
/// Routes via `fallback` so axum forwards every HTTP method — including
/// WebDAV-specific ones (`PROPFIND`, `MKCOL`, `COPY`, `MOVE`, `LOCK`, …) —
/// straight to [`dav_server`]. Auth mirrors the WS route: JWT is read from
/// `?token=…` because the eventual target audience (browser fetch + native
/// WebDAV clients) cannot reliably set custom auth headers.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().fallback(handle).with_state(state)
}

async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response<Body> {
    let pid = match parse_pid(req.uri().path()) {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "invalid project id").into_response(),
    };

    let token = req.uri().query().and_then(extract_token);
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    };
    if state.jwt.decode(&token).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }

    match state.projects.get(pid).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "project not found").into_response(),
        Err(e) => {
            tracing::error!("workspace project lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    }

    // Wrap LocalFs so successful mutations get reported back to AppState for
    // side-processing (see [`Filesystem`]).
    let dav = DavHandler::builder()
        .filesystem(Box::new(Filesystem::new(state.clone(), pid)))
        .locksystem(FakeLs::new())
        .strip_prefix(format!("/projects/{pid}/workspace"))
        .build_handler();

    dav.handle(req).await.map(Body::new)
}

fn parse_pid(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/projects/")?;
    let (pid_str, _) = rest.split_once('/')?;
    Uuid::parse_str(pid_str).ok()
}

fn extract_token(query: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.into_owned())
}

/// A [`DavFileSystem`] over a project's workspace. Wraps [`LocalFs`] and reports
/// every successful mutation — `PUT` (the file's `flush`), `MOVE`/`COPY`
/// (`rename`/`copy`), and `DELETE` (`remove_file`) — back to [`AppState`] for
/// side-processing, rather than guessing from the HTTP method/status after the
/// fact. Each report is classified as insert/update/remove from prior path
/// existence and dispatched to the matching `state.workspace` handler.
///
/// Only paths under `knowledge/` (see [`is_knowledge`]) are reported today;
/// further handlers (other paths) are added at the callsites.
///
/// Cloneable and `Send + Sync` so it satisfies dav-server's
/// `GuardedFileSystem<()>` blanket impl; clones share the inner `Arc`-backed
/// `LocalFs` and the `AppState` handle.
#[derive(Clone)]
struct Filesystem {
    inner: Box<LocalFs>,
    state: Arc<AppState>,
    pid: Uuid,
}

impl Filesystem {
    pub fn new(state: Arc<AppState>, pid: Uuid) -> Self {
        let root = state.workspace.root(pid);
        Self {
            inner: LocalFs::new(root, false, false, false),
            state,
            pid,
        }
    }
}

/// True when `rel_path` lives under the `knowledge/` directory. Component-wise
/// match on the leading dir: `knowledge/x` hits, `knowledgebase/x` does not.
fn is_knowledge(rel_path: &str) -> bool {
    rel_path.trim_start_matches('/').starts_with("knowledge/")
}

/// Workspace-relative path (leading `/`) in the shape the `state.workspace`
/// knowledge handlers expect.
/// `as_rel_ospath` already drops the leading slash, so we re-add one.
fn rel_path_string(path: &DavPath) -> String {
    format!("/{}", path.as_rel_ospath().to_string_lossy())
}

impl DavFileSystem for Filesystem {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(async move {
            let is_write = options.write || options.append || options.create || options.create_new;
            // Probe before opening: a create/truncating open would make the
            // file exist (or empty) regardless, so we must capture prior
            // existence here to tell an insert from an overwrite at flush time.
            let existed = is_write && self.inner.metadata(path).await.is_ok();
            let file = self.inner.open(path, options).await?;
            if is_write {
                Ok(Box::new(ObservedFile {
                    inner: file,
                    state: self.state.clone(),
                    pid: self.pid,
                    rel_path: rel_path_string(path),
                    existed,
                    wrote: false,
                }) as Box<dyn DavFile>)
            } else {
                Ok(file)
            }
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        self.inner.read_dir(path, meta)
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        self.inner.metadata(path)
    }

    fn symlink_metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        self.inner.symlink_metadata(path)
    }

    fn create_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        self.inner.create_dir(path)
    }

    fn remove_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        self.inner.remove_dir(path)
    }

    fn remove_file<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.inner.remove_file(path).await?;
            let rel = rel_path_string(path);
            if is_knowledge(&rel) {
                self.state.workspace.remove_knowledge(self.pid, &rel);
            }
            Ok(())
        })
    }

    fn rename<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            // Probe the destination before the move so we can tell whether it
            // landed on a fresh path (insert) or replaced one (update).
            let to_existed = self.inner.metadata(to).await.is_ok();
            self.inner.rename(from, to).await?;
            // The source path left `knowledge/`; the destination arrived in it.
            let from_rel = rel_path_string(from);
            if is_knowledge(&from_rel) {
                self.state.workspace.remove_knowledge(self.pid, &from_rel);
            }
            let to_rel = rel_path_string(to);
            if is_knowledge(&to_rel) {
                if to_existed {
                    self.state.workspace.update_knowledge(self.pid, &to_rel);
                } else {
                    self.state.workspace.insert_knowledge(self.pid, &to_rel);
                }
            }
            Ok(())
        })
    }

    fn copy<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            let to_existed = self.inner.metadata(to).await.is_ok();
            self.inner.copy(from, to).await?;
            let to_rel = rel_path_string(to);
            if is_knowledge(&to_rel) {
                if to_existed {
                    self.state.workspace.update_knowledge(self.pid, &to_rel);
                } else {
                    self.state.workspace.insert_knowledge(self.pid, &to_rel);
                }
            }
            Ok(())
        })
    }
}

/// A [`DavFile`] that delegates to the underlying file and, once a write
/// completes (`flush` after at least one `write_*`), reports it to
/// `state.workspace` as an insert or update depending on prior existence.
struct ObservedFile {
    inner: Box<dyn DavFile>,
    state: Arc<AppState>,
    pid: Uuid,
    rel_path: String,
    /// Whether the file already existed when it was opened — distinguishes an
    /// insert from an overwrite (update) at flush time.
    existed: bool,
    wrote: bool,
}

impl std::fmt::Debug for ObservedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedFile")
            .field("rel_path", &self.rel_path)
            .field("wrote", &self.wrote)
            .finish_non_exhaustive()
    }
}

impl DavFile for ObservedFile {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        self.inner.metadata()
    }

    fn write_buf(&mut self, buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        self.wrote = true;
        self.inner.write_buf(buf)
    }

    fn write_bytes(&mut self, buf: bytes::Bytes) -> FsFuture<'_, ()> {
        self.wrote = true;
        self.inner.write_bytes(buf)
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, bytes::Bytes> {
        self.inner.read_bytes(count)
    }

    fn seek(&mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        self.inner.seek(pos)
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        Box::pin(async move {
            self.inner.flush().await?;
            if self.wrote {
                // Clear first so a second flush on the same handle won't re-report.
                self.wrote = false;
                if is_knowledge(&self.rel_path) {
                    if self.existed {
                        self.state
                            .workspace
                            .update_knowledge(self.pid, &self.rel_path);
                    } else {
                        self.state
                            .workspace
                            .insert_knowledge(self.pid, &self.rel_path);
                    }
                }
            }
            Ok(())
        })
    }

    fn redirect_url(&mut self) -> FsFuture<'_, Option<String>> {
        self.inner.redirect_url()
    }
}
