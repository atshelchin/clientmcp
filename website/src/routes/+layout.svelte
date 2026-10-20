<script lang="ts">
	import '@fontsource-variable/geist';
	import '@fontsource-variable/geist-mono';
	import '@fontsource/instrument-serif/latin-400-italic.css';
	import '../app.css';
	import { page } from '$app/state';
	import favicon from '#lib/assets/favicon.svg';
	import Header from '#lib/components/Header.svelte';
	import Footer from '#lib/components/Footer.svelte';
	import { site } from '#lib/site.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	let url = $derived(new URL(page.url.pathname, site.url).href);
</script>

<svelte:head>
	<link rel="icon" href={favicon} type="image/svg+xml" />
	<link rel="apple-touch-icon" href="/apple-touch-icon.png" />
	<link rel="canonical" href={url} />
	<meta name="theme-color" content="#f4f3ee" media="(prefers-color-scheme: light)" />
	<meta name="theme-color" content="#0f0f0e" media="(prefers-color-scheme: dark)" />
	<meta property="og:type" content="website" />
	<meta property="og:site_name" content={site.name} />
	<meta property="og:url" content={url} />
	<meta property="og:image" content="{site.url}/og.png" />
	<meta property="og:image:width" content="1200" />
	<meta property="og:image:height" content="630" />
	<meta property="og:image:alt" content="ClientMCP: UI for humans. MCP for agents." />
	<meta name="twitter:card" content="summary_large_image" />
</svelte:head>

<a class="skip" href="#main">Skip to content</a>
<Header />
<main id="main">
	{@render children()}
</main>
<Footer />

<style>
	.skip {
		position: absolute;
		left: 12px;
		top: -48px;
		z-index: 100;
		padding: 8px 14px;
		border-radius: 8px;
		background: var(--ink);
		color: var(--bg);
		text-decoration: none;
		transition: top 0.15s;
	}

	.skip:focus {
		top: 10px;
	}
</style>
