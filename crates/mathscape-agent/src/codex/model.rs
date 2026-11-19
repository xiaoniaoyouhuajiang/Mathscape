//! Local Codex model selection: read/list/write via `~/.codex` (or `$CODEX_HOME`).
//!
//! Mathscape does not invent a model catalog. [`list_models`] prefers the CLI's
//! `models_cache.json` (what the Codex picker itself uses) and, when that is
//! missing or unreadable, asks `codex app-server` via `model/list`. [`current_model`] /
//! [`set_model`] touch only the top-level `model = "…"` in `config.toml`, before
//! the first `[section]`, matching how the CLI stores the user default.
//!
//! Reasoning effort follows the same shape: the CLI persists the user's choice
//! as top-level `model_reasoning_effort = "…"` next to `model`, and every
//! catalog entry advertises `supported_reasoning_levels[].effort` plus a
//! `default_reasoning_level` (app-server: `supportedReasoningEfforts[]
//! .reasoningEffort` / `defaultReasoningEffort`). [`effective_effort`] resolves
//! the configured value against the current model's entry; the session forwards
//! the result as `effort` on `turn/start`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{AgentEffort, AgentError, AgentModel};

const MODEL_KEY: &str = "model";
const EFFORT_KEY: &str = "model_reasoning_effort";

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

/// Raw top-level `model_reasoning_effort` from `config.toml`, if set.
pub(crate) fn configured_effort(home: Option<&Path>) -> Option<String> {
    let path = codex_home(home)?.join("config.toml");
    read_top_level_string(&path, EFFORT_KEY)
}

/// Catalog from `models_cache.json` only (no app-server spawn). Used on the
/// round hot path where spawning a second Codex just to read a list is wasteful.
pub(crate) fn cached_catalog(home: Option<&Path>) -> Vec<AgentModel> {
    codex_home(home)
        .map(|root| read_on_disk_catalog(&root))
        .unwrap_or_default()
}

const CATALOG_OVERRIDE_KEY: &str = "model_catalog_json";

/// On-disk catalog in the order Codex itself resolves it: a top-level
/// `model_catalog_json = "…"` in `config.toml` (a user/gateway supplied list
/// that replaces the fetched one) wins over the CLI's `models_cache.json`.
fn read_on_disk_catalog(root: &Path) -> Vec<AgentModel> {
    if let Some(path) = catalog_override_path(root) {
        let models = read_models_cache(&path);
        if !models.is_empty() {
            return models;
        }
    }
    read_models_cache(&root.join("models_cache.json"))
}

/// `model_catalog_json` resolved to an absolute path (`~/` expanded, relative
/// paths taken from the Codex home).
fn catalog_override_path(root: &Path) -> Option<PathBuf> {
    let raw = read_top_level_string(&root.join("config.toml"), CATALOG_OVERRIDE_KEY)?;
    let path = if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(&raw)
    };
    Some(if path.is_absolute() { path } else { root.join(path) })
}

/// Effort the next round will use for `model`, given the configured value and
/// the catalog:
/// - configured and advertised by the model (or the model advertises no list) → configured
/// - configured but not advertised → the model's catalog default
/// - not configured → the model's catalog default
pub(crate) fn effective_effort(
    configured: Option<&str>,
    model: Option<&str>,
    catalog: &[AgentModel],
) -> Option<String> {
    let configured = configured.map(str::trim).filter(|e| !e.is_empty());
    let entry = model.and_then(|id| catalog.iter().find(|m| m.id == id));
    match (configured, entry) {
        (Some(effort), Some(entry)) => {
            if entry.reasoning_efforts.is_empty()
                || entry.reasoning_efforts.iter().any(|e| e.id == effort)
            {
                Some(effort.to_owned())
            } else {
                entry
                    .default_reasoning_effort
                    .clone()
                    .or_else(|| Some(effort.to_owned()))
            }
        }
        (Some(effort), None) => Some(effort.to_owned()),
        (None, Some(entry)) => entry.default_reasoning_effort.clone(),
        (None, None) => None,
    }
}

/// Efforts the catalog advertises for `model` (empty when unknown).
pub(crate) fn efforts_for(model: &str, catalog: &[AgentModel]) -> Vec<AgentEffort> {
    catalog
        .iter()
        .find(|m| m.id == model)
        .map(|m| m.reasoning_efforts.clone())
        .unwrap_or_default()
}

