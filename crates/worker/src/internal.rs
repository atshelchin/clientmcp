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

/// The object's answer to a mint or a rotation: the generations to sign at.
#[derive(Deserialize, Serialize)]
pub(crate) struct Minting {
    pub client: u64,
    pub agent: u64,
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
            let mut answer = object
                .fetch_with_request(ask("mint", &json!({ "actor": actor }))?)
                .await?;
            if answer.status_code() != 200 {
                return Ok(answer);
            }
            let at: Minting = answer.json().await?;
            let mut out = json!({ "exp": exp });
            for role in roles {
                let generation = match role {
                    Role::Client => at.client,
                    Role::Agent => at.agent,
                };
                out[role.to_string()] =
                    json!({ "token": sign(config, id, role, generation, exp), "gen": generation });
                audit(env, id, "mint", &role.to_string(), Some(generation), actor).await;
            }
            Response::from_json(&out)
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
            let mut answer = object
                .fetch_with_request(ask(
                    "rotate",
                    &json!({ "role": role.to_string(), "grace_ms": grace.saturating_mul(1000), "actor": actor }),
                )?)
                .await?;
            if answer.status_code() != 200 {
                return Ok(answer);
            }
            let at: Minting = answer.json().await?;
            let generation = match role {
                Role::Client => at.client,
                Role::Agent => at.agent,
            };
            audit(
                env,
                id,
                "rotate",
                &role.to_string(),
                Some(generation),
                actor,
            )
            .await;
            Response::from_json(&json!({
                "exp": exp,
                role.to_string(): { "token": sign(config, id, role, generation, exp), "gen": generation },
            }))
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
