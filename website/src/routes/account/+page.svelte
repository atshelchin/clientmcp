<script lang="ts">
	import { resolve } from '$app/paths';
	import AgentConnect from '#lib/components/AgentConnect.svelte';
	import ConnectFlow from '#lib/components/ConnectFlow.svelte';
	import RemoteSecret from '#lib/components/RemoteSecret.svelte';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();

	const MASK = '••••••••••••••••';

	/** A credential, fetched only when the person copies it (account/token). */
	async function credential(client: string, role: 'agent' | 'client', part: 'token' | 'serverUrl') {
		const response = await fetch(resolve('/account/token'), {
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ client, role, csrf: data.csrf })
		});
		if (!response.ok) {
			// The server's own reason ({ message }), or the status if it gave none.
			const reason = await response
				.json()
				.then((body) => (body as { message?: string }).message)
				.catch(() => null);
			throw new Error(reason ?? `The server said ${response.status}. Please reload and try again.`);
		}
		const answer = (await response.json()) as Record<string, string>;
		return answer[part];
	}
</script>

<svelte:head>
	<title>Your account · ClientMCP</title>
	<meta name="robots" content="noindex" />
</svelte:head>

<section class="wrap account">
	<div class="who">
		{#if data.avatar}<img src={data.avatar} alt="" width="40" height="40" />{/if}
		<div>
			<h1>@{data.login}</h1>
			<form method="POST" action="/auth/signout">
				<input type="hidden" name="csrf" value={data.csrf} />
				<button class="linklike" type="submit">Sign out</button>
			</form>
			{#if data.admin}<a href={resolve('/admin')}>Admin</a>{/if}
		</div>
	</div>

	{#if data.suspended}<p class="warn">This account is suspended.</p>{/if}
	{#if form?.done}<p class="note">{form.done}</p>{/if}

	<ConnectFlow />

	<h2>Your clients <span class="count">{data.clients.length} of {data.limit}</span></h2>
	{#if data.clients.length === 0}
		<p>
			No clients yet. Connect an app from inside it, or
			<a href={resolve('/connect')}>create one for your own application</a>.
		</p>
	{/if}
	{#each data.clients as c (c.id)}
		<article class="client" data-client={c.id}>
			<header>
				<strong>{c.label}</strong>
				<span class="meta mono">{c.id}</span>
				<form
					class="delete"
					method="POST"
					action="?/delete"
					onsubmit={(e) => {
						if (
							!confirm(
								`Delete ${c.label}? Its agent and app stop working; you can create a new one any time.`
							)
						)
							e.preventDefault();
					}}
				>
					<input type="hidden" name="csrf" value={data.csrf} /><input
						type="hidden"
						name="client"
						value={c.id}
					/>
					<button class="link danger" type="submit">Delete</button>
				</form>
			</header>
			<p class="meta">
				{c.app ?? 'Your own application'} · today {c.today ? `${c.today.calls} calls` : '—'}
			</p>

			<AgentConnect
				name={c.label}
				mcpUrl={data.mcpUrl}
				serverUrl={() => credential(c.id, 'agent', 'serverUrl')}
				agentToken={() => credential(c.id, 'agent', 'token')}
			/>

			{#if !c.app}
				<p class="how">
					Your application connects with these two (see the <a
						href={resolve('/docs/[[page]]', { page: 'sdk' })}>SDK</a
					>):
				</p>
				<RemoteSecret
					label="Bridge"
					shown={data.bridgeUrl}
					fetchValue={async () => data.bridgeUrl}
					secret={false}
				/>
				<RemoteSecret
					label="Token"
					shown={MASK}
					fetchValue={() => credential(c.id, 'client', 'token')}
				/>
			{/if}
		</article>
	{/each}
	<p><a href={resolve('/connect')}>New client for your own application</a></p>
</section>

<style>
	.account {
		max-width: 760px;
		padding-block: 48px;
	}
	.who {
		display: flex;
		gap: 12px;
		align-items: center;
	}
	.who img {
		border-radius: 50%;
	}
	.who h1 {
		margin: 0;
	}
	.linklike {
		background: none;
		border: 0;
		padding: 0;
		color: inherit;
		text-decoration: underline;
		cursor: pointer;
		font: inherit;
	}
	.count,
	.meta {
		opacity: 0.7;
		font-size: 14px;
	}
	.client {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		padding: 14px 16px;
		margin: 12px 0;
		background: var(--bg-raised);
	}
	.client header {
		display: flex;
		gap: 10px;
		align-items: baseline;
		flex-wrap: wrap;
	}
	.client header .delete {
		margin-left: auto;
	}
	.how {
		font-size: 14px;
		margin: 14px 0 6px;
	}
	.link {
		background: none;
		border: 0;
		padding: 0;
		font: inherit;
		font-size: 13px;
		color: inherit;
		text-decoration: underline;
		cursor: pointer;
	}
	.danger {
		color: var(--human);
	}
	.note {
		padding: 8px 12px;
		border-radius: var(--radius-sm);
		background: var(--agent-soft);
	}
	.warn {
		padding: 8px 12px;
		border-radius: var(--radius-sm);
		background: var(--human-soft);
	}
</style>
