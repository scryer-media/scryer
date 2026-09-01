use async_trait::async_trait;
use scryer_application::{
    ApiKeyProvisioningSource, ApiKeyRecord, AppError, AppResult, OAuthAuthorizationCodeRecord,
    OAuthAuthorizationSource, OAuthClientRegistrationRecord, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshRotation, OAuthRefreshRotationOutcome,
    OAuthRefreshTokenRecord, OAuthRepository,
};
use sqlx::query;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct OAuthStore {
    datastore: StoreDatastore,
}

impl OAuthStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl OAuthRepository for OAuthStore {
    async fn create_api_key(&self, record: ApiKeyRecord) -> AppResult<ApiKeyRecord> {
        SqlRuntime::execute_write(
            &self.datastore,
            "create_api_key",
            "INSERT INTO api_keys
                (id, user_id, lookup_id, secret_hash, label, expires_at, revoked_at,
                 last_used_at, created_at, provisioning_source)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(record.id.clone()),
                SqlArg::Text(record.user_id.clone()),
                SqlArg::Text(record.lookup_id.clone()),
                SqlArg::Text(record.secret_hash.clone()),
                SqlArg::Text(record.label.clone()),
                SqlArg::OptTimestamp(record.expires_at),
                SqlArg::OptTimestamp(record.revoked_at),
                SqlArg::OptTimestamp(record.last_used_at),
                SqlArg::Timestamp(record.created_at),
                SqlArg::Text(record.provisioning_source.as_str().to_string()),
            ],
        )
        .await?;
        Ok(record)
    }

    async fn get_api_key_by_lookup_id(&self, lookup_id: &str) -> AppResult<Option<ApiKeyRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, lookup_id, secret_hash, label, expires_at, revoked_at,
                    last_used_at, created_at, provisioning_source
               FROM api_keys
              WHERE lookup_id = {}",
            &[SqlArg::Text(lookup_id.to_string())],
        )
        .await?;
        row.as_ref().map(row_to_api_key).transpose()
    }

    async fn list_api_keys(&self, user_id: &str) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, lookup_id, secret_hash, label, expires_at, revoked_at,
                    last_used_at, created_at, provisioning_source
               FROM api_keys
              WHERE user_id = {}
              ORDER BY created_at DESC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_api_key).collect()
    }

    async fn list_environment_api_keys(&self) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, lookup_id, secret_hash, label, expires_at, revoked_at,
                    last_used_at, created_at, provisioning_source
               FROM api_keys
              WHERE provisioning_source = 'environment'",
            &[],
        )
        .await?;
        rows.iter().map(row_to_api_key).collect()
    }

    async fn upsert_environment_api_key(&self, record: ApiKeyRecord) -> AppResult<ApiKeyRecord> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "upsert_environment_api_key",
            "INSERT INTO api_keys
                (id, user_id, lookup_id, secret_hash, label, expires_at, revoked_at,
                 last_used_at, created_at, provisioning_source)
             VALUES ({}, {}, {}, {}, {}, {}, NULL, NULL, {}, 'environment')
             ON CONFLICT(lookup_id) DO UPDATE SET
                user_id = excluded.user_id,
                secret_hash = excluded.secret_hash,
                label = excluded.label,
                expires_at = excluded.expires_at,
                revoked_at = NULL,
                provisioning_source = 'environment'
              WHERE api_keys.provisioning_source = 'environment'",
            vec![
                SqlArg::Text(record.id.clone()),
                SqlArg::Text(record.user_id.clone()),
                SqlArg::Text(record.lookup_id.clone()),
                SqlArg::Text(record.secret_hash.clone()),
                SqlArg::Text(record.label.clone()),
                SqlArg::OptTimestamp(record.expires_at),
                SqlArg::Timestamp(record.created_at),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::Validation(
                "an API key with this lookup ID is not environment-managed".into(),
            ));
        }
        Ok(record)
    }

    async fn revoke_api_key(
        &self,
        id: &str,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "revoke_api_key",
            "UPDATE api_keys
                SET revoked_at = COALESCE(revoked_at, {})
              WHERE id = {} AND user_id = {} AND revoked_at IS NULL",
            vec![
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn touch_api_key_last_used(
        &self,
        id: &str,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "touch_api_key_last_used",
            "UPDATE api_keys SET last_used_at = {}
              WHERE id = {}
                AND revoked_at IS NULL
                AND (expires_at IS NULL OR expires_at > {})",
            vec![
                SqlArg::Timestamp(used_at),
                SqlArg::Text(id.to_string()),
                SqlArg::Timestamp(used_at),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn create_client_registration(
        &self,
        record: OAuthClientRegistrationRecord,
    ) -> AppResult<OAuthClientRegistrationRecord> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_oauth_client_registration",
            move |tx| {
                let record = record.clone();
                Box::pin(async move {
                    tx.execute(
                        "INSERT INTO oauth_client_registrations
                            (client_id, display_name, enabled, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {})",
                        &[
                            SqlArg::Text(record.client_id.clone()),
                            SqlArg::Text(record.display_name.clone()),
                            SqlArg::Bool(record.enabled),
                            SqlArg::Timestamp(record.created_at),
                            SqlArg::Timestamp(record.updated_at),
                        ],
                    )
                    .await?;
                    replace_client_redirect_uris_tx(tx, &record.client_id, &record.redirect_uris)
                        .await?;
                    Ok(record)
                })
            },
        )
        .await
    }

    async fn get_client_registration(
        &self,
        client_id: &str,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
        load_client_registration(self.datastore.read_exec(), client_id).await
    }

    async fn list_client_registrations(&self) -> AppResult<Vec<OAuthClientRegistrationRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT c.client_id, c.display_name, c.enabled, c.created_at, c.updated_at,
                    r.redirect_uri
               FROM oauth_client_registrations c
               LEFT JOIN oauth_client_redirect_uris r ON r.client_id = c.client_id
              ORDER BY c.display_name ASC, c.client_id ASC, r.redirect_uri ASC",
            &[],
        )
        .await?;
        rows_to_client_registrations(&rows)
    }

    async fn update_client_registration(
        &self,
        record: OAuthClientRegistrationRecord,
        revoked_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_oauth_client_registration",
            move |tx| {
                let record = record.clone();
                Box::pin(async move {
                    if let Some(postgres) = tx.postgres() {
                        let locked = query(
                            "SELECT client_id FROM oauth_client_registrations WHERE client_id = $1 FOR UPDATE",
                        )
                        .bind(&record.client_id)
                        .fetch_optional(&mut **postgres)
                        .await
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                        if locked.is_none() {
                            return Ok(None);
                        }
                    }
                    let Some(current) =
                        load_client_registration(SqlExec::Tx(tx), &record.client_id).await?
                    else {
                        return Ok(None);
                    };
                    let redirect_uris_changed = current.redirect_uris != record.redirect_uris;
                    let revoke_grants = !record.enabled || redirect_uris_changed;
                    let revoke_reason = if !record.enabled {
                        "client_disabled"
                    } else if redirect_uris_changed {
                        "client_redirect_uris_changed"
                    } else {
                        "client_updated"
                    };
                    let rows = tx
                        .execute(
                            "UPDATE oauth_client_registrations
                                SET display_name = {}, enabled = {}, updated_at = {}
                              WHERE client_id = {}",
                            &[
                                SqlArg::Text(record.display_name.clone()),
                                SqlArg::Bool(record.enabled),
                                SqlArg::Timestamp(record.updated_at),
                                SqlArg::Text(record.client_id.clone()),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Ok(None);
                    }
                    replace_client_redirect_uris_tx(tx, &record.client_id, &record.redirect_uris)
                        .await?;
                    if revoke_grants {
                        revoke_client_grants_tx(tx, &record.client_id, revoked_at, revoke_reason)
                            .await?;
                    }
                    load_client_registration(SqlExec::Tx(tx), &record.client_id).await
                })
            },
        )
        .await
    }

    async fn delete_client_registration(
        &self,
        client_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        revoke_reason: &str,
    ) -> AppResult<bool> {
        let client_id = client_id.to_string();
        let revoke_reason = revoke_reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_oauth_client_registration",
            move |tx| {
                let client_id = client_id.clone();
                let revoke_reason = revoke_reason.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "DELETE FROM oauth_client_registrations WHERE client_id = {}",
                            &[SqlArg::Text(client_id.clone())],
                        )
                        .await?;
                    if rows == 0 {
                        return Ok(false);
                    }
                    revoke_client_grants_tx(tx, &client_id, revoked_at, &revoke_reason).await?;
                    Ok(true)
                })
            },
        )
        .await
    }

    async fn is_refresh_grant_active(&self, grant_id: &str, client_id: &str) -> AppResult<bool> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id
               FROM oauth_refresh_grants
              WHERE id = {} AND client_id = {} AND revoked_at IS NULL",
            &[
                SqlArg::Text(grant_id.to_string()),
                SqlArg::Text(client_id.to_string()),
            ],
        )
        .await?;
        Ok(row.is_some())
    }

    async fn create_authorization_code(
        &self,
        record: OAuthAuthorizationCodeRecord,
    ) -> AppResult<OAuthAuthorizationCodeRecord> {
        SqlRuntime::execute_write(
            &self.datastore,
            "create_oauth_authorization_code",
            "INSERT INTO oauth_authorization_codes
                (id, code_hash, client_id, user_id, auth_session_version, redirect_uri, scope, code_challenge,
                 code_challenge_method, authorization_source, jellyfin_connection_id, jellyfin_external_url,
                 jellyfin_base_url, jellyfin_api_key_hash,
                 created_at, expires_at, consumed_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(record.id.clone()),
                SqlArg::Text(record.code_hash.clone()),
                SqlArg::Text(record.client_id.clone()),
                SqlArg::Text(record.user_id.clone()),
                SqlArg::Text(record.auth_session_version.clone()),
                SqlArg::Text(record.redirect_uri.clone()),
                SqlArg::Text(record.scope.clone()),
                SqlArg::Text(record.code_challenge.clone()),
                SqlArg::Text(record.code_challenge_method.clone()),
                SqlArg::Text(record.authorization_source.as_str().to_string()),
                SqlArg::OptText(record.jellyfin_connection_id.clone()),
                SqlArg::OptText(record.jellyfin_external_url.clone()),
                SqlArg::OptText(record.jellyfin_base_url.clone()),
                SqlArg::OptText(record.jellyfin_api_key_hash.clone()),
                SqlArg::Timestamp(record.created_at),
                SqlArg::Timestamp(record.expires_at),
                SqlArg::OptTimestamp(record.consumed_at),
            ],
        )
        .await?;
        Ok(record)
    }

    async fn get_authorization_code(
        &self,
        id: &str,
    ) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
        load_authorization_code(self.datastore.read_exec(), id).await
    }

    async fn consume_authorization_code(
        &self,
        id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "consume_oauth_authorization_code",
            "UPDATE oauth_authorization_codes
                SET consumed_at = {}
              WHERE id = {}
                AND consumed_at IS NULL",
            vec![SqlArg::Timestamp(consumed_at), SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn consume_authorization_code_and_create_refresh_grant(
        &self,
        code: OAuthAuthorizationCodeRecord,
        consumed_at: chrono::DateTime<chrono::Utc>,
        grant: OAuthRefreshGrantRecord,
        token: OAuthRefreshTokenRecord,
        require_active_client_registration: bool,
    ) -> AppResult<Option<OAuthRefreshGrantRecord>> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "exchange_oauth_authorization_code",
            move |tx| {
                let code = code.clone();
                let grant = grant.clone();
                let token = token.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "UPDATE oauth_authorization_codes
                                SET consumed_at = {}
                              WHERE id = {}
                                AND code_hash = {}
                                AND client_id = {}
                                AND user_id = {}
                                AND auth_session_version = {}
                                AND redirect_uri = {}
                                AND scope = {}
                                AND COALESCE(jellyfin_connection_id, '') = {}
                                AND COALESCE(jellyfin_external_url, '') = {}
                                AND COALESCE(jellyfin_base_url, '') = {}
                                AND COALESCE(jellyfin_api_key_hash, '') = {}
                                AND code_challenge = {}
                                AND code_challenge_method = {}
                                AND authorization_source = {}
                                AND expires_at > {}
                                AND consumed_at IS NULL",
                            &[
                                SqlArg::Timestamp(consumed_at),
                                SqlArg::Text(code.id.clone()),
                                SqlArg::Text(code.code_hash.clone()),
                                SqlArg::Text(code.client_id.clone()),
                                SqlArg::Text(code.user_id.clone()),
                                SqlArg::Text(code.auth_session_version.clone()),
                                SqlArg::Text(code.redirect_uri.clone()),
                                SqlArg::Text(code.scope.clone()),
                                SqlArg::Text(
                                    code.jellyfin_connection_id.clone().unwrap_or_default(),
                                ),
                                SqlArg::Text(
                                    code.jellyfin_external_url.clone().unwrap_or_default(),
                                ),
                                SqlArg::Text(code.jellyfin_base_url.clone().unwrap_or_default()),
                                SqlArg::Text(
                                    code.jellyfin_api_key_hash.clone().unwrap_or_default(),
                                ),
                                SqlArg::Text(code.code_challenge.clone()),
                                SqlArg::Text(code.code_challenge_method.clone()),
                                SqlArg::Text(code.authorization_source.as_str().to_string()),
                                SqlArg::Timestamp(consumed_at),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Ok(None);
                    }
                    if require_active_client_registration {
                        let rows = tx
                            .execute(
                                "UPDATE oauth_client_registrations
                                    SET updated_at = updated_at
                                  WHERE client_id = {} AND enabled = {}",
                                &[SqlArg::Text(grant.client_id.clone()), SqlArg::Bool(true)],
                            )
                            .await?;
                        if rows == 0 {
                            return Err(AppError::Unauthorized(
                                "OAuth client is disabled or unavailable".into(),
                            ));
                        }
                    }
                    insert_refresh_grant_tx(tx, &grant).await?;
                    insert_refresh_token_tx(tx, &token).await?;
                    Ok(Some(grant))
                })
            },
        )
        .await
    }

    async fn create_refresh_grant(
        &self,
        grant: OAuthRefreshGrantRecord,
        token: OAuthRefreshTokenRecord,
        require_active_client_registration: bool,
    ) -> AppResult<OAuthRefreshGrantRecord> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_oauth_refresh_grant", move |tx| {
            let grant = grant.clone();
            let token = token.clone();
            Box::pin(async move {
                if require_active_client_registration {
                    let rows = tx
                        .execute(
                            "UPDATE oauth_client_registrations
                                SET updated_at = updated_at
                              WHERE client_id = {} AND enabled = {}",
                            &[SqlArg::Text(grant.client_id.clone()), SqlArg::Bool(true)],
                        )
                        .await?;
                    if rows == 0 {
                        return Err(AppError::Unauthorized(
                            "OAuth client is disabled or unavailable".into(),
                        ));
                    }
                }
                insert_refresh_grant_tx(tx, &grant).await?;
                insert_refresh_token_tx(tx, &token).await?;
                Ok(grant)
            })
        })
        .await
    }

    async fn get_refresh_token(
        &self,
        id: &str,
    ) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
        load_refresh_token_with_grant(self.datastore.read_exec(), id).await
    }

    async fn get_refresh_grant(&self, id: &str) -> AppResult<Option<OAuthRefreshGrantRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, family_id, user_id, client_id, redirect_uri, scope, jellyfin_connection_id,
                    jellyfin_external_url, jellyfin_base_url, jellyfin_api_key_hash, auth_session_version, authorization_source,
                    created_at, updated_at, last_used_at, revoked_at, revoked_reason
               FROM oauth_refresh_grants
              WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await?;
        row.as_ref().map(row_to_refresh_grant).transpose()
    }

    async fn rotate_refresh_token(
        &self,
        token_id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
        next_token: OAuthRefreshTokenRecord,
    ) -> AppResult<OAuthRefreshRotationOutcome> {
        let token_id = token_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "rotate_oauth_refresh_token", move |tx| {
            let token_id = token_id.clone();
            let next_token = next_token.clone();
            Box::pin(async move {
                let Some((previous_token, grant)) =
                    load_refresh_token_with_grant(SqlExec::Tx(tx), &token_id).await?
                else {
                    return Ok(OAuthRefreshRotationOutcome::Unavailable);
                };
                if previous_token.consumed_at.is_some() {
                    return Ok(OAuthRefreshRotationOutcome::Reused);
                }
                if previous_token.revoked_at.is_some() || grant.revoked_at.is_some() {
                    return Ok(OAuthRefreshRotationOutcome::Unavailable);
                }
                let rows = tx
                    .execute(
                        "UPDATE oauth_refresh_tokens
                            SET consumed_at = {}
                          WHERE id = {}
                            AND consumed_at IS NULL
                            AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(consumed_at),
                            SqlArg::Text(previous_token.id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Ok(OAuthRefreshRotationOutcome::Reused);
                }
                let grant_rows = tx
                    .execute(
                        "UPDATE oauth_refresh_grants
                        SET updated_at = {},
                            last_used_at = {}
                      WHERE id = {}
                        AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(consumed_at),
                            SqlArg::Timestamp(consumed_at),
                            SqlArg::Text(grant.id.clone()),
                        ],
                    )
                    .await?;
                if grant_rows == 0 {
                    return Ok(OAuthRefreshRotationOutcome::Unavailable);
                }
                insert_refresh_token_tx(tx, &next_token).await?;
                let grant = load_refresh_grant_by_id_tx(tx, &grant.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("OAuth grant {}", grant.id)))?;
                Ok(OAuthRefreshRotationOutcome::Rotated(Box::new(
                    OAuthRefreshRotation {
                        grant,
                        previous_token,
                    },
                )))
            })
        })
        .await
    }

    async fn revoke_refresh_grant(
        &self,
        grant_id: &str,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "revoke_oauth_refresh_grant",
            "UPDATE oauth_refresh_grants
                SET revoked_at = COALESCE(revoked_at, {}),
                    revoked_reason = COALESCE(revoked_reason, {}),
                    updated_at = {}
              WHERE id = {}
                AND user_id = {}
                AND revoked_at IS NULL",
            vec![
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(reason.to_string()),
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(grant_id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn revoke_refresh_family(
        &self,
        family_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64> {
        let family_id = family_id.to_string();
        let reason = reason.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "revoke_oauth_refresh_family", move |tx| {
            let family_id = family_id.clone();
            let reason = reason.clone();
            Box::pin(async move {
                let grant_rows = tx
                    .execute(
                        "UPDATE oauth_refresh_grants
                            SET revoked_at = COALESCE(revoked_at, {}),
                                revoked_reason = COALESCE(revoked_reason, {}),
                                updated_at = {}
                          WHERE family_id = {}
                            AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(revoked_at),
                            SqlArg::Text(reason.clone()),
                            SqlArg::Timestamp(revoked_at),
                            SqlArg::Text(family_id.clone()),
                        ],
                    )
                    .await?;
                tx.execute(
                    "UPDATE oauth_refresh_tokens
                        SET revoked_at = COALESCE(revoked_at, {})
                      WHERE family_id = {}
                        AND revoked_at IS NULL",
                    &[SqlArg::Timestamp(revoked_at), SqlArg::Text(family_id)],
                )
                .await?;
                Ok(grant_rows)
            })
        })
        .await
    }

    async fn revoke_user_refresh_grants(
        &self,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64> {
        let user_id = user_id.to_string();
        let reason = reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "revoke_oauth_user_refresh_grants",
            move |tx| {
                let user_id = user_id.clone();
                let reason = reason.clone();
                Box::pin(async move {
                    let grant_rows = tx
                        .execute(
                            "UPDATE oauth_refresh_grants
                            SET revoked_at = COALESCE(revoked_at, {}),
                                revoked_reason = COALESCE(revoked_reason, {}),
                                updated_at = {}
                          WHERE user_id = {}
                            AND revoked_at IS NULL",
                            &[
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(reason),
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(user_id.clone()),
                            ],
                        )
                        .await?;
                    tx.execute(
                        "UPDATE oauth_refresh_tokens
                        SET revoked_at = COALESCE(revoked_at, {})
                      WHERE grant_id IN (
                            SELECT id FROM oauth_refresh_grants WHERE user_id = {}
                        )
                        AND revoked_at IS NULL",
                        &[SqlArg::Timestamp(revoked_at), SqlArg::Text(user_id)],
                    )
                    .await?;
                    Ok(grant_rows)
                })
            },
        )
        .await
    }

    async fn revoke_authless_refresh_grants(
        &self,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64> {
        let reason = reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "revoke_authless_oauth_refresh_grants",
            move |tx| {
                let reason = reason.clone();
                Box::pin(async move {
                    let grant_rows = tx
                        .execute(
                            "UPDATE oauth_refresh_grants
                            SET revoked_at = COALESCE(revoked_at, {}),
                                revoked_reason = COALESCE(revoked_reason, {}),
                                updated_at = {}
                          WHERE authorization_source = {}
                            AND revoked_at IS NULL",
                            &[
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(reason),
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(
                                    OAuthAuthorizationSource::Authless.as_str().to_string(),
                                ),
                            ],
                        )
                        .await?;
                    tx.execute(
                        "UPDATE oauth_refresh_tokens
                        SET revoked_at = COALESCE(revoked_at, {})
                      WHERE grant_id IN (
                            SELECT id FROM oauth_refresh_grants WHERE authorization_source = {}
                        )
                        AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(revoked_at),
                            SqlArg::Text(OAuthAuthorizationSource::Authless.as_str().to_string()),
                        ],
                    )
                    .await?;
                    Ok(grant_rows)
                })
            },
        )
        .await
    }

    async fn touch_refresh_grant_last_used(
        &self,
        grant_id: &str,
        client_id: &str,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "touch_oauth_refresh_grant_last_used",
            "UPDATE oauth_refresh_grants
                SET updated_at = {},
                    last_used_at = {}
              WHERE id = {}
                AND client_id = {}
                AND revoked_at IS NULL",
            vec![
                SqlArg::Timestamp(used_at),
                SqlArg::Timestamp(used_at),
                SqlArg::Text(grant_id.to_string()),
                SqlArg::Text(client_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn list_connected_apps(&self, user_id: &str) -> AppResult<Vec<OAuthConnectedAppRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id AS grant_id, client_id, created_at, last_used_at
               FROM oauth_refresh_grants
              WHERE user_id = {}
                AND revoked_at IS NULL
              ORDER BY created_at DESC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_connected_app).collect()
    }
}

fn row_to_api_key(row: &SqlRow) -> AppResult<ApiKeyRecord> {
    let provisioning_source = row.text("provisioning_source")?;
    let provisioning_source = ApiKeyProvisioningSource::parse(&provisioning_source)
        .ok_or_else(|| AppError::Repository("invalid API key provisioning source".into()))?;
    Ok(ApiKeyRecord {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        lookup_id: row.text("lookup_id")?,
        secret_hash: row.text("secret_hash")?,
        label: row.text("label")?,
        expires_at: row.opt_timestamp("expires_at")?,
        revoked_at: row.opt_timestamp("revoked_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        created_at: row.timestamp("created_at")?,
        provisioning_source,
    })
}

async fn revoke_client_grants_tx(
    tx: &mut SqlTx<'_>,
    client_id: &str,
    revoked_at: chrono::DateTime<chrono::Utc>,
    reason: &str,
) -> AppResult<u64> {
    let grant_rows = tx
        .execute(
            "UPDATE oauth_refresh_grants
                SET revoked_at = COALESCE(revoked_at, {}),
                    revoked_reason = COALESCE(revoked_reason, {}),
                    updated_at = {}
              WHERE client_id = {} AND revoked_at IS NULL",
            &[
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(reason.to_string()),
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(client_id.to_string()),
            ],
        )
        .await?;
    tx.execute(
        "UPDATE oauth_refresh_tokens
            SET revoked_at = COALESCE(revoked_at, {})
          WHERE grant_id IN (
                SELECT id FROM oauth_refresh_grants WHERE client_id = {}
            ) AND revoked_at IS NULL",
        &[
            SqlArg::Timestamp(revoked_at),
            SqlArg::Text(client_id.to_string()),
        ],
    )
    .await?;
    Ok(grant_rows)
}

async fn load_client_registration(
    exec: SqlExec<'_, '_>,
    client_id: &str,
) -> AppResult<Option<OAuthClientRegistrationRecord>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT c.client_id, c.display_name, c.enabled, c.created_at, c.updated_at,
                r.redirect_uri
           FROM oauth_client_registrations c
           LEFT JOIN oauth_client_redirect_uris r ON r.client_id = c.client_id
          WHERE c.client_id = {}
          ORDER BY r.redirect_uri ASC",
        &[SqlArg::Text(client_id.to_string())],
    )
    .await?;
    Ok(rows_to_client_registrations(&rows)?.into_iter().next())
}

