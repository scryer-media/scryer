//! Carries an indexer component's sign-in session from the client that showed
//! the operator a login challenge to the clients that use the answer.
//!
//! A challenge image belongs to the tracker session that fetched it, and every
//! connection test and every saved indexer runs in a fresh component. Without
//! a carry, the session that showed the image is gone by the time the answer
//! is submitted, and so is the session that signed in with it. Prowlarr keeps
//! its request generator for a few minutes for the same reason.
//!
//! An entry is keyed by the plugin, the proxy, and the full host config except
//! the answer itself, so it only reaches a client with the same settings. It
//! lives in memory for [`CARRY_TTL`] and is never written to disk.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a carried session waits for the operator to answer and save.
pub(crate) const CARRY_TTL: Duration = Duration::from_secs(10 * 60);
/// Carried sessions kept at once; the oldest is dropped beyond this.
const MAX_CARRIED_SESSIONS: usize = 32;

/// The settings a carried session belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LoginSessionKey {
    plugin_id: String,
    proxy_config_id: Option<String>,
    config: BTreeMap<String, String>,
}

impl LoginSessionKey {
    /// `answer_keys` are left out: the session that shows the challenge has
    /// no answer yet and the one that signs in has it.
    pub(crate) fn new(
        plugin_id: &str,
        proxy_config_id: Option<&str>,
        host_config: &BTreeMap<String, String>,
        answer_keys: &[String],
    ) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
            proxy_config_id: proxy_config_id.map(str::to_string),
            config: host_config
                .iter()
                .filter(|(key, _)| !answer_keys.contains(key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        }
    }
}

struct CarriedSession {
    key: LoginSessionKey,
    state: BTreeMap<String, Vec<u8>>,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct LoginSessionCarry {
    sessions: Mutex<Vec<CarriedSession>>,
}

impl LoginSessionCarry {
    /// The process-wide carry.
    pub(crate) fn shared() -> &'static Self {
        static CARRY: OnceLock<LoginSessionCarry> = OnceLock::new();
        CARRY.get_or_init(Self::default)
    }

    /// Keep the session that just showed a challenge.
    pub(crate) fn remember(
        &self,
        key: LoginSessionKey,
        state: BTreeMap<String, Vec<u8>>,
        now: Instant,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        sessions.retain(|session| session.expires_at > now && session.key != key);
        if sessions.len() >= MAX_CARRIED_SESSIONS {
            sessions.sort_by_key(|session| session.expires_at);
            let excess = sessions.len() + 1 - MAX_CARRIED_SESSIONS;
            sessions.drain(..excess);
        }
        sessions.push(CarriedSession {
            key,
            state,
            expires_at: now + CARRY_TTL,
        });
    }

    /// The live session for these settings, to start a new client from.
    pub(crate) fn carried(
        &self,
        key: &LoginSessionKey,
        now: Instant,
    ) -> Option<BTreeMap<String, Vec<u8>>> {
        let sessions = self.sessions.lock().ok()?;
        sessions
            .iter()
            .find(|session| &session.key == key && session.expires_at > now)
            .map(|session| session.state.clone())
    }

    /// Replace a live session with where a client of the same settings has
    /// taken it, so a sign-in made by a connection test reaches the saved
    /// indexer. Settings with no live session are not added.
    pub(crate) fn refresh(
        &self,
        key: &LoginSessionKey,
        state: BTreeMap<String, Vec<u8>>,
        now: Instant,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        if let Some(session) = sessions
            .iter_mut()
            .find(|session| &session.key == key && session.expires_at > now)
        {
            session.state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(username: &str, answer: &str) -> LoginSessionKey {
        LoginSessionKey::new(
            "synthetic-engine",
            Some("proxy-1"),
            &BTreeMap::from([
                ("username".to_string(), username.to_string()),
                ("answer".to_string(), answer.to_string()),
            ]),
            &["answer".to_string()],
        )
    }

    fn state(value: &str) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([("session".to_string(), value.as_bytes().to_vec())])
    }

    #[test]
    fn a_session_reaches_only_clients_with_the_same_settings_until_it_expires() {
        let carry = LoginSessionCarry::default();
        let start = Instant::now();
        carry.remember(key("member", ""), state("challenge"), start);

        // The answer is not part of the key; everything else is.
        assert_eq!(
            carry.carried(&key("member", "solved"), start),
            Some(state("challenge"))
        );
        assert_eq!(carry.carried(&key("other-member", ""), start), None);
        let other_proxy = LoginSessionKey::new(
            "synthetic-engine",
            None,
            &BTreeMap::from([("username".to_string(), "member".to_string())]),
            &[],
        );
        assert_eq!(carry.carried(&other_proxy, start), None);

        assert_eq!(carry.carried(&key("member", ""), start + CARRY_TTL), None);
    }

    #[test]
    fn refresh_moves_a_live_session_forward_and_never_creates_one() {
        let carry = LoginSessionCarry::default();
        let start = Instant::now();
        carry.refresh(&key("member", ""), state("signed-in"), start);
        assert_eq!(carry.carried(&key("member", ""), start), None);

        carry.remember(key("member", ""), state("challenge"), start);
        carry.refresh(&key("member", "solved"), state("signed-in"), start);
        assert_eq!(
            carry.carried(&key("member", ""), start),
            Some(state("signed-in"))
        );
    }

    #[test]
    fn the_oldest_sessions_give_way_beyond_the_limit() {
        let carry = LoginSessionCarry::default();
        let start = Instant::now();
        for index in 0..=MAX_CARRIED_SESSIONS {
            carry.remember(
                key(&format!("member-{index}"), ""),
                state("challenge"),
                start + Duration::from_secs(index as u64),
            );
        }
        let now = start + Duration::from_secs(MAX_CARRIED_SESSIONS as u64);
        assert_eq!(carry.carried(&key("member-0", ""), now), None);
        assert!(
            carry
                .carried(&key(&format!("member-{MAX_CARRIED_SESSIONS}"), ""), now)
                .is_some()
        );
    }
}
