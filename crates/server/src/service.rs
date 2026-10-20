//! The HTTP service: routing, the WebSocket handshake and the agent endpoint.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use clientmcp_core::Config;
use clientmcp_core::agent::{self, CALL_KEEPALIVE, Listen, Op, Outcome};
use clientmcp_core::http::{self, ClientRefusal, Headers, Route, SSE_HEADERS, SSE_KEEPALIVE};
use clientmcp_core::session::{self, Link};
use clientmcp_core::tools::ToolSet;
use clientmcp_core::wire::{MAX_MESSAGE_BYTES, SUBPROTOCOL};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Request, StatusCode};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

use crate::client;
use crate::hub::Hub;

pub type Body = UnsyncBoxBody<Bytes, Infallible>;
pub type Response = hyper::Response<Body>;

const KEEPALIVE: Duration = Duration::from_secs(25);

pub struct Bridge {
    pub config: Config,
    pub hub: Arc<Hub>,
    /// Becomes `true` when the server shuts down.
    pub shutdown: watch::Receiver<bool>,
}

impl Bridge {
    pub async fn handle(self: Arc<Self>, req: Request<Incoming>) -> Result<Response, Infallible> {
        let path = req.uri().path().to_owned();
        Ok(match http::route(&path) {
            Route::Connect => self.connect(req),
            Route::Mcp { token } => self.agent(req, token).await,
            Route::Health => convert(http::Response::text(200, "ok")),
            Route::NotFound => convert(http::Response::text(404, "not found")),
        })
    }

    /// The client channel handshake (SPEC §3.1).
    fn connect(self: Arc<Self>, mut req: Request<Incoming>) -> Response {
        let headers = core_headers(&req);
        let upgrade = headers
            .get("upgrade")
            .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
            && headers.get("sec-websocket-version") == Some("13");
        let (Some(key), true) = (headers.get("sec-websocket-key"), upgrade) else {
            return convert(
                http::Response::text(426, "expected a WebSocket upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-version", "13"),
            );
        };
        let accept = derive_accept_key(key.as_bytes());
        let client = match http::accept_client(&self.config, &headers, now()) {
            Err(ClientRefusal::NotClientMcp) => {
                let why = format!("offer the {SUBPROTOCOL} subprotocol and a token");
                return convert(http::Response::text(400, &why));
            }
            accepted => accepted.ok(),
        };
        let upgrading = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            let Ok(upgraded) = upgrading.await else {
                return;
            };
            let socket = client::accept(upgraded).await;
            match client {
                Some(id) => client::serve(self.hub.clone(), id, socket).await,
                None => client::refuse(socket).await,
            }
        });
        convert(
            http::Response::empty(101)
                .header("upgrade", "websocket")
                .header("connection", "Upgrade")
                .header("sec-websocket-accept", accept)
                .header("sec-websocket-protocol", SUBPROTOCOL),
        )
    }

    /// The agent endpoint (SPEC §4).
    async fn agent(&self, req: Request<Incoming>, token: Option<&str>) -> Response {
        let method = req.method().as_str().to_owned();
        let headers = core_headers(&req);
        let admitted = match http::admit_agent(&self.config, &method, token, &headers, now()) {
            Ok(admitted) => admitted,
            Err(response) => return convert(response),
        };
        let body = match Limited::new(req.into_body(), MAX_MESSAGE_BYTES)
            .collect()
            .await
        {
            Ok(body) => body.to_bytes(),
            Err(_) => return convert(http::Response::text(413, "request body too large")),
        };
        let id = admitted.client.as_str();
        let client = self.hub.get(id);
        let response = match agent::parse(&method, &headers, &body) {
            Op::Reply(response) => response,
            Op::Describe(describe) => {
                let instructions = client
                    .and_then(|c| c.with(|s| s.and_then(|s| s.instructions().map(str::to_owned))));
                describe.respond(instructions.as_deref())
            }
            Op::ListTools(reply) => {
                let none = ToolSet::default();
                match client {
                    Some(c) => c.with(|s| reply.tools(s.map_or(&none, |s| s.tools()))),
                    None => reply.tools(&none),
                }
            }
            Op::CallTool(call) => {
                let Some(client) = client else {
                    return finish(call.respond(Outcome::Offline), &admitted.cors);
                };
                let streamable = call.streamable;
                let timeout = tokio::time::sleep(self.config.call_timeout);
                // Owns everything it needs, so a response stream can carry it.
                let mut pending = Box::pin(async move {
                    let outcome = session::call(&*client, &call, timeout).await;
                    call.respond(outcome)
                });
                if !streamable {
                    pending.await
                } else {
                    match tokio::time::timeout(CALL_KEEPALIVE, &mut pending).await {
                        Ok(response) => response,
                        Err(_) => return stream_call(pending, &admitted.cors),
                    }
                }
            }
            Op::Cancel(key) => {
                if let Some(c) = client {
                    c.with(|s| s.map(|s| s.cancel(&key)));
                }
                agent::accepted()
            }
            Op::Listen(listen) => return self.listen(id, listen, admitted.cors),
        };
        finish(response, &admitted.cors)
    }

    /// A notification stream (SPEC §4.4), fed by a task that ends with the stream.
    fn listen(&self, id: &str, listen: Listen, cors: Option<String>) -> Response {
        let Some((client, mut changes)) = self.hub.listen(id) else {
            return convert(http::Response::text(429, "too many notification streams"));
        };
        let (tx, rx) = mpsc::channel::<Bytes>(8);
        let (hub, id, mut shutdown) = (self.hub.clone(), id.to_owned(), self.shutdown.clone());
        tokio::spawn(async move {
            let mut open = match listen.opening() {
                Some(event) => tx.send(event.into()).await.is_ok(),
                None => true,
            };
            while open {
                tokio::select! {
                    changed = changes.changed() => match (changed, listen.changed()) {
                        (Err(_), _) => break,
                        (Ok(()), Some(event)) => open = tx.send(event.into()).await.is_ok(),
                        (Ok(()), None) => {}
                    },
                    () = tokio::time::sleep(KEEPALIVE) => {
                        open = tx.send(Bytes::from_static(SSE_KEEPALIVE.as_bytes())).await.is_ok();
                    }
                    () = stopped(&mut shutdown) => {
                        if let Some(event) = listen.closing() {
                            let _ = tx.send(event.into()).await;
                        }
                        break;
                    }
                    () = tx.closed() => break, // the agent went away
                }
            }
            drop((changes, client));
            hub.release(&id);
        });

        let frames = futures_util::stream::unfold(rx, |mut rx| async move {
            let bytes = rx.recv().await?;
            Some((Ok(Frame::data(bytes)), rx))
        });
        sse_response(StreamBody::new(frames).boxed_unsync(), &cors)
    }
}

