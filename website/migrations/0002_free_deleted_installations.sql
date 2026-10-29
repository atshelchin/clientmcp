-- A deleted client lets go of its app installation: the unique index on
-- (account_id, app, installation) covers deleted rows too, so a client
-- deleted on the site would otherwise stop the same installation from ever
-- connecting again. `deleteClient` clears it from now on; this clears the
-- ones deleted before.
UPDATE clients SET installation = NULL WHERE deleted_at IS NOT NULL;
