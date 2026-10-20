use serde_json::{Value, json};

use super::*;

const V: &str = "2026-07-28";

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs.iter().map(|(k, v)| (*k, *v)).collect()
}

fn modern_body(method: &str, extra: Value) -> Vec<u8> {
    let mut params = json!({ "_meta": {
        "io.modelcontextprotocol/protocolVersion": V,
        "io.modelcontextprotocol/clientCapabilities": {}
    }});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .unwrap()
}

fn modern(method: &str, extra: Value, more: &[(&str, &str)]) -> Op {
    let mut h = vec![("MCP-Protocol-Version", V), ("Mcp-Method", method)];
    h.extend_from_slice(more);
    parse("POST", &headers(&h), &modern_body(method, extra))
}

fn legacy(method: &str, params: Value, h: &[(&str, &str)]) -> Op {
    let body = json!({ "jsonrpc": "2.0", "id": "a", "method": method, "params": params });
    parse("POST", &headers(h), &serde_json::to_vec(&body).unwrap())
}

fn reply(op: Op) -> Response {
    match op {
        Op::Reply(response) => response,
        other => panic!("expected a reply, got {other:?}"),
    }
}

fn body(response: &Response) -> Value {
    serde_json::from_str(&response.body).unwrap()
}

fn error_code(op: Op) -> (u16, i64) {
    let response = reply(op);
    (
        response.status,
        body(&response)["error"]["code"].as_i64().unwrap(),
    )
}

