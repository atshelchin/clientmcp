#!/usr/bin/env bash
# Runs the Worker locally in workerd (wrangler dev) and the conformance suite against it.
set -e
WORKER="$(cd "$(dirname "$0")/../crates/worker" && pwd)"
# CLIENTMCP_APP=rust: the Rust client plays the application (tests/rust-app.mjs).
[ "$CLIENTMCP_APP" = rust ] && (cd "$WORKER/../.." && cargo build -q -p clientmcp-client --example conformance_app)
cd "$WORKER"
SECRET=$(openssl rand -hex 32)
printf 'CLIENTMCP_SECRET=%s\nCLIENTMCP_CALL_TIMEOUT=5\nCLIENTMCP_ALLOWED_ORIGINS=http://allowed.test\n' "$SECRET" > .dev.vars
PORT=${PORT:-18787}
# Its own process group, so that stopping it also stops workerd. bash, not sh:
# dash without a terminal (CI) cannot turn job control on, the group kill then
# fails, and `set -e` made that the run's exit status after every test passed.
set -m
npx wrangler dev --ip 127.0.0.1 --port "$PORT" > /dev/null 2>&1 & WRANGLER=$!
set +m
trap 'kill -- -$WRANGLER 2>/dev/null || true; rm -f "$WORKER/.dev.vars"' EXIT
until curl --noproxy '*' -sf "http://127.0.0.1:$PORT/health" > /dev/null; do sleep 1; done
cd ../../tests
BRIDGE_URL=http://127.0.0.1:$PORT CLIENTMCP_SECRET=$SECRET CLIENTMCP_CALL_TIMEOUT=5 \
  TEST_ALLOWED_ORIGIN=http://allowed.test \
  node --test --test-concurrency=1 --test-timeout=60000 conformance.test.mjs
