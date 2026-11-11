//! `resident-processes.jsonl` in the app log directory
//! (`~/Library/Logs/com.mathscape.desktop/` on macOS): one line per lifecycle
//! event of a resident child process (`mathscape.resident_event.v0`). The
//! resident Loogle (`instance: "loogle"`) and every sandbox session's
//! `codex app-server` (`instance: "codex-app-server:<sandbox_session_id>"`,
//! its `mathscape mcp` lives and dies with it) write here.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use mathscape_resident::{EventSink, ResidentEvent, null_sink};
use serde_json::json;

pub(crate) const RESIDENT_LOG_FILE: &str = "resident-processes.jsonl";
pub(crate) const RESIDENT_EVENT_SCHEMA: &str = "mathscape.resident_event.v0";

/// `<log_dir>/resident-processes.jsonl`.
pub(crate) fn log_file(log_dir: Option<&Path>) -> Option<PathBuf> {
    log_dir.map(|dir| dir.join(RESIDENT_LOG_FILE))
}

/// Appends every event to `log_file`; no file ⇒ events are dropped.
pub(crate) fn jsonl_sink(log_file: Option<PathBuf>) -> EventSink {
    let Some(path) = log_file else {
        return null_sink();
    };
    Arc::new(move |event: &ResidentEvent| {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or_default();
        let line = json!({
            "schema": RESIDENT_EVENT_SCHEMA,
            "at_ms": at_ms,
            "event": event,
        });
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // One `write` per line so concurrent writers do not interleave.
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    })
}

#[cfg(test)]
mod tests {
    use mathscape_resident::ResidentEventKind;
    use serde_json::Value;

    use super::*;

    #[test]
    fn events_are_appended_as_schema_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_file(Some(dir.path())).unwrap();
        let sink = jsonl_sink(Some(path.clone()));
        sink(&ResidentEvent {
            instance: "codex-app-server:s1".into(),
            kind: ResidentEventKind::Spawn,
            pid: Some(7),
            key: Some("s1#abc".into()),
            detail: None,
            elapsed_ms: None,
            consecutive_failures: 0,
        });
        let lines: Vec<Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["schema"], RESIDENT_EVENT_SCHEMA);
        assert_eq!(lines[0]["event"]["instance"], "codex-app-server:s1");
        assert_eq!(lines[0]["event"]["kind"], "spawn");
        assert!(lines[0]["at_ms"].as_u64().unwrap() > 0);
    }
}
