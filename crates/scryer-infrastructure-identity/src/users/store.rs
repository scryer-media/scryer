use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, UiDateTimeFormat, UiDefaultLandingView, UiDensity, UiSettings,
    UiSettingsFacet, UiSettingsUpdate, UiSidebarMode, UiTableColumnSetting, UiTableViewMode,
    UiTheme, UserExternalAccountRepository, UserLoginSnapshot, UserRepository,
    UserUiSettingsRepository,
};
use scryer_domain::{
    AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus, LibraryGrant, User,
    UserAccountKind, UserExternalAccount, UserLoginStatus,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct UserStore {
    datastore: StoreDatastore,
}

impl UserStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl UserRepository for UserStore {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        load_user_by_username(self.datastore.read_exec(), username).await
    }

    async fn get_login_snapshot_by_username(
        &self,
        username: &str,
    ) -> AppResult<Option<UserLoginSnapshot>> {
        load_user_login_snapshot_by_username(self.datastore.read_exec(), username).await
    }

    async fn create(&self, user: User) -> AppResult<User> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_user", move |tx| {
            let user = user.clone();
            Box::pin(async move {
                insert_user_tx(tx, &user).await?;
                Ok(user)
            })
        })
        .await
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, username, password_hash, password_change_required, account_kind, status FROM users",
            &[],
        )
        .await?;
        rows.iter().map(row_to_user).collect()
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        load_user_by_id(self.datastore.read_exec(), id).await
    }

    async fn auth_session_version(&self, user_id: &str) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT auth_session_version FROM users WHERE id = {}",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        row.map(|row| row.opt_text("auth_session_version"))
            .unwrap_or(Ok(None))
    }

    async fn rotate_auth_session_version(
        &self,
        user_id: &str,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let user_id = user_id.to_string();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "rotate_auth_session_version", move |tx| {
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
                load_user_by_id_tx(tx, &user_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))
            })
        })
        .await
    }

    async fn reset_authentication_factors_and_invalidate_sessions(
        &self,
        user_id: &str,
        auth_session_version: &str,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "reset_authentication_factors_and_invalidate_sessions",
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
                    for table in [
                        "totp_credentials",
                        "totp_recovery_codes",
                        "totp_failed_attempts",
                        "totp_enrollment_challenges",
                        "webauthn_challenges",
                        "webauthn_credentials",
                        "login_verification_challenges",
                    ] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE user_id = {{}}"),
                            &[SqlArg::Text(user_id.clone())],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn update_password_and_invalidate_sessions(
        &self,
        id: &str,
        password_hash: String,
        password_change_required: bool,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let id = id.to_string();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_password_and_invalidate_sessions",
            move |tx| {
                let id = id.clone();
                let password_hash = password_hash.clone();
                let auth_session_version = auth_session_version.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "UPDATE users
                             SET password_hash = {}, password_change_required = {}, auth_session_version = {}
                             WHERE id = {}",
                            &[
                                SqlArg::Text(password_hash),
                                SqlArg::Bool(password_change_required),
                                SqlArg::Text(auth_session_version),
                                SqlArg::Text(id.clone()),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!("user {id}")));
                    }
                    for table in [
                        "totp_enrollment_challenges",
                        "webauthn_challenges",
                        "login_verification_challenges",
                    ] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE user_id = {{}}"),
                            &[SqlArg::Text(id.clone())],
                        )
                        .await?;
                    }
                    load_user_by_id_tx(tx, &id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("user {id}")))
                })
            },
        )
        .await
    }

    async fn update_own_password_and_invalidate_sessions(
        &self,
        id: &str,
        password_hash: String,
        password_change_required: bool,
        auth_session_version: &str,
        expected_password_hash: Option<&str>,
    ) -> AppResult<User> {
        let id = id.to_string();
        let expected_password_hash = expected_password_hash.map(str::to_string);
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_own_password_and_invalidate_sessions",
            move |tx| {
                let id = id.clone();
                let password_hash = password_hash.clone();
                let expected_password_hash = expected_password_hash.clone();
                let auth_session_version = auth_session_version.clone();
                Box::pin(async move {
                    let rows = if let Some(expected_password_hash) = expected_password_hash {
                        tx.execute(
                            "UPDATE users
                             SET password_hash = {}, password_change_required = {}, auth_session_version = {}
                             WHERE id = {} AND password_hash = {}",
                            &[
                                SqlArg::Text(password_hash),
                                SqlArg::Bool(password_change_required),
                                SqlArg::Text(auth_session_version),
                                SqlArg::Text(id.clone()),
                                SqlArg::Text(expected_password_hash),
                            ],
                        )
                        .await?
                    } else {
                        tx.execute(
                            "UPDATE users
                             SET password_hash = {}, password_change_required = {}, auth_session_version = {}
                             WHERE id = {} AND password_hash IS NULL",
                            &[
                                SqlArg::Text(password_hash),
                                SqlArg::Bool(password_change_required),
                                SqlArg::Text(auth_session_version),
                                SqlArg::Text(id.clone()),
                            ],
                        )
                        .await?
                    };
                    if rows == 0 {
                        return Err(AppError::ReauthenticationRequired(
                            "account credentials changed; authenticate again".into(),
                        ));
                    }
                    for table in [
                        "totp_enrollment_challenges",
                        "webauthn_challenges",
                        "login_verification_challenges",
                    ] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE user_id = {{}}"),
                            &[SqlArg::Text(id.clone())],
                        )
                        .await?;
                    }
                    load_user_by_id_tx(tx, &id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("user {id}")))
                })
            },
        )
        .await
    }

    async fn complete_required_password_change(
        &self,
        id: &str,
        password_hash: String,
        expected_auth_session_version: &Option<String>,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let id = id.to_string();
        let expected_auth_session_version = expected_auth_session_version.clone();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "complete_required_password_change",
            move |tx| {
                let id = id.clone();
                let password_hash = password_hash.clone();
                let expected_auth_session_version = expected_auth_session_version.clone();
                let auth_session_version = auth_session_version.clone();
                Box::pin(async move {
                    let rows = tx
                        .execute(
                            "UPDATE users
                             SET password_hash = {}, password_change_required = {}, auth_session_version = {}
                             WHERE id = {} AND password_change_required = {}
                               AND (auth_session_version = {} OR (auth_session_version IS NULL AND {} IS NULL))",
                            &[
                                SqlArg::Text(password_hash),
                                SqlArg::Bool(false),
                                SqlArg::Text(auth_session_version),
                                SqlArg::Text(id.clone()),
                                SqlArg::Bool(true),
                                SqlArg::OptText(expected_auth_session_version.clone()),
                                SqlArg::OptText(expected_auth_session_version),
                            ],
                        )
                        .await?;
                    if rows == 0 {
                        return Err(AppError::Unauthorized(
                            "password change is no longer required".into(),
                        ));
                    }
                    load_user_by_id_tx(tx, &id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("user {id}")))
                })
            },
        )
        .await
    }

    async fn update_login_status_and_rotate_session(
        &self,
        id: &str,
        status: UserLoginStatus,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let id = id.to_string();
        let status = status.as_storage_str().to_string();
        let auth_session_version = auth_session_version.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_user_login_status", move |tx| {
            let id = id.clone();
            let status = status.clone();
            let auth_session_version = auth_session_version.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE users SET status = {}, auth_session_version = {} WHERE id = {}",
                        &[
                            SqlArg::Text(status),
                            SqlArg::Text(auth_session_version),
                            SqlArg::Text(id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("user {id}")));
                }
                load_user_by_id_tx(tx, &id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("user {id}")))
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_user", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "DELETE FROM users WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("user {id}")));
                }
                Ok(())
            })
        })
        .await
    }
}

