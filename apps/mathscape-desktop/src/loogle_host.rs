//! App-wide resident Loogle (docs/lean-tool-contract.md §3.4).
//!
//! `mathlib.search` runs in the per-round `mathscape mcp` process, which Codex
//! starts and kills with every round, so the resident Loogle lives here in the
//! desktop process instead. [`start`] binds a private Unix socket; each sandbox
//! round gets its path as `MATHSCAPE_LOOGLE_SOCKET` ([`round_env`]), forwarded
//! to the MCP child. The process itself starts lazily on the first query.
//!
//! The「Loogle 常驻模式」workspace setting toggles it ([`set_enabled`]);
//! `MATHSCAPE_LOOGLE_RESIDENT=0` pins it off. Lifecycle events are appended
//! to `resident-processes.jsonl` in the app log directory.

use std::path::Path;

use serde::Serialize;

#[cfg(unix)]
mod imp {
    use std::{
        fs::OpenOptions,
        io::Write,
        path::{Path, PathBuf},
        sync::{Arc, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    use mathscape_resident::{EventSink, ResidentEvent};
    use mathscape_tool_lean::{
        LoogleResidentHost, ResidentLoogle, default_socket_path, resident_forced_off,
    };
    use serde_json::json;

    static HOST: OnceLock<LoogleResidentHost> = OnceLock::new();

    pub(super) fn start(log_file: Option<PathBuf>, enabled: bool) -> Result<(), String> {
        if HOST.get().is_some() {
            return Ok(());
        }
        let loogle = ResidentLoogle::new(event_sink(log_file));
        let host = LoogleResidentHost::start(default_socket_path(), loogle, resident_forced_off())
            .map_err(|error| format!("failed to start the resident Loogle socket: {error}"))?;
        host.set_enabled(enabled);
        let _ = HOST.set(host);
        Ok(())
    }

    pub(super) fn host() -> Option<&'static LoogleResidentHost> {
        HOST.get()
    }

    pub(super) fn socket_path() -> Option<&'static Path> {
        HOST.get().map(LoogleResidentHost::socket_path)
    }

    fn event_sink(log_file: Option<PathBuf>) -> EventSink {
        Arc::new(move |event: &ResidentEvent| {
            let Some(path) = &log_file else { return };
            let at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or_default();
            let line = json!({
                "schema": "mathscape.resident_event.v0",
                "at_ms": at_ms,
                "event": event,
            });
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{line}");
            }
        })
    }
}

#[cfg(not(unix))]
mod imp {
    use std::path::{Path, PathBuf};

    pub(super) struct Host;

    impl Host {
        pub(super) fn set_enabled(&self, _enabled: bool) {}
        pub(super) fn shutdown(&self) {}
        pub(super) fn is_forced_off(&self) -> bool {
            true
        }
    }

    pub(super) fn start(_log_file: Option<PathBuf>, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    pub(super) fn host() -> Option<&'static Host> {
        None
    }

    pub(super) fn socket_path() -> Option<&'static Path> {
        None
    }
}

/// Starts the socket (not Loogle itself) once per app run.
pub(crate) fn start(log_dir: Option<&Path>, enabled: bool) -> Result<(), String> {
    imp::start(
        log_dir.map(|dir| dir.join("resident-processes.jsonl")),
        enabled,
    )
}

/// Applies the「Loogle 常驻模式」setting; off stops the process immediately.
pub(crate) fn set_enabled(enabled: bool) {
    if let Some(host) = imp::host() {
        host.set_enabled(enabled);
    }
}

/// What the settings panel shows next to the「Loogle 常驻模式」switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LoogleResidentStatus {
    /// The socket is up (Unix only); otherwise mathlib.search is always one-shot.
    available: bool,
    /// `MATHSCAPE_LOOGLE_RESIDENT=0` pins resident mode off.
    forced_off: bool,
}

pub(crate) fn status() -> LoogleResidentStatus {
    LoogleResidentStatus {
        available: imp::host().is_some(),
        forced_off: imp::host().is_some_and(|host| host.is_forced_off()),
    }
}

#[tauri::command]
pub(crate) fn get_loogle_resident_status() -> LoogleResidentStatus {
    status()
}

/// Stops Loogle and removes the socket; called on app exit.
pub(crate) fn shutdown() {
    if let Some(host) = imp::host() {
        host.shutdown();
    }
}

/// The environment entry for one sandbox round, when the host is running.
pub(crate) fn round_env() -> Option<(String, String)> {
    imp::socket_path().map(|path| {
        (
            mathscape_tool_lean_socket_env().to_owned(),
            path.display().to_string(),
        )
    })
}

#[cfg(unix)]
fn mathscape_tool_lean_socket_env() -> &'static str {
    mathscape_tool_lean::LOOGLE_SOCKET_ENV
}

#[cfg(not(unix))]
fn mathscape_tool_lean_socket_env() -> &'static str {
    "MATHSCAPE_LOOGLE_SOCKET"
}
