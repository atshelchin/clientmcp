# ClientMCP Protocol

**Version:** 1.0 · **Status:** Stable draft · **MCP revisions served:** 2026-07-28, 2025-11-25, 2025-06-18, 2025-03-26

> Expose live client applications as MCP servers.

ClientMCP lets a running browser, desktop or mobile application expose its own
functions as tools of a standard MCP server. The application opens one outbound
WebSocket to a **bridge**; AI agents talk to the bridge over standard MCP
Streamable HTTP. The application never opens a port or runs an HTTP server.

```text
 Human ──► UI ─┐
               ▼
        Application state & actions ◄── ClientMCP SDK
                                              │  WebSocket (client channel)
                                              ▼
                                         ClientMCP bridge
                                              ▲  MCP Streamable HTTP (agent endpoint)
                                              │
                                           AI agent
```

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be interpreted
as described in RFC 2119.

---

## 1. Model

| Term          | Meaning                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------- |
| **Client**    | An application instance connected to the bridge. It declares tools and executes calls.        |
| **Client ID** | The stable identity of a client, taken from its token. At most one connection holds it.       |
| **Agent**     | An MCP client (Claude Code, an agent framework, …) calling the tools of one client.           |
| **Bridge**    | The server translating between the client channel and the agent endpoint.                     |
| **Call**      | One `tools/call` routed from an agent to a client, correlated by a bridge-assigned call ID.    |

Principles:

1. **Client-side execution.** Every tool runs inside the client, against the
   same state and actions its UI uses. The bridge never executes tools.
2. **Outbound only.** The client initiates the only connection it needs.
3. **Standard MCP.** Agents need nothing beyond MCP Streamable HTTP and a bearer
   token.
4. **Declarative state.** The client declares its full tool set; the bridge
   mirrors it. There are no incremental add/remove messages to get out of sync.
5. **No persistence.** The bridge stores no messages and queues nothing. A
   client that is offline has no tools; a call that cannot be delivered fails.
6. **Stateless authorization.** Every credential is a signed token that any
   bridge instance can verify without a database.

The bridge is a trusted component: it sees every call and result in clear
text (§7).

## 2. Tokens

Both clients and agents authenticate with a JSON Web Token (RFC 7519) signed
with HMAC-SHA256 (`HS256`) using a secret configured on the bridge.

```json
{ "sub": "alice-wallet", "role": "client", "exp": 1798761600 }
```

| Claim  | Required | Meaning                                                                       |
| ------ | -------- | ----------------------------------------------------------------------------- |
| `sub`  | yes      | The client ID. 1–128 characters from `A–Z a–z 0–9 . _ - : @`.                 |
| `role` | yes      | `"client"` (may hold the client channel) or `"agent"` (may call the client).  |
| `exp`  | no       | Expiry, seconds since the Unix epoch.                                         |
| `nbf`  | no       | Not-before, seconds since the Unix epoch.                                     |

Other claims are ignored. A bridge MUST reject a token whose header `alg` is not
`HS256`, whose signature does not verify, whose `sub` or `role` is missing or
invalid, or whose `exp`/`nbf` is violated (a bridge MAY allow up to 60 seconds of
clock skew). A bridge MAY accept several secrets at once to allow rotation;
secrets MUST be at least 32 bytes.

Tokens are minted by whoever holds the secret: the operator (the reference
bridge ships `clientmcp token`) or the application's backend (any JWT library).
An agent token for client `X` grants calling every tool `X` declares, whenever
`X` is online. A client token for `X` grants becoming `X`.

`exp` is checked when a credential is presented: at connection time for the
client channel and on every request for the agent endpoint. An established
client connection is not closed when its token later expires.

## 3. Client channel

### 3.1 Connecting

The client opens a WebSocket to the bridge's client endpoint (the reference
bridge uses `/connect`) and offers exactly two subprotocols: the protocol
identifier `clientmcp.v1` and its token.

```js
new WebSocket("wss://bridge.example.com/connect", ["clientmcp.v1", token]);
```

Carrying the token in `Sec-WebSocket-Protocol` keeps it out of URLs and logs and
works in browsers, which cannot set an `Authorization` header on a WebSocket.
The token is the offered value that does not begin with `clientmcp.`. The bridge
MUST select `clientmcp.v1` in its handshake response and MUST NOT echo the
token.