#[async_trait]
impl UserUiSettingsRepository for UserStore {
    async fn get_by_user_id(&self, user_id: &str) -> AppResult<Option<UiSettings>> {
        load_ui_settings_by_user_id(&self.datastore, user_id).await
    }

    async fn upsert(&self, user_id: &str, settings: UiSettingsUpdate) -> AppResult<UiSettings> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "upsert_user_ui_settings", move |tx| {
            let user_id = user_id.clone();
            let settings = settings.clone();
            Box::pin(async move {
                upsert_ui_settings_tx(tx, &user_id, &settings).await?;
                replace_ui_table_columns_tx(tx, &user_id, &settings.table_columns).await?;
                load_ui_settings_by_user_id_tx(tx, &user_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("UI settings for user {user_id}")))
            })
        })
        .await
    }
}

#[async_trait]
impl UserExternalAccountRepository for UserStore {
    async fn create(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_user_external_account", move |tx| {
            let account = account.clone();
            Box::pin(async move {
                insert_external_account_tx(tx, &account).await?;
                load_external_account_by_id_tx(tx, &account.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("external account {}", account.id)))
            })
        })
        .await
    }

    async fn create_or_get_by_provider_identity(
        &self,
        account: UserExternalAccount,
    ) -> AppResult<UserExternalAccount> {
        if account
            .external_user_id
            .as_deref()
            .is_none_or(|external_user_id| external_user_id.trim().is_empty())
        {
            return Err(AppError::Validation(
                "external account identity must be present when claiming".into(),
            ));
        }
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "claim_user_external_account_provider_identity",
            move |tx| {
                let account = account.clone();
                Box::pin(async move {
                    if insert_external_account_if_provider_identity_absent_tx(tx, &account).await? {
                        return load_external_account_by_id_tx(tx, &account.id)
                            .await?
                            .ok_or_else(|| {
                                AppError::NotFound(format!("external account {}", account.id))
                            });
                    }

                    if let Some(existing) = load_external_account_by_provider_identity_tx(
                        tx,
                        &account.provider,
                        &account.connection_id,
                        account.external_user_id.as_deref().ok_or_else(|| {
                            AppError::Validation(
                                "external account identity must be present when claiming".into(),
                            )
                        })?,
                    )
                    .await?
                    {
                        return Ok(existing);
                    }
                    if load_external_account_by_user_binding_tx(
                        tx,
                        &account.user_id,
                        &account.provider,
                        &account.connection_id,
                    )
                    .await?
                    .is_some()
                    {
                        return Err(AppError::Validation(
                            "external account is already linked to a different provider identity"
                                .into(),
                        ));
                    }
                    Err(AppError::Repository(
                        "provider identity claim did not return its current external account"
                            .into(),
                    ))
                })
            },
        )
        .await
    }

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserExternalAccount>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE user_id = {}
              ORDER BY provider, username",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_external_account).collect()
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserExternalAccount>> {
        load_external_account_by_id(self.datastore.read_exec(), id).await
    }

    async fn get_by_provider_identity(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
        external_user_id: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE provider = {} AND connection_id = {} AND external_user_id = {}",
            &[
                SqlArg::Text(provider.as_str().to_string()),
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Text(external_user_id.to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_external_account).transpose()
    }

    async fn get_pending_claim_by_provider_username(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
        username: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE provider = {}
                AND connection_id = {}
                AND status = 'pending_claim'
                AND external_user_id IS NULL
                AND LOWER(username) = LOWER({})",
            &[
                SqlArg::Text(provider.as_str().to_string()),
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Text(username.trim().to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_external_account).transpose()
    }

    async fn list_verified_by_connection(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
    ) -> AppResult<Vec<UserExternalAccount>> {
        // The three conditions are the participant test, stated in SQL so a
        // caller cannot forget one: active, actually verified, and carrying a
        // provider user id we can address the provider's API with. The
        // emptiness check is written as a trimmed comparison because a link
        // repaired by hand can hold a blank string rather than NULL.
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE provider = {}
                AND connection_id = {}
                AND status = 'active'
                AND verified_at IS NOT NULL
                AND external_user_id IS NOT NULL
                AND TRIM(external_user_id) <> ''
              ORDER BY username, id",
            &[
                SqlArg::Text(provider.as_str().to_string()),
                SqlArg::Text(connection_id.to_string()),
            ],
        )
        .await?;
        rows.iter().map(row_to_external_account).collect()
    }

    async fn update(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        SqlRuntime::run_in_transaction(&self.datastore, "update_user_external_account", move |tx| {
            let account = account.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE user_external_accounts
                            SET user_id = {},
                                provider = {},
                                connection_id = {},
                                external_user_id = {},
                                username = {},
                                display_name = {},
                                avatar_url = {},
                                status = {},
                                verified_at = {},
                                last_login_at = {},
                                updated_at = {}
                           WHERE id = {}",
                        &[
                            SqlArg::Text(account.user_id.clone()),
                            SqlArg::Text(account.provider.as_str().to_string()),
                            SqlArg::Text(account.connection_id.clone()),
                            SqlArg::OptText(account.external_user_id.clone()),
                            SqlArg::Text(account.username.clone()),
                            SqlArg::OptText(account.display_name.clone()),
                            SqlArg::OptText(account.avatar_url.clone()),
                            SqlArg::Text(account.status.as_str().to_string()),
                            SqlArg::OptTimestamp(account.verified_at),
                            SqlArg::OptTimestamp(account.last_login_at),
                            SqlArg::Timestamp(account.updated_at),
                            SqlArg::Text(account.id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!(
                        "external account {}",
                        account.id
                    )));
                }
                load_external_account_by_id_tx(tx, &account.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("external account {}", account.id)))
            })
        })
        .await
    }

    async fn create_auto_added_user_with_account(
        &self,
        user: User,
        app_permissions: AppPermissionMask,
        library_grants: Vec<LibraryGrant>,
        account: UserExternalAccount,
    ) -> AppResult<(User, UserExternalAccount)> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_auto_added_user_with_account",
            move |tx| {
                let user = user.clone();
                let account = account.clone();
                let library_grants = library_grants.clone();
                Box::pin(async move {
                    insert_user_tx(tx, &user).await?;
                    upsert_app_permission_mask_tx(tx, &user.id, app_permissions).await?;
                    replace_library_grants_tx(tx, &user.id, &library_grants).await?;
                    insert_external_account_tx(tx, &account).await?;
                    let account = load_external_account_by_id_tx(tx, &account.id)
                        .await?
                        .ok_or_else(|| {
                            AppError::NotFound(format!("external account {}", account.id))
                        })?;
                    Ok((user, account))
                })
            },
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_user_external_account", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let Some(account) = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    "SELECT user_id FROM user_external_accounts WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?
                else {
                    return Err(AppError::NotFound(format!("external account {id}")));
                };
                let user_id = account.text("user_id")?;
                tx.execute(
                    "UPDATE users SET auth_session_version = auth_session_version WHERE id = {}",
                    &[SqlArg::Text(user_id.clone())],
                )
                .await?;
                let rows = tx
                    .execute(
                        "DELETE FROM user_external_accounts
                         WHERE id = {}
                           AND (
                                EXISTS (SELECT 1 FROM users
                                        WHERE id = {} AND password_hash IS NOT NULL)
                                OR EXISTS (SELECT 1 FROM webauthn_credentials WHERE user_id = {})
                                OR EXISTS (SELECT 1 FROM user_external_accounts
                                           WHERE user_id = {} AND status = 'active' AND id <> {})
                           )",
                        &[
                            SqlArg::Text(id.clone()),
                            SqlArg::Text(user_id.clone()),
                            SqlArg::Text(user_id.clone()),
                            SqlArg::Text(user_id),
                            SqlArg::Text(id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::Validation(
                        "cannot remove the last available sign-in method".into(),
                    ));
                }
                Ok(())
            })
        })
        .await
    }
}

