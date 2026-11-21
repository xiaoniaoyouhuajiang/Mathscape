use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

#[cfg(windows)]
use command_group::{CommandGroup, GroupChild as Child};
#[cfg(not(windows))]
use std::process::Child;

use crate::LeanRunnerError;

/// An argv-safe process specification. It is deliberately not a shell
/// command string: each argument remains an independent OS value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanCommand {
    program: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: BTreeMap<OsString, OsString>,
}

impl LeanCommand {
    pub fn new(program: impl Into<PathBuf>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            working_directory: working_directory.into(),
            environment: BTreeMap::new(),
        }
    }

    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
    pub fn environment(&self) -> &BTreeMap<OsString, OsString> {
        &self.environment
    }
}

/// Limits and process-control policy for one Lean/Lake invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanRunnerOptions {
    timeout: Duration,
    poll_interval: Duration,
    graceful_shutdown: Duration,
    max_output_bytes: usize,
}

impl LeanRunnerOptions {
    pub fn new(timeout: Duration) -> Result<Self, LeanRunnerError> {
        if timeout.is_zero() {
            return Err(LeanRunnerError::InvalidOption {
                field: "lean.runner.timeout",
                reason: "must be greater than zero",
            });
        }
        Ok(Self {
            timeout,
            ..Self::default()
        })
    }

    pub fn poll_interval(mut self, interval: Duration) -> Result<Self, LeanRunnerError> {
        if interval.is_zero() {
            return Err(LeanRunnerError::InvalidOption {
                field: "lean.runner.poll_interval",
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

    pub fn max_output_bytes(mut self, bytes: usize) -> Result<Self, LeanRunnerError> {
        if bytes == 0 {
            return Err(LeanRunnerError::InvalidOption {
                field: "lean.runner.max_output_bytes",
                reason: "must be greater than zero",
            });
        }
        self.max_output_bytes = bytes;
        Ok(self)
    }

    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
    pub const fn poll_interval_duration(&self) -> Duration {
        self.poll_interval
    }
    pub const fn graceful_shutdown_duration(&self) -> Duration {
        self.graceful_shutdown
    }
    pub const fn output_limit(&self) -> usize {
        self.max_output_bytes
    }
}

impl Default for LeanRunnerOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(25),
            graceful_shutdown: Duration::from_secs(1),
            max_output_bytes: 1024 * 1024,
        }
    }
}

/// Cloneable cooperative cancellation handle for one process.
#[derive(Clone, Debug, Default)]
pub struct LeanCancellation(Arc<AtomicBool>);

impl LeanCancellation {
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
pub enum LeanRunStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedOutput {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanRun {
    command: LeanCommand,
    status: LeanRunStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
    started_at: SystemTime,
    finished_at: SystemTime,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    forced_termination: bool,
}

impl LeanRun {
    pub fn command(&self) -> &LeanCommand {
        &self.command
    }
    pub const fn status(&self) -> LeanRunStatus {
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
    pub const fn stdout(&self) -> &CapturedOutput {
        &self.stdout
    }
    pub const fn stderr(&self) -> &CapturedOutput {
        &self.stderr
    }
    pub const fn forced_termination(&self) -> bool {
        self.forced_termination
    }
}

#[derive(Clone, Debug)]
pub struct LeanRunner {
    options: LeanRunnerOptions,
}

impl LeanRunner {
    pub fn new(options: LeanRunnerOptions) -> Self {
        Self { options }
    }
    pub const fn options(&self) -> &LeanRunnerOptions {
        &self.options
    }

