//! The client against the native bridge, for what the conformance suite does
//! not reach: reconnecting, tokens, closing and connecting again, and a
//! cancelled call's handler being dropped. (The suite itself runs with this
//! crate as the application: `CLIENTMCP_APP=rust tests/run-native.sh`.)

use std::net::TcpListener as StdListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use clientmcp_client::{Client, Options, Reply, Status, Tool, accepted};
use clientmcp_core::token::{Claims, Keys, Role};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

const SECRET: &str = "conformance-secret-for-the-rust-client-tests-0123456789";

fn bridge_binary() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let status = Command::new(env!("CARGO"))
                .args(["build", "-q", "-p", "clientmcp-server"])
                .current_dir(&root)
                .status()
                .expect("cargo runs");
            assert!(status.success(), "the bridge builds");
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map_or_else(|| root.join("target"), PathBuf::from);
            target.join("debug/clientmcp")
        })
        .clone()
}

struct Bridge {
    port: u16,
    child: Child,
}

impl Bridge {
    async fn start() -> Bridge {
        let port = StdListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Bridge::on(port).await
    }

    async fn on(port: u16) -> Bridge {
        // Killed and waited for when dropped, even if it never listens.
        let bridge = Bridge {
            port,
            child: Command::new(bridge_binary())
                .env("CLIENTMCP_SECRET", SECRET)
                .env("CLIENTMCP_LISTEN", format!("127.0.0.1:{port}"))
                .env("CLIENTMCP_CALL_TIMEOUT", "5")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("the bridge starts"),
        };
        for _ in 0..200 {
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return bridge;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("the bridge never listened on {port}");
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}/connect", self.port)
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

fn token(role: Role, id: &str) -> String {
    Keys::parse(SECRET).unwrap().sign(&Claims {
        sub: id.to_owned(),
        role,
        exp: None,
        nbf: None,
        generation: None,
    })
}

/// The statuses a client went through, with the close codes.
type Seen = Arc<Mutex<Vec<(Status, Option<u16>)>>>;

fn recorded(options: Options) -> (Options, Seen) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let options = options
        .on_status(move |status, close| sink.lock().unwrap().push((status, close.map(|c| c.code))));
    (options, seen)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// An MCP 2026-07-28 request to the bridge's agent endpoint, as raw HTTP/1.1.
fn agent_request(port: u16, id: &str, method: &str, params: &Value) -> String {
    let mut params = params.clone();
    params["_meta"] = json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {} });
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(|n| format!("Mcp-Name: {n}\r\n"))
        .unwrap_or_default();
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2026-07-28\r\nMcp-Method: {method}\r\n{name}\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        token(Role::Agent, id),
        body.len(),
    )
}

/// The JSON-RPC response, whether answered as JSON or as an event stream.
async fn agent(port: u16, id: &str, method: &str, params: Value) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(agent_request(port, id, method, &params).as_bytes())
        .await
        .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, mut body) = raw.split_once("\r\n\r\n").expect("an HTTP response");
    let mut decoded = String::new();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        while let Some((size, rest)) = body.split_once("\r\n") {
            let size = usize::from_str_radix(size.trim(), 16).unwrap();
            if size == 0 {
                break;
            }
            decoded.push_str(&rest[..size]);
            body = &rest[size + 2..];
        }
    } else {
        decoded.push_str(body);
    }
    let json = decoded
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .next_back()
        .unwrap_or(&decoded);
    serde_json::from_str(json).unwrap_or_else(|_| panic!("a JSON-RPC response, got {raw}"))
}

