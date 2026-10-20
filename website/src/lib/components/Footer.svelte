<script lang="ts">
	import { resolve } from '$app/paths';
	import { repoFile, site } from '#lib/site.ts';
	import Logo from './Logo.svelte';
</script>

<footer>
	<div class="wrap grid">
		<div class="brand">
			<Logo />
			<p>{site.tagline}</p>
			<p class="meta mono">
				Protocol v{site.specVersion} · MCP {site.mcpRevisions.join(' / ')}
			</p>
		</div>
		<nav aria-label="Documentation">
			<h2>Docs</h2>
			<a href={resolve('/docs/[[page]]', {})}>Guide</a>
			<a href={resolve('/docs/[[page]]', { page: 'sdk' })}>JavaScript SDK</a>
			<a href={resolve('/docs/[[page]]', { page: 'spec' })}>Specification</a>
			<a href={resolve('/docs/[[page]]', { page: 'design' })}>Design notes</a>
		</nav>
		<nav aria-label="Project">
			<h2>Project</h2>
			<a href={site.repo} rel="external noopener">GitHub</a>
			<a href={repoFile('examples/browser/index.html')} rel="external noopener">Browser example</a>
			<a href={repoFile('deploy')} rel="external noopener">Deploy with Caddy</a>
			<a href={repoFile('tests')} rel="external noopener">Conformance suite</a>
		</nav>
	</div>
	<div class="wrap legal">
		<span>MIT or Apache-2.0, at your option.</span>
		<span class="mono">clientmcp.v1</span>
	</div>
</footer>

<style>
	footer {
		border-top: 1px solid var(--line);
		margin-top: 120px;
		padding: 56px 0 32px;
		font-size: 14.5px;
	}

	.grid {
		display: grid;
		grid-template-columns: 1fr;
		gap: 40px;
	}

	@media (min-width: 720px) {
		.grid {
			grid-template-columns: 2fr 1fr 1fr;
		}
	}

	.brand p {
		margin-top: 14px;
		color: var(--ink-2);
		max-width: 36ch;
	}

	.brand .meta {
		font-size: 12px;
		color: var(--ink-3);
	}

	nav {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	h2 {
		font-family: var(--font-mono);
		font-size: 12px;
		font-weight: 500;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--ink-3);
		margin-bottom: 6px;
	}

	nav a {
		color: var(--ink-2);
		text-decoration: none;
		width: fit-content;
	}

	nav a:hover {
		color: var(--ink);
		text-decoration: underline;
		text-underline-offset: 3px;
	}

	.legal {
		display: flex;
		flex-wrap: wrap;
		justify-content: space-between;
		gap: 8px;
		margin-top: 56px;
		padding-top: 20px;
		border-top: 1px solid var(--line);
		color: var(--ink-3);
		font-size: 13px;
	}
</style>
