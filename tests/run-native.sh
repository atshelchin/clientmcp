#!/bin/sh
# Builds the server, starts it with a throwaway secret, and runs the conformance suite.
set -e
cd "$(dirname "$0")/.."
cargo build -q -p clientmcp-server
export CLIENTMCP_SECRET=$(openssl rand -hex 32) CLIENTMCP_CALL_TIMEOUT=5 \
  CLIENTMCP_ALLOWED_ORIGINS=http://allowed.test CLIENTMCP_LISTEN=127.0.0.1:${PORT:-18080}
target/debug/clientmcp & SERVER=$!
trap 'kill $SERVER' EXIT
sleep 0.5
cd tests
BRIDGE_URL=http://$CLIENTMCP_LISTEN TEST_ALLOWED_ORIGIN=http://allowed.test \
  node --test --test-concurrency=1 --test-timeout=60000 conformance.test.mjs
