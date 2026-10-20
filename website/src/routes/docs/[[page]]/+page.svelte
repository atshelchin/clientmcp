<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { site } from '#lib/site.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const href = (slug: string) => resolve('/docs/[[page]]', slug ? { page: slug } : {});

	let article: HTMLElement;
	let active = $state('');
	let tocOpen = $state(false);

	// Scroll spy: the active section is the last heading above the top third.
	$effect(() => {
		void data.html;
		const headings = Array.from(article.querySelectorAll<HTMLElement>('h2[id], h3[id]'));
		const update = () => {
			const line = window.innerHeight / 3;
			let current = '';
			for (const h of headings) {
				if (h.getBoundingClientRect().top <= line) current = h.id;
				else break;
			}
			active = current;
		};
		update();
		addEventListener('scroll', update, { passive: true });
		return () => removeEventListener('scroll', update);
	});

	$effect(() => {
		void page.url.pathname;
		tocOpen = false;
	});

	const sectionOf = (id: string) => {
		const index = data.toc.findIndex((entry) => entry.id === id);
		for (let i = index; i >= 0; i--) if (data.toc[i].depth === 2) return data.toc[i].id;
		return '';
	};
	let activeSection = $derived(sectionOf(active));
</script>

<svelte:head>
	<title>{data.label} · ClientMCP</title>
	<meta name="description" content={data.description} />
	<meta property="og:title" content="{data.label} · ClientMCP" />
	<meta property="og:description" content={data.description} />
</svelte:head>

