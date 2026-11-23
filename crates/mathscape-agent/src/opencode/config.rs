use std::path::{Path, PathBuf};
use std::time::Duration;

use mathscape_resident::ProcessSpec;
use serde_json::{Value, json};

use super::approval::tool_names;
use crate::{SessionSpec, codex::approval::enabled_tools};

#[derive(Clone, Debug)]
pub struct OpenCodeConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub mcp_program: PathBuf,
    /// Mathscape-owned instructions, model selection and per-session round files.
    pub state_dir: PathBuf,
    pub handshake_timeout: Duration,
    pub interrupt_grace: Duration,
    /// No ACP progress for this long: cancel, then fail the round.
    pub no_progress_timeout: Duration,
    pub idle_timeout: Option<Duration>,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(std::env::temp_dir);
        Self {
            program: "opencode".into(),
            args: vec!["acp".into()],
            mcp_program: "mathscape".into(),
            state_dir: base.join("mathscape/opencode"),
            handshake_timeout: Duration::from_secs(20),
            interrupt_grace: Duration::from_secs(5),
            no_progress_timeout: Duration::from_secs(90),
            idle_timeout: Some(Duration::from_secs(15 * 60)),
        }
    }
}

pub(crate) fn process(
    config: &OpenCodeConfig,
    spec: &SessionSpec,
    dir: &Path,
    model: Option<&str>,
) -> Result<ProcessSpec, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let instructions = dir.join("instructions.md");
    let mut text = format!(
        "You are working in a Mathscape session. Use only these Mathscape MCP tools: {}. Native file reads are restricted to the workspace and explicitly granted roots. Native writes are restricted to scratch. Shell is disabled; per-command approval is not implemented yet. Network tools are disabled. Map changes must use Mathscape proposals accepted by the user.",
        enabled_tools(&spec.tools_allow).join(", ")
    );
    if let Some(scratch) = &spec.harness.scratch_dir {
        text.push_str(&format!(" Scratch: {}.", scratch.display()));
    }
    if let Some(extra) = &spec.harness.instructions {
        text.push_str("\n\n");
        text.push_str(extra);
    }
    std::fs::write(&instructions, text).map_err(|e| e.to_string())?;
    let mut permissions = serde_json::Map::new();
    for name in [
        "*",
        "read",
        "edit",
        "glob",
        "grep",
        "list",
        "bash",
        "external_directory",
    ] {
        permissions.insert(name.into(), json!("ask"));
    }
    for name in ["webfetch", "websearch", "mathscape_*"] {
        permissions.insert(name.into(), json!("deny"));
    }
    let names = tool_names(&spec.tools_allow);
    for tool in enabled_tools(&spec.tools_allow) {
        let title = format!("mathscape_{}", tool.replace('.', "_"));
        if names.contains_key(&title) {
            permissions.insert(title, json!("allow"));
        }
    }
    // ACP instructions add to the user's own configuration; they do not replace it.
    let mut content = json!({"permission":permissions,"instructions":[instructions]});
    if let Some(model) = model {
        content["model"] = json!(model);
    }
    let mut process = ProcessSpec::new(&config.program).cwd(&spec.workspace);
    for arg in &config.args {
        process = process.arg(arg);
    }
    for (key, value) in &spec.harness.env {
        process = process.env(key, value);
    }
    Ok(process.env("OPENCODE_CONFIG_CONTENT", content.to_string()))
}

pub(crate) fn session_params(
    config: &OpenCodeConfig,
    spec: &SessionSpec,
    round_file: &Path,
) -> Value {
    let env: Vec<Value> = spec
        .harness
        .env
        .iter()
        .map(|(name, value)| json!({"name":name,"value":value}))
        .collect();
    json!({"cwd":spec.workspace,"mcpServers":[{"name":"mathscape","command":config.mcp_program,"args":["mcp","--workspace",spec.workspace,"--round-file",round_file],"env":env}]})
}
