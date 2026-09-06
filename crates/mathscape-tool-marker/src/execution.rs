use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    MarkerCommandSpec, MarkerExecutionError, MarkerInstallation, MarkerRequest, MarkerStagingArea,
    MarkerStagingDirectory,
};

#[derive(Clone, Debug)]
pub struct MarkerExecutionOptions {
    timeout: Duration,
    poll_interval: Duration,
    graceful_shutdown: Duration,
    captured_log_bytes: usize,
}

impl MarkerExecutionOptions {
    pub fn new(timeout: Duration) -> Result<Self, MarkerExecutionError> {
        if timeout.is_zero() {
            return Err(MarkerExecutionError::InvalidOption {
                field: "marker.execution.timeout",
                reason: "must be greater than zero",
            });
        }
        Ok(Self {
            timeout,
            ..Self::default()
        })
    }

    pub fn poll_interval(mut self, interval: Duration) -> Result<Self, MarkerExecutionError> {
        if interval.is_zero() {
            return Err(MarkerExecutionError::InvalidOption {
                field: "marker.execution.poll_interval",
                reason: "must be greater than zero",
            });
        }
        self.poll_interval = interval;
        Ok(self)
    }

    pub fn graceful_shutdown(mut self, duration: Duration) -> Self {
        self.graceful_shutdown = duration;
        self
    }

    pub fn captured_log_bytes(mut self, bytes: usize) -> Result<Self, MarkerExecutionError> {
        if bytes == 0 {
            return Err(MarkerExecutionError::InvalidOption {
                field: "marker.execution.captured_log_bytes",
                reason: "must be greater than zero",
            });
        }
        self.captured_log_bytes = bytes;
        Ok(self)
    }

    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Default for MarkerExecutionOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30 * 60),
            poll_interval: Duration::from_millis(100),
            graceful_shutdown: Duration::from_secs(5),
            captured_log_bytes: 1024 * 1024,
        }
    }
}

/// Cloneable cooperative cancellation handle for one or more queued runs.
#[derive(Clone, Debug, Default)]
pub struct MarkerCancellation(Arc<AtomicBool>);

impl MarkerCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerRunStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedLog {
    path: PathBuf,
    preview: Vec<u8>,
    truncated: bool,
}

impl CapturedLog {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn preview(&self) -> &[u8] {
        &self.preview
    }

    pub fn preview_lossy(&self) -> String {
        String::from_utf8_lossy(&self.preview).into_owned()
    }

    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerRun {
    command: MarkerCommandSpec,
    staging: MarkerStagingDirectory,
    status: MarkerRunStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
    started_at: SystemTime,
    finished_at: SystemTime,
    stdout: CapturedLog,
    stderr: CapturedLog,
    forced_termination: bool,
}

impl MarkerRun {
    pub const fn command(&self) -> &MarkerCommandSpec {
        &self.command
    }

    pub const fn staging(&self) -> &MarkerStagingDirectory {
        &self.staging
    }

    pub const fn status(&self) -> MarkerRunStatus {
        self.status
    }

    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub const fn finished_at(&self) -> SystemTime {
        self.finished_at
    }

    pub const fn stdout(&self) -> &CapturedLog {
        &self.stdout
    }

    pub const fn stderr(&self) -> &CapturedLog {
        &self.stderr
    }

    /// True when graceful interruption did not stop Marker before the deadline.
    pub const fn forced_termination(&self) -> bool {
        self.forced_termination
    }

    /// Explicitly removes this run's owned staging directory and all its products.
    pub fn cleanup(self) -> Result<(), MarkerExecutionError> {
        self.staging.cleanup()
    }
}

#[derive(Clone, Debug)]
pub struct MarkerRunner {
    options: MarkerExecutionOptions,
}

impl MarkerRunner {
    pub fn new(options: MarkerExecutionOptions) -> Self {
        Self { options }
    }

    pub const fn options(&self) -> &MarkerExecutionOptions {
        &self.options
    }

    pub fn execute(
        &self,
        installation: &MarkerInstallation,
        request: &MarkerRequest,
        staging_area: &MarkerStagingArea,
        cancellation: &MarkerCancellation,
    ) -> Result<MarkerRun, MarkerExecutionError> {
        validate_executable(installation.executable(), "marker.executable")?;
        validate_directory(installation.working_directory(), "marker.working_directory")?;
        validate_regular_file(request.source_pdf(), "marker.source_pdf")?;

        let staging = staging_area.allocate()?;
        let command = match request.prepare(installation, staging.path()) {
            Ok(command) => command,
            Err(error) => {
                staging.cleanup()?;
                return Err(error.into());
            }
        };
        self.execute_prepared(command, staging, cancellation)
    }

    pub(crate) fn execute_prepared(
        &self,
        command: MarkerCommandSpec,
        staging: MarkerStagingDirectory,
        cancellation: &MarkerCancellation,
    ) -> Result<MarkerRun, MarkerExecutionError> {
        let stdout_path = staging.path().join("marker.stdout.log");
        let stderr_path = staging.path().join("marker.stderr.log");
        let stdout_file = create_log(&stdout_path)?;
        let stderr_file = create_log(&stderr_path)?;
        let started_at = SystemTime::now();
        let started = Instant::now();

        if cancellation.is_cancelled() {
            return self.finish_run(
                command,
                staging,
                MarkerRunStatus::Cancelled,
                None,
                started.elapsed(),
                started_at,
                stdout_path,
                stderr_path,
                false,
            );
        }

        let mut process = Command::new(command.program());
        process
            .args(command.arguments())
            .current_dir(command.working_directory())
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        configure_child_process(&mut process);
        let mut child = process
            .spawn()
            .map_err(|source| MarkerExecutionError::Spawn {
                program: command.program().to_owned(),
                staging_root: staging.path().to_owned(),
                source,
            })?;

        let (status, exit_status, forced_termination) =
            loop {
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        let status = if exit_status.success() {
                            MarkerRunStatus::Succeeded
                        } else {
                            MarkerRunStatus::Failed
                        };
                        break (status, Some(exit_status), false);
                    }
                    Ok(None) => {}
                    Err(source) => {
                        return Err(MarkerExecutionError::Wait {
                            program: command.program().to_owned(),
                            staging_root: staging.path().to_owned(),
                            source,
                        });
                    }
                }

