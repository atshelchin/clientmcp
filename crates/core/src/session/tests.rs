use std::cell::RefCell;
use std::future::{Future, pending, ready};
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use serde_json::{Value, json};

use super::*;
use crate::agent::{Op, parse};
use crate::http::Headers;
use crate::wire::{CLOSE_PROTOCOL, PING};

/// Records what the session sends.
#[derive(Clone, Default)]
struct Sent(Rc<RefCell<Vec<String>>>);

impl Outbox for Sent {
    fn send(&self, text: &str) {
        self.0.borrow_mut().push(text.to_string());
    }
}

impl Sent {
    fn take(&self) -> Vec<Value> {
        self.0
            .borrow_mut()
            .drain(..)
            .map(|t| serde_json::from_str(&t).unwrap())
            .collect()
    }
}

/// A link to an optional session, as a runtime would keep it.
struct Slot(RefCell<Option<Session<Sent>>>);

impl Link for Slot {
    type Outbox = Sent;
    fn with<R>(&self, f: impl FnOnce(Option<&mut Session<Sent>>) -> R) -> R {
        f(self.0.borrow_mut().as_mut())
    }
}

const REGISTER: &str = r#"{"type":"register","instructions":"hi","tools":[{"name":"echo","inputSchema":{"type":"object"}}]}"#;

fn online() -> (Slot, Sent) {
    let sent = Sent::default();
    let mut session = Session::new(sent.clone());
    assert_eq!(
        session.on_message(REGISTER),
        Ok(Event::Registered {
            tools_changed: true
        })
    );
    assert_eq!(sent.take(), vec![json!({ "type": "registered" })]);
    (Slot(RefCell::new(Some(session))), sent)
}

fn legacy_call(name: &str, session: Option<&str>) -> Call {
    let body = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": name, "arguments": { "x": 1 } } });
    let headers: Headers = session.map(|s| ("mcp-session-id", s)).into_iter().collect();
    match parse("POST", &headers, &serde_json::to_vec(&body).unwrap()) {
        Op::CallTool(call) => call,
        other => panic!("{other:?}"),
    }
}

fn poll<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn result_message(id: &str, result: Value) -> String {
    json!({ "type": "result", "id": id, "result": result }).to_string()
}

#[test]
fn register_replaces_state() {
    let (slot, _) = online();
    slot.with(|s| {
        let s = s.unwrap();
        assert_eq!(s.instructions(), Some("hi"));
        assert_eq!(
            s.on_message(REGISTER),
            Ok(Event::Registered {
                tools_changed: false
            })
        );
        let empty = r#"{"type":"register","tools":[]}"#;
        assert_eq!(
            s.on_message(empty),
            Ok(Event::Registered {
                tools_changed: true
            })
        );
        assert!(s.tools().is_empty() && s.instructions().is_none());
        let bad = s.on_message(
            r#"{"type":"register","tools":[{"name":"bad name","inputSchema":{"type":"object"}}]}"#,
        );
        assert_eq!(bad.unwrap_err().code, CLOSE_PROTOCOL);
        assert_eq!(s.on_message("nonsense").unwrap_err().code, CLOSE_PROTOCOL);
    });
}

#[test]
fn ping_and_restore() {
    let (slot, sent) = online();
    slot.with(|s| s.unwrap().on_message(PING)).unwrap();
    assert_eq!(sent.take(), vec![json!({ "type": "pong" })]);
    let quiet = Sent::default();
    let restored = Session::restore(quiet.clone(), REGISTER);
    assert_eq!(restored.tools().len(), 1);
    assert_eq!(restored.instructions(), Some("hi"));
    assert!(
        quiet.take().is_empty(),
        "restoring is not a new registration"
    );
}

