<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import AgentConnect from '#lib/components/AgentConnect.svelte';
	import ConnectFlow from '#lib/components/ConnectFlow.svelte';
	import Secret from '#lib/components/Secret.svelte';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();

	// The app's request travels with the form: `?/approve` alone would drop
	// it. Kit finds the action by its `/approve` key wherever it stands.
	let approveAction = $derived(`${page.url.search}&/approve`);

	// Back to the app as soon as the code exists; the button is there for a
	// browser that does not follow, and the agent's credentials stay on the page.
	onMount(() => {
		if (form?.done) window.location.assign(form.done.returnTo);
	});
</script>

<svelte:head>
	<title>Connect · ClientMCP</title>
	<meta name="robots" content="noindex" />
</svelte:head>

<section class="wrap connect">
	{#if form?.limit}
		<h1>You have reached your limit</h1>
		<p>
			This account can have {form.limit} clients. Delete one on
			<a href={resolve('/account')}>your account page</a>, then try again.
		</p>
	{:else if form?.done}
		<h1>Connected</h1>
		<p>Going back to {form.done.appName}… If nothing happens:</p>
		<!-- The app's own address (a registered redirect URI), not a route of this site. -->
		<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
		<p><a class="btn" href={form.done.returnTo}>Return to {form.done.appName}</a></p>
		<h2>Now connect your AI agent</h2>
		<ConnectFlow appName={form.done.appName} />
		<AgentConnect
			name={form.done.appName}
			mcpUrl={form.done.mcpUrl}
			serverUrl={async () => `${form.done!.mcpUrl.replace(/\/$/, '')}/${form.done!.agentToken}`}
			agentToken={async () => form.done!.agentToken}
		/>
		<p class="meta">
			Everything here is also on <a href={resolve('/account')}>your account page</a>.
		</p>
	{:else if form?.created}
		<h1>{form.created.label} is ready</h1>
		<h2>For your application</h2>
		<Secret label="Bridge URL" value={form.created.bridgeUrl} />
		<Secret
			label="Client token"
			value={form.created.clientToken}
			secret
			hint="Your application connects with this."
		/>
		<h2>For your AI agent</h2>
		<ConnectFlow appName={form.created.label} />
		<AgentConnect
			name={form.created.label}
			mcpUrl={form.created.mcpUrl}
			serverUrl={async () =>
				`${form.created!.mcpUrl.replace(/\/$/, '')}/${form.created!.agentToken}`}
			agentToken={async () => form.created!.agentToken}
		/>
		<p class="meta">
			Everything here is also on <a href={resolve('/account')}>your account page</a>.
		</p>
	{:else if data.app}
		<h1>Connect {data.app.name}</h1>
		<p>
			Let <strong>{data.app.name}</strong> on <strong>{data.app.label}</strong> connect to your AI
			agent through ClientMCP? You are signed in as <strong>@{data.login}</strong>.
		</p>
		<p>
			{data.app.name} will get a token to connect. Your agent gets its own, shown on the next page. You
			can revoke either at any time.
		</p>
		<form method="POST" action={approveAction}>
			<input type="hidden" name="csrf" value={data.csrf} />
			{#if data.choices.length}
				<fieldset class="choices">
					<legend>Connect as</legend>
					{#each data.choices as c (c.id)}
						<label class="choice">
							<input type="radio" name="client" value={c.id} checked={c.here} />
							<span>
								<strong>{c.label}</strong>
								<span class="mono meta">{c.id}</span><br />
								<span class="meta"
									>{c.here
										? `This ${data.app.name} — keeps its address for your agent`
										: c.held
											? `${c.app ?? 'Your own application'} · in use there now; it will be disconnected`
											: (c.app ?? 'Your own application')}</span
								>
							</span>
						</label>
					{/each}
					<label class="choice">
						<input
							type="radio"
							name="client"
							value="new"
							checked={!data.choices.some((c) => c.here)}
						/>
						<span
							><strong>A new client</strong><br /><span class="meta"
								>With a new address for your agent</span
							></span
						>
					</label>
				</fieldset>
			{/if}
			<button class="btn" type="submit">Connect</button>
		</form>
	{:else}
		<h1>New client</h1>
		<p>Signed in as <strong>@{data.login}</strong>. Name the application you are connecting:</p>
		<form method="POST" action="?/create">
			<input type="hidden" name="csrf" value={data.csrf} />
			<label>Name <input name="label" maxlength="40" placeholder="My app" /></label>
			<button class="btn" type="submit">Create</button>
		</form>
	{/if}
</section>

<style>
	.connect {
		max-width: 680px;
		padding-block: 48px;
	}
	h2 {
		margin-top: 32px;
	}
	.meta {
		font-size: 14px;
		opacity: 0.75;
	}
	.choices {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		padding: 8px 16px;
		margin: 20px 0;
	}
	.choices legend {
		padding: 0 6px;
		font-size: 14px;
		opacity: 0.75;
	}
	.choice {
		align-items: flex-start;
		cursor: pointer;
	}
	.choice input {
		margin-top: 4px;
	}
	.mono {
		font-family: var(--font-mono);
	}
	label {
		display: flex;
		gap: 8px;
		align-items: center;
		margin-block: 12px;
	}
	input {
		font: inherit;
		padding: 6px 10px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		color: var(--ink);
	}
</style>
