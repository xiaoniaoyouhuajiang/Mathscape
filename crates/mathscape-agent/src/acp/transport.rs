use std::time::{Duration, Instant};

use mathscape_resident::{
    EventSink, ProcessSpec, ReadLineError, ResidentChild, ResidentPolicy, Supervisor, SystemClock,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct Transport {
    supervisor: Supervisor<String>,
    next_id: u64,
    pid: Option<u32>,
}

impl Transport {
    pub(crate) fn new(
        name: String,
        timeout: Duration,
        grace: Duration,
        idle: Option<Duration>,
        sink: EventSink,
    ) -> Self {
        Self {
            supervisor: Supervisor::new(
                name,
                ResidentPolicy {
                    startup_timeout: timeout,
                    request_timeout: timeout,
                    idle_timeout: idle,
                    shutdown_grace: grace.min(Duration::from_secs(2)),
                    max_consecutive_failures: u32::MAX,
                    ..ResidentPolicy::default()
                },
                Arc::new(SystemClock),
                sink,
            ),
            next_id: 10,
            pid: None,
        }
    }

    pub(crate) fn ensure(&mut self, key: &str, process: ProcessSpec) -> Result<bool, String> {
        let child = self.supervisor.acquire(&key.to_owned(), || Ok(process), |child, timeout| {
            child.write_line(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"mathscape","version":env!("CARGO_PKG_VERSION")}
            }}).to_string()).map_err(|e|e.to_string())?;
            let deadline=Instant::now()+timeout;
            loop {
                if Instant::now()>=deadline {return Err("ACP initialize timed out".into());}
                let message=read(child, deadline.saturating_duration_since(Instant::now()))?.ok_or("ACP initialize timed out")?;
                if message.get("id")==Some(&json!(1)) && message.get("method").is_none() {
                    response(&message)?;
                    return Ok(());
                }
                reject(child, &message)?;
            }
        }).map_err(|e|e.to_string())?;
        let pid = child.pid();
        let changed = self.pid != Some(pid);
        if changed {
            self.next_id = 10;
            self.pid = Some(pid);
        }
        Ok(changed)
    }

    pub(crate) fn send(&mut self, message: &Value) -> Result<(), String> {
        self.supervisor
            .child_mut()
            .ok_or("ACP process is not running")?
            .write_line(&message.to_string())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn start(&mut self, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }

    pub(crate) fn poll(&mut self, timeout: Duration) -> Result<Option<Value>, String> {
        read(
            self.supervisor
                .child_mut()
                .ok_or("ACP process is not running")?,
            timeout,
        )
    }

    /// Handshake/load traffic is deliberately not forwarded to a round mapper.
    pub(crate) fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.start(method, params)?;
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                self.shutdown();
                return Err(format!("ACP {method} timed out"));
            }
            let Some(message) = self.poll(deadline.saturating_duration_since(Instant::now()))?
            else {
                self.shutdown();
                return Err(format!("ACP {method} timed out"));
            };
            if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
                return response(&message);
            }
            if message.get("method").is_some() && message.get("id").is_some() {
                self.reject(&message)?;
            }
        }
    }

    pub(crate) fn reject(&mut self, message: &Value) -> Result<(), String> {
        reject(
            self.supervisor
                .child_mut()
                .ok_or("ACP process is not running")?,
            message,
        )
    }

    pub(crate) fn touch(&mut self) {
        self.supervisor.touch();
    }
    pub(crate) fn reap_idle(&mut self) -> bool {
        if self.supervisor.reap_idle() {
            self.pid = None;
            true
        } else {
            false
        }
    }
    pub(crate) fn shutdown(&mut self) {
        self.supervisor.shutdown();
        self.pid = None;
    }
}

pub(crate) fn response(message: &Value) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        return Err(format!("ACP request failed: {error}"));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| "ACP response has neither result nor error".into())
}

fn read(child: &mut ResidentChild, timeout: Duration) -> Result<Option<Value>, String> {
    match child.read_line(timeout) {
        Ok(line) if line.trim().is_empty() => Ok(None),
        Ok(line) => serde_json::from_str(&line)
            .map(Some)
            .map_err(|e| format!("invalid ACP JSON: {e}")),
        Err(ReadLineError::Timeout) => Ok(None),
        Err(error) => Err(format!("{error}: {}", child.stderr_tail())),
    }
}

fn reject(child: &mut ResidentChild, message: &Value) -> Result<(), String> {
    let (Some(id), Some(method)) = (
        message.get("id"),
        message.get("method").and_then(Value::as_str),
    ) else {
        return Ok(());
    };
    let reply = if method == "session/request_permission" {
        let option = message
            .pointer("/params/options")
            .and_then(Value::as_array)
            .and_then(|options| options.iter().find(|v| v["kind"] == "reject_once"));
        let outcome = option
            .map(|o| json!({"outcome":"selected","optionId":o["optionId"]}))
            .unwrap_or(json!({"outcome":"cancelled"}));
        json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}})
    } else {
        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"unsupported ACP client method"}})
    };
    child
        .write_line(&reply.to_string())
        .map_err(|e| e.to_string())
}
