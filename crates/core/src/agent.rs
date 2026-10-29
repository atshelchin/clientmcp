//! The agent endpoint: MCP Streamable HTTP in the modern (2026-07-28) and the
//! legacy (2025-03-26 … 2025-11-25) eras (SPEC §4).
//!
//! [`parse`] turns an HTTP request into an [`Op`] without touching any client
//! state; the runtime then executes the op against the client it is for.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::http::{Headers, Response, decode_header_value};
use crate::json::{Obj, quote};
use crate::tools::ToolSet;
pub use crate::wire::ToolResult;

/// A call that takes longer than this is answered with an SSE stream that
/// carries a keep-alive comment at this interval, then the response. Writing
/// is how a bridge notices an agent that has gone away, and keeps proxies
/// from timing out calls that wait for a human.
pub const CALL_KEEPALIVE: Duration = Duration::from_secs(1);

/// Protocol revisions served with per-request metadata.
pub const MODERN: &[&str] = &["2026-07-28"];
/// Protocol revisions served with an `initialize` handshake, newest first.
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];

const SUPPORTED: &str = r#"["2026-07-28","2025-11-25","2025-06-18","2025-03-26"]"#;
const CAPABILITIES: &str = r#"{"tools":{"listChanged":true}}"#;
const SERVER_INFO: &str = concat!(
    r#"{"name":"clientmcp","version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""}"#
);
const KEY_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const KEY_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const KEY_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const KEY_SUBSCRIPTION: &str = "io.modelcontextprotocol/subscriptionId";

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const HEADER_MISMATCH: i32 = -32020;
const UNSUPPORTED_VERSION: i32 = -32022;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

/// What an agent request asks for.
#[derive(Debug)]
pub enum Op {
    /// A complete response that needs no client state.
    Reply(Response),
    /// `server/discover`, or a legacy `initialize`.
    Describe(Describe),
    /// `tools/list`.
    ListTools(Reply),
    /// `tools/call`.
    CallTool(Call),
    /// A notification stream: `subscriptions/listen`, or a legacy `GET`.
    Listen(Listen),
    /// A legacy `notifications/cancelled`, naming a [`Call::cancel_key`].
    /// Answer it with [`accepted`].
    Cancel(String),
}

/// The response to a notification.
pub fn accepted() -> Response {
    Response::empty(202)
}

/// Classifies an agent request. `method` is the HTTP method.
pub fn parse(method: &str, headers: &Headers, body: &[u8]) -> Op {
    match method {
        "POST" => parse_post(headers, body),
        "GET"
            if headers
                .get("accept")
                .is_some_and(|a| a.contains("text/event-stream")) =>
        {
            Op::Listen(Listen {
                subscription: None,
                tools: true,
            })
        }
        _ => Op::Reply(Response::empty(405).header("allow", "GET, POST")),
    }
}

fn parse_post(headers: &Headers, body: &[u8]) -> Op {
    // Only the members the bridge needs are parsed; the rest stays raw JSON,
    // so arguments are forwarded to the client verbatim.
    let message: Members = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(_) if serde_json::from_slice::<IgnoredAny>(body).is_ok() => {
            return Op::Reply(rpc_error(
                400,
                None,
                INVALID_REQUEST,
                "Expected one JSON-RPC message",
            ));
        }
        Err(_) => return Op::Reply(rpc_error(400, None, PARSE_ERROR, "Parse error")),
    };
    let id = message
        .get("id")
        .map(|raw| member(raw).unwrap_or(Value::Null));
    let invalid =
        |id: Option<&Value>, why: &str| Op::Reply(rpc_error(400, id, INVALID_REQUEST, why));
    if message.get("jsonrpc").and_then(member::<String>).as_deref() != Some("2.0") {
        return invalid(id.as_ref(), "jsonrpc must be \"2.0\"");
    }
    let Some(method) = message.get("method").and_then(member::<String>) else {
        return invalid(id.as_ref(), "Expected a request or notification");
    };
    let params: Members = match message.get("params") {
        None => Members::new(),
        Some(raw) if is_null(raw) => Members::new(),
        Some(raw) => match member(raw) {
            Some(params) => params,
            None => {
                return Op::Reply(rpc_error(
                    400,
                    id.as_ref(),
                    INVALID_PARAMS,
                    "params must be an object",
                ));
            }
        },
    };
    let meta: Map<String, Value> = params.get("_meta").and_then(member).unwrap_or_default();
    match id {
        None => notification(&method, &params, headers),
        Some(id) if id.is_string() || id.is_i64() || id.is_u64() => {
            match era(&id, &method, &meta, headers) {
                Ok(era) => request(Reply { era, id }, &method, params, &meta, headers),
                Err(response) => Op::Reply(response),
            }
        }
        Some(id) => invalid(Some(&id), "id must be a string or an integer"),
    }
}

