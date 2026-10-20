<script lang="ts">
	import { resolve } from '$app/paths';
	import Code from '#lib/components/Code.svelte';
	import HeroDemo from '#lib/components/HeroDemo.svelte';
	import GitHubIcon from '#lib/components/GitHubIcon.svelte';
	import { site } from '#lib/site.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const features = [
		{
			title: 'No server in the app',
			text: 'The application opens no port and runs no HTTP server. One outbound WebSocket, so a browser tab behind NAT works as well as a desktop app.'
		},
		{
			title: 'Both MCP eras, one endpoint',
			text: 'The bridge serves MCP 2026-07-28 and the 2025 revisions side by side, decided per request, keeping no per-agent state in either.'
		},
		{
			title: 'Offline is a state, not an error',
			text: 'A closed app simply has no tools. Agents stay connected and get tools/list_changed the moment it comes back.'
		},
		{
			title: 'Calls can wait for a person',
			text: 'A call still running after a second streams as SSE with keep-alives, so a user can take a minute to confirm without a proxy cutting it off.'
		},
		{
			title: 'Stateless tokens',
			text: 'HS256 JWTs with a client ID and a role. Any backend can mint them and any bridge verifies them without a database. Secrets rotate without downtime.'
		},
		{
			title: 'Declarative tools',
			text: 'Each register declares the complete tool set and the bridge mirrors it. There are no incremental updates to lose or reorder.'
		}
	];

	const stats = [
		{
			value: '~11 µs',
			label: 'bridge CPU per tools/call round trip',
			note: '~14 µs on MCP 2026-07-28'
		},
		{
			value: '86–93k',
			label: 'calls per second, 64 agents → 1 client',
			note: 'p99 ≈ 1.9 ms on 2 shared vCPUs'
		},
		{
			value: '~11 KB',
			label: 'memory per idle connected client',
			note: '1.2 MB for the idle process'
		},
		{ value: '0', label: 'CPU while 10,000 clients sit idle', note: 'each pinging every 25 s' }
	];

	const covered = [
		'authentication',
		'origin checks & CORS',
		'both MCP eras',
		'header validation',
		'x-mcp-header',
		'notification streams',
		'streamed slow calls',
		'concurrency',
		'timeouts',
		'cancellation',
		'disconnection',
		'replacement',
		'size limits',
		'tool validation'
	];

	const limits = [
		{
			title: 'The bridge is trusted',
			text: 'It terminates TLS and sees every argument and result. There is no end-to-end encryption between agent and app: run a bridge you operate or trust.'
		},
		{
			title: 'Revocation means rotation',
			text: 'Before exp, only rotating the secret revokes a token. Several secrets can be active at once, so rotating needs no downtime.'
		},
		{
			title: 'One server instance per client ID',
			text: 'The single-process bridge holds tens of thousands of clients, but several instances need requests for a client routed to one of them. Workers scale by construction.'
		},
		{
			title: 'Tools only, for now',
			text: 'Resources, prompts, progress notifications and multi round-trip requests are not in v1. They fit later as new message types.'
		}
	];
</script>

<svelte:head>
	<title>ClientMCP: expose live client applications as MCP servers</title>
	<meta name="description" content={site.description} />
	<meta property="og:title" content="ClientMCP" />
	<meta property="og:description" content={site.description} />
</svelte:head>

<!-- Hero -->
<section class="hero">
	<div class="wrap">
		<div class="intro">
			<div>
				<p class="label">
					<span class="num">Open protocol</span> v{site.specVersion} · MIT / Apache-2.0
				</p>
				<h1>
					UI for <span class="human">humans</span>.<br />
					MCP for <span class="agent">agents</span>.
				</h1>
			</div>
			<div class="pitch">
				<p class="lead">
					ClientMCP exposes a live browser, desktop or mobile application as a standard MCP server.
					Agents call the same actions as its UI, on the same state. When an agent adds a todo, the
					user sees it appear.
				</p>
				<div class="cta">
					<a class="btn primary" href={resolve('/docs/[[page]]#quick-start', {})}
						>Get started <span class="arrow" aria-hidden="true">→</span></a
					>
					<a class="btn" href={resolve('/docs/[[page]]', { page: 'spec' })}>Read the spec</a>
				</div>
				<ul class="facts mono" aria-label="At a glance">
					<li>No port in the app</li>
					<li>Any MCP client</li>
					<li>Docker or Cloudflare Workers</li>
				</ul>
			</div>
		</div>
		<div class="stage">
			<HeroDemo />
		</div>
	</div>
