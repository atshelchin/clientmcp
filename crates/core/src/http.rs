//! HTTP plumbing both runtimes share: headers, responses, routing and the
//! checks every request passes before it reaches a client (SPEC §3.1, §4, §4.5).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::Config;
use crate::token::Role;
use crate::wire::SUBPROTOCOL;

/// Request headers with lowercase names.
#[derive(Clone, Debug, Default)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn push(&mut self, name: &str, value: impl Into<String>) {
        self.0.push((name.to_ascii_lowercase(), value.into()));
    }

    /// The headers whose names start with `prefix`, which must be lowercase.
    pub fn with_prefix(&self, prefix: &str) -> Headers {
        Headers(
            self.0
                .iter()
                .filter(|(n, _)| n.starts_with(prefix))
                .cloned()
                .collect(),
        )
    }

    /// The first value of a header; `name` must be lowercase.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

impl<K: AsRef<str>, V: Into<String>> FromIterator<(K, V)> for Headers {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut headers = Headers::default();
        for (name, value) in iter {
            headers.push(name.as_ref(), value);
        }
        headers
    }
}

/// A complete HTTP response, converted by each runtime into its own type.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl Response {
    pub fn empty(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn json(status: u16, body: String) -> Response {
        Response::empty(status)
            .header("content-type", "application/json")
            .body(body)
    }

    pub fn text(status: u16, body: &str) -> Response {
        Response::empty(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(body.to_string())
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Response {
        self.headers.push((name, value.into()));
        self
    }

    fn body(mut self, body: String) -> Response {
        self.body = body;
        self
    }
}

/// Response headers of an SSE stream.
pub const SSE_HEADERS: [(&str, &str); 3] = [
    ("content-type", "text/event-stream"),
    ("cache-control", "no-cache"),
    ("x-accel-buffering", "no"),
];

/// An SSE comment that keeps idle streams open through intermediaries.
pub const SSE_KEEPALIVE: &str = ":\n\n";

/// Frames a JSON message as an SSE event.
pub fn sse_event(json: &str) -> String {
    let mut event = String::with_capacity(json.len() + 8);
    for line in json.lines() {
        event.push_str("data: ");
        event.push_str(line);
        event.push('\n');
    }
    event.push('\n');
    event
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route<'a> {
    /// The client channel.
    Connect,
    /// The agent endpoint, optionally with the token as last path segment.
    Mcp {
        token: Option<&'a str>,
    },
    Health,
    NotFound,
}

pub fn route(path: &str) -> Route<'_> {
    match path {
        "/connect" => Route::Connect,
        "/mcp" => Route::Mcp { token: None },
        "/health" => Route::Health,
        _ => match path.strip_prefix("/mcp/") {
            Some(token) if !token.is_empty() && !token.contains('/') => {
                Route::Mcp { token: Some(token) }
            }
            _ => Route::NotFound,
        },
    }
}

/// Why a client handshake is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum ClientRefusal {
    /// Not a ClientMCP handshake: answer with plain HTTP 400.
    NotClientMcp,
    /// Complete the handshake, then close with 4401 so a browser can tell why.
    Unauthorized,
}

/// Authenticates a client handshake (SPEC §3.1) and returns its client ID.
pub fn accept_client(
    config: &Config,
    headers: &Headers,
    now: u64,
) -> Result<String, ClientRefusal> {
    let offered: Vec<&str> = headers
        .get("sec-websocket-protocol")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect();
    if !offered.contains(&SUBPROTOCOL) {
        return Err(ClientRefusal::NotClientMcp);
    }
    let mut tokens = offered.iter().filter(|p| !p.starts_with("clientmcp."));
    match (tokens.next(), tokens.next()) {
        (Some(token), None) => config
            .keys
            .verify(token, Role::Client, now)
            .map_err(|_| ClientRefusal::Unauthorized),
        _ => Err(ClientRefusal::Unauthorized),
    }
}