/// JSON object members, their values kept as raw JSON.
type Members = BTreeMap<String, Box<RawValue>>;

/// Parses a member's raw value; `None` if it does not have type `T`.
#[allow(clippy::borrowed_box)] // the signature `Option<&Box<RawValue>>::and_then` wants
fn member<T: DeserializeOwned>(raw: &Box<RawValue>) -> Option<T> {
    serde_json::from_str(raw.get()).ok()
}

fn is_null(raw: &RawValue) -> bool {
    raw.get() == "null"
}

fn notification(method: &str, params: &Members, headers: &Headers) -> Op {
    let request_id = params.get("requestId").and_then(member::<Value>);
    match (method, headers.get("mcp-session-id"), request_id) {
        ("notifications/cancelled", Some(session), Some(id)) => {
            Op::Cancel(legacy_key(session, &id))
        }
        _ => Op::Reply(accepted()),
    }
}

/// Decides how to serve a request (SPEC §4.1).
fn era(
    id: &Value,
    method: &str,
    meta: &Map<String, Value>,
    headers: &Headers,
) -> Result<Era, Response> {
    if method == "initialize" {
        return Ok(Era::Legacy);
    }
    let header = headers.get("mcp-protocol-version");
    match meta.get(KEY_VERSION) {
        Some(Value::String(version)) if header != Some(version) => Err(rpc_error(
            400,
            Some(id),
            HEADER_MISMATCH,
            "Header mismatch: MCP-Protocol-Version does not match _meta",
        )),
        Some(Value::String(version)) if MODERN.contains(&version.as_str()) => Ok(Era::Modern),
        Some(Value::String(version)) => Err(unsupported_version(id, version)),
        Some(_) => Err(rpc_error(
            400,
            Some(id),
            INVALID_PARAMS,
            "protocolVersion must be a string",
        )),
        None => match header {
            // MCP lets a server treat a request without the header as 2025-03-26.
            None => Ok(Era::Legacy),
            Some(version) if LEGACY.contains(&version) => Ok(Era::Legacy),
            Some(version) if MODERN.contains(&version) => Err(missing_meta(id, KEY_VERSION)),
            Some(version) => Err(unsupported_version(id, version)),
        },
    }
}

fn request(
    reply: Reply,
    method: &str,
    mut params: Members,
    meta: &Map<String, Value>,
    headers: &Headers,
) -> Op {
    if reply.era == Era::Modern {
        if headers.get("mcp-method") != Some(method) {
            return Op::Reply(reply.error(
                400,
                HEADER_MISMATCH,
                "Header mismatch: Mcp-Method does not match method",
            ));
        }
        if !meta.get(KEY_CAPABILITIES).is_some_and(Value::is_object) {
            return Op::Reply(missing_meta(&reply.id, KEY_CAPABILITIES));
        }
    }
    let has_cursor = params.get("cursor").is_some_and(|c| !is_null(c));
    match (reply.era, method) {
        (Era::Modern, "server/discover") => Op::Describe(Describe {
            reply,
            legacy_version: None,
        }),
        (Era::Legacy, "initialize") => {
            let requested = params.get("protocolVersion").and_then(member::<String>);
            let version = LEGACY
                .iter()
                .copied()
                .find(|v| Some(*v) == requested.as_deref());
            let legacy_version = Some(version.unwrap_or(LEGACY[0]));
            Op::Describe(Describe {
                reply,
                legacy_version,
            })
        }
        (Era::Legacy, "ping") => Op::Reply(reply.finish(reply.begin(0), None)),
        // The bridge never hands out cursors, so any cursor is invalid.
        (_, "tools/list") if has_cursor => {
            Op::Reply(reply.error(200, INVALID_PARAMS, "Invalid cursor"))
        }
        (_, "tools/list") => Op::ListTools(reply),
        (_, "tools/call") => call(reply, &mut params, headers),
        (Era::Modern, "subscriptions/listen") => match params
            .get("notifications")
            .and_then(member::<Map<String, Value>>)
        {
            Some(filter) => {
                let tools = filter.get("toolsListChanged") == Some(&Value::Bool(true));
                Op::Listen(Listen {
                    subscription: Some(reply.id),
                    tools,
                })
            }
            None => Op::Reply(reply.error(
                400,
                INVALID_PARAMS,
                "subscriptions/listen requires a notifications filter",
            )),
        },
        (Era::Modern, _) => Op::Reply(reply.error(404, METHOD_NOT_FOUND, "Method not found")),
        (Era::Legacy, _) => Op::Reply(reply.error(200, METHOD_NOT_FOUND, "Method not found")),
    }
}

