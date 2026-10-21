//! One Durable Object per client ID. It holds the client's WebSocket through
//! the Hibernation API and serves the agent requests the Worker forwards.
//!
//! In memory it keeps the client's [`Session`]. Hibernation discards memory but
//! keeps the WebSocket, so the last accepted `register` is also stored, and the
//! session is rebuilt from it on the next event. Calls in flight keep the
//! object awake, so they are never lost to hibernation.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use clientmcp_core::MAX_LISTENERS;
use clientmcp_core::agent::{self, Op};
use clientmcp_core::http::Response as CoreResponse;
use clientmcp_core::session::{self, Event, Link, Outbox, Session};
use clientmcp_core::tools::ToolSet;
use clientmcp_core::wire::{CLOSE_PROTOCOL, CLOSE_REPLACED, PING, PONG};
use worker::*;

use crate::{CANCEL_KEY, aborted, config, convert, core_headers, either_or, websocket_response};

/// Storage key of the last accepted `register` message.
const REGISTER: &str = "register";
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

#[durable_object]
pub struct ClientObject {
    state: State,
    env: Env,
    current: RefCell<Option<Current>>,
    loaded: Cell<bool>,
    /// Keys cancelled before their call arrived, and when: the Worker's
    /// cancel for an agent that hung up can overtake the call it forwarded.
    cancelled_early: RefCell<VecDeque<(String, u64)>>,
}

/// The client connection and its session.
struct Current {
    /// Identifies the connection's WebSocket across hibernation (its attachment).
    serial: String,
    session: Session<Socket>,
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
            cancelled_early: RefCell::new(VecDeque::new()),
        }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        match req.path().as_str() {
            "/connect" => self.accept_client().await,
            "/listen" => self.listen(),
            "/cancel" => {
                if let Some(key) = req.headers().get(CANCEL_KEY)? {
                    self.load().await;
                    self.cancel_or_remember(key);
                }
                Response::empty()
            }
            _ => self.agent(req).await,
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
            Some(current) if current.serial == serial => current.session.on_message(&text),
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

    async fn websocket_close(&self, ws: WebSocket, _: usize, _: String, _: bool) -> Result<()> {
        self.ended(&ws).await
    }

    async fn websocket_error(&self, ws: WebSocket, _: Error) -> Result<()> {
        self.ended(&ws).await
    }
}

impl ClientObject {
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
        }));
        for ws in self.state.get_websockets_with_tag(CLIENT) {
            if self::serial(&ws) != serial {
                let _ = ws.close(Some(CLOSE_REPLACED), Some("replaced by a newer connection"));
            }
        }
        // Dropping the previous session fails its calls in flight.
        if previous.is_some_and(|p| !p.session.tools().is_empty()) {
            self.announce();
        }
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
                    return convert(call.respond(agent::Outcome::Cancelled));
                }
                let timeout =
                    Delay::from(config(&self.env).map_err(Error::RustError)?.call_timeout);
                // The Worker's request is aborted when its agent hangs up and
                // Cloudflare ends the Worker's invocation. Dropping the call
                // then cancels it (`session::call`'s guard).
                match either_or(session::call(self, &call, timeout), aborted(&req)).await {
                    Ok(outcome) => call.respond(outcome),
                    Err(()) => {
                        console_log!(
                            "clientmcp: the Worker's request went away; its call is cancelled"
                        );
                        call.respond(agent::Outcome::Cancelled)
                    }
                }
            }
            Op::Cancel(key) => {
                self.with(|s| s.map(|s| s.cancel(&key)));
                agent::accepted()
            }
            Op::Listen(_) => {
                CoreResponse::text(500, "notification streams are served by the Worker")
            }
        };
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
                Some(Current { serial, session })
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
                self.state.storage().delete(REGISTER).await?;
                if ended.is_some_and(|e| !e.session.tools().is_empty()) {
                    self.announce();
                }
            }
            Some(false) => {} // a replaced connection
            // Closed while the object hibernated: only storage remembers the client.
            None => {
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

    fn announce(&self) {
        for ws in self.state.get_websockets_with_tag(LISTENER) {
            let _ = ws.send_with_str(CHANGED);
        }
    }

    fn is_client(&self, ws: &WebSocket) -> bool {
        self.state.get_tags(ws).iter().any(|t| t == CLIENT)
    }
}

fn serial(ws: &WebSocket) -> String {
    ws.deserialize_attachment::<String>()
        .ok()
        .flatten()
        .unwrap_or_default()
}
