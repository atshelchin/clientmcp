//! One Durable Object per client ID. It holds the client's WebSocket through
//! the Hibernation API and serves the agent requests the Worker forwards.
//!
//! In memory it keeps the client's [`Session`]. Hibernation discards memory but
//! keeps the WebSocket, so the last accepted `register` is also stored, and the
//! session is rebuilt from it on the next event. Calls in flight keep the
//! object awake, so they are never lost to hibernation.
//!
//! A client that sends nothing for `CLIENTMCP_CLIENT_IDLE` is presumed gone and
//! closed (SPEC §3.2), as the native bridge does. Its pings are answered at the
//! edge and never reach the object, so what it last sent is read from the
//! platform's own record of the last auto-response, beside the last message
//! the object saw. A phone that lost its network fails its calls at once
//! rather than holding them until the call timeout.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::Duration;

use clientmcp_core::MAX_LISTENERS;
use clientmcp_core::agent::{self, Op};
use clientmcp_core::generations::{Generations, Refusal};
use clientmcp_core::http::{self as core_http, Response as CoreResponse};
use clientmcp_core::session::{self, Event, Link, Outbox, Session};
use clientmcp_core::token::Role;
use clientmcp_core::tools::ToolSet;
use clientmcp_core::wire::CLOSE_UNAUTHORIZED;
use clientmcp_core::wire::{CLOSE_PROTOCOL, CLOSE_REPLACED, PING, PONG};
use serde_json::{Value, json};
use worker::wasm_bindgen::prelude::*;
use worker::*;

#[wasm_bindgen]
extern "C" {
    /// The `DurableObjectState` worker-rs wraps, for the one method it does
    /// not bind.
    type RawState;

    /// `getWebSocketAutoResponseTimestamp(ws)`: when the edge last answered
    /// this socket's ping, or `null`.
    #[wasm_bindgen(method, catch, js_name = getWebSocketAutoResponseTimestamp)]
    fn auto_response_timestamp(
        this: &RawState,
        ws: &web_sys::WebSocket,
    ) -> std::result::Result<Option<js_sys::Date>, JsValue>;
}

use crate::metering::Meter;
use crate::{
    CANCEL_KEY, CLIENT_ID, GENERATION, aborted, config, convert, core_headers, either_or,
    refuse_client, websocket_response,
};
use clientmcp_core::usage::Ended;

/// Storage key of the last accepted `register` message.
const REGISTER: &str = "register";
/// Storage key of the client's token generations (`docs/self-serve.md`).
const GENERATIONS: &str = "generations";
/// Mints per client per hour, unless `CLIENTMCP_ISSUE_LIMIT` says otherwise.
const ISSUE_LIMIT: u32 = 30;
const ISSUE_WINDOW_MS: u64 = 60 * 60 * 1000;
/// WebSocket tags.
const CLIENT: &str = "client";
const LISTENER: &str = "listen";
/// What the object sends to listeners when the tool list changes.
pub const CHANGED: &str = "changed";
/// `WebSocket.OPEN`.
const OPEN: u16 = 1;
/// How many cancels of calls not yet arrived are remembered, and for how long
/// (ms): an overtaken call arrives within moments.
const EARLY_CANCELS: usize = 64;
const EARLY_CANCEL_MS: u64 = 10_000;
/// How often a waiting call looks whether its client went silent: often
/// enough that a dead client fails its call within moments of `client_idle`.
const IDLE_POLL: Duration = Duration::from_secs(2);

#[durable_object]
pub struct ClientObject {
    state: State,
    env: Env,
    current: RefCell<Option<Current>>,
    loaded: Cell<bool>,
    /// The stored generations, kept in memory: this object is their only
    /// writer, so after the first read no request waits on storage for them.
    generations: RefCell<Option<Generations>>,
    /// The client's ID once named this instance (metering).
    named: RefCell<Option<String>>,
    /// Keys cancelled before their call arrived, and when: the Worker's
    /// cancel for an agent that hung up can overtake the call it forwarded.
    cancelled_early: RefCell<VecDeque<(String, u64)>>,
}

/// The client connection and its session.
struct Current {
    /// Identifies the connection's WebSocket across hibernation (its attachment).
    serial: String,
    session: Session<Socket>,
    /// When the object last heard from it (epoch ms): its accept, or a
    /// message. Unknown (0) after hibernation, where the edge's own record
    /// of its pings answers instead.
    heard_at: Cell<u64>,
}