fn call(reply: Reply, params: &mut Members, headers: &Headers) -> Op {
    let Some(name) = params.get("name").and_then(member::<String>) else {
        return Op::Reply(reply.error(200, INVALID_PARAMS, "tools/call requires a tool name"));
    };
    let arguments = match params.remove("arguments") {
        Some(raw) if raw.get().starts_with('{') => raw,
        Some(raw) if !is_null(&raw) => {
            return Op::Reply(reply.error(200, INVALID_PARAMS, "arguments must be an object"));
        }
        _ => RawValue::from_string("{}".into()).expect("valid JSON"),
    };
    let (param_headers, cancel_key) = match reply.era {
        Era::Modern => {
            if headers
                .get("mcp-name")
                .and_then(decode_header_value)
                .as_ref()
                != Some(&name)
            {
                return Op::Reply(reply.error(
                    400,
                    HEADER_MISMATCH,
                    "Header mismatch: Mcp-Name does not match the tool name",
                ));
            }
            (headers.with_prefix("mcp-param-"), None)
        }
        Era::Legacy => {
            let key = headers
                .get("mcp-session-id")
                .map(|s| legacy_key(s, &reply.id));
            (Headers::default(), key)
        }
    };
    Op::CallTool(Call {
        reply,
        name,
        arguments,
        headers: param_headers,
        cancel_key,
        streamable: headers
            .get("accept")
            .is_some_and(|a| a.contains("text/event-stream")),
    })
}

