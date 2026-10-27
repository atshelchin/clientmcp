# Self-serve tokens on clientmcp.dev

**Status:** approved by the owner with the review's changes (2026-10-27). Phase 1 (the bridge) is built and tested locally; not deployed.

Today a person who wants an AI agent to use their wallet must get a client
token and a bridge URL from the operator, paste both into the app, then hand
the agent an MCP URL and a second token. That is too technical for most people.

This design lets anyone sign in on clientmcp.dev with GitHub and get their
tokens. When the request starts in an app (Vela first), the client token goes
straight back into the app. It adds the revocation the stateless tokens lack,
a small admin page for the owner, and the usage data needed to price plans
later.

## Owner decisions (2026-10-27)

- Sign-in is GitHub only.
- Tokens are long-lived (about a year) and revocable. Signing in again from the
  same app installation rotates them.
- Tokens already issued keep working: the bridge's secret does not change.
- When the flow starts in an app, the client token returns to that app
  automatically. The agent's MCP URL and token are shown on the page, to copy.
- There is an admin page, open only to the owner's GitHub account.
- Usage is metered and abuse raises an alert. No quota yet: rate limiting only
  against blatant abuse.
- Usage data must support future plans: allowance per tier, price, and the
  Cloudflare cost each account causes.

Review changes adopted (2026-10-27):

- The bridge alone signs tokens; the site asks it over an internal channel.
- A client's two tokens are revoked separately, and each has one valid
  generation at a time.
- Accounts are the platform's own, linked to sign-in identities, so other
  sign-in methods can be added later.
- Metering records resource use only, never content.
- The client cap is a per-account setting.

## What a person sees

1. In Vela → Settings → AI agent, they tap **Get a token on clientmcp.dev**.
2. The browser opens `clientmcp.dev/connect`. They sign in with GitHub
   (first time only) and confirm: "Let *Vela on this device* connect to your
   agent".
3. The browser returns to Vela. Vela fills in the bridge and token itself; the
   person only turns the agent on (that switch stays theirs).
4. The page they leave behind shows what their AI agent needs: the MCP URL and
   the agent token, with copy buttons and short setup notes for common agents.
   It can be reopened later from clientmcp.dev/account.

Someone without an app (a developer trying the protocol) uses the same page
directly: sign in, name a client, copy both tokens.

On **clientmcp.dev/account** a person sees their clients (one per app
installation, or named by hand), when each last connected, and its usage this
month. They can show the agent token again, rotate a client's tokens, or delete
a client.

## Identities and tokens

- **Account:** the platform's own record, with an opaque `account_id`. It holds
  when it was created and last seen, its state (active, suspended) and its
  `client_limit` (default 20; a setting per account, so a plan can raise it).
- **Identity:** how an account signs in, as `(provider, provider_user_id)` →
  `account_id`. Today the only provider is GitHub, keyed by the numeric GitHub
  id (logins change); its login and avatar URL are kept for display. Other
  providers (Google, Apple, passkeys) are new identity rows, not a new account
  model. No email address is stored, and the GitHub OAuth app asks for no scope.
- **Client:** one per app installation or per hand-made client. Its ID (the
  tokens' `sub`) is opaque: `c-` and 20 random base32 characters. It reveals
  nothing about the account or its GitHub identity, so the bridge's records
  carry no personal identifier.
- **Tokens:** a client has a `client` token and an `agent` token, HS256, `exp`
  about one year out, and a new claim `gen` (a small integer, see below). The
  protocol already ignores claims it does not know (SPEC §2), so these are
  ordinary tokens to any bridge.

Nothing stores a token. The bridge can mint a client's token again on the
site's request (to show the agent token again), at the client's current
generation.

### Revocation: one generation per role

Tokens are stateless today, and revoking one before `exp` means rotating the
secret for everyone (SPEC §7). This design adds, for each client and **each
role separately**, a current generation, kept where the client already lives:

- Each client's Durable Object stores `client_gen` and `agent_gen` (absent
  means 0).
- A token is accepted only if its `gen` equals the current generation of its
  role, or the previous one during a grace period (below). A token **without**
  `gen` is generation 0, so every token issued before this change keeps
  working until that client is first rotated. Rejection is `4401` on the
  client channel and `401` on the agent endpoint.
- **One credential per generation.** Re-minting at the same generation (to
  show the agent token again) yields the same credential in effect, not an
  extra one: they stand and fall together. A client therefore has exactly one
  valid client credential and one valid agent credential.
