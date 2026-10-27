// The site's database (migrations/0001_accounts.sql). Every query is here,
// with bound parameters only.

import { newAccountId, newClientId } from './crypto.ts';
import type { GitHubUser } from './github.ts';

export interface Account {
	id: string;
	state: 'active' | 'suspended';
	client_limit: number;
	login: string;
	avatar_url: string;
}

export interface ClientRow {
	id: string;
	app: string | null;
	installation: string | null;
	label: string;
	created_at: number;
}

export interface CodeRow {
	client_id: string;
	account_id: string;
	app: string;
	redirect_uri: string;
	code_challenge: string;
	rotate: number;
}

const now = () => Math.floor(Date.now() / 1000);

export async function audit(
	db: D1Database,
	account: string | null,
	action: string,
	detail: string,
	actor: string
) {
	await db
		.prepare(
			'INSERT INTO audit (at, account_id, action, detail, actor) VALUES (?1, ?2, ?3, ?4, ?5)'
		)
		.bind(now(), account, action, detail, actor)
		.run();
}

/** The account a GitHub user signs in to; created on their first sign-in. */
export async function signIn(db: D1Database, user: GitHubUser): Promise<string> {
	const at = now();
	const known = await db
		.prepare(
			"SELECT account_id FROM identities WHERE provider = 'github' AND provider_user_id = ?1"
		)
		.bind(user.id)
		.first<{ account_id: string }>();
	if (known) {
		await db.batch([
			db
				.prepare(
					"UPDATE identities SET login = ?2, avatar_url = ?3 WHERE provider = 'github' AND provider_user_id = ?1"
				)
				.bind(user.id, user.login, user.avatarUrl),
			db.prepare('UPDATE accounts SET last_seen_at = ?2 WHERE id = ?1').bind(known.account_id, at)
		]);
		await audit(db, known.account_id, 'sign-in', `github:${user.login}`, 'person');
		return known.account_id;
	}
	const account = newAccountId();
	await db.batch([
		db
			.prepare('INSERT INTO accounts (id, created_at, last_seen_at) VALUES (?1, ?2, ?2)')
			.bind(account, at),
		db
			.prepare(
				"INSERT INTO identities (provider, provider_user_id, account_id, login, avatar_url, created_at) VALUES ('github', ?1, ?2, ?3, ?4, ?5)"
			)
			.bind(user.id, account, user.login, user.avatarUrl, at)
	]);
	await audit(db, account, 'account-created', `github:${user.login}`, 'person');
	return account;
}

export async function account(db: D1Database, id: string): Promise<Account | null> {
	return db
		.prepare(
			"SELECT a.id, a.state, a.client_limit, i.login, i.avatar_url FROM accounts a JOIN identities i ON i.account_id = a.id AND i.provider = 'github' WHERE a.id = ?1"
		)
		.bind(id)
		.first<Account>();
}

export async function clients(db: D1Database, account: string): Promise<ClientRow[]> {
	const { results } = await db
		.prepare(
			'SELECT id, app, installation, label, created_at FROM clients WHERE account_id = ?1 AND deleted_at IS NULL ORDER BY created_at'
		)
		.bind(account)
		.all<ClientRow>();
	return results;
}

export async function client(
	db: D1Database,
	account: string,
	id: string
): Promise<ClientRow | null> {
	return db
		.prepare(
			'SELECT id, app, installation, label, created_at FROM clients WHERE id = ?1 AND account_id = ?2 AND deleted_at IS NULL'
		)
		.bind(id, account)
		.first<ClientRow>();
}

export async function installationClient(
	db: D1Database,
	account: string,
	app: string,
	installation: string
): Promise<ClientRow | null> {
	return db
		.prepare(
			'SELECT id, app, installation, label, created_at FROM clients WHERE account_id = ?1 AND app = ?2 AND installation = ?3 AND deleted_at IS NULL'
		)
		.bind(account, app, installation)
		.first<ClientRow>();
}

/**
 * The app installation connects as `client` (one of the account's) from now
 * on: the client it had lets go of it, and `client` becomes this app's for
 * this installation. Its client token is then rotated by the exchange, so a
 * device that held it before stops.
 */
export async function bindClient(
	db: D1Database,
	account: string,
	client: string,
	app: string,
	installation: string
): Promise<void> {
	await db.batch([
		db
			.prepare(
				'UPDATE clients SET installation = NULL WHERE account_id = ?1 AND app = ?2 AND installation = ?3 AND id != ?4'
			)
			.bind(account, app, installation, client),
		db
			.prepare(
				'UPDATE clients SET app = ?2, installation = ?3 WHERE id = ?4 AND account_id = ?1 AND deleted_at IS NULL'
			)
			.bind(account, app, installation, client)
	]);
	await audit(db, account, 'client-bound', `${client} ${app}`, 'person');
}

/** The installation lets go of its client (it connects as a new one). */
export async function unbindInstallation(
	db: D1Database,
	account: string,
	app: string,
	installation: string
): Promise<void> {
	await db
		.prepare(
			'UPDATE clients SET installation = NULL WHERE account_id = ?1 AND app = ?2 AND installation = ?3'
		)
		.bind(account, app, installation)
		.run();
}

export class LimitReached extends Error {}