#[test]
fn call_completes_with_the_client_result() {
    let (slot, sent) = online();
    let request = legacy_call("echo", None);
    let mut future = pin!(call(&slot, &request, pending::<()>()));
    assert!(poll(future.as_mut()).is_pending());

    let messages = sent.take();
    assert_eq!(messages[0]["type"], "call");
    assert_eq!(
        (messages[0]["name"].as_str(), &messages[0]["arguments"]),
        (Some("echo"), &json!({ "x": 1 }))
    );
    let id = messages[0]["id"].as_str().unwrap().to_string();

    let result = json!({ "content": [{ "type": "text", "text": "1" }] });
    slot.with(|s| s.unwrap().on_message(&result_message(&id, result)))
        .unwrap();
    let Poll::Ready(Outcome::Done(_)) = poll(future.as_mut()) else {
        panic!()
    };
    assert_eq!(slot.with(|s| s.unwrap().in_flight()), 0);
    // A duplicate or late result is ignored.
    slot.with(|s| {
        s.unwrap()
            .on_message(&result_message(&id, json!({ "content": [] })))
    })
    .unwrap();
    assert!(sent.take().is_empty());
}

#[test]
fn a_malformed_result_is_a_protocol_error() {
    let (slot, sent) = online();
    let request = legacy_call("echo", None);
    let mut future = pin!(call(&slot, &request, pending::<()>()));
    assert!(poll(future.as_mut()).is_pending());
    let id = sent.take()[0]["id"].as_str().unwrap().to_string();
    let close = slot.with(|s| {
        s.unwrap()
            .on_message(&result_message(&id, json!({ "text": "no content" })))
    });
    assert_eq!(close.unwrap_err().code, CLOSE_PROTOCOL);
    // The runtime closes the connection, which fails the call.
    *slot.0.borrow_mut() = None;
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::Disconnected)
    ));
}

#[test]
fn call_failures() {
    let offline = Slot(RefCell::new(None));
    let request = legacy_call("echo", None);
    let mut future = pin!(call(&offline, &request, pending::<()>()));
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::Offline)
    ));

    let (slot, sent) = online();
    let unknown = legacy_call("missing", None);
    let mut future = pin!(call(&slot, &unknown, pending::<()>()));
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::UnknownTool)
    ));
    assert!(sent.take().is_empty());
}

#[test]
fn timeout_cancels_the_call() {
    let (slot, sent) = online();
    let request = legacy_call("echo", None);
    let mut future = pin!(call(&slot, &request, ready(())));
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::Timeout)
    ));
    let messages = sent.take();
    assert_eq!(
        (messages[1]["type"].as_str(), &messages[1]["id"]),
        (Some("cancel"), &messages[0]["id"])
    );
    assert_eq!(slot.with(|s| s.unwrap().in_flight()), 0);
}

#[test]
fn dropping_the_call_cancels_it() {
    let (slot, sent) = online();
    let request = legacy_call("echo", None);
    {
        let mut future = Box::pin(call(&slot, &request, pending::<()>()));
        assert!(poll(future.as_mut()).is_pending());
    }
    let messages = sent.take();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["type"], "cancel");
}

#[test]
fn disconnect_fails_calls_in_flight() {
    let (slot, sent) = online();
    let request = legacy_call("echo", None);
    let mut future = pin!(call(&slot, &request, pending::<()>()));
    assert!(poll(future.as_mut()).is_pending());
    // The connection ends, or a newer one replaces it.
    *slot.0.borrow_mut() = Some(Session::new(sent.clone()));
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::Disconnected)
    ));
    assert_eq!(
        sent.take().len(),
        1,
        "no cancel is sent to the new connection"
    );
}

#[test]
fn legacy_cancellation_and_busy() {
    let (slot, sent) = online();
    let request = legacy_call("echo", Some("s1"));
    let mut future = pin!(call(&slot, &request, pending::<()>()));
    assert!(poll(future.as_mut()).is_pending());
    let key = request.cancel_key.as_ref().unwrap();
    assert!(slot.with(|s| s.unwrap().cancel(key)));
    assert_eq!(sent.take()[1]["type"], "cancel");
    assert!(matches!(
        poll(future.as_mut()),
        Poll::Ready(Outcome::Cancelled)
    ));
    // Nothing in flight under that key any more (the Worker remembers such a
    // cancel until its call arrives).
    assert!(!slot.with(|s| s.unwrap().cancel(key)));

    slot.with(|s| {
        let s = s.unwrap();
        for _ in 0..MAX_IN_FLIGHT {
            assert!(s.start_call(&request).is_ok());
        }
        assert!(matches!(s.start_call(&request), Err(Outcome::Busy)));
    });
}
