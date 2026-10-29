// The site's configuration, from the Worker's bindings, secrets and vars.
// A dynamic page with any of it missing answers 503 rather than half-work:
// sign-in and issuance never run on a guess.

import { error } from '@sveltejs/kit';
import { env as workerEnv } from 'cloudflare:workers';
import type { BridgeConfig } from './bridge.ts';
import type { GitHubConfig } from './github.ts';

export interface SiteConfig {
	db: D1Database;
	bridge: BridgeConfig;
	github: GitHubConfig;
	sessionKey: string;
	/** What an app is told to connect to: `wss://…/connect`. */
	bridgePublicUrl: string;
	/** The owner's numeric GitHub ID: the admin page opens for that identity alone (a login can change hands). */
	adminGithubId: string;
	/** Cookies are `Secure` except on a local http:// test server. */
	secureCookies: boolean;
	/** `TOKEN_CORS_LOOPBACK=1` (staging, tests): a wallet's local development server may exchange codes. */
	loopbackCors: boolean;
}

export function siteConfig(url: URL): SiteConfig {
	// Kit 3's Cloudflare adapter hands the bindings over as `cloudflare:workers`'s `env`.
	const env = workerEnv as unknown as Record<string, unknown> | undefined;
	const text = (name: string) => (typeof env?.[name] === 'string' ? (env[name] as string) : '');
	const db = env?.SITE_DB as D1Database | undefined;
	const binding = env?.BRIDGE as Fetcher | undefined;
	const missing = [
		!db && 'SITE_DB',
		!binding && 'BRIDGE',
		text('SERVICE_SECRET').length < 32 && 'SERVICE_SECRET',
		!text('GITHUB_CLIENT_ID') && 'GITHUB_CLIENT_ID',
		!text('GITHUB_CLIENT_SECRET') && 'GITHUB_CLIENT_SECRET',
		text('SESSION_KEY').length < 32 && 'SESSION_KEY',
		!text('BRIDGE_PUBLIC_URL') && 'BRIDGE_PUBLIC_URL'
	].filter(Boolean);
	if (missing.length) {
		console.error(`clientmcp.dev: not configured: ${missing.join(', ')}`);
		error(503, 'Sign-in is not available right now.');
	}
	return {
		db: db!,
		bridge: { binding: binding!, serviceSecret: text('SERVICE_SECRET') },
		github: {
			clientId: text('GITHUB_CLIENT_ID'),
			clientSecret: text('GITHUB_CLIENT_SECRET'),
			oauthBase: text('GITHUB_OAUTH_BASE') || 'https://github.com',
			apiBase: text('GITHUB_API_BASE') || 'https://api.github.com'
		},
		sessionKey: text('SESSION_KEY'),
		bridgePublicUrl: text('BRIDGE_PUBLIC_URL'),
		adminGithubId: text('ADMIN_GITHUB_ID'),
		secureCookies: url.protocol === 'https:',
		loopbackCors: text('TOKEN_CORS_LOOPBACK') === '1'
	};
}
