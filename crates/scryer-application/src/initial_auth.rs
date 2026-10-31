//! The mandatory authentication decision on newly initialized instances.
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use scryer_domain::{ConfigurationChangeAction, User, UserAuthorization};
use serde::{Deserialize, Serialize};

use crate::{AppError, AppResult, AppUseCase};

pub const SETTING_KEY: &str = "auth.initial_setup";
pub const SETTING_ID: &str = "initial-authentication-setup";
pub const SEED_JSON: &str = "{\"stage\":\"choice\"}";

pub mod refusal {
    /// Authentication setup has not been completed.
    pub const REQUIRED: &str = "INITIAL_AUTH_REQUIRED";
    /// The initial authentication decision has already been made.
    pub const ALREADY_COMPLETED: &str = "INITIAL_AUTH_ALREADY_COMPLETED";
    /// The operator must replace the bootstrap password instead.
    pub const PASSWORD_REPLACEMENT_REQUIRED: &str = "INITIAL_AUTH_PASSWORD_REPLACEMENT_REQUIRED";
    /// The submitted setup code is absent or incorrect.
    pub const CODE_INVALID: &str = "INITIAL_AUTH_CODE_INVALID";
    /// Too many incorrect setup codes have been submitted.
    pub const CODE_RATE_LIMITED: &str = "INITIAL_AUTH_CODE_RATE_LIMITED";
    /// The selection conflicts with an environment override.
    pub const MODE_CONFLICT: &str = "INITIAL_AUTH_MODE_CONFLICT";
    /// The no-authentication choice requires acknowledgement.
    pub const ACKNOWLEDGEMENT_REQUIRED: &str = "INITIAL_AUTH_ACKNOWLEDGEMENT_REQUIRED";
    pub const ALL: &[&str] = &[
        REQUIRED,
        ALREADY_COMPLETED,
        PASSWORD_REPLACEMENT_REQUIRED,
        CODE_INVALID,
        CODE_RATE_LIMITED,
        MODE_CONFLICT,
        ACKNOWLEDGEMENT_REQUIRED,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum SetupState {
    Choice,
    PasswordReplacement { user_id: String },
    Complete,
}

impl SetupState {
    pub fn validate_runtime_mode(&self, form_login_enabled: bool) -> AppResult<()> {
        if matches!(self, Self::PasswordReplacement { .. }) && !form_login_enabled {
            return Err(AppError::validation_refused(
                refusal::MODE_CONFLICT,
                "temporary administrator password replacement requires form login; remove the disabled-authentication override",
            ));
        }
        Ok(())
    }

    pub fn is_pending(&self) -> bool {
        !matches!(self, Self::Complete)
    }
    pub fn stage(&self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::PasswordReplacement { .. } => "password_replacement",
            Self::Complete => "complete",
        }
    }
}

impl AppUseCase {
    pub async fn initial_authentication_state(&self) -> AppResult<Option<SetupState>> {
        self.services
            .identity
            .users
            .initial_authentication_state()
            .await
    }

    pub async fn refresh_initial_authentication_settings(
        &self,
    ) -> AppResult<crate::SecuritySettings> {
        // Initial credentials and settings share the identity transaction, which
        // deliberately bypasses the configuration store's separate transaction.
        self.services.config.settings.invalidate_cache();
        self.security_settings().await
    }