If the token is not a valid `client` token, the bridge completes the handshake
and immediately closes the connection with code `4401`. (A failed HTTP upgrade
is indistinguishable from a network error in browsers; a close code is not.)

When a connection for client ID `X` is accepted while another connection holds
`X`, the bridge MUST close the older connection with code `4409` and fail its
in-flight calls (§5.3). **The newest connection wins**, so a reconnect is never
blocked by a stale or half-open predecessor. A client closed with `4409` MUST
NOT reconnect automatically; otherwise two instances sharing an identity would
evict each other forever.

### 3.2 Messages

Every message is a UTF-8 JSON object in a WebSocket text frame with a `type`
member, at most 4 MiB (4,194,304 bytes) long. A receiver MUST ignore messages
with an unknown `type` (forward compatibility) and MUST treat anything else that
does not match this section as a protocol error.

**Client → bridge**

| `type`     | Members                                     | Meaning                                       |
| ---------- | ------------------------------------------- | --------------------------------------------- |
| `register` | `tools`: `Tool[]`, `instructions?`: string | Declare the complete current state.           |
| `result`   | `id`: string, `result`: `CallToolResult`    | Finish a call.                                |
| `ping`     | –                                           | Liveness probe; the bridge answers `pong`.    |

**Bridge → client**

| `type`   | Members                                          | Meaning                                  |
| -------- | ------------------------------------------------ | ---------------------------------------- |
| `registered` | –                                            | A `register` was accepted.               |
| `call`   | `id`: string, `name`: string, `arguments`: object | Execute a tool.                          |
| `cancel` | `id`: string                                     | Stop a call; its result is not wanted.   |
| `pong`   | –                                                | Answer to `ping`.                        |

`Tool` and `CallToolResult` are the MCP types of the same names. The bridge
passes them through unchanged except as stated below, so new MCP fields
(`title`, `icons`, `outputSchema`, `annotations`, `_meta`, …) need no protocol
change.

#### `register`

```json
{
  "type": "register",
  "instructions": "Alice's wallet. Amounts are decimal strings in ETH.",
  "tools": [
    {
      "name": "getBalance",
      "description": "Get the balance of the open account",
      "inputSchema": {
        "type": "object",
        "properties": { "chainId": { "type": "integer" } },
        "required": ["chainId"]
      }
    }
  ]
}
```

A `register` replaces the client's entire declared state: the tools of the
previous `register` that are not listed are revoked, and an absent
`instructions` clears them. A client connects with no tools and normally sends
`register` right after opening the connection, and again whenever its tools
change. The bridge serves the tools in the declared order.

The bridge answers every accepted `register`, in order, with `registered`.
The first `registered` on a connection is the client's proof that its token and
its declared state were accepted — the bridge refuses either by closing the
connection (§3.3), which a client cannot otherwise tell apart from a refusal
still in flight.

The bridge MUST reject (close with `4400`, reason naming the problem) a
`register` in which:

- a tool `name` is not 1–128 characters from `A–Z a–z 0–9 _ - .`, or is
  declared twice;
- an `inputSchema` is not a JSON object with `"type": "object"`;
- an `x-mcp-header` annotation violates MCP's constraints;
- a limit of the bridge is exceeded (§6).

Validating at the bridge turns a malformed tool, which MCP agents would
silently drop, into an immediate error for the developer.

#### `call`, `result`, `cancel`

```json
{ "type": "call", "id": "8f3a…", "name": "getBalance", "arguments": { "chainId": 1 } }
```
```json
{ "type": "result", "id": "8f3a…", "result": { "content": [{ "type": "text", "text": "100.50" }] } }
```

- Call IDs are opaque strings chosen by the bridge, unique among the calls in
  flight on a connection. The client MUST echo the ID unchanged.
- The bridge only calls tools in the client's latest `register`. `arguments` is
  always an object, forwarded exactly as the agent sent it. It is untrusted:
  the client MUST validate it before acting.
- Calls run concurrently; results may arrive in any order.
- Tool failures — invalid arguments, a business error, a user declining a
  confirmation — are reported as a `CallToolResult` with `"isError": true`, so
  the model can see the reason and recover. There is no separate error message.
