#!/bin/sh
# A client that goes silent is presumed gone (SPEC §3.2): idle.test.mjs against
# the native bridge, then the Worker in workerd, each with a short client idle
# and a long call timeout.
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRET=$(openssl rand -hex 32)
IDLE=3
TIMEOUT=20
run() {
  cd "$ROOT/tests"
  BRIDGE_URL=$1 CLIENTMCP_SECRET=$SECRET CLIENTMCP_CALL_TIMEOUT=$TIMEOUT CLIENTMCP_CLIENT_IDLE=$IDLE \
    node --test --test-concurrency=1 --test-timeout=60000 idle.test.mjs
}

echo "== native"
cd "$ROOT" && cargo build -q -p clientmcp-server
PORT=${PORT:-18081}
CLIENTMCP_SECRET=$SECRET CLIENTMCP_CALL_TIMEOUT=$TIMEOUT CLIENTMCP_CLIENT_IDLE=$IDLE \
  CLIENTMCP_LISTEN=127.0.0.1:$PORT "$ROOT/target/debug/clientmcp" & SERVER=$!
trap 'kill $SERVER 2>/dev/null' EXIT
sleep 0.5
run http://127.0.0.1:$PORT
kill $SERVER

echo "== worker (workerd)"
WORKER="$ROOT/crates/worker"
cd "$WORKER"
printf 'CLIENTMCP_SECRET=%s\nCLIENTMCP_CALL_TIMEOUT=%s\nCLIENTMCP_CLIENT_IDLE=%s\n' "$SECRET" "$TIMEOUT" "$IDLE" > .dev.vars
WPORT=${WPORT:-18788}
set -m
npx wrangler dev --ip 127.0.0.1 --port "$WPORT" > /dev/null 2>&1 & WRANGLER=$!
set +m
trap 'kill -- -$WRANGLER 2>/dev/null; rm -f "$WORKER/.dev.vars"' EXIT
until curl --noproxy '*' -sf "http://127.0.0.1:$WPORT/health" > /dev/null; do sleep 1; done
run http://127.0.0.1:$WPORT
