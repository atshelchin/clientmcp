//! Per-client token generations: revocation for stateless tokens
//! (`docs/self-serve.md`, "Revocation: one generation per role").
//!
//! A bridge that keeps generations stores one [`Generations`] per client,
//! beside the client itself. A token is accepted only at its role's current
//! generation, or — for an agent token rotated with a grace — the one before,
//! until the grace ends. A token without a `gen` claim is generation 0, so
//! every token minted before generations keeps working until its client is
//! first rotated.
//!
//! Pure: times are the caller's (epoch milliseconds).

use serde::{Deserialize, Serialize};

use crate::token::Role;

/// The longest grace an agent token's rotation may keep the old token for.
pub const MAX_AGENT_GRACE_MS: u64 = 24 * 60 * 60 * 1000;
/// How far a revocation moves a generation: past any token minted at it.
const REVOKE_STEP: u64 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generations {
    #[serde(default)]
    pub client: u64,
    #[serde(default)]
    pub agent: u64,
    /// The previous agent generation, accepted until `until_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_grace: Option<Grace>,
    /// A deleted client: no token is accepted, none is minted.
    #[serde(default)]
    pub deleted: bool,
    /// Mints in the current issuance window.
    #[serde(default)]
    pub issued: Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grace {
    pub generation: u64,
    pub until_ms: u64,
}

/// A fixed issuance window: how many mints since `started_ms`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub started_ms: u64,
    pub count: u32,
}

/// Why a rotation, revocation or mint was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The client was deleted.
    Deleted,
    /// More mints than the issuance limit allows in its window.
    TooMany,
    /// A grace on the client token, or longer than [`MAX_AGENT_GRACE_MS`].
    Grace,
}

impl Generations {
    /// The current generation of `role`.
    pub fn current(&self, role: Role) -> u64 {
        match role {
            Role::Client => self.client,
            Role::Agent => self.agent,
        }
    }

    /// Whether a `role` token at `generation` is accepted at `now_ms`.
    pub fn accepts(&self, role: Role, generation: u64, now_ms: u64) -> bool {
        if self.deleted {
            return false;
        }
        if generation == self.current(role) {
            return true;
        }
        role == Role::Agent
            && self
                .agent_grace
                .is_some_and(|g| g.generation == generation && now_ms < g.until_ms)
    }

    /// A new generation for `role`; the old one stops at once, or — for the
    /// agent token only — after `grace_ms`. Returns the new generation.
    pub fn rotate(&mut self, role: Role, grace_ms: u64, now_ms: u64) -> Result<u64, Refusal> {
        if self.deleted {
            return Err(Refusal::Deleted);
        }
        match role {
            Role::Client => {
                if grace_ms > 0 {
                    return Err(Refusal::Grace);
                }
                self.client += 1;
                Ok(self.client)
            }
            Role::Agent => {
                if grace_ms > MAX_AGENT_GRACE_MS {
                    return Err(Refusal::Grace);
                }
                self.agent_grace = (grace_ms > 0).then_some(Grace {
                    generation: self.agent,
                    until_ms: now_ms.saturating_add(grace_ms),
                });
                self.agent += 1;
                Ok(self.agent)
            }
        }
    }

    /// Every token of `role` stops at once, any grace with it. Nothing is
    /// minted: the client needs a new token before it works again.
    pub fn revoke(&mut self, role: Role) -> Result<u64, Refusal> {
        if self.deleted {
            return Err(Refusal::Deleted);
        }
        if role == Role::Agent {
            self.agent_grace = None;
        }
        let generation = match role {
            Role::Client => &mut self.client,
            Role::Agent => &mut self.agent,
        };
        *generation += REVOKE_STEP;
        Ok(*generation)
    }

    /// The client is gone: no token of either role is accepted again.
    pub fn delete(&mut self) {
        self.deleted = true;
        self.agent_grace = None;
    }

