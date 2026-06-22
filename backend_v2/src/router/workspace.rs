use std::future::ready;
use std::io::SeekFrom;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
        OpenOptions, ReadDirMeta,
    },
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::state::{
    AppState, DirEntry as WsDirEntry, File as WsFile, FsError as WsFsError,
    OpenOptions as WsOpenOptions, ReadDirMeta as WsReadDirMeta, WorkspaceFs,
};

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

    // The filesystem (and its side-processing) lives in `state.workspace`; here
    // we only wrap it in the WebDAV protocol (see [`DavFs`]).
    let dav = DavHandler::builder()
        .filesystem(Box::new(DavFs(state.workspace.get_fs(pid))))
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

/// Workspace-relative path (leading `/`) in the shape [`WorkspaceFs`] expects.
/// `as_rel_ospath` already drops the leading slash, so we re-add one.
fn rel_path_string(path: &DavPath) -> String {
    format!("/{}", path.as_rel_ospath().to_string_lossy())
}

/// Map a workspace [`WsFsError`] onto the WebDAV [`FsError`].
fn to_dav_err(e: WsFsError) -> FsError {
    match e {
        WsFsError::NotImplemented => FsError::NotImplemented,
        WsFsError::GeneralFailure => FsError::GeneralFailure,
        WsFsError::Exists => FsError::Exists,
        WsFsError::NotFound => FsError::NotFound,
        WsFsError::Forbidden => FsError::Forbidden,
    }
}

/// Map an `io::Error` from a `std::fs::Metadata` accessor onto a [`FsError`],
/// routing through the workspace's own classification.
fn io_to_dav_err(e: std::io::Error) -> FsError {
    to_dav_err(WsFsError::from(e))
}

/// Adapts a [`WorkspaceFs`] onto `dav_server`'s [`DavFileSystem`]. Pure
/// translation: [`DavPath`] ↔ workspace-relative string, and the workspace's
/// own file/metadata/dir-entry types onto the corresponding `dav_server`
/// trait objects. All disk access and side-processing happen inside
/// [`WorkspaceFs`].
#[derive(Clone)]
struct DavFs(WorkspaceFs);

impl DavFileSystem for DavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(async move {
            let opts = WsOpenOptions {
                read: options.read,
                write: options.write,
                append: options.append,
                truncate: options.truncate,
                create: options.create,
                create_new: options.create_new,
            };
            let file = self
                .0
                .open(&rel_path_string(path), opts)
                .await
                .map_err(to_dav_err)?;
            Ok(Box::new(DavFileAdapter(file)) as Box<dyn DavFile>)
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        Box::pin(async move {
            let meta = match meta {
                ReadDirMeta::Data => WsReadDirMeta::Data,
                ReadDirMeta::DataSymlink => WsReadDirMeta::DataSymlink,
                ReadDirMeta::None => WsReadDirMeta::None,
            };
            let stream = self
                .0
                .read_dir(&rel_path_string(path), meta)
                .await
                .map_err(to_dav_err)?;
            let mapped = stream.map(|res| {
                res.map(|e| Box::new(DavDirEntryAdapter(e)) as Box<dyn DavDirEntry>)
                    .map_err(to_dav_err)
            });
            Ok(Box::pin(mapped) as FsStream<Box<dyn DavDirEntry>>)
        })
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let meta = self
                .0
                .metadata(&rel_path_string(path))
                .await
                .map_err(to_dav_err)?;
            Ok(Box::new(DavMetaAdapter(meta)) as Box<dyn DavMetaData>)
        })
    }

    fn symlink_metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let meta = self
                .0
                .symlink_metadata(&rel_path_string(path))
                .await
                .map_err(to_dav_err)?;
            Ok(Box::new(DavMetaAdapter(meta)) as Box<dyn DavMetaData>)
        })
    }

    fn create_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .create_dir(&rel_path_string(path))
                .await
                .map_err(to_dav_err)
        })
    }

    fn remove_dir<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .remove_dir(&rel_path_string(path))
                .await
                .map_err(to_dav_err)
        })
    }

    fn remove_file<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .remove_file(&rel_path_string(path))
                .await
                .map_err(to_dav_err)
        })
    }

    fn rename<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .rename(&rel_path_string(from), &rel_path_string(to))
                .await
                .map_err(to_dav_err)
        })
    }

    fn copy<'a>(&'a self, from: &'a DavPath, to: &'a DavPath) -> FsFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .copy(&rel_path_string(from), &rel_path_string(to))
                .await
                .map_err(to_dav_err)
        })
    }
}

