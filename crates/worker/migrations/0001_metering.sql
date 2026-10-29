-- The bridge's own records (docs/self-serve.md): exact daily usage per
-- client, and the issuance audit. Resource use and actions only — never a
-- token, an argument or a result.

CREATE TABLE usage_daily (
  day TEXT NOT NULL,             -- YYYY-MM-DD, UTC
  client TEXT NOT NULL,          -- the opaque client ID
  calls_ok INTEGER NOT NULL DEFAULT 0,
  calls_tool_error INTEGER NOT NULL DEFAULT 0,
  calls_offline INTEGER NOT NULL DEFAULT 0,
  calls_timeout INTEGER NOT NULL DEFAULT 0,
  calls_cancelled INTEGER NOT NULL DEFAULT 0,
  calls_disconnected INTEGER NOT NULL DEFAULT 0,
  calls_refused INTEGER NOT NULL DEFAULT 0,
  calls_limited INTEGER NOT NULL DEFAULT 0,
  requests INTEGER NOT NULL DEFAULT 0,
  bytes_in INTEGER NOT NULL DEFAULT 0,
  bytes_out INTEGER NOT NULL DEFAULT 0,
  connected_ms INTEGER NOT NULL DEFAULT 0,
  connections INTEGER NOT NULL DEFAULT 0,
  rejections INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, client)
);
CREATE INDEX usage_daily_client ON usage_daily (client, day);

CREATE TABLE issuance_audit (
  at INTEGER NOT NULL,           -- epoch ms
  client TEXT NOT NULL,
  action TEXT NOT NULL,          -- mint | rotate | revoke | delete
  role TEXT NOT NULL,            -- client | agent | both
  gen INTEGER,                   -- the generation minted or rotated to
  actor TEXT NOT NULL            -- who asked, as the site says it
);
CREATE INDEX issuance_audit_client ON issuance_audit (client, at);
