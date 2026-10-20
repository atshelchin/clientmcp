// Scratch CLI agent for the Vela integration demo (not part of the suite):
//   node vela-agent.mjs tools | node vela-agent.mjs call <tool> [jsonArgs]
// env: AGENT_TOKEN_FILE, MCP_URL (default http://127.0.0.1:18700/mcp), MAX
import { readFileSync } from 'node:fs';
import { Client, StreamableHTTPClientTransport } from '@modelcontextprotocol/client';
const token = readFileSync(process.env.AGENT_TOKEN_FILE, 'utf8').trim();
const client = new Client({ name: 'vela-demo-agent', version: '1.0.0' }, { versionNegotiation: { mode: { pin: '2026-07-28' } } });
await client.connect(new StreamableHTTPClientTransport(new URL(process.env.MCP_URL ?? 'http://127.0.0.1:18700/mcp'), { requestInit: { headers: { Authorization: `Bearer ${token}` } } }));
const [cmd, name, args] = process.argv.slice(2);
let out;
if (cmd === 'tools') {
  const { tools } = await client.listTools();
  out = tools.map((t) => `${t.name}  [${t._meta?.['io.github.clientmcp/risk'] ?? '-'}]  ${JSON.stringify(t.annotations ?? {})}`).join('\n');
} else {
  const result = await client.callTool({ name, arguments: args ? JSON.parse(args) : {} });
  out = (result.isError ? 'REJECTED ' : '') + JSON.stringify(result.structuredContent ?? result.content, null, 1);
}
console.log(String(out).slice(0, Number(process.env.MAX ?? 8000)));
await client.close();