async fn load_authorization_code(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, code_hash, client_id, user_id, auth_session_version, redirect_uri, scope, code_challenge,
                code_challenge_method, authorization_source, jellyfin_connection_id, jellyfin_external_url,
                jellyfin_base_url, jellyfin_api_key_hash,
                created_at, expires_at, consumed_at
           FROM oauth_authorization_codes
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_authorization_code).transpose()
}

async fn load_refresh_token_with_grant(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT t.id AS token_id, t.grant_id, t.family_id AS token_family_id, t.token_hash,
                t.created_at AS token_created_at, t.consumed_at, t.revoked_at AS token_revoked_at,
                g.id AS grant_row_id, g.family_id AS grant_family_id, g.user_id, g.client_id, g.redirect_uri,
                g.scope, g.authorization_source, g.jellyfin_connection_id, g.jellyfin_external_url,
                g.jellyfin_base_url, g.jellyfin_api_key_hash,
                g.auth_session_version, g.created_at AS grant_created_at,
                g.updated_at AS grant_updated_at, g.last_used_at,
                g.revoked_at AS grant_revoked_at, g.revoked_reason
           FROM oauth_refresh_tokens t
           JOIN oauth_refresh_grants g ON g.id = t.grant_id
          WHERE t.id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref()
        .map(row_to_refresh_token_with_grant)
        .transpose()
}

