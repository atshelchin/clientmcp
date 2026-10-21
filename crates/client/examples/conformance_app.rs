//! The application side of the conformance suite, played by this crate:
//! `CLIENTMCP_APP=rust tests/run-native.sh` (see `tests/rust-app.mjs`).
//!
//! The suite's tool handlers stay in JavaScript. This process does all of the
//! protocol, and asks the suite to run a handler over stdio, one JSON object a
//! line:
//!
//! - in: `options`, `expose`, `unexpose`, `instructions`, `connect`, `close`,
//!   and `reply` to a call;
//! - out: `status`, `connected`, `call`, and `cancelled` — sent when the
//!   client drops a handler's future, which is how it cancels one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use clientmcp_client::{Client, Options, Reply, Risk, Status, Tool};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

type Out = mpsc::UnboundedSender<Value>;
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Reply, String>>>>>;

/// Tells the suite a call was cancelled, unless it was answered.
struct Cancelled {
    call: u64,
    out: Out,
    pending: Pending,
    answered: bool,
}

impl Drop for Cancelled {
    fn drop(&mut self) {
        if !self.answered {
            self.pending.lock().unwrap().remove(&self.call);
            let _ = self
                .out
                .send(json!({ "ev": "cancelled", "call": self.call }));
        }
    }
}

fn tool(definition: &Value, out: &Out, pending: &Pending) -> Tool {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let (out, pending) = (out.clone(), Arc::clone(pending));
    let mut tool = Tool::new(move |args, ctx| {
        let (out, pending) = (out.clone(), Arc::clone(&pending));
        async move {
            let call = CALLS.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            pending.lock().unwrap().insert(call, tx);
            let mut guard = Cancelled {
                call,
                out: out.clone(),
                pending,
                answered: false,
            };
            let _ = out.send(json!({ "ev": "call", "call": call, "name": ctx.name, "args": args }));
            let reply = rx
                .await
                .unwrap_or_else(|_| Err("the suite went away".into()));
            guard.answered = true;
            reply
        }
    });
    let text = |key: &str| {
        definition
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let object = |key: &str| definition.get(key).and_then(Value::as_object).cloned();
    if let Some(title) = text("title") {
        tool = tool.title(title);
    }
    if let Some(description) = text("description") {
        tool = tool.description(description);
    }
    if let Some(schema) = definition.get("inputSchema") {
        tool = tool.input_schema(schema.clone());
    }
    if let Some(schema) = definition.get("outputSchema") {
        tool = tool.output_schema(schema.clone());
    }
    if let Some(annotations) = object("annotations") {
        tool = tool.annotations(annotations);
    }
    if let Some(icons) = definition.get("icons") {
        tool = tool.icons(icons.clone());
    }
    if let Some(meta) = object("_meta") {
        tool = tool.meta(meta);
    }
    let risk = match text("risk").as_deref() {
        Some("observe") => Some(Risk::Observe),
        Some("operate") => Some(Risk::Operate),
        Some("manage") => Some(Risk::Manage),
        Some("transact") => Some(Risk::Transact),
        _ => None,
    };
    if let Some(risk) = risk {
        tool = tool.risk(risk);
    }
    tool
}

fn reply(reply: &Value) -> Result<Reply, String> {
    let value = reply.get("value").cloned().unwrap_or(Value::Null);
    match reply.get("kind").and_then(Value::as_str) {
        Some("text") => Ok(Reply::Text(value.as_str().unwrap_or_default().to_owned())),
        Some("json") => Ok(Reply::Json(value)),
        Some("result") => Ok(Reply::Result(value)),
        Some("error") => Err(value.as_str().unwrap_or_default().to_owned()),
        _ => Ok(Reply::Empty),
    }
}

fn status(status: Status) -> &'static str {
    match status {
        Status::Connecting => "connecting",
        Status::Online => "online",
        Status::Offline => "offline",
        Status::Closed => "closed",
    }
}

#[tokio::main]
async fn main() {
    let (out, mut lines_out) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = lines_out.recv().await {
            let mut text = line.to_string();
            text.push('\n');
            if stdout.write_all(text.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    let pending: Pending = Arc::default();
    let mut client: Option<Client> = None;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let text = |key: &str| message.get(key).and_then(Value::as_str).map(str::to_owned);
        match (message.get("op").and_then(Value::as_str), &client) {
            (Some("options"), None) => {
                let status_out = out.clone();
                let mut options = Options::new(
                    text("url").unwrap_or_default(),
                    text("token").unwrap_or_default(),
                )
                .on_status(move |status_now, detail| {
                    let mut event = Map::new();
                    event.insert("ev".into(), "status".into());
                    event.insert("status".into(), status(status_now).into());
                    if let Some(close) = detail {
                        event.insert("code".into(), close.code.into());
                        event.insert("reason".into(), close.reason.clone().into());
                    }
                    let _ = status_out.send(Value::Object(event));
                });
                if let Some(instructions) = text("instructions") {
                    options = options.instructions(instructions);
                }
                client = Some(Client::new(options));
            }
            (Some("expose"), Some(client)) => {
                let definition = message.get("tool").cloned().unwrap_or_else(|| json!({}));
                client.expose(
                    text("name").unwrap_or_default(),
                    tool(&definition, &out, &pending),
                );
            }
            (Some("unexpose"), Some(client)) => client.unexpose(&text("name").unwrap_or_default()),
            (Some("instructions"), Some(client)) => client.set_instructions(text("text")),
            (Some("connect"), Some(client)) => {
                let (client, out) = (client.clone(), out.clone());
                tokio::spawn(async move {
                    let event = match client.connect().await {
                        Ok(()) => json!({ "ev": "connected", "ok": true }),
                        Err(close) => {
                            json!({ "ev": "connected", "ok": false, "code": close.code, "message": close.to_string() })
                        }
                    };
                    let _ = out.send(event);
                });
            }
            (Some("close"), Some(client)) => client.close(),
            (Some("reply"), _) => {
                let call = message
                    .get("call")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX);
                let answer = reply(message.get("reply").unwrap_or(&Value::Null));
                if let Some(tx) = pending.lock().unwrap().remove(&call) {
                    let _ = tx.send(answer);
                }
            }
            _ => {}
        }
    }
    // The suite is done with this application.
    if let Some(client) = &client {
        client.close();
    }
    // Long enough for the close frame to go; every line written is flushed.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    writer.abort();
    std::process::exit(0);
}
