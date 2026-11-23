//! herdr's socket API: newline-delimited JSON over the Unix socket named by
//! `HERDR_SOCKET_PATH`. The pane lists agents once and then subscribes to
//! `pane.agent_status_changed`, so it never asks herdr on a timer.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use crate::Msg;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: String,
    pub agent: String,
    pub status: String,
    pub title: String,
    /// The Claude Code session id herdr's own hook reported for the pane.
    pub session: String,
    /// The agent's working folder, where its commits are looked up.
    pub cwd: String,
}

pub fn socket_path() -> Result<PathBuf, String> {
    std::env::var_os("HERDR_SOCKET_PATH")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HERDR_SOCKET_PATH is not set: open this pane from herdr".to_string())
}

fn request(method: &str, params: Value) -> Result<Value, String> {
    let mut stream =
        UnixStream::connect(socket_path()?).map_err(|e| format!("herdr socket: {e}"))?;
    let body = json!({"id": "ghostcto", "method": method, "params": params});
    writeln!(stream, "{body}").map_err(|e| format!("herdr socket: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("herdr socket: {e}"))?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| format!("herdr reply: {e}"))?;
    match reply.get("error") {
        Some(err) => Err(format!("herdr {method}: {err}")),
        None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
    }
}

pub fn parse_agents(result: &Value) -> Vec<Agent> {
    let text = |a: &Value, k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    result
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .map(|a| Agent {
                    pane_id: text(a, "pane_id"),
                    agent: text(a, "agent"),
                    status: text(a, "agent_status"),
                    title: text(a, "terminal_title_stripped"),
                    session: a
                        .pointer("/agent_session/value")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cwd: text(a, "cwd"),
                })
                .filter(|a| !a.pane_id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Claude Code agents herdr knows about. Only Claude Code sessions run the
/// GhostCTO hook, so other agents have no ghosts to show.
pub fn claude_agents() -> Result<Vec<Agent>, String> {
    let result = request("agent.list", json!({}))?;
    Ok(parse_agents(&result)
        .into_iter()
        .filter(|a| a.agent == "claude")
        .collect())
}

/// One line of the event stream, as the pane needs it.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamItem {
    Status { pane: String, status: String },
    Lost,
    Error(String),
    Other,
}

pub fn parse_stream_line(line: &str) -> StreamItem {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return StreamItem::Other;
    };
    if let Some(err) = value.get("error") {
        return match err.get("code").and_then(Value::as_str) {
            Some("events_lost") => StreamItem::Lost,
            _ => StreamItem::Error(format!(
                "herdr events: {}",
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
            )),
        };
    }
    if value.get("event").and_then(Value::as_str) != Some("pane.agent_status_changed") {
        return StreamItem::Other;
    }
    let field = |k: &str| {
        value
            .pointer(&format!("/data/{k}"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    match (field("pane_id"), field("agent_status")) {
        (Some(pane), Some(status)) => StreamItem::Status { pane, status },
        _ => StreamItem::Other,
    }
}

/// A live subscription to agent status changes. Dropping it closes the
/// socket and ends its reader thread without reporting an error.
pub struct Subscription {
    stream: UnixStream,
    closed: Arc<AtomicBool>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// Subscribes to `pane.agent_status_changed` for each pane (herdr requires a
/// pane per subscription) and forwards changes until the subscription is
/// dropped or herdr closes the socket.
pub fn subscribe(panes: &[String], tx: Sender<Msg>) -> Result<Subscription, String> {
    let mut stream =
        UnixStream::connect(socket_path()?).map_err(|e| format!("herdr socket: {e}"))?;
    let subscriptions: Vec<Value> = panes
        .iter()
        .map(|pane| json!({"type": "pane.agent_status_changed", "pane_id": pane}))
        .collect();
    let body = json!({"id": "ghostcto-events", "method": "events.subscribe", "params": {"subscriptions": subscriptions}});
    writeln!(stream, "{body}").map_err(|e| format!("herdr socket: {e}"))?;
    let reader = stream
        .try_clone()
        .map_err(|e| format!("herdr socket: {e}"))?;
    let closed = Arc::new(AtomicBool::new(false));
    let thread_closed = closed.clone();
    std::thread::spawn(move || {
        let mut reason = "herdr closed the event stream".to_string();
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            let msg = match parse_stream_line(&line) {
                StreamItem::Status { pane, status } => Msg::Status { pane, status },
                StreamItem::Lost => Msg::Resync,
                StreamItem::Error(message) => {
                    reason = message;
                    break;
                }
                StreamItem::Other => continue,
            };
            if tx.send(msg).is_err() {
                return;
            }
        }
        if !thread_closed.load(Ordering::SeqCst) {
            let _ = tx.send(Msg::HerdrGone(reason));
        }
    });
    Ok(Subscription { stream, closed })
}

#[derive(Debug, Default, serde::Deserialize)]
struct PluginContext {
    #[serde(default)]
    focused_pane_id: String,
    #[serde(default)]
    workspace_id: String,
}

/// The action: open the ghost pane as a split beside the focused pane and
/// tell it which pane to watch.
pub fn open_pane() -> Result<(), String> {
    let raw = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").map_err(|_| {
        "HERDR_PLUGIN_CONTEXT_JSON is not set: run this from the herdr action".to_string()
    })?;
    let ctx: PluginContext =
        serde_json::from_str(&raw).map_err(|e| format!("HERDR_PLUGIN_CONTEXT_JSON: {e}"))?;
    let herdr = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let mut cmd = Command::new(herdr);
    cmd.args([
        "plugin",
        "pane",
        "open",
        "--plugin",
        "ghostcto",
        "--entrypoint",
        "ghosts",
    ]);
    // The pane opens without focus so keys meant for the agent stay there.
    cmd.args(["--placement", "split", "--direction", "right", "--no-focus"]);
    if !ctx.focused_pane_id.is_empty() {
        cmd.args(["--target-pane", &ctx.focused_pane_id]);
        cmd.args([
            "--env",
            &format!("GHOSTCTO_WATCH_PANE={}", ctx.focused_pane_id),
        ]);
    } else if !ctx.workspace_id.is_empty() {
        cmd.args(["--workspace", &ctx.workspace_id]);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("could not run herdr: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("herdr plugin pane open exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_list() {
        let result = json!({"agents": [
            {"agent": "claude", "agent_status": "blocked", "pane_id": "w9:p1", "terminal_title_stripped": "Ghostcto", "agent_session": {"value": "abc"}},
            {"agent": "codex", "agent_status": "idle", "pane_id": "w2:p1"},
            {"agent": "claude"}
        ]});
        let agents = parse_agents(&result);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].status, "blocked");
        assert_eq!(agents[0].title, "Ghostcto");
        assert_eq!(agents[0].session, "abc");
        assert_eq!(agents[1].session, "");
    }

    #[test]
    fn parses_stream_lines() {
        assert_eq!(
            parse_stream_line(r#"{"id":"t1","result":{"type":"subscription_started"}}"#),
            StreamItem::Other
        );
        assert_eq!(
            parse_stream_line(
                r#"{"data":{"agent":"claude","agent_status":"working","pane_id":"w9:p1","workspace_id":"w9"},"event":"pane.agent_status_changed"}"#
            ),
            StreamItem::Status {
                pane: "w9:p1".into(),
                status: "working".into()
            }
        );
        assert_eq!(
            parse_stream_line(r#"{"error":{"code":"events_lost"}}"#),
            StreamItem::Lost
        );
        assert_eq!(parse_stream_line("garbage"), StreamItem::Other);
    }
}
