use chrono::{DateTime, Utc};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, LoginVerificationChallengeRecord, LoginVerificationMethod,
    WebauthnChallengePurpose, WebauthnChallengeRecord, WebauthnChallengeType,
    WebauthnCredentialRecord, WebauthnRepository,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use crate::workflow::stores::{opt_timestamp_string, timestamp_string};

#[derive(Clone)]
pub struct WebauthnStore {
    datastore: StoreDatastore,
}

impl WebauthnStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl WebauthnRepository for WebauthnStore {
    async fn list_credentials_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<WebauthnCredentialRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at
             FROM webauthn_credentials
             WHERE user_id = {}
             ORDER BY created_at ASC, id ASC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_credential_record).collect()
    }

    async fn get_credential_by_id_for_user(
        &self,
        credential_record_id: &str,
        user_id: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at
             FROM webauthn_credentials
             WHERE id = {} AND user_id = {}",
            &[
                SqlArg::Text(credential_record_id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_credential_record).transpose()
    }

    async fn get_credential_by_credential_id(
        &self,
        credential_id: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at
             FROM webauthn_credentials
             WHERE credential_id = {}",
            &[SqlArg::Text(credential_id.to_string())],
        )
        .await?;
        row.as_ref().map(row_to_credential_record).transpose()
    }

    async fn create_credential(
        &self,
        credential: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_webauthn_credential", move |tx| {
            let credential = credential.clone();
            Box::pin(async move {
                tx.execute(
                    "INSERT INTO webauthn_credentials
                     (id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(credential.id.clone()),
                        SqlArg::Text(credential.user_id.clone()),
                        SqlArg::Text(credential.credential_id.clone()),
                        SqlArg::Text(credential.credential_json.clone()),
                        SqlArg::OptText(credential.friendly_name.clone()),
                        timestamp_arg(&credential.created_at)?,
                        opt_timestamp_arg(credential.last_used_at.as_deref())?,
                    ],
                )
                .await?;
                load_credential_by_id_tx(tx, &credential.id, &credential.user_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("webauthn credential {}", credential.id)))
            })
        })
        .await
    }

    async fn create_credential_for_current_session(
        &self,
        credential: WebauthnCredentialRecord,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<WebauthnCredentialRecord> {
        let expected_auth_session_version = expected_auth_session_version.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_webauthn_credential_for_current_session",
            move |tx| {
                let credential = credential.clone();
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
                            "passkey enrollment session is no longer current".into(),
                        ));
                    }
                    tx.execute(
                        "INSERT INTO webauthn_credentials
                         (id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(credential.id.clone()),
                            SqlArg::Text(credential.user_id.clone()),
                            SqlArg::Text(credential.credential_id.clone()),
                            SqlArg::Text(credential.credential_json.clone()),
                            SqlArg::OptText(credential.friendly_name.clone()),
                            timestamp_arg(&credential.created_at)?,
                            opt_timestamp_arg(credential.last_used_at.as_deref())?,
                        ],
                    )
                    .await?;
                    load_credential_by_id_tx(tx, &credential.id, &credential.user_id)
                        .await?
                        .ok_or_else(|| {
                            AppError::NotFound(format!("webauthn credential {}", credential.id))
                        })
                })
            },
        )
        .await
    }

    async fn update_credential(
        &self,
        credential: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        SqlRuntime::run_in_transaction(&self.datastore, "update_webauthn_credential", move |tx| {
            let credential = credential.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE webauthn_credentials
                         SET credential_json = {}, friendly_name = {}, last_used_at = {}
                         WHERE id = {} AND user_id = {}",
                        &[
                            SqlArg::Text(credential.credential_json.clone()),
                            SqlArg::OptText(credential.friendly_name.clone()),
                            opt_timestamp_arg(credential.last_used_at.as_deref())?,
                            SqlArg::Text(credential.id.clone()),
                            SqlArg::Text(credential.user_id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!(
                        "webauthn credential {}",
                        credential.id
                    )));
                }
                load_credential_by_id_tx(tx, &credential.id, &credential.user_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("webauthn credential {}", credential.id))
                    })
            })
        })
        .await
    }

    async fn update_credential_if_current(
        &self,
        credential: WebauthnCredentialRecord,
        expected_credential_json: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        let expected_credential_json = expected_credential_json.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_webauthn_credential_if_current",
            move |tx| {
                let credential = credential.clone();
                let expected_credential_json = expected_credential_json.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "UPDATE webauthn_credentials
                             SET credential_json = {}, friendly_name = {}, last_used_at = {}
                             WHERE id = {} AND user_id = {} AND credential_json = {}",
                            &[
                                SqlArg::Text(credential.credential_json.clone()),
                                SqlArg::OptText(credential.friendly_name.clone()),
                                opt_timestamp_arg(credential.last_used_at.as_deref())?,
                                SqlArg::Text(credential.id.clone()),
                                SqlArg::Text(credential.user_id.clone()),
                                SqlArg::Text(expected_credential_json),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Ok(None);
                    }
                    load_credential_by_id_tx(tx, &credential.id, &credential.user_id).await
                })
            },
        )
        .await
    }

    async fn delete_credential_for_user(
        &self,
        credential_record_id: &str,
        user_id: &str,
    ) -> AppResult<()> {
        let rows = execute_write(
            &self.datastore,
            "delete_webauthn_credential",
            "DELETE FROM webauthn_credentials WHERE id = {} AND user_id = {}",
            vec![
                SqlArg::Text(credential_record_id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::NotFound(format!(
                "webauthn credential {credential_record_id}"
            )));
        }
        Ok(())
    }

    async fn delete_credential_preserving_login_route_for_current_session(
        &self,
        credential_record_id: &str,
        user_id: &str,
        expected_auth_session_version: Option<&str>,
    ) -> AppResult<()> {
        let credential_record_id = credential_record_id.to_string();
        let user_id = user_id.to_string();
        let expected_auth_session_version = expected_auth_session_version.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_webauthn_credential_preserving_login_route_for_current_session",
            move |tx| {
                let credential_record_id = credential_record_id.clone();
                let user_id = user_id.clone();
                let expected_auth_session_version = expected_auth_session_version.clone();
                Box::pin(async move {
                    let users = tx
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
                    if users == 0 {
                        return Err(AppError::Unauthorized(
                            "passkey deletion session is no longer current".into(),
                        ));
                    }

                    let deleted = tx
                        .execute(
                            "DELETE FROM webauthn_credentials
                             WHERE id = {} AND user_id = {}
                               AND (
                                    EXISTS (SELECT 1 FROM users
                                            WHERE id = {} AND password_hash IS NOT NULL)
                                    OR EXISTS (SELECT 1 FROM user_external_accounts
                                               WHERE user_id = {} AND status = 'active')
                                    OR EXISTS (SELECT 1 FROM webauthn_credentials
                                               WHERE user_id = {} AND id <> {})
                               )",
                            &[
                                SqlArg::Text(credential_record_id.clone()),
                                SqlArg::Text(user_id.clone()),
                                SqlArg::Text(user_id.clone()),
                                SqlArg::Text(user_id.clone()),
                                SqlArg::Text(user_id.clone()),
                                SqlArg::Text(credential_record_id.clone()),
                            ],
                        )
                        .await?;
                    if deleted == 0 {
                        if load_credential_by_id_tx(tx, &credential_record_id, &user_id)
                            .await?
                            .is_none()
                        {
                            return Err(AppError::NotFound(format!(
                                "webauthn credential {credential_record_id}"
                            )));
                        }
                        return Err(AppError::Validation(
                            "cannot remove the last available sign-in method".into(),
                        ));
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn create_challenge(
        &self,
        challenge: WebauthnChallengeRecord,
    ) -> AppResult<WebauthnChallengeRecord> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_webauthn_challenge", move |tx| {
            let challenge = challenge.clone();
            Box::pin(async move {
                tx.execute(
                    "INSERT INTO webauthn_challenges
                     (id, user_id, challenge_type, purpose, login_verification_challenge_id, auth_session_version, state_json, created_at, expires_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &[
                        SqlArg::Text(challenge.id.clone()),
                        SqlArg::OptText(challenge.user_id.clone()),
                        SqlArg::Text(challenge.challenge_type.as_str().to_string()),
                        SqlArg::Text(challenge.purpose.as_str().to_string()),
                        SqlArg::OptText(challenge.login_verification_challenge_id.clone()),
                        SqlArg::OptText(challenge.auth_session_version.clone()),
                        SqlArg::Text(challenge.state_json.clone()),
                        timestamp_arg(&challenge.created_at)?,
                        timestamp_arg(&challenge.expires_at)?,
                    ],
                )
                .await?;
                load_challenge_by_id_tx(tx, &challenge.id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("webauthn challenge {}", challenge.id))
                    })
            })
        })
        .await
    }

    async fn get_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, challenge_type, purpose, login_verification_challenge_id, auth_session_version, state_json, created_at, expires_at
             FROM webauthn_challenges
             WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref().map(row_to_challenge_record).transpose()
    }

    async fn take_challenge(&self, id: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "take_webauthn_challenge", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let row = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    "DELETE FROM webauthn_challenges
                     WHERE id = {}
                     RETURNING id, user_id, challenge_type, purpose, login_verification_challenge_id, auth_session_version, state_json, created_at, expires_at",
                    &[SqlArg::Text(id)],
                )
                .await?;
                row.as_ref().map(row_to_challenge_record).transpose()
            })
        })
        .await
    }

    async fn delete_challenge(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_webauthn_challenge",
            "DELETE FROM webauthn_challenges WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn delete_expired_challenges(&self, now: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "delete_expired_webauthn_challenges",
            "DELETE FROM webauthn_challenges WHERE expires_at <= {}",
            vec![timestamp_arg(now)?],
        )
        .await
    }

    async fn create_login_verification_challenge(
        &self,
        challenge: LoginVerificationChallengeRecord,
    ) -> AppResult<LoginVerificationChallengeRecord> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_login_verification_challenge",
            move |tx| {
                let challenge = challenge.clone();
                Box::pin(async move {
                    tx.execute(
                        "INSERT INTO login_verification_challenges
                         (id, user_id, login_method, persist_session, allow_passkey, allow_totp, auth_session_version, created_at, expires_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(challenge.id.clone()),
                            SqlArg::Text(challenge.user_id.clone()),
                            SqlArg::Text(challenge.login_method.as_str().to_string()),
                            SqlArg::Bool(challenge.persist_session),
                            SqlArg::Bool(challenge.allow_passkey),
                            SqlArg::Bool(challenge.allow_totp),
                            SqlArg::OptText(challenge.auth_session_version.clone()),
                            timestamp_arg(&challenge.created_at)?,
                            timestamp_arg(&challenge.expires_at)?,
                        ],
                    )
                    .await?;
                    load_login_verification_challenge_by_id_tx(tx, &challenge.id)
                        .await?
                        .ok_or_else(|| {
                            AppError::NotFound(format!(
                                "login verification challenge {}",
                                challenge.id
                            ))
                        })
                })
            },
        )
        .await
    }

    async fn get_login_verification_challenge(
        &self,
        id: &str,
    ) -> AppResult<Option<LoginVerificationChallengeRecord>> {
        load_login_verification_challenge_by_id(self.datastore.read_exec(), id).await
    }

    async fn take_login_verification_challenge(
        &self,
        id: &str,
    ) -> AppResult<Option<LoginVerificationChallengeRecord>> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "take_login_verification_challenge",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let row = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "DELETE FROM login_verification_challenges
                         WHERE id = {}
                         RETURNING id, user_id, login_method, persist_session, allow_passkey, allow_totp, auth_session_version, created_at, expires_at",
                        &[SqlArg::Text(id)],
                    )
                    .await?;
                    row.as_ref()
                        .map(row_to_login_verification_challenge_record)
                        .transpose()
                })
            },
        )
        .await
    }

    async fn delete_login_verification_challenges_for_user(&self, user_id: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "delete_login_verification_challenges_for_user",
            "DELETE FROM login_verification_challenges WHERE user_id = {}",
            vec![SqlArg::Text(user_id.to_string())],
        )
        .await
    }

    async fn delete_expired_login_verification_challenges(&self, now: &str) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "delete_expired_login_verification_challenges",
            "DELETE FROM login_verification_challenges WHERE expires_at <= {}",
            vec![timestamp_arg(now)?],
        )
        .await
    }
}