</section>

<!-- 01 The idea -->
<section class="section" id="idea">
	<div class="wrap split">
		<div class="copy">
			<p class="label"><span class="num">01</span> The idea</p>
			<h2>One application.<br />Two interfaces.<br /><em>Shared state.</em></h2>
			<p class="lead">
				Agents usually reach an application from the outside: screenshots and simulated clicks, or a
				separate backend API that cannot see what is open in the app. ClientMCP works from the
				inside. The tool you expose is the function your button already calls.
			</p>
			<ul class="points">
				<li>
					<strong>Nothing to keep in sync.</strong> There is one implementation of each action.
				</li>
				<li>
					<strong>No UI automation.</strong> No screenshots, DOM scripting or simulated input.
				</li>
				<li>
					<strong>Changes show up on both sides.</strong> The agent sees what the user did; the user sees
					what the agent did.
				</li>
			</ul>
		</div>
		<Code html={data.idea.html} raw={data.idea.raw} title="app.ts" />
	</div>
</section>

<!-- 02 How it works -->
<section class="section" id="how">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">02</span> How it works</p>
			<h2>Both sides dial out. The bridge meets them.</h2>
			<p class="lead">
				The application opens one WebSocket to a bridge and declares its tools. Agents talk to the
				same bridge over standard MCP Streamable HTTP. Neither needs to be reachable, and the bridge
				stores nothing.
			</p>
		</div>

		<div
			class="arch"
			role="img"
			aria-label="The application connects out to the bridge over a WebSocket; the agent connects to the bridge over MCP Streamable HTTP."
		>
			<div class="box app-box">
				<span class="tag human">Human</span>
				<div class="layer">UI</div>
				<div class="layer core">Application state &amp; actions</div>
				<div class="layer sdk">ClientMCP SDK</div>
				<span class="box-name">Your application</span>
			</div>
			<div class="link">
				<span class="link-label">WebSocket<br /><small>outbound only</small></span>
				<span class="line"><i class="head-to-bridge"></i></span>
			</div>
			<div class="box bridge-box">
				<div class="layer core">ClientMCP bridge</div>
				<span class="box-sub mono">Docker · Cloudflare Workers</span>
				<span class="box-name">Stores nothing</span>
			</div>
			<div class="link reverse">
				<span class="link-label">MCP Streamable HTTP<br /><small>Bearer token</small></span>
				<span class="line"><i class="head-to-bridge"></i></span>
			</div>
			<div class="box agent-box">
				<span class="tag agent">Agent</span>
				<div class="layer core">Any MCP client</div>
				<span class="box-sub mono">Claude Code · MCP SDKs · frameworks</span>
			</div>
		</div>

		<div class="trio">
			<div>
				<h3><span class="dot human"></span>Client SDK</h3>
				<p>
					Declares tools, receives calls and runs them in the app. Pings, reconnects with backoff
					and re-registers on its own. No dependencies.
				</p>
			</div>
			<div>
				<h3><span class="dot ink"></span>Bridge</h3>
				<p>
					Translates between the WebSocket and MCP. One Rust core with two runtimes: a 1.2 MB static
					binary for any Linux host, or Cloudflare Workers with Durable Objects.
				</p>
			</div>
			<div>
				<h3><span class="dot agent"></span>Agent</h3>
				<p>
					Needs nothing beyond MCP and a bearer token. tools/list returns what the app declared;
					tools/call runs inside the app.
				</p>
			</div>
		</div>
	</div>
</section>