/// What passed the agent endpoint's front door.
#[derive(Debug)]
pub struct Admitted {
    pub client: String,
    /// The `Origin` to answer with CORS headers, if the request came from a browser.
    pub cors: Option<String>,
}

/// Origin checks, CORS preflight and authentication of an agent request.
/// `Err` is the complete response; `Ok` names the client the request is for.
pub fn admit_agent(
    config: &Config,
    method: &str,
    path_token: Option<&str>,
    headers: &Headers,
    now: u64,
) -> Result<Admitted, Response> {
    let cors = match headers.get("origin") {
        None => None,
        Some(origin) if config.origins.allows(origin) => Some(origin.to_string()),
        Some(_) => return Err(Response::text(403, "origin not allowed")),
    };
    if method == "OPTIONS" {
        let response = match &cors {
            Some(origin) => preflight(origin, headers),
            None => Response::empty(405).header("allow", "GET, POST, DELETE"),
        };
        return Err(response);
    }
    let token = path_token.or_else(|| {
        let value = headers.get("authorization")?;
        let (scheme, token) = value.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
    });
    let admitted = token
        .and_then(|token| config.keys.verify(token, Role::Agent, now).ok())
        .map(|client| Admitted {
            client,
            cors: cors.clone(),
        });
    admitted.ok_or_else(|| {
        let mut response = Response::text(401, "a valid agent token is required")
            .header("www-authenticate", "Bearer");
        if let Some(origin) = &cors {
            add_cors(&mut response, origin);
        }
        response
    })
}

fn preflight(origin: &str, headers: &Headers) -> Response {
    let mut response = Response::empty(204)
        .header("access-control-allow-methods", "GET, POST, DELETE")
        .header("access-control-max-age", "86400");
    if let Some(requested) = headers.get("access-control-request-headers") {
        response = response.header("access-control-allow-headers", requested);
    }
    add_cors(&mut response, origin);
    response
}

/// Adds the CORS headers for an allowed origin to a response.
pub fn add_cors(response: &mut Response, origin: &str) {
    response.headers.extend(cors_headers(origin));
}

