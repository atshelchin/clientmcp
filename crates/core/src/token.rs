//! HS256 JSON Web Tokens: the only credential in ClientMCP (SPEC §2).

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Seconds of clock skew tolerated when checking `exp` and `nbf`.
const LEEWAY: u64 = 60;
/// Shortest secret accepted, in bytes.
pub const MIN_SECRET_LEN: usize = 32;
/// Longest client ID accepted, in characters.
pub const MAX_CLIENT_ID_LEN: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Client,
    Agent,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Role::Client => "client",
            Role::Agent => "agent",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    /// The token's generation for its client and role (`docs/self-serve.md`):
    /// a bridge that keeps generations accepts only the current one. Absent
    /// is generation 0, so a token minted before generations keeps working
    /// until its client is first rotated. Other bridges ignore it (SPEC §2).
    #[serde(rename = "gen", default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

/// What a verified token grants: the client it is bound to and its generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub client: String,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    Malformed,
    Algorithm,
    Signature,
    Claims,
    Expired,
    NotYetValid,
    Role,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TokenError::Malformed => "token is not a JWT",
            TokenError::Algorithm => "token algorithm must be HS256",
            TokenError::Signature => "token signature is invalid",
            TokenError::Claims => "token claims are invalid",
            TokenError::Expired => "token has expired",
            TokenError::NotYetValid => "token is not valid yet",
            TokenError::Role => "token has the wrong role",
        })
    }
}

impl std::error::Error for TokenError {}

/// Whether `id` is a valid client ID: 1–128 characters from `A–Z a–z 0–9 . _ - : @`.
pub fn is_client_id(id: &str) -> bool {
    (1..=MAX_CLIENT_ID_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:@".contains(&b))
}

/// The secrets a bridge accepts. The first one signs new tokens.
#[derive(Clone)]
pub struct Keys(Vec<Vec<u8>>);

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keys({} secret(s))", self.0.len())
    }
}

impl Keys {
    /// Parses whitespace-separated secrets, newest first.
    pub fn parse(secrets: &str) -> Result<Keys, String> {
        let keys: Vec<Vec<u8>> = secrets
            .split_whitespace()
            .map(|s| s.as_bytes().to_vec())
            .collect();
        if keys.is_empty() {
            return Err("no secret configured".into());
        }
        if keys.iter().any(|k| k.len() < MIN_SECRET_LEN) {
            return Err(format!("secrets must be at least {MIN_SECRET_LEN} bytes"));
        }
        Ok(Keys(keys))
    }

    /// Signs `claims` with the newest secret.
    pub fn sign(&self, claims: &Claims) -> String {
        let header = B64.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = B64.encode(serde_json::to_vec(claims).expect("claims serialize"));
        let input = format!("{header}.{payload}");
        let signature = B64.encode(mac(&self.0[0], input.as_bytes()).finalize().into_bytes());
        format!("{input}.{signature}")
    }

    /// Verifies `token` and returns the client ID it is bound to, if it carries `role`.
    /// `now` is the current Unix time in seconds.
    pub fn verify(&self, token: &str, role: Role, now: u64) -> Result<String, TokenError> {
        self.bind(token, role, now).map(|bound| bound.client)
    }

    /// [`Keys::verify`], with the token's generation (0 when it carries none).
    pub fn bind(&self, token: &str, role: Role, now: u64) -> Result<Bound, TokenError> {
        let claims = self.claims(token, now)?;
        if claims.role != role {
            return Err(TokenError::Role);
        }
        Ok(Bound {
            client: claims.sub,
            generation: claims.generation.unwrap_or(0),
        })
    }

    fn claims(&self, token: &str, now: u64) -> Result<Claims, TokenError> {
        let Some((input, signature)) = token.rsplit_once('.') else {
            return Err(TokenError::Malformed);
        };
        let Some((header, payload)) = input.split_once('.').filter(|(_, p)| !p.contains('.'))
        else {
            return Err(TokenError::Malformed);
        };

        #[derive(Deserialize)]
        struct Header {
            alg: String,
            #[serde(default)]
            crit: Option<serde_json::Value>,
        }
        let header: Header = decode_json(header)?;
        // Only HS256: never let the token choose a weaker algorithm (RFC 8725 §2.1).
        // `crit` would name extensions we do not understand, so it must be absent.
        if header.alg != "HS256" || header.crit.is_some() {
            return Err(TokenError::Algorithm);
        }

        let signature = B64.decode(signature).map_err(|_| TokenError::Malformed)?;
        if !self
            .0
            .iter()
            .any(|key| mac(key, input.as_bytes()).verify_slice(&signature).is_ok())
        {
            return Err(TokenError::Signature);
        }

        let claims: Claims = decode_json(payload).map_err(|_| TokenError::Claims)?;
        if !is_client_id(&claims.sub) {
            return Err(TokenError::Claims);
        }
        if claims
            .exp
            .is_some_and(|exp| now > exp.saturating_add(LEEWAY))
        {
            return Err(TokenError::Expired);
        }
        if claims
            .nbf
            .is_some_and(|nbf| now.saturating_add(LEEWAY) < nbf)
        {
            return Err(TokenError::NotYetValid);
        }
        Ok(claims)
    }
}

