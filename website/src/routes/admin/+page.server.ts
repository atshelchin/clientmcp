import { error, fail, redirect } from '@sveltejs/kit';
import { APPS } from '#lib/server/apps.ts';
import {
	BridgeError,
	issuanceAudit,
	remove,
	revoke,
	usage,
	usageDays,
	type UsageDay
} from '#lib/server/bridge.ts';
import { siteConfig, type SiteConfig } from '#lib/server/config.ts';
import {
	adminAccounts,
	adminClients,
	anyClient,
	audit,
	deleteClient,
	isAdmin,
	recentAudit,
	setAccountState,
	setClientLimit
} from '#lib/server/db.ts';
import { csrfToken, csrfValid, readSession } from '#lib/server/session.ts';
import type { Actions, PageServerLoad } from './$types';

export const prerender = false;

/** The owner, or a 404 as for any page that is not there. */
async function admin(config: SiteConfig, cookies: import('@sveltejs/kit').Cookies) {
	const session = await readSession(cookies, config.sessionKey);
	if (!session) redirect(303, `/auth/github/start?next=${encodeURIComponent('/admin')}`);
	if (!(await isAdmin(config.db, session.account, config.adminGithubId))) error(404, 'Not found');
	return session;
}

const calls = (d: UsageDay) =>
	d.calls_ok +
	d.calls_tool_error +
	d.calls_offline +
	d.calls_timeout +
	d.calls_cancelled +
	d.calls_disconnected +
	d.calls_refused +
	d.calls_limited;

interface Totals {
	calls: number;
	failed: number;
	limited: number;
	bytes: number;
	connectedHours: number;
	rejections: number;
}

const empty = (): Totals => ({
	calls: 0,
	failed: 0,
	limited: 0,
	bytes: 0,
	connectedHours: 0,
	rejections: 0
});

function add(t: Totals, d: UsageDay) {
	t.calls += calls(d);
	t.failed += calls(d) - d.calls_ok;
	t.limited += d.calls_limited;
	t.bytes += d.bytes_in + d.bytes_out;
	t.connectedHours += d.connected_ms / 3_600_000;
	t.rejections += d.rejections;
}

const today = () => new Date().toISOString().slice(0, 10);
/** Live counts are asked of each client's object: at most this many per page view. */
const LIVE_CLIENTS = 300;
const DAYS = 30;

/** The `days` days before today, oldest first (UTC). */
function lastDays(days: number): string[] {
	const now = Date.now();
	return Array.from({ length: days }, (_, i) =>
		new Date(now - (days - i) * 86_400_000).toISOString().slice(0, 10)
	);
}

/** The value below which `share` of `sorted` (ascending) falls. */
function percentile(sorted: number[], share: number): number {
	if (!sorted.length) return 0;
	return sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))];
}

function spread(values: number[]) {
	const sorted = [...values].sort((a, b) => a - b);
	return {
		median: percentile(sorted, 0.5),
		p90: percentile(sorted, 0.9),
		max: sorted.at(-1) ?? 0
	};
}

const at = (seconds: number) => new Date(seconds * 1000).toISOString();

