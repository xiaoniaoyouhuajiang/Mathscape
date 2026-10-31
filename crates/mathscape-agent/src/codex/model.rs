//! Local Codex model selection: read/list/write via `~/.codex` (or `$CODEX_HOME`).
//!
//! Mathscape does not invent a model catalog. [`list_models`] prefers the CLI's
//! `models_cache.json` (what the Codex picker itself uses) and, when that is
//! missing or unreadable, asks `codex app-server` via `model/list`. [`current_model`] /
//! [`set_model`] touch only the top-level `model = "…"` in `config.toml`, before
//! the first `[section]`, matching how the CLI stores the user default.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{AgentError, AgentModel};

/// Directory Codex uses for `config.toml` / `models_cache.json`.
pub(crate) fn codex_home(override_home: Option<&Path>) -> Option<PathBuf> {
    if let Some(home) = override_home {
        return Some(home.to_path_buf());
    }
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".codex"))
}

pub(crate) fn current_model(home: Option<&Path>) -> Option<String> {
    let path = codex_home(home)?.join("config.toml");
    read_default_model_from(&path)
}

/// Where [`list_models`] may ask a live Codex for its picker list when the
/// on-disk cache yields nothing (missing, unreadable or unknown shape).
pub(crate) struct AppServerProbe<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub timeout: Duration,
}

pub(crate) fn list_models(
    home: Option<&Path>,
    probe: Option<AppServerProbe<'_>>,
) -> Result<Vec<AgentModel>, AgentError> {
    let root = codex_home(home);
    let current = root
        .as_ref()
        .and_then(|root| read_default_model_from(&root.join("config.toml")));
    let mut models = root
        .as_ref()
        .map(|root| read_models_cache(&root.join("models_cache.json")))
        .unwrap_or_default();
    if models.is_empty() {
        // Second discovery path: the same `model/list` the Codex TUI picker uses.
        if let Some(probe) = probe {
            models = list_models_via_app_server(&probe, home).unwrap_or_default();
        }
    }
    if let Some(id) = current.as_ref()
        && !models.iter().any(|model| model.id == *id)
    {
        models.insert(
            0,
            AgentModel {
                id: id.clone(),
                display_name: None,
            },
        );
    }
    // Stable order: catalog order, current already first if it was missing.
    Ok(models)
}

pub(crate) fn set_model(home: Option<&Path>, id: &str) -> Result<(), AgentError> {
    let id = id.trim();
    if id.is_empty() || id.chars().any(char::is_control) {
        return Err(AgentError::Harness {
            backend: crate::AgentBackend::Codex,
            code: crate::ErrorCode::Internal,
            message: "model id must be a non-empty printable string".into(),
        });
    }
    let root = codex_home(home).ok_or_else(|| AgentError::Harness {
        backend: crate::AgentBackend::Codex,
        code: crate::ErrorCode::Internal,
        message: "cannot locate Codex home (set CODEX_HOME or HOME)".into(),
    })?;
    std::fs::create_dir_all(&root).map_err(|error| AgentError::Harness {
        backend: crate::AgentBackend::Codex,
        code: crate::ErrorCode::Internal,
        message: format!("cannot create {}: {error}", root.display()),
    })?;
    let path = root.join("config.toml");
    write_default_model(&path, id)
}

fn read_default_model_from(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            break;
        }
        let Some(rest) = trimmed.strip_prefix("model") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches(|c| c == '"' || c == '\'');
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

fn read_models_cache(path: &Path) -> Vec<AgentModel> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    parse_model_catalog(&value)
}

