# Design decisions

This records how ClientMCP v1 departs from the original 0.1 draft and why. The
normative protocol is [SPEC.md](../SPEC.md).

## From the draft to v1

| Draft 0.1                                          | v1                                                                                   |
| -------------------------------------------------- | ------------------------------------------------------------------------------------ |
| "Client authentication", "agent authorization"     | One credential: HS256 JWT with `sub` (client ID) and `role` (`client`/`agent`).       |
| Client token: unspecified                          | Sent as a WebSocket subprotocol.                                                       |
| "Duplicate connections need a conflict rule"       | Newest connection wins; the old one is closed with `4409` and must not reconnect.      |
| `register`, plus updates and revocation            | `register` declares the complete state; there is nothing else to keep in sync.         |
| —                                                  | `registered` acknowledges each `register`.                                             |
| `invoke` / `result` with arbitrary JSON            | `call` / `result` carrying MCP `CallToolResult`; `cancel`; exact-text `ping`/`pong`.   |
| "Follow the chosen MCP version"                    | Dual-era: MCP 2026-07-28 and 2025-03-26 … 2025-11-25 on the same endpoint.             |
| Offline: "do not execute"                          | Offline clients have no tools; agents stay connected and are notified on return.      |
| "All parameters must be validated"                 | The bridge validates protocol and tool definitions; the client validates arguments.   |

## Decisions

**Dual-era MCP.** MCP 2026-07-28 removed the `initialize` handshake, sessions and
the GET stream. Most deployed agents (MCP SDK v1, and SDK v2 in its default mode)
still speak 2025-era MCP, so a 2026-only bridge would be unusable today. The
bridge decides per request (`initialize`, or the `_meta` protocol version and
the `MCP-Protocol-Version` header) and keeps no per-agent state in either era.
Legacy session IDs are minted but never stored: they only scope the request IDs
that a legacy `notifications/cancelled` names.

**Stateless signed tokens.** Both runtimes verify a token with a secret and the
current time — no database, no coordination between Durable Objects or
instances. JWT HS256 was chosen over a custom format because every backend
language can mint it. Considered and rejected:
- *Agent keys declared by the client.* It gives per-agent revocation, but while
  the client is offline the bridge cannot tell a valid key from an invalid one,
  so it could neither answer `401` nor serve the "offline, wait for tools" state
  that lets an agent start before the application.
- *OAuth 2.1 per the MCP authorization spec.* The right long-term answer for
  hosted agents, but it needs an authorization server — out of scope for a
  bridge whose operator already has a way to mint tokens.
The cost is revocation: before `exp`, only rotating the secret revokes a token.
`CLIENTMCP_SECRET` accepts several secrets so rotation needs no downtime.

**The token in `Sec-WebSocket-Protocol`.** Browsers cannot set `Authorization` on
a WebSocket. A query parameter would land in access logs. A first-message
handshake would force Cloudflare to accept a socket before knowing which Durable
Object owns it. The subprotocol header is available at upgrade time everywhere,
and doubles as the protocol version (`clientmcp.v1`).

**Refusal by close code, confirmation by `registered`.** A browser cannot see
the HTTP status of a failed upgrade, so the bridge completes the handshake and
closes with `4401`. That made "open" ambiguous: the conformance suite caught the
SDK reporting `online` for connections about to be refused. `registered` closes
the gap — the first one proves the token and the tools were accepted. (A pong
after the register could not serve: Cloudflare answers pings at the edge,
possibly before the object has processed the register.)

**Newest connection wins.** Rejecting a second connection would let a half-open
TCP connection lock a client out until it times out — exactly when a client
reconnects after a network change. The displaced connection must not reconnect
automatically, or two tabs sharing an identity would evict each other forever.

**Declarative `register`.** Full replacement is idempotent and survives any
reordering or loss of intermediate updates; tool lists are small. Incremental
add/remove messages would need ordering guarantees and reconciliation for no
practical gain.

**Availability failures are tool errors.** "The application is offline" or "did
not answer in time" are returned as `CallToolResult` with `isError`, so the
model can tell the user to open the app. Protocol mistakes (unknown tool,
malformed request) stay JSON-RPC errors, as MCP prescribes.

