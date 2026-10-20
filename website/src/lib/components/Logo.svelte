<script lang="ts">
	// The mark: an application (the ring) holding its state (the dot), with its
	// one outbound connection leaving through the opening.
	let { size = 24, wordmark = true }: { size?: number; wordmark?: boolean } = $props();
</script>

<span class="logo">
	<svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true">
		<path
			d="M21.07 23.07A10 10 0 1 1 21.07 8.93"
			stroke="currentColor"
			stroke-width="3"
			stroke-linecap="round"
		/>
		<path d="M14 16H29" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
		<circle cx="14" cy="16" r="3.6" fill="var(--human)" />
	</svg>
	{#if wordmark}<span class="word">ClientMCP</span>{/if}
</span>

<style>
	.logo {
		display: inline-flex;
		align-items: center;
		gap: 9px;
		color: var(--ink);
	}

	.word {
		font-weight: 600;
		font-size: 17px;
		letter-spacing: -0.02em;
	}
</style>