                let requested_status = if cancellation.is_cancelled() {
                    Some(MarkerRunStatus::Cancelled)
                } else if started.elapsed() >= self.options.timeout {
                    Some(MarkerRunStatus::TimedOut)
                } else {
                    None
                };
                if let Some(requested_status) = requested_status {
                    let (exit_status, forced) = terminate_child(&mut child, &self.options)
                        .map_err(|source| MarkerExecutionError::Wait {
                            program: command.program().to_owned(),
                            staging_root: staging.path().to_owned(),
                            source,
                        })?;
                    break (requested_status, Some(exit_status), forced);
                }
                thread::sleep(self.options.poll_interval);
            };

        self.finish_run(
            command,
            staging,
            status,
            exit_status.and_then(|status| status.code()),
            started.elapsed(),
            started_at,
            stdout_path,
            stderr_path,
            forced_termination,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_run(
        &self,
        command: MarkerCommandSpec,
        staging: MarkerStagingDirectory,
        status: MarkerRunStatus,
        exit_code: Option<i32>,
        elapsed: Duration,
        started_at: SystemTime,
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        forced_termination: bool,
    ) -> Result<MarkerRun, MarkerExecutionError> {
        Ok(MarkerRun {
            command,
            staging,
            status,
            exit_code,
            elapsed,
            started_at,
            finished_at: SystemTime::now(),
            stdout: capture_log(stdout_path, self.options.captured_log_bytes)?,
            stderr: capture_log(stderr_path, self.options.captured_log_bytes)?,
            forced_termination,
        })
    }
}

impl Default for MarkerRunner {
    fn default() -> Self {
        Self::new(MarkerExecutionOptions::default())
    }
}

pub(crate) fn validate_regular_file(
    path: &Path,
    field: &'static str,
) -> Result<(), MarkerExecutionError> {
    let metadata = fs::metadata(path).map_err(|source| MarkerExecutionError::Access {
        field,
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(MarkerExecutionError::NotAFile {
            field,
            path: path.to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_executable(
    path: &Path,
    field: &'static str,
) -> Result<(), MarkerExecutionError> {
    validate_regular_file(path, field)?;
    let executable = is_executable(path).map_err(|source| MarkerExecutionError::Access {
        field,
        path: path.to_owned(),
        source,
    })?;
    if !executable {
        return Err(MarkerExecutionError::NotExecutable {
            field,
            path: path.to_owned(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> std::io::Result<bool> {
    // Windows executable resolution is delegated to CreateProcess at spawn.
    Ok(true)
}

pub(crate) fn validate_directory(
    path: &Path,
    field: &'static str,
) -> Result<(), MarkerExecutionError> {
    let metadata = fs::metadata(path).map_err(|source| MarkerExecutionError::Access {
        field,
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(MarkerExecutionError::NotADirectory {
            field,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn create_log(path: &Path) -> Result<File, MarkerExecutionError> {
    File::create(path).map_err(|source| MarkerExecutionError::Staging {
        path: path.to_owned(),
        source,
    })
}

fn capture_log(path: PathBuf, maximum: usize) -> Result<CapturedLog, MarkerExecutionError> {
    let mut file = File::open(&path).map_err(|source| MarkerExecutionError::Staging {
        path: path.clone(),
        source,
    })?;
    let mut preview = Vec::with_capacity(maximum.min(64 * 1024));
    file.by_ref()
        .take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut preview)
        .map_err(|source| MarkerExecutionError::Staging {
            path: path.clone(),
            source,
        })?;
    let truncated = preview.len() > maximum;
    preview.truncate(maximum);
    Ok(CapturedLog {
        path,
        preview,
        truncated,
    })
}

fn terminate_child(
    child: &mut Child,
    options: &MarkerExecutionOptions,
) -> std::io::Result<(ExitStatus, bool)> {
    request_interrupt(child)?;
    let deadline = Instant::now() + options.graceful_shutdown;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        thread::sleep(options.poll_interval.min(Duration::from_millis(50)));
    }
    request_force_kill(child)?;
    child.wait().map(|status| (status, true))
}

#[cfg(unix)]
fn configure_child_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // A private process group lets cancellation reach Marker's ordinary child
    // processes without ever signalling the Mathscape process group.
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process(_command: &mut Command) {}

#[cfg(unix)]
fn request_interrupt(child: &mut Child) -> std::io::Result<()> {
    send_process_group_signal(child, nix::sys::signal::Signal::SIGINT)
}

#[cfg(not(unix))]
fn request_interrupt(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

#[cfg(unix)]
fn request_force_kill(child: &mut Child) -> std::io::Result<()> {
    send_process_group_signal(child, nix::sys::signal::Signal::SIGKILL)
}

#[cfg(not(unix))]
fn request_force_kill(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

#[cfg(unix)]
fn send_process_group_signal(
    child: &Child,
    signal: nix::sys::signal::Signal,
) -> std::io::Result<()> {
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};

    let process_group = Pid::from_raw(-(child.id() as i32));
    match kill(process_group, signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}
