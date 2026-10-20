//! The ClientMCP bridge on Cloudflare Workers.
//!
//! The Worker is the stateless front door: it routes, checks origins, verifies
//! tokens and holds agents' notification streams. Everything about a client
//! lives in its Durable Object ([`client_object`]), addressed by client ID.

mod client_object;

use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::time::Duration;

use clientmcp_core::Config;
use clientmcp_core::agent::{self, CALL_KEEPALIVE, Listen, Op};
use clientmcp_core::http::{
    self, Admitted, ClientRefusal, Headers as CoreHeaders, Route, SSE_HEADERS, SSE_KEEPALIVE,
};
use clientmcp_core::wire::{CLOSE_UNAUTHORIZED, MAX_MESSAGE_BYTES, SUBPROTOCOL};
use futures_channel::mpsc;
use futures_util::{SinkExt, StreamExt};
use worker::*;

pub use client_object::ClientObject;

const KEEPALIVE: Duration = Duration::from_secs(25);
/// Header carrying a call's cancellation key to the client's object.
pub(crate) const CANCEL_KEY: &str = "x-clientmcp-cancel-key";

#[event(fetch)]
async fn fetch(req: Request, env: Env, ctx: Context) -> Result<Response> {
    let config = match config(&env) {
        Ok(config) => config,
        Err(e) => return Response::error(format!("bridge misconfigured: {e}"), 500),
    };
    let path = req.path();
    match http::route(&path) {
        Route::Connect => connect(req, &env, &config).await,
        Route::Mcp { token } => {
            let token = token.map(str::to_owned);
            agent(req, &env, Rc::new(ctx), &config, token.as_deref()).await
        }
        Route::Health => Response::ok("ok"),
        Route::NotFound => Response::error("not found", 404),
    }
}

/// The client channel handshake (SPEC §3.1): authenticate here, then hand the
/// upgrade to the client's Durable Object.
async fn connect(req: Request, env: &Env, config: &Config) -> Result<Response> {
    let headers = core_headers(&req);
    if !headers
        .get("upgrade")
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
    {
        return convert(http::Response::text(426, "expected a WebSocket upgrade"));
    }
    match http::accept_client(config, &headers, now()) {
        Ok(id) => {
            clients(env)?
                .get_by_name(&id)?
                .fetch_with_request(req)
                .await
        }
        Err(ClientRefusal::NotClientMcp) => convert(http::Response::text(
            400,
            &format!("offer the {SUBPROTOCOL} subprotocol and a token"),
        )),
        Err(ClientRefusal::Unauthorized) => {
            let pair = WebSocketPair::new()?;
            pair.server.accept()?;
            pair.server
                .close(Some(CLOSE_UNAUTHORIZED), Some("invalid or expired token"))?;
            websocket_response(pair.client)
        }
    }
}

/// The agent endpoint (SPEC §4).
async fn agent(
    mut req: Request,
    env: &Env,
    ctx: Rc<Context>,
    config: &Config,
    token: Option<&str>,
) -> Result<Response> {
    let method = req.method().to_string();
    let headers = core_headers(&req);
    let admitted = match http::admit_agent(config, &method, token, &headers, now()) {
        Ok(admitted) => admitted,
        Err(response) => return convert(response),
    };
    let body = req.bytes().await?;
    if body.len() > MAX_MESSAGE_BYTES {
        return convert(http::Response::text(413, "request body too large"));
    }
    let mut response = match agent::parse(&method, &headers, &body) {
        Op::Reply(response) => convert(response)?,
        Op::Listen(listen) => listen_stream(env, &admitted, listen).await?,
        Op::CallTool(call) => {
            let key = call
                .cancel_key
                .unwrap_or_else(|| clientmcp_core::random_hex(8));
            let forwarded = forward(&req, &body, Some(&key))?;
            let stub = clients(env)?.get_by_name(&admitted.client)?;
            let pending = Box::pin(async move { stub.fetch_with_request(forwarded).await });
            if call.streamable {
                let canceller = clients(env)?.get_by_name(&admitted.client)?;
                stream_call(
                    pending,
                    CancelOnDrop {
                        ctx,
                        stub: Some(canceller),
                        key,
                    },
                )
                .await?
            } else {
                pending.await?
            }
        }
        // Everything else needs the client's state.
        _ => {
            let forwarded = forward(&req, &body, None)?;
            clients(env)?
                .get_by_name(&admitted.client)?
                .fetch_with_request(forwarded)
                .await?
        }
    };
    if let Some(origin) = &admitted.cors {
        for (name, value) in http::cors_headers(origin) {
            response.headers_mut().append(name, &value)?;
        }
    }
    Ok(response)
}

/// The request for the client's object, carrying the agent's request.
fn forward(req: &Request, body: &[u8], cancel_key: Option<&str>) -> Result<Request> {
    let headers = req.headers().clone();
    if let Some(key) = cancel_key {
        headers.set(CANCEL_KEY, key)?;
    }
    let init = RequestInit {
        method: req.method(),
        headers,
        body: Some(js_sys::Uint8Array::from(body).into()),
        ..RequestInit::default()
    };
    Request::new_with_init("https://client/mcp", &init)
}

