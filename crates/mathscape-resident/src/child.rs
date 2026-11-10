use std::{
    collections::VecDeque,
    ffi::OsString,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

/// 单行 stdout 的上限。超过视为协议错乱。
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// 保留的 stderr 尾部行数，用于失败时的诊断。
const STDERR_TAIL_LINES: usize = 40;

/// 启动一个常驻子进程所需的 argv 描述。不是 shell 字符串：每个参数独立。
/// 子进程继承当前环境，再叠加 `env` 里的变量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub cwd: Option<PathBuf>,
}

impl ProcessSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadLineError {
    /// 在给定时间内没有读到完整的一行。
    Timeout,
    /// stdout 已关闭（子进程退出或崩溃）。
    Closed,
    /// 读取失败或单行超过上限。
    Io(String),
}

impl std::fmt::Display for ReadLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("timed out waiting for a line"),
            Self::Closed => f.write_str("stdout closed"),
            Self::Io(message) => write!(f, "stdout read failed: {message}"),
        }
    }
}

/// 一个运行中的常驻子进程。
///
/// Unix 上子进程以自己为组长（`process_group(0)`，等价于 setsid 的进程组
/// 部分），关闭时对整组发信号，所以它派生的孙进程也不会留下。drop 时一定
/// 整组 SIGKILL。
pub struct ResidentChild {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Result<String, String>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    exited: Option<ExitStatus>,
    group_killed: bool,
}

impl std::fmt::Debug for ResidentChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentChild")
            .field("pid", &self.child.id())
            .field("exited", &self.exited)
            .finish_non_exhaustive()
    }
}

impl ResidentChild {
    pub fn spawn(spec: &ProcessSpec) -> io::Result<Self> {
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            kill_group_now(&mut child);
            return Err(io::Error::other("child pipes were not created"));
        };
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || read_lines(stdout, &sender));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        {
            let tail = Arc::clone(&stderr_tail);
            thread::spawn(move || drain_stderr(stderr, &tail));
        }
        Ok(Self {
            child,
            stdin,
            lines,
            stderr_tail,
            exited: None,
            group_killed: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// 写一行（自动补换行）并 flush。
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin already closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    /// 读一行 stdout（不含换行），最多等 `timeout`。
    pub fn read_line(&mut self, timeout: Duration) -> Result<String, ReadLineError> {
        match self.lines.recv_timeout(timeout) {
            Ok(Ok(line)) => Ok(line),
            Ok(Err(message)) => Err(ReadLineError::Io(message)),
            Err(RecvTimeoutError::Timeout) => Err(ReadLineError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(ReadLineError::Closed),
        }
    }

    /// 读行直到 `accept` 返回 true，或超过 `deadline`。用于就绪握手。
    pub fn read_until(
        &mut self,
        timeout: Duration,
        mut accept: impl FnMut(&str) -> bool,
    ) -> Result<(), ReadLineError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ReadLineError::Timeout);
            }
            let line = self.read_line(remaining)?;
            if accept(&line) {
                return Ok(());
            }
        }
    }

    /// 子进程是否已经退出（不阻塞）。
    pub fn exit_status(&mut self) -> Option<ExitStatus> {
        if self.exited.is_none() {
            self.exited = self.child.try_wait().ok().flatten();
        }
        self.exited
    }

    /// 最近的 stderr 尾部，供诊断。
    pub fn stderr_tail(&self) -> String {
        let tail = self
            .stderr_tail
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    /// 优雅关闭：可选地先写一行告别消息（Loogle 是空行），关闭 stdin，
    /// 等 `grace`；仍未退出则对整组 SIGTERM，再等一小会，最后总是整组
    /// SIGKILL（孙进程可能忽略 TERM）。
    pub fn shutdown(&mut self, farewell: Option<&str>, grace: Duration) -> Option<ExitStatus> {
        if let Some(line) = farewell {
            let _ = self.write_line(line);
        }
        self.stdin = None;
        if self.wait_for(grace).is_none() {
            signal_group(&self.child, GroupSignal::Term);
            let _ = self.wait_for(grace.min(Duration::from_millis(500)));
        }
        self.kill()
    }

    /// 立即整组 SIGKILL 并回收。重复调用是空操作（组已杀过就不再发信号，
    /// 避免打到复用了同一 id 的无关进程组）。
    pub fn kill(&mut self) -> Option<ExitStatus> {
        self.stdin = None;
        if !self.group_killed {
            signal_group(&self.child, GroupSignal::Kill);
            self.group_killed = true;
        }
        self.reap()
    }

    fn wait_for(&mut self, duration: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + duration;
        loop {
            if let Some(status) = self.exit_status() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn reap(&mut self) -> Option<ExitStatus> {
        if self.exited.is_none() {
            let _ = self.child.kill();
            self.exited = self.child.wait().ok();
        }
        self.exited
    }
}

impl Drop for ResidentChild {
    fn drop(&mut self) {
        self.kill();
    }
}

fn read_lines(stdout: impl Read, sender: &mpsc::Sender<Result<String, String>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut buffer = Vec::new();
        let limit = (MAX_LINE_BYTES + 1) as u64;
        match reader.by_ref().take(limit).read_until(b'\n', &mut buffer) {
            Ok(0) => return,
            Ok(_) => {
                if buffer.last() == Some(&b'\n') {
                    buffer.pop();
                    if buffer.last() == Some(&b'\r') {
                        buffer.pop();
                    }
                } else if buffer.len() > MAX_LINE_BYTES {
                    let _ = sender.send(Err(format!("line exceeds {MAX_LINE_BYTES} bytes")));
                    return;
                }
                let line = String::from_utf8_lossy(&buffer).into_owned();
                if sender.send(Ok(line)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        }
    }
}

fn drain_stderr(stderr: impl Read, tail: &Mutex<VecDeque<String>>) {
    let reader = BufReader::new(stderr);
    for line in reader.split(b'\n') {
        let Ok(line) = line else { return };
        let mut text = String::from_utf8_lossy(&line).into_owned();
        text.truncate(text.floor_char_boundary(2048));
        let mut tail = tail.lock().unwrap_or_else(|poison| poison.into_inner());
        if tail.len() == STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(text);
    }
}

enum GroupSignal {
    Term,
    Kill,
}

#[cfg(unix)]
fn signal_group(child: &Child, signal: GroupSignal) {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    let signal = match signal {
        GroupSignal::Term => Signal::SIGTERM,
        GroupSignal::Kill => Signal::SIGKILL,
    };
    // 进程组 id = 组长 pid；组已经不存在时 ESRCH，忽略即可。
    let _ = kill(Pid::from_raw(-(child.id() as i32)), signal);
}

#[cfg(not(unix))]
fn signal_group(_child: &Child, _signal: GroupSignal) {}

fn kill_group_now(child: &mut Child) {
    signal_group(child, GroupSignal::Kill);
    let _ = child.kill();
    let _ = child.wait();
}
