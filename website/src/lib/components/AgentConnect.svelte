<script lang="ts">
	// Pick your agent, copy one thing that works as it is (the URL, the whole
	// command, the whole config) with this client's real address in it. The
	// address is fetched only on Show or Copy.
	import RemoteSecret from './RemoteSecret.svelte';

	let {
		name,
		mcpUrl,
		serverUrl,
		agentToken
	}: {
		/** What the agent calls this server: from the client's label. */
		name: string;
		/** The MCP URL without the token (not a secret). */
		mcpUrl: string;
		/** The MCP URL with the token: fetched on Show or Copy. */
		serverUrl: () => Promise<string>;
		agentToken: () => Promise<string>;
	} = $props();

	const MASK = '••••••••••••';
	let slug = $derived(
		name
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-|-$/g, '') || 'my-app'
	);
	let masked = $derived(`${mcpUrl.replace(/\/$/, '')}/${MASK}`);
	const agents = ['Claude', 'ChatGPT', 'Claude Code', 'Codex', 'Cursor', 'Other'] as const;
	let agent = $state<(typeof agents)[number]>('Claude');
</script>

<div class="tabs" role="tablist" aria-label="Your AI agent">
	{#each agents as a (a)}
		<button
			type="button"
			role="tab"
			aria-selected={agent === a}
			class:on={agent === a}
			onclick={() => (agent = a)}>{a}</button
		>
	{/each}
</div>

{#if agent === 'Claude'}
	<p class="how">Claude → Settings → Connectors → <b>Add custom connector</b> → paste:</p>
	<RemoteSecret shown={masked} fetchValue={serverUrl} />
{:else if agent === 'ChatGPT'}
	<p class="how">
		ChatGPT → Settings → Apps &amp; Connectors → Advanced → turn on <b>Developer mode</b> →
		<b>Create</b> → paste, with no authentication:
	</p>
	<RemoteSecret shown={masked} fetchValue={serverUrl} />
{:else if agent === 'Claude Code'}
	<p class="how">Run in a terminal:</p>
	<RemoteSecret
		shown={`claude mcp add --transport http ${slug} "${masked}"`}
		fetchValue={async () => `claude mcp add --transport http ${slug} "${await serverUrl()}"`}
	/>
{:else if agent === 'Codex'}
	<p class="how">Add to <code>~/.codex/config.toml</code>:</p>
	<RemoteSecret
		shown={`[mcp_servers.${slug}] url = "${masked}"`}
		fetchValue={async () => `[mcp_servers.${slug}]\nurl = "${await serverUrl()}"\n`}
	/>
{:else if agent === 'Cursor'}
	<p class="how">Add inside <code>"mcpServers"</code> in <code>~/.cursor/mcp.json</code>:</p>
	<RemoteSecret
		shown={`"${slug}": { "url": "${masked}" }`}
		fetchValue={async () => `"${slug}": { "url": "${await serverUrl()}" }`}
	/>
{:else}
	<p class="how">Any agent that takes a remote MCP server: paste the URL.</p>
	<RemoteSecret shown={masked} fetchValue={serverUrl} />
	<p class="how">
		Can it send headers? Safer: this address, with the token below as <code
			>Authorization: Bearer</code
		>.
	</p>
	<RemoteSecret shown={mcpUrl} fetchValue={async () => mcpUrl} secret={false} />
	<RemoteSecret shown={MASK} fetchValue={agentToken} />
{/if}

<style>
	.tabs {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin: 12px 0 4px;
	}
	.tabs button {
		font: inherit;
		font-size: 13px;
		padding: 4px 12px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		background: transparent;
		color: inherit;
		cursor: pointer;
	}
	.tabs button.on {
		background: var(--ink);
		color: var(--bg);
		border-color: var(--ink);
	}
	.how {
		font-size: 14px;
		margin: 10px 0 6px;
	}
	code {
		font-family: var(--font-mono);
		font-size: 13px;
	}
</style>