async fn call(port: u16, id: &str, name: &str, arguments: Value) -> Value {
    agent(
        port,
        id,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await["result"]
        .clone()
}

fn echo() -> Tool {
    Tool::new(|args, _| async move { Ok(Reply::Json(args)) })
}

#[tokio::test(flavor = "multi_thread")]
async fn it_reconnects_when_the_bridge_comes_back() {
    let mut bridge = Bridge::start().await;
    let (options, seen) = recorded(Options::new(bridge.url(), token(Role::Client, "back")));
    let client = Client::new(options);
    client.expose("echo", echo());
    client.expose(
        "pay",
        Tool::new(|_, _| async {
            let mut data = Map::new();
            data.insert("transfer".into(), json!({ "amount": "1" }));
            Ok(accepted("op-7", data).expect("no reserved member"))
        }),
    );
    client.connect().await.expect("online");
    let paid = call(bridge.port, "back", "pay", json!({})).await;
    assert_eq!(
        paid["structuredContent"],
        json!({ "transfer": { "amount": "1" }, "status": "accepted", "operation": "op-7" })
    );

    let port = bridge.port;
    bridge.stop();
    until("offline", || client.status() == Status::Offline).await;
    let _bridge = Bridge::on(port).await;
    until("online again", || client.status() == Status::Online).await;
    assert_eq!(
        call(port, "back", "echo", json!({ "value": 3 })).await["structuredContent"],
        json!({ "value": 3 })
    );
    until("Online reported", || {
        seen.lock().unwrap().last().map(|(s, _)| *s) == Some(Status::Online)
    })
    .await;
    let statuses: Vec<Status> = seen.lock().unwrap().iter().map(|(s, _)| *s).collect();
    assert!(statuses.contains(&Status::Offline), "{statuses:?}");
    client.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fixed_token_that_is_refused_closes_for_good() {
    let bridge = Bridge::start().await;
    let (options, seen) = recorded(Options::new(bridge.url(), token(Role::Agent, "wrong-role")));
    let client = Client::new(options);
    let close = client.connect().await.expect_err("refused");
    assert_eq!(close.code, 4401);
    assert_eq!(client.status(), Status::Closed);
    // The callback runs outside the client's lock, so it may land just after
    // connect() resolves.
    until("Closed 4401 reported", || {
        seen.lock().unwrap().last() == Some(&(Status::Closed, Some(4401)))
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_token_is_asked_for_each_connection_and_an_unauthorized_close_retried() {
    let bridge = Bridge::start().await;
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let options = Options::new(bridge.url(), "").fresh_token(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        // The first token has expired; the next one is good.
        async move {
            Ok(if n == 0 {
                token(Role::Agent, "fresh")
            } else {
                token(Role::Client, "fresh")
            })
        }
    });
    let client = Client::new(options);
    client.expose("echo", echo());
    timeout(Duration::from_secs(10), client.connect())
        .await
        .expect("in time")
        .expect("online");
    assert_eq!(asked.load(Ordering::SeqCst), 2);
    assert_eq!(
        call(bridge.port, "fresh", "echo", json!({ "value": 1 })).await["structuredContent"],
        json!({ "value": 1 })
    );
    client.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_after_close_connects_again() {
    let bridge = Bridge::start().await;
    let client = Client::new(Options::new(bridge.url(), token(Role::Client, "again")));
    client.expose("echo", echo());
    client.connect().await.expect("online");
    client.close();
    assert_eq!(client.status(), Status::Closed);
    let offline = call(bridge.port, "again", "echo", json!({})).await;
    assert_eq!(offline["isError"], true);
    client.connect().await.expect("online again");
    assert_eq!(
        call(bridge.port, "again", "echo", json!({ "value": 2 })).await["structuredContent"],
        json!({ "value": 2 })
    );
    client.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_hanging_up_drops_the_handler_and_cancels_its_token() {
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let bridge = Bridge::start().await;
    let client = Client::new(Options::new(bridge.url(), token(Role::Client, "hangup")));
    let (started, dropped) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let token_seen: Arc<Mutex<Option<clientmcp_client::CancellationToken>>> = Arc::default();
    let (s, d, t) = (
        Arc::clone(&started),
        Arc::clone(&dropped),
        Arc::clone(&token_seen),
    );
    client.expose(
        "hang",
        Tool::new(move |_, ctx| {
            let (s, d, t) = (Arc::clone(&s), Arc::clone(&d), Arc::clone(&t));
            async move {
                *t.lock().unwrap() = Some(ctx.cancelled.clone());
                let _guard = Dropped(d);
                s.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
                Ok(Reply::Empty)
            }
        }),
    );
    client.connect().await.expect("online");
    let mut request = TcpStream::connect(("127.0.0.1", bridge.port))
        .await
        .unwrap();
    let hang = agent_request(
        bridge.port,
        "hangup",
        "tools/call",
        &json!({ "name": "hang", "arguments": {} }),
    );
    request.write_all(hang.as_bytes()).await.unwrap();
    until("the call to start", || started.load(Ordering::SeqCst) == 1).await;
    // Within the first second, before the bridge switches to an event stream.
    drop(request);
    until("the handler to be dropped", || {
        dropped.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(
        token_seen
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(clientmcp_client::CancellationToken::is_cancelled)
    );
    assert_eq!(client.status(), Status::Online);
    client.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wss_url_goes_through_rustls_and_a_failed_handshake_is_retried() {
    // Not a TLS server: the handshake fails. Without a crypto provider of its
    // own the client would panic here instead, and stay "connecting".
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        }
    });
    let (options, seen) = recorded(Options::new(
        format!("wss://127.0.0.1:{port}/connect"),
        "token",
    ));
    let client = Client::new(options);
    let connecting = client.clone();
    tokio::spawn(async move { connecting.connect().await });
    // A client starts offline: the one that counts follows an attempt.
    until("offline after connecting", || {
        seen.lock()
            .unwrap()
            .starts_with(&[(Status::Connecting, None), (Status::Offline, None)])
    })
    .await;
    client.close();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dial_hook_opens_every_connection() {
    let bridge = Bridge::start().await;
    let dialled = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&dialled);
    let options =
        Options::new(bridge.url(), token(Role::Client, "dialled")).dial(move |host, port| {
            seen.lock().unwrap().push((host.clone(), port));
            async move { TcpStream::connect((host.as_str(), port)).await }
        });
    let client = Client::new(options);
    client.expose("echo", echo());
    client.connect().await.expect("online through the hook");
    assert_eq!(
        *dialled.lock().unwrap(),
        vec![("127.0.0.1".to_owned(), bridge.port)]
    );
    assert_eq!(
        call(bridge.port, "dialled", "echo", json!({ "value": 4 })).await["structuredContent"],
        json!({ "value": 4 })
    );
    client.close();
}