- **Rotate the client token:** `client_gen + 1`, no grace. The app receives the
  new token in the same exchange that rotated it; the object closes the open
  connection still on the old token.
- **Rotate the agent token:** `agent_gen + 1`. The person may ask to keep the
  old one valid for up to 24 hours while they update their agent; the object
  then also accepts `agent_gen − 1` until that moment. Without a grace, the old
  token stops at once.
- **Revoke** a role or **delete** the client: the generation jumps past any
  issued one and no grace applies; a deleted client's object refuses every
  token.

The client channel and the agent endpoint already route every request to the
client's object, so the check costs no extra lookup. The protocol change is
small and optional: SPEC §2 gains "a bridge MAY reject a token whose `gen` is
not the current generation of its role for its client".

### Issuance: only the bridge signs

The token secret stays where it is today, in the bridge alone. The site never
holds it.

- The site calls the bridge through a Cloudflare **Service Binding**: a
  Worker-to-Worker channel that is not reachable from the internet.
- The internal requests also carry a service secret (not the token secret), so
  that a misrouted public request to the same paths is refused. Paths under
  `/internal/` answer `404` to anything else.
- The internal API:
  - mint a client's tokens at its current generations;
  - rotate one role (with an optional grace for the agent token);
  - revoke a role, or delete a client.

  The site states who asked (account ID, admin or the person), and the bridge
  records it.
- One generation is one credential: asked for again, the bridge signs the
  same claims with the same expiry, so the same token comes back, uncounted
  (audited as `reveal`). What the bridge limits is new generations — at most
  30 rotations per client per hour — and it keeps the issuance audit: what
  was minted, shown again or rotated, when, for which client, at whose
  request. The site keeps the account-side audit
  (sign-ins, suspensions).

## Returning the token to the app

The connect page must never hand a client token to a page or app that merely
asked for it. It uses the OAuth 2.0 authorization-code flow with PKCE
(RFC 7636), the pattern RFC 8252 recommends for native apps:

1. The app creates a random `code_verifier` and opens
   `https://clientmcp.dev/connect?app=vela&redirect_uri=…&state=…&code_challenge=…&code_challenge_method=S256&label=…`.
2. The site accepts only a `redirect_uri` registered for that `app`, compared
   exactly:
   - Vela iOS and Android: `app.getvela.wallet:/clientmcp` (private-use scheme);
   - Vela web: `https://wallet.getvela.app/clientmcp` (the extension later);
   - Vela desktop: the loopback `http://127.0.0.1:<any port>/clientmcp`
     (RFC 8252 §7.3), where only the port may vary.
3. After GitHub sign-in and the person's confirm, the site redirects to
   `redirect_uri?code=…&state=…`. The code lives 2 minutes and works once.
4. The app checks `state`, then posts the `code` and `code_verifier` to
   `https://clientmcp.dev/api/token` (JSON; CORS for a registered web app's
   origin). The answer: bridge URL, client token, expiry, the account login
   (shown in the app as "connected as @login"), and `agent_url`.

Another app that registers the same scheme can see the redirect but cannot use
the code without the verifier.

`agent_url` is the MCP server address for the person's AI agent, with the agent
token in its path: the same one the account page copies. The owner decided on
2026-10-27 that the app hands it over itself ("这样才方便"), so the person copies
it from the wallet without visiting the site. The app keeps it with the client
token, in its secret store. Reconnecting from the app replaces only the client token,
so agents already set up keep working; deleting the client on the site retires
both.

Apps are registered in the site's configuration (name, icon, redirect URIs).
Only the owner adds an app.

## The admin page

`clientmcp.dev/admin`, open only to the GitHub id in the site's configuration;
anyone else gets a 404.

- **Accounts:** login, created, last seen, client count, usage this month, and
  a suspend switch. Suspending revokes both roles on all of the account's clients
  and blocks new tokens.
- **Clients:** ID, label, app, generation, last connected, usage. Revoke here
  too.
- **Usage:** totals and the top accounts by calls, connection hours and bytes,
  per day and per month, with the estimated Cloudflare cost (below).
- **Alerts:** what fired, when, and for whom.
- **Audit log:** every sign-in, token issue, rotation, revocation and
  suspension, with the actor.

## Metering and alerts

The bridge measures use. The site reads the results.

### What is recorded, and what never is

Recorded, per client:

- event type: call, connection opened/closed, credential rejected;
- client ID (opaque), tool name, outcome (ok, tool error, offline, timeout,
  cancelled, disconnected, rate-limited);