async fn insert_refresh_grant_tx(
    tx: &mut SqlTx<'_>,
    grant: &OAuthRefreshGrantRecord,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO oauth_refresh_grants
            (id, family_id, user_id, client_id, redirect_uri, scope, jellyfin_connection_id, jellyfin_external_url,
             jellyfin_base_url, jellyfin_api_key_hash, auth_session_version,
             authorization_source, created_at, updated_at, last_used_at, revoked_at, revoked_reason)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(grant.id.clone()),
            SqlArg::Text(grant.family_id.clone()),
            SqlArg::Text(grant.user_id.clone()),
            SqlArg::Text(grant.client_id.clone()),
            SqlArg::Text(grant.redirect_uri.clone()),
            SqlArg::Text(grant.scope.clone()),
            SqlArg::OptText(grant.jellyfin_connection_id.clone()),
            SqlArg::OptText(grant.jellyfin_external_url.clone()),
            SqlArg::OptText(grant.jellyfin_base_url.clone()),
            SqlArg::OptText(grant.jellyfin_api_key_hash.clone()),
            SqlArg::Text(grant.auth_session_version.clone()),
            SqlArg::Text(grant.authorization_source.as_str().to_string()),
            SqlArg::Timestamp(grant.created_at),
            SqlArg::Timestamp(grant.updated_at),
            SqlArg::OptTimestamp(grant.last_used_at),
            SqlArg::OptTimestamp(grant.revoked_at),
            SqlArg::OptText(grant.revoked_reason.clone()),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_refresh_token_tx(
    tx: &mut SqlTx<'_>,
    token: &OAuthRefreshTokenRecord,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO oauth_refresh_tokens
            (id, grant_id, family_id, token_hash, created_at, consumed_at, revoked_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(token.id.clone()),
            SqlArg::Text(token.grant_id.clone()),
            SqlArg::Text(token.family_id.clone()),
            SqlArg::Text(token.token_hash.clone()),
            SqlArg::Timestamp(token.created_at),
            SqlArg::OptTimestamp(token.consumed_at),
            SqlArg::OptTimestamp(token.revoked_at),
        ],
    )
    .await?;
    Ok(())
}

