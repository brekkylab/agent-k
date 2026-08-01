//! Convert PDFs to Markdown via a PyInstaller-bundled `docling` binary.
//!
//! `build.rs` runs `uv sync` + `pyinstaller` against the Python sources in
//! `python/`, producing a self-contained bundle directory. At runtime, the
//! library expects the bundle to sit as a `run_docling/` folder directly
//! beside the consuming executable. `build.rs` arranges this for cargo
//! builds; when shipping, copy `run_docling/` next to your executable.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const BUNDLE_DIR_NAME: &str = "run_docling";

#[cfg(windows)]
const BUNDLE_BINARY: &str = "run_docling.exe";
#[cfg(not(windows))]
const BUNDLE_BINARY: &str = "run_docling";

static RESOLVED_BUNDLE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Path to the bundle directory (`run_docling/`) sitting beside the
/// current executable, or `None` if it is not present.
pub fn bundle_dir() -> Option<&'static Path> {
    RESOLVED_BUNDLE_DIR
        .get_or_init(|| {
            let dir = std::env::current_exe().ok()?.parent()?.join(BUNDLE_DIR_NAME);
            dir.join(BUNDLE_BINARY).is_file().then_some(dir)
        })
        .as_deref()
}

/// TableFormer extraction mode. `Accurate` trades speed for quality.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TableStructureMode {
    Fast,
    Accurate,
}

/// Hardware device for model inference. Mirrors docling's `AcceleratorDevice`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AcceleratorDevice {
    Auto,
    Cpu,
    Cuda,
    Mps,
    Xpu,
}

/// Options forwarded to the `PdfPipelineOptions` constructor on the Python
/// side. Defaults match the previously-hardcoded behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfOptions {
    pub do_ocr: bool,
    pub do_table_structure: bool,
    pub do_cell_matching: bool,
    pub table_structure_mode: TableStructureMode,
    pub do_picture_classification: bool,
    pub do_picture_description: bool,
    pub do_chart_extraction: bool,
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub generate_page_images: bool,
    pub generate_picture_images: bool,
    pub num_threads: u32,
    pub device: AcceleratorDevice,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            do_ocr: false,
            do_table_structure: true,
            do_cell_matching: true,
            table_structure_mode: TableStructureMode::Accurate,
            do_picture_classification: false,
            do_picture_description: false,
            do_chart_extraction: false,
            do_code_enrichment: false,
            do_formula_enrichment: false,
            generate_page_images: false,
            generate_picture_images: false,
            num_threads: 4,
            device: AcceleratorDevice::Auto,
        }
    }
}

/// How long a single conversion may run before the parser is killed. The input
/// is a document from outside — uploaded, or fetched from the web — and the
/// parser runs on the host with the caller's own privileges, so a document that
/// makes it spin must not be able to hold a worker forever.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Convert PDF bytes to Markdown, giving up after [`DEFAULT_TIMEOUT`].
pub async fn convert_pdf_to_md(pdf_bytes: &[u8], options: &PdfOptions) -> anyhow::Result<String> {
    convert_pdf_to_md_within(pdf_bytes, options, DEFAULT_TIMEOUT).await
}

/// Convert PDF bytes to Markdown, giving up after `timeout`.
///
/// On expiry the parser is killed rather than left behind, and so is anything it
/// started: the command gets its own process group and the timeout path SIGKILLs
/// the group. `kill_on_drop` is the backstop for the direct child on any other
/// path out. Without this a parse that never finishes leaves orphans holding CPU
/// and memory for the life of the host process — docling shells out to OCR
/// helpers, so killing only the process we spawned is not enough.
pub async fn convert_pdf_to_md_within(
    pdf_bytes: &[u8],
    options: &PdfOptions,
    timeout: Duration,
) -> anyhow::Result<String> {
    let dir = bundle_dir().ok_or_else(|| {
        anyhow!(
            "docling bundle not found next to the current executable; \
             expected a `{BUNDLE_DIR_NAME}/` directory containing `{BUNDLE_BINARY}`"
        )
    })?;
    let exe = dir.join(BUNDLE_BINARY);
    let options_json = serde_json::to_string(options).context("serialize PdfOptions")?;
    let stdout = run_bundle(&exe, pdf_bytes, &options_json, timeout).await?;
    String::from_utf8(stdout).context("run_docling stdout was not valid UTF-8")
}

