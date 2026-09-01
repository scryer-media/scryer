use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, TotpCredentialRecord, TotpEnrollmentChallengeRecord,
    TotpFailedAttemptRecord, TotpRecoveryCodeRecord, TotpRepository,
};
use std::sync::RwLock;

use crate::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
use crate::settings::crypto::{current_encryption_key, decrypt_value, maybe_encrypt_value};
use crate::workflow::stores::{opt_timestamp_string, timestamp_string};

#[derive(Clone)]
pub struct TotpStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl TotpStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }
}

#[async_trait]
impl TotpRepository for TotpStore {
    async fn get_credential_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Option<TotpCredentialRecord>> {
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        load_credential_for_user(self.datastore.read_exec(), user_id, encryption_key.as_ref()).await
    }

    async fn upsert_credential(
        &self,
        credential: TotpCredentialRecord,
    ) -> AppResult<TotpCredentialRecord> {
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_totp_credential", move |tx| {
            let credential = credential.clone();
            let encryption_key = encryption_key.clone();
            Box::pin(async move {
                tx.execute(
                    "INSERT INTO totp_credentials
                     (id, user_id, secret_base32, algorithm, digits, period_seconds, last_accepted_step, created_at, updated_at, last_used_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                     ON CONFLICT (user_id) DO UPDATE SET
                        secret_base32 = excluded.secret_base32,
                        algorithm = excluded.algorithm,
                        digits = excluded.digits,
                        period_seconds = excluded.period_seconds,
                        last_accepted_step = excluded.last_accepted_step,
                        updated_at = excluded.updated_at,
                        last_used_at = excluded.last_used_at",
                    &credential_args(&credential, encryption_key.as_ref())?,
                )
                .await?;
                load_credential_for_user(
                    SqlExec::Tx(tx),
                    &credential.user_id,
                    encryption_key.as_ref(),
                )
                .await?
                .ok_or_else(|| AppError::NotFound(format!("TOTP credential {}", credential.id)))
            })
        })
        .await
    }

    async fn delete_credential_for_user(&self, user_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_totp_credential",
            "DELETE FROM totp_credentials WHERE user_id = {}",
            vec![SqlArg::Text(user_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn create_enrollment_challenge(
        &self,
        challenge: TotpEnrollmentChallengeRecord,
    ) -> AppResult<TotpEnrollmentChallengeRecord> {
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_totp_enrollment_challenge",
            move |tx| {
                let challenge = challenge.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    tx.execute(
                        "DELETE FROM totp_enrollment_challenges WHERE user_id = {}",
                        &[SqlArg::Text(challenge.user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "INSERT INTO totp_enrollment_challenges
                         (id, user_id, auth_session_version, secret_base32, algorithm, digits, period_seconds, created_at, expires_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &challenge_args(&challenge, encryption_key.as_ref())?,
                    )
                    .await?;
                    load_enrollment_challenge(
                        SqlExec::Tx(tx),
                        &challenge.id,
                        &challenge.user_id,
                        encryption_key.as_ref(),
                    )
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("TOTP challenge {}", challenge.id)))
                })
            },
        )
        .await
    }

    async fn get_enrollment_challenge(
        &self,
        id: &str,
        user_id: &str,
    ) -> AppResult<Option<TotpEnrollmentChallengeRecord>> {
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        load_enrollment_challenge(
            self.datastore.read_exec(),
            id,
            user_id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn delete_enrollment_challenge(&self, id: &str, user_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_totp_enrollment_challenge",
            "DELETE FROM totp_enrollment_challenges WHERE id = {} AND user_id = {}",
            vec![
                SqlArg::Text(id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn delete_enrollment_challenges_for_user(&self, user_id: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "delete_totp_enrollment_challenges_for_user",
            "DELETE FROM totp_enrollment_challenges WHERE user_id = {}",
            vec![SqlArg::Text(user_id.to_string())],
        )
        .await
    }

    async fn delete_expired_enrollment_challenges(&self, now: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "delete_expired_totp_enrollment_challenges",
            "DELETE FROM totp_enrollment_challenges WHERE expires_at <= {}",
            vec![timestamp_arg(now)?],
        )
        .await
    }

    async fn reset_user_mfa_and_invalidate_sessions(
        &self,
        user_id: &str,
        auth_session_version: &str,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "reset_user_mfa_and_invalidate_sessions",
            move |tx| {
                let user_id = user_id.clone();
                let auth_session_version = auth_session_version.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "UPDATE users SET auth_session_version = {} WHERE id = {}",
                            &[
                                SqlArg::Text(auth_session_version),
                                SqlArg::Text(user_id.clone()),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!("user {user_id}")));
                    }

                    tx.execute(
                        "DELETE FROM totp_credentials WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_recovery_codes WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_failed_attempts WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_enrollment_challenges WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM webauthn_challenges WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM webauthn_credentials WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM login_verification_challenges WHERE user_id = {}",
                        &[SqlArg::Text(user_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn complete_enrollment_for_current_session(
        &self,
        credential: TotpCredentialRecord,
        challenge_id: &str,
        recovery_codes: Vec<TotpRecoveryCodeRecord>,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        let encryption_key = current_encryption_key(&self.encryption_key)?;
        let challenge_id = challenge_id.to_string();
        let expected_auth_session_version = expected_auth_session_version.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "complete_totp_enrollment_for_current_session",
            move |tx| {
                let credential = credential.clone();
                let challenge_id = challenge_id.clone();
                let recovery_codes = recovery_codes.clone();
                let encryption_key = encryption_key.clone();
                let expected_auth_session_version = expected_auth_session_version.clone();
                Box::pin(async move {
                    let session_rows = tx.execute(
                        "UPDATE users SET auth_session_version = auth_session_version \
                         WHERE id = {} AND (auth_session_version = {} \
                         OR (auth_session_version IS NULL AND {} IS NULL))",
                        &[
                            SqlArg::Text(credential.user_id.clone()),
                            SqlArg::OptText(expected_auth_session_version.clone()),
                            SqlArg::OptText(expected_auth_session_version),
                        ],
                    )
                    .await?;
                    if session_rows == 0 {
                        return Err(AppError::Unauthorized(
                            "TOTP enrollment session is no longer current".into(),
                        ));
                    }
                    let challenge_rows = tx
                        .execute(
                            "DELETE FROM totp_enrollment_challenges WHERE id = {} AND user_id = {}",
                            &[
                                SqlArg::Text(challenge_id.clone()),
                                SqlArg::Text(credential.user_id.clone()),
                            ],
                        )
                        .await?;
                    if challenge_rows == 0 {
                        return Err(AppError::NotFound(format!(
                            "TOTP challenge {}",
                            challenge_id
                        )));
                    }
                    tx.execute(
                        "INSERT INTO totp_credentials
                         (id, user_id, secret_base32, algorithm, digits, period_seconds, last_accepted_step, created_at, updated_at, last_used_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &credential_args(&credential, encryption_key.as_ref())?,
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_recovery_codes WHERE user_id = {}",
                        &[SqlArg::Text(credential.user_id.clone())],
                    )
                    .await?;
                    for recovery_code in recovery_codes {
                        tx.execute(
                            "INSERT INTO totp_recovery_codes
                             (id, user_id, code_hash, created_at, used_at)
                             VALUES ({}, {}, {}, {}, {})",
                            &recovery_code_args(&recovery_code)?,
                        )
                        .await?;
                    }
                    tx.execute(
                        "DELETE FROM totp_enrollment_challenges WHERE user_id = {}",
                        &[SqlArg::Text(credential.user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_failed_attempts WHERE user_id = {}",
                        &[SqlArg::Text(credential.user_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn disable_for_current_session(
        &self,
        user_id: &str,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        let expected_auth_session_version = expected_auth_session_version.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "disable_totp_for_current_session",
            move |tx| {
                let user_id = user_id.clone();
                let expected_auth_session_version = expected_auth_session_version.clone();
                Box::pin(async move {
                    let session_rows = tx
                        .execute(
                            "UPDATE users SET auth_session_version = auth_session_version \
                             WHERE id = {} AND (auth_session_version = {} \
                             OR (auth_session_version IS NULL AND {} IS NULL))",
                            &[
                                SqlArg::Text(user_id.clone()),
                                SqlArg::OptText(expected_auth_session_version.clone()),
                                SqlArg::OptText(expected_auth_session_version),
                            ],
                        )
                        .await?;
                    if session_rows == 0 {
                        return Err(AppError::Unauthorized(
                            "TOTP disablement session is no longer current".into(),
                        ));
                    }
                    tx.execute(
                        "DELETE FROM totp_credentials WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_recovery_codes WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_failed_attempts WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    tx.execute(
                        "DELETE FROM totp_enrollment_challenges WHERE user_id = {}",
                        &[SqlArg::Text(user_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn replace_recovery_codes_for_current_session(
        &self,
        user_id: &str,
        codes: Vec<TotpRecoveryCodeRecord>,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        let codes = codes.clone();
        let expected_auth_session_version = expected_auth_session_version.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_totp_recovery_codes_for_current_session",
            move |tx| {
                let user_id = user_id.clone();
                let codes = codes.clone();
                let expected_auth_session_version = expected_auth_session_version.clone();
                Box::pin(async move {
                    let session_rows = tx
                        .execute(
                            "UPDATE users SET auth_session_version = auth_session_version \
                             WHERE id = {} AND (auth_session_version = {} \
                             OR (auth_session_version IS NULL AND {} IS NULL))",
                            &[
                                SqlArg::Text(user_id.clone()),
                                SqlArg::OptText(expected_auth_session_version.clone()),
                                SqlArg::OptText(expected_auth_session_version),
                            ],
                        )
                        .await?;
                    if session_rows == 0 {
                        return Err(AppError::Unauthorized(
                            "recovery-code replacement session is no longer current".into(),
                        ));
                    }
                    tx.execute(
                        "DELETE FROM totp_recovery_codes WHERE user_id = {}",
                        &[SqlArg::Text(user_id.clone())],
                    )
                    .await?;
                    for recovery_code in codes {
                        tx.execute(
                            "INSERT INTO totp_recovery_codes
                             (id, user_id, code_hash, created_at, used_at)
                             VALUES ({}, {}, {}, {}, {})",
                            &recovery_code_args(&recovery_code)?,
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn replace_recovery_codes(
        &self,
        user_id: &str,
        codes: Vec<TotpRecoveryCodeRecord>,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_totp_recovery_codes", move |tx| {
            let user_id = user_id.clone();
            let codes = codes.clone();
            Box::pin(async move {
                tx.execute(
                    "DELETE FROM totp_recovery_codes WHERE user_id = {}",
                    &[SqlArg::Text(user_id)],
                )
                .await?;
                for code in codes {
                    tx.execute(
                        "INSERT INTO totp_recovery_codes
                         (id, user_id, code_hash, created_at, used_at)
                         VALUES ({}, {}, {}, {}, {})",
                        &recovery_code_args(&code)?,
                    )
                    .await?;
                }
                Ok(())
            })
        })
        .await
    }

    async fn list_recovery_codes_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<TotpRecoveryCodeRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, code_hash, created_at, used_at
             FROM totp_recovery_codes
             WHERE user_id = {}
             ORDER BY created_at ASC, id ASC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_recovery_code).collect()
    }

    async fn mark_recovery_code_used(
        &self,
        id: &str,
        user_id: &str,
        used_at: &str,
    ) -> AppResult<()> {
        let rows = execute_write(
            &self.datastore,
            "mark_totp_recovery_code_used",
            "UPDATE totp_recovery_codes
                SET used_at = {}
              WHERE id = {} AND user_id = {} AND used_at IS NULL",
            vec![
                timestamp_arg(used_at)?,
                SqlArg::Text(id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::TotpRecoveryCodeUsed(
                "TOTP recovery code was already used".into(),
            ));
        }
        Ok(())
    }

    async fn reserve_totp_attempt(
        &self,
        user_id: &str,
        attempted_at: &str,
        window_started_after: &str,
        limit: i32,
    ) -> AppResult<bool> {
        let rows = execute_write(
            &self.datastore,
            "reserve_totp_attempt",
            "UPDATE totp_credentials
             SET attempt_window_started_at = CASE
                    WHEN attempt_window_started_at IS NULL OR attempt_window_started_at <= {}
                        THEN {}
                    ELSE attempt_window_started_at
                 END,
                 attempt_count = CASE
                    WHEN attempt_window_started_at IS NULL OR attempt_window_started_at <= {}
                        THEN 1
                    ELSE attempt_count + 1
                 END
             WHERE user_id = {}
               AND (attempt_window_started_at IS NULL
                    OR attempt_window_started_at <= {}
                    OR attempt_count < {})",
            vec![
                timestamp_arg(window_started_after)?,
                timestamp_arg(attempted_at)?,
                timestamp_arg(window_started_after)?,
                SqlArg::Text(user_id.to_string()),
                timestamp_arg(window_started_after)?,
                SqlArg::I32(limit),
            ],
        )
        .await?;
        Ok(rows == 1)
    }

    async fn clear_totp_attempt_reservations(&self, user_id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "clear_totp_attempt_reservations",
            "UPDATE totp_credentials
             SET attempt_window_started_at = NULL, attempt_count = 0
             WHERE user_id = {}",
            vec![SqlArg::Text(user_id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn claim_totp_step(&self, user_id: &str, step: i64, used_at: &str) -> AppResult<bool> {
        let rows = execute_write(
            &self.datastore,
            "claim_totp_step",
            "UPDATE totp_credentials
             SET last_accepted_step = {}, last_used_at = {}, updated_at = {}
             WHERE user_id = {}
               AND (last_accepted_step IS NULL OR last_accepted_step < {})",
            vec![
                SqlArg::I64(step),
                timestamp_arg(used_at)?,
                timestamp_arg(used_at)?,
                SqlArg::Text(user_id.to_string()),
                SqlArg::I64(step),
            ],
        )
        .await?;
        Ok(rows == 1)
    }

    async fn record_failed_attempt(&self, attempt: TotpFailedAttemptRecord) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "record_totp_failed_attempt",
            "INSERT INTO totp_failed_attempts (id, user_id, attempted_at)
             VALUES ({}, {}, {})",
            vec![
                SqlArg::Text(attempt.id),
                SqlArg::Text(attempt.user_id),
                timestamp_arg(&attempt.attempted_at)?,
            ],
        )
        .await?;
        Ok(())
    }

    async fn count_failed_attempts_since(&self, user_id: &str, since: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS count
             FROM totp_failed_attempts
             WHERE user_id = {} AND attempted_at >= {}",
            &[SqlArg::Text(user_id.to_string()), timestamp_arg(since)?],
        )
        .await?;
        row.map(|row| row.i64("count")).unwrap_or(Ok(0))
    }

    async fn clear_failed_attempts(&self, user_id: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "clear_totp_failed_attempts",
            "DELETE FROM totp_failed_attempts WHERE user_id = {}",
            vec![SqlArg::Text(user_id.to_string())],
        )
        .await
    }
}

async fn load_credential_for_user(
    exec: SqlExec<'_, '_>,
    user_id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<TotpCredentialRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, secret_base32, algorithm, digits, period_seconds, last_accepted_step, created_at, updated_at, last_used_at
         FROM totp_credentials
         WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    row.as_ref()
        .map(|row| row_to_credential(row, encryption_key))
        .transpose()
}

async fn load_enrollment_challenge(
    exec: SqlExec<'_, '_>,
    id: &str,
    user_id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<TotpEnrollmentChallengeRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, auth_session_version, secret_base32, algorithm, digits, period_seconds, created_at, expires_at
         FROM totp_enrollment_challenges
         WHERE id = {} AND user_id = {}",
        &[
            SqlArg::Text(id.to_string()),
            SqlArg::Text(user_id.to_string()),
        ],
    )
    .await?;
    row.as_ref()
        .map(|row| row_to_enrollment_challenge(row, encryption_key))
        .transpose()
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await })
    })
    .await
}

fn credential_args(
    credential: &TotpCredentialRecord,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(credential.id.clone()),
        SqlArg::Text(credential.user_id.clone()),
        SqlArg::Text(maybe_encrypt_value(
            encryption_key,
            &credential.secret_base32,
        )?),
        SqlArg::Text(credential.algorithm.clone()),
        SqlArg::I32(credential.digits),
        SqlArg::I32(credential.period_seconds),
        SqlArg::OptI64(credential.last_accepted_step),
        timestamp_arg(&credential.created_at)?,
        timestamp_arg(&credential.updated_at)?,
        opt_timestamp_arg(credential.last_used_at.as_deref())?,
    ])
}

fn challenge_args(
    challenge: &TotpEnrollmentChallengeRecord,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(challenge.id.clone()),
        SqlArg::Text(challenge.user_id.clone()),
        SqlArg::OptText(challenge.auth_session_version.clone()),
        SqlArg::Text(maybe_encrypt_value(
            encryption_key,
            &challenge.secret_base32,
        )?),
        SqlArg::Text(challenge.algorithm.clone()),
        SqlArg::I32(challenge.digits),
        SqlArg::I32(challenge.period_seconds),
        timestamp_arg(&challenge.created_at)?,
        timestamp_arg(&challenge.expires_at)?,
    ])
}

fn recovery_code_args(code: &TotpRecoveryCodeRecord) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(code.id.clone()),
        SqlArg::Text(code.user_id.clone()),
        SqlArg::Text(code.code_hash.clone()),
        timestamp_arg(&code.created_at)?,
        opt_timestamp_arg(code.used_at.as_deref())?,
    ])
}

fn row_to_credential(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<TotpCredentialRecord> {
    Ok(TotpCredentialRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        secret_base32: decrypt_value(
            encryption_key,
            row.text("secret_base32")?,
            "TOTP secret",
            true,
        )?,
        algorithm: row.text("algorithm")?,
        digits: row.i32("digits")?,
        period_seconds: row.i32("period_seconds")?,
        last_accepted_step: row.opt_i64("last_accepted_step")?,
        created_at: timestamp_string(row, "created_at")?,
        updated_at: timestamp_string(row, "updated_at")?,
        last_used_at: opt_timestamp_string(row, "last_used_at")?,
    })
}

fn row_to_enrollment_challenge(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<TotpEnrollmentChallengeRecord> {
    Ok(TotpEnrollmentChallengeRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        auth_session_version: row.opt_text("auth_session_version")?,
        secret_base32: decrypt_value(
            encryption_key,
            row.text("secret_base32")?,
            "TOTP enrollment secret",
            true,
        )?,
        algorithm: row.text("algorithm")?,
        digits: row.i32("digits")?,
        period_seconds: row.i32("period_seconds")?,
        created_at: timestamp_string(row, "created_at")?,
        expires_at: timestamp_string(row, "expires_at")?,
    })
}

fn row_to_recovery_code(row: &SqlRow) -> AppResult<TotpRecoveryCodeRecord> {
    Ok(TotpRecoveryCodeRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        code_hash: row.text("code_hash")?,
        created_at: timestamp_string(row, "created_at")?,
        used_at: opt_timestamp_string(row, "used_at")?,
    })
}

fn timestamp_arg(value: &str) -> AppResult<SqlArg> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            AppError::Repository(format!("invalid RFC3339 timestamp {value}: {error}"))
        })?
        .with_timezone(&Utc);
    Ok(SqlArg::Timestamp(parsed))
}

fn opt_timestamp_arg(value: Option<&str>) -> AppResult<SqlArg> {
    let parsed = value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|error| {
                    AppError::Repository(format!("invalid RFC3339 timestamp {value}: {error}"))
                })
        })
        .transpose()?;
    Ok(SqlArg::OptTimestamp(parsed))
}
