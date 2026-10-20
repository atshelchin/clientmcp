<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { site } from '#lib/site.ts';
	import Logo from './Logo.svelte';
	import GitHubIcon from './GitHubIcon.svelte';

	const links = [
		{ href: resolve('/docs/[[page]]', {}), label: 'Docs' },
		{ href: resolve('/docs/[[page]]', { page: 'sdk' }), label: 'SDK', wide: true },
		{ href: resolve('/docs/[[page]]', { page: 'spec' }), label: 'Spec' }
	];
</script>

<header>
	<div class="wrap bar">
		<a class="home" href={resolve('/')} aria-label="ClientMCP home"><Logo /></a>
		<nav aria-label="Main">
			{#each links as link (link.href)}
				<a
					href={link.href}
					class:wide={link.wide}
					aria-current={page.url.pathname === link.href ? 'page' : undefined}>{link.label}</a
				>
			{/each}
			<a class="gh" href={site.repo} rel="external noopener" aria-label="ClientMCP on GitHub">
				<GitHubIcon />
				<span class="gh-label">GitHub</span>
			</a>
		</nav>
	</div>
</header>

<style>
	header {
		position: sticky;
		top: 0;
		z-index: 50;
		height: var(--header-h);
		background: color-mix(in oklab, var(--bg) 82%, transparent);
		backdrop-filter: saturate(1.4) blur(12px);
		-webkit-backdrop-filter: saturate(1.4) blur(12px);
		border-bottom: 1px solid var(--line);
	}

	.bar {
		height: 100%;
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
	}

	.home {
		text-decoration: none;
		display: flex;
	}

	nav {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	nav a {
		display: inline-flex;
		align-items: center;
		gap: 7px;
		height: 36px;
		padding: 0 12px;
		border-radius: 999px;
		font-size: 14.5px;
		color: var(--ink-2);
		text-decoration: none;
		transition:
			color 0.15s,
			background 0.15s;
	}

	nav a:hover {
		color: var(--ink);
		background: var(--bg-sunken);
	}

	nav a[aria-current='page'] {
		color: var(--ink);
	}

	.gh {
		margin-left: 6px;
		border: 1px solid var(--line-strong);
	}

	@media (max-width: 480px) {
		nav a {
			padding: 0 10px;
		}

		.wide,
		.gh-label {
			display: none;
		}

		.gh {
			margin-left: 2px;
		}
	}
</style>
