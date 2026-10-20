//! The client channel: one task per WebSocket connection (SPEC §3).

use std::sync::Arc;
use std::time::Duration;

use clientmcp_core::session::{Event, Session};
use clientmcp_core::wire::{CLOSE_PROTOCOL, CLOSE_TOO_BIG, CLOSE_UNAUTHORIZED, MAX_MESSAGE_BYTES};
use futures_util::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error, Message};

use crate::hub::{Hub, Out, Outbox};

/// A client that sends nothing for this long is presumed gone (SPEC §3.2).
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

type Socket = WebSocketStream<TokioIo<Upgraded>>;

pub async fn accept(upgraded: Upgraded) -> Socket {
    // Most connections sit idle: tungstenite's default 128 KiB buffers would
    // dominate their cost. Reads grow past 4 KiB as messages require; writes go
    // out at once, which `send` does anyway.
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, Some(config)).await
}

/// Closes a connection whose token was refused.
pub async fn refuse(mut socket: Socket) {
    let _ = socket
        .close(Some(close_frame(
            CLOSE_UNAUTHORIZED,
            "invalid or expired token",
        )))
        .await;
}

/// Serves a connection for client `id` until it ends. A writer task drains
/// the session's outbox while this task reads, so a busy client's calls and
/// results flow in parallel.
pub async fn serve(hub: Arc<Hub>, id: String, socket: Socket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let outbox = Outbox(tx);
    let (client, serial) = hub.attach(&id, Session::new(outbox.clone()));

    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            let message = match out {
                Out::Text(text) => Message::Text(text.into()),
                Out::Close(code, reason) => Message::Close(Some(close_frame(code, &reason))),
            };
            let closing = matches!(message, Message::Close(_));
            if sink.send(message).await.is_err() || closing {
                break;
            }
        }
        // Completes the closing handshake, whichever side started it.
        let _ = sink.close().await;
    });

    let close = loop {
        let text = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
            Err(_) => break Some((1000, "idle".into())),
            Ok(Some(Ok(Message::Text(text)))) => text,
            // Ping and pong frames are answered by tungstenite.
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)))) => continue,
            Ok(Some(Ok(Message::Binary(_)))) => {
                break Some((
                    CLOSE_PROTOCOL,
                    "binary messages are not part of the protocol".into(),
                ));
            }
            Ok(Some(Err(Error::Capacity(_)))) => {
                break Some((CLOSE_TOO_BIG, "message too large".into()));
            }
            Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) => break None,
        };
        match client.session(serial, |s| s.on_message(&text)) {
            None => break None, // replaced: the newer connection's attach closed this one
            Some(Ok(Event::Registered {
                tools_changed: true,
            })) => client.announce_change(),
            Some(Ok(_)) => {}
            Some(Err(close)) => break Some((close.code, close.reason)),
        }
    };

    if let Some((code, reason)) = close {
        outbox.close(code, reason);
    }
    hub.detach(&id, &client, serial);
    drop(outbox); // the writer ends once every sender is gone
    let _ = writer.await;
}

fn close_frame(code: u16, reason: &str) -> CloseFrame {
    CloseFrame {
        code: code.into(),
        reason: reason.into(),
    }
}
