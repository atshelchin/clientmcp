<script lang="ts">
	// One value to copy, in a field: masked until Show, fetched only on Show
	// or Copy. Safari lets a clipboard write follow a fetch only when the
	// write is handed the promise, hence ClipboardItem where there is one.
	let {
		label,
		shown,
		fetchValue,
		secret = true
	}: {
		label?: string;
		shown: string;
		fetchValue: () => Promise<string>;
		secret?: boolean;
	} = $props();
	let status = $state<'idle' | 'busy' | 'copied' | 'failed'>('idle');
	let revealed = $state<string | null>(null);
	/** Why the last Show or Copy failed, said under the field. */
	let problem = $state<string | null>(null);

	function settle(next: typeof status) {
		status = next;
		setTimeout(() => (status = 'idle'), 1800);
	}

	function failed(e: unknown) {
		problem = e instanceof Error && e.message ? e.message : 'That did not work. Please try again.';
		settle('failed');
	}

	async function copy() {
		status = 'busy';
		problem = null;
		try {
			if (revealed !== null) {
				await navigator.clipboard.writeText(revealed);
			} else if (typeof ClipboardItem !== 'undefined') {
				const blob = fetchValue().then((text) => new Blob([text], { type: 'text/plain' }));
				await navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })]);
			} else {
				await navigator.clipboard.writeText(await fetchValue());
			}
			settle('copied');
		} catch (e) {
			failed(e);
		}
	}

	async function toggle() {
		if (revealed !== null) {
			revealed = null;
			return;
		}
		status = 'busy';
		problem = null;
		try {
			revealed = await fetchValue();
			status = 'idle';
		} catch (e) {
			failed(e);
		}
	}
</script>

<div class="field">
	{#if label}<span class="label">{label}</span>{/if}
	<code class="value">{revealed ?? shown}</code>
	<span class="buttons">
		{#if secret}
			<button type="button" class="ghost" onclick={toggle} disabled={status === 'busy'}
				>{revealed === null ? 'Show' : 'Hide'}</button
			>
		{/if}
		<button type="button" class="copy" onclick={copy} disabled={status === 'busy'}>
			{status === 'copied' ? 'Copied ✓' : status === 'failed' ? 'Try again' : 'Copy'}
		</button>
	</span>
</div>
{#if problem}<p class="problem" role="alert">{problem}</p>{/if}

<style>
	.field {
		display: flex;
		align-items: center;
		gap: 10px;
		border: 1px solid var(--line-strong);
		border-radius: var(--radius-sm);
		background: var(--bg);
		padding: 6px 6px 6px 12px;
		margin: 6px 0;
	}
	.label {
		flex: none;
		font-size: 13px;
		opacity: 0.7;
	}
	.value {
		flex: 1;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: 13px;
		word-break: break-all;
		white-space: pre-wrap;
	}
	.buttons {
		flex: none;
		display: flex;
		gap: 4px;
	}
	button {
		font: inherit;
		font-size: 13px;
		padding: 5px 12px;
		border-radius: 999px;
		cursor: pointer;
	}
	.ghost {
		border: 0;
		background: transparent;
		color: inherit;
		opacity: 0.7;
	}
	.copy {
		border: 1px solid var(--ink);
		background: var(--ink);
		color: var(--bg);
	}
	.problem {
		margin: -2px 0 8px;
		font-size: 13px;
		color: var(--human);
	}
	button:disabled {
		opacity: 0.5;
	}
</style>
