// The application side of the suite played by the Rust client (crates/client)
// instead of the JS SDK: CLIENTMCP_APP=rust. It has the SDK's surface; the
// tools' handlers run here, and the Rust process does all of the protocol
// (crates/client/examples/conformance_app.rs).

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const BINARY = fileURLToPath(new URL("../target/debug/examples/conformance_app", import.meta.url));

// The SDK marks a complete CallToolResult (toolResult, the outcomes) with a
// symbol it does not export.
const isToolResult = (value) =>
  Object.getPrototypeOf(value) === null && Object.getOwnPropertySymbols(value).some((s) => s.description === "clientmcp.result");

function reply(value) {
  if (value === undefined || value === null) return { kind: "empty" };
  if (typeof value === "string") return { kind: "text", value };
  if (typeof value === "object" && isToolResult(value)) return { kind: "result", value: { ...value } };
  return { kind: "json", value };
}

export class RustApp {
  #child;
  #handlers = new Map();
  #calls = new Map();
  #connects = [];
  #status = "offline";
  #ended = false;
  #onStatus;

  constructor({ url, token, instructions, onStatus }) {
    if (typeof token !== "string") throw new TypeError("the Rust app takes a fixed token here");
    this.#onStatus = onStatus;
    this.#child = spawn(BINARY, [], { stdio: ["pipe", "pipe", "inherit"] });
    this.#child.stdin.on("error", () => {});
    createInterface({ input: this.#child.stdout, crlfDelay: Infinity }).on("line", (line) => this.#event(JSON.parse(line)));
    this.#send({ op: "options", url, token, instructions });
  }

  get status() {
    return this.#status;
  }

  expose(name, { handler, ...definition }) {
    this.#handlers.set(name, handler);
    this.#send({ op: "expose", name, tool: definition });
  }

  unexpose(name) {
    this.#handlers.delete(name);
    this.#send({ op: "unexpose", name });
  }

  set instructions(text) {
    this.#send({ op: "instructions", text: text ?? null });
  }

  connect() {
    return new Promise((resolve, reject) => {
      this.#connects.push({ resolve, reject });
      this.#send({ op: "connect" });
    });
  }

  close() {
    this.#send({ op: "close" });
    if (!this.#ended) this.#child.stdin.end();
    this.#ended = true;
  }

  #send(message) {
    if (!this.#ended) this.#child.stdin.write(JSON.stringify(message) + "\n");
  }

  #event(event) {
    if (event.ev === "status") {
      this.#status = event.status;
      this.#onStatus?.(event.status, event.code === undefined ? undefined : { code: event.code, reason: event.reason });
    } else if (event.ev === "connected") {
      const waiter = this.#connects.shift();
      if (event.ok) waiter?.resolve();
      else waiter?.reject(new Error(event.message));
    } else if (event.ev === "call") {
      void this.#run(event);
    } else if (event.ev === "cancelled") {
      this.#calls.get(event.call)?.abort(new Error("cancelled by the agent"));
      this.#calls.delete(event.call);
    }
  }

  async #run({ call, name, args }) {
    const controller = new AbortController();
    this.#calls.set(call, controller);
    let answer;
    try {
      answer = reply(await this.#handlers.get(name)(args, { name, signal: controller.signal }));
    } catch (error) {
      answer = { kind: "error", value: error instanceof Error ? error.message : String(error) };
    }
    if (this.#calls.delete(call)) this.#send({ op: "reply", call, reply: answer });
  }
}