/// Adapts a workspace [`WsFile`] onto [`DavFile`].
struct DavFileAdapter(WsFile);

impl std::fmt::Debug for DavFileAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DavFileAdapter").finish_non_exhaustive()
    }
}

impl DavFile for DavFileAdapter {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let meta = self.0.metadata().await.map_err(to_dav_err)?;
            Ok(Box::new(DavMetaAdapter(meta)) as Box<dyn DavMetaData>)
        })
    }

    fn write_buf(&mut self, buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        Box::pin(async move { self.0.write_buf(buf).await.map_err(to_dav_err) })
    }

    fn write_bytes(&mut self, buf: bytes::Bytes) -> FsFuture<'_, ()> {
        Box::pin(async move { self.0.write_bytes(buf).await.map_err(to_dav_err) })
    }

    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, bytes::Bytes> {
        Box::pin(async move { self.0.read_bytes(count).await.map_err(to_dav_err) })
    }

    fn seek(&mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        Box::pin(async move { self.0.seek(pos).await.map_err(to_dav_err) })
    }

    fn flush(&mut self) -> FsFuture<'_, ()> {
        Box::pin(async move { self.0.flush().await.map_err(to_dav_err) })
    }
}

/// Adapts a [`std::fs::Metadata`] onto [`DavMetaData`]. The WebDAV-specific
/// projections (`status_changed`, `executable`) live here rather than in the
/// protocol-agnostic filesystem layer.
#[derive(Debug, Clone)]
struct DavMetaAdapter(std::fs::Metadata);

impl DavMetaData for DavMetaAdapter {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn modified(&self) -> FsResult<SystemTime> {
        self.0.modified().map_err(io_to_dav_err)
    }

    fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    fn is_file(&self) -> bool {
        self.0.is_file()
    }

    fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }

    fn accessed(&self) -> FsResult<SystemTime> {
        self.0.accessed().map_err(io_to_dav_err)
    }

    fn created(&self) -> FsResult<SystemTime> {
        self.0.created().map_err(io_to_dav_err)
    }

    #[cfg(unix)]
    fn status_changed(&self) -> FsResult<SystemTime> {
        use std::os::unix::fs::MetadataExt;
        Ok(UNIX_EPOCH + Duration::new(self.0.ctime() as u64, 0))
    }

    #[cfg(not(unix))]
    fn status_changed(&self) -> FsResult<SystemTime> {
        Err(FsError::NotImplemented)
    }

    #[cfg(unix)]
    fn executable(&self) -> FsResult<bool> {
        use std::os::unix::fs::PermissionsExt;
        if self.0.is_file() {
            return Ok((self.0.permissions().mode() & 0o100) > 0);
        }
        Err(FsError::NotImplemented)
    }

    #[cfg(not(unix))]
    fn executable(&self) -> FsResult<bool> {
        Err(FsError::NotImplemented)
    }
}

/// Adapts a workspace [`WsDirEntry`] onto [`DavDirEntry`].
struct DavDirEntryAdapter(WsDirEntry);

impl DavDirEntry for DavDirEntryAdapter {
    fn name(&self) -> Vec<u8> {
        self.0.name()
    }

    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let meta = self
            .0
            .metadata()
            .map(|m| Box::new(DavMetaAdapter(m)) as Box<dyn DavMetaData>)
            .map_err(to_dav_err);
        Box::pin(ready(meta))
    }
}
