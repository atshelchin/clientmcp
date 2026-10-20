<script lang="ts">
	// An illustration of ClientMCP, built the way the protocol asks applications to
	// be built: one set of actions, called by the UI (the visitor and the scripted
	// cursor) and by the agent alike. Nothing here talks to a real bridge.
	import { onMount, tick } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';

	type Actor = 'human' | 'agent';
	type Todo = { id: number; title: string; done: boolean; by?: Actor; touched: number };
	type Line =
		| { id: number; kind: 'prompt' | 'say'; text: string }
		| { id: number; kind: 'call'; name: string; args: string; risk?: string }
		| { id: number; kind: 'result'; text: string; tone: 'ok' | 'rejected' };
	type Packet = { seg: 'app' | 'agent'; to: 'app' | 'agent'; tone: Actor | 'ok'; key: number };
	type LineDraft = Line extends infer L ? (L extends Line ? Omit<L, 'id'> : never) : never;

	const HOP = 420;

	let todos = $state<Todo[]>([]);
	let lines = $state<Line[]>([]);
	let online = $state(false);
	let packet = $state<Packet | null>(null);
	let confirmText = $state<string | null>(null);
	let cursor = $state({ x: 0, y: 0, shown: false, pressing: false });
	let paused = $state(false);
	let still = $state(false);
	let draft = $state('');
	let decide = $state<((allow: boolean) => void) | null>(null);

	let root: HTMLElement;
	let appEl: HTMLElement;
	let logEl: HTMLElement;

	let counter = 0;
	const next = () => ++counter;
	let lastTodo = 0;

	// ── The application's state and actions: the only implementation ──
	const actions = {
		add(title: string, by: Actor) {
			const todo = { id: ++lastTodo, title, done: false, by, touched: next() };
			todos.push(todo);
			return { id: todo.id, title, done: false };
		},
		setDone(id: number, done: boolean, by: Actor) {
			const todo = todos.find((t) => t.id === id);
			if (!todo) throw new Error(`No todo with id ${id}`);
			Object.assign(todo, { done, by, touched: next() });
		},
		clearDone() {
			const removed = todos.filter((t) => t.done).length;
			todos = todos.filter((t) => !t.done);
			return removed;
		}
	};

	function reset() {
		counter = 0;
		lastTodo = 2;
		todos = [
			{ id: 1, title: 'Buy eggs', done: false, touched: 0 },
			{ id: 2, title: 'Book flights', done: false, touched: 0 }
		];
		lines = [];
		online = false;
		packet = null;
		confirmText = null;
		decide = null;
		cursor = { x: 0, y: 0, shown: false, pressing: false };
	}
	reset();

	// ── Timing: every wait honours pause, visibility and cancellation ──
	let run: AbortController | undefined;
	let inView = true;
	let wake: (() => void) | undefined;

	function resumeIfReady() {
		if (!paused && inView && !document.hidden) wake?.();
	}

	async function wait(ms: number) {
		if (still) return;
		const signal = run!.signal;
		await new Promise<void>((resolve, reject) => {
			const timer = setTimeout(resolve, ms);
			signal.addEventListener('abort', () => (clearTimeout(timer), reject(signal.reason)), {
				once: true
			});
		});
		while (paused || !inView || document.hidden) {
			await new Promise<void>((resolve, reject) => {
				wake = resolve;
				signal.addEventListener('abort', () => reject(signal.reason), { once: true });
			});
		}
	}

	function say(line: LineDraft) {
		lines.push({ ...line, id: next() });
	}

	async function hop(seg: Packet['seg'], to: Packet['to'], tone: Packet['tone']) {
		if (still) return;
		packet = { seg, to, tone, key: next() };
		await wait(HOP);
		packet = null;
	}

	/** A message crossing the bridge, in either direction. */
	async function flow(to: 'app' | 'agent', tone: Packet['tone']) {
		const [first, second]: Packet['seg'][] = to === 'app' ? ['agent', 'app'] : ['app', 'agent'];
		await hop(first, to, tone);
		await hop(second, to, tone);
	}

	async function pointAt(selector: string) {
		if (still) return false;
		const target = appEl?.querySelector(selector);
		if (!target) return false;
		const box = appEl.getBoundingClientRect();
		const r = target.getBoundingClientRect();
		if (!cursor.shown) {
			cursor = { x: box.width - 28, y: box.height - 20, shown: false, pressing: false };
			await tick();
			cursor.shown = true;
			await wait(60);
		}
		cursor.x = r.left - box.left + r.width / 2;
		cursor.y = r.top - box.top + r.height / 2;
		await wait(760);
		cursor.pressing = true;
		await wait(150);
		cursor.pressing = false;
		return true;
	}

	async function hideCursor() {
		cursor.shown = false;
		await wait(200);
	}

	// ── Confirmation: the application asks its user, the agent waits ──
	function ask(text: string) {
		confirmText = text;
		return new Promise<boolean>((resolve) => {
			decide = (allow) => {
				confirmText = null;
				decide = null;
				resolve(allow);
			};
		});
	}

	const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

	async function script() {
		reset();
		await wait(700);
		await hop('app', 'agent', 'ok');
		online = true;
		await wait(900);

		// The agent adds a todo; the user sees it appear.
		say({ kind: 'prompt', text: 'Add oat milk to my list' });
		await wait(900);
		say({ kind: 'call', name: 'add_todo', args: '{"title":"Oat milk"}' });
		await flow('app', 'agent');
		const added = actions.add('Oat milk', 'agent');
		await wait(300);
		await flow('agent', 'agent');
		say({ kind: 'result', tone: 'ok', text: `completed ${JSON.stringify({ id: added.id })}` });
		await wait(500);
		say({ kind: 'say', text: 'Added “Oat milk”.' });
		await wait(1500);

		// The user changes the state; nothing is sent, the state is simply shared.
		if (await pointAt('[data-title="Buy eggs"] .check')) {
			const eggs = todos.find((t) => t.title === 'Buy eggs');
			if (eggs && !eggs.done) actions.setDone(eggs.id, true, 'human');
			await wait(500);
			await hideCursor();
		} else if (still) {
			const eggs = todos.find((t) => t.title === 'Buy eggs');
			if (eggs) actions.setDone(eggs.id, true, 'human');
		}
		await wait(900);

		// The agent reads the same state.
		say({ kind: 'prompt', text: 'What’s still left?' });
		await wait(900);
		say({ kind: 'call', name: 'list_todos', args: '{}' });
		await flow('app', 'agent');
		await wait(200);
		const done = todos.filter((t) => t.done);
		const left = todos.filter((t) => !t.done);
		await flow('agent', 'agent');
		say({
			kind: 'result',
			tone: 'ok',
			text: `${plural(todos.length, 'todo')}, ${done.length} done`
		});
		await wait(500);
		say({
			kind: 'say',
			text:
				(done.length ? `${done.map((t) => t.title).join(', ')}: done. ` : '') +
				(left.length ? `Left: ${left.map((t) => t.title).join(', ')}.` : 'Nothing left.')
		});
		if (still) return;
		await wait(1800);

		// A consequential action: the application asks its user first.
		say({ kind: 'prompt', text: 'Clear the finished ones' });
		await wait(900);
		say({ kind: 'call', name: 'clear_done', args: '{}', risk: 'manage' });
		await flow('app', 'agent');
		const count = todos.filter((t) => t.done).length;
		const answer = ask(`An agent wants to remove ${plural(count, 'completed item')}.`);
		await wait(1300);
		if (decide && (await pointAt('.sheet .allow'))) decide?.(true);
		const allowed = await answer;
		await hideCursor();
		const removed = allowed ? actions.clearDone() : 0;
		await wait(350);
		await flow('agent', allowed ? 'agent' : 'human');
		if (allowed) {
			say({ kind: 'result', tone: 'ok', text: `completed ${JSON.stringify({ removed })}` });
			await wait(500);
			say({ kind: 'say', text: `Cleared ${plural(removed, 'item')}.` });
		} else {
			say({ kind: 'result', tone: 'rejected', text: 'rejected denied: the user said no' });
			await wait(500);
			say({ kind: 'say', text: 'You declined, so nothing was removed.' });
		}
		await wait(4200);
	}

	async function loop(controller: AbortController) {
		try {
			do await script();
			while (!still && !controller.signal.aborted);
		} catch {
			// Aborted: a replay or the component going away.
		}
	}

	function start() {
		run?.abort();
		const controller = (run = new AbortController());
		loop(controller);
	}

	function togglePause() {
		paused = !paused;
		resumeIfReady();
	}

	// ── The visitor is a user of the app too ──
	function toggle(todo: Todo) {
		actions.setDone(todo.id, !todo.done, 'human');
	}

	function submit(event: SubmitEvent) {
		event.preventDefault();
		const title = draft.trim();
		if (!title || todos.length >= 6) return;
		actions.add(title.slice(0, 40), 'human');
		draft = '';
	}

	$effect(() => {
		void lines.length;
		tick().then(() => {
			if (logEl) logEl.scrollTop = logEl.scrollHeight;
		});
	});

	onMount(() => {
		still = matchMedia('(prefers-reduced-motion: reduce)').matches;
		const observer = new IntersectionObserver(([entry]) => {
			inView = entry.isIntersecting;
			resumeIfReady();
		});
		observer.observe(root);
		const onVisibility = () => resumeIfReady();
		document.addEventListener('visibilitychange', onVisibility);
		start();
		return () => {
			run?.abort();
			observer.disconnect();
			document.removeEventListener('visibilitychange', onVisibility);
		};
	});
