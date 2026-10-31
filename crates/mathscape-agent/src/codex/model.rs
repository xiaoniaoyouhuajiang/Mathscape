//! Local Codex model selection: read/list/write via `~/.codex` (or `$CODEX_HOME`).
//!
//! Mathscape does not invent a model catalog. [`list_models`] prefers the CLI's
//! `models_cache.json` (what the Codex picker itself uses). [`current_model`] /
//! [`set_model`] touch only the top-level `model = "…"` in `config.toml`, before
//! the first `[section]`, matching how the CLI stores the user default.

use std::path::{Path, PathBuf};

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

pub(crate) fn list_models(home: Option<&Path>) -> Result<Vec<AgentModel>, AgentError> {
    let Some(root) = codex_home(home) else {
        return Ok(Vec::new());
    };
    let current = read_default_model_from(&root.join("config.toml"));
    let mut models = read_models_cache(&root.join("models_cache.json"));
    if let Some(id) = current.as_ref() {
        if !models.iter().any(|model| model.id == *id) {
            models.insert(
                0,
                AgentModel {
                    id: id.clone(),
                    display_name: None,
                },
            );
        }
    }
    // Stable order: cache order, current already first if it was missing.
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
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(entries) = value.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries {
        let Some(id) = entry
            .get("slug")
            .or_else(|| entry.get("id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        // Skip catalog entries the Codex picker itself hides, unless we have
        // nothing else — visibility "list" is the default picker set.
        let visibility = entry
            .get("visibility")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        if visibility != "list" {
            continue;
        }
        let display_name = entry
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty() && *name != id)
            .map(str::to_owned);
        out.push(AgentModel {
            id: id.to_owned(),
            display_name,
        });
    }
    out
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
        let models = list_models(Some(&home)).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["custom-local", "gpt-a"]
        );
        assert_eq!(models[1].display_name.as_deref(), Some("A"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn set_model_rejects_empty() {
        let home = temp_home("empty");
        assert!(set_model(Some(&home), "  ").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }
}
