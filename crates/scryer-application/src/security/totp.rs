use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Duration, Utc};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TotpProfile {
    algorithm: TotpAlgorithm,
    digits: i32,
    period_seconds: i32,
}

const DEFAULT_TOTP_PROFILE: TotpProfile = TotpProfile {
    algorithm: TotpAlgorithm::Sha1,
    digits: 6,
    period_seconds: 30,
};
const TOTP_SECRET_BYTES: usize = 20;
const TOTP_ENROLLMENT_TTL_MINUTES: i64 = 10;
pub(super) const MFA_FRESHNESS_TTL_MINUTES: i64 = 60;
const TOTP_ALLOWED_DRIFT_STEPS: i64 = 1;
const TOTP_FAILED_ATTEMPT_LIMIT: i64 = 5;
const TOTP_FAILED_ATTEMPT_WINDOW_MINUTES: i64 = 5;
const TOTP_RECOVERY_CODE_COUNT: usize = 10;
const TOTP_RECOVERY_CODE_BYTES: usize = 16;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

impl TotpAlgorithm {
    fn parse(value: &str) -> AppResult<Self> {
        match value.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            "SHA512" => Ok(Self::Sha512),
            _ => Err(AppError::Repository(format!(
                "unsupported TOTP algorithm {value}"
            ))),
        }
    }

    fn storage_value(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha512 => "SHA512",
        }
    }

    fn hmac_algorithm(self) -> hmac::Algorithm {
        match self {
            Self::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => hmac::HMAC_SHA256,
            Self::Sha512 => hmac::HMAC_SHA512,
        }
    }
}

impl TotpProfile {
    fn new(algorithm: TotpAlgorithm, digits: i32, period_seconds: i32) -> AppResult<Self> {
        if !matches!(digits, 6 | 8) {
            return Err(AppError::Repository(format!(
                "unsupported TOTP digit count {digits}"
            )));
        }
        if period_seconds <= 0 {
            return Err(AppError::Repository(format!(
                "unsupported TOTP period {period_seconds}"
            )));
        }
        Ok(Self {
            algorithm,
            digits,
            period_seconds,
        })
    }

    fn from_credential(credential: &TotpCredentialRecord) -> AppResult<Self> {
        Self::new(
            TotpAlgorithm::parse(&credential.algorithm)?,
            credential.digits,
            credential.period_seconds,
        )
    }

    fn from_enrollment_challenge(challenge: &TotpEnrollmentChallengeRecord) -> AppResult<Self> {
        Self::new(
            TotpAlgorithm::parse(&challenge.algorithm)?,
            challenge.digits,
            challenge.period_seconds,
        )
    }
}