    pub async fn complete_initial_authentication(
        &self,
        password: Option<String>,
        acknowledge_no_auth: bool,
    ) -> AppResult<User> {
        match self.initial_authentication_state().await? {
            Some(SetupState::Choice) => {}
            Some(SetupState::PasswordReplacement { .. }) => {
                return Err(AppError::validation_refused(
                    refusal::PASSWORD_REPLACEMENT_REQUIRED,
                    "sign in and replace the temporary administrator password",
                ));
            }
            _ => {
                return Err(AppError::validation_refused(
                    refusal::ALREADY_COMPLETED,
                    "authentication setup is already complete",
                ));
            }
        }
        let password_hash = if let Some(password) = password {
            self.validate_new_local_password(&password).await?;
            Some(self.hash_password(&password)?)
        } else {
            if !acknowledge_no_auth {
                return Err(AppError::validation_refused(
                    refusal::ACKNOWLEDGEMENT_REQUIRED,
                    "explicitly acknowledge running without authentication",
                ));
            }
            None
        };
        let user = User {
            id: "00000000000000000000000000000001".into(),
            username: "admin".into(),
            password_hash,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization::full_admin(),
        };
        let user = self
            .services
            .identity
            .users
            .complete_initial_authentication(user)
            .await?;
        self.services.config.settings.invalidate_cache();
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            None,
            "initial_authentication",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }
}

/// Process-local proof for initial setup. The durable state is authoritative.
pub struct SetupChallenge {
    state: Mutex<ChallengeState>,
    clock: Box<dyn Fn() -> Instant + Send + Sync>,
}
const MAX_SETUP_CLIENTS: usize = 4096;
struct ChallengeState {
    verifier: Option<blake3::Hash>,
    failures: HashMap<IpAddr, VecDeque<Instant>>,
    available: bool,
}
impl Default for SetupChallenge {
    fn default() -> Self {
        Self::with_clock(Instant::now)
    }
}
impl SetupChallenge {
    /// Supplies monotonic time so failure-window tests need no wall-clock waits.
    pub fn with_clock(clock: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        Self {
            clock: Box::new(clock),
            state: Mutex::new(ChallengeState {
                verifier: None,
                failures: HashMap::new(),
                available: false,
            }),
        }
    }
    // Exclude lookalikes: I/L/1, O/Q/0, B/8, G/6, S/5 and Z/2.
    const CODE_ALPHABET: &[u8] = b"ACDEFHJKMNPRTUVWXY3479";

