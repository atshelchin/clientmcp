import { error, json } from '@sveltejs/kit';
import { BridgeError, mcpUrl, mint } from '#lib/server/bridge.ts';
import { siteConfig } from '#lib/server/config.ts';
import { account as accountOf, client as clientOf } from '#lib/server/db.ts';
import { csrfValid, readSession } from '#lib/server/session.ts';
import type { RequestHandler } from './$types';

export const prerender = false;

/**
 * A credential for the account page's Copy buttons, fetched only when the
 * person copies it: the page itself never holds a token. The same credential
 * each time (the role's current generation), re-signed.
 *
 * JSON, so SvelteKit's form check does not apply: this checks the Origin and
 * the page's CSRF token itself. A client made by an app has no client token
 * to copy — the app holds it.
 */
export const POST: RequestHandler = async ({ url, cookies, request }) => {
	const config = siteConfig(url);
	if (request.headers.get('origin') !== url.origin) error(403, 'Cross-site request.');
	const session = await readSession(cookies, config.sessionKey);
	const acct = session && (await accountOf(config.db, session.account));
	if (!session || !acct) error(401, 'Sign in again.');
	if (acct.state !== 'active') error(403, 'This account is suspended.');
	const body = (await request.json().catch(() => ({}))) as Record<string, unknown>;
	if (
		!(await csrfValid(config.sessionKey, session, typeof body.csrf === 'string' ? body.csrf : null))
	) {
		error(403, 'This page has expired. Please reload it.');
	}
	const row = await clientOf(config.db, acct.id, String(body.client ?? ''));
	if (!row) error(404, 'No such client.');
	const role = body.role === 'client' ? 'client' : 'agent';
	if (role === 'client' && row.app)
		error(400, 'This client belongs to an app, which holds its own token.');
	try {
		const minted = await mint(config.bridge, row.id, [role], `account:${acct.id}`);
		const token = minted[role]!.token;
		return json(
			role === 'agent'
				? { token, serverUrl: `${mcpUrl(config.bridgePublicUrl).replace(/\/$/, '')}/${token}` }
				: { token },
			{ headers: { 'cache-control': 'no-store' } }
		);
	} catch (e) {
		if (e instanceof BridgeError)
			error(502, 'The bridge could not issue a token. Please try again.');
		throw e;
	}
};
