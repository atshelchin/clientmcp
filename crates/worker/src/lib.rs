//! The ClientMCP bridge on Cloudflare Workers.
//!
//! The Worker is the stateless front door: it routes, checks origins, verifies
//! tokens and holds agents' notification streams. Everything about a client
//! lives in its Durable Object ([`client_object`]), addressed by client ID.

mod client_object;
mod internal;
mod metering;

use std::cell::RefCell;
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
use futures_channel::{mpsc, oneshot};
use futures_util::{SinkExt, StreamExt};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use worker::*;

pub use client_object::ClientObject;

const KEEPALIVE: Duration = Duration::from_secs(25);
/// Header carrying a call's cancellation key to the client's object.
pub(crate) const CANCEL_KEY: &str = "x-clientmcp-cancel-key";
/// Header carrying the presented token's generation to the client's object,
/// which keeps the generations (`docs/self-serve.md`). Always set by the
/// Worker, over anything a request arrived with.
pub(crate) const GENERATION: &str = "x-clientmcp-generation";
/// Header naming the client to its object, which does not know its own name
/// (metering writes it beside the counts). Always set by the Worker.
pub(crate) const CLIENT_ID: &str = "x-clientmcp-client";

#[event(fetch)]
async fn fetch(req: Request, env: Env, ctx: Context) -> Result<Response> {
    let config = match config(&env) {
        Ok(config) => config,
        Err(e) => return Response::error(format!("bridge misconfigured: {e}"), 500),
    };
    let path = req.path();
    if path.starts_with("/internal/") {
        return internal::handle(req, &env, &config, now()).await;
    }
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
    match http::accept_client_bound(config, &headers, now()) {
        Ok(bound) => {
            let forwarded = with_generation(&req, &bound.client, bound.generation)?;
            clients(env)?
                .get_by_name(&bound.client)?
                .fetch_with_request(forwarded)
                .await
        }
        Err(ClientRefusal::NotClientMcp) => convert(http::Response::text(
            400,
            &format!("offer the {SUBPROTOCOL} subprotocol and a token"),
        )),
        Err(ClientRefusal::Unauthorized) => refuse_client("invalid or expired token"),
    }
}

/// A handshake completed only to say why it is refused: `4401` (SPEC §3.3).
pub(crate) fn refuse_client(reason: &str) -> Result<Response> {
    let pair = WebSocketPair::new()?;
    pair.server.accept()?;
    pair.server.close(Some(CLOSE_UNAUTHORIZED), Some(reason))?;
    websocket_response(pair.client)
}

/// The client's handshake, for its object, carrying its token's generation.
fn with_generation(req: &Request, client: &str, generation: u64) -> Result<Request> {
    let headers = req.headers().clone();
    headers.set(GENERATION, &generation.to_string())?;
    headers.set(CLIENT_ID, client)?;
    let init = RequestInit {
        method: req.method(),
        headers,
        ..RequestInit::default()
    };
    Request::new_with_init(req.url()?.as_ref(), &init)
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
    let response = match agent::parse(&method, &headers, &body) {
        Op::Reply(response) => convert(response)?,
        Op::Listen(listen) => listen_stream(env, &admitted, listen).await?,
        Op::CallTool(call) => {
            // The protective rate limit (docs/self-serve.md): only against
            // blatant abuse, and said to the model as a tool error.
            if limited(env, &admitted.client).await {
                signal_limited(env, &admitted.client, &call.name);
                return with_cors(convert(call.respond(agent::Outcome::Limited))?, &admitted);
            }
            let key = call
                .cancel_key
                .unwrap_or_else(|| clientmcp_core::random_hex(8));
            let forwarded = forward(&req, &body, Some(&key), &admitted)?;
            let stub = clients(env)?.get_by_name(&admitted.client)?;
            let pending = Box::pin(async move { stub.fetch_with_request(forwarded).await });
            let canceller = Canceller::new(ctx, clients(env)?.get_by_name(&admitted.client)?, key);
            // An agent hanging up before the answer: on Cloudflare the
            // invocation then ends without dropping anything, so only the
            // request's signal says so (the `enable_request_signal` flag).
            let on_hang_up = canceller.clone();
            on_abort(&req, move || {
                if on_hang_up.cancel() {
                    console_log!("clientmcp: an agent hung up; its call is cancelled");
                }
            });
            if call.streamable {
                stream_call(pending, CancelOnDrop(canceller)).await?
            } else {
                let response = pending.await;
                canceller.disarm();
                response?
            }
        }
        // Everything else needs the client's state.
        _ => {
            let forwarded = forward(&req, &body, None, &admitted)?;
            clients(env)?
                .get_by_name(&admitted.client)?
                .fetch_with_request(forwarded)
                .await?
        }
    };
    with_cors(response, &admitted)
}