/// Persist `effort` as top-level `model_reasoning_effort`. When the current
/// model's catalog entry lists efforts, `effort` must be one of them.
pub(crate) fn set_effort(
    home: Option<&Path>,
    effort: &str,
    catalog: &[AgentModel],
) -> Result<(), AgentError> {
    let effort = effort.trim();
    if effort.is_empty()
        || effort
            .chars()
            .any(|c| c.is_control() || c == '"' || c == '\\')
    {
        return Err(internal(
            "reasoning effort must be a non-empty printable string",
        ));
    }
    if let Some(model) = current_model(home) {
        let allowed = efforts_for(&model, catalog);
        if !allowed.is_empty() && !allowed.iter().any(|e| e.id == effort) {
            let list = allowed
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(internal(format!(
                "model {model} does not support reasoning effort {effort} (supported: {list})"
            )));
        }
    }
    let path = ensure_config_path(home)?;
    write_top_level_string(&path, EFFORT_KEY, effort)
}

fn internal(message: impl Into<String>) -> AgentError {
    AgentError::Harness {
        backend: crate::AgentBackend::Codex,
        code: crate::ErrorCode::Internal,
        message: message.into(),
    }
}

fn ensure_config_path(home: Option<&Path>) -> Result<PathBuf, AgentError> {
    let root = codex_home(home)
        .ok_or_else(|| internal("cannot locate Codex home (set CODEX_HOME or HOME)"))?;
    std::fs::create_dir_all(&root)
        .map_err(|error| internal(format!("cannot create {}: {error}", root.display())))?;
    Ok(root.join("config.toml"))
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
        .map(|root| read_on_disk_catalog(root))
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
                ..AgentModel::default()
            },
        );
    }
    // Stable order: catalog order, current already first if it was missing.
    Ok(models)
}

/// Persist `id` as the top-level `model`. Like the Codex picker, which always
/// stores a (model, effort) pair, a configured `model_reasoning_effort` the new
/// model does not advertise is reset to that model's catalog default so the
/// CLI and the next round never carry an effort the model rejects.
pub(crate) fn set_model(
    home: Option<&Path>,
    id: &str,
    catalog: &[AgentModel],
) -> Result<(), AgentError> {
    let id = id.trim();
    if id.is_empty() || id.chars().any(char::is_control) {
        return Err(internal("model id must be a non-empty printable string"));
    }
    let path = ensure_config_path(home)?;
    write_top_level_string(&path, MODEL_KEY, id)?;
    if let Some(configured) = read_top_level_string(&path, EFFORT_KEY) {
        let allowed = efforts_for(id, catalog);
        if !allowed.is_empty()
            && !allowed.iter().any(|e| e.id == configured)
            && let Some(default) = catalog
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.default_reasoning_effort.clone())
        {
            write_top_level_string(&path, EFFORT_KEY, &default)?;
        }
    }
    Ok(())
}

fn read_default_model_from(path: &Path) -> Option<String> {
    read_top_level_string(path, MODEL_KEY)
}

/// Top-level `key = "value"` before the first `[section]`.
fn read_top_level_string(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            break;
        }
        let Some(rest) = assignment_rhs(trimmed, key) else {
            continue;
        };
        let rest = rest.trim();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or(""),
            _ => rest.split('#').next().unwrap_or("").trim(),
        };
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// `Some(rhs)` when `trimmed` is `key = rhs` for exactly `key`.
fn assignment_rhs<'a>(trimmed: &'a str, key: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix(key)?.trim_start();
    rest.strip_prefix('=')
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
        let (reasoning_efforts, default_reasoning_effort) = parse_efforts(entry);
        out.push(AgentModel {
            id: id.to_owned(),
            display_name,
            reasoning_efforts,
            default_reasoning_effort,
        });
    }
    out
}

/// `supported_reasoning_levels[].effort` / `default_reasoning_level` (cache) or
/// `supportedReasoningEfforts[].reasoningEffort` / `defaultReasoningEffort`
/// (app-server). Duplicate efforts keep the first occurrence.
fn parse_efforts(entry: &Value) -> (Vec<AgentEffort>, Option<String>) {
    let mut efforts: Vec<AgentEffort> = Vec::new();
    let list = ["supported_reasoning_levels", "supportedReasoningEfforts"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_array));
    for level in list.into_iter().flatten() {
        let id = level
            .as_str()
            .or_else(|| {
                ["effort", "reasoningEffort", "reasoning_effort"]
                    .iter()
                    .find_map(|key| level.get(*key).and_then(Value::as_str))
            })
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let Some(id) = id else { continue };
        if efforts.iter().any(|e| e.id == id) {
            continue;
        }
        let description = level
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned);
        efforts.push(AgentEffort {
            id: id.to_owned(),
            description,
        });
    }
    let default = ["default_reasoning_level", "defaultReasoningEffort"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_owned);
    (efforts, default)
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

