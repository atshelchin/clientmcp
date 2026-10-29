import { error, fail, redirect } from '@sveltejs/kit';
import { APPS, appRequest, clean, redirectWithCode, type AppRequest } from '#lib/server/apps.ts';
import { BridgeError, mint, mcpUrl } from '#lib/server/bridge.ts';
import { siteConfig, type SiteConfig } from '#lib/server/config.ts';
import { randomToken, sha256 } from '#lib/server/crypto.ts';
import {
	LimitReached,
	account as accountOf,
	audit,
	bindClient,
	clients as clientsOf,
	createClient,
	installationClient,
	saveCode,
	unbindInstallation
} from '#lib/server/db.ts';
import { csrfToken, csrfValid, readSession, type Session } from '#lib/server/session.ts';
import type { Actions, PageServerLoad } from './$types';

export const prerender = false;

/** The signed-in account, or off to sign in and back to exactly this page. */
async function signedIn(config: SiteConfig, cookies: import('@sveltejs/kit').Cookies, url: URL) {
	const session = await readSession(cookies, config.sessionKey);
	const acct = session && (await accountOf(config.db, session.account));
	if (!session || !acct)
		redirect(303, `/auth/github/start?next=${encodeURIComponent(url.pathname + url.search)}`);
	if (acct.state !== 'active') error(403, 'This account is suspended.');
	return { session, acct };
}

export const load: PageServerLoad = async ({ url, cookies }) => {
	const config = siteConfig(url);
	const fromApp = url.searchParams.has('app');
	// An app's request is checked before anything else: a bad redirect is
	// never followed, and nobody is asked to sign in for nothing.
	const request = fromApp ? appRequest(url.searchParams) : null;
	if (request && 'error' in request) error(400, request.error);
	const { session, acct } = await signedIn(config, cookies, url);
	// An app may connect as one of the person's clients, or a new one: the
	// one this installation already is comes first and is chosen.
	const choices = request
		? (await clientsOf(config.db, acct.id))
				.map((c) => ({
					id: c.id,
					label: c.label,
					app: c.app ? (APPS[c.app]?.name ?? c.app) : null,
					here: c.app === request.ok.app && c.installation === request.ok.installation,
					/** Another installation (or an application of the person's) holds it now. */
					held: c.installation !== null || c.app === null
				}))
				.sort((a, b) => Number(b.here) - Number(a.here))
		: [];
	return {
		login: acct.login,
		csrf: await csrfToken(config.sessionKey, session),
		app: request ? { name: request.ok.appName, label: request.ok.label } : null,
		choices
	};
};

/** The agent's credentials, shown on the page the person leaves behind. */
async function agentCredentials(config: SiteConfig, client: string, actor: string) {
	const minted = await mint(config.bridge, client, ['agent'], actor);
	return {
		mcpUrl: mcpUrl(config.bridgePublicUrl),
		agentToken: minted.agent!.token,
		exp: minted.exp
	};
}

async function checked(config: SiteConfig, session: Session, form: FormData) {
	if (!(await csrfValid(config.sessionKey, session, form.get('csrf')))) {
		error(403, 'This form has expired. Please reload the page.');
	}
}

export const actions: Actions = {
	/** The person lets the app connect: a code for the app, the agent's token for them. */
	approve: async ({ url, cookies, request }) => {
		const config = siteConfig(url);
		const { session, acct } = await signedIn(config, cookies, url);
		const form = await request.formData();
		await checked(config, session, form);
		const parsed = appRequest(url.searchParams);
		if ('error' in parsed) error(400, parsed.error);
		const app: AppRequest = parsed.ok;
		try {
			const existing = await installationClient(config.db, acct.id, app.app, app.installation);
			// Which client to connect as: this installation's (the default), another
			// of the person's, or a new one.
			const chosen = String(form.get('client') ?? existing?.id ?? 'new');
			let client: string;
			let rotateClient: boolean;
			if (chosen === 'new') {
				if (existing) await unbindInstallation(config.db, acct.id, app.app, app.installation);
				client = await createClient(config.db, acct, {
					app: app.app,
					installation: app.installation,
					label: app.label
				});
				rotateClient = false;
			} else if (chosen === existing?.id) {
				client = existing.id;
				// Signing in again from the same installation rotates its client token.
				rotateClient = true;
			} else {
				const other = (await clientsOf(config.db, acct.id)).find((c) => c.id === chosen);
				if (!other) error(404, 'No such client.');
				await bindClient(config.db, acct.id, other.id, app.app, app.installation);
				client = other.id;
				// Whatever held it before stops: its client token is replaced.
				rotateClient = true;
			}
			const code = randomToken(32);
			await saveCode(config.db, await sha256(code), {
				client_id: client,
				account_id: acct.id,
				app: app.app,
				redirect_uri: app.redirectUri,
				code_challenge: app.codeChallenge,
				rotate: rotateClient ? 1 : 0
			});
			await audit(config.db, acct.id, 'app-approved', `${client} ${app.app}`, 'person');
			return {
				done: {
					appName: app.appName,
					returnTo: redirectWithCode(app.redirectUri, code, app.state),
					...(await agentCredentials(config, client, `account:${acct.id}`))
				}
			};
		} catch (e) {
			if (e instanceof LimitReached) return fail(409, { limit: acct.client_limit });
			if (e instanceof BridgeError)
				error(502, 'The bridge could not issue a token. Please try again.');
			throw e;
		}
	},

	/** A client made by hand (no app): both tokens, to copy. */
	create: async ({ url, cookies, request }) => {
		const config = siteConfig(url);
		const { session, acct } = await signedIn(config, cookies, url);
		const form = await request.formData();
		await checked(config, session, form);
		const label = clean(form.get('label') as string | null, 'My client');
		try {
			const client = await createClient(config.db, acct, { app: null, installation: null, label });
			const minted = await mint(config.bridge, client, ['client', 'agent'], `account:${acct.id}`);
			return {
				created: {
					label,
					clientId: client,
					bridgeUrl: config.bridgePublicUrl,
					clientToken: minted.client!.token,
					mcpUrl: mcpUrl(config.bridgePublicUrl),
					agentToken: minted.agent!.token,
					exp: minted.exp
				}
			};
		} catch (e) {
			if (e instanceof LimitReached) return fail(409, { limit: acct.client_limit });
			if (e instanceof BridgeError)
				error(502, 'The bridge could not issue a token. Please try again.');
			throw e;
		}
	}
};
