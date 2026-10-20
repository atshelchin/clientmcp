//! A connected client: its declared state and its calls in flight (SPEC §3, §5).

use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::task::Poll;

use futures_channel::oneshot;

use crate::agent::{Call, Outcome};
use crate::tools::ToolSet;
use crate::wire::{
    self, CLOSE_PROTOCOL, CLOSE_TOO_BIG, Inbound, MAX_MESSAGE_BYTES, PONG, REGISTERED,
};

/// Most calls in flight per client.
pub const MAX_IN_FLIGHT: usize = 256;

/// Sends text messages on a client connection.
pub trait Outbox {
    fn send(&self, text: &str);
}

/// Instructs the runtime to close the client connection.
#[derive(Debug, PartialEq, Eq)]
pub struct Close {
    pub code: u16,
    pub reason: String,
}

impl Close {
    fn protocol(reason: impl Into<String>) -> Close {
        let mut reason = reason.into();
        // A close frame's reason is limited to 123 bytes.
        while reason.len() > 123 {
            reason.pop();
        }
        Close {
            code: CLOSE_PROTOCOL,
            reason,
        }
    }
}

/// What a client message changed.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Nothing,
    /// A `register` was accepted; `tools_changed` if the tool list differs.
    Registered {
        tools_changed: bool,
    },
}

/// The state of one client connection. Dropping it fails the calls in flight.
pub struct Session<O> {
    outbox: O,
    tools: ToolSet,
    instructions: Option<String>,
    calls: HashMap<String, InFlight>,
    /// Call ID by cancellation key.
    keyed: HashMap<String, String>,
    /// Random per session, so a late result can never match a newer call.
    prefix: String,
    next: u64,
}

struct InFlight {
    done: oneshot::Sender<Outcome>,
    cancel_key: Option<String>,
}

/// A call sent to the client.
pub struct Dispatched {
    pub id: String,
    pub outcome: oneshot::Receiver<Outcome>,
}

impl<O: Outbox> Session<O> {
    pub fn new(outbox: O) -> Session<O> {
        Session {
            outbox,
            tools: ToolSet::default(),
            instructions: None,
            calls: HashMap::new(),
            keyed: HashMap::new(),
            prefix: crate::random_hex(6),
            next: 0,
        }
    }

    /// Rebuilds a session from the last accepted `register` message, for
    /// runtimes that evict state while the connection stays open.
    pub fn restore(outbox: O, register: &str) -> Session<O> {
        let mut session = Session::new(outbox);
        if let Ok(Inbound::Register {
            tools,
            instructions,
        }) = Inbound::parse(register)
        {
            session.tools = ToolSet::parse(&tools).unwrap_or_default();
            session.instructions = instructions;
        }
        session
    }

    pub fn outbox(&self) -> &O {
        &self.outbox
    }

