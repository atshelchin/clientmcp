//! Metering, in a client's object (`docs/self-serve.md`, "Metering and
//! alerts"): exact counts per UTC day, written to the bridge's database when
//! the day is over, and a fast signal per event to Analytics Engine.
//!
//! Resource use only — how calls ended, bytes, connection time, refused
//! credentials, a tool's name — never arguments, results or tokens.
//!
//! Every write is best effort. A bridge without the bindings (`BRIDGE_DB`,
//! `SIGNALS`) works exactly as before: metering never fails a call.

use clientmcp_core::usage::{Day, Ended, connection_days, day_of, next_day_ms};
use worker::*;

/// The client's ID, as the Worker last named it (an object does not know
/// its own name).
const CLIENT_ID: &str = "client_id";
/// When the current connection's time was last counted up to.
const CONNECTED_SINCE: &str = "connected_since";
/// One key per UTC day still to be written out.
const DAY_PREFIX: &str = "usage:";
/// A minute after midnight, so a day's last calls are in.
const AFTER_MIDNIGHT_MS: u64 = 60_000;

pub(crate) struct Meter<'a> {
    pub state: &'a State,
    pub env: &'a Env,
    /// The client's ID as this instance knows it: storage is touched once.
    pub named: &'a std::cell::RefCell<Option<String>>,
}

impl Meter<'_> {
    fn storage(&self) -> Storage {
        self.state.storage()
    }

    /// Remembers the client's ID the Worker passed along.
    pub async fn name(&self, id: Option<String>) {
        let Some(id) = id else { return };
        if self.named.borrow().as_deref() == Some(id.as_str()) {
            return;
        }
        let known: Option<String> = self.storage().get(CLIENT_ID).await.ok().flatten();
        if known.as_deref() != Some(id.as_str()) {
            let _ = self.storage().put(CLIENT_ID, &id).await;
        }
        *self.named.borrow_mut() = Some(id);
    }

    async fn client(&self) -> String {
        if let Some(id) = self.named.borrow().clone() {
            return id;
        }
        let stored: Option<String> = self.storage().get(CLIENT_ID).await.ok().flatten();
        stored.unwrap_or_default()
    }

    /// Applies `f` to `day`'s counts and keeps them, with an alarm to write
    /// them out once the day is over.
    async fn count(&self, day: String, f: impl FnOnce(&mut Day)) {
        let key = format!("{DAY_PREFIX}{day}");
        let mut counts: Day = self
            .storage()
            .get(&key)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| Day::new(day));
        f(&mut counts);
        let _ = self.storage().put(&key, &counts).await;
        self.ensure_alarm().await;
    }

    async fn ensure_alarm(&self) {
        if matches!(self.storage().get_alarm().await, Ok(Some(_))) {
            return;
        }
        let at = next_day_ms(now()) + AFTER_MIDNIGHT_MS;
        let _ = self.storage().set_alarm(at as i64).await;
    }

    /// A tool call, however it ended.
    pub async fn call(&self, ended: Ended, tool: &str, bytes_in: usize, bytes_out: usize, ms: u64) {
        self.count(day_of(now()), |d| {
            d.call(ended, bytes_in as u64, bytes_out as u64)
        })
        .await;
        self.signal(
            "call",
            ended.name(),
            tool,
            [bytes_in as f64, bytes_out as f64, ms as f64],
        )
        .await;
    }

    /// An agent request that is not a call (tools/list, server/discover…).
    pub async fn request(&self, bytes_in: usize, bytes_out: usize) {
        self.count(day_of(now()), |d| {
            d.request(bytes_in as u64, bytes_out as u64)
        })
        .await;
    }

    /// A credential this object refused: a rotated, revoked or deleted token.
    pub async fn rejected(&self, role: &str) {
        self.count(day_of(now()), |d| d.rejections += 1).await;
        self.signal("rejected", role, "", [0.0, 0.0, 0.0]).await;
    }

    /// A client connection began.
    pub async fn connected(&self) {
        let now = now();
        self.count(day_of(now), |d| d.connections += 1).await;
        let _ = self.storage().put(CONNECTED_SINCE, now).await;
        self.signal("connected", "", "", [0.0, 0.0, 0.0]).await;
    }

    /// The client connection ended: its time is counted on the days it spans.
    pub async fn disconnected(&self) {
        let Ok(Some(since)) = self.storage().get::<u64>(CONNECTED_SINCE).await else {
            return;
        };
        let now = now();
        self.add_connection(since, now).await;
        let _ = self.storage().delete(CONNECTED_SINCE).await;
        self.signal(
            "disconnected",
            "",
            "",
            [0.0, 0.0, (now.saturating_sub(since)) as f64],
        )
        .await;
    }

    async fn add_connection(&self, from: u64, to: u64) {
        for (day, ms) in connection_days(from, to) {
            self.count(day, |d| d.connected_ms += ms).await;
        }
    }

    /// The object's alarm: count an open connection up to now, write out
    /// every finished day, and wake again tomorrow if there is more to count.
    pub async fn alarm(&self) -> Result<()> {
        let now = now();
        if let Ok(Some(since)) = self.storage().get::<u64>(CONNECTED_SINCE).await {
            self.add_connection(since, now).await;
            let _ = self.storage().put(CONNECTED_SINCE, now).await;
        }
        let today = day_of(now);
        let client = self.client().await;
        let stored = self
            .storage()
            .list_with_options(ListOptions::new().prefix(DAY_PREFIX))
            .await?;
        let mut finished = Vec::new();
        for key in stored.keys() {
            let Some(key) = key?.as_string() else {
                continue;
            };
            let Some(day) = key.strip_prefix(DAY_PREFIX) else {
                continue;
            };
            if day < today.as_str() {
                finished.push(key);
            }
        }
        for key in finished {
            let Ok(Some(counts)) = self.storage().get::<Day>(&key).await else {
                continue;
            };
            // Kept until written: a failed write is tried again tomorrow.
            if write_day(self.env, &client, &counts).await.is_ok() {
                let _ = self.storage().delete(&key).await;
            }
        }
        let open = matches!(
            self.storage().get::<u64>(CONNECTED_SINCE).await,
            Ok(Some(_))
        );
        let pending = self
            .storage()
            .list_with_options(ListOptions::new().prefix(DAY_PREFIX))
            .await
            .map(|m| m.size() > 0)
            .unwrap_or(false);
        if open || pending {
            let _ = self
                .storage()
                .set_alarm((next_day_ms(now) + AFTER_MIDNIGHT_MS) as i64)
                .await;
        }
        Ok(())
    }

    /// The days not yet written out, today's included, oldest first.
    pub async fn pending(&self) -> Result<Vec<Day>> {
        let stored = self
            .storage()
            .list_with_options(ListOptions::new().prefix(DAY_PREFIX))
            .await?;
        let mut days = Vec::new();
        for value in stored.values() {
            if let Ok(day) = serde_wasm_bindgen::from_value::<Day>(value?) {
                days.push(day);
            }
        }
        days.sort_by(|a, b| a.day.cmp(&b.day));
        Ok(days)
    }

    /// One data point for the live view and alerts. Sampled under load by
    /// the platform; nothing depends on it being complete.
    async fn signal(&self, event: &str, detail: &str, tool: &str, numbers: [f64; 3]) {
        let Ok(dataset) = self.env.analytics_engine("SIGNALS") else {
            return;
        };
        let client = self.client().await;
        let point = AnalyticsEngineDataPointBuilder::new()
            .indexes([client.as_str()])
            .blobs([client.as_str(), event, detail, tool])
            .doubles(numbers)
            .build();
        let _ = dataset.write_data_point(&point);
    }
}