pub fn cors_headers(origin: &str) -> [(&'static str, String); 3] {
    [
        ("access-control-allow-origin", origin.to_string()),
        (
            "access-control-expose-headers",
            "Mcp-Session-Id, WWW-Authenticate".into(),
        ),
        ("vary", "Origin".into()),
    ]
}

/// The browser origins allowed to call the agent endpoint.
#[derive(Clone, Debug)]
pub enum Origins {
    Any,
    List(Vec<String>),
}

impl Origins {
    /// Parses `*` or a comma-separated list of origins; empty allows none.
    pub fn parse(s: &str) -> Origins {
        if s.trim() == "*" {
            return Origins::Any;
        }
        let list = s
            .split(',')
            .map(|o| o.trim().trim_end_matches('/').to_string())
            .filter(|o| !o.is_empty())
            .collect();
        Origins::List(list)
    }

    pub fn allows(&self, origin: &str) -> bool {
        match self {
            Origins::Any => true,
            Origins::List(list) => list.iter().any(|o| o == origin),
        }
    }
}

/// Decodes an `Mcp-Name` or `Mcp-Param-*` value: plain visible ASCII, or the
/// `=?base64?…?=` sentinel form for anything else (MCP 2026-07-28).
pub fn decode_header_value(value: &str) -> Option<String> {
    if let Some(encoded) = value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    {
        let bytes = STANDARD.decode(encoded).ok()?;
        return String::from_utf8(bytes).ok();
    }
    let plain = value
        .bytes()
        .all(|b| b == b'\t' || (b' '..=b'~').contains(&b));
    plain.then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::{Claims, Keys};
    use std::time::Duration;

    fn config() -> Config {
        Config {
            keys: Keys::parse("0123456789abcdef0123456789abcdef").unwrap(),
            call_timeout: Duration::from_secs(1),
            origins: Origins::parse("https://app.example, https://b.example/"),
        }
    }

    fn token(role: Role) -> String {
        config().keys.sign(&Claims {
            sub: "alice".into(),
            role,
            exp: None,
            nbf: None,
        })
    }

    #[test]
    fn routes() {
        assert_eq!(route("/connect"), Route::Connect);
        assert_eq!(route("/mcp"), Route::Mcp { token: None });
        assert_eq!(
            route("/mcp/abc.def"),
            Route::Mcp {
                token: Some("abc.def")
            }
        );
        assert_eq!(route("/mcp/"), Route::NotFound);
        assert_eq!(route("/mcp/a/b"), Route::NotFound);
        assert_eq!(route("/"), Route::NotFound);
    }

    #[test]
    fn client_handshake() {
        let offer = |value: String| {
            [("Sec-WebSocket-Protocol", value)]
                .into_iter()
                .collect::<Headers>()
        };
        let good = offer(format!("clientmcp.v1, {}", token(Role::Client)));
        assert_eq!(accept_client(&config(), &good, 0).unwrap(), "alice");
        let agent = offer(format!("clientmcp.v1, {}", token(Role::Agent)));
        assert_eq!(
            accept_client(&config(), &agent, 0),
            Err(ClientRefusal::Unauthorized)
        );
        let missing = offer("clientmcp.v1".into());
        assert_eq!(
            accept_client(&config(), &missing, 0),
            Err(ClientRefusal::Unauthorized)
        );
        let foreign = offer(format!("graphql-ws, {}", token(Role::Client)));
        assert_eq!(
            accept_client(&config(), &foreign, 0),
            Err(ClientRefusal::NotClientMcp)
        );
    }

    #[test]
    fn agent_front_door() {
        let bearer = [("Authorization", format!("Bearer {}", token(Role::Agent)))];
        let ok = admit_agent(
            &config(),
            "POST",
            None,
            &bearer.iter().cloned().collect(),
            0,
        )
        .unwrap();
        assert_eq!((ok.client.as_str(), ok.cors), ("alice", None));

        let path = token(Role::Agent);
        assert!(admit_agent(&config(), "POST", Some(&path), &Headers::default(), 0).is_ok());

        let wrong_role = [("authorization", format!("bearer {}", token(Role::Client)))];
        let denied = admit_agent(
            &config(),
            "POST",
            None,
            &wrong_role.iter().cloned().collect(),
            0,
        );
        assert_eq!(denied.unwrap_err().status, 401);

        let evil = [("origin", "https://evil.example".to_string())];
        assert_eq!(
            admit_agent(
                &config(),
                "POST",
                Some(&path),
                &evil.iter().cloned().collect(),
                0
            )
            .unwrap_err()
            .status,
            403
        );

        let from_app: Headers = [
            ("origin", "https://b.example"),
            (
                "access-control-request-headers",
                "authorization, mcp-method",
            ),
        ]
        .into_iter()
        .collect();
        let preflight = admit_agent(&config(), "OPTIONS", None, &from_app, 0).unwrap_err();
        assert_eq!(preflight.status, 204);
        assert!(
            preflight
                .headers
                .iter()
                .any(|(n, v)| *n == "access-control-allow-origin" && v == "https://b.example")
        );
    }

    #[test]
    fn header_values_and_sse() {
        assert_eq!(decode_header_value("us-west1").unwrap(), "us-west1");
        assert_eq!(
            decode_header_value("=?base64?SGVsbG8sIOS4lueVjA==?=").unwrap(),
            "Hello, 世界"
        );
        assert!(decode_header_value("bad\u{1}").is_none());
        assert!(decode_header_value("=?base64?***?=").is_none());
        assert_eq!(sse_event("{\"a\":1}"), "data: {\"a\":1}\n\n");
        assert_eq!(sse_event("{\n}"), "data: {\ndata: }\n\n");
    }
}
