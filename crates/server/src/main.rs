//! `clientmcp`: the ClientMCP bridge for Linux servers and Docker.

mod client;
mod hub;
mod service;

use std::net::SocketAddr;
use std::pin::pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clientmcp_core::token::{Claims, Role};
use clientmcp_core::wire::CLOSE_GOING_AWAY;
use clientmcp_core::{Config, VERSION};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::hub::Hub;
use crate::service::Bridge;

const USAGE: &str = "\
Usage:
  clientmcp [serve]                                  Run the bridge
  clientmcp token <client|agent> <client-id> [--ttl <duration>]
                                                     Mint a token (duration: 90s, 15m, 12h, 30d)

Environment:
  CLIENTMCP_SECRET           Token secrets, newest first, whitespace-separated (required, >= 32 bytes)
  CLIENTMCP_LISTEN           Listen address (default 127.0.0.1:8080)
  CLIENTMCP_CALL_TIMEOUT     Tool call timeout in seconds (default 120)
  CLIENTMCP_ALLOWED_ORIGINS  Browser origins allowed on /mcp, comma-separated, or * (default none)
";

/// How long shutdown waits for connections to finish.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("serve") => serve(),
        Some("token") => token(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("clientmcp {VERSION}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("clientmcp: {message}");
            ExitCode::FAILURE
        }
    }
}

fn config() -> Result<Config, String> {
    Config::from_vars(|name| std::env::var(name).ok())
}

fn token(args: &[String]) -> Result<(), String> {
    let usage =
        || "usage: clientmcp token <client|agent> <client-id> [--ttl <duration>]".to_string();
    let (role, sub, rest) = match args {
        [role, sub, rest @ ..] => (role, sub, rest),
        _ => return Err(usage()),
    };
    let role = match role.as_str() {
        "client" => Role::Client,
        "agent" => Role::Agent,
        _ => return Err(usage()),
    };
    if !clientmcp_core::token::is_client_id(sub) {
        return Err("a client ID is 1-128 characters from A-Z a-z 0-9 . _ - : @".into());
    }
    let exp = match rest {
        [] => None,
        [flag, ttl] if flag == "--ttl" => Some(service::now() + parse_duration(ttl)?),
        _ => return Err(usage()),
    };
    let claims = Claims {
        sub: sub.clone(),
        role,
        exp,
        nbf: None,
        generation: None,
    };
    println!("{}", config()?.keys.sign(&claims));
    Ok(())
}

fn parse_duration(text: &str) -> Result<u64, String> {
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => return Err(format!("invalid duration {text:?}")),
    };
    number
        .parse::<u64>()
        .map(|n| n * scale)
        .map_err(|_| format!("invalid duration {text:?}"))
}

fn serve() -> Result<(), String> {
    let config = config()?;
    let listen = std::env::var("CLIENTMCP_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let addr: SocketAddr = listen
        .parse()
        .map_err(|_| format!("CLIENTMCP_LISTEN: invalid address {listen:?}"))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime
        .block_on(run(config, addr))
        .map_err(|e| e.to_string())
}

async fn run(config: Config, addr: SocketAddr) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    eprintln!(
        "clientmcp {VERSION} listening on http://{}",
        listener.local_addr()?
    );

    let (shutdown, shutting_down) = watch::channel(false);
    let hub = Arc::new(Hub::default());
    let bridge = Arc::new(Bridge {
        config,
        hub: hub.clone(),
        shutdown: shutting_down.clone(),
    });
    let mut connections = JoinSet::new();
    let mut signal = pin!(shutdown_signal());

    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // Typically out of file descriptors: back off instead of spinning.
                    eprintln!("clientmcp: accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            () = &mut signal => break,
        };
        let _ = stream.set_nodelay(true);
        let bridge = bridge.clone();
        let service = service_fn(move |req| bridge.clone().handle(req));
        let mut shutting_down = shutting_down.clone();
        connections.spawn(async move {
            let mut conn = pin!(
                http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
            );
            tokio::select! {
                _ = conn.as_mut() => return,
                () = service::stopped(&mut shutting_down) => {}
            }
            conn.as_mut().graceful_shutdown();
            let _ = conn.await;
        });
        while connections.try_join_next().is_some() {}
    }

    eprintln!("clientmcp: shutting down");
    shutdown.send_replace(true);
    hub.close_all(CLOSE_GOING_AWAY, "bridge shutting down");
    let drained = async {
        while connections.join_next().await.is_some() {}
        while hub.online() > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, drained).await;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
