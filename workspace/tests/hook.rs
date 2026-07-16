//! The injected FsHook receives one FsEvent per local mutation, in order, with
//! workspace-relative paths — the seam the backend uses for knowledge ingestion.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use uuid::Uuid;
use workspace::{FsEvent, FsHook, OpenOptions, WorkspaceFs};

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl FsHook for Recorder {
    fn on_change(&self, _wid: Uuid, event: FsEvent<'_>) {
        let line = match event {
            FsEvent::Created(p) => format!("created {p}"),
            FsEvent::Modified(p) => format!("modified {p}"),
            FsEvent::Removed(p) => format!("removed {p}"),
        };
        self.0.lock().unwrap().push(line);
    }
}

fn write_opts() -> OpenOptions {
    OpenOptions {
        write: true,
        create: true,
        truncate: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn fs_hook_receives_mutation_events_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let hook: Arc<dyn FsHook> = rec.clone();
    let fs = WorkspaceFs::new(tmp.path().to_path_buf(), Uuid::new_v4()).with_hook(Some(hook));

    // create -> modify (same path, second open sees it existing)
    let mut f = fs.open("/note.txt", write_opts()).await.unwrap();
    f.write_bytes(Bytes::from_static(b"hi")).await.unwrap();
    f.flush().await.unwrap();
    let mut f = fs.open("/note.txt", write_opts()).await.unwrap();
    f.write_bytes(Bytes::from_static(b"yo")).await.unwrap();
    f.flush().await.unwrap();

    // rename (source removed + fresh destination created), copy, unlink
    fs.rename("/note.txt", "/renamed.txt").await.unwrap();
    fs.copy("/renamed.txt", "/copy.txt").await.unwrap();
    fs.remove_file("/copy.txt").await.unwrap();

    // mkdir fires nothing; rmdir removes.
    fs.create_dir("/d").await.unwrap();
    fs.remove_dir("/d").await.unwrap();

    assert_eq!(
        *rec.0.lock().unwrap(),
        vec![
            "created /note.txt",
            "modified /note.txt",
            "removed /note.txt",
            "created /renamed.txt",
            "created /copy.txt",
            "removed /copy.txt",
            "removed /d",
        ]
    );
}

#[tokio::test]
async fn no_hook_is_a_silent_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = WorkspaceFs::new(tmp.path().to_path_buf(), Uuid::new_v4());
    // Same mutations, no hook attached: must succeed without firing/panicking.
    let mut f = fs.open("/x.txt", write_opts()).await.unwrap();
    f.write_bytes(Bytes::from_static(b"z")).await.unwrap();
    f.flush().await.unwrap();
    fs.remove_file("/x.txt").await.unwrap();
}