export const load: PageServerLoad = async ({ url, cookies }) => {
	const config = siteConfig(url);
	const session = await admin(config, cookies);
	const [accounts, clients, siteAudit] = await Promise.all([
		adminAccounts(config.db),
		adminClients(config.db),
		recentAudit(config.db, 50)
	]);
	const [finished, issuance] = await Promise.all([
		usageDays(config.bridge, DAYS).catch(() => null),
		issuanceAudit(config.bridge, 50).catch(() => null)
	]);
	const live = await Promise.all(
		clients.slice(0, LIVE_CLIENTS).map((c) =>
			usage(config.bridge, c.id).then(
				(days) => [c.id, days.find((d) => d.day === today())] as const,
				() => [c.id, undefined] as const
			)
		)
	);
	const clientById = new Map(clients.map((c) => [c.id, c]));
	const loginOf = new Map(accounts.map((a) => [a.id, a.login]));
	/** `account:a-…` / `admin:a-…` as a person: `@login`, or `admin @login`. */
	const who = (actor: string) =>
		actor.replace(/^(account|admin):(a-\w+)$/, (whole, kind: string, id: string) => {
			const login = loginOf.get(id);
			if (!login) return whole;
			return kind === 'admin' ? `admin @${login}` : `@${login}`;
		});

	const tally = (map: Map<string, Totals>, key: string, day: UsageDay) => {
		const t = map.get(key) ?? empty();
		add(t, day);
		map.set(key, t);
	};

	const todayTotals = empty();
	const todayByAccount = new Map<string, Totals>();
	const todayByClient = new Map<string, Totals>();
	for (const [client, day] of live) {
		if (!day) continue;
		add(todayTotals, day);
		tally(todayByAccount, clientById.get(client)!.account_id, day);
		tally(todayByClient, client, day);
	}

	const days = lastDays(DAYS);
	const byDay = new Map(days.map((d) => [d, empty()]));
	const monthTotals = empty();
	const monthByAccount = new Map<string, Totals>();
	const monthByClient = new Map<string, Totals>();
	/** One account's one day: what a plan would have to cover. */
	const accountDays = new Map<string, Totals>();
	for (const row of finished ?? []) {
		add(monthTotals, row);
		const series = byDay.get(row.day);
		if (series) add(series, row);
		tally(monthByClient, row.client, row);
		const account = clientById.get(row.client)?.account_id;
		if (!account) continue;
		tally(monthByAccount, account, row);
		tally(accountDays, `${account} ${row.day}`, row);
	}
	const active = [...accountDays.values()].filter((t) => t.calls > 0 || t.connectedHours > 0);
	const activeLastWeek = new Set(
		[...accountDays.entries()]
			.filter(([key, t]) => key.split(' ')[1] >= days[DAYS - 7] && (t.calls || t.connectedHours))
			.map(([key]) => key.split(' ')[0])
	);
	for (const id of todayByAccount.keys()) activeLastWeek.add(id);

	return {
		csrf: await csrfToken(config.sessionKey, session),
		self: session.account,
		now: new Date().toISOString(),
		bridgeRecords: finished !== null,
		totals: {
			accounts: accounts.length,
			suspended: accounts.filter((a) => a.state !== 'active').length,
			activeLastWeek: activeLastWeek.size,
			clients: clients.length,
			today: todayTotals,
			month: monthTotals
		},
		series: days.map((day) => ({ day, ...byDay.get(day)! })),
		sizing: {
			accountDays: active.length,
			calls: spread(active.map((t) => t.calls)),
			connectedHours: spread(active.map((t) => t.connectedHours)),
			megabytes: spread(active.map((t) => t.bytes / 1_000_000))
		},
		accounts: accounts.map((a) => ({
			...a,
			created: at(a.created_at),
			lastSeen: at(a.last_seen_at),
			today: todayByAccount.get(a.id) ?? empty(),
			month: monthByAccount.get(a.id) ?? empty(),
			clientList: clients
				.filter((c) => c.account_id === a.id)
				.map((c) => ({
					id: c.id,
					label: c.label,
					app: c.app ? (APPS[c.app]?.name ?? c.app) : null,
					created: at(c.created_at),
					today: todayByClient.get(c.id) ?? empty(),
					month: monthByClient.get(c.id) ?? empty()
				}))
		})),
		audit: siteAudit.map((r) => ({
			at: at(r.at),
			action: r.action,
			login: r.account_id ? (loginOf.get(r.account_id) ?? null) : null,
			detail: r.detail,
			actor: who(r.actor)
		})),
		issuance: (issuance ?? []).map((r) => {
			const c = clientById.get(r.client);
			return {
				at: new Date(r.at).toISOString(),
				action: r.action,
				role: r.role,
				gen: r.gen,
				client: r.client,
				label: c?.label ?? null,
				login: c ? (loginOf.get(c.account_id) ?? null) : null,
				actor: who(r.actor)
			};
		})
	};
};

async function checked(url: URL, cookies: import('@sveltejs/kit').Cookies, request: Request) {
	const config = siteConfig(url);
	const session = await admin(config, cookies);
	const form = await request.formData();
	if (!(await csrfValid(config.sessionKey, session, form.get('csrf')))) {
		error(403, 'This form has expired. Please reload the page.');
	}
	return { config, session, form, actor: `admin:${session.account}` };
}

/** Revokes, tolerating a client the bridge already deleted. */
async function revokeAll(config: SiteConfig, client: string, actor: string) {
	await revoke(config.bridge, client, 'both', actor).catch((e) => {
		if (!(e instanceof BridgeError && e.status === 410)) throw e;
	});
}

export const actions: Actions = {
	/** Every client's tokens stop, and the account can mint no more. */
	suspend: async ({ url, cookies, request }) => {
		const { config, session, form, actor } = await checked(url, cookies, request);
		const account = String(form.get('account') ?? '');
		if (account === session.account)
			return fail(400, { message: 'You cannot suspend your own account.' });
		const clients = (await adminClients(config.db)).filter((c) => c.account_id === account);
		for (const c of clients) await revokeAll(config, c.id, actor);
		await setAccountState(config.db, account, 'suspended', actor);
		return { message: `Suspended; ${clients.length} client(s) revoked.` };
	},

	/** The account works again. Its clients need new tokens: copied from its account page, or from its app. */
	reactivate: async ({ url, cookies, request }) => {
		const { config, form, actor } = await checked(url, cookies, request);
		await setAccountState(config.db, String(form.get('account') ?? ''), 'active', actor);
		return { message: 'Reactivated. Its clients need new tokens.' };
	},

	setLimit: async ({ url, cookies, request }) => {
		const { config, form, actor } = await checked(url, cookies, request);
		const limit = Number(form.get('limit'));
		if (!Number.isInteger(limit) || limit < 0 || limit > 1000)
			return fail(400, { message: 'A limit is 0–1000.' });
		await setClientLimit(config.db, String(form.get('account') ?? ''), limit, actor);
		return { message: `Limit set to ${limit}.` };
	},

	revokeClient: async ({ url, cookies, request }) => {
		const { config, form, actor } = await checked(url, cookies, request);
		const client = await anyClient(config.db, String(form.get('client') ?? ''));
		if (!client) error(404, 'No such client.');
		await revokeAll(config, client.id, actor);
		await audit(config.db, client.account_id, 'client-revoked', client.id, actor);
		return { message: `${client.label}: both tokens revoked.` };
	},

	deleteClient: async ({ url, cookies, request }) => {
		const { config, form, actor } = await checked(url, cookies, request);
		const client = await anyClient(config.db, String(form.get('client') ?? ''));
		if (!client) error(404, 'No such client.');
		await remove(config.bridge, client.id, actor).catch((e) => {
			if (!(e instanceof BridgeError && e.status === 410)) throw e;
		});
		await deleteClient(config.db, client.account_id, client.id);
		return { message: `${client.label} deleted.` };
	}
};