/// Writes one finished day. Absolute counts, so writing it twice is harmless.
async fn write_day(env: &Env, client: &str, d: &Day) -> Result<()> {
    let db = env.d1("BRIDGE_DB")?;
    let values: Vec<wasm_bindgen::JsValue> = vec![
        d.day.as_str().into(),
        client.into(),
        (d.calls_ok as f64).into(),
        (d.calls_tool_error as f64).into(),
        (d.calls_offline as f64).into(),
        (d.calls_timeout as f64).into(),
        (d.calls_cancelled as f64).into(),
        (d.calls_disconnected as f64).into(),
        (d.calls_refused as f64).into(),
        (d.calls_limited as f64).into(),
        (d.requests as f64).into(),
        (d.bytes_in as f64).into(),
        (d.bytes_out as f64).into(),
        (d.connected_ms as f64).into(),
        (d.connections as f64).into(),
        (d.rejections as f64).into(),
    ];
    db.prepare(
        "INSERT OR REPLACE INTO usage_daily (day, client, calls_ok, calls_tool_error, \
         calls_offline, calls_timeout, calls_cancelled, calls_disconnected, calls_refused, \
         calls_limited, requests, bytes_in, bytes_out, connected_ms, connections, rejections) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )
    .bind(&values)?
    .run()
    .await?;
    Ok(())
}

/// The issuance audit (`internal.rs`): what was minted, rotated or revoked,
/// for which client, at whose request. Written in the same request as the
/// action; a failure to write is logged, not hidden.
pub(crate) async fn audit(
    env: &Env,
    client: &str,
    action: &str,
    role: &str,
    generation: Option<u64>,
    actor: &str,
) {
    let Ok(db) = env.d1("BRIDGE_DB") else {
        console_log!("clientmcp: no BRIDGE_DB: {action} {role} not audited");
        return;
    };
    let values: Vec<wasm_bindgen::JsValue> = vec![
        (now() as f64).into(),
        client.into(),
        action.into(),
        role.into(),
        generation.map_or(wasm_bindgen::JsValue::NULL, |g| (g as f64).into()),
        actor.into(),
    ];
    let written = match db
        .prepare(
            "INSERT INTO issuance_audit (at, client, action, role, gen, actor) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&values)
    {
        Ok(statement) => statement.run().await.map(|_| ()),
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        console_error!("clientmcp: the issuance audit was not written: {e}");
    }
}

fn now() -> u64 {
    Date::now().as_millis()
}
