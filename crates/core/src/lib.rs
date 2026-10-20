//! The runtime-independent core of the ClientMCP bridge.
//!
//! Everything protocol-related lives here, free of I/O: authentication, the
//! client channel ([`wire`], [`session`]), tool validation ([`tools`]) and
//! the MCP agent endpoint ([`agent`]). A runtime supplies sockets, timers and
//! the place where sessions live — a process-wide map for the server, one
//! Durable Object per client on Cloudflare.

pub mod agent;
pub mod http;
mod json;
pub mod session;
pub mod token;
pub mod tools;
pub mod wire;

use std::time::Duration;

use http::Origins;
use token::Keys;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Most notification streams open per client.
pub const MAX_LISTENERS: usize = 64;

/// Bridge configuration, shared by every runtime.
#[derive(Clone, Debug)]
pub struct Config {
    pub keys: Keys,
    pub call_timeout: Duration,
    pub origins: Origins,
}

impl Config {
    /// Reads the configuration from variables:
    ///
    /// - `CLIENTMCP_SECRET` (required): token secrets, newest first, whitespace-separated
    /// - `CLIENTMCP_CALL_TIMEOUT`: call timeout in seconds, default 120
    /// - `CLIENTMCP_ALLOWED_ORIGINS`: browser origins allowed on the agent endpoint, or `*`
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let keys = Keys::parse(&var("CLIENTMCP_SECRET").unwrap_or_default())
            .map_err(|e| format!("CLIENTMCP_SECRET: {e}"))?;
        let call_timeout = match var("CLIENTMCP_CALL_TIMEOUT") {
            None => Duration::from_secs(120),
            Some(secs) => match secs.trim().parse::<u64>() {
                Ok(secs) if secs > 0 => Duration::from_secs(secs),
                _ => {
                    return Err(
                        "CLIENTMCP_CALL_TIMEOUT: expected a positive number of seconds".into(),
                    );
                }
            },
        };
        let origins = Origins::parse(&var("CLIENTMCP_ALLOWED_ORIGINS").unwrap_or_default());
        Ok(Config {
            keys,
            call_timeout,
            origins,
        })
    }
}

/// `bytes` random bytes, hex-encoded.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the platform provides randomness");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
