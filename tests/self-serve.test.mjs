// Token generations and the internal issuance API (docs/self-serve.md, phase 1),
// on the Cloudflare bridge. Run by run-self-serve.sh, which sets
// CLIENTMCP_SERVICE_SECRET and a small CLIENTMCP_ISSUE_LIMIT.

import { after, afterEach, describe, test } from "node:test";
import assert from "node:assert/strict";
import { BRIDGE, MCP_URL, WS_URL, closeAll, mint, modern, sleep, uid } from "./helpers.mjs";

const SERVICE = process.env.CLIENTMCP_SERVICE_SECRET;
const ISSUE_LIMIT = Number(process.env.CLIENTMCP_ISSUE_LIMIT ?? 30);
const RATE_LIMIT = Number(process.env.CLIENTMCP_RATE_LIMIT ?? 600);
if (!SERVICE) throw new Error("run with CLIENTMCP_SERVICE_SECRET (run-self-serve.sh)");

const sockets = [];
afterEach(async () => {
  for (const socket of sockets.splice(0)) socket.close();
  await closeAll();
});
after(() => closeAll());

const inAYear = () => Math.floor(Date.now() / 1000) + 365 * 24 * 3600;

/** The internal API, as the site calls it. */
async function internal(path, body, secret = SERVICE) {
  const response = await fetch(BRIDGE + path, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json", ...(secret ? { "x-clientmcp-service": secret } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  let json;
  try {
    json = JSON.parse(text);
  } catch {}
  return { status: response.status, json, text };
}

const mintFor = (id, extra = {}) => internal(`/internal/clients/${id}/mint`, { exp: inAYear(), actor: "test", ...extra });

/** A client on `token` that registers one tool; resolves once registered, or with its close. */
async function client(token) {
  const socket = new WebSocket(WS_URL, ["clientmcp.v1", token]);
  sockets.push(socket);
  const closed = new Promise((resolve) => socket.addEventListener("close", (e) => resolve(e.code)));
  const registered = new Promise((resolve) => {
    socket.addEventListener("message", (e) => {
      const message = JSON.parse(e.data);
      if (message.type === "registered") resolve(true);
      if (message.type === "call") socket.send(JSON.stringify({ type: "result", id: message.id, result: { content: [{ type: "text", text: "ok" }] } }));
    });
  });
  socket.addEventListener("open", () =>
    socket.send(JSON.stringify({ type: "register", tools: [{ name: "echo", description: "Echo", inputSchema: { type: "object" } }] })),
  );
  const first = await Promise.race([registered, closed]);
  return { socket, closed, ok: first === true, code: first === true ? undefined : first };
}

/** The agent endpoint's answer to tools/list with `token` (and any extra headers). */
async function listTools(token, headers = {}) {
  const { body, headers: mirrored } = modern("tools/list");
  const response = await fetch(MCP_URL, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      authorization: `Bearer ${token}`,
      ...mirrored,
      ...headers,
    },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  let tools;
  try {
    tools = JSON.parse(text).result?.tools;
  } catch {}
  return { status: response.status, tools };
}

/** A tools/call of `echo` with `token`: the HTTP status and the result. */
async function callEcho(token, args = {}) {
  const { body, headers: mirrored } = modern("tools/call", { name: "echo", arguments: args });
  const response = await fetch(MCP_URL, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      authorization: `Bearer ${token}`,
      ...mirrored,
    },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  let result;
  try {
    result = JSON.parse(text).result;
  } catch {}
  return { status: response.status, result };
}

const today = () => new Date().toISOString().slice(0, 10);

describe("the internal API", () => {
  test("is not there without the service secret, or with a wrong one", async () => {
    const id = uid();
    // `null`, not `undefined`: an explicit `undefined` takes the default (the real secret).
    for (const secret of [null, "", "x".repeat(40), SERVICE + "x"]) {
      const { status } = await internal(`/internal/clients/${id}/mint`, { exp: inAYear(), actor: "t" }, secret);
      assert.equal(status, 404, `secret ${JSON.stringify(secret?.slice(0, 4))}`);
    }
    assert.equal((await internal(`/internal/clients/${id}`, undefined, null)).status, 404);
  });

  test("mints both tokens at the current generations, signed by the bridge", async () => {
    const id = uid();
    const { status, json } = await mintFor(id);
    assert.equal(status, 200);
    assert.equal(json.client.gen, 0);
    assert.equal(json.agent.gen, 0);
    const claims = JSON.parse(Buffer.from(json.agent.token.split(".")[1], "base64url"));
    assert.deepEqual({ sub: claims.sub, role: claims.role, gen: claims.gen }, { sub: id, role: "agent", gen: 0 });
    const app = await client(json.client.token);
    assert.ok(app.ok, "the minted client token connects");
    const listed = await listTools(json.agent.token);
    assert.equal(listed.status, 200);
    assert.equal(listed.tools.length, 1);
  });

  test("asks who asked, and refuses an expiry beyond 400 days", async () => {
    const id = uid();
    assert.equal((await internal(`/internal/clients/${id}/mint`, { exp: inAYear() })).status, 400);
    const far = Math.floor(Date.now() / 1000) + 401 * 24 * 3600;
    assert.equal((await mintFor(id, { exp: far })).status, 400);
    assert.equal((await mintFor(id, { exp: 1 })).status, 400);
  });
});

describe("the bridge's own records", () => {
  test("usage and the audit are there for the site, and for nobody else", async () => {
    for (const path of ["/internal/usage", "/internal/audit"]) {
      assert.equal((await internal(path, undefined, null)).status, 404, path);
    }
    const id = uid();
    await mintFor(id);
    const audit = await internal("/internal/audit?limit=5");
    assert.equal(audit.status, 200);
    const mine = audit.json.filter((row) => row.client === id);
    assert.deepEqual(mine.map((r) => [r.action, r.actor]).sort(), [["mint", "test"], ["mint", "test"]]);
    const usage = await internal("/internal/usage?days=7");
    assert.equal(usage.status, 200);
    assert.ok(Array.isArray(usage.json), "finished days, as rows (none yet on a fresh bridge)");
  });
});

describe("generations", () => {
  test("a token minted before generations keeps working", async () => {
    const id = uid();
    const app = await client(mint("client", id));
    assert.ok(app.ok);
    assert.equal((await listTools(mint("agent", id))).tools.length, 1);
  });

  test("rotating the client token closes the connection on the old one, and only that", async () => {
    const id = uid();
    const before = await client(mint("client", id));
    assert.ok(before.ok);
    const { status, json } = await internal(`/internal/clients/${id}/rotate`, { role: "client", exp: inAYear(), actor: "test" });
    assert.equal(status, 200);
    assert.equal(json.client.gen, 1);
    assert.equal(await before.closed, 4401, "the open connection on the old token ends at once");
    const old = await client(mint("client", id));
    assert.equal(old.code, 4401, "the old token is refused");
    const fresh = await client(json.client.token);
    assert.ok(fresh.ok, "the new one connects");
    assert.equal((await listTools(mint("agent", id))).status, 200, "the agent's token is untouched");
  });

  test("rotating the agent token with a grace keeps the old one until it ends", async () => {
    const id = uid();
    await client(mint("client", id));
    const graced = await internal(`/internal/clients/${id}/rotate`, { role: "agent", exp: inAYear(), grace_seconds: 3600, actor: "test" });
    assert.equal(graced.json.agent.gen, 1);
    assert.equal((await listTools(mint("agent", id))).status, 200, "the old token, within its grace");
    assert.equal((await listTools(graced.json.agent.token)).status, 200, "the new one");
    const sharp = await internal(`/internal/clients/${id}/rotate`, { role: "agent", exp: inAYear(), actor: "test" });
    assert.equal(sharp.json.agent.gen, 2);
    assert.equal((await listTools(graced.json.agent.token)).status, 401, "no grace: the previous one stops");
    assert.equal((await listTools(mint("agent", id))).status, 401, "and the one before it");
    assert.equal((await listTools(sharp.json.agent.token)).status, 200);
  });

  test("a grace is for the agent token, and at most a day", async () => {
    const id = uid();
    const client24 = await internal(`/internal/clients/${id}/rotate`, { role: "client", exp: inAYear(), grace_seconds: 60, actor: "t" });
    assert.equal(client24.status, 400);
    const long = await internal(`/internal/clients/${id}/rotate`, { role: "agent", exp: inAYear(), grace_seconds: 86401, actor: "t" });
    assert.equal(long.status, 400);
    assert.deepEqual((await internal(`/internal/clients/${id}`)).json?.agent ?? 0, 0, "nothing moved");
  });

  test("revoking the agent token leaves the client connected", async () => {
    const id = uid();
    const app = await client(mint("client", id));
    const { status } = await internal(`/internal/clients/${id}/revoke`, { role: "agent", actor: "test" });
    assert.equal(status, 200);
    assert.equal((await listTools(mint("agent", id))).status, 401);
    await sleep(300);
    assert.equal(app.socket.readyState, WebSocket.OPEN, "the client is still there");
  });

  test("a generation header from outside changes nothing: the Worker sets its own", async () => {
    const id = uid();
    await client(mint("client", id));
    await internal(`/internal/clients/${id}/revoke`, { role: "agent", actor: "test" });
    const spoofed = await listTools(mint("agent", id), { "x-clientmcp-generation": "1" });
    assert.equal(spoofed.status, 401);
  });

  test("a deleted client accepts nothing and is minted nothing", async () => {
    const id = uid();
    const app = await client(mint("client", id));
    assert.equal((await internal(`/internal/clients/${id}/delete`, { actor: "test" })).status, 200);
    assert.equal(await app.closed, 4401);
    assert.equal((await client(mint("client", id))).code, 4401);
    assert.equal((await listTools(mint("agent", id))).status, 401);
    assert.equal((await mintFor(id)).status, 410);
  });

  test("showing a credential again is the same token, and costs nothing", async () => {
    const id = uid();
    const first = (await mintFor(id)).json;
    for (let i = 0; i < ISSUE_LIMIT + 2; i++) {
      const again = await mintFor(id, { exp: inAYear() + 1000 + i });
      assert.equal(again.status, 200, `shown again, ${i + 1}`);
      assert.equal(again.json.agent.token, first.agent.token, "the very same agent token");
      assert.equal(again.json.client.token, first.client.token, "the very same client token");
    }
    const audit = (await internal("/internal/audit?limit=50")).json.filter((r) => r.client === id);
    assert.equal(audit.filter((r) => r.action === "mint").length, 2, "issued once per role");
    assert.ok(audit.some((r) => r.action === "reveal"), "shown again, audited as such");
  });

  test("new generations are limited per client", async () => {
    const id = uid();
    await mintFor(id);
    let status = 200;
    let rotated = 0;
    while (status === 200 && rotated <= ISSUE_LIMIT) {
      status = (await internal(`/internal/clients/${id}/rotate`, { role: "agent", exp: inAYear(), actor: "test" })).status;
      if (status === 200) rotated++;
    }
    assert.equal(status, 429, `refused after ${rotated} rotations`);
    assert.equal(rotated, ISSUE_LIMIT);
    assert.equal((await mintFor(uid())).status, 200, "another client is not affected");
    assert.equal((await mintFor(id)).status, 200, "the current credentials can still be shown");
  });
});

describe("metering", () => {
  test("counts calls, bytes, connections and refusals exactly — never content", async () => {
    const id = uid();
    const app = await client(mint("client", id));
    assert.ok(app.ok);
    const secret = "a-wallet-address-or-any-argument";
    for (let i = 0; i < 3; i++) assert.equal((await callEcho(mint("agent", id), { note: secret })).status, 200);
    await listTools(mint("agent", id));
    await internal(`/internal/clients/${id}/revoke`, { role: "agent", actor: "test" });
    assert.equal((await listTools(mint("agent", id))).status, 401);

    const { status, json, text } = await internal(`/internal/clients/${id}/usage`);
    assert.equal(status, 200);
    const day = json.find((d) => d.day === today());
    assert.ok(day, `today's counts: ${text}`);
    assert.equal(day.calls_ok, 3);
    assert.equal(day.requests, 1, "the tools/list");
    assert.equal(day.connections, 1);
    assert.equal(day.rejections, 1, "the revoked agent token");
    assert.ok(day.bytes_in > 0 && day.bytes_out > 0);
    assert.ok(!text.includes(secret), "no argument is kept");
    assert.ok(!text.includes("echo"), "per-day counts carry no tool name either");
  });

  test("a connection's time is counted when it ends", async () => {
    const id = uid();
    const app = await client(mint("client", id));
    await sleep(1200);
    app.socket.close();
    await app.closed;
    await sleep(300);
    const day = (await internal(`/internal/clients/${id}/usage`)).json.find((d) => d.day === today());
    assert.ok(day.connected_ms >= 1000, `connected_ms ${day.connected_ms}`);
  });
});

describe("the protective rate limit", () => {
  test("past the limit, a call is answered as a tool error that says so", async () => {
    const id = uid();
    await client(mint("client", id));
    const answers = [];
    for (let i = 0; i < RATE_LIMIT + 3; i++) answers.push(await callEcho(mint("agent", id)));
    const limited = answers.filter((a) => a.result?.isError);
    assert.ok(limited.length >= 1, "some call was limited");
    assert.ok(answers.every((a) => a.status === 200), "a limited call is a tool error, not an HTTP error");
    assert.match(limited[0].result.content[0].text, /too many tool calls/);
    const other = uid();
    await client(mint("client", other));
    assert.equal((await callEcho(mint("agent", other))).result?.isError, undefined, "another client is not limited");
  });
});