- After `cancel`, the client SHOULD stop the work and MAY omit the result. The
  bridge ignores results for unknown or finished calls, so cancel/result races
  are harmless.
- `result.result` MUST be an object with a `content` array; anything else is a
  protocol error. The bridge sets `resultType` itself and forwards the other
  members unchanged.
- A result that would make the message exceed the size limit cannot be
  delivered; the client SHOULD send a tool error saying so instead.

#### `ping`, `pong`

The client SHOULD send a ping when it has been idle for 25 seconds, and SHOULD
consider the connection dead and reconnect if nothing arrives within 10 seconds
of a ping. Clients SHOULD send exactly the text `{"type":"ping"}`; the bridge
answers with exactly `{"type":"pong"}`. (Fixed byte strings let a bridge answer
without waking hibernated state, see §8.) A bridge MAY close a connection that
has sent nothing for 90 seconds.

### 3.3 Close codes

| Code   | Meaning                                         | Client behaviour        |
| ------ | ----------------------------------------------- | ----------------------- |
| `4400` | Protocol error (malformed message, bad register) | Do not reconnect; fix.  |
| `4401` | Invalid or expired token                        | Reconnect only with a new token. |
| `4409` | Replaced by a newer connection with the same ID  | Do not reconnect.       |
| `1009` | Message too large                               | Do not reconnect; fix.  |
| other  | Network failure, bridge restart (`1001`), …     | Reconnect with backoff. |

### 3.4 Lifecycle

```text
          connect + valid token              register / registered
OFFLINE ───────────────────────────► ONLINE ◄──────────┐
   ▲         (empty tool set)          │ ─────────────────┘
   └───────────────────────────────────┘
        close, network loss, 4409 replacement
```

A client is **online** while it holds its connection. When the connection ends,
for any reason, its tools disappear and its in-flight calls fail. On reconnect
it starts again from an empty tool set and re-registers. Nothing about a client
survives its connection, so a bridge restart only requires clients to reconnect.

## 4. Agent endpoint

The bridge is an MCP server reachable over Streamable HTTP at an MCP endpoint
(the reference bridge uses `/mcp`). An agent authenticates every request with
an `agent` token, which also selects the client:

```http
POST /mcp HTTP/1.1
Authorization: Bearer <agent token>
```

For MCP hosts that cannot send custom headers, the bridge MAY also accept the
token as a final path segment, `/mcp/<agent token>`. Such URLs are secrets and
end up in logs and history; the header form SHOULD be preferred.

A request without a valid agent token is answered with `401` and
`WWW-Authenticate: Bearer`.

### 4.1 Protocol revisions

The bridge is dual-era. It serves each request independently according to the
revision the request uses, and keeps no per-agent state.

- **Modern** (2026-07-28): requests carry `_meta` with
  `io.modelcontextprotocol/protocolVersion`. The bridge implements the
  stateless protocol: `server/discover`, `tools/list`, `tools/call` and
  `subscriptions/listen`, with header validation (`MCP-Protocol-Version`,
  `Mcp-Method`, `Mcp-Name`, `Mcp-Param-*`), `resultType`, cache hints and the
  modern error codes and HTTP statuses.
- **Legacy** (2025-11-25, 2025-06-18, 2025-03-26): an `initialize` request, or a
  request without `_meta` protocol version whose `MCP-Protocol-Version` header
  names a legacy revision or is absent. The bridge implements `initialize`,
  `ping`, `tools/list`, `tools/call`, `notifications/cancelled` and the GET
  notification stream.

`initialize` returns an `Mcp-Session-Id`. The bridge stores nothing under it;
it only namespaces the JSON-RPC IDs that a legacy `notifications/cancelled` refers
to. Requests that omit it are served, unknown values are never rejected, and
`DELETE` returns `405`.

A request naming an unsupported revision is answered with `400` and
`UnsupportedProtocolVersionError` (`-32022`) listing the supported revisions.

### 4.2 Mapping