/// Accepts every catalog shape Codex has used:
/// - `models_cache.json`: `{"models":[{"slug","display_name","visibility"}]}`
/// - app-server `model/list`: `{"data":[{"id","model","displayName","hidden"}]}`
/// - a bare array of either entry shape.
///
/// Entries the Codex picker hides (`visibility` other than `"list"`, or
/// `hidden: true`) are skipped. Duplicate ids keep the first occurrence.
pub(crate) fn parse_model_catalog(value: &Value) -> Vec<AgentModel> {
    let entries = value
        .as_array()
        .or_else(|| value.get("models").and_then(Value::as_array))
        .or_else(|| value.get("data").and_then(Value::as_array))
        .or_else(|| value.pointer("/result/data").and_then(Value::as_array));
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut out: Vec<AgentModel> = Vec::new();
    for entry in entries {
        let Some(id) = ["slug", "model", "id"]
            .iter()
            .filter_map(|key| entry.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .find(|id| !id.is_empty())
        else {
            continue;
        };
        let visibility = entry
            .get("visibility")
            .and_then(Value::as_str)
            .map(|v| v.trim().to_ascii_lowercase());
        if visibility.as_deref().is_some_and(|v| v != "list") {
            continue;
        }
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if out.iter().any(|model| model.id == id) {
            continue;
        }
        let display_name = ["display_name", "displayName"]
            .iter()
            .filter_map(|key| entry.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .find(|name| !name.is_empty())
            .filter(|name| *name != id)
            .map(str::to_owned);
        out.push(AgentModel {
            id: id.to_owned(),
            display_name,
        });
    }
    out
}

/// Spawn `codex app-server`, run `initialize` + `model/list`, and stop it.
/// Best effort: any failure (no binary, not logged in, timeout) yields `None`.
fn list_models_via_app_server(
    probe: &AppServerProbe<'_>,
    home: Option<&Path>,
) -> Option<Vec<AgentModel>> {
    let mut command = Command::new(probe.program);
    command
        .args(probe.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(home) = home {
        command.env("CODEX_HOME", home);
    }
    let mut child = ChildGuard(command.spawn().ok()?);
    let mut stdin = child.0.stdin.take()?;
    let stdout = child.0.stdout.take()?;
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(value) = serde_json::from_str::<Value>(line.trim())
                && tx.send(value).is_err()
            {
                break;
            }
        }
    });
    let deadline = Instant::now() + probe.timeout;
    let send = |stdin: &mut std::process::ChildStdin, value: Value| -> Option<()> {
        let mut line = serde_json::to_string(&value).ok()?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).ok()?;
        stdin.flush().ok()
    };
    send(
        &mut stdin,
        json!({
            "method": "initialize",
            "id": 1,
            "params": { "clientInfo": {
                "name": "mathscape", "title": "Mathscape", "version": env!("CARGO_PKG_VERSION")
            }}
        }),
    )?;
    wait_for_response(&rx, 1, deadline)?;
    send(&mut stdin, json!({ "method": "initialized" }))?;
    let mut models = Vec::new();
    let mut cursor: Option<String> = None;
    for page in 0..8u64 {
        let id = 2 + page;
        let mut params = json!({ "includeHidden": false });
        if let Some(cursor) = cursor.as_ref() {
            params["cursor"] = json!(cursor);
        }
        send(
            &mut stdin,
            json!({ "method": "model/list", "id": id, "params": params }),
        )?;
        let response = wait_for_response(&rx, id, deadline)?;
        let result = response.get("result")?;
        for model in parse_model_catalog(result) {
            if !models.iter().any(|m: &AgentModel| m.id == model.id) {
                models.push(model);
            }
        }
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    Some(models)
}