async fn load_refresh_grant_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<OAuthRefreshGrantRecord>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, family_id, user_id, client_id, redirect_uri, scope, jellyfin_connection_id, jellyfin_external_url,
                jellyfin_base_url, jellyfin_api_key_hash, auth_session_version, created_at,
                authorization_source, updated_at, last_used_at, revoked_at, revoked_reason
           FROM oauth_refresh_grants
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_refresh_grant).transpose()
}

fn row_to_authorization_code(row: &SqlRow) -> AppResult<OAuthAuthorizationCodeRecord> {
    Ok(OAuthAuthorizationCodeRecord {
        id: row.text("id")?,
        code_hash: row.text("code_hash")?,
        client_id: row.text("client_id")?,
        user_id: row.text("user_id")?,
        auth_session_version: row.text("auth_session_version")?,
        redirect_uri: row.text("redirect_uri")?,
        scope: row.text("scope")?,
        jellyfin_connection_id: row.opt_text("jellyfin_connection_id")?,
        jellyfin_external_url: row.opt_text("jellyfin_external_url")?,
        jellyfin_base_url: row.opt_text("jellyfin_base_url")?,
        jellyfin_api_key_hash: row.opt_text("jellyfin_api_key_hash")?,
        code_challenge: row.text("code_challenge")?,
        code_challenge_method: row.text("code_challenge_method")?,
        authorization_source: OAuthAuthorizationSource::parse(&row.text("authorization_source")?),
        created_at: row.timestamp("created_at")?,
        expires_at: row.timestamp("expires_at")?,
        consumed_at: row.opt_timestamp("consumed_at")?,
    })
}