async fn load_credential_by_id_tx(
    tx: &mut SqlTx<'_>,
    credential_record_id: &str,
    user_id: &str,
) -> AppResult<Option<WebauthnCredentialRecord>> {
    load_credential_by_id(SqlExec::Tx(tx), credential_record_id, user_id).await
}

async fn load_credential_by_id(
    exec: SqlExec<'_, '_>,
    credential_record_id: &str,
    user_id: &str,
) -> AppResult<Option<WebauthnCredentialRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, credential_id, credential_json, friendly_name, created_at, last_used_at
         FROM webauthn_credentials
         WHERE id = {} AND user_id = {}",
        &[
            SqlArg::Text(credential_record_id.to_string()),
            SqlArg::Text(user_id.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_credential_record).transpose()
}

async fn load_challenge_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<WebauthnChallengeRecord>> {
    load_challenge_by_id(SqlExec::Tx(tx), id).await
}

async fn load_challenge_by_id(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<WebauthnChallengeRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, challenge_type, purpose, login_verification_challenge_id, auth_session_version, state_json, created_at, expires_at
         FROM webauthn_challenges
         WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_challenge_record).transpose()
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

fn row_to_credential_record(row: &SqlRow) -> AppResult<WebauthnCredentialRecord> {
    Ok(WebauthnCredentialRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        credential_id: row.text("credential_id")?,
        credential_json: row.text("credential_json")?,
        friendly_name: row.opt_text("friendly_name")?,
        created_at: timestamp_string(row, "created_at")?,
        last_used_at: opt_timestamp_string(row, "last_used_at")?,
    })
}