fn wait_for_response(rx: &mpsc::Receiver<Value>, id: u64, deadline: Instant) -> Option<Value> {
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let message = rx.recv_timeout(left).ok()?;
        if message.get("id").and_then(Value::as_u64) == Some(id) {
            return message.get("error").is_none().then_some(message);
        }
    }
}

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn write_default_model(path: &Path, id: &str) -> Result<(), AgentError> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let assignment = format!("model = \"{}\"\n", escape_toml_basic(id));
    let mut out = String::new();
    let mut replaced = false;
    let mut past_header = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if !past_header {
            if trimmed.starts_with('[') {
                if !replaced {
                    out.push_str(&assignment);
                    if !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                    replaced = true;
                }
                past_header = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if is_top_level_model_assignment(trimmed) {
                if !replaced {
                    out.push_str(&assignment);
                    replaced = true;
                }
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !replaced {
        out = if out.is_empty() {
            assignment
        } else {
            format!("{assignment}\n{out}")
        };
    }
    std::fs::write(path, out).map_err(|error| AgentError::Harness {
        backend: crate::AgentBackend::Codex,
        code: crate::ErrorCode::Internal,
        message: format!("failed to write {}: {error}", path.display()),
    })?;
    Ok(())
}

fn is_top_level_model_assignment(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("model") else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with('=')
}

fn escape_toml_basic(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_home(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mathscape-codex-model-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn read_skips_sections_and_comments() {
        let home = temp_home("read");
        std::fs::write(
            home.join("config.toml"),
            "# comment\nmodel = \"gpt-test\"\n\n[projects.\"/x\"]\nmodel = \"ignored\"\n",
        )
        .unwrap();
        assert_eq!(current_model(Some(&home)).as_deref(), Some("gpt-test"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_model_rewrites_top_level_and_preserves_sections() {
        let home = temp_home("set");
        std::fs::write(
            home.join("config.toml"),
            "model_reasoning_effort = \"max\"\nmodel = \"old\"\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n",
        )
        .unwrap();
        set_model(Some(&home), "gpt-new").unwrap();
        let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("model = \"gpt-new\""));
        assert!(!text.contains("model = \"old\""));
        assert!(text.contains("model_reasoning_effort = \"max\""));
        assert!(text.contains("[projects.\"/x\"]"));
        assert!(text.contains("trust_level = \"trusted\""));
        assert_eq!(current_model(Some(&home)).as_deref(), Some("gpt-new"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn list_models_reads_cache_and_injects_current() {
        let home = temp_home("list");
        std::fs::write(home.join("config.toml"), "model = \"custom-local\"\n").unwrap();
        std::fs::write(
            home.join("models_cache.json"),
            r#"{"models":[
              {"slug":"gpt-a","display_name":"A","visibility":"list"},
              {"slug":"gpt-hidden","display_name":"H","visibility":"hidden"}
            ]}"#,
        )
        .unwrap();
        let models = list_models(Some(&home), None).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["custom-local", "gpt-a"]
        );
        assert_eq!(models[1].display_name.as_deref(), Some("A"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Mirrors the real `models_cache.json` layout (Codex 0.166): top-level
    /// metadata, `models[]` with `slug` / `display_name` / `visibility` plus
    /// many unrelated fields. Ids here are placeholders.
    const REAL_SHAPE_CACHE: &str = r#"{
      "fetched_at": "2026-10-31T06:46:00Z", "etag": "W/\"x\"", "client_version": "0.166.0",
      "identity": {"kind": "chatgpt"},
      "models": [
        {"slug":"m-one","display_name":"M-One","visibility":"list","priority":1,
         "shell_type":"unified_exec","supported_in_api":true,"context_window":1000,
         "supported_reasoning_levels":[{"effort":"low"}],"model_messages":{"x":"y"}},
        {"slug":"m-reserve","display_name":"M-Reserve","visibility":"hide","priority":4},
        {"slug":"m-two","display_name":"M Two","visibility":"list","priority":2},
        {"slug":"group/m-three","display_name":"M Three · routing group","visibility":"list"},
        {"slug":"m-auto-review","display_name":"Auto Review","visibility":"hide"},
        {"slug":"m-one","display_name":"dup","visibility":"list"}
      ]
    }"#;

    fn ids(models: &[AgentModel]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn real_cache_shape_lists_every_visible_model() {
        let home = temp_home("real");
        std::fs::write(
            home.join("config.toml"),
            "model = \"m-two\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .unwrap();
        std::fs::write(home.join("models_cache.json"), REAL_SHAPE_CACHE).unwrap();
        let models = list_models(Some(&home), None).unwrap();
        assert_eq!(ids(&models), ["m-one", "m-two", "group/m-three"]);
        assert_eq!(
            models[2].display_name.as_deref(),
            Some("M Three · routing group")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn app_server_and_bare_array_shapes_parse() {
        let v2: Value = serde_json::from_str(
            r#"{"data":[{"id":"a","model":"a","displayName":"A","hidden":false},
                        {"id":"h","model":"h","displayName":"H","hidden":true},
                        {"id":"b","model":"b","displayName":"b","hidden":false}],
                "nextCursor":null}"#,
        )
        .unwrap();
        let models = parse_model_catalog(&v2);
        assert_eq!(ids(&models), ["a", "b"]);
        assert_eq!(models[0].display_name.as_deref(), Some("A"));
        assert_eq!(models[1].display_name, None, "same as id → no label");

        let bare: Value =
            serde_json::from_str(r#"[{"id":"x"},{"slug":"y","visibility":"LIST"},{"slug":"z","visibility":"none"},{"foo":1}]"#)
                .unwrap();
        assert_eq!(ids(&parse_model_catalog(&bare)), ["x", "y"]);
        assert!(parse_model_catalog(&serde_json::json!({"unexpected": []})).is_empty());
    }

    #[test]
    fn missing_or_broken_cache_yields_only_current() {
        let home = temp_home("missing");
        std::fs::write(home.join("config.toml"), "model = \"only-current\"\n").unwrap();
        assert_eq!(
            ids(&list_models(Some(&home), None).unwrap()),
            ["only-current"]
        );
        std::fs::write(home.join("models_cache.json"), "{not json").unwrap();
        assert_eq!(
            ids(&list_models(Some(&home), None).unwrap()),
            ["only-current"]
        );
        std::fs::write(home.join("models_cache.json"), r#"{"models":[]}"#).unwrap();
        assert_eq!(
            ids(&list_models(Some(&home), None).unwrap()),
            ["only-current"]
        );
        std::fs::remove_file(home.join("config.toml")).unwrap();
        assert!(list_models(Some(&home), None).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unreachable_app_server_falls_back_quietly() {
        let home = temp_home("noserver");
        std::fs::write(home.join("config.toml"), "model = \"cur\"\n").unwrap();
        let program = home.join("no-such-codex");
        let models = list_models(
            Some(&home),
            Some(AppServerProbe {
                program: &program,
                args: &[],
                timeout: Duration::from_secs(1),
            }),
        )
        .unwrap();
        assert_eq!(ids(&models), ["cur"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_model_rejects_empty() {
        let home = temp_home("empty");
        assert!(set_model(Some(&home), "  ").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
