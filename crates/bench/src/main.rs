//! Load generator for a ClientMCP bridge.
//!
//! ```text
//! clientmcp-bench call --url http://127.0.0.1:8080 --clients 1 --agents 64 --seconds 10 [--payload 100] [--modern]
//! clientmcp-bench list --url ...          tools/list: the bridge alone, no round trip to a client
//! clientmcp-bench idle --url ... --clients 10000 --seconds 30
//! ```
//!
//! Echo clients connect over WebSocket and answer every call with its
//! arguments; agents send `tools/call` back to back over keep-alive HTTP.
//! Tokens are minted from `CLIENTMCP_SECRET`.

use std::time::{Duration, Instant};

use bytes::Bytes;
use clientmcp_core::token::{Claims, Keys, Role};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct Args {
    mode: String,
    url: String,
    clients: usize,
    agents: usize,
    seconds: u64,
    payload: usize,
    modern: bool,
}

fn args() -> Args {
    let mut argv = std::env::args().skip(1);
    let mut args = Args {
        mode: argv.next().unwrap_or_else(|| "call".into()),
        url: "http://127.0.0.1:8080".into(),
        clients: 1,
        agents: 32,
        seconds: 10,
        payload: 16,
        modern: false,
    };
    while let Some(flag) = argv.next() {
        let mut value = || argv.next().expect("flag value");
        match flag.as_str() {
            "--url" => args.url = value(),
            "--clients" => args.clients = value().parse().unwrap(),
            "--agents" => args.agents = value().parse().unwrap(),
            "--seconds" => args.seconds = value().parse().unwrap(),
            "--payload" => args.payload = value().parse().unwrap(),
            "--modern" => args.modern = true,
            other => panic!("unknown flag {other}"),
        }
    }
    args
}

#[tokio::main]
async fn main() {
    let args = args();
    let keys = Keys::parse(&std::env::var("CLIENTMCP_SECRET").expect("CLIENTMCP_SECRET")).unwrap();
    let token = |role, sub: &str| {
        keys.sign(&Claims {
            sub: sub.into(),
            role,
            exp: None,
            nbf: None,
        })
    };
    let ws_url = format!("{}/connect", args.url.replacen("http", "ws", 1));

    let started = Instant::now();
    let mut ready = Vec::new();
    for i in 0..args.clients {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(echo_client(
            ws_url.clone(),
            token(Role::Client, &format!("bench-{i}")),
            tx,
        ));
        ready.push(rx);
        if i % 500 == 499 {
            tokio::time::sleep(Duration::from_millis(20)).await; // don't overflow the accept queue
        }
    }
    let mut online = 0;
    for rx in ready {
        online += usize::from(rx.await.is_ok());
    }
    println!(
        "{online}/{} clients online in {:.2?}",
        args.clients,
        started.elapsed()
    );

    if args.mode == "idle" {
        tokio::time::sleep(Duration::from_secs(args.seconds)).await;
        return;
    }

    let client = Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(args.agents)
        .build_http::<Full<Bytes>>();
    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let data = "x".repeat(args.payload);
    let mut agents = Vec::new();
    for a in 0..args.agents {
        let client = client.clone();
        let url = format!("{}/mcp", args.url);
        let auth = format!(
            "Bearer {}",
            token(Role::Agent, &format!("bench-{}", a % args.clients.max(1)))
        );
        let (method, params) = match args.mode.as_str() {
            "list" => ("tools/list", json!({})),
            _ => (
                "tools/call",
                json!({ "name": "echo", "arguments": { "data": data } }),
            ),
        };
        let (body, headers) = request(method, params, args.modern);
        agents.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut errors = 0u64;
            while Instant::now() < deadline {
                let mut req = hyper::Request::post(&url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("authorization", &auth);
                for (name, value) in &headers {
                    req = req.header(*name, value);
                }
                let req = req.body(Full::new(body.clone())).unwrap();
                let start = Instant::now();
                let ok = match client.request(req).await {
                    Ok(response) => {
                        let status = response.status();
                        let bytes = response.into_body().collect().await.map(|b| b.to_bytes());
                        status == 200
                            && bytes.is_ok_and(|b| {
                                !contains(&b, b"\"isError\":true") && !contains(&b, b"\"error\"")
                            })
                    }
                    Err(_) => false,
                };
                latencies.push(start.elapsed());
                errors += u64::from(!ok);
            }
            (latencies, errors)
        }));
    }

    let mut latencies = Vec::new();
    let mut errors = 0;
    for agent in agents {
        let (l, e) = agent.await.unwrap();
        latencies.extend(l);
        errors += e;
    }
    latencies.sort_unstable();
    let n = latencies.len();
    let pct = |p: f64| latencies[((n as f64 * p) as usize).min(n - 1)];
    println!(
        "{} {} agents={} clients={} payload={}B: {n} requests, {errors} errors, {:.0} req/s",
        args.mode,
        if args.modern { "modern" } else { "legacy" },
        args.agents,
        args.clients,
        args.payload,
        n as f64 / args.seconds as f64,
    );
    println!(
        "latency p50={:.2?} p90={:.2?} p99={:.2?} p99.9={:.2?} max={:.2?}",
        pct(0.50),
        pct(0.90),
        pct(0.99),
        pct(0.999),
        latencies[n - 1]
    );
}

/// A request body and its headers, in the legacy or modern era.
fn request(method: &str, mut params: Value, modern: bool) -> (Bytes, Vec<(&'static str, String)>) {
    let mut headers = Vec::new();
    if modern {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        headers.push(("mcp-protocol-version", "2026-07-28".to_string()));
        headers.push(("mcp-method", method.to_string()));
        if let Some(name) = params["name"].as_str() {
            headers.push(("mcp-name", name.to_string()));
        }
    }
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    (Bytes::from(body.to_string()), headers)
}

/// A client exposing `echo`, which returns its `data` argument.
async fn echo_client(url: String, token: String, ready: oneshot::Sender<()>) {
    let mut request = url.into_client_request().unwrap();
    let protocols = format!("clientmcp.v1, {token}").parse().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", protocols);
    let Ok((socket, _)) = tokio_tungstenite::connect_async(request).await else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let register = json!({ "type": "register", "tools": [{
        "name": "echo",
        "inputSchema": { "type": "object", "properties": { "data": { "type": "string" } } }
    }]});
    if sink
        .send(Message::Text(register.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    let mut ready = Some(ready);
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = ping.tick() => {
                let _ = sink.send(Message::Text(r#"{"type":"ping"}"#.into())).await;
                continue;
            }
        };
        let Some(Ok(Message::Text(text))) = message else {
            if matches!(message, Some(Ok(_))) {
                continue;
            } else {
                return;
            }
        };
        let message: Value = serde_json::from_str(&text).unwrap();
        match message["type"].as_str() {
            Some("registered") => {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
            Some("call") => {
                let data = message["arguments"]["data"].as_str().unwrap_or_default();
                let result = json!({ "type": "result", "id": message["id"], "result": {
                    "content": [{ "type": "text", "text": data }]
                }});
                let _ = sink.send(Message::Text(result.to_string().into())).await;
            }
            _ => {}
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
