// Code shown on the landing page, taken from README.md and sdk/js/README.md.
// Highlighted at build time by src/routes/+page.server.ts.

export type Snippet = { lang: string; code: string };

export const idea: Snippet = {
	lang: 'ts',
	code: `// One implementation of each action…
const addTodo = (title: string) => todos.add(title);

// …called by the UI, for people,
form.onsubmit = () => addTodo(input.value);

// …and by agents, through ClientMCP.
mcp.expose("add_todo", {
  description: "Add a todo",
  inputSchema: {
    type: "object",
    properties: { title: { type: "string" } },
    required: ["title"],
  },
  handler: ({ title }) => addTodo(title),
});`
};

export const quickstart: { title: string; text: string; snippet: Snippet }[] = [
	{
		title: 'Run a bridge',
		text: 'A static binary in a scratch image. It stores nothing; the secret signs every token.',
		snippet: {
			lang: 'sh',
			code: `git clone https://github.com/atshelchin/clientmcp && cd clientmcp
docker build -t clientmcp .
export CLIENTMCP_SECRET=$(openssl rand -hex 32)
docker run -d -p 8080:8080 -e CLIENTMCP_SECRET clientmcp`
		}
	},
	{
		title: 'Mint tokens',
		text: 'A client token lets the application connect as my-app; an agent token lets an agent call it.',
		snippet: {
			lang: 'sh',
			code: `docker run --rm -e CLIENTMCP_SECRET clientmcp token client my-app
docker run --rm -e CLIENTMCP_SECRET clientmcp token agent my-app --ttl 30d`
		}
	},
	{
		title: 'Expose functions from the application',
		text: 'The SDK in sdk/js has no dependencies and runs in browsers, Node.js 22+, Deno and Bun.',
		snippet: {
			lang: 'ts',
			code: `import { ClientMCP } from "clientmcp";

const mcp = new ClientMCP({ url: "ws://localhost:8080/connect", token: CLIENT_TOKEN });
mcp.expose("add_todo", {
  description: "Add a todo",
  inputSchema: { type: "object", properties: { title: { type: "string" } }, required: ["title"] },
  handler: ({ title }) => todos.add(title), // the same action the UI calls
});
await mcp.connect();`
		}
	},
	{
		title: 'Connect an agent',
		text: 'Any MCP client works. Hosts that cannot send headers can put the token in the URL: /mcp/<agent token>.',
		snippet: {
			lang: 'sh',
			code: `claude mcp add --transport http my-app http://localhost:8080/mcp \\
  --header "Authorization: Bearer $AGENT_TOKEN"`
		}
	}
];

export const authority: Snippet = {
	lang: 'ts',
	code: `import { completed, rejected } from "clientmcp";

mcp.expose("clear_done", {
  description: "Remove completed todos",
  risk: "manage", // observe | operate | manage | transact
  handler: async () => {
    if (!(await askTheUser("Remove completed items?")))
      return rejected("denied", "the user said no");
    todos.clearDone();
    return completed(); // the change holds — the app has seen it
  },
});`
};

export const docker: Snippet = {
	lang: 'sh',
	code: `docker run -d -p 8080:8080 -e CLIENTMCP_SECRET clientmcp
# or, without Docker
cargo build --release -p clientmcp-server`
};

export const workers: Snippet = {
	lang: 'sh',
	code: `cd crates/worker
npx wrangler secret put CLIENTMCP_SECRET
npx wrangler deploy`
};

/** The client channel, SPEC §3.2. */
export const messages: {
	dir: 'up' | 'down';
	type: string;
	meaning: string;
	example: string;
}[] = [
	{
		dir: 'up',
		type: 'register',
		meaning: 'Declare the complete tool set',
		example: '{"type":"register","tools":[{"name":"add_todo","inputSchema":{…}}]}'
	},
	{
		dir: 'down',
		type: 'registered',
		meaning: 'Token and tools accepted',
		example: '{"type":"registered"}'
	},
	{
		dir: 'down',
		type: 'call',
		meaning: 'Execute a tool',
		example: '{"type":"call","id":"8f3a","name":"add_todo","arguments":{"title":"Oat milk"}}'
	},
	{
		dir: 'up',
		type: 'result',
		meaning: 'Finish a call with a CallToolResult',
		example: '{"type":"result","id":"8f3a","result":{"content":[…]}}'
	},
	{
		dir: 'down',
		type: 'cancel',
		meaning: 'Stop a call; its result is not wanted',
		example: '{"type":"cancel","id":"8f3a"}'
	},
	{
		dir: 'up',
		type: 'ping',
		meaning: 'Liveness, every 25 s when idle',
		example: '{"type":"ping"}'
	},
	{
		dir: 'down',
		type: 'pong',
		meaning: 'Answered at the edge on Workers',
		example: '{"type":"pong"}'
	}
];