/// Answers a call (SPEC §4.3). One that takes longer than [`CALL_KEEPALIVE`]
/// becomes an SSE stream of keep-alive comments ending with the response: a
/// write to an agent that has gone away cancels the stream, which drops
/// `cancel`, which tells the client's object to cancel the call.
async fn stream_call(
    mut pending: Pin<Box<impl Future<Output = Result<Response>> + 'static>>,
    mut cancel: CancelOnDrop,
) -> Result<Response> {
    if let Ok(response) = either_or(&mut pending, Delay::from(CALL_KEEPALIVE)).await {
        cancel.disarm();
        return response;
    }
    let events = futures_util::stream::unfold(Some((pending, cancel)), |state| async move {
        let (mut pending, mut cancel) = state?;
        match either_or(&mut pending, Delay::from(CALL_KEEPALIVE)).await {
            Ok(response) => {
                cancel.disarm();
                let body = response.ok()?.text().await.ok()?;
                Some((Ok::<_, Error>(http::sse_event(&body).into_bytes()), None))
            }
            Err(()) => Some((
                Ok(SSE_KEEPALIVE.as_bytes().to_vec()),
                Some((pending, cancel)),
            )),
        }
    });
    sse_response(Response::from_stream(events)?)
}

/// Tells the client's object to cancel a call, unless disarmed first.
struct CancelOnDrop {
    ctx: Rc<Context>,
    stub: Option<Stub>,
    key: String,
}

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.stub = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(stub) = self.stub.take() else { return };
        let headers = Headers::new();
        let _ = headers.set(CANCEL_KEY, &self.key);
        let init = RequestInit {
            method: Method::Post,
            headers,
            ..RequestInit::default()
        };
        if let Ok(req) = Request::new_with_init("https://client/cancel", &init) {
            // The agent is gone; `wait_until` keeps the Worker running to deliver this.
            self.ctx.wait_until(async move {
                let _ = stub.fetch_with_request(req).await;
            });
        }
    }
}

fn sse_response(response: Response) -> Result<Response> {
    let headers = Headers::new();
    for (name, value) in SSE_HEADERS {
        headers.set(name, value)?;
    }
    Ok(response.with_headers(headers))
}

/// A notification stream (SPEC §4.4). The Worker holds the agent's SSE stream
/// and listens to the Durable Object over a hibernatable WebSocket, so neither
/// keeps the object awake.
async fn listen_stream(env: &Env, admitted: &Admitted, listen: Listen) -> Result<Response> {
    let upgrade = Headers::new();
    upgrade.set("upgrade", "websocket")?;
    let init = RequestInit {
        headers: upgrade,
        ..RequestInit::default()
    };
    let req = Request::new_with_init("https://client/listen", &init)?;
    let response = clients(env)?
        .get_by_name(&admitted.client)?
        .fetch_with_request(req)
        .await?;
    if response.status_code() != 101 {
        return Ok(response); // refused, e.g. too many streams
    }
    let Some(ws) = response.websocket() else {
        return Response::error("no WebSocket from the client object", 500);
    };
    let (tx, rx) = mpsc::channel::<Vec<u8>>(8);
    wasm_bindgen_futures::spawn_local(feed(ws, listen, tx));

    sse_response(Response::from_stream(rx.map(Ok::<_, Error>))?)
}

/// Turns the object's change signals into SSE events until either side goes away.
async fn feed(ws: WebSocket, listen: Listen, mut tx: mpsc::Sender<Vec<u8>>) {
    let Ok(mut events) = ws.events() else { return };
    if ws.accept().is_err() {
        return;
    }
    let mut open = match listen.opening() {
        Some(event) => tx.send(event.into_bytes()).await.is_ok(),
        None => true,
    };
    while open {
        let chunk = match either_or(events.next(), Delay::from(KEEPALIVE)).await {
            Ok(Some(Ok(WebsocketEvent::Message(_)))) => match listen.changed() {
                Some(event) => event.into_bytes(),
                None => continue,
            },
            Ok(_) => break, // the object closed the socket
            Err(()) => SSE_KEEPALIVE.as_bytes().to_vec(),
        };
        open = tx.send(chunk).await.is_ok();
    }
    let _ = ws.close(Some(1000), Some("stream ended"));
}

pub(crate) fn clients(env: &Env) -> Result<ObjectNamespace> {
    env.durable_object("CLIENTS")
}

pub(crate) fn config(env: &Env) -> std::result::Result<Config, String> {
    Config::from_vars(|name| env.var(name).ok().map(|v| v.to_string()))
}

pub(crate) fn core_headers(req: &Request) -> CoreHeaders {
    req.headers().entries().collect()
}

pub(crate) fn convert(response: http::Response) -> Result<Response> {
    let headers = Headers::new();
    for (name, value) in &response.headers {
        // A value echoed from a request may not be a valid header value; drop it.
        let _ = headers.append(name, value);
    }
    let builder = ResponseBuilder::new()
        .with_status(response.status)
        .with_headers(headers);
    Ok(if response.body.is_empty() {
        builder.empty()
    } else {
        builder.fixed(response.body.into_bytes())
    })
}

/// The `101` response completing a client handshake.
pub(crate) fn websocket_response(client: WebSocket) -> Result<Response> {
    Ok(ResponseBuilder::new()
        .with_header("sec-websocket-protocol", SUBPROTOCOL)?
        .with_websocket(client)
        .with_status(101)
        .empty())
}

async fn either_or<A: Future, B: Future>(a: A, b: B) -> std::result::Result<A::Output, B::Output> {
    let mut a = pin!(a);
    let mut b = pin!(b);
    std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(value) = a.as_mut().poll(cx) {
            return std::task::Poll::Ready(Ok(value));
        }
        b.as_mut().poll(cx).map(Err)
    })
    .await
}

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}