<!-- 03 Quick start -->
<section class="section" id="start">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">03</span> Quick start</p>
			<h2>From a running app to a working agent in four steps.</h2>
		</div>
		<ol class="steps">
			{#each data.quickstart as step, i (step.title)}
				<li>
					<div class="step-copy">
						<span class="step-num mono">{String(i + 1).padStart(2, '0')}</span>
						<h3>{step.title}</h3>
						<p>{step.text}</p>
					</div>
					<Code html={step.code.html} raw={step.code.raw} />
				</li>
			{/each}
		</ol>
		<p class="after">
			<a href={resolve('/docs/[[page]]', {})}>Read the full guide</a> for production tokens, TLS and
			configuration, or open the
			<a href={`${site.repo}/blob/main/examples/browser/index.html`} rel="external noopener"
				>browser example</a
			>, a complete shared todo list.
		</p>
	</div>
</section>

<!-- 04 Details -->
<section class="section" id="details">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">04</span> The details</p>
			<h2>Built around what goes wrong in practice.</h2>
		</div>
		<div class="features">
			{#each features as feature (feature.title)}
				<article>
					<h3>{feature.title}</h3>
					<p>{feature.text}</p>
				</article>
			{/each}
		</div>
	</div>
</section>

<!-- 05 Authority -->
<section class="section" id="authority">
	<div class="wrap split">
		<div class="copy">
			<p class="label"><span class="num">05</span> Control</p>
			<h2>The application stays in charge.</h2>
			<p class="lead">
				Arguments come from a language model, and prompt injection can steer it. So the app decides
				which tools exist, validates every argument and asks its user before anything consequential.
				Two optional conventions let every app say so in the same words.
			</p>
			<dl class="conventions">
				<div>
					<dt>Risk</dt>
					<dd>
						<span class="chip">observe</span><span class="chip">operate</span><span
							class="chip warn">manage</span
						><span class="chip warn">transact</span>
						<p>Published as <code>_meta</code> with the MCP annotations each level implies.</p>
					</dd>
				</div>
				<div>
					<dt>Outcome</dt>
					<dd>
						<span class="chip ok">completed</span><span class="chip">accepted</span><span
							class="chip warn">rejected</span
						>
						<p>The change held, it started and goes on, or nothing changed and why.</p>
					</dd>
				</div>
			</dl>
		</div>
		<Code html={data.authority.html} raw={data.authority.raw} title="app.ts" />
	</div>
</section>

<!-- 06 Protocol -->
<section class="section" id="protocol">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">06</span> Protocol</p>
			<h2>Seven JSON messages. That is the whole client side.</h2>
			<p class="lead">
				The bridge maps them onto MCP: <code>tools/list</code> returns the declared tools,
				<code>tools/call</code> becomes a <code>call</code>, and every change is pushed as
				<code>notifications/tools/list_changed</code>. Tool definitions, arguments and results pass
				through verbatim, so new MCP fields need no bridge release.
			</p>
		</div>
		<div class="messages">
			<div class="legend mono">
				<span><b class="human">↑</b> app → bridge</span>
				<span><b class="agent">↓</b> bridge → app</span>
			</div>
			<ul>
				{#each data.messages as message (message.type)}
					<li class={message.dir}>
						<span
							class="arrow"
							aria-label={message.dir === 'up' ? 'app to bridge' : 'bridge to app'}
							>{message.dir === 'up' ? '↑' : '↓'}</span
						>
						<span class="type mono">{message.type}</span>
						<span class="meaning">{message.meaning}</span>
						<!-- eslint-disable-next-line svelte/no-at-html-tags -- highlighted at build time -->
						<code class="example">{@html message.html}</code>
					</li>
				{/each}
			</ul>
		</div>
		<p class="after">
			<a href={resolve('/docs/[[page]]', { page: 'spec' })}>Read the specification</a>: tokens,
			close codes, both MCP eras, limits and security, in one page.
		</p>
	</div>
</section>

<!-- 07 Deploy -->
<section class="section" id="deploy">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">07</span> Deploy</p>
			<h2>Two runtimes. One Rust core. The same tests.</h2>
		</div>
		<div class="deploys">
			<article>
				<h3>Linux &amp; Docker</h3>
				<p>
					A 1.2 MB static binary on <code>scratch</code>, running as an unprivileged user. Put TLS
					in front; the repository has a Compose file with Caddy that gets certificates
					automatically.
				</p>
				<Code html={data.docker.html} raw={data.docker.raw} />
			</article>
			<article>
				<h3>Cloudflare Workers</h3>
				<p>
					One Durable Object per client ID holds its WebSocket through the Hibernation API. Pings
					are answered at the edge, so a connected but idle app keeps nothing awake.
				</p>
				<Code html={data.workers.html} raw={data.workers.raw} />
			</article>
		</div>
	</div>
</section>

<!-- 08 Performance & verification -->
<section class="section" id="performance">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">08</span> Measured</p>
			<h2>Small enough to forget it is there.</h2>
		</div>
		<dl class="stats">
			{#each stats as stat (stat.label)}
				<div>
					<dt>{stat.label}</dt>
					<dd>
						<span class="value">{stat.value}</span>
						<span class="note">{stat.note}</span>
					</dd>
				</div>
			{/each}
		</dl>
		<p class="footnote">
			Static binary on Linux aarch64, loopback, with the load generator on the same machine, so the
			numbers understate the bridge alone. <a href={resolve('/docs/[[page]]#performance', {})}
				>Full table</a
			>.
		</p>

		<div class="verified">
			<div>
				<h3>Verified against the real thing</h3>
				<p>
					The conformance suite drives a bridge with the official MCP SDKs as agents (v1 for the
					2025 era, v2 for 2026-07-28) and passes unchanged against the native server, the Docker
					image and the Worker running in workerd. Beyond the suite, the browser example was run end
					to end in Chrome, and Claude Code, as a real agent, listed and added todos through the
					bridge.
				</p>
			</div>
			<ul class="covered mono" aria-label="Covered by the conformance suite">
				{#each covered as item (item)}<li>{item}</li>{/each}
			</ul>
		</div>
	</div>
</section>

<!-- 09 Limits -->
<section class="section" id="limits">
	<div class="wrap">
		<div class="head">
			<p class="label"><span class="num">09</span> Limits</p>
			<h2>What v1 does not do.</h2>
		</div>
		<div class="limits">
			{#each limits as limit (limit.title)}
				<article>
					<h3>{limit.title}</h3>
					<p>{limit.text}</p>
				</article>
			{/each}
		</div>
	</div>
</section>

<!-- Closing -->
<section class="closing">
	<div class="wrap">
		<h2>Give your app a <em>second interface.</em></h2>
		<div class="cta">
			<a class="btn primary" href={resolve('/docs/[[page]]#quick-start', {})}
				>Get started <span class="arrow" aria-hidden="true">→</span></a
			>
			<a class="btn" href={site.repo} rel="external noopener"><GitHubIcon size={16} /> GitHub</a>
		</div>
	</div>
</section>

<style>
	/* Hero */

	.hero {
		padding: 64px 0 24px;
	}

	.intro {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 0 64px;
	}

	@media (min-width: 1100px) {
		.hero {
			padding-top: 80px;
		}

		.intro {
			grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
			align-items: end;
		}
	}

	h1 {
		margin-top: 22px;
		font-size: clamp(44px, 9vw, 86px);
		line-height: 0.98;
		letter-spacing: -0.045em;
		font-weight: 600;
	}

	.lead {
		margin-top: 22px;
		max-width: 60ch;
		font-size: 18px;
		line-height: 1.55;
		color: var(--ink-2);
	}

	.hero .lead {
		font-size: clamp(18px, 2.2vw, 20px);
		max-width: 56ch;
	}

	@media (min-width: 1100px) {
		.pitch .lead {
			margin-top: 0;
		}
	}

	.cta {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
		margin-top: 30px;
	}

	.facts {
		display: flex;
		flex-wrap: wrap;
		gap: 8px 22px;
		margin: 26px 0 0;
		padding: 0;
		list-style: none;
		font-size: 12.5px;
		color: var(--ink-3);
	}

	.facts li::before {
		content: '+';
		margin-right: 8px;
		color: var(--ink-2);
	}

	.stage {
		margin-top: 56px;
	}

	/* Sections */

	.section {
		padding: 72px 0;
		border-top: 1px solid var(--line);
	}

	@media (min-width: 900px) {
		.section {
			padding: 112px 0;
		}
	}

	.hero + .section {
		margin-top: 56px;
	}

	h2 {
		margin-top: 18px;
		font-size: clamp(32px, 5vw, 52px);
		line-height: 1.04;
		letter-spacing: -0.035em;
		max-width: 22ch;
	}

	h2 em,
	.closing em {
		font-family: var(--font-serif);
		font-style: italic;
		font-weight: 400;
		letter-spacing: -0.01em;
	}

	.head {
		margin-bottom: 48px;
	}

	.split {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 40px;
		align-items: center;
	}

	@media (min-width: 960px) {
		.split {
			grid-template-columns: minmax(0, 1fr) minmax(0, 1.05fr);
			gap: 64px;
		}
	}

	.copy .lead {
		font-size: 17px;
	}

	.points {
		margin: 28px 0 0;
		padding: 0;
		list-style: none;
		display: grid;
		gap: 12px;
		color: var(--ink-2);
	}

	.points li {
		padding-left: 22px;
		position: relative;
	}

	.points li::before {
		content: '';
		position: absolute;
		left: 2px;
		top: 0.62em;
		width: 9px;
		height: 1.5px;
		background: var(--ink);
	}

	.points strong {
		color: var(--ink);
		font-weight: 600;
	}

	.after {
		margin-top: 36px;
		color: var(--ink-2);
	}

	.after a,
	.footnote a {
		color: var(--ink);
		text-underline-offset: 3px;
	}

	code {
		padding: 1px 5px;
		border-radius: 4px;
		background: var(--bg-sunken);
		font-size: 0.86em;
	}

	/* Architecture */

	.arch {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		align-items: stretch;
		gap: 0;
	}

	@media (min-width: 960px) {
		.arch {
			grid-template-columns:
				minmax(0, 1.25fr) minmax(120px, 0.7fr) minmax(0, 1fr) minmax(120px, 0.7fr)
				minmax(0, 1fr);
			align-items: center;
		}
	}

	.box {
		position: relative;
		padding: 18px;
		border: 1px solid var(--line-strong);
		border-radius: 14px;
		background: var(--bg-raised);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.tag {
		position: absolute;
		top: -10px;
		right: 14px;
		padding: 1px 8px;
		border-radius: 999px;
		background: var(--bg);
		border: 1px solid currentColor;
		font-family: var(--font-mono);
		font-size: 11px;
	}

	.layer {
		padding: 9px 12px;
		border: 1px dashed var(--line-strong);
		border-radius: 8px;
		font-size: 14px;
		color: var(--ink-2);
	}

	.layer.core {
		border-style: solid;
		border-color: var(--ink);
		color: var(--ink);
		font-weight: 600;
	}

	.layer.sdk {
		border-style: solid;
		border-color: var(--human);
		color: var(--human);
		font-family: var(--font-mono);
		font-size: 13px;
	}

	.agent-box .layer.core {
		border-color: var(--agent);
		color: var(--agent);
	}

	.box-name {
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-3);
	}

	.box-sub {
		font-size: 11.5px;
		color: var(--ink-2);
	}

	.app-box {
		padding-top: 22px;
	}

	@media (min-width: 960px) {
		.tag {
			right: auto;
			left: 14px;
		}
	}

	.agent-box {
		padding-top: 22px;
	}

	.link {
		position: relative;
		display: grid;
		grid-template-columns: auto 1fr;
		align-items: center;
		min-height: 92px;
		padding-left: 28px;
	}

	.link .line {
		grid-row: 1;
		grid-column: 1;
		position: absolute;
		left: 28px;
		top: 0;
		bottom: 0;
		border-left: 1.5px solid var(--ink);
	}

	.link-label {
		grid-column: 2;
		padding-left: 18px;
		font-family: var(--font-mono);
		font-size: 12px;
		line-height: 1.4;
	}

	.link-label small {
		color: var(--ink-3);
		font-size: 11px;
	}

	/* arrowheads point into the bridge */
	.head-to-bridge {
		position: absolute;
		left: -5.5px;
		bottom: -1px;
		width: 0;
		height: 0;
		border-left: 5px solid transparent;
		border-right: 5px solid transparent;
		border-top: 8px solid var(--ink);
	}

	.link.reverse .head-to-bridge {
		top: -1px;
		bottom: auto;
		border-top: 0;
		border-bottom: 8px solid var(--ink);
	}

	@media (min-width: 960px) {
		.link {
			display: flex;
			flex-direction: column;
			justify-content: center;
			min-height: 0;
			padding: 0 10px;
			text-align: center;
		}

		.link .line {
			position: relative;
			left: auto;
			top: auto;
			bottom: auto;
			display: block;
			width: 100%;
			height: 0;
			margin-top: 10px;
			border-left: 0;
			border-top: 1.5px solid var(--ink);
		}

		.link-label {
			padding-left: 0;
		}

		.head-to-bridge {
			left: auto;
			right: -1px;
			top: -5.75px;
			bottom: auto;
			border-top: 5px solid transparent;
			border-bottom: 5px solid transparent;
			border-left: 8px solid var(--ink);
			border-right: 0;
		}

		.link.reverse .head-to-bridge {
			right: auto;
			left: -1px;
			top: -5.75px;
			border-top: 5px solid transparent;
			border-bottom: 5px solid transparent;
			border-right: 8px solid var(--ink);
			border-left: 0;
		}
	}

	.trio {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 28px;
		margin-top: 56px;
	}

	@media (min-width: 760px) {
		.trio {
			grid-template-columns: repeat(3, minmax(0, 1fr));
			gap: 40px;
		}
	}

	.trio h3,
	.features h3,
	.limits h3,
	.deploys h3,
	.verified h3 {
		font-size: 17px;
		letter-spacing: -0.015em;
	}

	.trio h3 {
		display: flex;
		align-items: center;
		gap: 10px;
	}

	.trio p,
	.features p,
	.limits p,
	.deploys p,
	.verified p {
		margin-top: 8px;
		color: var(--ink-2);
		font-size: 15.5px;
	}

	.dot {
		width: 9px;
		height: 9px;
		border-radius: 50%;
	}

	.dot.human {
		background: var(--human);
	}

	.dot.agent {
		background: var(--agent);
	}

	.dot.ink {
		background: var(--ink);
	}

	/* Quick start */

	.steps {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 0;
	}

	.steps li {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 16px;
		padding: 28px 0;
		border-top: 1px solid var(--line);
	}

	.steps li:first-child {
		border-top: 0;
		padding-top: 0;
	}

	@media (min-width: 960px) {
		.steps li {
			grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.4fr);
			gap: 48px;
			align-items: start;
		}
	}

	.step-num {
		font-size: 12px;
		color: var(--ink-3);
	}

	.steps h3 {
		margin-top: 6px;
		font-size: 20px;
		letter-spacing: -0.02em;
	}

	.steps p {
		margin-top: 8px;
		color: var(--ink-2);
		font-size: 15.5px;
		max-width: 44ch;
	}

	/* Features */

	.features,
	.limits {
		display: grid;
		gap: 1px;
		background: var(--line);
		border: 1px solid var(--line);
		border-radius: 14px;
		overflow: hidden;
	}

	@media (min-width: 640px) {
		.features,
		.limits {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	@media (min-width: 1000px) {
		.features {
			grid-template-columns: repeat(3, minmax(0, 1fr));
		}
	}

	.features article,
	.limits article {
		padding: 28px;
		background: var(--bg);
	}

	/* Conventions */

	.conventions {
		margin: 28px 0 0;
		display: grid;
		gap: 18px;
	}

	.conventions > div {
		display: grid;
		grid-template-columns: 80px minmax(0, 1fr);
		gap: 12px;
		padding-top: 18px;
		border-top: 1px solid var(--line);
	}

	.conventions dt {
		font-family: var(--font-mono);
		font-size: 12px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--ink-3);
		padding-top: 3px;
	}

	.conventions dd {
		margin: 0;
	}

	.conventions dd p {
		margin-top: 8px;
		color: var(--ink-2);
		font-size: 15px;
	}

	.chip {
		display: inline-block;
		margin: 0 6px 6px 0;
		padding: 2px 9px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		font-family: var(--font-mono);
		font-size: 12px;
	}

	.chip.warn {
		color: var(--human);
		border-color: currentColor;
	}

	.chip.ok {
		color: var(--ok);
		border-color: currentColor;
	}

	/* Protocol */

	.messages {
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--bg-raised);
		overflow: hidden;
	}

	.legend {
		display: flex;
		gap: 22px;
		padding: 12px 18px;
		border-bottom: 1px solid var(--line);
		font-size: 12px;
		color: var(--ink-3);
	}

	.legend b {
		font-weight: 600;
		margin-right: 4px;
	}

	.messages ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.messages li {
		display: grid;
		grid-template-columns: 22px minmax(0, 1fr);
		gap: 4px 12px;
		align-items: baseline;
		padding: 14px 18px;
		border-top: 1px solid var(--line);
	}

	.messages li:first-child {
		border-top: 0;
	}

	@media (min-width: 900px) {
		.messages li {
			grid-template-columns: 22px 110px minmax(0, 0.9fr) minmax(0, 1.6fr);
		}
	}

	.messages .arrow {
		font-weight: 700;
		font-family: var(--font-mono);
	}

	.messages li.up .arrow {
		color: var(--human);
	}

	.messages li.down .arrow {
		color: var(--agent);
	}

	.type {
		font-weight: 600;
		font-size: 14px;
	}

	.meaning {
		grid-column: 2;
		color: var(--ink-2);
		font-size: 14.5px;
	}

	.example {
		grid-column: 2;
		padding: 6px 10px;
		background: var(--code-bg);
		color: var(--code-ink);
		border-radius: 6px;
		font-size: 12px;
		overflow-x: auto;
		white-space: nowrap;
		display: block;
	}

	@media (min-width: 900px) {
		.meaning {
			grid-column: 3;
		}

		.example {
			grid-column: 4;
		}
	}

	/* Deploy */

	.deploys {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 24px;
	}

	@media (min-width: 900px) {
		.deploys {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}

	.deploys article {
		display: flex;
		flex-direction: column;
		gap: 0;
		padding: 28px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--bg-raised);
	}

	.deploys article p {
		margin-bottom: 20px;
		flex: 1;
	}

	/* Stats */

	.stats {
		margin: 0;
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		border-top: 1px solid var(--ink);
	}

	@media (min-width: 900px) {
		.stats {
			grid-template-columns: repeat(4, minmax(0, 1fr));
		}
	}

	.stats > div {
		display: flex;
		flex-direction: column-reverse;
		justify-content: flex-end;
		gap: 10px;
		padding: 22px 16px 26px 0;
		border-bottom: 1px solid var(--line);
	}

	.stats dt {
		font-size: 14.5px;
		color: var(--ink);
		max-width: 22ch;
	}

	.stats dd {
		margin: 0;
		display: contents;
	}

	.value {
		order: 1;
		font-size: clamp(36px, 5vw, 56px);
		font-weight: 600;
		letter-spacing: -0.045em;
		line-height: 1;
		font-variant-numeric: tabular-nums;
	}

	.note {
		order: -1;
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-3);
	}

	.footnote {
		margin-top: 18px;
		font-size: 13.5px;
		color: var(--ink-3);
		max-width: 80ch;
	}

	.verified {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 24px;
		margin-top: 64px;
		padding: 28px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--bg-raised);
	}

	@media (min-width: 900px) {
		.verified {
			grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
			gap: 48px;
		}
	}

	.covered {
		display: flex;
		flex-wrap: wrap;
		align-content: flex-start;
		gap: 6px;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.covered li {
		padding: 3px 10px;
		border: 1px solid var(--line-strong);
		border-radius: 999px;
		font-size: 12px;
		color: var(--ink-2);
	}

	.covered li::before {
		content: '✓';
		margin-right: 6px;
		color: var(--ok);
	}

	/* Closing */

	.closing {
		padding: 120px 0 0;
		border-top: 1px solid var(--line);
		text-align: center;
	}

	.closing h2 {
		margin: 0 auto;
		max-width: 16ch;
		font-size: clamp(38px, 7vw, 72px);
		letter-spacing: -0.04em;
	}

	.closing .cta {
		justify-content: center;
		margin-top: 36px;
	}
</style>
