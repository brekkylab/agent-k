use std::future::ready;
use std::io::SeekFrom;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

use crate::auth::authenticate;
use crate::state::AppState;
use workspace::{
    DirEntry as WsDirEntry, File as WsFile, FsError as WsFsError, OpenOptions as WsOpenOptions,
    Stat, WorkspaceFs,
};

/// WebDAV workspace router. Mounted by [`super::get_router`] at
/// `/workspaces/{wid}/files[/…]`; exposes
/// `data_root/workspaces/{wid}/files` as a per-workspace filesystem.
///
/// Routes via `fallback` so axum forwards every HTTP method — including
/// WebDAV-specific ones (`PROPFIND`, `MKCOL`, `COPY`, `MOVE`, `LOCK`, …) —
/// straight to [`dav_server`]. Auth mirrors the WS route: JWT is read from
/// `?token=…` because the eventual target audience (browser fetch + native
/// WebDAV clients) cannot reliably set custom auth headers. The token's subject
/// must own the workspace.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new().fallback(handle).with_state(state)
}

async fn handle(State(state): State<Arc<AppState>>, req: Request) -> Response<Body> {
    let wid = match parse_wid(req.uri().path()) {
        Some(w) => w,
        None => return (StatusCode::BAD_REQUEST, "invalid workspace id").into_response(),
    };

    let token = req.uri().query().and_then(extract_token);
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    };
    let user = match authenticate(&state, &token).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };

    // Access is gated by `get_for_user`: a workspace the caller can't reach is
    // indistinguishable from a missing one (404), so existence can't be probed.
    match state.workspaces.get_for_user(user.id, wid).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "workspace not found").into_response(),
        Err(e) => {
            tracing::error!("workspace lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    }

    // The filesystem (and its side-processing) lives in `state.workspaces`;
    // here we only wrap it in the WebDAV protocol (see [`DavFs`]). Building it
    // loads the workspace's external-provider mounts, which can fail (bad
    // stored config); surface that as a 500 rather than panicking.
    let fs = match state.workspaces.get_fs(wid).await {
        Ok(fs) => fs,
        Err(e) => {
            tracing::error!("failed to build workspace filesystem: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };
    let dav = DavHandler::builder()
        .filesystem(Box::new(DavFs(fs)))
        .locksystem(FakeLs::new())
        .strip_prefix(format!("/workspaces/{wid}/files"))
        .build_handler();

    dav.handle(req).await.map(Body::new)
}

fn parse_wid(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/workspaces/")?;
    let (wid_str, _) = rest.split_once('/')?;
    let wid = Uuid::parse_str(wid_str).ok()?;
    // Require the canonical (lowercase-hyphenated) form. The DAV strip prefix is
    // built from `wid`'s Display and dav-server matches it byte-for-byte, so a
    // non-canonical segment (uppercase, or the 32-char no-hyphen form) would
    // authenticate but then 502 on PrefixMismatch. Reject it up front as a 400.
    (wid_str == wid.to_string()).then_some(wid)
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
        _meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        Box::pin(async move {
            // `_meta` (symlink-follow hint) is moot: the workspace resolves
            // symlinks to their target and never reports the symlink kind.
            let stream = self
                .0
                .read_dir(&rel_path_string(path))
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

/// Adapts a workspace [`Stat`] onto [`DavMetaData`]. The `Stat` already carries
/// the WebDAV-specific projections (`status_changed`, `executable`), computed at
/// capture time in the filesystem layer; a `None` there (e.g. an external
/// provider that doesn't report the field) surfaces as `NotImplemented`.
#[derive(Debug, Clone)]
struct DavMetaAdapter(Stat);

impl DavMetaData for DavMetaAdapter {
    fn len(&self) -> u64 {
        self.0.len
    }

    fn modified(&self) -> FsResult<SystemTime> {
        // WebDAV wants a Last-Modified; fall back to the epoch when the source
        // (e.g. an external object without a timestamp) doesn't report one.
        Ok(self.0.modified.unwrap_or(UNIX_EPOCH))
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
        self.0.accessed.ok_or(FsError::NotImplemented)
    }

    fn created(&self) -> FsResult<SystemTime> {
        self.0.created.ok_or(FsError::NotImplemented)
    }

    fn status_changed(&self) -> FsResult<SystemTime> {
        self.0.status_changed.ok_or(FsError::NotImplemented)
    }

    fn executable(&self) -> FsResult<bool> {
        self.0.executable.ok_or(FsError::NotImplemented)
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

#[cfg(test)]
mod tests {
    use super::parse_wid;

    #[test]
    fn parse_wid_requires_canonical_form() {
        let canon = "15fc016b-0000-4000-8000-000000000000";
        // Canonical form, with or without a trailing sub-path.
        assert!(parse_wid(&format!("/workspaces/{canon}/files")).is_some());
        assert!(parse_wid(&format!("/workspaces/{canon}/files/sub/a.txt")).is_some());

        // Uppercase and 32-char no-hyphen forms parse to the same Uuid but
        // aren't canonical — they'd 502 on the byte-exact DAV strip prefix.
        assert!(parse_wid("/workspaces/15FC016B-0000-4000-8000-000000000000/files").is_none());
        assert!(parse_wid("/workspaces/15fc016b000040008000000000000000/files").is_none());

        // Malformed or missing id segment.
        assert!(parse_wid("/workspaces/not-a-uuid/files").is_none());
        assert!(parse_wid(&format!("/workspaces/{canon}")).is_none());
    }
}