    pub fn initialize(&self) -> AppResult<String> {
        const ALPHABET: &[u8] = SetupChallenge::CODE_ALPHABET;
        const ACCEPT_BELOW: usize = 256 - (256 % ALPHABET.len());
        let mut code = String::new();
        while code.len() < 6 {
            let mut bytes = [0u8; 16];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| AppError::Repository("setup code generation failed".into()))?;
            for byte in bytes {
                if usize::from(byte) < ACCEPT_BELOW && code.len() < 6 {
                    code.push(ALPHABET[usize::from(byte) % ALPHABET.len()] as char);
                }
            }
        }
        let mut state = self.state.lock().expect("setup challenge lock poisoned");
        state.verifier = Some(blake3::hash(code.as_bytes()));
        state.available = true;
        state.failures.clear();
        Ok(format!("{}-{}", &code[..3], &code[3..]))
    }
    /// Opens setup with no code; the first client to answer makes the choice.
    pub fn initialize_without_code(&self) {
        let mut state = self.state.lock().expect("setup challenge lock poisoned");
        state.verifier = None;
        state.available = true;
        state.failures.clear();
    }
    pub fn code_required(&self) -> bool {
        self.state
            .lock()
            .expect("setup challenge lock poisoned")
            .verifier
            .is_some()
    }
    pub fn verify(&self, client_ip: IpAddr, code: Option<&str>) -> AppResult<()> {
        self.verify_at(client_ip, code, (self.clock)())
    }
    pub fn is_available(&self) -> bool {
        self.state
            .lock()
            .expect("setup challenge lock poisoned")
            .available
    }
    fn verify_at(&self, client_ip: IpAddr, code: Option<&str>, now: Instant) -> AppResult<()> {
        let client_ip = client_ip.to_canonical();
        let mut state = self.state.lock().expect("setup challenge lock poisoned");
        if !state.available {
            return Err(AppError::validation_refused(
                refusal::ALREADY_COMPLETED,
                "setup proof is not available",
            ));
        }
        let Some(verifier) = state.verifier else {
            return Ok(());
        };
        state.failures.retain(|_, failures| {
            while failures
                .front()
                .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
            {
                failures.pop_front();
            }
            !failures.is_empty()
        });
        if state
            .failures
            .get(&client_ip)
            .is_some_and(|failures| failures.len() >= 5)
        {
            return Err(AppError::validation_refused_with_facts(
                refusal::CODE_RATE_LIMITED,
                "too many incorrect setup codes; try again later",
                vec![crate::RefusalFact::new("retryAfterSeconds", "60")],
            ));
        }
        let normalized: String = code
            .unwrap_or_default()
            .trim()
            .chars()
            .filter(|c| *c != '-')
            .flat_map(char::to_uppercase)
            .collect();
        if normalized.len() != 6 || blake3::hash(normalized.as_bytes()) != verifier {
            // Bound unauthenticated bookkeeping without making a full table
            // lock out unrelated clients. Evict the least recent failure.
            if state.failures.len() >= MAX_SETUP_CLIENTS
                && !state.failures.contains_key(&client_ip)
                && let Some(oldest) = state
                    .failures
                    .iter()
                    .min_by_key(|(_, failures)| failures.back().copied())
                    .map(|(client, _)| *client)
            {
                state.failures.remove(&oldest);
            }
            state.failures.entry(client_ip).or_default().push_back(now);
            return Err(AppError::validation_refused(
                refusal::CODE_INVALID,
                "enter the setup code from the server console",
            ));
        }
        Ok(())
    }
    pub fn consume(&self) {
        let mut state = self.state.lock().expect("setup challenge lock poisoned");
        state.available = false;
        state.verifier = None;
        state.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLIENT: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

    #[test]
    fn initial_auth_code_alphabet_excludes_lookalikes() {
        let alphabet = SetupChallenge::CODE_ALPHABET;
        assert!(alphabet.len() >= 20);
        for character in b"ILOQBGSZ012568" {
            assert!(!alphabet.contains(character));
        }
        let unique: std::collections::HashSet<_> = alphabet.iter().collect();
        assert_eq!(unique.len(), alphabet.len());
        let code = SetupChallenge::default().initialize().unwrap();
        assert_eq!(code.as_bytes()[3], b'-');
        assert!(
            code.bytes()
                .filter(|byte| *byte != b'-')
                .all(|byte| alphabet.contains(&byte))
        );
    }

    #[test]
    fn initial_auth_code_is_retryable_until_commit_and_accepts_display_variants() {
        let challenge = SetupChallenge::default();
        let code = challenge.initialize().unwrap();
        assert_eq!(code.len(), 7);
        challenge.verify(CLIENT, Some(&code)).unwrap();
        challenge
            .verify(CLIENT, Some(&code.to_lowercase().replace('-', "")))
            .unwrap();
        challenge
            .verify(CLIENT, Some(&format!(" {code} ")))
            .unwrap();
        challenge.consume();
        assert_eq!(
            challenge
                .verify(CLIENT, Some(&code))
                .unwrap_err()
                .validation_reason(),
            Some(refusal::ALREADY_COMPLETED)
        );
    }

    #[test]
    fn initial_auth_code_limit_uses_explicit_time_and_counts_missing_codes() {
        let challenge = SetupChallenge::default();
        let code = challenge.initialize().unwrap();
        let now = Instant::now();
        for _ in 0..5 {
            assert_eq!(
                challenge
                    .verify_at(CLIENT, None, now)
                    .unwrap_err()
                    .validation_reason(),
                Some(refusal::CODE_INVALID)
            );
        }
        assert_eq!(
            challenge
                .verify_at(CLIENT, Some(&code), now)
                .unwrap_err()
                .validation_reason(),
            Some(refusal::CODE_RATE_LIMITED)
        );
        challenge
            .verify_at(CLIENT, Some(&code), now + Duration::from_secs(60))
            .unwrap();
        assert!(challenge.state.lock().unwrap().failures.is_empty());
    }

    #[test]
    fn initial_auth_code_limits_are_per_client_and_canonicalize_addresses() {
        let challenge = SetupChallenge::default();
        let code = challenge.initialize().unwrap();
        let now = Instant::now();
        for _ in 0..5 {
            assert!(challenge.verify_at(CLIENT, None, now).is_err());
        }
        assert_eq!(
            challenge
                .verify_at("::ffff:127.0.0.1".parse().unwrap(), Some(&code), now)
                .unwrap_err()
                .validation_reason(),
            Some(refusal::CODE_RATE_LIMITED)
        );
        challenge
            .verify_at("192.0.2.2".parse().unwrap(), Some(&code), now)
            .unwrap();
        challenge.initialize().unwrap();
        assert!(challenge.state.lock().unwrap().failures.is_empty());
    }

    #[test]
    fn initial_auth_code_client_tracking_is_bounded_without_a_global_lockout() {
        let challenge = SetupChallenge::default();
        let code = challenge.initialize().unwrap();
        let now = Instant::now();
        for index in 0..=MAX_SETUP_CLIENTS {
            let ip = IpAddr::V6(std::net::Ipv6Addr::from(index as u128 + (1u128 << 64)));
            assert_eq!(
                challenge
                    .verify_at(ip, None, now)
                    .unwrap_err()
                    .validation_reason(),
                Some(refusal::CODE_INVALID)
            );
        }
        assert_eq!(
            challenge.state.lock().unwrap().failures.len(),
            MAX_SETUP_CLIENTS
        );
        challenge.verify_at(CLIENT, Some(&code), now).unwrap();
        challenge.consume();
        assert!(challenge.state.lock().unwrap().failures.is_empty());
    }

    #[test]
    fn initial_auth_challenge_without_code_admits_until_consumed() {
        let challenge = SetupChallenge::default();
        challenge.initialize_without_code();
        assert!(!challenge.code_required());
        assert!(challenge.is_available());
        challenge.verify(CLIENT, None).unwrap();
        challenge.consume();
        assert_eq!(
            challenge
                .verify(CLIENT, None)
                .unwrap_err()
                .validation_reason(),
            Some(refusal::ALREADY_COMPLETED)
        );
    }

    #[test]
    fn initial_auth_initialized_challenge_requires_a_code_and_is_single_use() {
        let challenge = SetupChallenge::default();
        assert!(challenge.verify(CLIENT, None).is_err());
        let code = challenge.initialize().unwrap();
        assert!(challenge.code_required());
        assert_eq!(
            challenge
                .verify(CLIENT, None)
                .unwrap_err()
                .validation_reason(),
            Some(refusal::CODE_INVALID)
        );
        challenge.verify(CLIENT, Some(&code)).unwrap();
        challenge.consume();
        assert!(challenge.verify(CLIENT, None).is_err());
    }

    #[test]
    fn initial_auth_pending_replacement_rejects_disabled_form_login() {
        let pending = SetupState::PasswordReplacement {
            user_id: "operator".into(),
        };
        assert_eq!(
            pending
                .validate_runtime_mode(false)
                .unwrap_err()
                .validation_reason(),
            Some(refusal::MODE_CONFLICT)
        );
        pending.validate_runtime_mode(true).unwrap();
        for state in [SetupState::Choice, SetupState::Complete] {
            state.validate_runtime_mode(false).unwrap();
            state.validate_runtime_mode(true).unwrap();
        }
    }

    #[test]
    fn initial_auth_refusal_codes_are_distinct_and_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for code in refusal::ALL {
            assert!(code.starts_with("INITIAL_AUTH_"));
            assert!(code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'));
            assert!(seen.insert(code));
        }
    }
}
