-- clientmcp.dev's accounts (docs/self-serve.md, "Identities and tokens").
-- No token is stored here: the bridge mints them on request.

CREATE TABLE accounts (
  id TEXT PRIMARY KEY,                   -- opaque: a-<20 base32>
  created_at INTEGER NOT NULL,           -- epoch seconds
  last_seen_at INTEGER NOT NULL,
  state TEXT NOT NULL DEFAULT 'active',  -- active | suspended
  client_limit INTEGER NOT NULL DEFAULT 20
);

-- How an account signs in. Other providers are new rows, not a new model.
CREATE TABLE identities (
  provider TEXT NOT NULL,                -- github
  provider_user_id TEXT NOT NULL,        -- GitHub's numeric ID (logins change)
  account_id TEXT NOT NULL REFERENCES accounts (id),
  login TEXT NOT NULL,
  avatar_url TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (provider, provider_user_id)
);
CREATE INDEX identities_account ON identities (account_id);

-- One per app installation, or made by hand. The ID is the tokens' `sub`.
CREATE TABLE clients (
  id TEXT PRIMARY KEY,                   -- opaque: c-<20 base32>
  account_id TEXT NOT NULL REFERENCES accounts (id),
  app TEXT,                              -- vela, or NULL for a hand-made client
  installation TEXT,                     -- the app's stable installation ID
  label TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  deleted_at INTEGER
);
CREATE INDEX clients_account ON clients (account_id, deleted_at);
CREATE UNIQUE INDEX clients_installation ON clients (account_id, app, installation)
  WHERE deleted_at IS NULL AND app IS NOT NULL;

-- Authorization codes: two minutes, once, bound to a PKCE challenge.
CREATE TABLE auth_codes (
  code_hash TEXT PRIMARY KEY,            -- SHA-256 of the code; the code itself is never stored
  client_id TEXT NOT NULL REFERENCES clients (id),
  account_id TEXT NOT NULL,
  app TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  rotate INTEGER NOT NULL,               -- 1: the installation had a client; its token is rotated
  expires_at INTEGER NOT NULL,
  used_at INTEGER
);

-- The account side of the audit (the bridge keeps the issuance side).
CREATE TABLE audit (
  at INTEGER NOT NULL,
  account_id TEXT,
  action TEXT NOT NULL,                  -- sign-in | client-created | token-exchanged | …
  detail TEXT NOT NULL DEFAULT '',
  actor TEXT NOT NULL
);
CREATE INDEX audit_account ON audit (account_id, at);
