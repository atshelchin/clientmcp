// The security pieces of self-serve sign-in (docs/self-serve.md): signed
// values, PKCE, which apps may receive a code, and where sign-in may return.
import { describe, expect, it } from 'vitest';
import { appRequest, clean, corsOrigin, redirectWithCode } from './apps.ts';
import { mcpUrl } from './bridge.ts';
import {
	constantTimeEqual,
	isCodeVerifier,
	newClientId,
	pkceMatches,
	sign,
	verify
} from './crypto.ts';
import { safeNext } from './session.ts';

const KEY = 'k'.repeat(40);

describe('signed values', () => {
	it('read back for their purpose until they expire', async () => {
		const signed = await sign(KEY, 'session', { account: 'a-1' }, 1_000);
		expect(await verify(KEY, 'session', signed, 999)).toEqual({ account: 'a-1' });
		expect(await verify(KEY, 'session', signed, 1_000)).toBeNull();
		expect(await verify(KEY, 'oauth', signed, 0)).toBeNull();
		expect(await verify('x'.repeat(40), 'session', signed, 0)).toBeNull();
	});

	it('cannot be altered', async () => {
		const signed = await sign(KEY, 'session', { account: 'a-1' }, 1_000);
		const [payload, mac] = signed.split('.');
		const forged = btoa(JSON.stringify({ p: 'session', v: { account: 'a-2' }, e: 1_000 }))
			.replace(/\+/g, '-')
			.replace(/\//g, '_')
			.replace(/=+$/, '');
		expect(await verify(KEY, 'session', `${forged}.${mac}`, 0)).toBeNull();
		expect(await verify(KEY, 'session', `${payload}.${mac.slice(1)}x`, 0)).toBeNull();
		expect(await verify(KEY, 'session', 'garbage', 0)).toBeNull();
	});

	it('compares in constant time, and only equal strings are equal', () => {
		expect(constantTimeEqual('abc', 'abc')).toBe(true);
		expect(constantTimeEqual('abc', 'abd')).toBe(false);
		expect(constantTimeEqual('abc', 'ab')).toBe(false);
	});
});

describe('PKCE (RFC 7636)', () => {
	// RFC 7636, Appendix B.
	const verifier = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
	const challenge = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';

	it('accepts the RFC example and nothing else', async () => {
		expect(await pkceMatches(verifier, challenge)).toBe(true);
		expect(await pkceMatches(verifier.slice(0, -1) + 'x', challenge)).toBe(false);
		expect(await pkceMatches(challenge, challenge)).toBe(false);
		expect(await pkceMatches('short', challenge)).toBe(false);
	});

	it('knows a verifier when it sees one', () => {
		expect(isCodeVerifier(verifier)).toBe(true);
		expect(isCodeVerifier('a'.repeat(42))).toBe(false);
		expect(isCodeVerifier('a'.repeat(129))).toBe(false);
		expect(isCodeVerifier('a'.repeat(43) + ' ')).toBe(false);
	});
});

describe('app requests', () => {
	const good = {
		app: 'vela',
		redirect_uri: 'app.getvela.wallet:/clientmcp',
		state: 'state-12345678',
		code_challenge: 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM',
		code_challenge_method: 'S256',
		label: 'iPhone',
		installation: 'install-abcdef12'
	};
	const params = (over: Record<string, string | null> = {}) => {
		const p = new URLSearchParams();
		for (const [k, v] of Object.entries({ ...good, ...over })) if (v !== null) p.set(k, v);
		return p;
	};

	it('accepts a registered app at its exact address', () => {
		const result = appRequest(params());
		expect('ok' in result && result.ok.appName).toBe('Vela');
	});

	it('accepts the web wallet and a desktop loopback on any port', () => {
		for (const redirect_uri of [
			'https://wallet.getvela.app/clientmcp',
			'http://127.0.0.1:1/clientmcp',
			'http://127.0.0.1:53682/clientmcp',
			'http://127.0.0.1:65535/clientmcp'
		]) {
			expect('ok' in appRequest(params({ redirect_uri })), redirect_uri).toBe(true);
		}
	});

	it('never accepts another return address', () => {
		for (const redirect_uri of [
			'app.getvela.wallet:/clientmcp/',
			'app.getvela.wallet:/clientmcp?x=1',
			'app.getvela.wallet://clientmcp',
			'https://evil.example/clientmcp',
			'APP.getvela.wallet:/clientmcp',
			'https://wallet.getvela.app/clientmcp/',
			'https://wallet.getvela.app.evil.example/clientmcp',
			'http://wallet.getvela.app/clientmcp',
			'http://127.0.0.1:53682/clientmcp/',
			'http://127.0.0.1:53682/clientmcp?x=1',
			'http://127.0.0.1:53682/other',
			'http://127.0.0.1:65536/clientmcp',
			'http://127.0.0.1:0/clientmcp',
			'http://127.0.0.1/clientmcp',
			'http://localhost:53682/clientmcp',
			'http://127.0.0.1.evil.example:53682/clientmcp',
			'https://127.0.0.1:53682/clientmcp',
			'http://user@127.0.0.1:53682/clientmcp'
		]) {
			expect('error' in appRequest(params({ redirect_uri })), redirect_uri).toBe(true);
		}
	});

	it('requires S256 PKCE, a state and an installation, and a known app', () => {
		expect('error' in appRequest(params({ code_challenge_method: 'plain' }))).toBe(true);
		expect('error' in appRequest(params({ code_challenge: null }))).toBe(true);
		expect('error' in appRequest(params({ state: 'short' }))).toBe(true);
		expect('error' in appRequest(params({ installation: null }))).toBe(true);
		expect('error' in appRequest(params({ app: 'other' }))).toBe(true);
	});

	it('allows CORS for the web wallet, and loopback origins only where asked', () => {
		expect(corsOrigin('https://wallet.getvela.app', false)).toBe('https://wallet.getvela.app');
		expect(corsOrigin('https://evil.example', true)).toBeNull();
		expect(corsOrigin('https://wallet.getvela.app.evil.example', true)).toBeNull();
		expect(corsOrigin(null, true)).toBeNull();
		expect(corsOrigin('http://127.0.0.1:5173', false)).toBeNull();
		expect(corsOrigin('http://127.0.0.1:5173', true)).toBe('http://127.0.0.1:5173');
		expect(corsOrigin('http://localhost:5173', true)).toBe('http://localhost:5173');
		expect(corsOrigin('http://localhost:5173.evil.example', true)).toBeNull();
	});

	it('shows a label as text, briefly', () => {
		expect(clean('<b>My\u0007 phone</b>', 'x')).toBe('bMy phone/b');
		expect(clean('', 'Vela')).toBe('Vela');
		expect(clean('a'.repeat(60), 'x')).toHaveLength(40);
	});

	it('adds the code and state to the return address', () => {
		expect(redirectWithCode('app.getvela.wallet:/clientmcp', 'c/1', 's&1')).toBe(
			'app.getvela.wallet:/clientmcp?code=c%2F1&state=s%261'
		);
	});
});

describe('after sign-in', () => {
	it('returns only to a path on this site', () => {
		expect(safeNext('/connect?app=vela')).toBe('/connect?app=vela');
		for (const next of [
			null,
			'',
			'https://evil.example',
			'//evil.example',
			'/\\evil.example',
			'connect'
		]) {
			expect(safeNext(next)).toBe('/account');
		}
	});
});

describe('identifiers and addresses', () => {
	it('client IDs are opaque and within the protocol', () => {
		const id = newClientId();
		expect(id).toMatch(/^c-[a-z2-7]{20}$/);
		expect(newClientId()).not.toBe(id);
	});

	it('the agent endpoint follows from the bridge address', () => {
		expect(mcpUrl('wss://bridge.clientmcp.dev/connect')).toBe('https://bridge.clientmcp.dev/mcp');
		expect(mcpUrl('ws://127.0.0.1:8787/connect')).toBe('http://127.0.0.1:8787/mcp');
	});
});