/// Spawn `exe`, feed it `pdf_bytes` on stdin, and return its stdout — or kill it
/// and fail once `timeout` elapses.
///
/// Takes the executable path rather than resolving the bundle so the timeout
/// path is exercisable against a stand-in parser.
async fn run_bundle(
    exe: &Path,
    pdf_bytes: &[u8],
    options_json: &str,
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new(exe);
    command
        .arg("--options")
        .arg(options_json)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Backstop for the direct child if this future is dropped for any
        // reason other than the timeout below.
        .kill_on_drop(true);
    // Its own process group, so the kill on timeout reaches whatever the parser
    // started as well. docling shells out (OCR helpers, for one), and killing
    // only the process we spawned would leave those holding the CPU.
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", exe.display()))?;
    let pid = child.id();

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("child stdin unavailable"))?;

    // The write is inside the timeout too, not just the wait: a parser that
    // never drains stdin fills the pipe buffer and blocks the write instead of
    // the wait, which is the same hang through a different door.
    let pdf_bytes = pdf_bytes.to_vec();
    let run = async move {
        stdin.write_all(&pdf_bytes).await?;
        stdin.shutdown().await?;
        drop(stdin);
        child.wait_with_output().await
    };

    // Pinned to a named local so `run` — and with it the child — is visibly
    // still alive in the timeout arm below. Passing the future to `timeout`
    // directly also works today, but only because a temporary in a match
    // scrutinee outlives the arms, an invisible rule that binding the result
    // with `let` first would quietly remove.
    let mut run = std::pin::pin!(run);
    let output = match tokio::time::timeout(timeout, &mut run).await {
        Ok(result) => result?,
        Err(_) => {
            kill_process_group(pid);
            anyhow::bail!("run_docling timed out after {timeout:?}; parser killed");
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("run_docling exited with {}: {}", output.status, stderr);
    }
    Ok(output.stdout)
}

/// SIGKILL the process group led by `pid`. Best-effort: by the time this runs
/// the group may already be gone, and the only failure mode that matters
/// (nothing left to kill) is the one we want anyway.
#[cfg(unix)]
fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: `killpg` on a pid we spawned into its own group, so `pid` is
        // that group's id. What keeps it from naming an unrelated group is the
        // kernel, not the caller: a pid stays reserved while it is in use as a
        // process group id with live members, so it cannot be handed to a new
        // process while there is still anything here to kill. Holding the child
        // handle is NOT what guarantees this — `wait_with_output` is a join over
        // `wait()` and the two pipe drains, so `wait()` can reap the parser as
        // soon as it exits while a grandchild still holds the pipes open, which
        // is exactly the OCR-helper case this kill exists for. A group that has
        // already exited returns ESRCH, which is the outcome we want anyway.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// No process groups here; `kill_on_drop` handles the direct child.
#[cfg(not(unix))]
fn kill_process_group(_pid: Option<u32>) {}

/// Read a file from disk and convert it to Markdown.
pub async fn convert_pdf_file(
    path: impl AsRef<Path>,
    options: &PdfOptions,
) -> anyhow::Result<String> {
    let bytes = tokio::fs::read(path.as_ref())
        .await
        .with_context(|| format!("failed to read {}", path.as_ref().display()))?;
    convert_pdf_to_md(&bytes, options).await
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    /// Write an executable stand-in parser that records the pid of a child it
    /// starts, then hangs. The recorded child is what proves the kill reaches
    /// past the process we spawned.
    fn hanging_parser(dir: &Path, pidfile: &Path) -> PathBuf {
        let script = dir.join("stub_parser");
        let mut f = std::fs::File::create(&script).expect("create stub");
        write!(
            f,
            "#!/bin/sh\nsleep 120 &\necho $! > {}\nwait\n",
            pidfile.display()
        )
        .expect("write stub");
        drop(f);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod stub");
        script
    }

    fn pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 only probes for the process's existence.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// The stub writes its pidfile with a shell redirect, so allow a moment for
    /// the file to appear and hold a complete number.
    fn read_recorded_pid(pidfile: &Path) -> Option<u32> {
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(pidfile)
                && let Ok(pid) = s.trim().parse::<u32>()
            {
                return Some(pid);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// A parse that never finishes must be cut off by the library itself, and
    /// must not leave the parser (or what the parser started) behind. Before the
    /// timeout existed, the call returned only when the caller imposed its own
    /// deadline, and the child stayed on the host.
    #[tokio::test]
    async fn timeout_cuts_off_a_hanging_parse_and_kills_its_children() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("child.pid");
        let stub = hanging_parser(dir.path(), &pidfile);

        // Comfortably longer than shell startup, so the stub is certain to have
        // recorded its child before the deadline fires. A tighter deadline races
        // the stub and turns this into a flake under parallel test load.
        let started = std::time::Instant::now();
        let err = run_bundle(&stub, b"%PDF-1.4", "{}", Duration::from_secs(2))
            .await
            .expect_err("a hanging parser must not return Ok");
        let elapsed = started.elapsed();

        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "the library must impose the deadline itself, took {elapsed:?}"
        );

        // The grandchild `sleep` is only reachable through the process group, so
        // this is what distinguishes a group kill from killing just the child.
        let pid: u32 = read_recorded_pid(&pidfile).expect("stub should record its child pid");
        let mut alive = true;
        for _ in 0..40 {
            if !pid_alive(pid) {
                alive = false;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive, "parser's child {pid} survived the timeout");
    }

    /// The happy path still returns stdout, so the timeout wrapper is not
    /// swallowing successful conversions.
    #[tokio::test]
    async fn successful_parse_returns_stdout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("stub_ok");
        let mut f = std::fs::File::create(&script).expect("create stub");
        write!(f, "#!/bin/sh\ncat >/dev/null\nprintf '# ok'\n").expect("write stub");
        drop(f);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod stub");

        let out = run_bundle(&script, b"%PDF-1.4", "{}", Duration::from_secs(20))
            .await
            .expect("stub should succeed");
        assert_eq!(String::from_utf8(out).unwrap(), "# ok");
    }
}