fn result_of(json: &str) -> ToolResult {
    let message = format!(r#"{{"type":"result","id":"1","result":{json}}}"#);
    match crate::wire::Inbound::parse(&message).unwrap() {
        crate::wire::Inbound::Result { result, .. } => result,
        other => panic!("{other:?}"),
    }
}

fn arguments(call: &Call) -> Value {
    serde_json::from_str(call.arguments.get()).unwrap()
}

#[test]
fn malformed_requests() {
    let post = |b: &str| reply(parse("POST", &Headers::default(), b.as_bytes()));
    assert_eq!(post("{").status, 400);
    assert_eq!(body(&post("{"))["error"]["code"], -32700);
    assert_eq!(body(&post("[]"))["error"]["code"], -32600);
    assert_eq!(
        body(&post(r#"{"id":1,"method":"ping"}"#))["error"]["code"],
        -32600
    );
    assert_eq!(
        body(&post(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#))["error"]["code"],
        -32600
    );
    assert_eq!(
        body(&post(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#))["error"]["code"],
        -32600
    );
    assert_eq!(reply(parse("DELETE", &Headers::default(), b"")).status, 405);
    assert_eq!(reply(parse("GET", &Headers::default(), b"")).status, 405);
}

#[test]
fn legacy_initialize_negotiates_and_mints_a_session() {
    let Op::Describe(describe) = legacy(
        "initialize",
        json!({ "protocolVersion": "2025-06-18" }),
        &[],
    ) else {
        panic!()
    };
    let response = describe.respond(Some("Be careful."));
    let result = &body(&response)["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["capabilities"]["tools"]["listChanged"], true);
    assert_eq!(result["serverInfo"]["name"], "clientmcp");
    assert_eq!(result["instructions"], "Be careful.");
    assert!(result.get("resultType").is_none());
    assert!(
        response
            .headers
            .iter()
            .any(|(n, v)| *n == "mcp-session-id" && v.len() == 32)
    );

    let Op::Describe(newer) = legacy(
        "initialize",
        json!({ "protocolVersion": "2099-01-01" }),
        &[],
    ) else {
        panic!()
    };
    assert_eq!(
        body(&newer.respond(None))["result"]["protocolVersion"],
        LEGACY[0]
    );
}

#[test]
fn legacy_methods() {
    let pong = reply(legacy(
        "ping",
        json!({}),
        &[("mcp-protocol-version", "2025-11-25")],
    ));
    assert_eq!(
        body(&pong),
        json!({ "jsonrpc": "2.0", "id": "a", "result": {} })
    );
    assert_eq!(
        error_code(legacy("resources/list", json!({}), &[])),
        (200, -32601)
    );
    assert_eq!(
        error_code(legacy("tools/list", json!({ "cursor": "x" }), &[])),
        (200, -32602)
    );
    assert_eq!(
        error_code(legacy(
            "tools/list",
            json!({}),
            &[("mcp-protocol-version", "1999-01-01")]
        )),
        (400, -32022)
    );

    let Op::ListTools(list) = legacy("tools/list", json!({}), &[]) else {
        panic!()
    };
    let tools =
        ToolSet::parse(&[
            serde_json::from_str(r#"{"name":"t","inputSchema":{"type":"object"}}"#).unwrap(),
        ])
        .unwrap();
    assert_eq!(
        body(&list.tools(&tools))["result"],
        json!({ "tools": [{ "name": "t", "inputSchema": { "type": "object" } }] })
    );

    let Op::CallTool(call) = legacy(
        "tools/call",
        json!({ "name": "t" }),
        &[("Mcp-Session-Id", "s1")],
    ) else {
        panic!()
    };
    assert_eq!((call.name.as_str(), arguments(&call)), ("t", json!({})));
    let notification = json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": "a" } });
    let cancel = parse(
        "POST",
        &headers(&[("mcp-session-id", "s1")]),
        &serde_json::to_vec(&notification).unwrap(),
    );
    match cancel {
        Op::Cancel(key) => assert_eq!(Some(key), call.cancel_key),
        other => panic!("{other:?}"),
    }
    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert_eq!(
        reply(parse(
            "POST",
            &Headers::default(),
            &serde_json::to_vec(&initialized).unwrap()
        ))
        .status,
        202
    );
}

#[test]
fn modern_discover() {
    let Op::Describe(describe) = modern("server/discover", json!({}), &[]) else {
        panic!()
    };
    let result = body(&describe.respond(None))["result"].clone();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"][0], V);
    assert_eq!(
        result["capabilities"],
        json!({ "tools": { "listChanged": true } })
    );
    assert_eq!(
        (result["ttlMs"].as_i64(), result["cacheScope"].as_str()),
        (Some(0), Some("private"))
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "clientmcp"
    );
    assert!(result.get("instructions").is_none());
}

#[test]
fn modern_validation() {
    // Header missing or different from _meta.
    let body_only = parse(
        "POST",
        &headers(&[("Mcp-Method", "tools/list")]),
        &modern_body("tools/list", json!({})),
    );
    assert_eq!(error_code(body_only), (400, -32020));
    // A modern header with a legacy-shaped body.
    let no_meta = legacy(
        "tools/list",
        json!({}),
        &[("MCP-Protocol-Version", V), ("Mcp-Method", "tools/list")],
    );
    assert_eq!(error_code(no_meta), (400, -32602));
    // Mcp-Method must mirror the method.
    let wrong_method = parse(
        "POST",
        &headers(&[("MCP-Protocol-Version", V), ("Mcp-Method", "tools/call")]),
        &modern_body("tools/list", json!({})),
    );
    assert_eq!(error_code(wrong_method), (400, -32020));
    // Unknown versions list what is supported.
    let mut future = modern_body("tools/list", json!({}));
    future = String::from_utf8(future)
        .unwrap()
        .replace(V, "2099-01-01")
        .into_bytes();
    let response = reply(parse(
        "POST",
        &headers(&[
            ("MCP-Protocol-Version", "2099-01-01"),
            ("Mcp-Method", "tools/list"),
        ]),
        &future,
    ));
    assert_eq!(response.status, 400);
    assert_eq!(body(&response)["error"]["data"]["supported"][0], V);
    assert_eq!(body(&response)["error"]["data"]["requested"], "2099-01-01");
    // Capabilities are required.
    let caps = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": V } } });
    let no_caps = parse(
        "POST",
        &headers(&[("MCP-Protocol-Version", V), ("Mcp-Method", "tools/list")]),
        &serde_json::to_vec(&caps).unwrap(),
    );
    assert_eq!(error_code(no_caps), (400, -32602));
    // Removed and unknown methods are 404.
    assert_eq!(error_code(modern("ping", json!({}), &[])), (404, -32601));
    assert_eq!(
        error_code(modern("resources/list", json!({}), &[])),
        (404, -32601)
    );
}

#[test]
fn modern_call_requires_mcp_name() {
    let call = |name_header: Option<&str>| {
        let h: Vec<(&str, &str)> = name_header.map(|n| ("Mcp-Name", n)).into_iter().collect();
        modern(
            "tools/call",
            json!({ "name": "get_weather", "arguments": { "city": "Oslo" } }),
            &h,
        )
    };
    assert_eq!(error_code(call(None)), (400, -32020));
    assert_eq!(error_code(call(Some("other"))), (400, -32020));
    let Op::CallTool(call) = call(Some("=?base64?Z2V0X3dlYXRoZXI=?=")) else {
        panic!()
    };
    assert_eq!(arguments(&call)["city"], "Oslo");
    assert!(call.cancel_key.is_none());
    let bad_args = modern(
        "tools/call",
        json!({ "name": "t", "arguments": [1] }),
        &[("Mcp-Name", "t")],
    );
    assert_eq!(error_code(bad_args), (200, -32602));
}

#[test]
fn call_outcomes() {
    let Op::CallTool(modern_call) =
        modern("tools/call", json!({ "name": "t" }), &[("Mcp-Name", "t")])
    else {
        panic!()
    };
    let Op::CallTool(legacy_call) = legacy("tools/call", json!({ "name": "t" }), &[]) else {
        panic!()
    };
    let client = r#"{"content":[{"type":"text","text":"ok"}],"resultType":"bogus","structuredContent":{"n":1},"_meta":{"app/x":1}}"#;

    let result = body(&modern_call.respond(Outcome::Done(result_of(client))))["result"].clone();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["structuredContent"], json!({ "n": 1 }));
    assert_eq!(result["_meta"]["app/x"], 1);
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "clientmcp"
    );

    let result = body(&legacy_call.respond(Outcome::Done(result_of(client))))["result"].clone();
    assert!(result.get("resultType").is_none());
    assert_eq!(result["_meta"], json!({ "app/x": 1 }));

    for outcome in [
        Outcome::Offline,
        Outcome::Disconnected,
        Outcome::Timeout,
        Outcome::Busy,
    ] {
        let result = body(&legacy_call.respond(outcome))["result"].clone();
        assert_eq!(result["isError"], true);
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("application")
        );
    }
    let unknown = body(&legacy_call.respond(Outcome::UnknownTool));
    assert_eq!(
        (
            unknown["error"]["code"].as_i64(),
            unknown["error"]["message"].as_str()
        ),
        (Some(-32602), Some("Unknown tool: t"))
    );
    assert_eq!(
        modern_call
            .respond(Outcome::HeaderMismatch("x".into()))
            .status,
        400
    );
    // Arguments are forwarded byte for byte.
    let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"t","arguments":{ "b":1,  "a":[2.50,"\u00e9"] }}}"#;
    let Op::CallTool(verbatim) = parse("POST", &Headers::default(), body) else {
        panic!()
    };
    assert_eq!(
        verbatim.arguments.get(),
        r#"{ "b":1,  "a":[2.50,"\u00e9"] }"#
    );
}

#[test]
fn notification_streams() {
    let Op::Listen(listen) = modern(
        "subscriptions/listen",
        json!({ "notifications": { "toolsListChanged": true, "promptsListChanged": true } }),
        &[],
    ) else {
        panic!()
    };
    let event = |sse: String| -> Value {
        serde_json::from_str(sse.strip_prefix("data: ").unwrap().trim_end()).unwrap()
    };
    let ack = event(listen.opening().unwrap());
    assert_eq!(ack["method"], "notifications/subscriptions/acknowledged");
    assert_eq!(
        ack["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        1
    );
    assert_eq!(
        ack["params"]["notifications"],
        json!({ "toolsListChanged": true })
    );
    let changed = event(listen.changed().unwrap());
    assert_eq!(changed["method"], "notifications/tools/list_changed");
    assert_eq!(
        changed["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        1
    );
    let closing = event(listen.closing().unwrap());
    assert_eq!(
        (
            closing["id"].as_i64(),
            closing["result"]["resultType"].as_str()
        ),
        (Some(1), Some("complete"))
    );

    let Op::Listen(quiet) = modern("subscriptions/listen", json!({ "notifications": {} }), &[])
    else {
        panic!()
    };
    assert!(quiet.changed().is_none());
    assert_eq!(
        error_code(modern("subscriptions/listen", json!({}), &[])),
        (400, -32602)
    );

    let Op::Listen(get) = parse("GET", &headers(&[("accept", "text/event-stream")]), b"") else {
        panic!()
    };
    assert!(get.opening().is_none() && get.closing().is_none());
    assert_eq!(
        event(get.changed().unwrap()),
        json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })
    );
}

#[test]
fn supported_versions_list_every_era() {
    let listed: Vec<String> = serde_json::from_str(SUPPORTED).unwrap();
    assert_eq!(
        listed,
        MODERN
            .iter()
            .chain(LEGACY)
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
    );
}
