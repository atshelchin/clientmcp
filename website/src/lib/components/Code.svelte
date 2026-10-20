<script lang="ts">
	// A code block whose HTML was highlighted at build time.
	let { html, raw, title = '' }: { html: string; raw: string; title?: string } = $props();

	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | undefined;

	async function copy() {
		try {
			await navigator.clipboard.writeText(raw);
			copied = true;
			clearTimeout(timer);
			timer = setTimeout(() => (copied = false), 1600);
		} catch {
			// Clipboard unavailable (insecure context or denied): nothing to do.
		}
	}
</script>

<figure class="block">
	{#if title}
		<figcaption>{title}</figcaption>
	{/if}
	<!-- eslint-disable-next-line svelte/no-at-html-tags -- highlighted at build time from our own snippets -->
	<pre class="code"><code>{@html html}</code></pre>
	<button type="button" class="copy" onclick={copy} aria-label="Copy code">
		{copied ? 'Copied' : 'Copy'}
	</button>
</figure>

<style>
	.block {
		position: relative;
		margin: 0;
		min-width: 0;
	}

	figcaption {
		position: absolute;
		top: 0;
		left: 0;
		padding: 9px 20px 0;
		font-family: var(--font-mono);
		font-size: 11.5px;
		letter-spacing: 0.04em;
		color: var(--code-dim);
	}

	figcaption + .code {
		padding-top: 38px;
	}

	.copy {
		position: absolute;
		top: 8px;
		right: 8px;
		padding: 3px 10px;
		border: 1px solid var(--code-line);
		border-radius: 999px;
		background: var(--code-bg);
		color: var(--code-dim);
		font-family: var(--font-mono);
		font-size: 11.5px;
		cursor: pointer;
		opacity: 0;
		transition:
			opacity 0.15s,
			color 0.15s;
	}

	.block:hover .copy,
	.copy:focus-visible {
		opacity: 1;
	}

	.copy:hover {
		color: var(--code-ink);
	}

	@media (hover: none) {
		.copy {
			opacity: 1;
		}
	}
</style>
