#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mathscape_pdf::{PdfPageIndex, PdfSelection};
use mathscape_tool_marker::{
    MarkerCancellation, MarkerExecutionOptions, MarkerInstallation, MarkerProfile, MarkerRequest,
    MarkerRunStatus, MarkerRunner, MarkerStagingArea,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct WorkspaceTestDirectory {
    path: PathBuf,
}

impl WorkspaceTestDirectory {
    fn create(case_name: &str) -> Self {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".test-runs");
        fs::create_dir_all(&parent).unwrap();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = parent.join(format!(
            "{case_name}-{timestamp}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Deliberately explicit: a panicking test retains its workspace-local
    /// directory for inspection instead of erasing the failure evidence.
    fn cleanup(self) {
        let parent = self.path.parent().unwrap().to_owned();
        fs::remove_dir_all(&self.path).unwrap();
        let _ = fs::remove_dir(parent);
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn fixture(
    case_name: &str,
    script: &str,
) -> (
    WorkspaceTestDirectory,
    MarkerInstallation,
    MarkerRequest,
    MarkerStagingArea,
) {
    let directory = WorkspaceTestDirectory::create(case_name);
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let executable = bin.join("marker_single");
    write_executable(&executable, script);

    let source_pdf = directory.path().join("source book.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n% test fixture\n").unwrap();
    let selection = PdfSelection::from_pages(1, vec![PdfPageIndex::new(0)]).unwrap();
    let request = MarkerRequest::new(source_pdf, selection, MarkerProfile::MathOcrV1).unwrap();
    let installation = MarkerInstallation::new(executable).unwrap();
    let staging = MarkerStagingArea::new(directory.path().join("staging")).unwrap();
    (directory, installation, request, staging)
}

fn short_runner(log_bytes: usize) -> MarkerRunner {
    let options = MarkerExecutionOptions::new(Duration::from_secs(2))
        .unwrap()
        .poll_interval(Duration::from_millis(10))
        .unwrap()
        .graceful_shutdown(Duration::from_millis(100))
        .captured_log_bytes(log_bytes)
        .unwrap();
    MarkerRunner::new(options)
}

#[test]
fn runs_without_a_shell_command_string_and_captures_bounded_logs() {
    let script = r#"#!/bin/sh
output_root=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output_dir) output_root="$2"; shift 2 ;;
        *) shift ;;
    esac
done
printf '0123456789abcdefghijklmnopqrstuvwxyz\n'
printf 'marker warning\n' >&2
mkdir -p "$output_root/source book"
printf '# converted\n' > "$output_root/source book/source book.md"
"#;
    let (directory, installation, request, staging) = fixture("success", script);
    let run = short_runner(16)
        .execute(
            &installation,
            &request,
            &staging,
            &MarkerCancellation::new(),
        )
        .unwrap();

    assert_eq!(run.status(), MarkerRunStatus::Succeeded);
    assert_eq!(run.exit_code(), Some(0));
    assert!(run.stdout().is_truncated());
    assert_eq!(run.stdout().preview(), b"0123456789abcdef");
    assert_eq!(run.stderr().preview_lossy(), "marker warning\n");
    assert!(
        run.staging()
            .path()
            .join("source book/source book.md")
            .is_file()
    );
    assert!(run.stdout().path().is_file());
    assert!(run.stderr().path().is_file());

    let run_path = run.staging().path().to_owned();
    run.cleanup().unwrap();
    assert!(!run_path.exists());
    directory.cleanup();
}

#[test]
fn classifies_nonzero_exit_and_preserves_the_run_until_explicit_cleanup() {
    let script = "#!/bin/sh\nprintf 'conversion failed\\n' >&2\nexit 7\n";
    let (directory, installation, request, staging) = fixture("failed", script);
    let run = short_runner(1024)
        .execute(
            &installation,
            &request,
            &staging,
            &MarkerCancellation::new(),
        )
        .unwrap();

    assert_eq!(run.status(), MarkerRunStatus::Failed);
    assert_eq!(run.exit_code(), Some(7));
    assert_eq!(run.stderr().preview_lossy(), "conversion failed\n");
    let run_path = run.staging().path().to_owned();
    assert!(run_path.is_dir());

    run.cleanup().unwrap();
    assert!(!run_path.exists());
    directory.cleanup();
}

#[test]
fn times_out_and_terminates_the_private_process_group() {
    let script = "#!/bin/sh\ntrap 'exit 130' INT\nwhile :; do :; done\n";
    let (directory, installation, request, staging) = fixture("timeout", script);
    let options = MarkerExecutionOptions::new(Duration::from_millis(80))
        .unwrap()
        .poll_interval(Duration::from_millis(5))
        .unwrap()
        .graceful_shutdown(Duration::from_millis(100));
    let run = MarkerRunner::new(options)
        .execute(
            &installation,
            &request,
            &staging,
            &MarkerCancellation::new(),
        )
        .unwrap();

    assert_eq!(run.status(), MarkerRunStatus::TimedOut);
    assert!(run.elapsed() < Duration::from_secs(2));
    let run_path = run.staging().path().to_owned();
    assert!(run_path.is_dir());

    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn observes_live_cancellation() {
    let script = "#!/bin/sh\ntrap 'exit 130' INT\nwhile :; do :; done\n";
    let (directory, installation, request, staging) = fixture("cancel", script);
    let cancellation = MarkerCancellation::new();
    let signal = cancellation.clone();
    let worker = thread::spawn(move || {
        short_runner(1024).execute(&installation, &request, &staging, &signal)
    });
    thread::sleep(Duration::from_millis(80));
    cancellation.cancel();
    let run = worker.join().unwrap().unwrap();

    assert_eq!(run.status(), MarkerRunStatus::Cancelled);
    assert!(run.elapsed() < Duration::from_secs(2));
    run.cleanup().unwrap();
    directory.cleanup();
}

#[test]
fn probes_marker_version_through_the_configured_python() {
    let directory = WorkspaceTestDirectory::create("probe");
    let marker = directory.path().join("marker_single");
    let python = directory.path().join("python");
    write_executable(&marker, "#!/bin/sh\nexit 0\n");
    write_executable(&python, "#!/bin/sh\nprintf '2.0.0'\n");
    let installation = MarkerInstallation::with_python(&marker, &python)
        .unwrap()
        .with_working_directory(directory.path())
        .unwrap();

    let probe = MarkerRunner::default().probe(&installation).unwrap();
    assert_eq!(probe.executable(), marker);
    assert_eq!(probe.python_executable(), python);
    assert_eq!(probe.version().raw(), "2.0.0");

    directory.cleanup();
}