</script>

<figure class="demo" bind:this={root} aria-labelledby="demo-caption">
	<figcaption id="demo-caption" class="visually-hidden">
		Illustration: an agent adds “Oat milk” to a todo app through ClientMCP and it appears in the
		app; the user checks off “Buy eggs” and the agent sees it; the agent asks to clear finished
		items and the app asks the user to confirm.
	</figcaption>

	<!-- The application -->
	<section class="panel app" bind:this={appEl} aria-label="Your application">
		<header class="bar">
			<span class="who"><i class="dot human"></i>Your app</span>
			<span class="status" class:on={online}>
				{online ? 'online · 4 tools' : 'connecting…'}
			</span>
		</header>
		<div class="app-body">
			<h3>Groceries</h3>
			<form onsubmit={submit}>
				<input
					type="text"
					bind:value={draft}
					placeholder="Add a todo"
					aria-label="Add a todo"
					maxlength="40"
					autocomplete="off"
				/>
			</form>
			<ul>
				{#each todos as todo (todo.id)}
					<li
						class="item"
						data-title={todo.title}
						class:done={todo.done}
						animate:flip={{ duration: 250 }}
						in:fly={{ y: -6, duration: 260 }}
						out:fade={{ duration: 200 }}
					>
						{#key todo.touched}
							<span class="flash {todo.by ?? ''}" class:on={todo.touched > 0}></span>
						{/key}
						<button
							type="button"
							class="check"
							role="checkbox"
							aria-checked={todo.done}
							aria-label={todo.title}
							onclick={() => toggle(todo)}
						>
							<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>
						</button>
						<span class="title">{todo.title}</span>
						{#if todo.by}
							<span class="by {todo.by}">{todo.by === 'agent' ? 'agent' : 'you'}</span>
						{/if}
					</li>
				{/each}
			</ul>
		</div>

		{#if confirmText}
			<div
				class="sheet"
				transition:fly={{ y: 16, duration: 220 }}
				role="alertdialog"
				aria-label="Confirm"
			>
				<p><span class="risk">manage</span>{confirmText}</p>
				<div class="actions">
					<button type="button" class="deny" onclick={() => decide?.(false)}>Deny</button>
					<button type="button" class="allow" onclick={() => decide?.(true)}>Allow</button>
				</div>
			</div>
		{/if}

		<svg
			class="cursor"
			class:shown={cursor.shown}
			class:pressing={cursor.pressing}
			style:transform="translate({cursor.x}px, {cursor.y}px)"
			width="20"
			height="22"
			viewBox="0 0 20 22"
			aria-hidden="true"
		>
			<path d="M2 1.5v16l4.2-4 3 6.6 3-1.4-3-6.4H15z" />
		</svg>
	</section>

	<!-- The bridge -->
	<div class="wire" aria-hidden="true">
		<div class="seg seg-app">
			{#if packet?.seg === 'app'}
				{#key packet.key}<i class="packet to-{packet.to} {packet.tone}"></i>{/key}
			{/if}
		</div>
		<div class="node" class:busy={packet}>
			<span class="node-name">bridge</span>
		</div>
		<div class="seg seg-agent">
			{#if packet?.seg === 'agent'}
				{#key packet.key}<i class="packet to-{packet.to} {packet.tone}"></i>{/key}
			{/if}
		</div>
		<span class="proto proto-app">WebSocket</span>
		<span class="proto proto-agent">MCP</span>
	</div>

	<!-- The agent -->
	<section class="panel agent-panel" aria-label="An AI agent">
		<header class="bar">
			<span class="who"><i class="dot agent"></i>Agent</span>
			<span class="status mono">POST /mcp</span>
		</header>
		<ol class="log" bind:this={logEl}>
			{#each lines as line (line.id)}
				<li class="line {line.kind}" in:fly={{ y: 6, duration: 220 }}>
					{#if line.kind === 'prompt'}
						<span class="caret">›</span>{line.text}
					{:else if line.kind === 'call'}
						<span class="call-dot"></span><span class="fn">{line.name}</span><span class="args"
							>{line.args}</span
						>{#if line.risk}<span class="risk">{line.risk}</span>{/if}
					{:else if line.kind === 'result'}
						<span class="elbow">└</span><span class={line.tone}>{line.text}</span>
					{:else}
						{line.text}
					{/if}
				</li>
			{/each}
		</ol>
	</section>

	{#if !still}
		<div class="controls">
			<button type="button" onclick={togglePause} aria-pressed={paused}>
				{paused ? 'Play' : 'Pause'}
			</button>
			<button type="button" onclick={start}>Replay</button>
		</div>
	{/if}
</figure>

<style>
	.demo {
		position: relative;
		margin: 0;
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		grid-template-rows: auto 88px auto auto;
	}

	@media (min-width: 900px) {
		.demo {
			grid-template-columns: minmax(0, 1fr) 200px minmax(0, 1.12fr);
			grid-template-rows: auto auto;
		}
	}

	/* Panels */

	.panel {
		position: relative;
		overflow: hidden;
		background: var(--bg-raised);
		border: 1px solid var(--line);
		border-radius: 14px;
		box-shadow: var(--shadow);
		height: 330px;
		display: flex;
		flex-direction: column;
	}

	@media (min-width: 900px) {
		.panel {
			height: 372px;
		}
	}

	.bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		height: 42px;
		padding: 0 14px;
		border-bottom: 1px solid var(--line);
		font-size: 12.5px;
		flex: none;
	}

	.who {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		font-weight: 500;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
	}

	.dot.human {
		background: var(--human);
	}

	.dot.agent {
		background: var(--agent);
	}

	.status {
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-3);
	}

	.status.on {
		color: var(--ok);
	}

	.status.on::before {
		content: '';
		display: inline-block;
		width: 6px;
		height: 6px;
		margin-right: 6px;
		border-radius: 50%;
		background: currentColor;
		vertical-align: 1px;
	}

	/* The app */

	.app-body {
		padding: 16px 16px 8px;
		display: flex;
		flex-direction: column;
		min-height: 0;
		flex: 1;
	}

	h3 {
		font-size: 19px;
		letter-spacing: -0.02em;
	}

	form input {
		width: 100%;
		margin: 12px 0 6px;
		height: 38px;
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 8px;
		background: var(--bg);
		color: var(--ink);
		font: inherit;
		font-size: 16px;
	}

	@media (min-width: 900px) {
		form input {
			font-size: 14px;
		}
	}

	form input::placeholder {
		color: var(--ink-3);
	}

	form input:focus {
		outline: none;
		border-color: var(--human);
		box-shadow: 0 0 0 3px var(--human-soft);
	}

	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		overflow-y: auto;
		min-height: 0;
	}

	.item {
		position: relative;
		display: flex;
		align-items: center;
		gap: 11px;
		height: 44px;
		padding: 0 8px;
		border-bottom: 1px solid var(--line);
		font-size: 15px;
	}

	.item:last-child {
		border-bottom: 0;
	}

	.flash {
		position: absolute;
		inset: 3px 0;
		border-radius: 8px;
		pointer-events: none;
	}

	.flash.on.agent {
		animation: flash-agent 1.6s ease-out;
	}

	.flash.on.human {
		animation: flash-human 1.6s ease-out;
	}

	@keyframes flash-agent {
		0%,
		30% {
			background: var(--agent-soft);
		}
		100% {
			background: transparent;
		}
	}

	@keyframes flash-human {
		0%,
		30% {
			background: var(--human-soft);
		}
		100% {
			background: transparent;
		}
	}

	.check {
		position: relative;
		flex: none;
		width: 20px;
		height: 20px;
		padding: 0;
		border: 1.5px solid var(--line-strong);
		border-radius: 6px;
		background: var(--bg);
		cursor: pointer;
		display: grid;
		place-items: center;
		transition:
			background 0.15s,
			border-color 0.15s;
	}

	.check svg {
		width: 13px;
		height: 13px;
		fill: none;
		stroke: var(--bg);
		stroke-width: 2.2;
		stroke-linecap: round;
		stroke-linejoin: round;
		opacity: 0;
	}

	.item.done .check {
		background: var(--ink);
		border-color: var(--ink);
	}

	.item.done .check svg {
		opacity: 1;
	}

	.title {
		position: relative;
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.item.done .title {
		color: var(--ink-3);
		text-decoration: line-through;
	}

	.by {
		position: relative;
		font-family: var(--font-mono);
		font-size: 10.5px;
		padding: 2px 7px;
		border-radius: 999px;
	}

	.by.agent {
		color: var(--agent);
		background: var(--agent-soft);
	}

	.by.human {
		color: var(--human);
		background: var(--human-soft);
	}

	/* Confirmation sheet */

	.sheet {
		position: absolute;
		left: 10px;
		right: 10px;
		bottom: 10px;
		padding: 14px;
		background: var(--bg-raised);
		border: 1px solid var(--line-strong);
		border-radius: 12px;
		box-shadow: 0 12px 32px -12px rgb(0 0 0 / 0.35);
		font-size: 14px;
	}

	.sheet p {
		line-height: 1.45;
	}

	.risk {
		display: inline-block;
		margin-right: 8px;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid currentColor;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--human);
		vertical-align: 1px;
	}

	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 12px;
	}

	.actions button {
		height: 32px;
		padding: 0 14px;
		border-radius: 8px;
		border: 1px solid var(--line-strong);
		background: transparent;
		font-size: 13.5px;
		font-weight: 500;
		cursor: pointer;
	}

	.actions .allow {
		background: var(--ink);
		border-color: var(--ink);
		color: var(--bg);
	}

	/* Scripted cursor */

	.cursor {
		position: absolute;
		top: 0;
		left: 0;
		margin: -2px 0 0 -3px;
		pointer-events: none;
		opacity: 0;
		transition:
			transform 0.7s cubic-bezier(0.6, 0, 0.25, 1),
			opacity 0.2s;
		z-index: 3;
	}

	.cursor path {
		fill: var(--human);
		stroke: var(--bg-raised);
		stroke-width: 1.4;
		stroke-linejoin: round;
		transition: transform 0.12s;
		transform-origin: 2px 2px;
	}

	.cursor.shown {
		opacity: 1;
	}

	.cursor.pressing path {
		transform: scale(0.82);
	}

	/* The wire and the bridge */

	.wire {
		position: relative;
		display: grid;
		grid-template-rows: 1fr auto 1fr;
		justify-items: center;
	}

	@media (min-width: 900px) {
		.wire {
			grid-template-rows: none;
			grid-template-columns: 1fr auto 1fr;
			align-items: center;
			justify-items: stretch;
		}
	}

	.seg {
		position: relative;
		width: 0;
		border-left: 1.5px dashed var(--line-strong);
	}

	@media (min-width: 900px) {
		.seg {
			width: auto;
			height: 0;
			border-left: 0;
			border-top: 1.5px dashed var(--line-strong);
		}
	}

	.node {
		display: grid;
		place-items: center;
		padding: 6px 12px;
		border: 1px solid var(--line-strong);
		border-radius: 999px;
		background: var(--bg);
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-2);
		transition:
			border-color 0.2s,
			color 0.2s;
		z-index: 1;
	}

	.node.busy {
		border-color: var(--ink);
		color: var(--ink);
	}

	.proto {
		position: absolute;
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--ink-3);
		white-space: nowrap;
	}

	.proto-app {
		top: 10px;
		left: calc(50% + 12px);
	}

	.proto-agent {
		bottom: 10px;
		left: calc(50% + 12px);
	}

	@media (min-width: 900px) {
		.proto-app {
			top: calc(50% + 10px);
			left: 8px;
		}

		.proto-agent {
			top: calc(50% + 10px);
			bottom: auto;
			left: auto;
			right: 8px;
		}
	}

	.packet {
		position: absolute;
		width: 9px;
		height: 9px;
		border-radius: 50%;
		left: 0;
		top: 0;
		translate: -50% -50%;
		animation: run-v 420ms cubic-bezier(0.5, 0, 0.5, 1) forwards;
		margin-left: -0.75px;
	}

	.packet.to-app {
		animation-direction: reverse;
	}

	@media (min-width: 900px) {
		.packet {
			margin: -0.75px 0 0;
			animation-name: run-h;
		}
	}

	@keyframes run-v {
		from {
			top: 0;
		}
		to {
			top: 100%;
		}
	}

	@keyframes run-h {
		from {
			left: 0;
		}
		to {
			left: 100%;
		}
	}

	.packet.agent {
		background: var(--agent);
		box-shadow: 0 0 0 4px var(--agent-soft);
	}

	.packet.human {
		background: var(--human);
		box-shadow: 0 0 0 4px var(--human-soft);
	}

	.packet.ok {
		background: var(--ok);
		box-shadow: 0 0 0 4px color-mix(in oklab, var(--ok) 25%, transparent);
	}

	/* The agent */

	.log {
		list-style: none;
		margin: 0;
		padding: 14px 16px 18px;
		flex: 1;
		min-height: 0;
		overflow: hidden;
		display: flex;
		flex-direction: column;
		gap: 7px;
		font-family: var(--font-mono);
		font-size: 12.5px;
		line-height: 1.5;
		mask-image: linear-gradient(to bottom, transparent 0, #000 36px);
	}

	.log > :first-child {
		margin-top: auto;
	}

	.line {
		overflow-wrap: anywhere;
	}

	.line.prompt {
		margin-top: 10px;
		color: var(--ink);
		font-family: var(--font-sans);
		font-size: 14px;
		font-weight: 500;
	}

	.line.prompt:first-child {
		margin-top: auto;
	}

	.caret {
		color: var(--ink-3);
		margin-right: 8px;
	}

	.line.call {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0 6px;
	}

	.call-dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--agent);
		margin-right: 2px;
	}

	.fn {
		color: var(--agent);
		font-weight: 500;
	}

	.args {
		color: var(--ink-2);
	}

	.line.call .risk {
		margin: 0 0 0 4px;
	}

	.line.result {
		color: var(--ink-3);
		padding-left: 4px;
	}

	.elbow {
		margin-right: 8px;
	}

	.result .ok {
		color: var(--ok);
	}

	.result .rejected {
		color: var(--human);
	}

	.line.say {
		color: var(--ink-2);
		font-family: var(--font-sans);
		font-size: 14px;
	}

	/* Controls */

	.controls {
		display: flex;
		justify-content: flex-end;
		gap: 4px;
		margin-top: 10px;
	}

	@media (min-width: 900px) {
		.controls {
			grid-column: 1 / -1;
		}
	}

	.controls button {
		padding: 4px 10px;
		border: 0;
		border-radius: 999px;
		background: transparent;
		color: var(--ink-3);
		font-family: var(--font-mono);
		font-size: 11.5px;
		cursor: pointer;
	}

	.controls button:hover {
		color: var(--ink);
		background: var(--bg-sunken);
	}
</style>