/// A legacy request ID is unique only within its session. Hex keeps the key
/// safe to pass along in an HTTP header.
fn legacy_key(session: &str, id: &Value) -> String {
    format!("{session}\n{id}")
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Correlates a response with its request.
#[derive(Debug)]
pub struct Reply {
    pub era: Era,
    id: Value,
}

impl Reply {
    /// Starts a result response; add the result's members, then [`Reply::finish`].
    fn begin(&self, capacity: usize) -> Obj {
        envelope(&self.id, capacity + 160).open("result")
    }

    /// Completes a result response. `client_meta` is `_meta` supplied by the client.
    fn finish(&self, result: Obj, client_meta: Option<&str>) -> Response {
        let result = match self.era {
            Era::Modern => result
                .str("resultType", "complete")
                .raw("_meta", &server_meta(client_meta)),
            Era::Legacy => match client_meta {
                Some(meta) => result.raw("_meta", meta),
                None => result,
            },
        };
        Response::json(200, result.close().end())
    }

    fn error(&self, status: u16, code: i32, message: &str) -> Response {
        rpc_error(status, Some(&self.id), code, message)
    }

    /// The `tools/list` response.
    pub fn tools(&self, tools: &ToolSet) -> Response {
        let result = self.begin(tools.json().len()).raw("tools", tools.json());
        self.finish(cacheable(result, self.era), None)
    }
}

/// `server/discover` or `initialize`.
#[derive(Debug)]
pub struct Describe {
    reply: Reply,
    legacy_version: Option<&'static str>,
}

impl Describe {
    pub fn respond(&self, instructions: Option<&str>) -> Response {
        let with_instructions = |obj: Obj| match instructions {
            Some(text) => obj.str("instructions", text),
            None => obj,
        };
        match self.legacy_version {
            None => {
                let result = self
                    .reply
                    .begin(instructions.map_or(0, str::len))
                    .raw("supportedVersions", SUPPORTED)
                    .raw("capabilities", CAPABILITIES);
                self.reply
                    .finish(cacheable(with_instructions(result), Era::Modern), None)
            }
            Some(version) => {
                let result = self
                    .reply
                    .begin(instructions.map_or(0, str::len))
                    .str("protocolVersion", version)
                    .raw("capabilities", CAPABILITIES)
                    .raw("serverInfo", SERVER_INFO);
                self.reply
                    .finish(with_instructions(result), None)
                    .header("mcp-session-id", crate::random_hex(16))
            }
        }
    }
}

/// `tools/call`.
#[derive(Debug)]
pub struct Call {
    pub reply: Reply,
    pub name: String,
    /// A JSON object, forwarded to the client verbatim.
    pub arguments: Box<RawValue>,
    /// The `Mcp-Param-*` headers, to check against the tool's parameters (modern).
    pub headers: Headers,
    /// The key under which the call can be cancelled: for a legacy request,
    /// its session and request ID, which `notifications/cancelled` names. A
    /// runtime may set its own.
    pub cancel_key: Option<String>,
    /// Whether the agent accepts an SSE response (see [`CALL_KEEPALIVE`]).
    pub streamable: bool,
}

/// How a call ended.
#[derive(Debug)]
pub enum Outcome {
    Done(ToolResult),
    UnknownTool,
    /// `Mcp-Param-*` headers disagree with the arguments.
    HeaderMismatch(String),
    Offline,
    Disconnected,
    Timeout,
    Busy,
    /// The agent cancelled the call.
    Cancelled,
    /// Held back by a bridge's protective rate limit (`docs/self-serve.md`).
    Limited,
}

impl Call {
    pub fn respond(&self, outcome: Outcome) -> Response {
        let reply = &self.reply;
        match outcome {
            Outcome::Done(result) => {
                let size = result
                    .0
                    .iter()
                    .map(|(k, v)| k.len() + v.get().len() + 4)
                    .sum();
                let mut obj = reply.begin(size);
                for (key, value) in &result.0 {
                    if key != "resultType" && key != "_meta" {
                        obj = obj.raw(key, value.get());
                    }
                }
                reply.finish(obj, result.0.get("_meta").map(|m| m.get()))
            }
            Outcome::UnknownTool => {
                reply.error(200, INVALID_PARAMS, &format!("Unknown tool: {}", self.name))
            }
            Outcome::HeaderMismatch(why) => {
                reply.error(400, HEADER_MISMATCH, &format!("Header mismatch: {why}"))
            }
            Outcome::Offline => {
                self.failed("The application is offline: it is not connected to the bridge.")
            }
            Outcome::Disconnected => {
                self.failed("The application disconnected before the tool call completed.")
            }
            Outcome::Timeout => {
                self.failed("The application did not complete the tool call in time.")
            }
            Outcome::Busy => {
                self.failed("The application has too many tool calls in progress. Try again later.")
            }
            Outcome::Cancelled => self.failed("The tool call was cancelled."),
            Outcome::Limited => self.failed(
                "This application is receiving too many tool calls. Slow down and try again in a minute.",
            ),
        }
    }

    /// A tool execution error, which the model sees (SPEC §4.3).
    fn failed(&self, message: &str) -> Response {
        let content = format!(r#"[{{"type":"text","text":{}}}]"#, quote(message));
        let result = self
            .reply
            .begin(content.len())
            .raw("content", &content)
            .raw("isError", "true");
        self.reply.finish(result, None)
    }
}

/// A notification stream for one client (SPEC §4.4).
#[derive(Debug)]
pub struct Listen {
    /// The `subscriptions/listen` request ID; `None` for a legacy `GET`.
    subscription: Option<Value>,
    /// Whether the agent asked for tool list changes.
    tools: bool,
}

impl Listen {
    /// The first SSE event of the stream.
    pub fn opening(&self) -> Option<String> {
        let id = self.subscription.as_ref()?;
        let filter = if self.tools {
            r#"{"toolsListChanged":true}"#
        } else {
            "{}"
        };
        let params = Obj::new()
            .raw("_meta", &subscription_meta(id))
            .raw("notifications", filter);
        Some(notification_json(
            "notifications/subscriptions/acknowledged",
            Some(params),
        ))
    }

    /// The SSE event announcing a tool list change, if the agent wants it.
    pub fn changed(&self) -> Option<String> {
        if !self.tools {
            return None;
        }
        let params = self
            .subscription
            .as_ref()
            .map(|id| Obj::new().raw("_meta", &subscription_meta(id)));
        Some(notification_json(
            "notifications/tools/list_changed",
            params,
        ))
    }

    /// The SSE event ending the stream gracefully, if the era has one.
    pub fn closing(&self) -> Option<String> {
        let id = self.subscription.as_ref()?;
        let response = envelope(id, 256)
            .open("result")
            .str("resultType", "complete")
            .open("_meta")
            .raw(KEY_SUBSCRIPTION, &id.to_string())
            .raw(KEY_SERVER_INFO, SERVER_INFO)
            .close()
            .close()
            .end();
        Some(crate::http::sse_event(&response))
    }
}

fn subscription_meta(id: &Value) -> String {
    Obj::new().raw(KEY_SUBSCRIPTION, &id.to_string()).end()
}

fn notification_json(method: &str, params: Option<Obj>) -> String {
    let message = Obj::new().str("jsonrpc", "2.0").str("method", method);
    let message = match params {
        Some(params) => message.raw("params", &params.end()),
        None => message,
    };
    crate::http::sse_event(&message.end())
}

/// Cache hints for list results: the tool set can change at any time and
/// belongs to one client.
fn cacheable(result: Obj, era: Era) -> Obj {
    match era {
        Era::Modern => result.raw("ttlMs", "0").str("cacheScope", "private"),
        Era::Legacy => result,
    }
}

/// `_meta` of a modern result: the client's, plus the server identity.
fn server_meta(client_meta: Option<&str>) -> String {
    let Some(client_meta) = client_meta else {
        return Obj::new().raw(KEY_SERVER_INFO, SERVER_INFO).end();
    };
    let client: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(client_meta).unwrap_or_default();
    let mut meta = Obj::new();
    for (key, value) in &client {
        if key != KEY_SERVER_INFO {
            meta = meta.raw(key, value.get());
        }
    }
    meta.raw(KEY_SERVER_INFO, SERVER_INFO).end()
}

fn envelope(id: &Value, capacity: usize) -> Obj {
    Obj::with_capacity(capacity)
        .str("jsonrpc", "2.0")
        .raw("id", &id.to_string())
}

fn rpc_error(status: u16, id: Option<&Value>, code: i32, message: &str) -> Response {
    rpc_error_data(status, id, code, message, None)
}

fn rpc_error_data(
    status: u16,
    id: Option<&Value>,
    code: i32,
    message: &str,
    data: Option<&str>,
) -> Response {
    let error = Obj::new()
        .raw("code", &code.to_string())
        .str("message", message);
    let error = match data {
        Some(data) => error.raw("data", data),
        None => error,
    };
    let message = Obj::new().str("jsonrpc", "2.0");
    let message = match id {
        Some(id) => message.raw("id", &id.to_string()),
        None => message,
    };
    Response::json(status, message.raw("error", &error.end()).end())
}

fn unsupported_version(id: &Value, requested: &str) -> Response {
    let data = Obj::new()
        .raw("supported", SUPPORTED)
        .str("requested", requested)
        .end();
    rpc_error_data(
        400,
        Some(id),
        UNSUPPORTED_VERSION,
        "Unsupported protocol version",
        Some(&data),
    )
}

fn missing_meta(id: &Value, key: &str) -> Response {
    rpc_error(
        400,
        Some(id),
        INVALID_PARAMS,
        &format!("Missing _meta {key}"),
    )
}

#[cfg(test)]
mod tests;