| MCP                                  | Bridge behaviour                                                                    |
| ------------------------------------ | ----------------------------------------------------------------------------------- |
| `server/discover`, `initialize`      | Capabilities `{"tools":{"listChanged":true}}`; the client's `instructions`.        |
| `tools/list`                         | The client's declared tools, in order, unpaginated. Empty while it is offline.     |
| `tools/call`                         | Sent to the client as `call`; its `result` becomes the response.                   |
| `subscriptions/listen`, `GET`        | SSE stream of `notifications/tools/list_changed` (§4.4).                            |
| `ping` (legacy)                      | Empty result.                                                                       |
| anything else                        | `-32601` Method not found (`404` for modern requests).                              |

Modern list results carry `"ttlMs": 0` and `"cacheScope": "private"`: the tool
set can change at any moment and belongs to one client.

### 4.3 Call outcomes

A `tools/call` is answered with:

| Situation                                     | Response                                              |
| --------------------------------------------- | ----------------------------------------------------- |
| Client returns a result                       | That `CallToolResult`.                                |
| Tool not declared by an online client         | JSON-RPC error `-32602` `Unknown tool: <name>`.       |
| `arguments` present but not an object         | JSON-RPC error `-32602`.                              |
| Client offline                                | `CallToolResult` with `isError: true`.                |
| Client disconnects before answering           | `CallToolResult` with `isError: true`.                |
| No result within the call timeout             | `CallToolResult` with `isError: true`; `cancel` sent. |
| Too many calls in flight for the client       | `CallToolResult` with `isError: true`.                |

Availability problems are tool execution errors rather than protocol errors so
that the model learns why the tool failed ("the app is not open") and can tell
the user, as MCP recommends for API failures.

**Streaming.** A call can take long — a user may be deciding whether to confirm
it. When the agent accepts `text/event-stream` and a call has not completed
within a second, the reference bridges answer with an SSE stream instead of a
JSON body: an SSE comment every second, then the response as the only event.
This keeps proxies from timing out calls that wait for a human, and the writes
are how a bridge notices an agent that has gone away; some platforms report a
disconnect no other way.

**Cancellation.** When the agent abandons a call — a modern agent closes the HTTP
response, a legacy agent sends `notifications/cancelled` with the session ID
from `initialize` — the bridge sends `cancel` to the client.

### 4.4 Tool list changes

The client's tool set changes when it registers a different set, connects
with a set, or goes offline. Each change is announced with
`notifications/tools/list_changed` on every open notification stream for that
client:

- **Modern:** `subscriptions/listen` with `"toolsListChanged": true`. The
  stream starts with `notifications/subscriptions/acknowledged`, every
  notification carries `io.modelcontextprotocol/subscriptionId`, and a bridge
  ending the stream on its own (e.g. shutdown) first sends the final
  `subscriptions/listen` result.
- **Legacy:** `GET` on the MCP endpoint with `Accept: text/event-stream`.

Streams stay open while the client is offline, so an agent started before the
application learns about the tools as soon as the application connects. Bridges
SHOULD send an SSE comment as keep-alive at least every 30 seconds.

### 4.5 Origin and CORS

Requests without an `Origin` header (agents are rarely browsers) are served.
A request with an `Origin` that is not on the bridge's allow-list is rejected
with `403`, as MCP requires against DNS rebinding. For allowed origins the
bridge answers CORS preflights and adds the CORS response headers. The client
endpoint does not check `Origin`: it is authenticated by a token, not by
ambient credentials.

## 5. Behaviour

### 5.1 Routing

A call reaches exactly the connection that holds the client ID when the call
is dispatched. A call is never queued for later delivery and never retried.

### 5.2 Timeouts

Every call has a timeout (the reference bridge defaults to 120 seconds). When
it expires the bridge answers the agent (§4.3) and sends `cancel` to the
client.

### 5.3 Disconnection

When a client connection ends, every call in flight on it fails immediately
(§4.3), and the tool list becomes empty. When the bridge shuts down gracefully
it closes client connections with `1001`, ends notification streams (§4.4) and
lets in-flight HTTP requests fail.

## 6. Limits

Client channel messages and agent request bodies are at most 4 MiB, on every
bridge, so that a client can check a result before sending it. Beyond that a
bridge MUST bound the resources a single client or agent can consume and SHOULD
document its limits. The reference bridge uses:

