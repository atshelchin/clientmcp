//! The bridge's internal API (`docs/self-serve.md`, "Issuance: only the bridge
//! signs"): the site asks, the bridge mints. The token secret never leaves
//! the bridge.
//!
//! Reached over a Service Binding from the site's Worker, and authenticated
//! besides by `CLIENTMCP_SERVICE_SECRET` in `x-clientmcp-service`. Without
//! that secret configured, or with any other value, every path here answers
//! `404`, as an unknown path does: nothing tells a stranger it exists.
//!
//! - `GET    /internal/clients/<id>`          the client's generations
//! - `GET    /internal/clients/<id>/usage`    the days not yet written out,
//!   today's included: exact counts, never content
//! - `POST   /internal/clients/<id>/mint`     `{exp, roles?, actor}` → tokens
//!   at the current generations
//! - `POST   /internal/clients/<id>/rotate`   `{role, exp, grace_seconds?,
//!   actor}` → a token at the role's next generation
//! - `POST   /internal/clients/<id>/revoke`   `{role: client|agent|both, actor}`
//! - `POST   /internal/clients/<id>/delete`   `{actor}`
//! - `GET    /internal/usage?days=30`         finished days' usage, per client
//! - `GET    /internal/audit?limit=100`       the latest issuance audit rows
//!
//! The generations live in the client's object; this module signs.

use clientmcp_core::Config;
use clientmcp_core::token::{Claims, Role, is_client_id};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use worker::*;

use crate::metering::audit;
use crate::{CLIENT_ID, clients};

/// The header carrying the service secret.
const SERVICE_HEADER: &str = "x-clientmcp-service";
/// The longest validity a minted token may have: a little over a year.
const MAX_EXP_SECONDS: u64 = 400 * 24 * 60 * 60;

/// One credential as the object hands it out: what to sign.
#[derive(Deserialize, Serialize)]
pub(crate) struct Issued {
    #[serde(rename = "gen")]
    pub generation: u64,
    pub exp: u64,
    /// Issued just now, or the current credential shown again.
    pub new: bool,
}

pub(crate) async fn handle(
    mut req: Request,
    env: &Env,
    config: &Config,
    now: u64,
) -> Result<Response> {
    if !authorized(&req, env) {
        return Response::error("not found", 404);
    }
    // The bridge's own records, for the site's admin page.
    match req.path().as_str() {
        "/internal/usage" => return usage_days(&req, env).await,
        "/internal/audit" => return audit_log(&req, env).await,
        _ => {}
    }
    let path = req.path();
    let mut parts = path.trim_start_matches("/internal/clients/").splitn(2, '/');
    let (Some(id), action) = (parts.next(), parts.next()) else {
        return Response::error("not found", 404);
    };
    if !is_client_id(id) {
        return Response::error("not a client ID", 400);
    }
    let body: Value = if req.method() == Method::Post {
        match req.json().await {
            Ok(body) => body,
            Err(_) => return Response::error("the body must be JSON", 400),
        }
    } else {
        Value::Null
    };
    let actor = body.get("actor").and_then(Value::as_str).unwrap_or("");
    if req.method() == Method::Post && actor.is_empty() {
        return Response::error("say who asked: `actor`", 400);
    }
    let exp = body
        .get("exp")
        .and_then(Value::as_u64)
        .filter(|exp| *exp > now && *exp - now <= MAX_EXP_SECONDS);
    const BAD_EXP: &str = "`exp` must be in the future, within 400 days";
    let object = clients(env)?.get_by_name(id)?;
    let ask = |what: &str, payload: &Value| -> Result<Request> {
        let headers = Headers::new();
        headers.set(CLIENT_ID, id)?;
        let init = RequestInit {
            method: Method::Post,
            headers,
            body: Some(payload.to_string().into()),
            ..RequestInit::default()
        };
        Request::new_with_init(&format!("https://client/internal/{what}"), &init)
    };
    match (req.method(), action) {
        (Method::Get, None) => object.fetch_with_request(ask("state", &Value::Null)?).await,
        (Method::Get, Some("usage")) => {
            object.fetch_with_request(ask("usage", &Value::Null)?).await
        }
        (Method::Post, Some("mint")) => {
            let Some(exp) = exp else {
                return Response::error(BAD_EXP, 400);
            };
            let roles = match roles(body.get("roles")) {
                Some(roles) => roles,
                None => return Response::error("`roles` is client and/or agent", 400),
            };
            let names: Vec<String> = roles.iter().map(Role::to_string).collect();
            let answer = object
                .fetch_with_request(ask(
                    "mint",
                    &json!({ "roles": names, "exp": exp, "actor": actor }),
                )?)
                .await?;
            signed(answer, config, env, id, &roles, actor, "mint").await
        }
        (Method::Post, Some("rotate")) => {
            let Some(exp) = exp else {
                return Response::error(BAD_EXP, 400);
            };
            let Some(role) = body.get("role").and_then(Value::as_str).and_then(role) else {
                return Response::error("`role` is client or agent", 400);
            };
            let grace = body
                .get("grace_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let answer = object
                .fetch_with_request(ask(
                    "rotate",
                    &json!({ "role": role.to_string(), "grace_ms": grace.saturating_mul(1000), "exp": exp, "actor": actor }),
                )?)
                .await?;
            signed(answer, config, env, id, &[role], actor, "rotate").await
        }
        (Method::Post, Some("revoke")) => {
            let which = body.get("role").and_then(Value::as_str).unwrap_or("");
            if !["client", "agent", "both"].contains(&which) {
                return Response::error("`role` is client, agent or both", 400);
            }
            let answer = object
                .fetch_with_request(ask("revoke", &json!({ "role": which, "actor": actor }))?)
                .await?;
            if answer.status_code() == 200 {
                audit(env, id, "revoke", which, None, actor).await;
            }
            Ok(answer)
        }
        (Method::Post, Some("delete")) => {
            let answer = object
                .fetch_with_request(ask("delete", &json!({ "actor": actor }))?)
                .await?;
            if answer.status_code() == 200 {
                audit(env, id, "delete", "both", None, actor).await;
            }
            Ok(answer)
        }
        _ => Response::error("not found", 404),
    }
}

/// Whether the request carries the service secret. Compared in constant time.
fn authorized(req: &Request, env: &Env) -> bool {
    let expected = env
        .secret("CLIENTMCP_SERVICE_SECRET")
        .map(|s| s.to_string())
        .or_else(|_| env.var("CLIENTMCP_SERVICE_SECRET").map(|v| v.to_string()))
        .unwrap_or_default();
    // A short or absent secret leaves the internal API closed.
    if expected.len() < clientmcp_core::token::MIN_SECRET_LEN {
        return false;
    }
    let offered = req
        .headers()
        .get(SERVICE_HEADER)
        .ok()
        .flatten()
        .unwrap_or_default();
    offered.len() == expected.len()
        && offered
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn sign(config: &Config, id: &str, role: Role, generation: u64, exp: u64) -> String {
    config.keys.sign(&Claims {
        sub: id.to_owned(),
        role,
        exp: Some(exp),
        nbf: None,
        generation: Some(generation),
    })
}

pub(crate) fn role(name: &str) -> Option<Role> {
    match name {
        "client" => Some(Role::Client),
        "agent" => Some(Role::Agent),
        _ => None,
    }
}

/// `roles` as asked, both when absent; `None` for anything else.
fn roles(value: Option<&Value>) -> Option<Vec<Role>> {
    let Some(value) = value else {
        return Some(vec![Role::Client, Role::Agent]);
    };
    let list = value.as_array()?;
    let roles: Option<Vec<Role>> = list.iter().map(|v| v.as_str().and_then(role)).collect();
    roles.filter(|r| !r.is_empty())
}

/// A query parameter as a number within `1..=max`, else `default`.
fn bounded(req: &Request, name: &str, default: u32, max: u32) -> u32 {
    req.url()
        .ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .and_then(|(_, v)| v.parse::<u32>().ok())
        })
        .filter(|n| (1..=max).contains(n))
        .unwrap_or(default)
}

