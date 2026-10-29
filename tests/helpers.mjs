// Shared helpers for the conformance suite. The bridge under test is given by
// BRIDGE_URL and CLIENTMCP_SECRET (and optionally CLIENTMCP_CALL_TIMEOUT).

import { createHmac, randomUUID } from "node:crypto";
import { Client as LegacyClient } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport as LegacyTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { Client as ModernClient, StreamableHTTPClientTransport as ModernTransport } from "@modelcontextprotocol/client";
import { ClientMCP } from "../sdk/js/dist/index.js";

// The application: the JS SDK, or the Rust client with CLIENTMCP_APP=rust.
const App = process.env.CLIENTMCP_APP === "rust" ? (await import("./rust-app.mjs")).RustApp : ClientMCP;

export const BRIDGE = (process.env.BRIDGE_URL ?? "http://127.0.0.1:8080").replace(/\/$/, "");
export const SECRET = (process.env.CLIENTMCP_SECRET ?? "").trim().split(/\s+/)[0];
export const CALL_TIMEOUT = Number(process.env.CLIENTMCP_CALL_TIMEOUT ?? 120);
export const ALLOWED_ORIGIN = process.env.TEST_ALLOWED_ORIGIN; // must be in CLIENTMCP_ALLOWED_ORIGINS
export const WS_URL = BRIDGE.replace(/^http/, "ws") + "/connect";
export const MCP_URL = BRIDGE + "/mcp";
export const MODERN = "2026-07-28";

if (!SECRET) throw new Error("set CLIENTMCP_SECRET to the bridge's secret");

const b64url = (data) => Buffer.from(data).toString("base64url");

export function mint(role, sub, claims = {}) {
  const input = `${b64url(JSON.stringify({ alg: "HS256", typ: "JWT" }))}.${b64url(JSON.stringify({ sub, role, ...claims }))}`;
  return `${input}.${createHmac("sha256", SECRET).update(input).digest("base64url")}`;
}

export const uid = () => `t-${randomUUID().slice(0, 13)}`;
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function waitFor(predicate, timeout = 5000, what = "condition") {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await sleep(20);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** Everything a test opened, closed after the test. */
const opened = [];
export async function closeAll() {
  for (const resource of opened.splice(0)) {
    try {
      await resource.close();
    } catch {}
  }
}

/** Starts an application exposing `tools`, and waits until it is online. */
export async function startApp(id, tools = {}, options = {}) {
  const statuses = [];
  const app = new App({
    url: WS_URL,
    token: mint("client", id),
    onStatus: (status, detail) => statuses.push({ status, ...detail }),
    ...options,
  });
  app.statuses = statuses;
  for (const [name, tool] of Object.entries(tools)) app.expose(name, tool);
  opened.push(app);
  await app.connect();
  return app;
}

const agentHeaders = (id) => ({ Authorization: `Bearer ${mint("agent", id)}` });

/** An agent speaking the legacy (initialize-based) protocol, via MCP SDK v1. */
export async function legacyAgent(id, { onToolsChanged } = {}) {
  const client = new LegacyClient({ name: "conformance-legacy", version: "1.0.0" });
  if (onToolsChanged) {
    const { ToolListChangedNotificationSchema } = await import("@modelcontextprotocol/sdk/types.js");
    client.setNotificationHandler(ToolListChangedNotificationSchema, onToolsChanged);
  }
  const transport = new LegacyTransport(new URL(MCP_URL), { requestInit: { headers: agentHeaders(id) } });
  await client.connect(transport);
  opened.push(client);
  return client;
}

/** An agent speaking MCP 2026-07-28, via MCP SDK v2. */
export async function modernAgent(id, { mode = { pin: MODERN } } = {}) {
  const client = new ModernClient({ name: "conformance-modern", version: "1.0.0" }, { versionNegotiation: { mode } });
  const transport = new ModernTransport(new URL(MCP_URL), { requestInit: { headers: agentHeaders(id) } });
  await client.connect(transport);
  opened.push(client);
  return client;
}

/** POSTs a raw JSON-RPC body to the agent endpoint. */
export async function rpc(id, body, headers = {}) {
  const response = await fetch(MCP_URL, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...agentHeaders(id), ...headers },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
  const text = await response.text();
  let json;
  try {
    json = JSON.parse(text);
  } catch {}
  return { status: response.status, headers: response.headers, text, json };
}

/** A modern request body and its mirrored headers. */
export function modern(method, params = {}, id = 1) {
  const body = {
    jsonrpc: "2.0",
    id,
    method,
    params: {
      ...params,
      _meta: { "io.modelcontextprotocol/protocolVersion": MODERN, "io.modelcontextprotocol/clientCapabilities": {} },
    },
  };
  const headers = { "MCP-Protocol-Version": MODERN, "Mcp-Method": method };
  if (params.name) headers["Mcp-Name"] = params.name;
  return { body, headers };
}

/** Opens an SSE stream and collects its events. */
export async function openStream(url, init) {
  const controller = new AbortController();
  const response = await fetch(url, { ...init, signal: controller.signal });
  const events = [];
  let comments = 0;
  const done = (async () => {
    const decoder = new TextDecoder();
    let buffer = "";
    try {
      for await (const chunk of response.body) {
        buffer += decoder.decode(chunk, { stream: true });
        let end;
        while ((end = buffer.indexOf("\n\n")) >= 0) {
          const block = buffer.slice(0, end);
          buffer = buffer.slice(end + 2);
          const data = block.split("\n").filter((l) => l.startsWith("data: ")).map((l) => l.slice(6)).join("\n");
          if (data) events.push(JSON.parse(data));
          else comments++;
        }
      }
    } catch {}
  })();
  const stream = { response, events, get comments() { return comments; }, close: () => controller.abort(), done };
  opened.push(stream);
  return stream;
}

export const text = (result) => result.content.map((c) => c.text).join("");
