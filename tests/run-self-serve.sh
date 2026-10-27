#!/bin/sh
# Runs the Worker locally in workerd (wrangler dev) and the self-serve suite
# (token generations, the internal issuance API, metering, the rate limit)
# against it. Its own config (no route, a rate limit of 5) and its own local
# state, so nothing is shared with another run.
set -e
WORKER="$(cd "$(dirname "$0")/../crates/worker" && pwd)"
cd "$WORKER"
CONFIG=wrangler.selfserve-test.toml
STATE=$(mktemp -d)
SECRET=$(openssl rand -hex 32)
SERVICE=$(openssl rand -hex 32)
LIMIT=5
printf 'CLIENTMCP_SECRET=%s\nCLIENTMCP_SERVICE_SECRET=%s\nCLIENTMCP_ISSUE_LIMIT=%s\nCLIENTMCP_CALL_TIMEOUT=5\n' "$SECRET" "$SERVICE" "$LIMIT" > .dev.vars
npx wrangler d1 migrations apply BRIDGE_DB --local -c "$CONFIG" --persist-to "$STATE" > /dev/null
PORT=${PORT:-18797}
# Its own process group, so that stopping it also stops workerd.
set -m
npx wrangler dev -c "$CONFIG" --persist-to "$STATE" --ip 127.0.0.1 --port "$PORT" > /dev/null 2>&1 & WRANGLER=$!
set +m
trap 'kill -- -$WRANGLER 2>/dev/null; rm -f "$WORKER/.dev.vars"; rm -rf "$STATE"' EXIT
# At most two minutes: a dev server that never comes up fails the run instead of hanging it.
TRIES=0
until curl --noproxy '*' -sf "http://127.0.0.1:$PORT/health" > /dev/null; do
  TRIES=$((TRIES + 1))
  [ "$TRIES" -lt 120 ] || { echo "wrangler dev did not come up" >&2; exit 1; }
  sleep 1
done
cd ../../tests
BRIDGE_URL=http://127.0.0.1:$PORT CLIENTMCP_SECRET=$SECRET CLIENTMCP_SERVICE_SECRET=$SERVICE \
  CLIENTMCP_ISSUE_LIMIT=$LIMIT CLIENTMCP_RATE_LIMIT=5 CLIENTMCP_CALL_TIMEOUT=5 \
  node --test --test-concurrency=1 --test-timeout=60000 self-serve.test.mjs
# The issuance audit is the bridge's database: every action the suite took is there.
cd "$WORKER"
AUDITED=$(npx wrangler d1 execute BRIDGE_DB --local -c "$CONFIG" --persist-to "$STATE" --json \
  --command "SELECT COUNT(*) AS n, SUM(actor = 'test') AS by_test FROM issuance_audit" | node -e '
  let s = ""; process.stdin.on("data", (c) => (s += c)).on("end", () => {
    const row = JSON.parse(s)[0].results[0]; console.log(`${row.n} ${row.by_test}`); });')
echo "issuance audit rows: $AUDITED"
set -- $AUDITED
[ "$1" -gt 0 ] && [ "$1" = "$2" ] || { echo "the issuance audit is missing or wrong" >&2; exit 1; }