fn with_cors(mut response: Response, admitted: &Admitted) -> Result<Response> {
    if let Some(origin) = &admitted.cors {
        for (name, value) in http::cors_headers(origin) {
            response.headers_mut().append(name, &value)?;
        }
    }
    Ok(response)
}

/// Whether the client's calls are over the protective limit. No binding, or
/// a limiter that fails to answer, limits nothing.
async fn limited(env: &Env, client: &str) -> bool {
    let Ok(limiter) = env.rate_limiter("AGENT_LIMIT") else {
        return false;
    };
    matches!(limiter.limit(client.to_owned()).await, Ok(outcome) if !outcome.success)
}

/// A limited call never reaches the client's object, so its signal is the
/// Worker's.
fn signal_limited(env: &Env, client: &str, tool: &str) {
    if let Ok(dataset) = env.analytics_engine("SIGNALS") {
        let point = AnalyticsEngineDataPointBuilder::new()
            .indexes([client])
            .blobs([client, "call", "limited", tool])
            .doubles([0.0, 0.0, 0.0])
            .build();
        let _ = dataset.write_data_point(&point);
    }
}

/// The request for the client's object, carrying the agent's request.
fn forward(
    req: &Request,
    body: &[u8],
    cancel_key: Option<&str>,
    admitted: &Admitted,
) -> Result<Request> {
    let headers = req.headers().clone();
    headers.set(GENERATION, &admitted.generation.to_string())?;
    headers.set(CLIENT_ID, &admitted.client)?;
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
/// `cancel`, which tells the client's object to cancel the call. (A hang-up
/// before that is the request's abort signal's, see [`agent`].)
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

/// Tells the client's object to cancel a call, once, unless disarmed first.
#[derive(Clone)]
struct Canceller(Rc<RefCell<Option<Cancel>>>);

/// What a cancel needs: the invocation to keep alive, the object, the key.
type Cancel = (Rc<Context>, Stub, String);

impl Canceller {
    fn new(ctx: Rc<Context>, stub: Stub, key: String) -> Self {
        Canceller(Rc::new(RefCell::new(Some((ctx, stub, key)))))
    }

    fn disarm(&self) {
        self.0.borrow_mut().take();
    }

    /// Whether this sent the cancel (it was neither disarmed nor sent).
    fn cancel(&self) -> bool {
        let Some((ctx, stub, key)) = self.0.borrow_mut().take() else {
            return false;
        };
        let headers = Headers::new();
        let _ = headers.set(CANCEL_KEY, &key);
        let init = RequestInit {
            method: Method::Post,
            headers,
            ..RequestInit::default()
        };
        if let Ok(req) = Request::new_with_init("https://client/cancel", &init) {
            // The agent is gone; `wait_until` keeps the Worker running to deliver this.
            ctx.wait_until(async move {
                let _ = stub.fetch_with_request(req).await;
            });
        }
        true
    }
}

/// Cancels the call when dropped, unless disarmed first.
struct CancelOnDrop(Canceller);

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.0.disarm();
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Runs `f` if the request's sender goes away before the response is done.
pub(crate) fn on_abort(req: &Request, f: impl FnOnce() + 'static) {
    let signal = req.inner().signal();
    if signal.aborted() {
        return f();
    }
    let listener = Closure::once_into_js(f);
    signal.set_onabort(Some(listener.unchecked_ref()));
}

/// Resolves if the request's sender goes away before the response is done.
pub(crate) async fn aborted(req: &Request) {
    let (tx, rx) = oneshot::channel();
    on_abort(req, move || {
        let _ = tx.send(());
    });
    if rx.await.is_err() {
        std::future::pending::<()>().await; // never aborted
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
    upgrade.set(GENERATION, &admitted.generation.to_string())?;
    upgrade.set(CLIENT_ID, &admitted.client)?;
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

pub(crate) async fn either_or<A: Future, B: Future>(
    a: A,
    b: B,
) -> std::result::Result<A::Output, B::Output> {
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
