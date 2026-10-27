// A client that goes silent is presumed gone (SPEC §3.2), on every bridge.
// Run by run-idle.sh against a bridge whose CLIENTMCP_CLIENT_IDLE is short
// and whose call timeout is long, so the two cannot be mistaken for each other.

import { after, afterEach, describe, test } from "node:test";
import assert from "node:assert/strict";
import { CALL_TIMEOUT, WS_URL, closeAll, mint, modernAgent, sleep, text, uid, waitFor } from "./helpers.mjs";

const IDLE = Number(process.env.CLIENTMCP_CLIENT_IDLE ?? 90);
if (IDLE * 2 >= CALL_TIMEOUT) throw new Error("run with a client idle well under the call timeout");

const sockets = [];
afterEach(async () => {
  for (const socket of sockets.splice(0)) socket.close();
  await closeAll();
});
after(() => closeAll());

/** A client that registers one tool and then answers nothing — no calls, no pings. */
async function mute(id) {
  const socket = new WebSocket(WS_URL, ["clientmcp.v1", mint("client", id)]);
  sockets.push(socket);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  const closed = new Promise((resolve) => (socket.onclose = resolve));
  socket.send(
    JSON.stringify({
      type: "register",
      tools: [{ name: "echo", description: "Echo", inputSchema: { type: "object" } }],
    }),
  );
  return { socket, closed };
}

describe("a silent client", () => {
  test("a call to a client gone silent fails at the idle limit, not the call timeout", async () => {
    const id = uid();
    const { closed } = await mute(id);
    const agent = await modernAgent(id);
    await waitFor(async () => (await agent.listTools()).tools.length === 1, 3000, "registered");
    const started = Date.now();
    const result = await agent.callTool({ name: "echo", arguments: {} });
    const elapsed = (Date.now() - started) / 1000;
    assert.equal(result.isError, true);
    assert.match(text(result), /offline|disconnected/);
    assert.ok(elapsed < CALL_TIMEOUT - 2, `failed after ${elapsed}s, the call timeout is ${CALL_TIMEOUT}s`);
    // And the bridge closed it.
    const close = await Promise.race([closed, sleep(5000).then(() => null)]);
    assert.ok(close, "the silent client's socket was not closed");
  });

  test("a silent client's tools are not offered, and a call is told it is offline", async () => {
    const id = uid();
    await mute(id);
    const agent = await modernAgent(id);
    await waitFor(async () => (await agent.listTools()).tools.length === 1, 3000, "registered");
    await sleep((IDLE + 1) * 1000);
    assert.deepEqual((await agent.listTools()).tools, []);
    const result = await agent.callTool({ name: "echo", arguments: {} });
    assert.equal(result.isError, true);
    assert.match(text(result), /offline/);
  });

  test("a client that keeps pinging is kept past the idle limit", async () => {
    const id = uid();
    const { socket } = await mute(id);
    const timer = setInterval(() => socket.send('{"type":"ping"}'), 1000);
    try {
      const agent = await modernAgent(id);
      await sleep((IDLE * 2 + 1) * 1000);
      assert.equal((await agent.listTools()).tools.length, 1);
      assert.equal(socket.readyState, WebSocket.OPEN);
    } finally {
      clearInterval(timer);
    }
  });
});