async fn replace_client_redirect_uris_tx(
    tx: &mut SqlTx<'_>,
    client_id: &str,
    redirect_uris: &[String],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM oauth_client_redirect_uris WHERE client_id = {}",
        &[SqlArg::Text(client_id.to_string())],
    )
    .await?;
    for redirect_uri in redirect_uris {
        tx.execute(
            "INSERT INTO oauth_client_redirect_uris (client_id, redirect_uri) VALUES ({}, {})",
            &[
                SqlArg::Text(client_id.to_string()),
                SqlArg::Text(redirect_uri.clone()),
            ],
        )
        .await?;
    }
    Ok(())
}

fn rows_to_client_registrations(rows: &[SqlRow]) -> AppResult<Vec<OAuthClientRegistrationRecord>> {
    let mut registrations = Vec::new();
    for row in rows {
        let client_id = row.text("client_id")?;
        if registrations
            .last()
            .is_none_or(|record: &OAuthClientRegistrationRecord| record.client_id != client_id)
        {
            registrations.push(OAuthClientRegistrationRecord {
                client_id,
                display_name: row.text("display_name")?,
                redirect_uris: Vec::new(),
                enabled: row.bool("enabled")?,
                created_at: row.timestamp("created_at")?,
                updated_at: row.timestamp("updated_at")?,
            });
        }
        if let Some(redirect_uri) = row.opt_text("redirect_uri")? {
            registrations
                .last_mut()
                .expect("OAuth client registration is present")
                .redirect_uris
                .push(redirect_uri);
        }
    }
    Ok(registrations)
}

