# ClientMCP

**Expose live client applications as MCP servers.**

A browser, desktop or mobile application opens one outbound WebSocket to a
ClientMCP bridge and declares tools. AI agents use them through the bridge's
standard MCP endpoint. The tools run inside the application, on the same state
and actions as its UI — when an agent adds a todo, the user sees it appear.

```text
 Human ──► UI ─┐
               ▼
        Application state & actions ◄── ClientMCP SDK
                                              │  WebSocket, outbound only
                                              ▼
                                       ClientMCP bridge  ── Docker, or Cloudflare Workers
                                              ▲
                                              │  MCP Streamable HTTP (2026-07-28 and 2025 revisions)
                                           AI agent
```

- **No server in the app.** The application opens no port; the bridge stores nothing.
- **Standard MCP.** Any MCP client works — the bridge speaks MCP 2026-07-28 and
  the 2025 revisions on one endpoint.
- **Two deployments, one protocol.** A 1.2 MB static binary for any Linux host,
  or Cloudflare Workers with Durable Objects, where idle applications cost
  nothing. Both share one Rust core and pass the same conformance suite.

## Quick start

**1. Run a bridge.**

```sh
docker build -t clientmcp .
export CLIENTMCP_SECRET=$(openssl rand -hex 32)
docker run -d -p 8080:8080 -e CLIENTMCP_SECRET clientmcp
```

**2. Mint tokens.** A client token lets the application connect as `my-app`; an
agent token lets an agent call it.

```sh
docker run --rm -e CLIENTMCP_SECRET clientmcp token client my-app
docker run --rm -e CLIENTMCP_SECRET clientmcp token agent my-app --ttl 30d
```

**3. Expose functions from the application** ([SDK reference](sdk/js/README.md)):

```ts
import { ClientMCP } from "clientmcp";

const mcp = new ClientMCP({ url: "ws://localhost:8080/connect", token: CLIENT_TOKEN });
mcp.expose("add_todo", {
  description: "Add a todo",
  inputSchema: { type: "object", properties: { title: { type: "string" } }, required: ["title"] },
  handler: ({ title }) => todos.add(title), // the same action the UI calls
});
await mcp.connect();
```