async fn load_user_by_username(exec: SqlExec<'_, '_>, username: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash, password_change_required, account_kind, status FROM users WHERE username = {}",
        &[SqlArg::Text(username.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_login_snapshot_by_username(
    exec: SqlExec<'_, '_>,
    username: &str,
) -> AppResult<Option<UserLoginSnapshot>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash, password_change_required, account_kind, status, auth_session_version FROM users WHERE username = {}",
        &[SqlArg::Text(username.to_string())],
    )
    .await?;
    row.as_ref()
        .map(|row| {
            Ok(UserLoginSnapshot {
                user: row_to_user(row)?,
                auth_session_version: row.opt_text("auth_session_version")?,
            })
        })
        .transpose()
}

async fn load_user_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash, password_change_required, account_kind, status FROM users WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<User>> {
    load_user_by_id(SqlExec::Tx(tx), id).await
}

async fn insert_user_tx(tx: &mut SqlTx<'_>, user: &User) -> AppResult<()> {
    tx.execute(
        "INSERT INTO users (id, username, password_hash, password_change_required, account_kind, status)
         VALUES ({}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(user.id.clone()),
            SqlArg::Text(user.username.clone()),
            SqlArg::OptText(user.password_hash.clone()),
            SqlArg::Bool(user.password_change_required),
            SqlArg::Text(user.account_kind.as_str().to_string()),
            SqlArg::Text(user.login_status().as_storage_str().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn upsert_app_permission_mask_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    permissions: AppPermissionMask,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
         VALUES ({}, {}, {})
         ON CONFLICT(user_id) DO UPDATE SET
            permission_mask = excluded.permission_mask,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(user_id.to_string()),
            SqlArg::I64(permissions.bits() as i64),
            SqlArg::Timestamp(chrono::Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

async fn replace_library_grants_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    grants: &[LibraryGrant],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM user_library_permission_masks WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    for grant in grants.iter().filter(|grant| !grant.permissions.is_empty()) {
        tx.execute(
            "INSERT INTO user_library_permission_masks
             (user_id, library_id, permission_mask, updated_at)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(user_id.to_string()),
                SqlArg::Text(grant.library_id.clone()),
                SqlArg::I64(grant.permissions.bits() as i64),
                SqlArg::Timestamp(chrono::Utc::now()),
            ],
        )
        .await?;
    }
    Ok(())
}

fn row_to_user(row: &SqlRow) -> AppResult<User> {
    let login_status = UserLoginStatus::parse_storage(&row.text("status")?).ok_or_else(|| {
        AppError::Repository(format!(
            "invalid user login status for user {}",
            row.text("id").unwrap_or_else(|_| "<unknown>".to_string())
        ))
    })?;
    Ok(User {
        id: row.text("id")?,
        username: row.text("username")?,
        password_hash: row.opt_text("password_hash")?,
        password_change_required: row.bool("password_change_required")?,
        account_kind: UserAccountKind::parse(&row.text("account_kind")?).ok_or_else(|| {
            AppError::Repository(format!(
                "invalid user account kind for user {}",
                row.text("id").unwrap_or_else(|_| "<unknown>".to_string())
            ))
        })?,
        authorization: scryer_domain::UserAuthorization {
            login_status,
            ..Default::default()
        },
    })
}

async fn load_ui_settings_by_user_id(
    datastore: &StoreDatastore,
    user_id: &str,
) -> AppResult<Option<UiSettings>> {
    let row = SqlRuntime::fetch_optional(
        datastore.read_exec(),
        "SELECT user_id, theme, date_time_format, highlight_color, secondary_color, high_contrast_mode,
                reduce_motion, hide_sponsor_button, density, sidebar_mode, default_landing_view, created_at, updated_at
           FROM user_ui_settings
          WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut settings = row_to_ui_settings(&row)?;
    settings.table_columns = load_ui_table_columns(datastore, user_id).await?;
    Ok(Some(settings))
}

async fn load_ui_settings_by_user_id_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
) -> AppResult<Option<UiSettings>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT user_id, theme, date_time_format, highlight_color, secondary_color, high_contrast_mode,
                reduce_motion, hide_sponsor_button, density, sidebar_mode, default_landing_view, created_at, updated_at
           FROM user_ui_settings
          WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut settings = row_to_ui_settings(&row)?;
    settings.table_columns = load_ui_table_columns_tx(tx, user_id).await?;
    Ok(Some(settings))
}

async fn upsert_ui_settings_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    settings: &UiSettingsUpdate,
) -> AppResult<()> {
    let now = Utc::now();
    tx.execute(
        "INSERT INTO user_ui_settings (
             user_id, theme, date_time_format, highlight_color, secondary_color, high_contrast_mode, reduce_motion,
             hide_sponsor_button, density, sidebar_mode, default_landing_view, created_at, updated_at
         )
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
         ON CONFLICT(user_id) DO UPDATE SET
             theme = excluded.theme,
             date_time_format = excluded.date_time_format,
             highlight_color = excluded.highlight_color,
             secondary_color = excluded.secondary_color,
             high_contrast_mode = excluded.high_contrast_mode,
             reduce_motion = excluded.reduce_motion,
             hide_sponsor_button = excluded.hide_sponsor_button,
             density = excluded.density,
             sidebar_mode = excluded.sidebar_mode,
             default_landing_view = excluded.default_landing_view,
             updated_at = excluded.updated_at",
        &[
            SqlArg::Text(user_id.to_string()),
            SqlArg::Text(settings.theme.as_str().to_string()),
            SqlArg::Text(settings.date_time_format.as_str().to_string()),
            SqlArg::OptText(settings.highlight_color.clone()),
            SqlArg::OptText(settings.secondary_color.clone()),
            SqlArg::Bool(settings.high_contrast_mode),
            SqlArg::Bool(settings.reduce_motion),
            SqlArg::Bool(settings.hide_sponsor_button),
            SqlArg::Text(settings.density.as_str().to_string()),
            SqlArg::Text(settings.sidebar_mode.as_str().to_string()),
            SqlArg::Text(settings.default_landing_view.as_str().to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(())
}

async fn replace_ui_table_columns_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    columns: &[UiTableColumnSetting],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM user_ui_table_columns WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;

    let now = Utc::now();
    for column in columns {
        tx.execute(
            "INSERT INTO user_ui_table_columns (
                 user_id, facet, table_view_mode, column_id, column_order, visible,
                 created_at, updated_at
             )
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(user_id.to_string()),
                SqlArg::Text(column.facet.as_str().to_string()),
                SqlArg::Text(column.table_view_mode.as_str().to_string()),
                SqlArg::Text(column.column_id.clone()),
                SqlArg::I64(column.column_order as i64),
                SqlArg::Bool(column.visible),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

fn row_to_ui_settings(row: &SqlRow) -> AppResult<UiSettings> {
    Ok(UiSettings {
        user_id: row.text("user_id")?,
        theme: parse_ui_theme(row.text("theme")?)?,
        date_time_format: parse_ui_date_time_format(row.text("date_time_format")?)?,
        highlight_color: row.opt_text("highlight_color")?,
        secondary_color: row.opt_text("secondary_color")?,
        high_contrast_mode: row.bool("high_contrast_mode")?,
        reduce_motion: row.bool("reduce_motion")?,
        hide_sponsor_button: row.bool("hide_sponsor_button")?,
        density: parse_ui_density(row.text("density")?)?,
        sidebar_mode: parse_ui_sidebar_mode(row.text("sidebar_mode")?)?,
        default_landing_view: parse_ui_default_landing_view(row.text("default_landing_view")?)?,
        table_columns: Vec::new(),
        created_at: Some(row.timestamp("created_at")?),
        updated_at: Some(row.timestamp("updated_at")?),
    })
}

async fn load_ui_table_columns_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
) -> AppResult<Vec<UiTableColumnSetting>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT facet, table_view_mode, column_id, column_order, visible
           FROM user_ui_table_columns
          WHERE user_id = {}
          ORDER BY facet, table_view_mode, column_order, column_id",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_ui_table_column).collect()
}

async fn load_ui_table_columns(
    datastore: &StoreDatastore,
    user_id: &str,
) -> AppResult<Vec<UiTableColumnSetting>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT facet, table_view_mode, column_id, column_order, visible
           FROM user_ui_table_columns
          WHERE user_id = {}
          ORDER BY facet, table_view_mode, column_order, column_id",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_ui_table_column).collect()
}

fn row_to_ui_table_column(row: &SqlRow) -> AppResult<UiTableColumnSetting> {
    Ok(UiTableColumnSetting {
        facet: parse_ui_settings_facet(row.text("facet")?)?,
        table_view_mode: parse_ui_table_view_mode(row.text("table_view_mode")?)?,
        column_id: row.text("column_id")?,
        column_order: row.i32("column_order")?,
        visible: row.bool("visible")?,
    })
}

fn parse_ui_theme(value: String) -> AppResult<UiTheme> {
    UiTheme::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI theme {value:?}")))
}

fn parse_ui_date_time_format(value: String) -> AppResult<UiDateTimeFormat> {
    UiDateTimeFormat::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI date time format {value:?}")))
}

fn parse_ui_density(value: String) -> AppResult<UiDensity> {
    UiDensity::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI density {value:?}")))
}

fn parse_ui_sidebar_mode(value: String) -> AppResult<UiSidebarMode> {
    UiSidebarMode::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI sidebar mode {value:?}")))
}

fn parse_ui_default_landing_view(value: String) -> AppResult<UiDefaultLandingView> {
    UiDefaultLandingView::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI default landing view {value:?}")))
}

fn parse_ui_settings_facet(value: String) -> AppResult<UiSettingsFacet> {
    UiSettingsFacet::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI settings facet {value:?}")))
}

fn parse_ui_table_view_mode(value: String) -> AppResult<UiTableViewMode> {
    UiTableViewMode::parse(&value)
        .ok_or_else(|| AppError::Repository(format!("invalid UI table view mode {value:?}")))
}

async fn load_external_account_by_id(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, provider, connection_id, external_user_id, username,
                display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
           FROM user_external_accounts
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_external_account).transpose()
}

async fn load_external_account_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    load_external_account_by_id(SqlExec::Tx(tx), id).await
}