fn row_to_refresh_grant(row: &SqlRow) -> AppResult<OAuthRefreshGrantRecord> {
    Ok(OAuthRefreshGrantRecord {
        id: row.text("id")?,
        family_id: row.text("family_id")?,
        user_id: row.text("user_id")?,
        client_id: row.text("client_id")?,
        redirect_uri: row.text("redirect_uri")?,
        scope: row.text("scope")?,
        jellyfin_connection_id: row.opt_text("jellyfin_connection_id")?,
        jellyfin_external_url: row.opt_text("jellyfin_external_url")?,
        jellyfin_base_url: row.opt_text("jellyfin_base_url")?,
        jellyfin_api_key_hash: row.opt_text("jellyfin_api_key_hash")?,
        auth_session_version: row.text("auth_session_version")?,
        authorization_source: OAuthAuthorizationSource::parse(&row.text("authorization_source")?),
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        revoked_at: row.opt_timestamp("revoked_at")?,
        revoked_reason: row.opt_text("revoked_reason")?,
    })
}

fn row_to_refresh_token_with_grant(
    row: &SqlRow,
) -> AppResult<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)> {
    let token = OAuthRefreshTokenRecord {
        id: row.text("token_id")?,
        grant_id: row.text("grant_id")?,
        family_id: row.text("token_family_id")?,
        token_hash: row.text("token_hash")?,
        created_at: row.timestamp("token_created_at")?,
        consumed_at: row.opt_timestamp("consumed_at")?,
        revoked_at: row.opt_timestamp("token_revoked_at")?,
    };
    let grant = OAuthRefreshGrantRecord {
        id: row.text("grant_row_id")?,
        family_id: row.text("grant_family_id")?,
        user_id: row.text("user_id")?,
        client_id: row.text("client_id")?,
        redirect_uri: row.text("redirect_uri")?,
        scope: row.text("scope")?,
        jellyfin_connection_id: row.opt_text("jellyfin_connection_id")?,
        jellyfin_external_url: row.opt_text("jellyfin_external_url")?,
        jellyfin_base_url: row.opt_text("jellyfin_base_url")?,
        jellyfin_api_key_hash: row.opt_text("jellyfin_api_key_hash")?,
        auth_session_version: row.text("auth_session_version")?,
        authorization_source: OAuthAuthorizationSource::parse(&row.text("authorization_source")?),
        created_at: row.timestamp("grant_created_at")?,
        updated_at: row.timestamp("grant_updated_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        revoked_at: row.opt_timestamp("grant_revoked_at")?,
        revoked_reason: row.opt_text("revoked_reason")?,
    };
    Ok((token, grant))
}

fn row_to_connected_app(row: &SqlRow) -> AppResult<OAuthConnectedAppRecord> {
    Ok(OAuthConnectedAppRecord {
        grant_id: row.text("grant_id")?,
        client_id: row.text("client_id")?,
        created_at: row.timestamp("created_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
    })
}
