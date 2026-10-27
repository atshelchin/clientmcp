<script lang="ts">
	import { resolve } from '$app/paths';
	import type { PageProps } from './$types';

	let { data, form }: PageProps = $props();

	const n = (x: number, digits = 1) => x.toLocaleString('en-US', { maximumFractionDigits: digits });
	const mb = (bytes: number) => n(bytes / 1_000_000);
	const pct = (part: number, whole: number) => (whole ? `${n((100 * part) / whole, 1)}%` : '0%');
	const date = (iso: string) => iso.slice(0, 10);
	const time = (iso: string) => iso.slice(0, 16).replace('T', ' ');

	/** "3 h ago", for a timestamp. */
	function ago(iso: string): string {
		const minutes = Math.round((Date.parse(data.now) - Date.parse(iso)) / 60_000);
		if (minutes < 1) return 'just now';
		if (minutes < 60) return `${minutes} min ago`;
		if (minutes < 48 * 60) return `${Math.round(minutes / 60)} h ago`;
		return `${Math.round(minutes / 1440)} d ago`;
	}

	// The 30-day chart: one metric at a time.
	const metrics = {
		calls: { label: 'Calls', value: (d: (typeof data.series)[number]) => d.calls, fmt: n },
		hours: {
			label: 'Connected hours',
			value: (d: (typeof data.series)[number]) => d.connectedHours,
			fmt: n
		},
		mb: {
			label: 'MB',
			value: (d: (typeof data.series)[number]) => d.bytes / 1_000_000,
			fmt: n
		}
	} as const;
	let metric = $state<keyof typeof metrics>('calls');
	let chart = $derived(data.series.map((d) => metrics[metric].value(d)));
	let chartMax = $derived(Math.max(...chart, 0));
	let chartTotal = $derived(chart.reduce((a, b) => a + b, 0));

	// The accounts list.
	let query = $state('');
	let sort = $state<'active' | 'recent' | 'new'>('active');
	let accounts = $derived(
		data.accounts
			.filter((a) => {
				const q = query.trim().toLowerCase().replace(/^@/, '');
				return (
					!q ||
					a.login.toLowerCase().includes(q) ||
					a.id.includes(q) ||
					a.clientList.some((c) => c.id.includes(q) || c.label.toLowerCase().includes(q))
				);
			})
			.sort((x, y) =>
				sort === 'active'
					? y.month.calls + y.today.calls - (x.month.calls + x.today.calls)
					: sort === 'recent'
						? y.lastSeen.localeCompare(x.lastSeen)
						: y.created.localeCompare(x.created)
			)
	);
	let busiest = $derived(Math.max(...data.accounts.map((a) => a.month.calls + a.today.calls), 0));

	let log = $state<'tokens' | 'accounts'>('tokens');

	function confirmFirst(message: string) {
		return (e: SubmitEvent) => {
			if (!confirm(message)) e.preventDefault();
		};
	}
</script>

<svelte:head>
	<title>Admin · ClientMCP</title>
	<meta name="robots" content="noindex" />
</svelte:head>