fn mac(key: &[u8], data: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    mac
}

fn decode_json<T: serde::de::DeserializeOwned>(part: &str) -> Result<T, TokenError> {
    let bytes = B64.decode(part).map_err(|_| TokenError::Malformed)?;
    serde_json::from_slice(&bytes).map_err(|_| TokenError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn keys() -> Keys {
        Keys::parse(SECRET).unwrap()
    }

    fn claims(role: Role) -> Claims {
        Claims {
            sub: "alice".into(),
            role,
            exp: Some(1_000),
            nbf: None,
            generation: None,
        }
    }

    /// `docs/self-serve.md`: a token with no `gen` is generation 0 — every
    /// token minted before generations keeps working — and the claim's name
    /// on the wire is `gen`, which bridges without generations ignore.
    #[test]
    fn generations() {
        let old = keys().sign(&claims(Role::Agent));
        assert_eq!(keys().bind(&old, Role::Agent, 0).unwrap().generation, 0);
        let new = keys().sign(&Claims {
            generation: Some(3),
            ..claims(Role::Agent)
        });
        let bound = keys().bind(&new, Role::Agent, 0).unwrap();
        assert_eq!((bound.client.as_str(), bound.generation), ("alice", 3));
        assert_eq!(keys().verify(&new, Role::Agent, 0).unwrap(), "alice");
        let payload = new.split('.').nth(1).unwrap();
        let json = String::from_utf8(B64.decode(payload).unwrap()).unwrap();
        assert!(json.contains("\"gen\":3"), "{json}");
    }

    #[test]
    fn round_trip() {
        let token = keys().sign(&claims(Role::Agent));
        assert_eq!(keys().verify(&token, Role::Agent, 900).unwrap(), "alice");
        assert_eq!(
            keys().verify(&token, Role::Client, 900),
            Err(TokenError::Role)
        );
    }

    #[test]
    fn expiry_with_leeway() {
        let token = keys().sign(&claims(Role::Client));
        assert!(keys().verify(&token, Role::Client, 1_060).is_ok());
        assert_eq!(
            keys().verify(&token, Role::Client, 1_061),
            Err(TokenError::Expired)
        );
        let early = keys().sign(&Claims {
            nbf: Some(500),
            ..claims(Role::Client)
        });
        assert_eq!(
            keys().verify(&early, Role::Client, 400),
            Err(TokenError::NotYetValid)
        );
        assert!(keys().verify(&early, Role::Client, 450).is_ok());
    }

    #[test]
    fn rejects_tampering_and_foreign_keys() {
        let token = keys().sign(&claims(Role::Client));
        let other = Keys::parse("ffffffffffffffffffffffffffffffff").unwrap();
        assert_eq!(
            other.verify(&token, Role::Client, 0),
            Err(TokenError::Signature)
        );

        let forged_payload = B64.encode(br#"{"sub":"mallory","role":"client"}"#);
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[1] = &forged_payload;
        assert_eq!(
            keys().verify(&parts.join("."), Role::Client, 0),
            Err(TokenError::Signature)
        );
    }

    #[test]
    fn rejects_other_algorithms() {
        let header = B64.encode(br#"{"alg":"none"}"#);
        let payload = B64.encode(br#"{"sub":"alice","role":"client"}"#);
        let token = format!("{header}.{payload}.");
        assert_eq!(
            keys().verify(&token, Role::Client, 0),
            Err(TokenError::Algorithm)
        );
        assert_eq!(
            keys().verify("a.b", Role::Client, 0),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn rotation_accepts_every_configured_secret() {
        let old = keys().sign(&claims(Role::Client));
        let rotated = Keys::parse(&format!("{} {SECRET}", "n".repeat(32))).unwrap();
        assert!(rotated.verify(&old, Role::Client, 0).is_ok());
        assert!(
            rotated
                .verify(&rotated.sign(&claims(Role::Client)), Role::Client, 0)
                .is_ok()
        );
    }

    #[test]
    fn validates_secrets_and_client_ids() {
        assert!(Keys::parse("short").is_err());
        assert!(Keys::parse("  ").is_err());
        assert!(is_client_id("user:42@app.example-1_x"));
        assert!(!is_client_id(""));
        assert!(!is_client_id("a b"));
        assert!(!is_client_id(&"a".repeat(129)));
        let bad = keys().sign(&Claims {
            sub: "a/b".into(),
            ..claims(Role::Client)
        });
        assert_eq!(
            keys().verify(&bad, Role::Client, 0),
            Err(TokenError::Claims)
        );
    }
}