<div class="wrap docs">
	<aside class="side">
		<nav class="pages" aria-label="Documentation">
			{#each data.nav as item (item.slug)}
				<a
					href={href(item.slug)}
					aria-current={page.url.pathname === href(item.slug) ? 'page' : undefined}>{item.label}</a
				>
			{/each}
		</nav>

		{#if data.toc.length}
			{#snippet entries(close: boolean)}
				<ol>
					{#each data.toc as entry (entry.id)}
						<li class="d{entry.depth}">
							<a
								href="#{entry.id}"
								class:active={entry.id === active || entry.id === activeSection}
								onclick={() => close && (tocOpen = false)}>{entry.text}</a
							>
						</li>
					{/each}
				</ol>
			{/snippet}
			<details class="toc compact" bind:open={tocOpen}>
				<summary>On this page</summary>
				{@render entries(true)}
			</details>
			<nav class="toc full" aria-label="On this page">
				<p>On this page</p>
				{@render entries(false)}
			</nav>
		{/if}
	</aside>

	<article bind:this={article}>
		<header class="doc-head">
			<p class="label"><span class="num">Docs</span> {data.label}</p>
			<h1>{data.title}</h1>
		</header>

		<!-- eslint-disable-next-line svelte/no-at-html-tags -- the repository's own Markdown, rendered at build time -->
		<div class="prose">{@html data.html}</div>

		<footer class="doc-foot">
			<nav class="pager" aria-label="Previous and next">
				{#if data.prev}
					<a class="prev" href={href(data.prev.slug)}><small>Previous</small>{data.prev.label}</a>
				{/if}
				{#if data.next}
					<a class="next" href={href(data.next.slug)}><small>Next</small>{data.next.label}</a>
				{/if}
			</nav>
			<p class="source mono">
				Rendered from <a href={data.source} rel="external noopener">{data.path}</a> in
				<a href={site.repo} rel="external noopener">atshelchin/clientmcp</a>.
			</p>
		</footer>
	</article>
</div>

<style>
	.docs {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 0 64px;
		padding-top: 20px;
	}

	@media (min-width: 960px) {
		.docs {
			grid-template-columns: 232px minmax(0, 1fr);
			padding-top: 56px;
		}
	}

	/* Sidebar */

	.side {
		min-width: 0;
	}

	@media (min-width: 960px) {
		.side {
			position: sticky;
			top: calc(var(--header-h) + 32px);
			align-self: start;
			max-height: calc(100vh - var(--header-h) - 48px);
			overflow-y: auto;
			padding-bottom: 24px;
			scrollbar-width: thin;
		}
	}

	.pages {
		display: flex;
		gap: 6px;
		margin: 0 calc(-1 * var(--gutter));
		padding: 0 var(--gutter) 14px;
		overflow-x: auto;
		scrollbar-width: none;
	}

	.pages::-webkit-scrollbar {
		display: none;
	}

	.pages a {
		flex: none;
		padding: 6px 13px;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 14px;
		color: var(--ink-2);
		text-decoration: none;
	}

	.pages a[aria-current='page'] {
		color: var(--bg);
		background: var(--ink);
		border-color: var(--ink);
	}

	@media (min-width: 960px) {
		.pages {
			flex-direction: column;
			gap: 2px;
			margin: 0;
			padding: 0;
			overflow: visible;
		}

		.pages a {
			padding: 7px 12px;
			border: 0;
			border-radius: 8px;
			font-size: 14.5px;
		}

		.pages a:hover {
			color: var(--ink);
			background: var(--bg-sunken);
		}

		.pages a[aria-current='page'] {
			color: var(--ink);
			background: var(--bg-sunken);
			font-weight: 600;
		}
	}

	.toc ol {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.toc a {
		display: block;
		font-size: 13.5px;
		line-height: 1.4;
		color: var(--ink-2);
		text-decoration: none;
	}

	.toc .d3 a {
		font-size: 13px;
		color: var(--ink-3);
	}

	.toc a:hover,
	.toc a.active {
		color: var(--ink);
	}

	.toc a.active {
		font-weight: 500;
	}

	/* Mobile: a collapsible list. */
	.compact {
		border: 1px solid var(--line);
		border-radius: 10px;
		background: var(--bg-raised);
	}

	.compact summary {
		padding: 10px 14px;
		font-family: var(--font-mono);
		font-size: 12px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--ink-3);
		cursor: pointer;
	}

	.compact ol {
		padding: 0 6px 10px;
	}

	.compact a {
		padding: 5px 8px;
	}

	.compact .d3 a {
		padding-left: 22px;
	}

	/* Desktop: always open, under the pages. */
	.full {
		display: none;
		margin-top: 28px;
		padding-top: 18px;
		border-top: 1px solid var(--line);
	}

	.full p {
		padding: 0 12px 8px;
		font-family: var(--font-mono);
		font-size: 12px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--ink-3);
	}

	.full a {
		padding: 4px 12px;
		border-left: 1.5px solid transparent;
	}

	.full .d3 a {
		padding-left: 24px;
	}

	.full a.active {
		border-left-color: var(--human);
	}

	@media (min-width: 960px) {
		.compact {
			display: none;
		}

		.full {
			display: block;
		}
	}

	/* Article */

	article {
		min-width: 0;
		max-width: 780px;
		padding-top: 28px;
	}

	@media (min-width: 960px) {
		article {
			padding-top: 0;
		}
	}

	.doc-head h1 {
		margin-top: 14px;
		font-size: clamp(34px, 5vw, 48px);
		letter-spacing: -0.035em;
	}

	.doc-foot {
		margin-top: 72px;
		padding-top: 28px;
		border-top: 1px solid var(--line);
	}

	.pager {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 12px;
	}

	.pager a {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding: 14px 18px;
		border: 1px solid var(--line);
		border-radius: 10px;
		text-decoration: none;
		font-weight: 600;
		transition: border-color 0.15s;
	}

	.pager a:hover {
		border-color: var(--ink-3);
	}

	.pager small {
		font-family: var(--font-mono);
		font-weight: 400;
		font-size: 11.5px;
		color: var(--ink-3);
	}

	.pager .next {
		grid-column: 2;
		text-align: right;
	}

	.source {
		margin-top: 24px;
		font-size: 12px;
		color: var(--ink-3);
	}

	.source a {
		color: var(--ink-2);
	}

	/* Rendered Markdown */

	.prose {
		margin-top: 28px;
		font-size: 16.5px;
		line-height: 1.7;
		color: var(--ink-2);
	}

	.prose :global(> * + *) {
		margin-top: 18px;
	}

	.prose :global(:is(h2, h3, h4)) {
		color: var(--ink);
		scroll-margin-top: calc(var(--header-h) + 20px);
	}

	.prose :global(h2) {
		margin-top: 64px;
		padding-top: 28px;
		border-top: 1px solid var(--line);
		font-size: 28px;
		letter-spacing: -0.03em;
	}

	.prose :global(h3) {
		margin-top: 44px;
		font-size: 21px;
		letter-spacing: -0.02em;
	}

	.prose :global(h4) {
		margin-top: 32px;
		font-size: 17px;
	}

	.prose :global(:is(h2, h3, h4) + *) {
		margin-top: 14px;
	}

	.prose :global(.anchor) {
		color: inherit;
		text-decoration: none;
	}

	.prose :global(.anchor:hover::after) {
		content: '#';
		margin-left: 10px;
		color: var(--ink-3);
		font-weight: 400;
	}

	.prose :global(p strong),
	.prose :global(li strong) {
		color: var(--ink);
		font-weight: 600;
	}

	.prose :global(a:not(.anchor)) {
		color: var(--ink);
		text-decoration-color: var(--line-strong);
		text-underline-offset: 3px;
	}

	.prose :global(a:not(.anchor):hover) {
		text-decoration-color: var(--ink);
	}

	.prose :global(:is(ul, ol)) {
		padding-left: 22px;
	}

	.prose :global(li + li) {
		margin-top: 6px;
	}

	.prose :global(li::marker) {
		color: var(--ink-3);
	}

	.prose :global(:not(pre) > code) {
		padding: 1.5px 5px;
		border-radius: 5px;
		background: var(--bg-sunken);
		color: var(--ink);
		font-size: 0.84em;
		overflow-wrap: anywhere;
	}

	.prose :global(pre.code) {
		position: relative;
		font-size: 13px;
	}

	.prose :global(pre.code[data-lang]::before) {
		content: attr(data-lang);
		position: absolute;
		top: 6px;
		right: 12px;
		font-size: 10.5px;
		color: var(--code-dim);
	}

	.prose :global(blockquote) {
		margin-left: 0;
		margin-right: 0;
		padding: 2px 0 2px 18px;
		border-left: 2px solid var(--human);
		color: var(--ink);
	}

	.prose :global(hr) {
		border: 0;
		border-top: 1px solid var(--line);
		margin: 40px 0;
	}

	/* A section heading already draws its own rule. */
	.prose :global(hr:has(+ h2)) {
		display: none;
	}

	.prose :global(.table-wrap) {
		overflow-x: auto;
		border: 1px solid var(--line);
		border-radius: 10px;
		-webkit-overflow-scrolling: touch;
	}

	.prose :global(table) {
		width: 100%;
		border-collapse: collapse;
		font-size: 14.5px;
		line-height: 1.5;
	}

	.prose :global(th) {
		text-align: left;
		font-weight: 600;
		color: var(--ink);
		background: var(--bg-sunken);
		white-space: nowrap;
	}

	.prose :global(:is(th, td)) {
		padding: 9px 14px;
		border-bottom: 1px solid var(--line);
		vertical-align: top;
	}

	.prose :global(tr:last-child td) {
		border-bottom: 0;
	}

	.prose :global(td code) {
		white-space: nowrap;
	}
</style>
