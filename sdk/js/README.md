# clientmcp

Expose a running application's functions as MCP tools. The application keeps
one outbound WebSocket to a [ClientMCP bridge](../../README.md); AI agents call
its tools through the bridge's standard MCP endpoint. No dependencies; works in
browsers, Node.js 22+, Deno and Bun.

```ts
import { ClientMCP } from "clientmcp";

const mcp = new ClientMCP({
  url: "wss://bridge.example.com/connect",
  token: () => fetch("/api/clientmcp-token").then((r) => r.text()),
  instructions: "Alice's wallet. Amounts are decimal strings in ETH.",
});

mcp.expose("getBalance", {
  description: "Get the balance of the open account",
  inputSchema: { type: "object", properties: { chainId: { type: "integer" } }, required: ["chainId"] },
  annotations: { readOnlyHint: true },
  handler: ({ chainId }) => wallet.getBalance(chainId), // the same action the UI uses
});

await mcp.connect();
```

## API

### `new ClientMCP(options)`

| Option         | Type                                         | Description                                                                 |
| -------------- | -------------------------------------------- | --------------------------------------------------------------------------- |
| `url`          | `string`                                     | The bridge's client endpoint, e.g. `wss://bridge.example.com/connect`.      |
| `token`        | `string \| () => string \| Promise<string>`  | A client token. A function is called on every (re)connection, so short-lived tokens from your backend work. |
| `instructions` | `string`                                     | Guidance for the model on how to use the tools.                             |
| `onStatus`     | `(status, detail?) => void`                  | Called on every status change; `detail` is `{ code, reason }` of the close that caused it. |
| `WebSocket`    | `typeof WebSocket`                           | An implementation for runtimes without a global `WebSocket`.               |

### `expose(name, tool) → () => void`

Exposes a tool and returns a function that removes it. `tool` is an MCP tool
definition (`title`, `description`, `inputSchema` — default `{ type: "object" }`,
`outputSchema`, `annotations`, `icons`, `_meta`) plus a `handler(args, { name, signal })`.
Changes are sent to the bridge right away; agents are notified.

The handler's return value becomes the tool result:

| Returned                     | Result                                                                 |
| ---------------------------- | ---------------------------------------------------------------------- |
| a string                     | one text block                                                         |
| an object                    | its JSON as text, and the object as `structuredContent`                |
| any other JSON value         | its JSON as text                                                       |
| `undefined` / `null`         | no content                                                             |
| `toolResult({ content, … })` | that `CallToolResult`, unchanged (images, resources, `isError`, …)     |
| a thrown error               | a tool error carrying the message, which the model sees                |

`args` come from a language model: **validate them**, and ask the user before
doing anything consequential — throw (e.g. `"The user declined"`) to refuse.
`signal` aborts when the agent cancels, the call times out or the connection
drops. A handler that waits for the user should listen to it: the agent has
already been told the call failed, so withdraw the question and do nothing
after it (SPEC §9.1). Results over 4 MiB cannot be delivered and become tool
errors.

#### Risk and outcomes (SPEC §9)

Two optional conventions let every application say the same things the same
way. The bridge passes them through unread; what a level requires is the
application's to decide and enforce.

```js
import { completed, accepted, rejected } from "clientmcp";

app.expose("delete_contact", {
  risk: "manage", // observe | operate | manage | transact
  inputSchema: { type: "object", properties: { address: { type: "string" } }, required: ["address"] },
  handler: async ({ address }) => {
    if (!(await askTheUser(`Delete ${address}?`))) return rejected("denied", "the user said no");
    await deleteContact(address);
    return completed(); // the change holds — the app has seen it
  },
});
```

- `risk` is published as `_meta["io.github.clientmcp/risk"]`, with the MCP
  annotations it implies (`readOnlyHint` for `observe`, `destructiveHint` for
  `manage` and `transact`); the tool's own `annotations` win.
- `completed(data?)`, `accepted(operation, data?)` and `rejected(code, detail?)`
  answer with `structuredContent.status` — the change holds; it started and goes
  on as `operation` (its id); nothing changed, for `code` (`isError: true`).
  `data` is merged in, and may not use the outcome's own names (`status`,
  `operation`, `reason`): that throws a `TypeError`.

### `unexpose(name)`, `instructions = text`

Remove a tool; replace the instructions.

### `connect() → Promise<void>`

Connects and keeps the connection alive: pings every 25 s, reconnects with
exponential backoff (0.5 s to 30 s), re-registers the tools. Resolves once the
bridge has accepted the token and the tools; rejects if the bridge refuses them
for good.

### `close()`

Disconnects for good and aborts calls in flight.

### `status`

`"connecting" | "online" | "offline" | "closed"`. The SDK stops (`"closed"`)
without reconnecting when the bridge closes with `4400` (protocol error, e.g. an
invalid tool definition), `4409` (another connection took over this client ID),
`1009` (message too large), or `4401` (invalid token) when `token` is a string.
