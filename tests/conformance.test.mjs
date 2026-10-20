// Conformance suite for a ClientMCP bridge (see SPEC.md). Runs against any
// deployment: BRIDGE_URL=... CLIENTMCP_SECRET=... node --test conformance.test.mjs

import assert from "node:assert/strict";
import http from "node:http";
import { afterEach, describe, test } from "node:test";
import { RISK_META, accepted, completed, rejected, toolResult } from "../sdk/js/dist/index.js";
import {
  ALLOWED_ORIGIN, BRIDGE, CALL_TIMEOUT, MCP_URL, MODERN, WS_URL, closeAll, legacyAgent, mint,
  modern, modernAgent, openStream, rpc, sleep, startApp, text, uid, waitFor,
} from "./helpers.mjs";

afterEach(closeAll);

// Cancellation must be observed before the bridge's own call timeout would
// cancel the call anyway, or a test could pass for the wrong reason.
const beforeTimeout = (since) => since + (CALL_TIMEOUT - 1.5) * 1000 - Date.now();

const echo = {
  description: "Echo the arguments",
  inputSchema: { type: "object", properties: { value: {} } },
  handler: (args) => args,
};

describe("front door", () => {
  test("health and unknown paths", async () => {
    assert.equal(await (await fetch(`${BRIDGE}/health`)).text(), "ok");
    assert.equal((await fetch(`${BRIDGE}/nope`)).status, 404);
  });

  test("agent requests need an agent token for the client", async () => {
    const id = uid();
    const ping = { jsonrpc: "2.0", id: 1, method: "ping" };
    const none = await fetch(MCP_URL, { method: "POST", body: JSON.stringify(ping) });
    assert.equal(none.status, 401);
    assert.equal(none.headers.get("www-authenticate"), "Bearer");
    const asClient = await rpc(id, ping, { Authorization: `Bearer ${mint("client", id)}` });
    assert.equal(asClient.status, 401);
    const expired = await rpc(id, ping, { Authorization: `Bearer ${mint("agent", id, { exp: 1000 })}` });
    assert.equal(expired.status, 401);
    const forged = mint("agent", id).slice(0, -2) + "xx";
    assert.equal((await rpc(id, ping, { Authorization: `Bearer ${forged}` })).status, 401);
    assert.equal((await rpc(id, ping)).status, 200);
  });

  test("the agent token may be the last path segment", async () => {
    const id = uid();
    await startApp(id, { echo });
    const response = await fetch(`${MCP_URL}/${mint("agent", id)}`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
    });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).result.tools[0].name, "echo");
  });

  test("browser origins are checked", async () => {
    const id = uid();
    const ping = { jsonrpc: "2.0", id: 1, method: "ping" };
    assert.equal((await rpc(id, ping, { Origin: "https://evil.example" })).status, 403);
    if (!ALLOWED_ORIGIN) return;
    const preflight = await fetch(MCP_URL, {
      method: "OPTIONS",
      headers: { Origin: ALLOWED_ORIGIN, "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "authorization,content-type,mcp-method" },
    });
    assert.equal(preflight.status, 204);
    assert.equal(preflight.headers.get("access-control-allow-origin"), ALLOWED_ORIGIN);
    assert.match(preflight.headers.get("access-control-allow-headers"), /authorization/);
    const allowed = await rpc(id, ping, { Origin: ALLOWED_ORIGIN });
    assert.equal(allowed.status, 200);
    assert.equal(allowed.headers.get("access-control-allow-origin"), ALLOWED_ORIGIN);
  });

  test("a client handshake without the subprotocol is refused with HTTP 400", async () => {
    const status = await new Promise((resolve, reject) => {
      const url = new URL(WS_URL.replace(/^ws/, "http"));
      const req = http.request(url, {
        headers: { Connection: "Upgrade", Upgrade: "websocket", "Sec-WebSocket-Version": "13", "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==" },
      });
      req.on("response", (res) => (res.socket.destroy(), resolve(res.statusCode)));
      req.on("upgrade", (res, socket) => (socket.destroy(), resolve(res.statusCode)));
      req.on("error", reject);
      req.end();
    });
    assert.equal(status, 400);
  });

  test("an invalid client token is refused with close code 4401", async () => {
    const socket = new WebSocket(WS_URL, ["clientmcp.v1", mint("agent", uid())]);
    const closed = await new Promise((resolve) => (socket.onclose = resolve));
    assert.equal(closed.code, 4401);
    assert.equal(socket.protocol, "clientmcp.v1");

    const app = startApp(uid(), {}, { token: "not-a-token" });
    await assert.rejects(app, /4401/);
  });
});

