//! Where clients live: one entry per client ID that is online, or awaited by
//! an agent's notification stream. This map is the server's whole state.
//!
//! Lock order: the client map, then a client's connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use clientmcp_core::MAX_LISTENERS;
use clientmcp_core::session::{self, Link, Session};
use clientmcp_core::wire::CLOSE_REPLACED;
use tokio::sync::{mpsc, watch};

/// A message for a connection's writer task.
pub enum Out {
    Text(String),
    Close(u16, String),
}

#[derive(Clone)]
pub struct Outbox(pub mpsc::UnboundedSender<Out>);

impl session::Outbox for Outbox {
    fn send(&self, text: &str) {
        let _ = self.0.send(Out::Text(text.to_owned()));
    }
}

impl Outbox {
    pub fn close(&self, code: u16, reason: impl Into<String>) {
        let _ = self.0.send(Out::Close(code, reason.into()));
    }
}

pub struct Client {
    conn: Mutex<Option<Conn>>,
    /// Signalled whenever the tool list changes.
    changes: watch::Sender<()>,
}

struct Conn {
    serial: u64,
    session: Session<Outbox>,
}

impl Link for Client {
    type Outbox = Outbox;

    fn with<R>(&self, f: impl FnOnce(Option<&mut Session<Outbox>>) -> R) -> R {
        f(self.conn.lock().unwrap().as_mut().map(|c| &mut c.session))
    }
}

impl Client {
    /// Runs `f` on connection `serial`'s session, unless a newer connection replaced it.
    pub fn session<R>(&self, serial: u64, f: impl FnOnce(&mut Session<Outbox>) -> R) -> Option<R> {
        let mut conn = self.conn.lock().unwrap();
        conn.as_mut()
            .filter(|c| c.serial == serial)
            .map(|c| f(&mut c.session))
    }

    pub fn announce_change(&self) {
        self.changes.send_replace(());
    }
}

#[derive(Default)]
pub struct Hub {
    clients: Mutex<HashMap<String, Arc<Client>>>,
    serial: AtomicU64,
}

impl Hub {
    /// The client, if it is online or awaited.
    pub fn get(&self, id: &str) -> Option<Arc<Client>> {
        self.clients.lock().unwrap().get(id).cloned()
    }

    /// Makes `session` the client's connection. A previous connection is
    /// closed with 4409 and its calls fail (SPEC §3.1).
    pub fn attach(&self, id: &str, session: Session<Outbox>) -> (Arc<Client>, u64) {
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        // Under the map lock, so that a concurrent `release` cannot orphan the entry.
        let (client, previous) = {
            let mut clients = self.clients.lock().unwrap();
            let client = clients
                .entry(id.to_owned())
                .or_insert_with(new_client)
                .clone();
            let previous = client
                .conn
                .lock()
                .unwrap()
                .replace(Conn { serial, session });
            (client, previous)
        };
        if let Some(previous) = previous {
            previous
                .session
                .outbox()
                .close(CLOSE_REPLACED, "replaced by a newer connection");
            if !previous.session.tools().is_empty() {
                client.announce_change();
            }
        }
        (client, serial)
    }

    /// Ends connection `serial`, if it is still the client's connection.
    pub fn detach(&self, id: &str, client: &Client, serial: u64) {
        let ended = {
            let mut conn = client.conn.lock().unwrap();
            match conn.as_ref() {
                Some(c) if c.serial == serial => conn.take(),
                _ => None,
            }
        };
        // Dropping the session fails its calls in flight.
        if ended.is_some_and(|c| !c.session.tools().is_empty()) {
            client.announce_change();
        }
        self.release(id);
    }

    /// Subscribes to the client's tool list changes; `None` if it has too many listeners.
    pub fn listen(&self, id: &str) -> Option<(Arc<Client>, watch::Receiver<()>)> {
        let mut clients = self.clients.lock().unwrap();
        let client = clients
            .entry(id.to_owned())
            .or_insert_with(new_client)
            .clone();
        if client.changes.receiver_count() >= MAX_LISTENERS {
            drop(clients);
            self.release(id);
            return None;
        }
        let changes = client.changes.subscribe();
        Some((client, changes))
    }

    /// Forgets a client that is neither online nor awaited.
    pub fn release(&self, id: &str) {
        let mut clients = self.clients.lock().unwrap();
        let idle = clients
            .get(id)
            .is_some_and(|c| c.conn.lock().unwrap().is_none() && c.changes.receiver_count() == 0);
        if idle {
            clients.remove(id);
        }
    }

    /// Closes every client connection.
    pub fn close_all(&self, code: u16, reason: &str) {
        for client in self.clients.lock().unwrap().values() {
            if let Some(conn) = client.conn.lock().unwrap().as_ref() {
                conn.session.outbox().close(code, reason);
            }
        }
    }

    /// Number of clients online.
    pub fn online(&self) -> usize {
        let clients = self.clients.lock().unwrap();
        clients
            .values()
            .filter(|c| c.conn.lock().unwrap().is_some())
            .count()
    }
}

fn new_client() -> Arc<Client> {
    Arc::new(Client {
        conn: Mutex::new(None),
        changes: watch::Sender::new(()),
    })
}
