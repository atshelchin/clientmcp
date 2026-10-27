//! What a client uses, counted exactly per UTC day (`docs/self-serve.md`,
//! "Metering and alerts").
//!
//! Resource use only, never content: how many calls and how they ended, the
//! bytes that crossed, how long the client held its connection, and how many
//! credentials its object refused. No arguments, no results, no tokens, no
//! addresses. A bridge keeps one [`Day`] per client per day and writes it out
//! when the day is over.
//!
//! Pure: times are the caller's (epoch milliseconds).

use serde::{Deserialize, Serialize};

use crate::agent::Outcome;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// How a call ended, as metering counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Ok,
    ToolError,
    Offline,
    Timeout,
    Cancelled,
    Disconnected,
    /// Refused before the client saw it: unknown tool, mismatched headers, busy.
    Refused,
    /// Held back by the protective rate limit.
    Limited,
}

impl Ended {
    pub fn of(outcome: &Outcome) -> Ended {
        match outcome {
            Outcome::Done(result) if result.is_error() => Ended::ToolError,
            Outcome::Done(_) => Ended::Ok,
            Outcome::Offline => Ended::Offline,
            Outcome::Timeout => Ended::Timeout,
            Outcome::Cancelled => Ended::Cancelled,
            Outcome::Disconnected => Ended::Disconnected,
            Outcome::UnknownTool | Outcome::HeaderMismatch(_) | Outcome::Busy => Ended::Refused,
            Outcome::Limited => Ended::Limited,
        }
    }

    /// The name metering stores it under.
    pub fn name(self) -> &'static str {
        match self {
            Ended::Ok => "ok",
            Ended::ToolError => "tool_error",
            Ended::Offline => "offline",
            Ended::Timeout => "timeout",
            Ended::Cancelled => "cancelled",
            Ended::Disconnected => "disconnected",
            Ended::Refused => "refused",
            Ended::Limited => "limited",
        }
    }
}

/// One client's use on one UTC day.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub calls_ok: u64,
    pub calls_tool_error: u64,
    pub calls_offline: u64,
    pub calls_timeout: u64,
    pub calls_cancelled: u64,
    pub calls_disconnected: u64,
    pub calls_refused: u64,
    pub calls_limited: u64,
    /// Agent requests other than calls (listing tools, describing the server).
    pub requests: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Milliseconds the client held its connection.
    pub connected_ms: u64,
    pub connections: u64,
    /// Credentials this client's object refused (a rotated or revoked token).
    pub rejections: u64,
}

impl Day {
    pub fn new(day: String) -> Day {
        Day {
            day,
            ..Day::default()
        }
    }

    pub fn call(&mut self, ended: Ended, bytes_in: u64, bytes_out: u64) {
        let counter = match ended {
            Ended::Ok => &mut self.calls_ok,
            Ended::ToolError => &mut self.calls_tool_error,
            Ended::Offline => &mut self.calls_offline,
            Ended::Timeout => &mut self.calls_timeout,
            Ended::Cancelled => &mut self.calls_cancelled,
            Ended::Disconnected => &mut self.calls_disconnected,
            Ended::Refused => &mut self.calls_refused,
            Ended::Limited => &mut self.calls_limited,
        };
        *counter += 1;
        self.bytes_in += bytes_in;
        self.bytes_out += bytes_out;
    }

    pub fn request(&mut self, bytes_in: u64, bytes_out: u64) {
        self.requests += 1;
        self.bytes_in += bytes_in;
        self.bytes_out += bytes_out;
    }

    pub fn calls(&self) -> u64 {
        self.calls_ok
            + self.calls_tool_error
            + self.calls_offline
            + self.calls_timeout
            + self.calls_cancelled
            + self.calls_disconnected
            + self.calls_refused
            + self.calls_limited
    }
}

/// The UTC day of `epoch_ms`, as `YYYY-MM-DD`.
pub fn day_of(epoch_ms: u64) -> String {
    let days = (epoch_ms / DAY_MS) as i64;
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The first millisecond of the UTC day after `epoch_ms`.
pub fn next_day_ms(epoch_ms: u64) -> u64 {
    (epoch_ms / DAY_MS + 1) * DAY_MS
}

/// A connection held from `from_ms` to `to_ms`, split across the UTC days it
/// spans: `(day, milliseconds)` in order.
pub fn connection_days(from_ms: u64, to_ms: u64) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut at = from_ms;
    while at < to_ms {
        let end = next_day_ms(at).min(to_ms);
        out.push((day_of(at), end - at));
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_utc_dates() {
        assert_eq!(day_of(0), "1970-01-01");
        assert_eq!(day_of(1_793_059_200_000), "2026-10-27");
        assert_eq!(day_of(1_793_059_200_000 - 1), "2026-10-26");
        assert_eq!(day_of(951_782_400_000), "2000-02-29");
        assert_eq!(
            next_day_ms(1_793_059_200_000 + 5),
            1_793_059_200_000 + DAY_MS
        );
    }

    #[test]
    fn a_connection_is_counted_on_each_day_it_spans() {
        let midnight = 1_793_059_200_000;
        let split = connection_days(midnight - 1_000, midnight + 2_500);
        assert_eq!(
            split,
            vec![
                ("2026-10-26".to_owned(), 1_000),
                ("2026-10-27".to_owned(), 2_500)
            ]
        );
        assert!(connection_days(5, 5).is_empty());
    }

    #[test]
    fn calls_are_counted_by_how_they_ended_and_bytes_add_up() {
        let mut day = Day::new("2026-10-27".into());
        day.call(Ended::Ok, 100, 2_000);
        day.call(Ended::Offline, 80, 60);
        day.call(Ended::Limited, 90, 50);
        day.request(40, 900);
        assert_eq!(
            (
                day.calls_ok,
                day.calls_offline,
                day.calls_limited,
                day.requests
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(day.calls(), 3);
        assert_eq!((day.bytes_in, day.bytes_out), (310, 3_010));
    }

    #[test]
    fn what_is_stored_reads_back() {
        let mut day = Day::new("2026-10-27".into());
        day.call(Ended::Timeout, 1, 2);
        day.connected_ms = 61_000;
        let text = serde_json::to_string(&day).unwrap();
        assert_eq!(serde_json::from_str::<Day>(&text).unwrap(), day);
    }
}