- time, duration, request and response byte counts, protocol revision, close
  code.

Never recorded: tool arguments or results, tokens, request headers, IP
addresses beyond what Cloudflare itself keeps, wallet addresses, or anything
else that comes from the content of a call. A tool's name is the most specific
thing kept. That is enough to bill and to see abuse, and it means a
compromised metrics store reveals no one's wallet activity.

### Exact counts, and fast signals

Two stores, for two jobs:

- **Billing-grade counts:** each client's Durable Object already sees every
  call and holds every connection. It keeps exact counters per UTC day (calls
  by outcome, bytes in and out, connection-seconds, rejections) in its own
  storage, and writes one row per client per day to the bridge's database when
  the day closes (an alarm), or when the object next wakes. These rows are
  complete and are what plans and cost estimates are based on.
- **Near-real-time signals:** Workers Analytics Engine, one data point per
  event, for the admin page's live view and for alerts. Its sampling under high
  volume and its retention must be verified before relying on it; it is not
  an audit log and nothing depends on its being complete.

The audit logs (issuance on the bridge, sign-ins and suspensions on the site)
are ordinary database tables, written in the same request as the action.

### Alerts and limits

- **Abuse alerts:** a scheduled job (every 10 minutes) checks recent totals
  against thresholds: calls per minute, bytes per hour, connection-hours per
  day, and credential rejections from one client (a leaked or guessed token).
  An alert is written to the admin page and emailed to the owner.
- **Protective rate limit:** only against blatant abuse, for example more than
  600 agent calls per minute for one client, through the Workers rate-limiting
  binding. A limited call is answered as a tool error that says so. Thresholds
  are configuration, not code.
- **Cost model:** the main costs are Worker requests (agent calls), Durable
  Object requests and duration (a connected client holds its object; with
  hibernation, idle connections are cheap), WebSocket messages, and the
  metering writes. The admin page multiplies each account's exact daily counts
  by the current list prices, held in configuration, to estimate its monthly
  cost. That number, set against usage, is what tier allowances and prices will
  be based on.

## Where it runs

- **Site (`clientmcp-website`):** today fully prerendered. The landing page and
  docs stay prerendered; `/connect`, `/account`, `/admin` and `/api/*` become
  server routes. New bindings:
  - D1: accounts, identities, clients, apps, authorization codes, the account
    audit log, and daily usage copied from the bridge for the admin page;
  - a Service Binding to the bridge;
  - secrets: the service secret, the GitHub OAuth app's client ID and secret,
    and a session-signing key. **Not** the token secret.

  Sessions are a signed, HTTP-only, `SameSite=Lax` cookie. Every form that
  changes state carries a CSRF token.
- **Bridge (`clientmcp`):**
  - the `gen` check and the per-role generations in `ClientObject`;
  - the internal API and the issuance limit;
  - the exact daily counters and their D1 table;
  - the Analytics Engine and rate-limit bindings;
  - a new secret: the service secret.

  No change for clients or agents.

## Phases

| # | What | Exit |
|---|---|---|
| 1 | Bridge: per-role generations, the internal issuance API, exact counters, signals, rate limit; SPEC §2/§7 note | Rust and conformance tests; old tokens still accepted; deployed after the owner's go |
| 2 | Site: GitHub sign-in, D1, `/connect` (no app), `/account`, the token API with PKCE | e2e against a staging bridge and a test OAuth app |
| 3 | Vela: "Get a token" on the four shells, the redirect and code exchange, "connected as @login" | each shell's tests; the iPhone and the S22 |
| 4 | Admin page, alerts, rollups, cost estimate | e2e; one alert fired on staging |
| 5 | Rollout: the GitHub OAuth app for production, deploy, docs page | the owner signs in for real |

## What the owner needs to provide

- A GitHub OAuth app for clientmcp.dev (and one for staging): its client ID
  and secret. The owner creates these in GitHub; they are entered as Worker
  secrets and never committed.
- Where alerts go: an email address to send to.
- Confirmation of the Vela web wallet's redirect origin.

## Open questions

- **Terms:** a short acceptable-use page and a privacy note, since accounts are
  now people. Who writes them, and in which languages?
- **Limits:** start values for the abuse thresholds and the issuance limit.
- **The open trial bridge:** keep handing out operator-minted tokens alongside
  this, or stop once sign-in works?
- **Accounts other than GitHub:** out of scope now; the client ID's `gh:`
  prefix leaves room for others.