| Limit                               | Value     |
| ----------------------------------- | --------- |
| Tools per client                    | 1,000     |
| Calls in flight per client          | 256       |
| Notification streams per client     | 64        |
| Call timeout                        | 120 s (configurable) |

## 7. Security

- **The bridge is trusted.** It terminates TLS, reads every argument and result
  and could forge calls. ClientMCP offers no end-to-end confidentiality from
  the bridge operator; deploy a bridge you trust or operate.
- **Tokens are bearer credentials.** Anyone holding an agent token can call
  the client's tools; anyone holding a client token can impersonate the
  client and receive its calls. Use `exp`. A client token embedded in a web
  page is exposed to any script on that page; prefer short-lived tokens issued
  by the application's backend (the SDK accepts a token function that is called
  on every connection). Revoking tokens before `exp` requires rotating the
  secret.
- **The client is the authority.** It decides which tools exist, validates
  every argument, and SHOULD ask the user to confirm consequential actions.
  `arguments` are written by a language model that may be manipulated by
  prompt injection.
- **Transport security.** Production deployments MUST use `https`/`wss`.
- **Isolation.** A client ID is the unit of isolation: tools, calls and
  notifications of one client are never visible to another.

## 8. Implementation notes (non-normative)

- *Cloudflare Workers.* One Durable Object per client ID holds the client
  WebSocket through the Hibernation API, so an idle client costs nothing. The
  exact-text ping is answered by `setWebSocketAutoResponse` without waking the
  object. Notification streams are held by the stateless Worker, which listens
  to the Durable Object over an internal hibernatable WebSocket, so an agent
  waiting for changes does not keep the object awake either. The tool set is
  kept in Durable Object storage only so that it survives hibernation; it is
  deleted when the client disconnects. The Worker notices an agent hanging up
  on a streamed call when a keep-alive write fails, and asks the object to
  cancel the call. When the platform restarts an object (e.g. on deploy), its
  streams end without the final result and agents listen again.
- *Single process.* A map from client ID to connection, with a notification
  channel per client, is the whole state. A single instance serves all
  clients; running several instances requires routing all requests for a
  client ID to the same instance.

## 9. Application conventions (non-normative)

The protocol carries an application's tools; it does not decide what they
may do. Two conventions let applications say so in the same words, so that
agents and their hosts can read any application alike. A bridge passes both
through unread.

### 9.1 Risk

A tool MAY publish how much its action matters, on the application's own
scale, as `_meta["io.github.clientmcp/risk"]`:

| Value | Meaning | MCP annotations it implies |
|---|---|---|
| `observe` | changes nothing | `readOnlyHint: true` |
| `operate` | ordinary, reversible use | `readOnlyHint: false`, `destructiveHint: false` |
| `manage` | security-relevant or hard to undo | `destructiveHint: true` |
| `transact` | moves value or signs | `destructiveHint: true` |

What each level requires — the user's approval in the application, a grant,
nothing — is the application's to decide and to enforce, in the application.
An agent's access to the bridge is not the user's approval of any one call.

A call that waits for the user can outlive the bridge's call timeout (§4.3).
The bridge then answers the agent with an error and sends `cancel`. An
application SHOULD treat that `cancel` as the end of the request: it withdraws
its question to the user and does nothing after it. Otherwise an agent told
that the call failed finds the change made anyway. It SHOULD also keep its own
waits shorter than the bridge's call timeout, so that it says how the wait
ended.

### 9.2 Outcome

A tool's result MAY say how the call ended, as `structuredContent` with a
`status`:

- `completed` — the change holds: the application has seen it in its state,
  not merely run the function that asks for it;
- `accepted` — it started and goes on; `operation` names it, and the
  application reports its state through its own tools;
- `rejected` — nothing changed; `reason.code` is for the agent to act on,
  `reason.detail` for a reader. The result has `isError: true`.

`status`, `operation` and `reason` are the outcome's own members: an
application's further data MUST NOT use those names. `operation` is the
operation's id, a string.

## 10. Non-goals

UI automation, application state management, message persistence, offline
queues, resources, prompts, progress notifications and multi round-trip
requests are out of scope for this version. Each can be added later as new
message types and new optional `register` members without breaking v1.