    /// Counts a mint against `limit` per `window_ms`; refused past it.
    pub fn issue(&mut self, limit: u32, window_ms: u64, now_ms: u64) -> Result<(), Refusal> {
        if self.deleted {
            return Err(Refusal::Deleted);
        }
        // A window opens at its first mint, not at the epoch.
        if self.issued.count == 0 || now_ms.saturating_sub(self.issued.started_ms) >= window_ms {
            self.issued = Window {
                started_ms: now_ms,
                count: 0,
            };
        }
        if self.issued.count >= limit {
            return Err(Refusal::TooMany);
        }
        self.issued.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 60 * 60 * 1000;

    #[test]
    fn tokens_minted_before_generations_keep_working() {
        let fresh = Generations::default();
        assert!(fresh.accepts(Role::Client, 0, 0));
        assert!(fresh.accepts(Role::Agent, 0, 0));
        assert!(!fresh.accepts(Role::Agent, 1, 0), "nothing was minted at 1");
    }

    #[test]
    fn the_two_roles_are_revoked_apart() {
        let mut g = Generations::default();
        assert_eq!(g.rotate(Role::Client, 0, 0), Ok(1));
        assert!(
            !g.accepts(Role::Client, 0, 0),
            "the old client token stops at once"
        );
        assert!(g.accepts(Role::Client, 1, 0));
        assert!(
            g.accepts(Role::Agent, 0, 0),
            "the agent's token is untouched"
        );

        assert_eq!(g.revoke(Role::Agent), Ok(1));
        assert!(!g.accepts(Role::Agent, 0, 0));
        assert!(
            g.accepts(Role::Client, 1, 0),
            "the client's token is untouched"
        );
    }

    #[test]
    fn an_agent_rotation_may_keep_the_old_token_for_a_while() {
        let mut g = Generations::default();
        assert_eq!(g.rotate(Role::Agent, HOUR, 1_000), Ok(1));
        assert!(g.accepts(Role::Agent, 1, 1_000));
        assert!(
            g.accepts(Role::Agent, 0, 1_000 + HOUR - 1),
            "within the grace"
        );
        assert!(!g.accepts(Role::Agent, 0, 1_000 + HOUR), "not after it");

        // A second rotation ends the first grace: only one old token at a time.
        assert_eq!(g.rotate(Role::Agent, 0, 2_000), Ok(2));
        assert!(!g.accepts(Role::Agent, 0, 2_000));
        assert!(!g.accepts(Role::Agent, 1, 2_000));

        // A revocation ends a grace at once.
        g.rotate(Role::Agent, HOUR, 3_000).unwrap();
        assert!(g.accepts(Role::Agent, 2, 3_000));
        g.revoke(Role::Agent).unwrap();
        assert!(!g.accepts(Role::Agent, 2, 3_000));
    }

    #[test]
    fn graces_are_for_agent_tokens_and_at_most_a_day() {
        let mut g = Generations::default();
        assert_eq!(g.rotate(Role::Client, 1, 0), Err(Refusal::Grace));
        assert_eq!(
            g.rotate(Role::Agent, MAX_AGENT_GRACE_MS + 1, 0),
            Err(Refusal::Grace)
        );
        assert_eq!(
            g,
            Generations::default(),
            "a refused rotation changes nothing"
        );
    }

    #[test]
    fn a_deleted_client_accepts_nothing_and_mints_nothing() {
        let mut g = Generations::default();
        g.rotate(Role::Agent, HOUR, 0).unwrap();
        g.delete();
        assert!(!g.accepts(Role::Client, 0, 0));
        assert!(!g.accepts(Role::Agent, 1, 0));
        assert!(!g.accepts(Role::Agent, 0, 1), "not even within a grace");
        assert_eq!(g.rotate(Role::Agent, 0, 0), Err(Refusal::Deleted));
        assert_eq!(g.issue(30, HOUR, 0), Err(Refusal::Deleted));
    }

    #[test]
    fn issuance_is_limited_per_window() {
        let mut g = Generations::default();
        for _ in 0..3 {
            assert_eq!(g.issue(3, HOUR, 10), Ok(()));
        }
        assert_eq!(g.issue(3, HOUR, 10 + HOUR - 1), Err(Refusal::TooMany));
        assert_eq!(g.issue(3, HOUR, 10 + HOUR), Ok(()), "a new window");
    }

    #[test]
    fn what_is_stored_reads_back_and_absent_fields_are_zero() {
        let mut g = Generations::default();
        g.rotate(Role::Agent, HOUR, 5).unwrap();
        let text = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Generations>(&text).unwrap(), g);
        assert_eq!(
            serde_json::from_str::<Generations>("{}").unwrap(),
            Generations::default()
        );
    }
}
