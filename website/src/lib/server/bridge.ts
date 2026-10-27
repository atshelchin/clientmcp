// The bridge's internal API, over the Service Binding (docs/self-serve.md,
// "Issuance: only the bridge signs"). The site never holds the token secret:
// it asks, the bridge mints, and the bridge keeps the issuance audit.

export interface BridgeConfig {
	/** The Service Binding to the bridge Worker. */
	binding: Fetcher;
	/** CLIENTMCP_SERVICE_SECRET, as the bridge has it. */
	serviceSecret: string;
}

export interface Minted {
	token: string;
	gen: number;
}

export interface UsageDay {
	day: string;
	calls_ok: number;
	calls_tool_error: number;
	calls_offline: number;
	calls_timeout: number;
	calls_cancelled: number;
	calls_disconnected: number;
	calls_refused: number;
	calls_limited: number;
	requests: number;
	bytes_in: number;
	bytes_out: number;
	connected_ms: number;
	connections: number;
	rejections: number;
}

export class BridgeError extends Error {
	constructor(
		readonly status: number,
		message: string
	) {
		super(message);
	}
}

/** About a year: the owner's choice for self-serve tokens. */
export const TOKEN_DAYS = 365;

const expiry = () => Math.floor(Date.now() / 1000) + TOKEN_DAYS * 24 * 3600;

async function call<T>(config: BridgeConfig, path: string, body?: unknown): Promise<T> {
	return internal<T>(config, `clients/${path}`, body);
}

async function internal<T>(config: BridgeConfig, path: string, body?: unknown): Promise<T> {
	const response = await config.binding.fetch(`https://bridge/internal/${path}`, {
		method: body === undefined ? 'GET' : 'POST',
		headers: { 'content-type': 'application/json', 'x-clientmcp-service': config.serviceSecret },
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	if (!response.ok) throw new BridgeError(response.status, (await response.text()).slice(0, 200));
	return (await response.json()) as T;
}

export async function mint(
	config: BridgeConfig,
	client: string,
	roles: ('client' | 'agent')[],
	actor: string
): Promise<{ exp: number; client?: Minted; agent?: Minted }> {
	return call(config, `${client}/mint`, { exp: expiry(), roles, actor });
}

export async function rotate(
	config: BridgeConfig,
	client: string,
	role: 'client' | 'agent',
	actor: string,
	graceSeconds = 0
): Promise<{ exp: number; client?: Minted; agent?: Minted }> {
	return call(config, `${client}/rotate`, {
		role,
		exp: expiry(),
		grace_seconds: graceSeconds,
		actor
	});
}

export async function revoke(
	config: BridgeConfig,
	client: string,
	role: 'client' | 'agent' | 'both',
	actor: string
) {
	return call(config, `${client}/revoke`, { role, actor });
}

export async function remove(config: BridgeConfig, client: string, actor: string) {
	return call(config, `${client}/delete`, { actor });
}

/** The days the bridge has not written out yet, today's included. */
export async function usage(config: BridgeConfig, client: string): Promise<UsageDay[]> {
	return call(config, `${client}/usage`);
}

/** The agent endpoint for a bridge's client endpoint: wss://host/connect → https://host/mcp. */
export function mcpUrl(bridgePublicUrl: string): string {
	const url = new URL(bridgePublicUrl);
	url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
	url.pathname = url.pathname.replace(/\/connect\/?$/, '') + '/mcp';
	return url.toString();
}

/** A finished day's usage for one client, as the bridge wrote it. */
export interface UsageRow extends UsageDay {
	client: string;
}

/** Finished days' usage, every client's (admin). */
export async function usageDays(config: BridgeConfig, days = 30): Promise<UsageRow[]> {
	return internal(config, `usage?days=${days}`);
}

export interface IssuanceRow {
	at: number;
	client: string;
	action: string;
	role: string;
	gen: number | null;
	actor: string;
}

/** The bridge's latest issuance audit rows (admin). */
export async function issuanceAudit(config: BridgeConfig, limit = 100): Promise<IssuanceRow[]> {
	return internal(config, `audit?limit=${limit}`);
}