/// Replace (or insert) top-level `key = "value"` before the first `[section]`,
/// leaving every other line untouched.
fn write_top_level_string(path: &Path, key: &str, value: &str) -> Result<(), AgentError> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let assignment = format!("{key} = \"{}\"\n", escape_toml_basic(value));
    let mut out = String::new();
    let mut replaced = false;
    let mut past_header = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if !past_header {
            if trimmed.starts_with('[') {
                if !replaced {
                    // Keep the new key with the other top-level keys (no blank
                    // line between), then one blank line before the section.
                    while out.ends_with("\n\n") {
                        out.pop();
                    }
                    out.push_str(&assignment);
                    out.push('\n');
                    replaced = true;
                }
                past_header = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if assignment_rhs(trimmed, key).is_some() {
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
        out.push_str(&assignment);
    }
    std::fs::write(path, out)
        .map_err(|error| internal(format!("failed to write {}: {error}", path.display())))?;
    Ok(())
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
        set_model(Some(&home), "gpt-new", &[]).unwrap();
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
    fn model_catalog_json_override_wins_over_models_cache() {
        let home = temp_home("override");
        std::fs::write(home.join("models_cache.json"), REAL_SHAPE_CACHE).unwrap();
        std::fs::write(
            home.join("gateway-models.json"),
            r#"{"models":[
              {"slug":"m-one","display_name":"M-One","visibility":"list"},
              {"slug":"google/g-four","display_name":"G Four · Google","visibility":"list",
               "supported_reasoning_levels":[{"effort":"low"},{"effort":"ultra"}]},
              {"slug":"cursor/c-five","display_name":"C Five · Cursor","visibility":"list"},
              {"slug":"m-reserve","visibility":"hide"}
            ]}"#,
        )
        .unwrap();
        // Relative path → resolved against the Codex home.
        std::fs::write(
            home.join("config.toml"),
            "model = \"m-one\"\nmodel_catalog_json = \"gateway-models.json\"\n\n[x]\nmodel_catalog_json = \"ignored.json\"\n",
        )
        .unwrap();
        let models = list_models(Some(&home), None).unwrap();
        assert_eq!(ids(&models), ["m-one", "google/g-four", "cursor/c-five"]);
        assert_eq!(
            ids(&cached_catalog(Some(&home))),
            ["m-one", "google/g-four", "cursor/c-five"]
        );
        assert_eq!(efforts_for("google/g-four", &models).len(), 2);

        // Absolute path works too; a missing/broken override falls back to the cache.
        let abs = home.join("gateway-models.json");
        std::fs::write(
            home.join("config.toml"),
            format!("model_catalog_json = \"{}\"\n", abs.display()),
        )
        .unwrap();
        assert_eq!(list_models(Some(&home), None).unwrap().len(), 3);
        std::fs::write(home.join("config.toml"), "model_catalog_json = \"nope.json\"\n").unwrap();
        assert_eq!(
            ids(&list_models(Some(&home), None).unwrap()),
            ["m-one", "m-two", "group/m-three"]
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
        assert!(set_model(Some(&home), "  ", &[]).is_err());
        assert!(set_effort(Some(&home), "  ", &[]).is_err());
        assert!(set_effort(Some(&home), "a\"b", &[]).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Same layout as the real cache (Codex 0.166): `supported_reasoning_levels`
    /// objects with `effort` + `description`, plus `default_reasoning_level`.
    const EFFORT_CACHE: &str = r#"{"models":[
      {"slug":"m-sol","display_name":"Sol","visibility":"list","default_reasoning_level":"low",
       "supported_reasoning_levels":[
         {"effort":"low","description":"Fast"},{"effort":"medium","description":"Bal"},
         {"effort":"high","description":"Deep"},{"effort":"xhigh","description":"X"},
         {"effort":"max","description":"Max"},{"effort":"ultra","description":"Ultra"}]},
      {"slug":"m-luna","display_name":"Luna","visibility":"list","default_reasoning_level":"medium",
       "supported_reasoning_levels":[
         {"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"},{"effort":"max"},{"effort":"low"}]},
      {"slug":"m-plain","visibility":"list"}
    ]}"#;

    fn effort_ids(efforts: &[AgentEffort]) -> Vec<&str> {
        efforts.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn catalog_parses_reasoning_levels_from_cache_and_app_server() {
        let cache: Value = serde_json::from_str(EFFORT_CACHE).unwrap();
        let models = parse_model_catalog(&cache);
        assert_eq!(
            effort_ids(&models[0].reasoning_efforts),
            ["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("low"));
        assert_eq!(
            models[0].reasoning_efforts[0].description.as_deref(),
            Some("Fast")
        );
        assert_eq!(
            effort_ids(&models[1].reasoning_efforts),
            ["low", "medium", "high", "xhigh", "max"],
            "duplicates dropped"
        );
        assert!(models[2].reasoning_efforts.is_empty());
        assert_eq!(models[2].default_reasoning_effort, None);

        let v2: Value = serde_json::from_str(
            r#"{"data":[{"id":"a","model":"a","hidden":false,"defaultReasoningEffort":"medium",
                "supportedReasoningEfforts":[{"reasoningEffort":"low","description":"L"},
                                             {"reasoningEffort":"medium","description":"M"}]}]}"#,
        )
        .unwrap();
        let models = parse_model_catalog(&v2);
        assert_eq!(effort_ids(&models[0].reasoning_efforts), ["low", "medium"]);
        assert_eq!(
            models[0].default_reasoning_effort.as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn effective_effort_resolves_against_model_entry() {
        let cache: Value = serde_json::from_str(EFFORT_CACHE).unwrap();
        let catalog = parse_model_catalog(&cache);
        // configured + supported
        assert_eq!(
            effective_effort(Some("max"), Some("m-sol"), &catalog).as_deref(),
            Some("max")
        );
        // configured but unsupported by this model → model default
        assert_eq!(
            effective_effort(Some("ultra"), Some("m-luna"), &catalog).as_deref(),
            Some("medium")
        );
        // not configured → model default
        assert_eq!(
            effective_effort(None, Some("m-sol"), &catalog).as_deref(),
            Some("low")
        );
        // model advertises nothing / unknown model → configured passes through
        assert_eq!(
            effective_effort(Some("high"), Some("m-plain"), &catalog).as_deref(),
            Some("high")
        );
        assert_eq!(
            effective_effort(Some("high"), Some("unknown"), &catalog).as_deref(),
            Some("high")
        );
        assert_eq!(effective_effort(None, Some("m-plain"), &catalog), None);
        assert_eq!(effective_effort(None, None, &catalog), None);
    }

    #[test]
    fn set_effort_validates_and_persists_like_codex() {
        let home = temp_home("effort");
        std::fs::write(
            home.join("config.toml"),
            "model = \"m-luna\"\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n",
        )
        .unwrap();
        std::fs::write(home.join("models_cache.json"), EFFORT_CACHE).unwrap();
        let catalog = cached_catalog(Some(&home));
        assert!(
            set_effort(Some(&home), "ultra", &catalog).is_err(),
            "luna has no ultra"
        );
        assert_eq!(configured_effort(Some(&home)), None);
        set_effort(Some(&home), "xhigh", &catalog).unwrap();
        let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert_eq!(
            text,
            "model = \"m-luna\"\nmodel_reasoning_effort = \"xhigh\"\n\n[projects.\"/x\"]\ntrust_level = \"trusted\"\n"
        );
        set_effort(Some(&home), "high", &catalog).unwrap();
        assert_eq!(configured_effort(Some(&home)).as_deref(), Some("high"));
        assert_eq!(current_model(Some(&home)).as_deref(), Some("m-luna"));
        assert!(
            std::fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("trust_level = \"trusted\"")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_model_resets_effort_the_new_model_does_not_support() {
        let home = temp_home("model-effort");
        std::fs::write(
            home.join("config.toml"),
            "model = \"m-sol\"\nmodel_reasoning_effort = \"ultra\"\n",
        )
        .unwrap();
        std::fs::write(home.join("models_cache.json"), EFFORT_CACHE).unwrap();
        let catalog = cached_catalog(Some(&home));
        set_model(Some(&home), "m-luna", &catalog).unwrap();
        assert_eq!(configured_effort(Some(&home)).as_deref(), Some("medium"));
        // supported efforts survive a model switch
        set_effort(Some(&home), "high", &catalog).unwrap();
        set_model(Some(&home), "m-sol", &catalog).unwrap();
        assert_eq!(configured_effort(Some(&home)).as_deref(), Some("high"));
        // a model with no advertised list keeps whatever is configured
        set_model(Some(&home), "m-plain", &catalog).unwrap();
        assert_eq!(configured_effort(Some(&home)).as_deref(), Some("high"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn effort_key_does_not_shadow_model_key() {
        let home = temp_home("keys");
        std::fs::write(
            home.join("config.toml"),
            "model_reasoning_effort = 'max' # inline\nmodel = \"m\"\n",
        )
        .unwrap();
        assert_eq!(current_model(Some(&home)).as_deref(), Some("m"));
        assert_eq!(configured_effort(Some(&home)).as_deref(), Some("max"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
