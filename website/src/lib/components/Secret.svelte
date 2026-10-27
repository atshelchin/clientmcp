<script lang="ts">
	// A value to copy. A secret one is shown masked: the person copies it,
	// they do not need to read it (and a screen can be seen or captured).
	let {
		label,
		value,
		hint,
		secret = false
	}: { label: string; value: string; hint?: string; secret?: boolean } = $props();
	let copied = $state(false);
	let revealed = $state(false);

	async function copy() {
		await navigator.clipboard.writeText(value);
		copied = true;
		setTimeout(() => (copied = false), 1500);
	}
</script>

<div class="secret">
	<span class="label">{label}</span>
	<code class="mono value" data-testid={label} data-value={value}
		>{secret && !revealed ? '•'.repeat(24) : value}</code
	>
	<span class="buttons">
		{#if secret}
			<button type="button" class="btn small ghost" onclick={() => (revealed = !revealed)}
				>{revealed ? 'Hide' : 'Show'}</button
			>
		{/if}
		<button type="button" class="btn small" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
	</span>
	{#if hint}<p class="hint">{hint}</p>{/if}
</div>

<style>
	.secret {
		display: grid;
		grid-template-columns: 1fr auto;
		gap: 4px 12px;
		align-items: center;
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
		background: var(--bg-raised);
		padding: 10px 14px;
		margin: 10px 0;
	}
	.label {
		grid-column: 1 / -1;
	}
	.value {
		word-break: break-all;
		font-size: 13px;
	}
	.hint {
		grid-column: 1 / -1;
		margin: 4px 0 0;
		font-size: 13px;
		opacity: 0.8;
	}
	.buttons {
		display: flex;
		gap: 6px;
	}
	.ghost {
		opacity: 0.75;
	}
	.small {
		padding: 4px 12px;
		font-size: 13px;
	}
</style>