**Verbatim forwarding.** Tool definitions, arguments and results pass through
as raw JSON; the bridge parses only what it needs. New MCP fields work without a
bridge release, and a large payload is not parsed and re-serialized on its way
(this took the bridge's CPU per 64 KiB call from 161 to 133 µs in one A/B measurement). Arguments are parsed
only for tools that declare `x-mcp-header` parameters, which MCP 2026-07-28
requires a server to check against the `Mcp-Param-*` headers.

**Slow calls stream.** A call not complete within a second is answered as an SSE
stream with a keep-alive comment every second. Two findings led here. Testing
cancellation with a call timeout long enough not to mask it showed that in
workerd neither the Worker nor the Durable Object learns that an agent hung up
on a pending JSON response; a failed write is the reliable signal, and the
Worker then tells the object to cancel. Second, a JSON response that waits for a
human for a minute is cut off by common proxy defaults (nginx: 60 s). Fast calls
stay plain JSON.

**A hang-up in the first second.** On Cloudflare itself (not in workerd), an
agent hanging up on a pending JSON response ends the Worker's invocation
without dropping anything, so nothing told the object, and the application
kept the call until the call timeout. The request's abort signal
(`enable_request_signal`) is the signal there: the Worker asks the object to
cancel, and the object also watches its own request's signal. That cancel can
overtake the call it is about, so the object remembers it for a few seconds
and cancels the call on arrival. Measured on `bridge.clientmcp.dev`: hang-ups
at 0.6–0.8 s were never cancelled before, and are now cancelled within
0.25 s, like hang-ups after the switch to SSE. A client behind an HTTP
proxy hides the difference: the proxy holds the connection to Cloudflare open
until the next write.

**A fixed 4 MiB message limit.** Making the limit part of the protocol lets the
SDK turn an oversized result into a tool error before sending it. Otherwise the
bridge has to close the connection mid-message; with the rest of the message
unread, the operating system resets the connection and the client sees `1006`
instead of `1009`. Cloudflare accepts up to 32 MiB, so 4 MiB is safe on both
runtimes.

**Exact-text ping.** `setWebSocketAutoResponse` answers a fixed byte string at
the edge without waking the Durable Object, so liveness costs nothing while an
application sits idle. The single-process bridge answers any ping.

## Runtimes

**Shared core.** `clientmcp-core` has no I/O: authentication, the client
channel, tool validation, both MCP eras and the call state machine — including
the async call flow (dispatch, wait, timeout, cancel on drop), written against a
two-method `Link` trait so that it runs on tokio and in Workers alike. Each
runtime is a few hundred lines of plumbing.

**Cloudflare.** One Durable Object per client ID holds the client's WebSocket
through the Hibernation API. The Worker verifies tokens before any object is
touched. Notification streams live in the Worker, which subscribes to the object
over an internal hibernatable WebSocket: holding the agent's SSE stream in the
object would keep it awake (and billed) for as long as an agent is connected.
Because hibernation drops memory, the last accepted `register` is kept in
object storage and the session is rebuilt on the next event; this was verified
by idling 45 seconds and watching the object rebuild while pings were answered
at the edge.

**Single process.** A map from client ID to connection is the whole state.
hyper and tokio-tungstenite directly, without a web framework: there are four
routes. Two measurements shaped the connection handling:
- tungstenite's default 128 KiB read and write buffers made an idle connection
  cost ~125 KiB; with a 4 KiB read buffer and unbuffered writes it costs ~11 KiB.
- One task per connection instead of a reader plus a writer saved ~0.8 KiB per
  connection but cost ~18% throughput for a client receiving many concurrent
  calls; the two-task design stayed.
mimalloc was measured too: 17–25% less CPU per call, but 54 MB instead of 1.5 MB
resident under the same load, and a C dependency. musl's allocator stayed.

## Limitations

- **Revocation** before `exp` requires rotating the secret.
- **Horizontal scaling** of the single-process bridge needs requests for a
  client ID routed to the instance holding it; one instance holds tens of
  thousands of clients. The Cloudflare bridge scales by construction.
- **Bridge trust.** The bridge sees all arguments and results; there is no
  end-to-end encryption between agent and client.
- **Not in v1:** progress notifications, resources, prompts, MCP's multi
  round-trip requests and tasks. They fit as new message types and `register`
  members.
