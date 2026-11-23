//! Scripted ACP peer for adapter tests; no model or external service.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

struct Peer<W> {
    writer: W,
    log: Option<PathBuf>,
}
impl<W: Write> Peer<W> {
    fn log(&self, message: &Value) -> io::Result<()> {
        if let Some(path) = &self.log {
            writeln!(
                OpenOptions::new().create(true).append(true).open(path)?,
                "{message}"
            )?;
        }
        Ok(())
    }
    fn send(&mut self, message: Value) -> io::Result<()> {
        writeln!(self.writer, "{message}")?;
        self.writer.flush()
    }
    fn result(&mut self, id: &Value, result: Value) -> io::Result<()> {
        self.send(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
    fn update(&mut self, session: &str, update: Value) -> io::Result<()> {
        self.send(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":update}}))
    }
    fn text(&mut self, session: &str, text: &str) -> io::Result<()> {
        self.update(
            session,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}),
        )
    }
    fn tool(
        &mut self,
        session: &str,
        id: &str,
        title: &str,
        status: &str,
        business: &str,
    ) -> io::Result<()> {
        self.update(session,json!({"sessionUpdate":"tool_call","toolCallId":id,"title":title,"kind":"other","status":"pending","rawInput":{"query":"fixture"}}))?;
        self.update(session,json!({"sessionUpdate":"tool_call_update","toolCallId":id,"status":status,"rawOutput":{"output":json!({"status":business,"data":{}}).to_string()}}))
    }
}
fn catalog(model: &str) -> Value {
    json!({"configOptions":[{"id":"model","currentValue":model,"options":[{"value":"fixture/a","name":"Fixture A"},{"value":"fixture/b","name":"Fixture B"}]}]})
}

struct Operation {
    id: String,
    kind: &'static str,
    path: Option<PathBuf>,
    title: &'static str,
}
fn operations(cwd: &Path) -> Vec<Operation> {
    [
        ("scratch", "edit", Some(cwd.join("scratch/ok.txt")), "write"),
        ("workspace", "edit", Some(cwd.join("no.txt")), "write"),
        (
            "outside-write",
            "edit",
            Some(cwd.parent().unwrap().join("outside/no.txt")),
            "write",
        ),
        (
            "link-read",
            "read",
            Some(cwd.join("scratch/link/sentinel")),
            "read",
        ),
        (
            "link-write",
            "edit",
            Some(cwd.join("scratch/link/no.txt")),
            "write",
        ),
        ("bash", "execute", None, "bash"),
        (
            "outside-read",
            "read",
            Some(cwd.parent().unwrap().join("outside/sentinel")),
            "read",
        ),
        ("no-path", "read", None, "read"),
        ("mcp-allowed", "other", None, "mathscape_ink_page_image"),
        ("mcp-denied", "other", None, "mathscape_artifact_stage"),
    ]
    .into_iter()
    .map(|(id, kind, path, title)| Operation {
        id: id.into(),
        kind,
        path,
        title,
    })
    .collect()
}

/// Modes: normal, permissions, hold, ignore-cancel, quota, rpc-error,
/// bad-initialize. Load always emits replay; `missing` fails to load.
pub fn serve_acp_fixture(
    mut reader: impl BufRead,
    writer: impl Write,
    mode: &str,
    log: Option<&Path>,
) -> io::Result<()> {
    let mut peer = Peer {
        writer,
        log: log.map(Path::to_path_buf),
    };
    let config: Value = serde_json::from_str(
        &std::env::var("OPENCODE_CONFIG_CONTENT").unwrap_or_else(|_| "{}".into()),
    )
    .unwrap_or_default();
    peer.log(&json!({"fixture":"startup","pid":std::process::id(),"config":config,"forwarded":std::env::var("MATHSCAPE_ACP_TEST_ENV").ok()}))?;
    let mut model = config["model"].as_str().unwrap_or("fixture/a").to_owned();
    let mut sessions = HashMap::new();
    let mut n = 0;
    let mut pending: Option<(Value, String)> = None;
    let mut permissions: HashMap<String, Operation> = HashMap::new();
    for line in (&mut reader).lines() {
        let message: Value = serde_json::from_str(&line?).map_err(io::Error::other)?;
        peer.log(&message)?;
        let id = &message["id"];
        let params = &message["params"];
        match message["method"].as_str() {
            Some("initialize") => {
                if mode == "bad-initialize" {
                    peer.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":"fixture initialization failure"}}))?;
                } else {
                    peer.result(id,json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true},"agentInfo":{"name":"fixture","version":"1"}}))?;
                }
            }
            Some("session/new") => {
                n += 1;
                let session = format!("fixture-{}-{n}", std::process::id());
                sessions.insert(
                    session.clone(),
                    PathBuf::from(params["cwd"].as_str().unwrap()),
                );
                let mut result = catalog(&model);
                result["sessionId"] = json!(session);
                peer.result(id, result)?;
            }
            Some("session/load") => {
                let session = params["sessionId"].as_str().unwrap();
                if session == "missing" {
                    peer.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"session not found"}}))?;
                    continue;
                }
                sessions.insert(
                    session.into(),
                    PathBuf::from(params["cwd"].as_str().unwrap()),
                );
                peer.text(session, "OLD REPLAY MUST NOT LEAK")?;
                peer.tool(
                    session,
                    "old",
                    "mathscape_workspace_search",
                    "completed",
                    "ok",
                )?;
                peer.result(id, catalog(&model))?;
            }
            Some("session/set_config_option") => {
                model = params["value"].as_str().unwrap().into();
                peer.result(id, catalog(&model))?;
            }
            Some("session/prompt") => {
                let session = params["sessionId"].as_str().unwrap();
                let cwd = sessions.get(session).unwrap();
                if mode == "quota" {
                    peer.result(id,json!({"stopReason":"end_turn","usage":{"inputTokens":0,"outputTokens":0,"totalTokens":0}}))?;
                } else if mode == "rpc-error" {
                    peer.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"fixture provider error"}}))?;
                } else if matches!(mode, "hold" | "ignore-cancel") {
                    pending = Some((id.clone(), session.into()));
                } else if mode == "permissions" {
                    pending = Some((id.clone(), session.into()));
                    for op in operations(cwd) {
                        let input = if let Some(path) = &op.path {
                            json!({"filepath":path})
                        } else if op.kind == "execute" {
                            json!({"command":"echo denied"})
                        } else {
                            json!({})
                        };
                        let locations = op
                            .path
                            .as_ref()
                            .map(|path| json!([{"path":path}]))
                            .unwrap_or_else(|| json!([]));
                        let tool = json!({"toolCallId":op.id,"kind":op.kind,"title":op.title,"status":"pending","rawInput":input,"locations":locations});
                        let mut update = tool.clone();
                        update["sessionUpdate"] = json!("tool_call");
                        peer.update(session, update)?;
                        peer.send(json!({"jsonrpc":"2.0","id":op.id,"method":"session/request_permission","params":{"sessionId":session,"toolCall":tool,"options":[{"kind":"allow_once","optionId":"custom-yes"},{"kind":"allow_always","optionId":"never-this"},{"kind":"reject_once","optionId":"custom-no"}]}}))?;
                        permissions.insert(op.id.clone(), op);
                    }
                } else {
                    peer.tool(
                        session,
                        "search",
                        "mathscape_workspace_search",
                        "completed",
                        "ok",
                    )?;
                    peer.tool(
                        session,
                        "image",
                        "mathscape_ink_page_image",
                        "completed",
                        "not_found",
                    )?;
                    peer.tool(session, "read", "mathscape_material_read", "failed", "ok")?;
                    peer.text(session, "fixture ")?;
                    peer.text(session, "reply")?;
                    peer.result(id,json!({"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":3,"cachedReadTokens":2}}))?;
                }
            }
            Some("session/cancel") => {
                if mode != "ignore-cancel"
                    && let Some((id, _)) = pending.take()
                {
                    peer.result(&id,json!({"stopReason":"cancelled","usage":{"inputTokens":0,"outputTokens":0}}))?;
                }
            }
            None => {
                if let Some(op) = id.as_str().and_then(|id| permissions.remove(id)) {
                    let allowed = message
                        .pointer("/result/outcome/optionId")
                        .and_then(Value::as_str)
                        == Some("custom-yes");
                    let mut output = "denied".to_owned();
                    if allowed && let Some(path) = op.path {
                        if op.kind == "edit" {
                            fs::write(path, "SCRATCH_OK")?;
                            output = "written".into();
                        } else {
                            output = fs::read_to_string(path)?;
                        }
                    }
                    let session = &pending.as_ref().unwrap().1;
                    peer.update(session,json!({"sessionUpdate":"tool_call_update","toolCallId":op.id,"status":if allowed {"completed"} else {"failed"},"rawOutput":{"output":output}}))?;
                    if permissions.is_empty() {
                        let (id, session) = pending.take().unwrap();
                        peer.text(&session, "permissions checked")?;
                        peer.result(&id,json!({"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":3}}))?;
                    }
                }
            }
            _ => {
                if !id.is_null() {
                    peer.send(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unknown fixture method"}}))?;
                }
            }
        }
    }
    Ok(())
}