    pub fn tools(&self) -> &ToolSet {
        &self.tools
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Handles a text message from the client.
    pub fn on_message(&mut self, text: &str) -> Result<Event, Close> {
        if text.len() > MAX_MESSAGE_BYTES {
            return Err(Close {
                code: CLOSE_TOO_BIG,
                reason: "message too large".into(),
            });
        }
        match Inbound::parse(text).map_err(Close::protocol)? {
            Inbound::Register {
                tools,
                instructions,
            } => {
                let tools = ToolSet::parse(&tools).map_err(Close::protocol)?;
                let tools_changed = tools.json() != self.tools.json();
                self.tools = tools;
                self.instructions = instructions;
                self.outbox.send(REGISTERED);
                Ok(Event::Registered { tools_changed })
            }
            Inbound::Result { id, result } => {
                // Unknown IDs are late answers to finished calls: ignore them.
                if let Some(call) = self.remove(&id) {
                    let _ = call.done.send(Outcome::Done(result));
                }
                Ok(Event::Nothing)
            }
            Inbound::Ping => {
                self.outbox.send(PONG);
                Ok(Event::Nothing)
            }
            Inbound::Unknown => Ok(Event::Nothing),
        }
    }

    /// Sends a call to the client, or says why it cannot be made.
    pub fn start_call(&mut self, call: &Call) -> Result<Dispatched, Outcome> {
        let Some(tool) = self.tools.get(&call.name) else {
            return Err(Outcome::UnknownTool);
        };
        if call.reply.era == crate::agent::Era::Modern {
            tool.check_param_headers(&call.arguments, &call.headers)
                .map_err(Outcome::HeaderMismatch)?;
        }
        if self.calls.len() >= MAX_IN_FLIGHT {
            return Err(Outcome::Busy);
        }
        self.next += 1;
        let id = format!("{}{:x}", self.prefix, self.next);
        let (done, outcome) = oneshot::channel();
        if let Some(key) = &call.cancel_key {
            self.keyed.insert(key.clone(), id.clone());
        }
        let cancel_key = call.cancel_key.clone();
        self.calls.insert(id.clone(), InFlight { done, cancel_key });
        self.outbox
            .send(&wire::call(&id, &call.name, &call.arguments));
        Ok(Dispatched { id, outcome })
    }

    /// Gives up on a call: the client is told to stop, and its result will be ignored.
    pub fn abandon(&mut self, id: &str) {
        if let Some(call) = self.remove(id) {
            self.outbox.send(&wire::cancel(id));
            let _ = call.done.send(Outcome::Cancelled);
        }
    }

    /// Cancels the call with [`Call::cancel_key`] `key`, if it is in flight.
    pub fn cancel(&mut self, key: &str) {
        if let Some(id) = self.keyed.get(key).cloned() {
            self.abandon(&id);
        }
    }

    pub fn in_flight(&self) -> usize {
        self.calls.len()
    }

    fn remove(&mut self, id: &str) -> Option<InFlight> {
        let call = self.calls.remove(id)?;
        if let Some(key) = &call.cancel_key {
            self.keyed.remove(key);
        }
        Some(call)
    }
}

/// Access to the session of one client, whichever connection currently holds it.
pub trait Link {
    type Outbox: Outbox;

    /// Runs `f` on the client's session, or on `None` while the client is offline.
    fn with<R>(&self, f: impl FnOnce(Option<&mut Session<Self::Outbox>>) -> R) -> R;
}

/// Performs a call: dispatches it, then waits for the result, the client's
/// disconnection or `timeout`. If the returned future is dropped first (the
/// agent went away), the client is told to cancel.
pub async fn call<L: Link>(link: &L, call: &Call, timeout: impl Future<Output = ()>) -> Outcome {
    let dispatched = link.with(|session| match session {
        Some(session) => session.start_call(call),
        None => Err(Outcome::Offline),
    });
    let Dispatched { id, outcome } = match dispatched {
        Ok(dispatched) => dispatched,
        Err(outcome) => return outcome,
    };
    let mut guard = AbandonOnDrop { link, id: Some(id) };
    match race(outcome, timeout).await {
        Ok(result) => {
            guard.id = None; // finished: the result arrived, or the session ended
            result.unwrap_or(Outcome::Disconnected)
        }
        Err(()) => Outcome::Timeout, // the guard cancels the call
    }
}

struct AbandonOnDrop<'a, L: Link> {
    link: &'a L,
    id: Option<String>,
}

impl<L: Link> Drop for AbandonOnDrop<'_, L> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.link.with(|session| {
                if let Some(session) = session {
                    session.abandon(&id);
                }
            });
        }
    }
}

/// Resolves with whichever future finishes first.
async fn race<A: Future, B: Future>(a: A, b: B) -> Result<A::Output, B::Output> {
    let mut a = pin!(a);
    let mut b = pin!(b);
    poll_fn(|cx| {
        if let Poll::Ready(value) = a.as_mut().poll(cx) {
            return Poll::Ready(Ok(value));
        }
        b.as_mut().poll(cx).map(Err)
    })
    .await
}

#[cfg(test)]
mod tests;