<section class="wrap admin">
	<header class="top">
		<div>
			<h1>Admin</h1>
			<p class="sub">
				As of {time(data.now)} UTC · today is live, the 30 days are finished UTC days
			</p>
		</div>
		<a class="back" href={resolve('/account')}>Your account →</a>
	</header>

	{#if form?.message}<p class="note" role="status">{form.message}</p>{/if}
	{#if !data.bridgeRecords}
		<p class="warn">The bridge's records could not be read: the 30-day figures are missing.</p>
	{/if}

	<div class="stats">
		<div class="stat">
			<span class="k">Accounts</span>
			<span class="v">{n(data.totals.accounts)}</span>
			<span class="s"
				>{data.totals.activeLastWeek} active this week{#if data.totals.suspended}
					· <b class="bad">{data.totals.suspended} suspended</b>{/if}</span
			>
		</div>
		<div class="stat">
			<span class="k">Clients</span>
			<span class="v">{n(data.totals.clients)}</span>
			<span class="s"
				>{n(data.totals.accounts ? data.totals.clients / data.totals.accounts : 0)} per account</span
			>
		</div>
		<div class="stat">
			<span class="k">Calls today</span>
			<span class="v">{n(data.totals.today.calls)}</span>
			<span class="s"
				>{pct(data.totals.today.failed, data.totals.today.calls)} unanswered{#if data.totals.today.limited}
					· <b class="bad">{n(data.totals.today.limited)} limited</b>{/if}</span
			>
		</div>
		<div class="stat">
			<span class="k">Connected today</span>
			<span class="v">{n(data.totals.today.connectedHours)} h</span>
			<span class="s">{mb(data.totals.today.bytes)} MB through the bridge</span>
		</div>
	</div>

	<div class="panels">
		<section class="panel chart-panel">
			<div class="panel-head">
				<h2>Last 30 days</h2>
				<div class="seg" role="tablist" aria-label="Metric">
					{#each Object.entries(metrics) as [key, m] (key)}
						<button
							type="button"
							role="tab"
							aria-selected={metric === key}
							class:on={metric === key}
							onclick={() => (metric = key as keyof typeof metrics)}>{m.label}</button
						>
					{/each}
				</div>
			</div>
			<p class="total">
				<span class="v">{metrics[metric].fmt(chartTotal)}</span>
				<span class="s">{metrics[metric].label.toLowerCase()} in 30 days</span>
			</p>
			<div class="chart" aria-label="{metrics[metric].label} per day">
				{#each data.series as d, i (d.day)}
					<div
						class="bar"
						style:--h={chartMax
							? `${Math.max((chart[i] / chartMax) * 100, chart[i] ? 2 : 0)}%`
							: '0%'}
						title="{d.day}: {metrics[metric].fmt(chart[i])} {metrics[metric].label.toLowerCase()}"
					></div>
				{/each}
			</div>
			<div class="axis">
				<span>{data.series[0]?.day.slice(5)}</span>
				<span>max {metrics[metric].fmt(chartMax)} / day</span>
				<span>{data.series.at(-1)?.day.slice(5)}</span>
			</div>
		</section>

		<section class="panel">
			<div class="panel-head"><h2>Plan sizing</h2></div>
			<p class="hint">
				Per account per active day, over {n(data.sizing.accountDays)} account-days: what a plan would
				have to cover.
			</p>
			<table class="sizing">
				<thead><tr><th></th><th>Median</th><th>P90</th><th>Max</th></tr></thead>
				<tbody>
					<tr>
						<th>Calls</th><td>{n(data.sizing.calls.median, 0)}</td><td
							>{n(data.sizing.calls.p90, 0)}</td
						><td>{n(data.sizing.calls.max, 0)}</td>
					</tr>
					<tr>
						<th>Hours</th><td>{n(data.sizing.connectedHours.median)}</td><td
							>{n(data.sizing.connectedHours.p90)}</td
						><td>{n(data.sizing.connectedHours.max)}</td>
					</tr>
					<tr>
						<th>MB</th><td>{n(data.sizing.megabytes.median)}</td><td
							>{n(data.sizing.megabytes.p90)}</td
						><td>{n(data.sizing.megabytes.max)}</td>
					</tr>
				</tbody>
			</table>
		</section>
	</div>

	<section class="panel">
		<div class="panel-head">
			<h2>Accounts <span class="count">{accounts.length}</span></h2>
			<div class="tools">
				<input
					type="search"
					placeholder="Search login, account or client"
					bind:value={query}
					aria-label="Search accounts"
				/>
				<select bind:value={sort} aria-label="Sort">
					<option value="active">Most active</option>
					<option value="recent">Last seen</option>
					<option value="new">Newest</option>
				</select>
			</div>
		</div>

		<div class="rows">
			<div class="row head" aria-hidden="true">
				<span>Account</span><span>Clients</span><span>Today</span><span>30 days</span><span
					>Share</span
				><span>Last seen</span>
			</div>
			{#each accounts as a (a.id)}
				{@const share = a.month.calls + a.today.calls}
				<details class="account" class:off={a.state !== 'active'}>
					<summary class="row">
						<span class="who">
							{#if a.avatar_url}
								<img src={a.avatar_url} alt="" width="28" height="28" loading="lazy" />
							{:else}
								<span class="avatar">{a.login.slice(0, 1).toUpperCase()}</span>
							{/if}
							<span class="name">
								<strong>@{a.login}</strong>
								{#if a.id === data.self}<span class="pill">you</span>{/if}
								{#if a.state !== 'active'}<span class="pill bad">suspended</span>{/if}
								<span class="mono dim">{a.id}</span>
							</span>
						</span>
						<span data-k="Clients">{a.clients} <span class="dim">of {a.client_limit}</span></span>
						<span data-k="Today">{n(a.today.calls)} <span class="dim">calls</span></span>
						<span data-k="30 days">
							{n(a.month.calls)}
							<span class="dim">calls · {n(a.month.connectedHours)} h</span>
							{#if a.month.rejections}<b class="bad">· {a.month.rejections} refused</b>{/if}
						</span>
						<span class="share" data-k="Share">
							<span class="meter"
								><span style:width={busiest ? `${(100 * share) / busiest}%` : '0'}></span></span
							>
						</span>
						<span class="dim" data-k="Last seen" title={time(a.lastSeen)}>{ago(a.lastSeen)}</span>
					</summary>

					<div class="detail">
						<div class="manage">
							<span class="dim">Joined {date(a.created)}</span>
							<form method="POST" action="?/setLimit" class="inline">
								<input type="hidden" name="csrf" value={data.csrf} />
								<input type="hidden" name="account" value={a.id} />
								<label
									>Client limit
									<input name="limit" type="number" min="0" max="1000" value={a.client_limit} />
								</label>
								<button class="btn small" type="submit">Save</button>
							</form>
							{#if a.id !== data.self}
								{#if a.state === 'active'}
									<form
										method="POST"
										action="?/suspend"
										onsubmit={confirmFirst(`Suspend @${a.login}? Every one of its tokens stops.`)}
									>
										<input type="hidden" name="csrf" value={data.csrf} />
										<input type="hidden" name="account" value={a.id} />
										<button class="btn small danger" type="submit">Suspend account</button>
									</form>
								{:else}
									<form method="POST" action="?/reactivate">
										<input type="hidden" name="csrf" value={data.csrf} />
										<input type="hidden" name="account" value={a.id} />
										<button class="btn small" type="submit">Reactivate</button>
									</form>
								{/if}
							{/if}
						</div>

						{#if a.clientList.length}
							<table class="clients">
								<thead>
									<tr><th>Client</th><th>Today</th><th>30 days</th><th></th></tr>
								</thead>
								<tbody>
									{#each a.clientList as c (c.id)}
										<tr>
											<td>
												<strong>{c.label}</strong>
												<span class="dim">{c.app ?? 'own app'} · {date(c.created)}</span><br />
												<span class="mono dim">{c.id}</span>
											</td>
											<td>{n(c.today.calls)}</td>
											<td>
												{n(c.month.calls)}
												<span class="dim"
													>· {n(c.month.connectedHours)} h · {mb(c.month.bytes)} MB</span
												>
											</td>
											<td class="end">
												<form
													method="POST"
													action="?/revokeClient"
													onsubmit={confirmFirst(`Revoke both tokens of ${c.label}?`)}
												>
													<input type="hidden" name="csrf" value={data.csrf} />
													<input type="hidden" name="client" value={c.id} />
													<button class="link" type="submit">Revoke</button>
												</form>
												<form
													method="POST"
													action="?/deleteClient"
													onsubmit={confirmFirst(`Delete ${c.label}? This cannot be undone.`)}
												>
													<input type="hidden" name="csrf" value={data.csrf} />
													<input type="hidden" name="client" value={c.id} />
													<button class="link danger" type="submit">Delete</button>
												</form>
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						{:else}
							<p class="dim">No clients.</p>
						{/if}
					</div>
				</details>
			{:else}
				<p class="empty">No account matches.</p>
			{/each}
		</div>
	</section>

	<section class="panel">
		<div class="panel-head">
			<h2>Recent activity</h2>
			<div class="seg" role="tablist" aria-label="Log">
				<button
					type="button"
					role="tab"
					aria-selected={log === 'tokens'}
					class:on={log === 'tokens'}
					onclick={() => (log = 'tokens')}>Tokens</button
				>
				<button
					type="button"
					role="tab"
					aria-selected={log === 'accounts'}
					class:on={log === 'accounts'}
					onclick={() => (log = 'accounts')}>Accounts</button
				>
			</div>
		</div>
		<ol class="log">
			{#if log === 'tokens'}
				{#each data.issuance as r, i (i)}
					<li>
						<time datetime={r.at}>{time(r.at)}</time>
						<span class="pill {r.action}">{r.action}</span>
						<span>
							{r.role} token
							{#if r.gen !== null}<span class="dim">· gen {r.gen}</span>{/if}
						</span>
						<span class="subject">
							{r.label ?? ''}
							{#if r.login}<span class="dim">@{r.login}</span>{/if}
							<span class="mono dim">{r.client}</span>
						</span>
						<span class="dim">{r.actor}</span>
					</li>
				{:else}
					<li class="empty">Nothing yet.</li>
				{/each}
			{:else}
				{#each data.audit as r, i (i)}
					<li>
						<time datetime={r.at}>{time(r.at)}</time>
						<span class="pill">{r.action}</span>
						<span
							>{#if r.login}@{r.login}{/if}</span
						>
						<span class="subject mono dim">{r.detail}</span>
						<span class="dim">{r.actor}</span>
					</li>
				{:else}
					<li class="empty">Nothing yet.</li>
				{/each}
			{/if}
		</ol>
	</section>
</section>

<style>
	.admin {
		padding-block: 40px 64px;
		display: grid;
		gap: 20px;
	}
	.top {
		display: flex;
		align-items: flex-end;
		justify-content: space-between;
		gap: 16px;
		flex-wrap: wrap;
	}
	h1 {
		margin: 0;
		font-size: 32px;
		letter-spacing: -0.02em;
	}
	h2 {
		margin: 0;
		font-size: 15px;
		font-weight: 600;
	}
	.sub,
	.hint {
		margin: 4px 0 0;
		color: var(--ink-3);
		font-size: 13px;
	}
	.back {
		font-size: 14px;
	}
	.dim {
		color: var(--ink-3);
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 12px;
	}
	.bad {
		color: var(--human);
		font-weight: 500;
	}
	.note,
	.warn {
		margin: 0;
		padding: 10px 14px;
		border-radius: var(--radius-sm);
		font-size: 14px;
	}
	.note {
		background: var(--agent-soft);
	}
	.warn {
		background: var(--human-soft);
	}

	/* Headline figures */
	.stats {
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-raised);
		box-shadow: var(--shadow);
		overflow: hidden;
	}
	.stat {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding: 16px 18px;
		min-width: 0;
	}
	.stat + .stat {
		border-left: 1px solid var(--line);
	}
	.k {
		font-size: 12px;
		font-weight: 500;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--ink-3);
	}
	.v {
		font-size: 28px;
		font-weight: 600;
		letter-spacing: -0.02em;
		font-variant-numeric: tabular-nums;
	}
	.s {
		font-size: 13px;
		color: var(--ink-2);
	}

	/* Panels */
	.panels {
		display: grid;
		grid-template-columns: 2fr 1fr;
		gap: 20px;
	}
	.panel {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--bg-raised);
		padding: 16px 18px;
		min-width: 0;
	}
	.panel-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		flex-wrap: wrap;
		margin-bottom: 8px;
	}
	.count {
		margin-left: 4px;
		color: var(--ink-3);
		font-weight: 400;
	}
	.seg {
		display: inline-flex;
		padding: 2px;
		border-radius: 999px;
		background: var(--bg-sunken);
	}
	.seg button {
		font: inherit;
		font-size: 12px;
		border: 0;
		background: none;
		color: var(--ink-2);
		padding: 4px 10px;
		border-radius: 999px;
		cursor: pointer;
	}
	.seg button.on {
		background: var(--bg-raised);
		color: var(--ink);
		box-shadow: 0 1px 2px rgb(0 0 0 / 0.12);
	}

	/* The chart */
	.total {
		margin: 0 0 10px;
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	.chart {
		height: 140px;
		display: flex;
		align-items: flex-end;
		gap: 3px;
		border-bottom: 1px solid var(--line);
	}
	.bar {
		flex: 1;
		height: var(--h);
		min-width: 0;
		border-radius: 3px 3px 0 0;
		background: var(--agent);
		opacity: 0.85;
	}
	.bar:hover {
		opacity: 1;
	}
	.axis {
		display: flex;
		justify-content: space-between;
		margin-top: 6px;
		font-size: 12px;
		color: var(--ink-3);
		font-variant-numeric: tabular-nums;
	}

	/* Plan sizing */
	.sizing {
		width: 100%;
		margin-top: 12px;
		border-collapse: collapse;
		font-size: 14px;
		font-variant-numeric: tabular-nums;
	}
	.sizing th,
	.sizing td {
		padding: 8px 4px;
		text-align: right;
		border-bottom: 1px solid var(--line);
	}
	.sizing thead th {
		font-size: 12px;
		font-weight: 500;
		color: var(--ink-3);
	}
	.sizing tbody th {
		text-align: left;
		font-weight: 500;
	}
	.sizing tr:last-child > * {
		border-bottom: 0;
	}

	/* Accounts */
	.tools {
		display: flex;
		gap: 8px;
	}
	.tools input,
	.tools select,
	.manage input {
		font: inherit;
		font-size: 13px;
		color: inherit;
		background: var(--bg);
		border: 1px solid var(--line-strong);
		border-radius: var(--radius-sm);
		padding: 5px 9px;
	}
	.tools input {
		width: 240px;
	}
	.rows {
		margin: 0 -18px -16px;
	}
	.row {
		display: grid;
		grid-template-columns: minmax(220px, 2.4fr) 0.7fr 0.8fr 1.4fr 1fr 0.9fr;
		gap: 12px;
		align-items: center;
		padding: 10px 18px;
		font-size: 14px;
		font-variant-numeric: tabular-nums;
	}
	.row.head {
		padding-block: 6px;
		font-size: 12px;
		font-weight: 500;
		color: var(--ink-3);
		border-bottom: 1px solid var(--line);
	}
	.account {
		border-bottom: 1px solid var(--line);
	}
	.account:last-child {
		border-bottom: 0;
	}
	.account summary {
		cursor: pointer;
		list-style: none;
	}
	.account summary::-webkit-details-marker {
		display: none;
	}
	.account summary:hover,
	.account[open] summary {
		background: var(--bg-sunken);
	}
	.account.off .who,
	.account.off summary > span:not(.who) {
		opacity: 0.6;
	}
	.who {
		display: flex;
		align-items: center;
		gap: 10px;
		min-width: 0;
	}
	.who img,
	.avatar {
		flex: none;
		width: 28px;
		height: 28px;
		border-radius: 50%;
	}
	.avatar {
		display: grid;
		place-items: center;
		background: var(--bg-sunken);
		font-size: 13px;
		font-weight: 600;
	}
	.name {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		column-gap: 6px;
		min-width: 0;
	}
	.name .mono {
		flex-basis: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.pill {
		display: inline-block;
		padding: 1px 7px;
		border-radius: 999px;
		font-size: 11px;
		font-weight: 500;
		line-height: 1.5;
		background: var(--bg-sunken);
		color: var(--ink-2);
		white-space: nowrap;
	}
	.pill.bad,
	.pill.revoke,
	.pill.delete {
		background: var(--human-soft);
		color: var(--human);
	}
	.pill.mint,
	.pill.rotate {
		background: var(--agent-soft);
		color: var(--agent);
	}
	.meter {
		display: block;
		height: 6px;
		border-radius: 3px;
		background: var(--bg-sunken);
		overflow: hidden;
	}
	.meter span {
		display: block;
		height: 100%;
		background: var(--ink-2);
	}
	.detail {
		padding: 4px 18px 16px 56px;
		background: var(--bg-sunken);
	}
	.manage {
		display: flex;
		align-items: center;
		gap: 16px;
		flex-wrap: wrap;
		padding: 8px 0 12px;
		font-size: 13px;
	}
	.manage label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.manage input[type='number'] {
		width: 72px;
	}
	.inline {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.btn.small {
		padding: 4px 14px;
		font-size: 13px;
		min-height: 0;
	}
	.danger {
		color: var(--human);
	}
	.clients {
		width: 100%;
		border-collapse: collapse;
		font-size: 13px;
		font-variant-numeric: tabular-nums;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: var(--radius-sm);
	}
	.clients th {
		text-align: left;
		font-weight: 500;
		font-size: 12px;
		color: var(--ink-3);
	}
	.clients th,
	.clients td {
		padding: 8px 10px;
		border-bottom: 1px solid var(--line);
		vertical-align: top;
	}
	.clients tr:last-child td {
		border-bottom: 0;
	}
	.end {
		text-align: right;
		white-space: nowrap;
	}
	.end form {
		display: inline;
		margin-left: 10px;
	}
	.link {
		font: inherit;
		background: none;
		border: 0;
		padding: 0;
		color: var(--ink-2);
		text-decoration: underline;
		cursor: pointer;
	}
	.link.danger {
		color: var(--human);
	}
	.empty {
		padding: 16px 18px;
		color: var(--ink-3);
	}

	/* Activity */
	.log {
		list-style: none;
		margin: 0 -18px -16px;
		padding: 0;
		font-size: 13px;
		max-height: 420px;
		overflow-y: auto;
	}
	.log li {
		display: grid;
		grid-template-columns: 130px 80px minmax(110px, 0.8fr) 2fr minmax(120px, 1fr);
		gap: 12px;
		align-items: baseline;
		padding: 7px 18px;
		border-top: 1px solid var(--line);
	}
	.log time {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink-3);
	}
	.log .pill {
		justify-self: start;
	}
	.subject {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	@media (max-width: 900px) {
		.stats {
			grid-template-columns: repeat(2, 1fr);
		}
		.stat:nth-child(3) {
			border-left: 0;
		}
		.stat:nth-child(n + 3) {
			border-top: 1px solid var(--line);
		}
		.panels {
			grid-template-columns: 1fr;
		}
		.row.head {
			display: none;
		}
		.row {
			grid-template-columns: 1fr 1fr 1fr;
			row-gap: 6px;
		}
		.row .who {
			grid-column: 1 / -1;
		}
		.row > [data-k]::before {
			content: attr(data-k);
			display: block;
			font-size: 11px;
			color: var(--ink-3);
		}
		.detail {
			padding-left: 18px;
		}
		.tools,
		.tools input {
			width: 100%;
		}
		.log li {
			grid-template-columns: auto 1fr;
			row-gap: 2px;
		}
	}
</style>
