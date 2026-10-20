<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
</script>

<svelte:head>
	<title>{page.status} · ClientMCP</title>
</svelte:head>

<section class="wrap">
	<p class="code mono">{page.status}</p>
	<h1>{page.status === 404 ? 'No tools here.' : 'Something went wrong.'}</h1>
	<p class="text">
		{page.status === 404
			? 'This page does not exist, or it went offline with the app that declared it.'
			: page.error?.message}
	</p>
	<div class="links">
		<a class="btn primary" href={resolve('/')}>Home</a>
		<a class="btn" href={resolve('/docs/[[page]]', {})}>Docs</a>
	</div>
</section>

<style>
	section {
		padding-top: 120px;
		padding-bottom: 40px;
	}

	.code {
		font-size: 13px;
		color: var(--ink-3);
		letter-spacing: 0.06em;
	}

	h1 {
		margin-top: 14px;
		font-size: clamp(36px, 6vw, 60px);
		letter-spacing: -0.04em;
	}

	.text {
		margin-top: 16px;
		color: var(--ink-2);
		font-size: 18px;
		max-width: 50ch;
	}

	.links {
		display: flex;
		gap: 10px;
		margin-top: 28px;
	}
</style>