    /// Runs one argv-safe command with no stdin. The project directory and
    /// executable are validated before spawn. The runner itself does not
    /// write project files; the child is not an OS-level sandbox and must be
    /// controlled by the caller's approval policy.
    pub fn run(
        &self,
        command: &LeanCommand,
        cancellation: &LeanCancellation,
    ) -> Result<LeanRun, LeanRunnerError> {
        validate_working_directory(command.working_directory())?;
        validate_program(command.program())?;
        let started_at = SystemTime::now();
        let started = Instant::now();

        if cancellation.is_cancelled() {
            return Ok(empty_run(
                command.clone(),
                LeanRunStatus::Cancelled,
                started_at,
            ));
        }

        let mut process = Command::new(command.program());
        process
            .args(command.arguments())
            .current_dir(command.working_directory())
            .envs(&command.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_child(&mut process).map_err(|source| LeanRunnerError::Spawn {
            program: command.program().to_owned(),
            source,
        })?;

        let Some(stdout) = child_stdio(&mut child).stdout.take() else {
            let _ = terminate_child(&mut child, &self.options);
            return Err(LeanRunnerError::Capture {
                program: command.program().to_owned(),
                reason: "stdout pipe was not created",
            });
        };
        let Some(stderr) = child_stdio(&mut child).stderr.take() else {
            let _ = terminate_child(&mut child, &self.options);
            return Err(LeanRunnerError::Capture {
                program: command.program().to_owned(),
                reason: "stderr pipe was not created",
            });
        };
        let stdout_handle = spawn_capture(stdout, self.options.max_output_bytes);
        let stderr_handle = spawn_capture(stderr, self.options.max_output_bytes);

        let (status, exit_status, forced_termination) =
            loop {
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        let status = if exit_status.success() {
                            LeanRunStatus::Succeeded
                        } else {
                            LeanRunStatus::Failed
                        };
                        break (status, Some(exit_status), false);
                    }
                    Ok(None) => {}
                    Err(source) => {
                        let _ = terminate_child(&mut child, &self.options);
                        let _ = stdout_handle.join();
                        let _ = stderr_handle.join();
                        return Err(LeanRunnerError::Wait {
                            program: command.program().to_owned(),
                            source,
                        });
                    }
                }

                let requested_status = if cancellation.is_cancelled() {
                    Some(LeanRunStatus::Cancelled)
                } else if started.elapsed() >= self.options.timeout {
                    Some(LeanRunStatus::TimedOut)
                } else {
                    None
                };
                if let Some(requested_status) = requested_status {
                    let (exit_status, forced) = terminate_child(&mut child, &self.options)
                        .map_err(|source| LeanRunnerError::Terminate {
                            program: command.program().to_owned(),
                            source,
                        })?;
                    break (requested_status, Some(exit_status), forced);
                }
                thread::sleep(self.options.poll_interval);
            };

