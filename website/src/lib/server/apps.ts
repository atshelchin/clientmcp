// The applications that may ask clientmcp.dev for a token on a person's
// behalf (docs/self-serve.md, "Returning the token to the app"). Only the
// owner adds one. A redirect URI is compared exactly: no prefix, no added
// query, no other scheme — a code is only ever handed to an address listed
// here.

export interface App {
	/** Shown to the person: "Let <name> on <label> connect…". */
	name: string;
	redirectUris: readonly string[];
	/**
	 * A desktop app's loopback return (RFC 8252 §7.3): `http://127.0.0.1:<port><path>`,
	 * any port, this path exactly.
	 */
	loopbackPath?: string;
	/** Web origins whose pages may exchange a code from the browser (CORS on /api/token). */
	webOrigins?: readonly string[];
}

export const APPS: Readonly<Record<string, App>> = {
	vela: {
		name: 'Vela',
		redirectUris: [
			// iOS and Android: the private-use scheme (RFC 8252 §7.1).
			'app.getvela.wallet:/clientmcp',
			// The web wallet.
			'https://wallet.getvela.app/clientmcp'
		],
		// The desktop.
		loopbackPath: '/clientmcp',
		webOrigins: ['https://wallet.getvela.app']
	}
};

const LOOPBACK = /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})(\/[a-z0-9/-]*)$/;

/** Whether `app` may be sent back to `redirectUri`. */
export function returnsTo(app: App, redirectUri: string): boolean {
	if (app.redirectUris.includes(redirectUri)) return true;
	const loopback = LOOPBACK.exec(redirectUri);
	return (
		!!loopback &&
		app.loopbackPath !== undefined &&
		Number(loopback[1]) <= 65535 &&
		loopback[2] === app.loopbackPath
	);
}

/**
 * The CORS origin to allow for a browser's code exchange, or `null`. Loopback
 * origins (a wallet's development server) only where `loopback` is allowed:
 * staging and local tests, never production.
 */
export function corsOrigin(origin: string | null, loopback: boolean): string | null {
	if (!origin) return null;
	if (Object.values(APPS).some((app) => app.webOrigins?.includes(origin))) return origin;
	if (loopback && /^http:\/\/(127\.0\.0\.1|localhost):[1-9][0-9]{0,4}$/.test(origin)) return origin;
	return null;
}

export interface AppRequest {
	app: string;
	appName: string;
	redirectUri: string;
	state: string;
	codeChallenge: string;
	/** What the person sees this installation called, e.g. "iPhone". */
	label: string;
	/** Stable per app installation: signing in again from it rotates its client token. */
	installation: string;
}

const STATE = /^[A-Za-z0-9\-._~]{8,256}$/;
const INSTALLATION = /^[A-Za-z0-9\-._~]{8,64}$/;

/**
 * A `/connect` request from an app, validated, or why not. An invalid
 * redirect URI is never redirected to: the page says so instead
 * (RFC 6749 §4.1.2.1).
 */
export function appRequest(params: URLSearchParams): { ok: AppRequest } | { error: string } {
	const app = APPS[params.get('app') ?? ''];
	if (!app) return { error: 'This application is not registered with clientmcp.dev.' };
	const redirectUri = params.get('redirect_uri') ?? '';
	if (!returnsTo(app, redirectUri)) {
		return { error: `This return address is not registered for ${app.name}.` };
	}
	const state = params.get('state') ?? '';
	const codeChallenge = params.get('code_challenge') ?? '';
	if (
		params.get('code_challenge_method') !== 'S256' ||
		!/^[A-Za-z0-9_-]{43}$/.test(codeChallenge)
	) {
		return { error: 'The request is missing its PKCE challenge (S256).' };
	}
	if (!STATE.test(state)) return { error: 'The request is missing its state.' };
	const installation = params.get('installation') ?? '';
	if (!INSTALLATION.test(installation))
		return { error: 'The request does not name its installation.' };
	return {
		ok: {
			app: params.get('app')!,
			appName: app.name,
			redirectUri,
			state,
			codeChallenge,
			label: clean(params.get('label'), app.name),
			installation
		}
	};
}

/** A label as shown: printable, at most 40 characters, `fallback` when empty. */
export function clean(label: string | null, fallback: string): string {
	const text = (label ?? '')
		// Control characters and angle brackets: a label is plain text.
		// eslint-disable-next-line no-control-regex
		.replace(/[\u0000-\u001f\u007f<>]/g, '')
		.trim()
		.slice(0, 40);
	return text || fallback;
}

/** `redirectUri` with `code` and `state` added. */
export function redirectWithCode(redirectUri: string, code: string, state: string): string {
	const separator = redirectUri.includes('?') ? '&' : '?';
	return `${redirectUri}${separator}code=${encodeURIComponent(code)}&state=${encodeURIComponent(state)}`;
}
