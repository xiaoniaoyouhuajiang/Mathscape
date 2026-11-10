//! Local socket between the long-lived desktop process (which owns the
//! resident Loogle) and the per-round `mathscape mcp` server.
//!
//! Codex app-server starts a fresh `mathscape mcp` for every round and kills
//! it with the round, so a resident Loogle inside the MCP process would reload
//! the 5.5GB index each round. The desktop therefore hosts the
//! [`ResidentLoogle`] behind a Unix socket in a private (0700) directory and
//! passes the path to the round as [`LOOGLE_SOCKET_ENV`]. The MCP server sends
//! one JSON line and reads one JSON line per query; when the socket is absent
//! or unreachable, or the host answers [`LoogleIpcResponse::Fallback`], it runs
//! the existing one-shot path instead.
//!
//! Trust boundary: only the current user can reach the socket. The host still
//! re-validates the [`LoogleTarget`] (absolute paths, existing executables,
//! index inside the managed profile cache) and never rebuilds an index.

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    LeanLoogleError, LoogleSearchResult, LoogleTarget, ResidentLoogle, ResidentLoogleOutcome,
};

/// Socket path handed to the MCP child.
pub const LOOGLE_SOCKET_ENV: &str = "MATHSCAPE_LOOGLE_SOCKET";
/// `0` / `false` / `off` forces resident mode off regardless of the setting.
pub const LOOGLE_RESIDENT_ENV: &str = "MATHSCAPE_LOOGLE_RESIDENT";
pub const LOOGLE_IPC_VERSION: u32 = 1;

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
const REAP_TICK: Duration = Duration::from_secs(30);

/// Whether `MATHSCAPE_LOOGLE_RESIDENT` turns resident mode off.
pub fn resident_forced_off() -> bool {
    std::env::var(LOOGLE_RESIDENT_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        )
    })
}

/// Which backend answered a `mathlib.search`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoogleBackendMode {
    Resident,
    Oneshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoogleIpcRequest {
    pub v: u32,
    pub target: LoogleTarget,
    pub query: String,
    pub module: String,
    pub max_results: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LoogleIpcResponse {
    Ok {
        result: LoogleSearchResult,
        index_fingerprint: String,
    },
    /// Resident mode is off or has fallen back; use the one-shot path.
    Fallback {
        reason: String,
    },
    Error {
        error: LoogleIpcError,
    },
}

/// [`LeanLoogleError`] in a form that survives the socket.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoogleIpcError {
    InvalidQuery {
        message: String,
    },
    QueryRejected {
        message: String,
        suggestions: Vec<String>,
    },
    Unavailable {
        reason: String,
    },
    TimedOut {
        stage: String,
        seconds: u64,
    },
    Failed {
        status: String,
        message: String,
    },
}

impl From<LeanLoogleError> for LoogleIpcError {
    fn from(error: LeanLoogleError) -> Self {
        match error {
            LeanLoogleError::InvalidQuery(message) => Self::InvalidQuery { message },
            LeanLoogleError::QueryRejected {
                message,
                suggestions,
            } => Self::QueryRejected {
                message,
                suggestions,
            },
            LeanLoogleError::CapabilityUnavailable { reason } => Self::Unavailable {
                reason: reason.to_owned(),
            },
            LeanLoogleError::ResidentUnavailable { reason } => Self::Unavailable { reason },
            LeanLoogleError::TimedOut { stage, seconds } => Self::TimedOut { stage, seconds },
            LeanLoogleError::RunFailed { status, stderr } => Self::Failed {
                status,
                message: stderr,
            },
            other => Self::Failed {
                status: "failed".to_owned(),
                message: other.to_string(),
            },
        }
    }
}

impl From<LoogleIpcError> for LeanLoogleError {
    fn from(error: LoogleIpcError) -> Self {
        match error {
            LoogleIpcError::InvalidQuery { message } => Self::InvalidQuery(message),
            LoogleIpcError::QueryRejected {
                message,
                suggestions,
            } => Self::QueryRejected {
                message,
                suggestions,
            },
            LoogleIpcError::Unavailable { reason } => Self::ResidentUnavailable { reason },
            LoogleIpcError::TimedOut { stage, seconds } => Self::TimedOut { stage, seconds },
            LoogleIpcError::Failed { status, message } => Self::RunFailed {
                status,
                stderr: message,
            },
        }
    }
}

/// One request/response exchange with the desktop host. `timeout` bounds the
/// whole wait, including queueing behind other queries and a cold start.
pub fn request_over_socket(
    path: &Path,
    request: &LoogleIpcRequest,
    timeout: Duration,
) -> io::Result<LoogleIpcResponse> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    let mut reply = Vec::new();
    BufReader::new(stream.take(MAX_RESPONSE_BYTES)).read_until(b'\n', &mut reply)?;
    if reply.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "resident Loogle host closed the connection without a reply",
        ));
    }
    serde_json::from_slice(&reply).map_err(io::Error::other)
}

/// `<tmp>/mathscape-<pid>/loogle.sock`. Short enough for the macOS 104-byte
/// `sun_path` limit.
pub fn default_socket_path() -> PathBuf {
    std::env::temp_dir()
        .join(format!("mathscape-{}", std::process::id()))
        .join("loogle.sock")
}

struct HostInner {
    loogle: Mutex<ResidentLoogle>,
    socket_path: PathBuf,
    stopped: AtomicBool,
    forced_off: bool,
}

/// The desktop side: owns the resident Loogle and serves the socket.
#[derive(Clone)]
pub struct LoogleResidentHost {
    inner: Arc<HostInner>,
}