/** A new client, within the account's `client_limit`. */
export async function createClient(
	db: D1Database,
	acct: Account,
	fields: { app: string | null; installation: string | null; label: string }
): Promise<string> {
	const { n } = (await db
		.prepare('SELECT COUNT(*) AS n FROM clients WHERE account_id = ?1 AND deleted_at IS NULL')
		.bind(acct.id)
		.first<{ n: number }>())!;
	if (n >= acct.client_limit) throw new LimitReached();
	const id = newClientId();
	await db
		.prepare(
			'INSERT INTO clients (id, account_id, app, installation, label, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)'
		)
		.bind(id, acct.id, fields.app, fields.installation, fields.label, now())
		.run();
	await audit(db, acct.id, 'client-created', `${id} ${fields.app ?? 'manual'}`, 'person');
	return id;
}

export async function deleteClient(db: D1Database, account: string, id: string) {
	await db
		.prepare(
			// It lets go of its app installation, which may connect again as a new client.
			'UPDATE clients SET deleted_at = ?3, installation = NULL WHERE id = ?1 AND account_id = ?2 AND deleted_at IS NULL'
		)
		.bind(id, account, now())
		.run();
	await audit(db, account, 'client-deleted', id, 'person');
}

export const CODE_SECONDS = 120;

export async function saveCode(db: D1Database, codeHash: string, row: CodeRow) {
	await db
		.prepare(
			'INSERT INTO auth_codes (code_hash, client_id, account_id, app, redirect_uri, code_challenge, rotate, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)'
		)
		.bind(
			codeHash,
			row.client_id,
			row.account_id,
			row.app,
			row.redirect_uri,
			row.code_challenge,
			row.rotate,
			now() + CODE_SECONDS
		)
		.run();
}

/**
 * The code's row, used up in the same statement: a code works once, and
 * only before it expires. A second attempt finds nothing.
 */
export async function takeCode(db: D1Database, codeHash: string): Promise<CodeRow | null> {
	const at = now();
	return db
		.prepare(
			'UPDATE auth_codes SET used_at = ?2 WHERE code_hash = ?1 AND used_at IS NULL AND expires_at > ?2 RETURNING client_id, account_id, app, redirect_uri, code_challenge, rotate'
		)
		.bind(codeHash, at)
		.first<CodeRow>();
}

// ---------------------------------------------------------------------------
// The admin page (docs/self-serve.md, "The admin page")
// ---------------------------------------------------------------------------

/** Whether `account` signs in as the owner's GitHub identity — the ID, never the login. */
export async function isAdmin(
	db: D1Database,
	account: string,
	adminGithubId: string
): Promise<boolean> {
	if (!/^\d+$/.test(adminGithubId)) return false;
	const row = await db
		.prepare(
			"SELECT 1 AS yes FROM identities WHERE account_id = ?1 AND provider = 'github' AND provider_user_id = ?2"
		)
		.bind(account, adminGithubId)
		.first<{ yes: number }>();
	return row !== null;
}

export interface AdminAccount {
	id: string;
	login: string;
	avatar_url: string;
	created_at: number;
	last_seen_at: number;
	state: 'active' | 'suspended';
	client_limit: number;
	clients: number;
}

export async function adminAccounts(db: D1Database): Promise<AdminAccount[]> {
	const { results } = await db
		.prepare(
			"SELECT a.id, i.login, i.avatar_url, a.created_at, a.last_seen_at, a.state, a.client_limit, (SELECT COUNT(*) FROM clients c WHERE c.account_id = a.id AND c.deleted_at IS NULL) AS clients FROM accounts a JOIN identities i ON i.account_id = a.id AND i.provider = 'github' ORDER BY a.last_seen_at DESC LIMIT 1000"
		)
		.all<AdminAccount>();
	return results;
}

export interface AdminClient extends ClientRow {
	account_id: string;
}

export async function adminClients(db: D1Database): Promise<AdminClient[]> {
	const { results } = await db
		.prepare(
			'SELECT id, account_id, app, installation, label, created_at FROM clients WHERE deleted_at IS NULL ORDER BY created_at DESC LIMIT 5000'
		)
		.all<AdminClient>();
	return results;
}

export async function setAccountState(
	db: D1Database,
	account: string,
	state: 'active' | 'suspended',
	actor: string
) {
	await db.prepare('UPDATE accounts SET state = ?2 WHERE id = ?1').bind(account, state).run();
	await audit(
		db,
		account,
		state === 'suspended' ? 'account-suspended' : 'account-reactivated',
		'',
		actor
	);
}

export async function setClientLimit(
	db: D1Database,
	account: string,
	limit: number,
	actor: string
) {
	await db
		.prepare('UPDATE accounts SET client_limit = ?2 WHERE id = ?1')
		.bind(account, limit)
		.run();
	await audit(db, account, 'limit-set', String(limit), actor);
}

export interface AuditRow {
	at: number;
	account_id: string | null;
	action: string;
	detail: string;
	actor: string;
}

export async function recentAudit(db: D1Database, limit = 100): Promise<AuditRow[]> {
	const { results } = await db
		.prepare('SELECT at, account_id, action, detail, actor FROM audit ORDER BY at DESC LIMIT ?1')
		.bind(limit)
		.all<AuditRow>();
	return results;
}

/** The account a client belongs to, whoever's it is (admin only). */
export async function anyClient(db: D1Database, id: string): Promise<AdminClient | null> {
	return db
		.prepare(
			'SELECT id, account_id, app, installation, label, created_at FROM clients WHERE id = ?1 AND deleted_at IS NULL'
		)
		.bind(id)
		.first<AdminClient>();
}