impl AppUseCase {
    pub async fn totp_status(&self, actor: &User) -> AppResult<TotpStatus> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        let credential = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?;
        self.totp_status_from_credential(actor, credential).await
    }

    pub async fn totp_enrollment_start(&self, actor: &User) -> AppResult<TotpEnrollmentStart> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.totp_enrollment_start_inner(actor).await
    }

    pub async fn start_login_mfa_enrollment(&self, actor: &User) -> AppResult<TotpEnrollmentStart> {
        self.totp_enrollment_start_inner(actor).await
    }

    async fn totp_enrollment_start_inner(&self, actor: &User) -> AppResult<TotpEnrollmentStart> {
        self.cleanup_expired_totp_enrollment_challenges().await?;

        if self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation("TOTP is already enabled".into()));
        }

        let now = Utc::now();
        let profile = DEFAULT_TOTP_PROFILE;
        let secret_base32 = generate_base32_secret(TOTP_SECRET_BYTES)?;
        let challenge = TotpEnrollmentChallengeRecord {
            id: Id::new().0,
            user_id: actor.id.clone(),
            secret_base32: secret_base32.clone(),
            algorithm: profile.algorithm.storage_value().to_string(),
            digits: profile.digits,
            period_seconds: profile.period_seconds,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::minutes(TOTP_ENROLLMENT_TTL_MINUTES)).to_rfc3339(),
        };
        self.services
            .identity
            .totp
            .create_enrollment_challenge(challenge.clone())
            .await?;

        Ok(TotpEnrollmentStart {
            challenge_id: challenge.id,
            otpauth_url: totp_otpauth_url(&actor.username, &secret_base32),
            secret_base32,
            expires_at: challenge.expires_at,
        })
    }

    pub async fn totp_enrollment_complete(
        &self,
        actor: &User,
        challenge_id: &str,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.totp_enrollment_complete_inner(actor, challenge_id, code)
            .await
    }

    pub async fn complete_login_mfa_enrollment(
        &self,
        actor: &User,
        challenge_id: &str,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.totp_enrollment_complete_inner(actor, challenge_id, code)
            .await
    }

    async fn totp_enrollment_complete_inner(
        &self,
        actor: &User,
        challenge_id: &str,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.cleanup_expired_totp_enrollment_challenges().await?;

        let challenge = self
            .services
            .identity
            .totp
            .get_enrollment_challenge(challenge_id, &actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("TOTP challenge {challenge_id}")))?;

        if timestamp_expired(&challenge.expires_at) {
            self.services
                .identity
                .totp
                .delete_enrollment_challenge(challenge_id, &actor.id)
                .await?;
            return Err(AppError::Validation(
                "TOTP enrollment challenge has expired".into(),
            ));
        }

        let profile = TotpProfile::from_enrollment_challenge(&challenge)?;
        let normalized_code = normalize_totp_code(code, profile.digits)?;
        let secret = base32_decode(&challenge.secret_base32)?;
        let now = Utc::now();
        let Some(step) = matching_totp_step(&secret, &normalized_code, now, profile)? else {
            return Err(AppError::TotpInvalidCode(
                "TOTP code did not match the enrollment secret".into(),
            ));
        };

        let credential = TotpCredentialRecord {
            id: Id::new().0,
            user_id: actor.id.clone(),
            secret_base32: challenge.secret_base32.clone(),
            algorithm: profile.algorithm.storage_value().to_string(),
            digits: profile.digits,
            period_seconds: profile.period_seconds,
            last_accepted_step: Some(step),
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            last_used_at: None,
        };
        self.services
            .identity
            .totp
            .upsert_credential(credential.clone())
            .await?;
        self.services
            .identity
            .totp
            .delete_enrollment_challenge(challenge_id, &actor.id)
            .await?;

        let recovery_codes = self.replace_totp_recovery_codes(actor).await?;
        let status = self
            .totp_status_from_credential(actor, Some(credential))
            .await?;
        Ok(TotpEnrollmentComplete {
            status,
            recovery_codes,
        })
    }

    pub async fn mfa_verify_step_up(&self, actor: &User, code: &str) -> AppResult<DateTime<Utc>> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.verify_totp_for_user(actor, code).await
    }

    pub async fn totp_disable(&self, actor: &User, code: &str) -> AppResult<TotpStatus> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.verify_totp_for_user(actor, code).await?;
        self.services
            .identity
            .totp
            .delete_credential_for_user(&actor.id)
            .await?;
        self.services
            .identity
            .totp
            .replace_recovery_codes(&actor.id, Vec::new())
            .await?;
        self.services
            .identity
            .totp
            .clear_failed_attempts(&actor.id)
            .await?;
        self.totp_status(actor).await
    }

    pub async fn totp_regenerate_recovery_codes(
        &self,
        actor: &User,
        code: &str,
    ) -> AppResult<TotpEnrollmentComplete> {
        self.require_actor_capability(actor, scryer_domain::ActorCapability::ManageOwnAccount)
            .await?;
        self.verify_totp_for_user(actor, code).await?;
        let recovery_codes = self.replace_totp_recovery_codes(actor).await?;
        let status = self.totp_status(actor).await?;
        Ok(TotpEnrollmentComplete {
            status,
            recovery_codes,
        })
    }

    pub async fn require_mfa_step_up(
        &self,
        actor: &User,
        mfa_step_up_verified_until: Option<i64>,
    ) -> AppResult<()> {
        let settings = self.security_settings().await?;
        if !settings.mfa_require_config_step_up {
            return Ok(());
        }

        if mfa_step_up_verified_until.is_some_and(|expires_at| expires_at > Utc::now().timestamp())
        {
            return Ok(());
        }

        let credential = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?;
        if credential.is_none() {
            return Err(AppError::TotpEnrollmentRequired(
                "MFA enrollment is required before changing system configuration".into(),
            ));
        }

        Err(AppError::MfaStepUpRequired(
            "MFA verification is required before changing system configuration".into(),
        ))
    }

    pub async fn require_api_key_mfa_step_up(
        &self,
        actor: &User,
        mfa_step_up_verified_until: Option<i64>,
    ) -> AppResult<()> {
        if mfa_step_up_verified_until.is_some_and(|expires_at| expires_at > Utc::now().timestamp())
        {
            return Ok(());
        }

        if self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?
            .is_none()
        {
            return Ok(());
        }

        Err(AppError::MfaStepUpRequired(
            "MFA verification is required before creating an API key".into(),
        ))
    }

    pub async fn verify_totp_for_user(&self, actor: &User, code: &str) -> AppResult<DateTime<Utc>> {
        let Some(mut credential) = self
            .services
            .identity
            .totp
            .get_credential_for_user(&actor.id)
            .await?
        else {
            return Err(AppError::TotpEnrollmentRequired(
                "TOTP enrollment is required".into(),
            ));
        };

        self.ensure_totp_attempt_allowed(&actor.id).await?;
        match self
            .verify_totp_or_recovery_code(&mut credential, code)
            .await
        {
            Ok(()) => {
                self.services
                    .identity
                    .totp
                    .clear_failed_attempts(&actor.id)
                    .await?;
                Ok(Utc::now() + Duration::minutes(MFA_FRESHNESS_TTL_MINUTES))
            }
            Err(error) => {
                self.record_totp_failed_attempt(&actor.id).await?;
                Err(error)
            }
        }
    }

    async fn verify_totp_or_recovery_code(
        &self,
        credential: &mut TotpCredentialRecord,
        code: &str,
    ) -> AppResult<()> {
        let profile = match TotpProfile::from_credential(credential) {
            Ok(profile) => profile,
            Err(_) => {
                return match self
                    .verify_totp_recovery_code(&credential.user_id, code)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(_) => Err(AppError::TotpInvalidCode("invalid TOTP code".into())),
                };
            }
        };
        if let Ok(normalized_code) = normalize_totp_code(code, profile.digits) {
            let secret = base32_decode(&credential.secret_base32)?;
            let now = Utc::now();
            if let Some(step) = matching_totp_step(&secret, &normalized_code, now, profile)?
                && accepts_totp_step(step, credential.last_accepted_step)
            {
                credential.last_accepted_step = Some(step);
                credential.last_used_at = Some(now.to_rfc3339());
                credential.updated_at = now.to_rfc3339();
                self.services
                    .identity
                    .totp
                    .upsert_credential(credential.clone())
                    .await?;
                return Ok(());
            }
        }

        self.verify_totp_recovery_code(&credential.user_id, code)
            .await
    }

    async fn verify_totp_recovery_code(&self, user_id: &str, code: &str) -> AppResult<()> {
        let normalized = normalize_recovery_code(code);
        if normalized.is_empty() {
            return Err(AppError::TotpInvalidCode("invalid TOTP code".into()));
        }

        let recovery_codes = self
            .services
            .identity
            .totp
            .list_recovery_codes_for_user(user_id)
            .await?;
        for recovery_code in recovery_codes {
            if self.validate_password(&normalized, &recovery_code.code_hash)? {
                if recovery_code.used_at.is_some() {
                    return Err(AppError::TotpRecoveryCodeUsed(
                        "TOTP recovery code was already used".into(),
                    ));
                }
                self.services
                    .identity
                    .totp
                    .mark_recovery_code_used(&recovery_code.id, user_id, &Utc::now().to_rfc3339())
                    .await?;
                return Ok(());
            }
        }

        Err(AppError::TotpInvalidCode("invalid TOTP code".into()))
    }

    async fn replace_totp_recovery_codes(&self, actor: &User) -> AppResult<Vec<String>> {
        let now = Utc::now().to_rfc3339();
        let mut display_codes = Vec::with_capacity(TOTP_RECOVERY_CODE_COUNT);
        let mut records = Vec::with_capacity(TOTP_RECOVERY_CODE_COUNT);
        for _ in 0..TOTP_RECOVERY_CODE_COUNT {
            let normalized = generate_base32_secret(TOTP_RECOVERY_CODE_BYTES)?;
            let display = group_recovery_code(&normalized);
            records.push(TotpRecoveryCodeRecord {
                id: Id::new().0,
                user_id: actor.id.clone(),
                code_hash: self.hash_password(&normalized)?,
                created_at: now.clone(),
                used_at: None,
            });
            display_codes.push(display);
        }
        self.services
            .identity
            .totp
            .replace_recovery_codes(&actor.id, records)
            .await?;
        Ok(display_codes)
    }

    async fn totp_status_from_credential(
        &self,
        actor: &User,
        credential: Option<TotpCredentialRecord>,
    ) -> AppResult<TotpStatus> {
        let recovery_codes_remaining = self
            .services
            .identity
            .totp
            .list_recovery_codes_for_user(&actor.id)
            .await?
            .into_iter()
            .filter(|code| code.used_at.is_none())
            .count() as i32;
        Ok(TotpStatus {
            enabled: credential.is_some(),
            created_at: credential.as_ref().map(|record| record.created_at.clone()),
            last_used_at: credential.and_then(|record| record.last_used_at),
            recovery_codes_remaining,
        })
    }

    async fn cleanup_expired_totp_enrollment_challenges(&self) -> AppResult<()> {
        self.services
            .identity
            .totp
            .delete_expired_enrollment_challenges(&Utc::now().to_rfc3339())
            .await?;
        Ok(())
    }

    async fn ensure_totp_attempt_allowed(&self, user_id: &str) -> AppResult<()> {
        let since =
            (Utc::now() - Duration::minutes(TOTP_FAILED_ATTEMPT_WINDOW_MINUTES)).to_rfc3339();
        let attempts = self
            .services
            .identity
            .totp
            .count_failed_attempts_since(user_id, &since)
            .await?;
        if attempts >= TOTP_FAILED_ATTEMPT_LIMIT {
            return Err(AppError::TotpInvalidCode(
                "too many invalid TOTP attempts; try again shortly".into(),
            ));
        }
        Ok(())
    }

    async fn record_totp_failed_attempt(&self, user_id: &str) -> AppResult<()> {
        self.services
            .identity
            .totp
            .record_failed_attempt(TotpFailedAttemptRecord {
                id: Id::new().0,
                user_id: user_id.to_string(),
                attempted_at: Utc::now().to_rfc3339(),
            })
            .await
    }
}

