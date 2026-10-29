import { json } from '@sveltejs/kit';
import { corsOrigin } from '#lib/server/apps.ts';
import { BridgeError, mint, rotate, mcpUrl } from '#lib/server/bridge.ts';
import { siteConfig } from '#lib/server/config.ts';
import { pkceMatches, sha256 } from '#lib/server/crypto.ts';
import { account as accountOf, audit, takeCode } from '#lib/server/db.ts';
import type { RequestHandler } from './$types';

export const prerender = false;

const NO_STORE = { 'cache-control': 'no-store', pragma: 'no-cache' };

/**
 * A registered web app's page exchanges its code from the browser: CORS for
 * its origin alone, without credentials (the code and verifier are the proof).
 */
function cors(request: Request, loopback: boolean): Record<string, string> {
	const origin = corsOrigin(request.headers.get('origin'), loopback);
	return origin ? { 'access-control-allow-origin': origin, vary: 'origin' } : { vary: 'origin' };
}

export const OPTIONS: RequestHandler = async ({ url, request }) => {
	const headers = cors(request, siteConfig(url).loopbackCors);
	if (!headers['access-control-allow-origin']) return new Response(null, { status: 403, headers });
	return new Response(null, {
		status: 204,
		headers: {
			...headers,
			'access-control-allow-methods': 'POST',
			'access-control-allow-headers': 'content-type',
			'access-control-max-age': '600'
		}
	});
};

/**
 * The app's half of the PKCE exchange (RFC 7636): the code from the redirect
 * and the verifier only the app holds, for the client token. The code is
 * used up by the statement that reads it, so a second attempt — or one with
 * the wrong verifier — finds nothing to try again with.
 *
 * The body is JSON. (Form-encoded bodies, RFC 6749's usual, are refused by
 * SvelteKit's cross-site check before this runs: a native app sends no
 * `Origin`. Our apps send JSON.)
 */
export const POST: RequestHandler = async ({ url, request }) => {
	const config = siteConfig(url);
	const headers = { ...NO_STORE, ...cors(request, config.loopbackCors) };
	/** RFC 6749 §5.2. */
	const refuse = (error: string, description: string, status = 400) =>
		json({ error, error_description: description }, { status, headers });
	const type = request.headers.get('content-type') ?? '';
	const raw: unknown = type.includes('application/json')
		? await request.json().catch(() => ({}))
		: Object.fromEntries(new URLSearchParams(await request.text()));
	const params: Record<string, string> = {};
	if (raw && typeof raw === 'object') {
		for (const [key, value] of Object.entries(raw))
			if (typeof value === 'string') params[key] = value;
	}
	if (params.grant_type !== 'authorization_code') {
		return refuse('unsupported_grant_type', 'Only authorization_code is supported.');
	}
	const code = params.code ?? '';
	const verifier = params.code_verifier ?? '';
	const redirectUri = params.redirect_uri ?? '';
	if (!code || !verifier || !redirectUri)
		return refuse('invalid_request', 'code, code_verifier and redirect_uri are required.');

	const row = await takeCode(config.db, await sha256(code));
	if (!row) return refuse('invalid_grant', 'The code is unknown, used or expired.');
	if (row.redirect_uri !== redirectUri || !(await pkceMatches(verifier, row.code_challenge))) {
		await audit(config.db, row.account_id, 'token-refused', row.client_id, 'app');
		return refuse('invalid_grant', 'The code does not match this request.');
	}
	const acct = await accountOf(config.db, row.account_id);
	if (!acct || acct.state !== 'active')
		return refuse('invalid_grant', 'The account is not active.');

	try {
		const actor = `account:${acct.id}`;
		// The same installation signing in again: its old client token stops.
		const minted = row.rotate
			? await rotate(config.bridge, row.client_id, 'client', actor)
			: await mint(config.bridge, row.client_id, ['client'], actor);
		// The agent's address too, so the app can hand it to the person's AI
		// agent itself: the same one the account page copies (the role's
		// current credential, shown again, not issued anew).
		const agent = await mint(config.bridge, row.client_id, ['agent'], actor);
		const mcp = mcpUrl(config.bridgePublicUrl);
		await audit(config.db, acct.id, 'token-exchanged', `${row.client_id} ${row.app}`, 'app');
		return json(
			{
				token_type: 'clientmcp',
				client_id: row.client_id,
				client_token: minted.client!.token,
				expires_at: minted.exp,
				bridge_url: config.bridgePublicUrl,
				mcp_url: mcp,
				/** The MCP server address for the person's AI agent, its token in the path (SPEC §4). */
				agent_url: `${mcp.replace(/\/$/, '')}/${agent.agent!.token}`,
				login: acct.login
			},
			{ headers }
		);
	} catch (e) {
		if (e instanceof BridgeError)
			return refuse('temporarily_unavailable', 'The bridge could not issue a token.', 503);
		throw e;
	}
};
