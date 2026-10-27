<script lang="ts">
	// The whole tutorial, in one line.
	let { appName = 'your app' }: { appName?: string } = $props();
</script>

<p class="flow">
	<span><b>1</b> Copy</span><span class="to">→</span><span><b>2</b> Paste into your AI agent</span
	><span class="to">→</span><span><b>3</b> Keep {appName} open</span>
</p>

<style>
	.flow {
		display: flex;
		flex-wrap: wrap;
		gap: 6px 10px;
		align-items: center;
		margin: 8px 0 16px;
		font-size: 15px;
	}
	b {
		display: inline-grid;
		place-items: center;
		width: 20px;
		height: 20px;
		margin-right: 4px;
		border-radius: 50%;
		background: var(--agent-soft);
		color: var(--agent);
		font-size: 12px;
	}
	.to {
		opacity: 0.4;
	}
</style>