/// A slow call's response: keep-alive comments, then the response as an SSE
/// event. If the agent goes away, hyper drops the body and with it `pending`,
/// which cancels the call.
fn stream_call(
    pending: Pin<Box<impl Future<Output = http::Response> + Send + 'static>>,
    cors: &Option<String>,
) -> Response {
    let frames = futures_util::stream::unfold(Some(pending), |pending| async move {
        let mut pending = pending?;
        tokio::select! {
            response = &mut pending => Some((frame(http::sse_event(&response.body)), None)),
            () = tokio::time::sleep(CALL_KEEPALIVE) => {
                Some((frame(SSE_KEEPALIVE.to_owned()), Some(pending)))
            }
        }
    });
    sse_response(StreamBody::new(frames).boxed_unsync(), cors)
}

fn frame(text: String) -> Result<Frame<Bytes>, Infallible> {
    Ok(Frame::data(Bytes::from(text)))
}

fn sse_response(body: Body, cors: &Option<String>) -> Response {
    let mut response = hyper::Response::new(body);
    for (name, value) in SSE_HEADERS {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    if let Some(origin) = cors {
        for (name, value) in http::cors_headers(origin) {
            insert_header(&mut response, name, &value);
        }
    }
    response
}

/// Converts a complete response, adding CORS headers for a browser agent.
fn finish(mut response: http::Response, cors: &Option<String>) -> Response {
    if let Some(origin) = cors {
        http::add_cors(&mut response, origin);
    }
    convert(response)
}

fn core_headers<B>(req: &Request<B>) -> Headers {
    req.headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn convert(response: http::Response) -> Response {
    let mut converted = hyper::Response::new(Full::new(Bytes::from(response.body)).boxed_unsync());
    *converted.status_mut() = StatusCode::from_u16(response.status).expect("valid status");
    for (name, value) in &response.headers {
        insert_header(&mut converted, name, value);
    }
    converted
}

fn insert_header(response: &mut Response, name: &'static str, value: &str) {
    // A value echoed from a request may not be a valid header value; drop it.
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .append(HeaderName::from_static(name), value);
    }
}

/// Resolves once the server shuts down.
pub async fn stopped(shutdown: &mut watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|down| *down).await;
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