fn generate_base32_secret(byte_count: usize) -> AppResult<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0_u8; byte_count];
    rng.fill(&mut bytes).map_err(|error| {
        AppError::Repository(format!("failed to generate TOTP secret: {error}"))
    })?;
    Ok(base32_encode_no_pad(&bytes))
}

fn matching_totp_step(
    secret: &[u8],
    normalized_code: &str,
    now: DateTime<Utc>,
    profile: TotpProfile,
) -> AppResult<Option<i64>> {
    let current_step = now.timestamp() / i64::from(profile.period_seconds);
    for offset in -TOTP_ALLOWED_DRIFT_STEPS..=TOTP_ALLOWED_DRIFT_STEPS {
        let step = current_step + offset;
        if step < 0 {
            continue;
        }
        let expected = hotp(secret, step as u64, profile)?;
        if constant_time_eq(expected.as_bytes(), normalized_code.as_bytes()) {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

fn hotp(secret: &[u8], counter: u64, profile: TotpProfile) -> AppResult<String> {
    let profile = TotpProfile::new(profile.algorithm, profile.digits, profile.period_seconds)?;
    let key = hmac::Key::new(profile.algorithm.hmac_algorithm(), secret);
    let tag = hmac::sign(&key, &counter.to_be_bytes());
    let digest = tag.as_ref();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    let modulus = 10_u32.pow(profile.digits as u32);
    Ok(format!(
        "{:0width$}",
        value % modulus,
        width = profile.digits as usize
    ))
}

fn normalize_totp_code(code: &str, digits: i32) -> AppResult<String> {
    let normalized = code
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if !matches!(digits, 6 | 8)
        || normalized.len() != digits as usize
        || !normalized.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(AppError::TotpInvalidCode("invalid TOTP code".into()));
    }
    Ok(normalized)
}

fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn group_recovery_code(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

fn timestamp_expired(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn totp_otpauth_url(username: &str, secret_base32: &str) -> String {
    let issuer = "Scryer";
    let profile = DEFAULT_TOTP_PROFILE;
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm={}&digits={}&period={}",
        percent_encode_component(issuer),
        percent_encode_component(username),
        secret_base32,
        percent_encode_component(issuer),
        profile.algorithm.storage_value(),
        profile.digits,
        profile.period_seconds
    )
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn base32_encode_no_pad(bytes: &[u8]) -> String {
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0_u16;
    let mut bits_left = 0_u8;

    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits_left += 8;
        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            output.push(char::from(BASE32_ALPHABET[index]));
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        output.push(char::from(BASE32_ALPHABET[index]));
    }

    output
}

fn base32_decode(value: &str) -> AppResult<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0_u32;
    let mut bits_left = 0_u8;

    for ch in value.chars().filter(|ch| *ch != '=') {
        let Some(index) = base32_index(ch) else {
            return Err(AppError::Repository("stored TOTP secret is invalid".into()));
        };
        buffer = (buffer << 5) | u32::from(index);
        bits_left += 5;
        if bits_left >= 8 {
            output.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }

    Ok(output)
}

fn accepts_totp_step(step: i64, last_accepted_step: Option<i64>) -> bool {
    last_accepted_step.is_none_or(|last_step| step > last_step)
}

fn base32_index(ch: char) -> Option<u8> {
    match ch.to_ascii_uppercase() {
        'A'..='Z' => Some(ch.to_ascii_uppercase() as u8 - b'A'),
        '2'..='7' => Some(ch as u8 - b'2' + 26),
        _ => None,
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.iter().zip(right) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn base32_round_trips_without_padding() {
        let bytes = b"hello scryer";
        let encoded = base32_encode_no_pad(bytes);
        assert_eq!(base32_decode(&encoded).unwrap(), bytes);
        assert!(!encoded.contains('='));
    }

    #[test]
    fn hotp_sha1_matches_rfc6238_test_vector() {
        let secret = b"12345678901234567890";
        let profile = TotpProfile::new(TotpAlgorithm::Sha1, 8, 30).unwrap();
        assert_eq!(hotp(secret, 59 / 30, profile).unwrap(), "94287082");
    }

    #[test]
    fn default_profile_generates_six_digit_codes() {
        let code = hotp(b"12345678901234567890", 1, DEFAULT_TOTP_PROFILE).unwrap();
        assert_eq!(DEFAULT_TOTP_PROFILE.digits, 6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn otpauth_url_uses_explicit_1password_parameters() {
        let url = totp_otpauth_url("jen@example.test", "JBSWY3DPEHPK3PXP");
        assert_eq!(
            url,
            "otpauth://totp/Scryer:jen%40example.test?secret=JBSWY3DPEHPK3PXP&issuer=Scryer&algorithm=SHA1&digits=6&period=30"
        );
    }

    #[test]
    fn matching_totp_uses_stored_profile() {
        let secret = b"12345678901234567890123456789012";
        let profile = TotpProfile::new(TotpAlgorithm::Sha256, 8, 30).unwrap();
        let code = hotp(secret, 59 / 30, profile).unwrap();
        let now = Utc.timestamp_opt(59, 0).single().unwrap();

        assert_eq!(
            matching_totp_step(secret, &code, now, profile).unwrap(),
            Some(1)
        );
        assert_eq!(
            matching_totp_step(secret, &code, now, DEFAULT_TOTP_PROFILE).unwrap(),
            None
        );
    }

    #[test]
    fn matching_totp_accepts_adjacent_drift_step() {
        let secret = b"12345678901234567890";
        let now = Utc.timestamp_opt(300, 0).single().unwrap();
        let previous_code = hotp(secret, 9, DEFAULT_TOTP_PROFILE).unwrap();
        let next_code = hotp(secret, 11, DEFAULT_TOTP_PROFILE).unwrap();

        assert_eq!(
            matching_totp_step(secret, &previous_code, now, DEFAULT_TOTP_PROFILE).unwrap(),
            Some(9)
        );
        assert_eq!(
            matching_totp_step(secret, &next_code, now, DEFAULT_TOTP_PROFILE).unwrap(),
            Some(11)
        );
    }

    #[test]
    fn accepted_totp_step_rejects_replay() {
        assert!(accepts_totp_step(42, None));
        assert!(accepts_totp_step(42, Some(41)));
        assert!(!accepts_totp_step(42, Some(42)));
        assert!(!accepts_totp_step(42, Some(43)));
    }

    #[test]
    fn normalize_totp_code_rejects_invalid_values() {
        assert_eq!(normalize_totp_code("123 456", 6).unwrap(), "123456");
        assert!(normalize_totp_code("12345", 6).is_err());
        assert!(normalize_totp_code("1234567", 6).is_err());
        assert!(normalize_totp_code("12345a", 6).is_err());
        assert!(normalize_totp_code("1234567", 7).is_err());
    }

    #[test]
    fn enrollment_challenge_uses_stored_profile() {
        let challenge = TotpEnrollmentChallengeRecord {
            id: "challenge_1".into(),
            user_id: "user_1".into(),
            secret_base32: "JBSWY3DPEHPK3PXP".into(),
            algorithm: "SHA512".into(),
            digits: 8,
            period_seconds: 45,
            created_at: Utc::now().to_rfc3339(),
            expires_at: Utc::now().to_rfc3339(),
        };

        let profile = TotpProfile::from_enrollment_challenge(&challenge).unwrap();
        assert_eq!(profile.algorithm, TotpAlgorithm::Sha512);
        assert_eq!(profile.digits, 8);
        assert_eq!(profile.period_seconds, 45);
    }

    #[test]
    fn unsupported_stored_profile_fails_cleanly() {
        let credential = TotpCredentialRecord {
            id: "totp_1".into(),
            user_id: "user_1".into(),
            secret_base32: "JBSWY3DPEHPK3PXP".into(),
            algorithm: "MD5".into(),
            digits: 6,
            period_seconds: 30,
            last_accepted_step: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            last_used_at: None,
        };

        assert!(TotpProfile::from_credential(&credential).is_err());
    }
}