pub struct Socket(WebSocket);

impl Outbox for Socket {
    fn send(&self, text: &str) {
        let _ = self.0.send_with_str(text);
    }
}

impl Link for ClientObject {
    type Outbox = Socket;

    fn with<R>(&self, f: impl FnOnce(Option<&mut Session<Socket>>) -> R) -> R {
        f(self.current.borrow_mut().as_mut().map(|c| &mut c.session))
    }
}

impl DurableObject for ClientObject {
    fn new(state: State, env: Env) -> Self {
        // Answer pings at the edge, without waking the object (SPEC §3.2).
        if let Ok(pair) = WebSocketRequestResponsePair::new(PING, PONG) {
            state.set_websocket_auto_response(&pair);
        }
        ClientObject {
            state,
            env,
            current: RefCell::new(None),
            loaded: Cell::new(false),
            generations: RefCell::new(None),
            named: RefCell::new(None),
            cancelled_early: RefCell::new(VecDeque::new()),
        }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let path = req.path();
        self.meter()
            .name(req.headers().get(CLIENT_ID).ok().flatten())
            .await;
        if let Some(action) = path.strip_prefix("/internal/") {
            return self.internal(action, req).await;
        }
        // The Worker verified the token; whether its generation is still the
        // client's is this object's to say.
        let generation = presented_generation(&req);
        match path.as_str() {
            "/connect" => {
                if !self.accepts(Role::Client, generation).await {
                    self.meter().rejected("client").await;
                    return refuse_client("token revoked or replaced");
                }
                self.accept_client().await
            }
            "/listen" => {
                if !self.accepts(Role::Agent, generation).await {
                    self.meter().rejected("agent").await;
                    return convert(core_http::unauthorized_agent(None));
                }
                self.listen()
            }
            "/cancel" => {
                if let Some(key) = req.headers().get(CANCEL_KEY)? {
                    self.load().await;
                    self.cancel_or_remember(key);
                }
                Response::empty()
            }
            _ => {
                if !self.accepts(Role::Agent, generation).await {
                    self.meter().rejected("agent").await;
                    return convert(core_http::unauthorized_agent(None));
                }
                self.agent(req).await
            }
        }
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        if !self.is_client(&ws) {
            return Ok(());
        }
        let WebSocketIncomingMessage::String(text) = message else {
            let _ = ws.close(
                Some(CLOSE_PROTOCOL),
                Some("binary messages are not part of the protocol"),
            );
            return Ok(());
        };
        self.load().await;
        let serial = serial(&ws);
        let event = match self.current.borrow_mut().as_mut() {
            Some(current) if current.serial == serial => {
                current.heard_at.set(Date::now().as_millis());
                current.session.on_message(&text)
            }
            _ => return Ok(()), // a replaced connection
        };
        match event {
            Ok(Event::Registered { tools_changed }) => {
                self.state.storage().put(REGISTER, &text).await?;
                if tools_changed {
                    self.announce();
                }
            }
            Ok(Event::Nothing) => {}
            Err(close) => {
                let _ = ws.close(Some(close.code), Some(close.reason));
            }
        }
        Ok(())
    }

    /// Once a day after midnight: finished days are written out (metering).
    async fn alarm(&self) -> Result<Response> {
        self.meter().alarm().await?;
        Response::empty()
    }

    async fn websocket_close(&self, ws: WebSocket, _: usize, _: String, _: bool) -> Result<()> {
        self.ended(&ws).await
    }

    async fn websocket_error(&self, ws: WebSocket, _: Error) -> Result<()> {
        self.ended(&ws).await
    }
}