impl std::fmt::Debug for LoogleResidentHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoogleResidentHost")
            .field("socket_path", &self.inner.socket_path)
            .finish_non_exhaustive()
    }
}

impl LoogleResidentHost {
    /// Binds `socket_path` (its parent is created with mode 0700; a stale
    /// socket file left by a crashed run is replaced) and starts the accept
    /// loop and the idle reaper. `forced_off` pins resident mode off (the
    /// [`LOOGLE_RESIDENT_ENV`] override) so the setting cannot turn it on.
    pub fn start(
        socket_path: PathBuf,
        mut loogle: ResidentLoogle,
        forced_off: bool,
    ) -> io::Result<Self> {
        if let Some(parent) = socket_path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
        if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", socket_path.display()),
                ));
            }
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        if forced_off {
            loogle.set_enabled(false);
        }
        let inner = Arc::new(HostInner {
            loogle: Mutex::new(loogle),
            socket_path,
            stopped: AtomicBool::new(false),
            forced_off,
        });
        {
            let weak = Arc::downgrade(&inner);
            thread::Builder::new()
                .name("loogle-resident-accept".to_owned())
                .spawn(move || accept_loop(&listener, &weak))?;
        }
        {
            let weak = Arc::downgrade(&inner);
            thread::Builder::new()
                .name("loogle-resident-reaper".to_owned())
                .spawn(move || reap_loop(&weak))?;
        }
        Ok(Self { inner })
    }

    pub fn socket_path(&self) -> &Path {
        &self.inner.socket_path
    }

    pub fn is_forced_off(&self) -> bool {
        self.inner.forced_off
    }

    /// Applies the「Loogle 常驻模式」setting. Turning it off stops the process now.
    pub fn set_enabled(&self, enabled: bool) {
        let enabled = enabled && !self.inner.forced_off;
        lock(&self.inner.loogle).set_enabled(enabled);
    }

    pub fn handle(&self, request: LoogleIpcRequest) -> LoogleIpcResponse {
        handle(&self.inner, request)
    }

    /// Stops the resident process, the accept loop and removes the socket.
    /// Called on app exit; idempotent.
    pub fn shutdown(&self) {
        if self.inner.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        // Wake the blocking accept so the thread sees `stopped`.
        let _ = UnixStream::connect(&self.inner.socket_path);
        lock(&self.inner.loogle).shutdown();
        let _ = fs::remove_file(&self.inner.socket_path);
    }
}

impl Drop for HostInner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

fn lock(loogle: &Mutex<ResidentLoogle>) -> std::sync::MutexGuard<'_, ResidentLoogle> {
    loogle.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn handle(inner: &HostInner, request: LoogleIpcRequest) -> LoogleIpcResponse {
    if request.v != LOOGLE_IPC_VERSION {
        return LoogleIpcResponse::Fallback {
            reason: format!("unsupported resident protocol version {}", request.v),
        };
    }
    if inner.stopped.load(Ordering::Acquire) {
        return LoogleIpcResponse::Fallback {
            reason: "resident host is shutting down".to_owned(),
        };
    }
    // The lock is the serial queue: Loogle answers one query at a time.
    let mut loogle = lock(&inner.loogle);
    match loogle.search(
        &request.target,
        &request.query,
        &request.module,
        request.max_results,
    ) {
        Ok(ResidentLoogleOutcome::Done(done)) => LoogleIpcResponse::Ok {
            result: done.result,
            index_fingerprint: done.index_fingerprint,
        },
        Ok(ResidentLoogleOutcome::Fallback { reason }) => LoogleIpcResponse::Fallback { reason },
        Err(error) => LoogleIpcResponse::Error {
            error: error.into(),
        },
    }
}

fn accept_loop(listener: &UnixListener, inner: &Weak<HostInner>) {
    for stream in listener.incoming() {
        let Some(strong) = inner.upgrade() else {
            return;
        };
        if strong.stopped.load(Ordering::Acquire) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let weak = Arc::downgrade(&strong);
        drop(strong);
        let _ = thread::Builder::new()
            .name("loogle-resident-conn".to_owned())
            .spawn(move || serve_connection(stream, &weak));
    }
}

fn serve_connection(stream: UnixStream, inner: &Weak<HostInner>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let mut line = Vec::new();
    if BufReader::new((&stream).take(MAX_REQUEST_BYTES))
        .read_until(b'\n', &mut line)
        .is_err()
        || line.last() != Some(&b'\n')
    {
        return;
    }
    let response = match serde_json::from_slice::<LoogleIpcRequest>(&line) {
        Ok(request) => match inner.upgrade() {
            Some(inner) => handle(&inner, request),
            None => return,
        },
        Err(error) => LoogleIpcResponse::Error {
            error: LoogleIpcError::InvalidQuery {
                message: format!("malformed resident request: {error}"),
            },
        },
    };
    let Ok(mut bytes) = serde_json::to_vec(&response) else {
        return;
    };
    bytes.push(b'\n');
    let mut writer = writer;
    let _ = writer.write_all(&bytes);
    let _ = writer.flush();
}

fn reap_loop(inner: &Weak<HostInner>) {
    let step = Duration::from_millis(500);
    let mut waited = Duration::ZERO;
    loop {
        thread::sleep(step);
        let Some(strong) = inner.upgrade() else {
            return;
        };
        if strong.stopped.load(Ordering::Acquire) {
            return;
        }
        waited += step;
        if waited < REAP_TICK {
            continue;
        }
        waited = Duration::ZERO;
        // Do not queue behind a running query: try again next tick.
        if let Ok(mut loogle) = strong.loogle.try_lock() {
            loogle.reap_idle();
        }
    }
}