        let stdout = join_capture(stdout_handle, command.program())?;
        let stderr = join_capture(stderr_handle, command.program())?;
        Ok(LeanRun {
            command: command.clone(),
            status,
            exit_code: exit_status.and_then(|status| status.code()),
            elapsed: started.elapsed(),
            started_at,
            finished_at: SystemTime::now(),
            stdout,
            stderr,
            forced_termination,
        })
    }
}

impl Default for LeanRunner {
    fn default() -> Self {
        Self::new(LeanRunnerOptions::default())
    }
}

fn empty_run(command: LeanCommand, status: LeanRunStatus, started_at: SystemTime) -> LeanRun {
    LeanRun {
        command,
        status,
        exit_code: None,
        elapsed: Duration::ZERO,
        started_at,
        finished_at: SystemTime::now(),
        stdout: CapturedOutput {
            bytes: Vec::new(),
            truncated: false,
        },
        stderr: CapturedOutput {
            bytes: Vec::new(),
            truncated: false,
        },
        forced_termination: false,
    }
}

fn validate_working_directory(path: &Path) -> Result<(), LeanRunnerError> {
    let metadata = fs::metadata(path).map_err(|source| LeanRunnerError::Access {
        field: "lean.working_directory",
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(LeanRunnerError::NotADirectory {
            field: "lean.working_directory",
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn validate_program(path: &Path) -> Result<(), LeanRunnerError> {
    if !path.is_absolute() {
        return Err(LeanRunnerError::InvalidOption {
            field: "lean.program",
            reason: "must be an absolute executable path",
        });
    }
    let metadata = fs::metadata(path).map_err(|source| LeanRunnerError::Access {
        field: "lean.program",
        path: path.to_owned(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(LeanRunnerError::NotAFile {
            field: "lean.program",
            path: path.to_owned(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(LeanRunnerError::NotExecutable {
                field: "lean.program",
                path: path.to_owned(),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
struct CaptureBuffer {
    bytes: Vec<u8>,
    truncated: bool,
    error: Option<io::Error>,
}

fn spawn_capture<R: Read + Send + 'static>(
    mut reader: R,
    maximum: usize,
) -> thread::JoinHandle<CaptureBuffer> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
        let mut scratch = [0_u8; 8192];
        let mut truncated = false;
        let mut total_read = 0_usize;
        loop {
            match reader.read(&mut scratch) {
                Ok(0) => break,
                Ok(read) => {
                    total_read = total_read.saturating_add(read);
                    if bytes.len() < maximum {
                        let take = (maximum - bytes.len()).min(read);
                        bytes.extend_from_slice(&scratch[..take]);
                    }
                    truncated |= total_read > maximum;
                }
                Err(error) => {
                    return CaptureBuffer {
                        bytes,
                        truncated,
                        error: Some(error),
                    };
                }
            }
        }
        CaptureBuffer {
            bytes,
            truncated,
            error: None,
        }
    })
}

fn join_capture(
    handle: thread::JoinHandle<CaptureBuffer>,
    program: &Path,
) -> Result<CapturedOutput, LeanRunnerError> {
    let captured = handle.join().map_err(|_| LeanRunnerError::Capture {
        program: program.to_owned(),
        reason: "output reader panicked",
    })?;
    if captured.error.is_some() {
        return Err(LeanRunnerError::Capture {
            program: program.to_owned(),
            reason: "output reader failed",
        });
    }
    Ok(CapturedOutput {
        bytes: captured.bytes,
        truncated: captured.truncated,
    })
}

fn terminate_child(
    child: &mut Child,
    options: &LeanRunnerOptions,
) -> io::Result<(ExitStatus, bool)> {
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

#[cfg(windows)]
fn spawn_child(command: &mut Command) -> io::Result<Child> {
    command.group().kill_on_drop(true).spawn()
}

#[cfg(not(windows))]
fn spawn_child(command: &mut Command) -> io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

#[cfg(windows)]
fn child_stdio(child: &mut Child) -> &mut std::process::Child {
    child.inner()
}

#[cfg(not(windows))]
fn child_stdio(child: &mut Child) -> &mut std::process::Child {
    child
}

#[cfg(unix)]
fn request_interrupt(child: &mut Child) -> io::Result<()> {
    send_process_group_signal(child, nix::sys::signal::Signal::SIGINT)
}

#[cfg(not(unix))]
fn request_interrupt(child: &mut Child) -> io::Result<()> {
    child.kill()
}

#[cfg(unix)]
fn request_force_kill(child: &mut Child) -> io::Result<()> {
    send_process_group_signal(child, nix::sys::signal::Signal::SIGKILL)
}

#[cfg(not(unix))]
fn request_force_kill(child: &mut Child) -> io::Result<()> {
    child.kill()
}

#[cfg(unix)]
fn send_process_group_signal(
    child: &mut Child,
    signal: nix::sys::signal::Signal,
) -> io::Result<()> {
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    let process_group = Pid::from_raw(-(child.id() as i32));
    let signal_group = || match kill(process_group, signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error),
    };
    match signal_group() {
        Ok(()) => Ok(()),
        // macOS refuses to signal a group whose leader is an unreaped zombie.
        // Reap it, then signal whatever descendants are left in the group.
        Err(Errno::EPERM) if child.try_wait()?.is_some() => {
            signal_group().map_err(|error| io::Error::other(error.to_string()))
        }
        Err(error) => Err(io::Error::other(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, path::PathBuf, thread, time::Duration};

    use super::*;

    fn fixture(mode: &str, cwd: &Path) -> LeanCommand {
        LeanCommand::new(std::env::current_exe().unwrap(), cwd)
            .args(["--exact", "runner::tests::fixture_process", "--nocapture"])
            .env("MATHSCAPE_LEAN_RUNNER_FIXTURE", mode)
    }

    #[test]
    fn fixture_process() {
        let Ok(mode) = std::env::var("MATHSCAPE_LEAN_RUNNER_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "output" => {
                io::stderr().write_all(b"0123456789").unwrap();
                std::process::exit(7);
            }
            "wait" => thread::sleep(Duration::from_secs(10)),
            "descendant" => {
                let command = fixture("descendant_wait", &std::env::current_dir().unwrap());
                let mut child = Command::new(command.program())
                    .args(command.arguments())
                    .envs(command.environment())
                    .spawn()
                    .unwrap();
                child.wait().unwrap();
            }
            "descendant_wait" => {
                println!("descendant-ready");
                io::stdout().flush().unwrap();
                thread::sleep(Duration::from_secs(4));
            }
            _ => panic!("unknown fixture mode: {mode}"),
        }
    }

    #[test]
    fn captures_bounded_output_and_nonzero_status() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let options = LeanRunnerOptions::new(Duration::from_secs(2))
            .unwrap()
            .max_output_bytes(4)
            .unwrap();
        let run = LeanRunner::new(options)
            .run(&fixture("output", &cwd), &LeanCancellation::new())
            .unwrap();
        assert_eq!(run.status(), LeanRunStatus::Failed);
        assert_eq!(run.exit_code(), Some(7));
        assert_eq!(run.stderr().bytes(), b"0123");
        assert!(run.stderr().is_truncated());
    }

    #[test]
    fn timeout_terminates_the_private_process_group() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let options = LeanRunnerOptions::new(Duration::from_millis(80))
            .unwrap()
            .poll_interval(Duration::from_millis(5))
            .unwrap()
            .graceful_shutdown(Duration::from_millis(50));
        let run = LeanRunner::new(options)
            .run(&fixture("wait", &cwd), &LeanCancellation::new())
            .unwrap();
        assert_eq!(run.status(), LeanRunStatus::TimedOut);
        assert!(run.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_is_observed_while_running() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cancellation = LeanCancellation::new();
        let signal = cancellation.clone();
        let runner = LeanRunner::new(
            LeanRunnerOptions::new(Duration::from_secs(2))
                .unwrap()
                .poll_interval(Duration::from_millis(5))
                .unwrap(),
        );
        let command = fixture("wait", &cwd);
        let worker = thread::spawn(move || runner.run(&command, &signal));
        thread::sleep(Duration::from_millis(50));
        cancellation.cancel();
        let run = worker.join().unwrap().unwrap();
        assert_eq!(run.status(), LeanRunStatus::Cancelled);
    }

    #[test]
    fn timeout_terminates_descendants_holding_output_pipes() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let options = LeanRunnerOptions::new(Duration::from_millis(500))
            .unwrap()
            .poll_interval(Duration::from_millis(5))
            .unwrap()
            .graceful_shutdown(Duration::from_millis(50));
        let run = LeanRunner::new(options)
            .run(&fixture("descendant", &cwd), &LeanCancellation::new())
            .unwrap();
        assert_eq!(run.status(), LeanRunStatus::TimedOut);
        assert!(run.stdout().lossy().contains("descendant-ready"));
        assert!(run.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_terminates_descendants_holding_output_pipes() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cancellation = LeanCancellation::new();
        let signal = cancellation.clone();
        let command = fixture("descendant", &cwd);
        let runner = LeanRunner::new(LeanRunnerOptions::new(Duration::from_secs(2)).unwrap());
        let worker = thread::spawn(move || runner.run(&command, &signal));
        thread::sleep(Duration::from_millis(500));
        cancellation.cancel();
        let run = worker.join().unwrap().unwrap();
        assert_eq!(run.status(), LeanRunStatus::Cancelled);
        assert!(run.stdout().lossy().contains("descendant-ready"));
        assert!(run.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_before_spawn_has_empty_output() {
        let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cancellation = LeanCancellation::new();
        cancellation.cancel();
        let run = LeanRunner::default()
            .run(&fixture("output", &cwd), &cancellation)
            .unwrap();
        assert_eq!(run.status(), LeanRunStatus::Cancelled);
        assert_eq!(run.exit_code(), None);
        assert!(run.stdout().bytes().is_empty());
        assert!(run.stderr().bytes().is_empty());
        assert!(!run.stdout().is_truncated());
        assert!(!run.stderr().is_truncated());
    }
}