async fn load_external_account_by_provider_identity_tx(
    tx: &mut SqlTx<'_>,
    provider: &ExternalAccountProvider,
    connection_id: &str,
    external_user_id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, user_id, provider, connection_id, external_user_id, username,
                display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
           FROM user_external_accounts
          WHERE provider = {} AND connection_id = {} AND external_user_id = {}",
        &[
            SqlArg::Text(provider.as_str().to_string()),
            SqlArg::Text(connection_id.to_string()),
            SqlArg::Text(external_user_id.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_external_account).transpose()
}

async fn load_external_account_by_user_binding_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    provider: &ExternalAccountProvider,
    connection_id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, user_id, provider, connection_id, external_user_id, username,
                display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
           FROM user_external_accounts
          WHERE user_id = {} AND provider = {} AND connection_id = {}",
        &[
            SqlArg::Text(user_id.to_string()),
            SqlArg::Text(provider.as_str().to_string()),
            SqlArg::Text(connection_id.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_external_account).transpose()
}

async fn insert_external_account_tx(
    tx: &mut SqlTx<'_>,
    account: &UserExternalAccount,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO user_external_accounts (
             id, user_id, provider, connection_id, external_user_id, username,
             display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
          )
          VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(account.id.clone()),
            SqlArg::Text(account.user_id.clone()),
            SqlArg::Text(account.provider.as_str().to_string()),
            SqlArg::Text(account.connection_id.clone()),
            SqlArg::OptText(account.external_user_id.clone()),
            SqlArg::Text(account.username.clone()),
            SqlArg::OptText(account.display_name.clone()),
            SqlArg::OptText(account.avatar_url.clone()),
            SqlArg::Text(account.status.as_str().to_string()),
            SqlArg::OptTimestamp(account.verified_at),
            SqlArg::OptTimestamp(account.last_login_at),
            SqlArg::Timestamp(account.created_at),
            SqlArg::Timestamp(account.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_external_account_if_provider_identity_absent_tx(
    tx: &mut SqlTx<'_>,
    account: &UserExternalAccount,
) -> AppResult<bool> {
    let rows = tx
        .execute(
            "INSERT INTO user_external_accounts (
                 id, user_id, provider, connection_id, external_user_id, username,
                 display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
              )
              VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
              ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(account.id.clone()),
                SqlArg::Text(account.user_id.clone()),
                SqlArg::Text(account.provider.as_str().to_string()),
                SqlArg::Text(account.connection_id.clone()),
                SqlArg::OptText(account.external_user_id.clone()),
                SqlArg::Text(account.username.clone()),
                SqlArg::OptText(account.display_name.clone()),
                SqlArg::OptText(account.avatar_url.clone()),
                SqlArg::Text(account.status.as_str().to_string()),
                SqlArg::OptTimestamp(account.verified_at),
                SqlArg::OptTimestamp(account.last_login_at),
                SqlArg::Timestamp(account.created_at),
                SqlArg::Timestamp(account.updated_at),
            ],
        )
        .await?;
    Ok(rows == 1)
}

fn row_to_external_account(row: &SqlRow) -> AppResult<UserExternalAccount> {
    let provider = ExternalAccountProvider::parse(&row.text("provider")?)
        .ok_or_else(|| AppError::Repository("invalid external account provider".into()))?;
    let status = ExternalAccountStatus::parse(&row.text("status")?)
        .ok_or_else(|| AppError::Repository("invalid external account status".into()))?;
    Ok(UserExternalAccount {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        provider,
        connection_id: row.text("connection_id")?,
        external_user_id: row.opt_text("external_user_id")?,
        username: row.text("username")?,
        display_name: row.opt_text("display_name")?,
        avatar_url: row.opt_text("avatar_url")?,
        status,
        verified_at: row.opt_timestamp("verified_at")?,
        last_login_at: row.opt_timestamp("last_login_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;
    use scryer_application::{
        UserExternalAccountRepository, UserRepository, UserUiSettingsRepository,
    };
    use scryer_domain::{AppPermissionMask, LibraryPermissionMask, UserAuthorization};
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::sync::Mutex;

    use super::*;

    async fn test_store() -> UserStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create sqlite pool");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("enable foreign keys");
        sqlx::query(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY NOT NULL,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT,
                password_change_required INTEGER NOT NULL DEFAULT 0,
                account_kind TEXT NOT NULL DEFAULT 'local',
                status TEXT NOT NULL DEFAULT 'active',
                auth_session_version TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create users table");
        sqlx::query(
            "CREATE TABLE user_external_accounts (
                id TEXT PRIMARY KEY NOT NULL,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                provider TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                external_user_id TEXT,
                username TEXT NOT NULL,
                display_name TEXT,
                avatar_url TEXT,
                status TEXT NOT NULL DEFAULT 'pending_claim',
                verified_at TEXT,
                last_login_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                CHECK (provider IN ('plex', 'jellyfin', 'emby')),
                CHECK (status IN ('pending_claim', 'active', 'disabled'))
            )",
        )
        .execute(&pool)
        .await
        .expect("create user_external_accounts table");
        sqlx::query(
            "CREATE TABLE user_ui_settings (
                user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                theme TEXT NOT NULL DEFAULT 'dark',
                date_time_format TEXT NOT NULL DEFAULT 'locale',
                highlight_color TEXT,
                secondary_color TEXT,
                high_contrast_mode INTEGER NOT NULL DEFAULT 0,
                reduce_motion INTEGER NOT NULL DEFAULT 0,
                hide_sponsor_button INTEGER NOT NULL DEFAULT 0,
                density TEXT NOT NULL DEFAULT 'comfortable',
                sidebar_mode TEXT NOT NULL DEFAULT 'expanded',
                default_landing_view TEXT NOT NULL DEFAULT 'movies',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .expect("create user_ui_settings table");
        sqlx::query(
            "CREATE TABLE user_ui_table_columns (
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                facet TEXT NOT NULL,
                table_view_mode TEXT NOT NULL,
                column_id TEXT NOT NULL,
                column_order INTEGER NOT NULL,
                visible INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (user_id, facet, table_view_mode, column_id)
            )",
        )
        .execute(&pool)
        .await
        .expect("create user_ui_table_columns table");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
               ON user_external_accounts(provider, connection_id, external_user_id)",
        )
        .execute(&pool)
        .await
        .expect("create provider identity index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
               ON user_external_accounts(provider, connection_id, LOWER(username))
               WHERE status = 'pending_claim' AND external_user_id IS NULL",
        )
        .execute(&pool)
        .await
        .expect("create pending username index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
               ON user_external_accounts(user_id, provider, connection_id)",
        )
        .execute(&pool)
        .await
        .expect("create user provider connection index");

        UserStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(Mutex::new(())),
        })
    }

    fn test_user(id: &str) -> User {
        User {
            id: id.to_string(),
            username: format!("{id}_name"),
            password_hash: Some("hash".to_string()),
            password_change_required: false,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
                login_status: Default::default(),
                loaded: true,
            },
        }
    }

    fn test_account(
        id: &str,
        user_id: &str,
        provider: ExternalAccountProvider,
        connection_id: &str,
        external_user_id: &str,
    ) -> UserExternalAccount {
        let now = Utc::now();
        UserExternalAccount {
            id: id.to_string(),
            user_id: user_id.to_string(),
            provider,
            connection_id: connection_id.to_string(),
            external_user_id: Some(external_user_id.to_string()),
            username: format!("{external_user_id}_name"),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::PendingClaim,
            verified_at: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn emby_pending_invite_provider_round_trips_through_repository() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("emby_invited_user"))
            .await
            .expect("create invited user");
        let expected = test_account(
            "emby-invite",
            "emby_invited_user",
            ExternalAccountProvider::Emby,
            "emby-main",
            "emby-local-user-id",
        );

        let stored = UserExternalAccountRepository::create(&store, expected.clone())
            .await
            .expect("persist Emby pending invite");

        assert_eq!(stored, expected);
        assert_eq!(stored.provider, ExternalAccountProvider::Emby);
        assert_eq!(stored.status, ExternalAccountStatus::PendingClaim);
        assert_eq!(
            stored.external_user_id.as_deref(),
            Some("emby-local-user-id")
        );
    }

    #[tokio::test]
    async fn ui_settings_defaults_upsert_replace_and_cascade() {
        let store = test_store().await;
        let user = UserRepository::create(&store, test_user("ui_user"))
            .await
            .expect("create user");

        let missing = UserUiSettingsRepository::get_by_user_id(&store, &user.id)
            .await
            .expect("load missing UI settings");
        assert!(missing.is_none());

        let stored = UserUiSettingsRepository::upsert(
            &store,
            &user.id,
            UiSettingsUpdate {
                theme: UiTheme::Pride,
                date_time_format: UiDateTimeFormat::Iso24h,
                highlight_color: Some("#ff3366".to_string()),
                secondary_color: Some("#2277aa".to_string()),
                high_contrast_mode: true,
                reduce_motion: true,
                hide_sponsor_button: true,
                density: UiDensity::Compact,
                sidebar_mode: UiSidebarMode::Collapsed,
                default_landing_view: UiDefaultLandingView::Calendar,
                table_columns: vec![
                    UiTableColumnSetting {
                        facet: UiSettingsFacet::Movies,
                        table_view_mode: UiTableViewMode::Compact,
                        column_id: "name".to_string(),
                        column_order: 0,
                        visible: true,
                    },
                    UiTableColumnSetting {
                        facet: UiSettingsFacet::Series,
                        table_view_mode: UiTableViewMode::PosterTable,
                        column_id: "episodes".to_string(),
                        column_order: 1,
                        visible: false,
                    },
                ],
            },
        )
        .await
        .expect("upsert UI settings");
        assert_eq!(stored.user_id, user.id);
        assert_eq!(stored.theme, UiTheme::Pride);
        assert_eq!(stored.date_time_format, UiDateTimeFormat::Iso24h);
        assert!(stored.hide_sponsor_button);
        assert_eq!(stored.table_columns.len(), 2);
        assert_eq!(stored.table_columns[0].column_id, "name");
        assert_eq!(stored.table_columns[1].column_id, "episodes");

        let replaced = UserUiSettingsRepository::upsert(
            &store,
            &user.id,
            UiSettingsUpdate {
                theme: UiTheme::System,
                date_time_format: UiDateTimeFormat::Locale,
                highlight_color: None,
                secondary_color: None,
                high_contrast_mode: false,
                reduce_motion: false,
                hide_sponsor_button: false,
                density: UiDensity::Comfortable,
                sidebar_mode: UiSidebarMode::Expanded,
                default_landing_view: UiDefaultLandingView::Movies,
                table_columns: vec![UiTableColumnSetting {
                    facet: UiSettingsFacet::Anime,
                    table_view_mode: UiTableViewMode::Compact,
                    column_id: "status".to_string(),
                    column_order: 0,
                    visible: true,
                }],
            },
        )
        .await
        .expect("replace UI settings");
        assert_eq!(replaced.theme, UiTheme::System);
        assert_eq!(replaced.date_time_format, UiDateTimeFormat::Locale);
        assert!(!replaced.hide_sponsor_button);
        assert_eq!(replaced.table_columns.len(), 1);
        assert_eq!(replaced.table_columns[0].facet, UiSettingsFacet::Anime);
        assert_eq!(replaced.table_columns[0].column_id, "status");

        UserRepository::delete(&store, &user.id)
            .await
            .expect("delete user");
        let after_delete = UserUiSettingsRepository::get_by_user_id(&store, &user.id)
            .await
            .expect("load UI settings after cascade");
        assert!(after_delete.is_none());
    }

    #[tokio::test]
    async fn external_account_provider_identity_is_unique() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create first user");
        UserRepository::create(&store, test_user("user_b"))
            .await
            .expect("create second user");

        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        let duplicate = UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_b",
                "user_b",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn claim_external_account_provider_identity_returns_the_existing_owner() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create first user");
        UserRepository::create(&store, test_user("user_b"))
            .await
            .expect("create second user");

        let created = UserExternalAccountRepository::create_or_get_by_provider_identity(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("claim unowned provider identity");
        let existing = UserExternalAccountRepository::create_or_get_by_provider_identity(
            &store,
            test_account(
                "account_b",
                "user_b",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("return the winner instead of surfacing a unique violation");

        assert_eq!(created.id, "account_a");
        assert_eq!(existing.id, "account_a");
        assert_eq!(existing.user_id, "user_a");
    }

    #[tokio::test]
    async fn claim_external_account_rejects_a_different_identity_for_the_same_user_binding() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        UserExternalAccountRepository::create_or_get_by_provider_identity(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("claim first provider identity");

        let conflict = UserExternalAccountRepository::create_or_get_by_provider_identity(
            &store,
            test_account(
                "account_b",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_2",
            ),
        )
        .await;

        assert!(matches!(
            conflict,
            Err(AppError::Validation(message))
                if message == "external account is already linked to a different provider identity"
        ));
    }

    #[tokio::test]
    async fn claim_external_account_provider_identity_requires_a_nonempty_identity() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        for external_user_id in [None, Some(String::new()), Some("   ".to_string())] {
            let mut account = test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            );
            account.external_user_id = external_user_id;
            let result =
                UserExternalAccountRepository::create_or_get_by_provider_identity(&store, account)
                    .await;

            assert!(matches!(
                result,
                Err(AppError::Validation(message))
                    if message == "external account identity must be present when claiming"
            ));
        }
    }

    #[tokio::test]
    async fn pending_external_account_username_is_unique_for_connection() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create first user");
        UserRepository::create(&store, test_user("user_b"))
            .await
            .expect("create second user");

        let mut first = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        first.external_user_id = None;
        first.username = "JellyUser".to_string();
        UserExternalAccountRepository::create(&store, first)
            .await
            .expect("create pending account");

        let mut duplicate = test_account(
            "account_b",
            "user_b",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_2",
        );
        duplicate.external_user_id = None;
        duplicate.username = "jellyuser".to_string();
        let duplicate = UserExternalAccountRepository::create(&store, duplicate).await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn pending_external_account_can_be_found_by_username() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        let mut account = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        account.external_user_id = None;
        account.username = "JellyUser".to_string();
        UserExternalAccountRepository::create(&store, account)
            .await
            .expect("create pending account");

        let found = UserExternalAccountRepository::get_pending_claim_by_provider_username(
            &store,
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "jellyuser",
        )
        .await
        .expect("lookup pending account")
        .expect("pending account exists");

        assert_eq!(found.id, "account_a");
        assert_eq!(found.external_user_id, None);
    }

    #[tokio::test]
    async fn external_account_user_provider_connection_is_unique() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Plex,
                "plex_main",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        let duplicate = UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_b",
                "user_a",
                ExternalAccountProvider::Plex,
                "plex_main",
                "external_2",
            ),
        )
        .await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn external_account_status_transition_persists() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");
        let mut new_account = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        let initial_login_at = Utc::now();
        new_account.status = ExternalAccountStatus::Active;
        new_account.verified_at = Some(initial_login_at);
        new_account.last_login_at = Some(initial_login_at);
        let mut account = UserExternalAccountRepository::create(&store, new_account)
            .await
            .expect("create account");
        assert_eq!(account.last_login_at, Some(initial_login_at));

        let listed = UserExternalAccountRepository::list_by_user_id(&store, "user_a")
            .await
            .expect("list accounts");
        assert_eq!(listed[0].last_login_at, Some(initial_login_at));

        account.status = ExternalAccountStatus::Active;
        let now = Utc::now();
        account.verified_at = Some(now);
        account.last_login_at = Some(now);
        let updated = UserExternalAccountRepository::update(&store, account)
            .await
            .expect("update account");

        assert_eq!(updated.status, ExternalAccountStatus::Active);
        assert!(updated.verified_at.is_some());
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn deleting_user_cascades_external_accounts() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");
        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        UserRepository::delete(&store, "user_a")
            .await
            .expect("delete user");

        let remaining = UserExternalAccountRepository::list_by_user_id(&store, "user_a")
            .await
            .expect("list accounts");
        assert!(remaining.is_empty());
    }
}
