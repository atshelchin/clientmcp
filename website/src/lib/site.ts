export const site = {
	name: 'ClientMCP',
	url: 'https://clientmcp.dev',
	tagline: 'Expose live client applications as MCP servers.',
	description:
		'ClientMCP exposes a running browser, desktop or mobile application as a standard MCP server. Agents call the same actions as its UI, on the same state, through one outbound WebSocket.',
	repo: 'https://github.com/atshelchin/clientmcp',
	specVersion: '1.0',
	mcpRevisions: ['2026-07-28', '2025-11-25', '2025-06-18', '2025-03-26']
} as const;

/** A file in the repository on GitHub. */
export function repoFile(path: string): string {
	return `${site.repo}/blob/main/${path}`;
}
