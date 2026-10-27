#!/bin/sh
# The site's self-serve flow end to end, all local: a fake GitHub, the bridge
# in workerd (its self-serve test config) and the built site bound to it.
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKER="$ROOT/crates/worker"
SITE="$ROOT/website"
RUN=$(mktemp -d)
BRIDGE_PORT=${BRIDGE_PORT:-18797}
SITE_PORT=${SITE_PORT:-18798}
GITHUB_PORT=${GITHUB_PORT:-18799}
SECRET=$(openssl rand -hex 32)
SERVICE=$(openssl rand -hex 32)
GH_ID=test-client-id
GH_SECRET=$(openssl rand -hex 20)
printf 'CLIENTMCP_SECRET=%s\nCLIENTMCP_SERVICE_SECRET=%s\nCLIENTMCP_ISSUE_LIMIT=1000\nCLIENTMCP_CALL_TIMEOUT=5\n' "$SECRET" "$SERVICE" > "$RUN/bridge.vars"
printf 'SERVICE_SECRET=%s\nGITHUB_CLIENT_ID=%s\nGITHUB_CLIENT_SECRET=%s\nSESSION_KEY=%s\n' "$SERVICE" "$GH_ID" "$GH_SECRET" "$(openssl rand -hex 32)" > "$RUN/site.vars"

PIDS=""
cleanup() { for p in $PIDS; do kill -- -"$p" 2>/dev/null || kill "$p" 2>/dev/null; done; rm -rf "$RUN"; }
trap cleanup EXIT
set -m
node "$ROOT/tests/fake-github.mjs" "$GITHUB_PORT" "$GH_ID" "$GH_SECRET" > "$RUN/github.log" 2>&1 & PIDS="$PIDS $!"

cd "$WORKER"
npx wrangler d1 migrations apply BRIDGE_DB --local -c wrangler.selfserve-test.toml --persist-to "$RUN/bridge" > /dev/null
npx wrangler dev -c wrangler.selfserve-test.toml --env-file "$RUN/bridge.vars" --persist-to "$RUN/bridge" \
  --ip 127.0.0.1 --port "$BRIDGE_PORT" > "$RUN/bridge.log" 2>&1 & PIDS="$PIDS $!"

cd "$SITE"
SITE_ORIGIN="http://127.0.0.1:$SITE_PORT" npx vite build > "$RUN/build.log" 2>&1 || { cat "$RUN/build.log" >&2; exit 1; }
npx wrangler d1 migrations apply SITE_DB --local -c wrangler.e2e.jsonc --persist-to "$RUN/site" > /dev/null
npx wrangler dev -c wrangler.e2e.jsonc --env-file "$RUN/site.vars" --persist-to "$RUN/site" \
  --var "GITHUB_OAUTH_BASE:http://127.0.0.1:$GITHUB_PORT" --var "GITHUB_API_BASE:http://127.0.0.1:$GITHUB_PORT" \
  --var "BRIDGE_PUBLIC_URL:ws://127.0.0.1:$BRIDGE_PORT/connect" \
  --var "ADMIN_GITHUB_ID:1001" --var "TOKEN_CORS_LOOPBACK:1" \
  --ip 127.0.0.1 --port "$SITE_PORT" > "$RUN/site.log" 2>&1 & PIDS="$PIDS $!"
set +m

for port in "$BRIDGE_PORT" "$SITE_PORT"; do
  TRIES=0
  until curl --noproxy '*' -s -o /dev/null "http://127.0.0.1:$port/"; do
    TRIES=$((TRIES + 1))
    [ "$TRIES" -lt 120 ] || { echo "port $port did not come up" >&2; tail -20 "$RUN/bridge.log" "$RUN/site.log" >&2; exit 1; }
    sleep 1
  done
done

# SITE_HOLD=1: keep everything up for a browser instead of running the tests.
if [ -n "$SITE_HOLD" ]; then
  echo "site http://127.0.0.1:$SITE_PORT  github http://127.0.0.1:$GITHUB_PORT  bridge http://127.0.0.1:$BRIDGE_PORT"
  while :; do sleep 3600; done
fi

cd "$ROOT/tests"
SITE_URL=http://127.0.0.1:$SITE_PORT BRIDGE_URL=http://127.0.0.1:$BRIDGE_PORT CLIENTMCP_SECRET=$SECRET \
  GITHUB_URL=http://127.0.0.1:$GITHUB_PORT \
  node --test --test-concurrency=1 --test-timeout=120000 site.test.mjs || { tail -40 "$RUN/site.log" >&2; exit 1; }
