#!/bin/sh
# Runs the Worker locally in workerd (wrangler dev) and the conformance suite against it.
set -e
WORKER="$(cd "$(dirname "$0")/../crates/worker" && pwd)"
cd "$WORKER"
SECRET=$(openssl rand -hex 32)
printf 'CLIENTMCP_SECRET=%s\nCLIENTMCP_CALL_TIMEOUT=5\nCLIENTMCP_ALLOWED_ORIGINS=http://allowed.test\n' "$SECRET" > .dev.vars
PORT=${PORT:-18787}
# Its own process group, so that stopping it also stops workerd.
set -m
npx wrangler dev --ip 127.0.0.1 --port "$PORT" > /dev/null 2>&1 & WRANGLER=$!
set +m
trap 'kill -- -$WRANGLER 2>/dev/null; rm -f "$WORKER/.dev.vars"' EXIT
until curl --noproxy '*' -sf "http://127.0.0.1:$PORT/health" > /dev/null; do sleep 1; done
cd ../../tests
BRIDGE_URL=http://127.0.0.1:$PORT CLIENTMCP_SECRET=$SECRET CLIENTMCP_CALL_TIMEOUT=5 \
  TEST_ALLOWED_ORIGIN=http://allowed.test \
  node --test --test-concurrency=1 --test-timeout=60000 conformance.test.mjs
