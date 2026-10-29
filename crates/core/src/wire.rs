//! Messages of the client channel (SPEC §3.2).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::json::quote;

/// The WebSocket subprotocol a client must offer.
pub const SUBPROTOCOL: &str = "clientmcp.v1";

/// The exact ping text, so a bridge can answer it without waking state.
pub const PING: &str = r#"{"type":"ping"}"#;
pub const PONG: &str = r#"{"type":"pong"}"#;
/// Acknowledges an accepted `register`.
pub const REGISTERED: &str = r#"{"type":"registered"}"#;

/// Largest message on the client channel, and largest agent request body (SPEC §6).
pub const MAX_MESSAGE_BYTES: usize = 4 << 20;

/// Close codes (SPEC §3.3).
pub const CLOSE_GOING_AWAY: u16 = 1001;
pub const CLOSE_TOO_BIG: u16 = 1009;
pub const CLOSE_PROTOCOL: u16 = 4400;
pub const CLOSE_UNAUTHORIZED: u16 = 4401;
pub const CLOSE_REPLACED: u16 = 4409;

/// A message from the client.
#[derive(Debug)]
pub enum Inbound {
    Register {
        tools: Vec<Box<RawValue>>,
        instructions: Option<String>,
    },
    Result {
        id: String,
        result: ToolResult,
    },
    Ping,
    /// A message type this bridge does not know; ignored for forward compatibility.
    Unknown,
}

impl Inbound {
    pub fn parse(text: &str) -> Result<Inbound, String> {
        // A flat struct rather than a tagged enum: serde cannot keep `RawValue`s
        // raw inside internally tagged enums.
        #[derive(Deserialize)]
        struct Message {
            #[serde(rename = "type")]
            kind: String,
            tools: Option<Vec<Box<RawValue>>>,
            instructions: Option<String>,
            id: Option<String>,
            result: Option<BTreeMap<String, Box<RawValue>>>,
        }
        let message: Message =
            serde_json::from_str(text).map_err(|e| format!("malformed message: {e}"))?;
        Ok(match message.kind.as_str() {
            "register" => Inbound::Register {
                tools: message.tools.ok_or("register without tools")?,
                instructions: message.instructions,
            },
            "result" => Inbound::Result {
                id: message.id.ok_or("result without id")?,
                result: message
                    .result
                    .and_then(ToolResult::new)
                    .ok_or("result without a CallToolResult")?,
            },
            "ping" => Inbound::Ping,
            _ => Inbound::Unknown,
        })
    }
}

/// A `CallToolResult` from the client: an object with a `content` array,
/// its members kept verbatim.
#[derive(Debug)]
pub struct ToolResult(pub(crate) BTreeMap<String, Box<RawValue>>);

impl ToolResult {
    fn new(members: BTreeMap<String, Box<RawValue>>) -> Option<ToolResult> {
        let content = members.get("content")?;
        content
            .get()
            .starts_with('[')
            .then_some(ToolResult(members))
    }

    /// Whether the client answered with a tool error (`isError: true`).
    pub fn is_error(&self) -> bool {
        self.0
            .get("isError")
            .is_some_and(|v| v.get().trim() == "true")
    }
}

/// The `call` message. `arguments` is a JSON object, forwarded verbatim.
pub fn call(id: &str, name: &str, arguments: &RawValue) -> String {
    format!(
        r#"{{"type":"call","id":{},"name":{},"arguments":{}}}"#,
        quote(id),
        quote(name),
        arguments.get()
    )
}

/// The `cancel` message.
pub fn cancel(id: &str) -> String {
    format!(r#"{{"type":"cancel","id":{}}}"#, quote(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn parses_client_messages() {
        let register = r#"{"type":"register","tools":[{"name":"a"}],"instructions":"hi"}"#;
        let Inbound::Register {
            tools,
            instructions,
        } = Inbound::parse(register).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (tools[0].get(), instructions.as_deref()),
            (r#"{"name":"a"}"#, Some("hi"))
        );

        let result = r#"{"type":"result","id":"7","result":{"content":[]}}"#;
        assert!(matches!(Inbound::parse(result).unwrap(), Inbound::Result { id, .. } if id == "7"));
        assert!(matches!(Inbound::parse(PING).unwrap(), Inbound::Ping));
        assert!(matches!(
            Inbound::parse(r#"{"type":"future","x":1}"#).unwrap(),
            Inbound::Unknown
        ));

        let bad_results = [
            r#"{"type":"result","id":7,"result":{"content":[]}}"#,
            r#"{"type":"result","id":"7","result":{"content":{}}}"#,
            r#"{"type":"result","id":"7","result":[]}"#,
        ];
        for bad in ["", "[]", r#"{"tools":[]}"#, r#"{"type":"register"}"#]
            .into_iter()
            .chain(bad_results)
        {
            assert!(Inbound::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn formats_bridge_messages() {
        let args = RawValue::from_string(r#"{"chainId":1}"#.into()).unwrap();
        let call: Value = serde_json::from_str(&call("c\"1", "getBalance", &args)).unwrap();
        assert_eq!(
            call,
            json!({ "type": "call", "id": "c\"1", "name": "getBalance", "arguments": { "chainId": 1 } })
        );
        assert_eq!(cancel("x"), r#"{"type":"cancel","id":"x"}"#);
    }
}