describe("legacy agent (MCP SDK v1, initialize handshake)", () => {
  test("initialize, list and call", async () => {
    const id = uid();
    await startApp(id, {
      greet: {
        title: "Greet",
        description: "Say hello",
        inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
        annotations: { readOnlyHint: true },
        handler: ({ name }) => `Hello, ${name}!`,
      },
      echo,
    }, { instructions: "A test application." });
    const agent = await legacyAgent(id);
    assert.equal(agent.getServerVersion().name, "clientmcp");
    assert.equal(agent.getInstructions(), "A test application.");
    assert.equal(agent.getServerCapabilities().tools.listChanged, true);

    const { tools } = await agent.listTools();
    assert.deepEqual(tools.map((t) => t.name), ["greet", "echo"]);
    assert.equal(tools[0].title, "Greet");
    assert.deepEqual(tools[0].annotations, { readOnlyHint: true });
    assert.deepEqual(tools[0].inputSchema.required, ["name"]);

    const greeting = await agent.callTool({ name: "greet", arguments: { name: "Ada" } });
    assert.deepEqual(greeting.content, [{ type: "text", text: "Hello, Ada!" }]);
    assert.equal(greeting.resultType, undefined);
    const echoed = await agent.callTool({ name: "echo", arguments: { value: { n: [1, "二", null] } } });
    assert.deepEqual(echoed.structuredContent, { value: { n: [1, "二", null] } });
    assert.deepEqual(JSON.parse(text(echoed)), { value: { n: [1, "二", null] } });
  });

  test("tool errors, rich results and unknown tools", async () => {
    const id = uid();
    await startApp(id, {
      fail: { handler: () => { throw new Error("insufficient funds"); } },
      image: { handler: () => toolResult({ content: [{ type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" }], _meta: { "app.example/x": 1 } }) },
      nothing: { handler: () => undefined },
    });
    const agent = await legacyAgent(id);
    const failed = await agent.callTool({ name: "fail" });
    assert.equal(failed.isError, true);
    assert.equal(text(failed), "insufficient funds");
    const image = await agent.callTool({ name: "image" });
    assert.deepEqual(image.content, [{ type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" }]);
    assert.deepEqual(image._meta, { "app.example/x": 1 });
    assert.deepEqual((await agent.callTool({ name: "nothing" })).content, []);
    await assert.rejects(agent.callTool({ name: "missing" }), (e) => e.code === -32602 && /Unknown tool: missing/.test(e.message));
  });

  test("risk and outcomes, as applications publish them (SPEC §9)", async () => {
    const id = uid();
    await startApp(id, {
      look: { risk: "observe", handler: () => completed({ items: 2 }) },
      wipe: { risk: "manage", annotations: { title: "Wipe" }, handler: () => rejected("denied", "the user said no") },
      pay: { risk: "transact", _meta: { "app.example/x": 1 }, handler: () => accepted("op-1") },
    });
    const agent = await legacyAgent(id);
    const { tools } = await agent.listTools();
    const byName = Object.fromEntries(tools.map((t) => [t.name, t]));
    assert.deepEqual(byName.look.annotations, { readOnlyHint: true });
    assert.deepEqual(byName.look._meta, { [RISK_META]: "observe" });
    // The tool's own annotations win; the implied ones fill in.
    assert.deepEqual(byName.wipe.annotations, { readOnlyHint: false, destructiveHint: true, title: "Wipe" });
    assert.deepEqual(byName.pay._meta, { "app.example/x": 1, [RISK_META]: "transact" });

    const looked = await agent.callTool({ name: "look" });
    assert.deepEqual(looked.structuredContent, { items: 2, status: "completed" });
    assert.equal(looked.isError, undefined);
    const wiped = await agent.callTool({ name: "wipe" });
    assert.equal(wiped.isError, true);
    assert.deepEqual(wiped.structuredContent, { status: "rejected", reason: { code: "denied", detail: "the user said no" } });
    const paid = await agent.callTool({ name: "pay" });
    assert.deepEqual(paid.structuredContent, { status: "accepted", operation: "op-1" });
    // The outcome's own members are not data's to set: a collision is a bug, said at once.
    assert.deepEqual(accepted("op-2", { transfer: { id: "op-2" } }).structuredContent, {
      transfer: { id: "op-2" },
      status: "accepted",
      operation: "op-2",
    });
    assert.throws(() => accepted("op-2", { operation: { id: "op-2" } }), TypeError);
    assert.throws(() => completed({ status: "done" }), TypeError);
    assert.throws(() => completed({ reason: "x" }), TypeError);
  });

  test("tool list changes reach the GET stream", async () => {
    const id = uid();
    const app = await startApp(id, { echo });
    let changes = 0;
    const agent = await legacyAgent(id, { onToolsChanged: () => changes++ });
    await sleep(300); // let the SDK open its GET stream
    app.expose("second", echo);
    await waitFor(() => changes >= 1, 5000, "list_changed");
    assert.deepEqual((await agent.listTools()).tools.map((t) => t.name), ["echo", "second"]);
  });

  test("an agent can start before the application", async () => {
    const id = uid();
    let changes = 0;
    const agent = await legacyAgent(id, { onToolsChanged: () => changes++ });
    assert.deepEqual((await agent.listTools()).tools, []);
    const offline = await agent.callTool({ name: "echo", arguments: {} });
    assert.equal(offline.isError, true);
    assert.match(text(offline), /offline/);
    await sleep(300);
    await startApp(id, { echo });
    await waitFor(() => changes >= 1, 5000, "list_changed after connect");
    assert.equal((await agent.callTool({ name: "echo", arguments: { value: 1 } })).structuredContent.value, 1);
  });
});

describe("modern agent (MCP SDK v2, 2026-07-28)", () => {
  test("pinned modern era: discover, list, call", async () => {
    const id = uid();
    await startApp(id, { echo }, { instructions: "Modern." });
    const agent = await modernAgent(id);
    assert.equal(agent.getProtocolEra(), "modern");
    assert.equal(agent.getInstructions(), "Modern.");
    const { tools } = await agent.listTools();
    assert.deepEqual(tools.map((t) => t.name), ["echo"]);
    const result = await agent.callTool({ name: "echo", arguments: { value: "x" } });
    assert.deepEqual(result.structuredContent, { value: "x" });
  });

  test("auto negotiation selects the modern era", async () => {
    const id = uid();
    await startApp(id, { echo });
    const agent = await modernAgent(id, { mode: "auto" });
    assert.equal(agent.getProtocolEra(), "modern");
    assert.equal((await agent.callTool({ name: "echo", arguments: { value: 2 } })).structuredContent.value, 2);
  });

  test("v2 in its default legacy mode works too", async () => {
    const id = uid();
    await startApp(id, { echo });
    const agent = await modernAgent(id, { mode: "legacy" });
    assert.equal((await agent.callTool({ name: "echo", arguments: { value: 3 } })).structuredContent.value, 3);
  });

  test("subscriptions/listen delivers tool list changes", async () => {
    const id = uid();
    const app = await startApp(id, { echo });
    const agent = await modernAgent(id);
    let changes = 0;
    agent.setNotificationHandler("notifications/tools/list_changed", () => changes++);
    const subscription = await agent.listen({ toolsListChanged: true, promptsListChanged: true });
    assert.deepEqual(subscription.honoredFilter, { toolsListChanged: true });
    app.unexpose("echo");
    await waitFor(() => changes >= 1, 5000, "list_changed");
    assert.deepEqual((await agent.listTools()).tools, []);
    await subscription.close();
  });
});

describe("protocol details", () => {
  test("modern header validation", async () => {
    const id = uid();
    await startApp(id, { echo });
    const list = modern("tools/list");
    const code = async (body, headers) => {
      const r = await rpc(id, body, headers);
      return [r.status, r.json?.error?.code];
    };
    assert.deepEqual(await code(list.body, { "MCP-Protocol-Version": MODERN }), [400, -32020]);
    assert.deepEqual(await code(list.body, { ...list.headers, "MCP-Protocol-Version": "2025-11-25" }), [400, -32020]);
    assert.deepEqual(await code(list.body, { ...list.headers, "Mcp-Method": "tools/call" }), [400, -32020]);
    const call = modern("tools/call", { name: "echo", arguments: {} });
    assert.deepEqual(await code(call.body, { ...call.headers, "Mcp-Name": "other" }), [400, -32020]);
    const noCaps = structuredClone(list.body);
    delete noCaps.params._meta["io.modelcontextprotocol/clientCapabilities"];
    assert.deepEqual(await code(noCaps, list.headers), [400, -32602]);
    assert.deepEqual(await code(modern("ping").body, modern("ping").headers), [404, -32601]);

    const future = structuredClone(list.body);
    future.params._meta["io.modelcontextprotocol/protocolVersion"] = "2099-01-01";
    const r = await rpc(id, future, { ...list.headers, "MCP-Protocol-Version": "2099-01-01" });
    assert.equal(r.status, 400);
    assert.equal(r.json.error.code, -32022);
    assert.ok(r.json.error.data.supported.includes(MODERN));
  });

  test("modern results carry resultType, cache hints and server identity", async () => {
    const id = uid();
    await startApp(id, { echo });
    const { body, headers } = modern("tools/list");
    const { json } = await rpc(id, body, headers);
    assert.equal(json.result.resultType, "complete");
    assert.equal(json.result.ttlMs, 0);
    assert.equal(json.result.cacheScope, "private");
    assert.equal(json.result._meta["io.modelcontextprotocol/serverInfo"].name, "clientmcp");
    const discover = modern("server/discover");
    const described = (await rpc(id, discover.body, discover.headers)).json.result;
    assert.deepEqual(described.capabilities, { tools: { listChanged: true } });
    assert.equal(described.supportedVersions[0], MODERN);
  });

  test("x-mcp-header parameters must be mirrored", async () => {
    const id = uid();
    await startApp(id, {
      query: {
        inputSchema: { type: "object", properties: { region: { type: "string", "x-mcp-header": "Region" }, sql: { type: "string" } } },
        handler: ({ region }) => region,
      },
    });
    const call = modern("tools/call", { name: "query", arguments: { region: "eu-west", sql: "select 1" } });
    assert.equal((await rpc(id, call.body, call.headers)).json.error.code, -32020);
    const ok = await rpc(id, call.body, { ...call.headers, "Mcp-Param-Region": "eu-west" });
    assert.equal(ok.json.result.content[0].text, "eu-west");
    const unicode = modern("tools/call", { name: "query", arguments: { region: "東京" } });
    const encoded = `=?base64?${Buffer.from("東京").toString("base64")}?=`;
    assert.equal((await rpc(id, unicode.body, { ...unicode.headers, "Mcp-Param-Region": encoded })).json.result.content[0].text, "東京");
  });

  test("malformed requests", async () => {
    const id = uid();
    assert.equal((await rpc(id, "{")).json.error.code, -32700);
    assert.equal((await rpc(id, "[]")).status, 400);
    assert.equal((await rpc(id, { jsonrpc: "2.0", id: 1, result: {} })).status, 400);
    const auth = { Authorization: `Bearer ${mint("agent", id)}` };
    assert.equal((await fetch(MCP_URL, { method: "DELETE", headers: auth })).status, 405);
    assert.equal((await fetch(MCP_URL, { headers: auth })).status, 405); // GET without Accept: text/event-stream
    assert.equal((await rpc(id, { jsonrpc: "2.0", method: "notifications/initialized" })).status, 202);
  });

  test("raw notification streams", async () => {
    const id = uid();
    const app = await startApp(id, { echo });
    const listen = modern("subscriptions/listen", { notifications: { toolsListChanged: true } }, "sub-1");
    const stream = await openStream(MCP_URL, {
      method: "POST",
      headers: { ...listen.headers, Authorization: `Bearer ${mint("agent", id)}`, "content-type": "application/json", accept: "text/event-stream" },
      body: JSON.stringify(listen.body),
    });
    assert.equal(stream.response.headers.get("content-type"), "text/event-stream");
    await waitFor(() => stream.events.length >= 1, 3000, "acknowledgement");
    assert.equal(stream.events[0].method, "notifications/subscriptions/acknowledged");
    assert.equal(stream.events[0].params._meta["io.modelcontextprotocol/subscriptionId"], "sub-1");
    app.expose("more", echo);
    await waitFor(() => stream.events.length >= 2, 3000, "list_changed");
    assert.deepEqual(stream.events[1], {
      jsonrpc: "2.0", method: "notifications/tools/list_changed", params: { _meta: { "io.modelcontextprotocol/subscriptionId": "sub-1" } },
    });
    app.close(); // going offline is a change too
    await waitFor(() => stream.events.length >= 3, 3000, "list_changed on disconnect");
  });
});

describe("calls", () => {
  test("concurrent calls are correlated", async () => {
    const id = uid();
    await startApp(id, {
      slow: { handler: async ({ n }) => (await sleep(Math.random() * 50), { n }) },
    });
    const agent = await modernAgent(id);
    const results = await Promise.all(
      Array.from({ length: 100 }, (_, n) => agent.callTool({ name: "slow", arguments: { n } })),
    );
    results.forEach((r, n) => assert.equal(r.structuredContent.n, n));
  });

  test("slow calls stream keep-alives, fast calls answer with JSON", async () => {
    const id = uid();
    await startApp(id, {
      fast: { handler: () => "fast" },
      slow: { handler: async () => (await sleep(2500), "slow") },
    });
    const post = async (name) => {
      const { body, headers } = modern("tools/call", { name, arguments: {} });
      return fetch(MCP_URL, {
        method: "POST",
        headers: { ...headers, Authorization: `Bearer ${mint("agent", id)}`, "content-type": "application/json", accept: "application/json, text/event-stream" },
        body: JSON.stringify(body),
      });
    };
    const fast = await post("fast");
    assert.equal(fast.headers.get("content-type"), "application/json");
    assert.equal((await fast.json()).result.content[0].text, "fast");
    const slow = await post("slow");
    assert.equal(slow.headers.get("content-type"), "text/event-stream");
    const events = (await slow.text()).split("\n\n").filter(Boolean);
    assert.ok(events.length >= 2 && events[0] === ":", "keep-alive comments come first");
    assert.equal(JSON.parse(events.at(-1).replace(/^data: /, "")).result.content[0].text, "slow");
    // Both SDK generations read streamed results.
    assert.equal(text(await (await legacyAgent(id)).callTool({ name: "slow" })), "slow");
    assert.equal(text(await (await modernAgent(id)).callTool({ name: "slow" })), "slow");
  });

  test("a call times out and the client is told to cancel", { skip: CALL_TIMEOUT > 5 && "needs CLIENTMCP_CALL_TIMEOUT <= 5" }, async () => {
    const id = uid();
    let aborted = false;
    await startApp(id, {
      hang: { handler: (_, { signal }) => new Promise((_, reject) => signal.addEventListener("abort", () => ((aborted = true), reject(signal.reason)))) },
    });
    const agent = await legacyAgent(id);
    const result = await agent.callTool({ name: "hang" });
    assert.equal(result.isError, true);
    assert.match(text(result), /in time/);
    await waitFor(() => aborted, 2000, "cancel");
  });

  test("an agent closing the request cancels the call", async () => {
    const id = uid();
    let aborted = false;
    let started = false;
    await startApp(id, {
      hang: { handler: (_, { signal }) => new Promise(() => ((started = true), signal.addEventListener("abort", () => (aborted = true)))) },
    });
    const { body, headers } = modern("tools/call", { name: "hang", arguments: {} });
    const controller = new AbortController();
    const request = fetch(MCP_URL, {
      method: "POST",
      signal: controller.signal,
      headers: { ...headers, Authorization: `Bearer ${mint("agent", id)}`, "content-type": "application/json", accept: "application/json, text/event-stream" },
      body: JSON.stringify(body),
    }).catch(() => {});
    await waitFor(() => started, 3000, "call start");
    const since = Date.now();
    controller.abort();
    await request;
    await waitFor(() => aborted, beforeTimeout(since), "cancel after disconnect");
  });

  test("a legacy notifications/cancelled cancels the call", async () => {
    const id = uid();
    let aborted = false;
    let started = false;
    await startApp(id, {
      hang: { handler: (_, { signal }) => new Promise(() => ((started = true), signal.addEventListener("abort", () => (aborted = true)))) },
    });
    const agent = await legacyAgent(id);
    const controller = new AbortController();
    const call = agent.callTool({ name: "hang" }, undefined, { signal: controller.signal }).catch((e) => e);
    await waitFor(() => started, 3000, "call start");
    const since = Date.now();
    controller.abort("user gave up");
    await call;
    await waitFor(() => aborted, beforeTimeout(since), "cancel via notification");
  });

  test("calls in flight fail when the application disconnects", async () => {
    const id = uid();
    let started = false;
    const app = await startApp(id, { hang: { handler: () => new Promise(() => (started = true)) } });
    const agent = await legacyAgent(id);
    const call = agent.callTool({ name: "hang" });
    await waitFor(() => started, 3000, "call start");
    app.close();
    const result = await call;
    assert.equal(result.isError, true);
    assert.match(text(result), /disconnected/);
  });

  test("too many calls in flight are refused", async () => {
    const id = uid();
    let release;
    const gate = new Promise((r) => (release = r));
    let started = 0;
    await startApp(id, { wait: { handler: async () => (started++, await gate, "done") } });
    const agent = await modernAgent(id);
    let busy = 0;
    const calls = Array.from({ length: 300 }, () =>
      agent.callTool({ name: "wait" }).then((r) => (r.isError && /too many/.test(text(r)) && busy++, r)),
    );
    // Nothing can finish before the gate opens, so exactly 256 run and 44 are refused.
    await waitFor(() => started === 256 && busy === 44, 15000, "256 running, 44 refused");
    release();
    const results = await Promise.all(calls);
    assert.equal(results.filter((r) => text(r) === "done").length, 256);
    assert.equal(started, 256);
  });

  test("large results pass; oversized results become tool errors", async () => {
    const id = uid();
    const app = await startApp(id, {
      big: { handler: ({ size }) => "x".repeat(size) },
    });
    const agent = await legacyAgent(id);
    assert.equal(text(await agent.callTool({ name: "big", arguments: { size: 4_000_000 } })).length, 4_000_000);
    const tooBig = await agent.callTool({ name: "big", arguments: { size: 5_000_000 } });
    assert.equal(tooBig.isError, true);
    assert.match(text(tooBig), /too large/);
    assert.equal(app.status, "online");
  });

  test("an oversized message closes the connection with 1009", async () => {
    const socket = new WebSocket(WS_URL, ["clientmcp.v1", mint("client", uid())]);
    await new Promise((resolve) => (socket.onopen = resolve));
    socket.send(JSON.stringify({ type: "register", tools: [], instructions: "x".repeat(5 * 1024 * 1024) }));
    const closed = await new Promise((resolve) => (socket.onclose = resolve));
    assert.ok([1009, 1006].includes(closed.code), `closed with ${closed.code}`);
  });

  test("an agent request body over 4 MiB is refused", async () => {
    const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping", params: { pad: "x".repeat(5 * 1024 * 1024) } });
    // A bridge answering before the upload ends may reset the connection instead.
    const status = await rpc(uid(), body).then((r) => r.status, () => "reset");
    assert.ok([413, "reset"].includes(status), `got ${status}`);
  });
});

describe("client lifecycle", () => {
  test("the newest connection wins", async () => {
    const id = uid();
    const first = await startApp(id, { first: echo });
    const second = await startApp(id, { second: echo });
    await waitFor(() => first.status === "closed", 3000, "replacement");
    assert.equal(first.statuses.at(-1).code, 4409);
    assert.equal(second.status, "online");
    const agent = await legacyAgent(id);
    assert.deepEqual((await agent.listTools()).tools.map((t) => t.name), ["second"]);
  });

  test("invalid tool definitions are rejected with 4400", async () => {
    const id = uid();
    const app = startApp(id, { "bad name": echo });
    await assert.rejects(app, /4400/);
    const bad = startApp(uid(), { t: { ...echo, inputSchema: { type: "object", properties: { n: { type: "number", "x-mcp-header": "N" } } } } });
    await assert.rejects(bad, /4400/);
  });

  test("clients are isolated from each other", async () => {
    const a = uid();
    const b = uid();
    await startApp(a, { onlyA: echo });
    await startApp(b, { onlyB: echo });
    const agentA = await legacyAgent(a);
    assert.deepEqual((await agentA.listTools()).tools.map((t) => t.name), ["onlyA"]);
    await assert.rejects(agentA.callTool({ name: "onlyB" }), /Unknown tool/);
  });

  test("register replaces the declared state", async () => {
    const id = uid();
    const app = await startApp(id, { a: echo, b: echo }, { instructions: "v1" });
    const agent = await legacyAgent(id);
    app.unexpose("a");
    app.instructions = undefined;
    await waitFor(async () => (await agent.listTools()).tools.length === 1, 3000, "re-register");
    const again = await legacyAgent(id);
    assert.equal(again.getInstructions(), undefined);
  });
});
