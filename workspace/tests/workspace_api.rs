//! External integration test: drive the crate's public Rust API end-to-end
//! (no backend, no DB, no VM), proving the workspace filesystem is usable and
//! testable standalone through `workspace::…` alone. Local files live under the
//! `/files` mount.

use bytes::Bytes;
use futures_util::StreamExt;
use workspace::{OpenOptions, WorkspaceFs};

async fn dir_names(fs: &WorkspaceFs, path: &str) -> Vec<String> {
    let mut stream = fs.read_dir(path).await.unwrap();
    let mut names = Vec::new();
    while let Some(e) = stream.next().await {
        names.push(String::from_utf8(e.unwrap().name()).unwrap());
    }
    names.sort();
    names
}

#[tokio::test]
async fn local_file_round_trip_through_public_api() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = WorkspaceFs::local(tmp.path().to_path_buf());

    // Write via the public open/write/flush API.
    let mut f = fs
        .open(
            "/files/note.txt",
            OpenOptions {
                write: true,
                create: true,
                truncate: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    f.write_bytes(Bytes::from_static(b"hello workspace"))
        .await
        .unwrap();
    f.flush().await.unwrap();

    // metadata reports a file of the right size.
    let st = fs.metadata("/files/note.txt").await.unwrap();
    assert!(st.is_file());
    assert_eq!(st.len, 15);

    // Read it back.
    let mut r = fs
        .open(
            "/files/note.txt",
            OpenOptions {
                read: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let bytes = r.read_bytes(64).await.unwrap();
    assert_eq!(&bytes[..], b"hello workspace");

    // The root lists the mount; the mount lists the file.
    assert!(dir_names(&fs, "/").await.contains(&"files".to_string()));
    assert!(dir_names(&fs, "/files").await.contains(&"note.txt".to_string()));
}

#[tokio::test]
async fn create_and_remove_dir_through_public_api() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = WorkspaceFs::local(tmp.path().to_path_buf());

    fs.create_dir("/files/sub").await.unwrap();
    assert!(fs.metadata("/files/sub").await.unwrap().is_dir());
    assert!(dir_names(&fs, "/files").await.contains(&"sub".to_string()));

    fs.remove_dir("/files/sub").await.unwrap();
    assert!(fs.metadata("/files/sub").await.is_err());
}