/// Finished days' exact usage per client (metering.rs writes them after
/// midnight), newest first. Today's are in each client's object (`…/usage`).
async fn usage_days(req: &Request, env: &Env) -> Result<Response> {
    let days = bounded(req, "days", 30, 400);
    let since = clientmcp_core::usage::day_of(
        (js_sys::Date::now() as u64).saturating_sub(u64::from(days) * 24 * 60 * 60 * 1000),
    );
    let db = env.d1("BRIDGE_DB")?;
    let rows = db
        .prepare("SELECT * FROM usage_daily WHERE day >= ?1 ORDER BY day DESC, client LIMIT 10000")
        .bind(&[since.into()])?
        .all()
        .await?
        .results::<Value>()?;
    Response::from_json(&rows)
}

/// The latest issuance audit rows, newest first.
async fn audit_log(req: &Request, env: &Env) -> Result<Response> {
    let limit = bounded(req, "limit", 100, 1000);
    let db = env.d1("BRIDGE_DB")?;
    let rows = db
        .prepare("SELECT at, client, action, role, gen, actor FROM issuance_audit ORDER BY at DESC LIMIT ?1")
        .bind(&[f64::from(limit).into()])?
        .all()
        .await?
        .results::<Value>()?;
    Response::from_json(&rows)
}

/// The object's credentials, signed. A new one is audited as `action`; one
/// shown again as `reveal` — the same credential, not a new one.
async fn signed(
    mut answer: Response,
    config: &Config,
    env: &Env,
    id: &str,
    roles: &[Role],
    actor: &str,
    action: &str,
) -> Result<Response> {
    if answer.status_code() != 200 {
        return Ok(answer);
    }
    let body: Value = answer.json().await?;
    let mut out = json!({});
    let mut latest = 0;
    for role in roles {
        let Some(issued) = body
            .get(role.to_string())
            .and_then(|v| serde_json::from_value::<Issued>(v.clone()).ok())
        else {
            return Response::error("the client object did not issue a credential", 500);
        };
        out[role.to_string()] = json!({
            "token": sign(config, id, *role, issued.generation, issued.exp),
            "gen": issued.generation,
            "exp": issued.exp,
        });
        latest = latest.max(issued.exp);
        let what = if issued.new { action } else { "reveal" };
        audit(
            env,
            id,
            what,
            &role.to_string(),
            Some(issued.generation),
            actor,
        )
        .await;
    }
    out["exp"] = json!(latest);
    Response::from_json(&out)
}