fn row_to_challenge_record(row: &SqlRow) -> AppResult<WebauthnChallengeRecord> {
    let challenge_type_raw = row.text("challenge_type")?;
    let challenge_type = WebauthnChallengeType::parse(&challenge_type_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "unknown webauthn challenge type: {challenge_type_raw}"
        ))
    })?;
    let purpose_raw = row.text("purpose")?;
    let purpose = WebauthnChallengePurpose::parse(&purpose_raw).ok_or_else(|| {
        AppError::Repository(format!("unknown webauthn challenge purpose: {purpose_raw}"))
    })?;
    Ok(WebauthnChallengeRecord {
        id: row.text("id")?,
        user_id: row.opt_text("user_id")?,
        challenge_type,
        purpose,
        login_verification_challenge_id: row.opt_text("login_verification_challenge_id")?,
        auth_session_version: row.opt_text("auth_session_version")?,
        state_json: row.text("state_json")?,
        created_at: timestamp_string(row, "created_at")?,
        expires_at: timestamp_string(row, "expires_at")?,
    })
}

async fn load_login_verification_challenge_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<LoginVerificationChallengeRecord>> {
    load_login_verification_challenge_by_id(SqlExec::Tx(tx), id).await
}

async fn load_login_verification_challenge_by_id(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<LoginVerificationChallengeRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, login_method, persist_session, allow_passkey, allow_totp, auth_session_version, created_at, expires_at
         FROM login_verification_challenges
         WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref()
        .map(row_to_login_verification_challenge_record)
        .transpose()
}

fn row_to_login_verification_challenge_record(
    row: &SqlRow,
) -> AppResult<LoginVerificationChallengeRecord> {
    let login_method_raw = row.text("login_method")?;
    let login_method = LoginVerificationMethod::parse(&login_method_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "unknown login verification method: {login_method_raw}"
        ))
    })?;
    Ok(LoginVerificationChallengeRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        login_method,
        persist_session: row.bool("persist_session")?,
        allow_passkey: row.bool("allow_passkey")?,
        allow_totp: row.bool("allow_totp")?,
        auth_session_version: row.opt_text("auth_session_version")?,
        created_at: timestamp_string(row, "created_at")?,
        expires_at: timestamp_string(row, "expires_at")?,
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
