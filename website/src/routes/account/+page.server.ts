import { error, redirect } from '@sveltejs/kit';
import { APPS } from '#lib/server/apps.ts';
import { BridgeError, remove, usage, type UsageDay, mcpUrl } from '#lib/server/bridge.ts';
import { siteConfig, type SiteConfig } from '#lib/server/config.ts';
import {
	account as accountOf,
	client as clientOf,
	clients,
	deleteClient,
	isAdmin
} from '#lib/server/db.ts';
import { csrfToken, csrfValid, readSession } from '#lib/server/session.ts';
import type { Actions, PageServerLoad } from './$types';

export const prerender = false;

async function signedIn(config: SiteConfig, cookies: import('@sveltejs/kit').Cookies) {
	const session = await readSession(cookies, config.sessionKey);
	const acct = session && (await accountOf(config.db, session.account));
	if (!session || !acct) redirect(303, `/auth/github/start?next=${encodeURIComponent('/account')}`);
	return { session, acct };
}

const today = () => new Date().toISOString().slice(0, 10);

function summary(days: UsageDay[]) {
	const d = days.find((day) => day.day === today());
	if (!d) return { calls: 0, failed: 0, connectedMinutes: 0 };
	const calls =
		d.calls_ok +
		d.calls_tool_error +
		d.calls_offline +
		d.calls_timeout +
		d.calls_cancelled +
		d.calls_disconnected +
		d.calls_refused +
		d.calls_limited;
	return {
		calls,
		failed: calls - d.calls_ok,
		connectedMinutes: Math.round(d.connected_ms / 60000)
	};
}

export const load: PageServerLoad = async ({ url, cookies }) => {
	const config = siteConfig(url);
	const { session, acct } = await signedIn(config, cookies);
	const rows = await clients(config.db, acct.id);
	const listed = await Promise.all(
		rows.map(async (row) => ({
			id: row.id,
			label: row.label,
			app: row.app ? (APPS[row.app]?.name ?? row.app) : null,
			created: new Date(row.created_at * 1000).toISOString().slice(0, 10),
			today: await usage(config.bridge, row.id).then(summary, () => null)
		}))
	);
	return {
		login: acct.login,
		avatar: acct.avatar_url,
		suspended: acct.state !== 'active',
		limit: acct.client_limit,
		csrf: await csrfToken(config.sessionKey, session),
		bridgeUrl: config.bridgePublicUrl,
		mcpUrl: mcpUrl(config.bridgePublicUrl),
		admin: await isAdmin(config.db, acct.id, config.adminGithubId),
		clients: listed
	};
};

/** The form's client, if it is this account's. */
async function owned(
	config: SiteConfig,
	cookies: import('@sveltejs/kit').Cookies,
	url: URL,
	request: Request
) {
	const { session, acct } = await signedIn(config, cookies);
	const form = await request.formData();
	if (!(await csrfValid(config.sessionKey, session, form.get('csrf')))) {
		error(403, 'This form has expired. Please reload the page.');
	}
	if (acct.state !== 'active') error(403, 'This account is suspended.');
	const row = await clientOf(config.db, acct.id, String(form.get('client') ?? ''));
	if (!row) error(404, 'No such client.');
	return { acct, row, form, actor: `account:${acct.id}` };
}

function bridgeFailed(e: unknown): never {
	if (e instanceof BridgeError) error(502, 'The bridge could not do that. Please try again.');
	throw e;
}

export const actions: Actions = {
	/** The client is gone: both tokens stop, at once, for good. */
	delete: async ({ url, cookies, request }) => {
		const config = siteConfig(url);
		const { acct, row, actor } = await owned(config, cookies, url, request);
		await remove(config.bridge, row.id, actor).catch(bridgeFailed);
		await deleteClient(config.db, acct.id, row.id);
		return { done: `${row.label} is deleted.` };
	}
};