Rust applications (desktop, or a mobile app's core) use `clientmcp-client`
(`crates/client`), with the same protocol and behaviour:

```rust
let mcp = Client::new(Options::new("ws://localhost:8080/connect", client_token));
mcp.expose("add_todo", Tool::new(move |args, _ctx| add_todo(args)).description("Add a todo"));
mcp.connect().await?;
```

**4. Connect an agent**, e.g. Claude Code:

```sh
claude mcp add --transport http my-app http://localhost:8080/mcp \
  --header "Authorization: Bearer $AGENT_TOKEN"
```

Hosts that cannot send headers can use the token as the last path segment:
`https://bridge.example.com/mcp/<agent token>` — such a URL is a secret.

[`examples/browser`](examples/browser/index.html) is a complete application.
Build the SDK (`cd sdk/js && npm install && npm run build`), serve the
repository root with any static server, and open
`examples/browser/index.html?bridge=ws://localhost:8080/connect&token=<client token>`.

## Deploy

### Linux / Docker

The image is a static binary on `scratch`, running as an unprivileged user.
Put TLS in front of it; [`deploy/`](deploy) has a Compose file with Caddy that
obtains certificates automatically. Without Docker:
`cargo build --release -p clientmcp-server` and run `target/release/clientmcp`.

### Cloudflare Workers

```sh
cargo install worker-build
cd crates/worker
npx wrangler secret put CLIENTMCP_SECRET
npx wrangler deploy
```

One Durable Object per client ID holds its WebSocket through the Hibernation
API; pings are answered at the edge and agents' notification streams are held
by the Worker, so a connected but idle application does not keep anything awake.

### Configuration

| Variable                    | Default          | Meaning                                                                 |
| --------------------------- | ---------------- | ----------------------------------------------------------------------- |
| `CLIENTMCP_SECRET`          | — (required)     | Token secrets, ≥ 32 bytes, newest first, space-separated for rotation.  |
| `CLIENTMCP_CALL_TIMEOUT`    | `120`            | Seconds before an unanswered call fails and is cancelled.               |
| `CLIENTMCP_CLIENT_IDLE`     | `90`             | Seconds an application may send nothing (pings included) before it is presumed gone and closed; its calls fail then, not at the call timeout. |
| `CLIENTMCP_ALLOWED_ORIGINS` | none             | Browser origins allowed on `/mcp` (comma-separated, or `*`).            |
| `CLIENTMCP_LISTEN`          | `127.0.0.1:8080` | Listen address (server only; the image uses `0.0.0.0:8080`).            |

Endpoints: `/connect` (applications), `/mcp` and `/mcp/<token>` (agents), `/health`.

## Tokens

Tokens are HS256 JWTs, so an application backend can mint them with any JWT
library:

```json
{ "sub": "my-app", "role": "client", "exp": 1798761600 }
```

`sub` is the client ID: a token for `my-app` can only connect as, or call,
`my-app`. In production, have your backend issue short-lived client tokens
(the SDK accepts a token function, called on every reconnection) and give
agents tokens with an `exp`. See [SPEC.md §2](SPEC.md#2-tokens) and §7.

## Protocol

[SPEC.md](SPEC.md) is the normative specification; [docs/design.md](docs/design.md)
explains the decisions behind it. In short, the client channel is six JSON
messages:

```text
client → bridge   register {tools, instructions}   result {id, result}   ping
bridge → client   registered   call {id, name, arguments}   cancel {id}   pong
```

The bridge maps them onto MCP: `tools/list` returns the declared tools,
`tools/call` becomes a `call`, and every change is pushed as
`notifications/tools/list_changed`. An offline application simply has no
tools; agents stay connected and are notified when it returns.

## Verification

```sh
cargo test --workspace --exclude clientmcp-worker   # core unit tests
tests/run-native.sh                                  # conformance suite vs. the server
tests/run-worker.sh                                  # conformance suite vs. the Worker in workerd
CLIENTMCP_APP=rust tests/run-native.sh               # the same, with the Rust client as the application
```

The conformance suite (`tests/`, 34 cases) drives a bridge with the official
MCP SDKs as agents — v1 (2025 era) and v2 (2026-07-28, pinned and
auto-negotiated) — and this repository's SDK as the application; with
`CLIENTMCP_APP=rust` the Rust client plays the application instead (its
handlers stay in the suite, the Rust client does all of the protocol). It covers
authentication, origin checks and CORS, both MCP eras, header validation,
`x-mcp-header`, notification streams, streamed slow calls, concurrency,
timeouts, cancellation in both eras (checked before the call timeout could
cancel instead), disconnection, replacement, size limits and tool validation. It
passes unchanged against the native server, the Docker image, the Worker
running in workerd and the deployed Worker (`bridge.clientmcp.dev`, where the
call-timeout case is skipped: it needs `CLIENTMCP_CALL_TIMEOUT` of 5 or less).
`tests/run-idle.sh` runs `idle.test.mjs` against the native server and the
Worker in workerd with a 3-second client idle: a call to an application that
went silent fails at the idle limit, its tools stop being offered, and one that
keeps pinging is kept.
Beyond the suite: the browser example was exercised end to end in Chrome —
agent changes appeared in the UI, user edits were visible to the agent, a
declined confirmation reached the agent as a tool error, and the page
reconnected and re-registered after a bridge restart — and Claude Code, as a
real agent, listed and added todos in a running application through the bridge.

### Performance

`clientmcp-bench` (in `crates/bench`) runs echo clients and concurrent agents over
keep-alive HTTP. Numbers below are from loopback runs with the load generator
on the same machine, so they understate what the bridge alone can do.

| Measure (Linux, aarch64, static binary)                                | Result                         |
| ---------------------------------------------------------------------- | ------------------------------ |
| Bridge CPU per `tools/call` round trip, 2025 / 2026-07-28 MCP          | ~11 µs / ~14 µs                |
| Bridge CPU per `tools/list`                                            | ~5.5 µs                        |
| Bridge CPU per call with 64 KiB arguments and result                   | ~110 µs                        |
| One call, unloaded                                                     | p50 54 µs, p99 134 µs          |
| 64 agents → 1 client, 2 vCPUs shared with the load generator           | 86–93k calls/s, p99 ≈ 1.9 ms   |
| 256 agents → 64 clients, same                                          | 71–85k calls/s, p99 8–11 ms    |
| Memory: idle process / per idle connected client                       | 1.2 MB / ~11 KB                |
| CPU while 10,000 clients sit idle (pinging every 25 s)                 | 0                              |

On an Apple M2 Pro (macOS, 12 cores shared with the load generator), 64
agents calling one client sustain 81–90k calls/s with p99 ≈ 1.4–2.2 ms.

The Worker passes the same suite in workerd, but `wrangler dev`'s local proxy
caps throughput at ~1.2k requests/s, so local numbers say nothing about
Cloudflare's network; measure a deployed Worker for that.

## Repository

```text
SPEC.md            the protocol
crates/core        protocol logic shared by both runtimes, no I/O
crates/server      the Linux/Docker bridge (tokio, hyper, tokio-tungstenite)
crates/worker      the Cloudflare bridge (workers-rs, Durable Objects)
sdk/js             the TypeScript SDK for applications
crates/client      the Rust client for applications, as sdk/js
tests              the conformance suite
crates/bench       the load generator
examples/browser   a shared todo list
deploy             Compose + Caddy
```

## Limitations

- Tokens can be revoked before `exp` only by rotating the secret.
- The server bridge is a single instance: several instances need requests for
  one client ID routed to the same instance.
- The bridge sees all arguments and results; it must be trusted.
- v1 covers tools. Progress notifications, resources, prompts and MCP's multi
  round-trip requests are not implemented.

## License

MIT or Apache-2.0, at your option.