impl ClientObject {
    fn meter(&self) -> Meter<'_> {
        Meter {
            state: &self.state,
            env: &self.env,
            named: &self.named,
        }
    }

    async fn generations(&self) -> Generations {
        if let Some(known) = self.generations.borrow().clone() {
            return known;
        }
        let stored: Generations = self
            .state
            .storage()
            .get(GENERATIONS)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        *self.generations.borrow_mut() = Some(stored.clone());
        stored
    }

    /// Whether a `role` token at `generation` is still this client's.
    async fn accepts(&self, role: Role, generation: u64) -> bool {
        self.generations()
            .await
            .accepts(role, generation, Date::now().as_millis())
    }

    /// The internal API's side in the object (`internal.rs` signs): the
    /// generations, and the connections a revocation ends.
    async fn internal(&self, action: &str, mut req: Request) -> Result<Response> {
        let body: Value = req.json().await.unwrap_or(Value::Null);
        let now = Date::now().as_millis();
        let mut generations = self.generations().await;
        let role = body.get("role").and_then(Value::as_str).unwrap_or("");
        let outcome = match action {
            "state" => return Response::from_json(&generations),
            "usage" => return Response::from_json(&self.meter().pending().await?),
            "mint" => {
                let limit = self
                    .env
                    .var("CLIENTMCP_ISSUE_LIMIT")
                    .ok()
                    .and_then(|v| v.to_string().parse().ok())
                    .unwrap_or(ISSUE_LIMIT);
                generations.issue(limit, ISSUE_WINDOW_MS, now)
            }
            "rotate" => {
                let Some(role) = crate::internal::role(role) else {
                    return Response::error("`role` is client or agent", 400);
                };
                let grace = body.get("grace_ms").and_then(Value::as_u64).unwrap_or(0);
                generations.rotate(role, grace, now).map(|_| ())
            }
            "revoke" => {
                let roles: &[Role] = match role {
                    "client" => &[Role::Client],
                    "agent" => &[Role::Agent],
                    _ => &[Role::Client, Role::Agent],
                };
                let mut result = Ok(());
                for role in roles {
                    if let Err(refusal) = generations.revoke(*role).map(|_| ()) {
                        result = Err(refusal);
                        break;
                    }
                }
                result
            }
            "delete" => {
                generations.delete();
                Ok(())
            }
            _ => return Response::error("not found", 404),
        };
        if let Err(refusal) = outcome {
            let (status, reason) = match refusal {
                Refusal::Deleted => (410, "the client was deleted"),
                Refusal::TooMany => (429, "too many tokens minted for this client; try later"),
                Refusal::Grace => (400, "a grace is for the agent token, at most 24 hours"),
            };
            return Response::error(reason, status);
        }
        self.state.storage().put(GENERATIONS, &generations).await?;
        *self.generations.borrow_mut() = Some(generations.clone());
        // What no longer holds goes now, not at its next connect.
        let ended_client = match action {
            "rotate" => role == "client",
            "revoke" => role != "agent",
            "delete" => true,
            _ => false,
        };
        let ended_agent = match action {
            "rotate" => {
                role == "agent" && body.get("grace_ms").and_then(Value::as_u64).unwrap_or(0) == 0
            }
            "revoke" => role != "client",
            "delete" => true,
            _ => false,
        };
        if ended_client {
            self.load().await;
            for ws in self.state.get_websockets_with_tag(CLIENT) {
                let _ = ws.close(Some(CLOSE_UNAUTHORIZED), Some("token revoked or replaced"));
                self.ended(&ws).await?;
            }
        }
        if ended_agent {
            for ws in self.state.get_websockets_with_tag(LISTENER) {
                let _ = ws.close(Some(1000), Some("token revoked or replaced"));
            }
        }
        Response::from_json(&json!({
            "client": generations.client,
            "agent": generations.agent,
            "deleted": generations.deleted,
        }))
    }

    /// A client connection. The newest one wins (SPEC §3.1).
    async fn accept_client(&self) -> Result<Response> {
        self.load().await;
        let pair = WebSocketPair::new()?;
        let serial = clientmcp_core::random_hex(8);
        pair.server.serialize_attachment(&serial)?;
        self.state
            .accept_websocket_with_tags(&pair.server, &[CLIENT]);
        self.state.storage().delete(REGISTER).await?;

        let session = Session::new(Socket(pair.server.clone()));
        let previous = self.current.replace(Some(Current {
            serial: serial.clone(),
            session,
            heard_at: Cell::new(Date::now().as_millis()),
        }));
        for ws in self.state.get_websockets_with_tag(CLIENT) {
            if self::serial(&ws) != serial {
                let _ = ws.close(Some(CLOSE_REPLACED), Some("replaced by a newer connection"));
            }
        }
        // Dropping the previous session fails its calls in flight.
        let replaced = previous.is_some();
        if previous.is_some_and(|p| !p.session.tools().is_empty()) {
            self.announce();
        }
        // A replaced connection's time ends where the new one's begins.
        if replaced {
            self.meter().disconnected().await;
        }
        self.meter().connected().await;
        websocket_response(pair.client)
    }

    /// A notification stream held by the Worker (SPEC §8).
    fn listen(&self) -> Result<Response> {
        if self.state.get_websockets_with_tag(LISTENER).len() >= MAX_LISTENERS {
            return Response::error("too many notification streams", 429);
        }
        let pair = WebSocketPair::new()?;
        self.state
            .accept_websocket_with_tags(&pair.server, &[LISTENER]);
        Response::from_websocket(pair.client)
    }

    /// An agent request, authenticated by the Worker.
    async fn agent(&self, mut req: Request) -> Result<Response> {
        let method = req.method().to_string();
        let headers = core_headers(&req);
        let body = req.bytes().await?;
        self.load().await;
        let idle = config(&self.env).map_err(Error::RustError)?.client_idle;
        // A client gone silent is gone: its tools are not offered, and a call
        // to it is told it is offline at once.
        self.close_if_silent(idle).await?;
        let started = Date::now().as_millis();
        let mut called: Option<(String, Ended)> = None;
        let response = match agent::parse(&method, &headers, &body) {
            Op::Reply(response) => response,
            Op::Describe(describe) => {
                let instructions =
                    self.with(|s| s.and_then(|s| s.instructions().map(str::to_owned)));
                describe.respond(instructions.as_deref())
            }
            Op::ListTools(reply) => {
                let none = ToolSet::default();
                self.with(|s| reply.tools(s.map_or(&none, |s| s.tools())))
            }
            Op::CallTool(mut call) => {
                // The Worker's key, so that it can cancel the call (see lib.rs).
                call.cancel_key = req.headers().get(CANCEL_KEY)?;
                if call
                    .cancel_key
                    .as_ref()
                    .is_some_and(|key| self.was_cancelled(key))
                {
                    let response = call.respond(agent::Outcome::Cancelled);
                    self.meter()
                        .call(
                            Ended::Cancelled,
                            &call.name,
                            body.len(),
                            response.body.len(),
                            0,
                        )
                        .await;
                    return convert(response);
                }
                let timeout =
                    Delay::from(config(&self.env).map_err(Error::RustError)?.call_timeout);
                // The Worker's request is aborted when its agent hangs up and
                // Cloudflare ends the Worker's invocation. Dropping the call
                // then cancels it (`session::call`'s guard). A client that
                // goes silent while the call waits is closed, and the call
                // told so.
                let gone = either_or(aborted(&req), self.silent(idle));
                let outcome = match either_or(session::call(self, &call, timeout), gone).await {
                    Ok(outcome) => outcome,
                    Err(Ok(())) => {
                        console_log!(
                            "clientmcp: the Worker's request went away; its call is cancelled"
                        );
                        agent::Outcome::Cancelled
                    }
                    Err(Err(())) => {
                        self.close_if_silent(idle).await?;
                        agent::Outcome::Disconnected
                    }
                };
                called = Some((call.name.clone(), Ended::of(&outcome)));
                call.respond(outcome)
            }
            Op::Cancel(key) => {
                self.with(|s| s.map(|s| s.cancel(&key)));
                agent::accepted()
            }
            Op::Listen(_) => {
                CoreResponse::text(500, "notification streams are served by the Worker")
            }
        };
        let meter = self.meter();
        match called {
            Some((tool, ended)) => {
                let ms = Date::now().as_millis().saturating_sub(started);
                meter
                    .call(ended, &tool, body.len(), response.body.len(), ms)
                    .await;
            }
            None => meter.request(body.len(), response.body.len()).await,
        }
        convert(response)
    }

    /// Rebuilds the session after the object was evicted from memory.
    async fn load(&self) {
        if self.loaded.get() {
            return;
        }
        let open = self
            .state
            .get_websockets_with_tag(CLIENT)
            .into_iter()
            .find(|ws| ws.as_ref().ready_state() == OPEN);
        let current = match open {
            Some(ws) => {
                let register: Option<String> =
                    self.state.storage().get(REGISTER).await.ok().flatten();
                let serial = serial(&ws);
                let session = match register {
                    Some(register) => {
                        console_log!("clientmcp: rebuilt a session after hibernation");
                        Session::restore(Socket(ws), &register)
                    }
                    None => Session::new(Socket(ws)),
                };
                Some(Current {
                    serial,
                    session,
                    heard_at: Cell::new(0),
                })
            }
            None => None,
        };
        if !self.loaded.replace(true) {
            *self.current.borrow_mut() = current;
        }
    }

    /// A client or listener WebSocket closed.
    async fn ended(&self, ws: &WebSocket) -> Result<()> {
        if !self.is_client(ws) {
            return Ok(());
        }
        self.load().await;
        let serial = serial(ws);
        let current = self.current.borrow().as_ref().map(|c| c.serial == serial);
        match current {
            Some(true) => {
                let ended = self.current.take();
                self.meter().disconnected().await;
                self.state.storage().delete(REGISTER).await?;
                if ended.is_some_and(|e| !e.session.tools().is_empty()) {
                    self.announce();
                }
            }
            Some(false) => {} // a replaced connection
            // Closed while the object hibernated: only storage remembers the client.
            None => {
                self.meter().disconnected().await;
                if self.state.storage().delete(REGISTER).await? {
                    self.announce();
                }
            }
        }
        Ok(())
    }

    /// The Worker's cancel of the call with `key`: one that has not arrived
    /// yet is cancelled when it does. (Not an agent's own cancel: that one is
    /// often late, and an agent may reuse the request ID it names. A legacy
    /// key is made of that ID too, hence the short memory.)
    fn cancel_or_remember(&self, key: String) {
        if !self.with(|s| s.is_some_and(|s| s.cancel(&key))) {
            let mut early = self.cancelled_early.borrow_mut();
            if early.len() == EARLY_CANCELS {
                early.pop_front();
            }
            early.push_back((key, Date::now().as_millis()));
        }
    }

    /// Whether the call with `key` was cancelled moments before it arrived.
    fn was_cancelled(&self, key: &str) -> bool {
        let now = Date::now().as_millis();
        let mut early = self.cancelled_early.borrow_mut();
        early.retain(|(_, at)| now.saturating_sub(*at) < EARLY_CANCEL_MS);
        let found = early.iter().position(|(k, _)| k == key);
        found.map(|i| early.remove(i)).is_some()
    }

    /// The current client's socket and when it was last heard from (epoch
    /// ms): the later of its last message and the edge's last answer to its
    /// ping. `None` with no client, or none known.
    fn last_heard(&self) -> Option<(WebSocket, u64)> {
        let (serial, heard_at) = self
            .current
            .borrow()
            .as_ref()
            .map(|c| (c.serial.clone(), c.heard_at.get()))?;
        let ws = self
            .state
            .get_websockets_with_tag(CLIENT)
            .into_iter()
            .find(|ws| self::serial(ws) == serial)?;
        let pinged = self
            .state
            .as_raw()
            .unchecked_ref::<RawState>()
            .auto_response_timestamp(ws.as_ref())
            .ok()
            .flatten()
            .map_or(0, |date| date.get_time() as u64);
        let last = heard_at.max(pinged);
        (last > 0).then_some((ws, last))
    }

    /// Whether the current client has sent nothing for `idle`.
    fn is_silent(&self, idle: Duration) -> bool {
        self.last_heard().is_some_and(|(_, last)| {
            Date::now().as_millis().saturating_sub(last) > idle.as_millis() as u64
        })
    }

    /// Resolves once the current client has gone silent for `idle`.
    async fn silent(&self, idle: Duration) {
        while !self.is_silent(idle) {
            Delay::from(IDLE_POLL).await;
        }
    }

    /// Closes the current client if it has sent nothing for `idle`, as the
    /// native bridge does (SPEC §3.2): its session ends, failing its calls.
    async fn close_if_silent(&self, idle: Duration) -> Result<()> {
        if !self.is_silent(idle) {
            return Ok(());
        }
        if let Some((ws, last)) = self.last_heard() {
            console_log!(
                "clientmcp: a client silent for {} ms is presumed gone",
                Date::now().as_millis().saturating_sub(last)
            );
            let _ = ws.close(Some(1000), Some("idle"));
            self.ended(&ws).await?;
        }
        Ok(())
    }

    fn announce(&self) {
        for ws in self.state.get_websockets_with_tag(LISTENER) {
            let _ = ws.send_with_str(CHANGED);
        }
    }

    fn is_client(&self, ws: &WebSocket) -> bool {
        self.state.get_tags(ws).iter().any(|t| t == CLIENT)
    }
}

/// The generation of the token the Worker verified; 0 when it said none.
fn presented_generation(req: &Request) -> u64 {
    req.headers()
        .get(GENERATION)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn serial(ws: &WebSocket) -> String {
    ws.deserialize_attachment::<String>()
        .ok()
        .flatten()
        .unwrap_or_default()
}
