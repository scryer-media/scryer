//! Members' list-provider accounts and members' list policies.
//!
//! The credential column is the one secret in Lists. It is encrypted with the
//! datastore key through the same helpers every other at-rest secret uses, and
//! decrypted only into the in-memory domain value the sync engine hands to a
//! plugin call.

use async_trait::async_trait;
use scryer_application::lists::{UserListAccountRepository, UserListPolicyRepository};
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    ListAccountCredential, ListPolicy, UserListAccount, UserListAccountStatus, UserListPolicy,
};

use super::{ListStore, parse_or_repo_err, placeholders};
use crate::config_store::{decrypt_value, maybe_encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlRow, SqlRuntime};

const ACCOUNT_COLUMNS: &str = "id, user_id, provider, external_user_id, username, display_name,
    credential_encrypted, status, error_message, linked_at, last_used_at, last_refresh_at,
    updated_at";

const CREDENTIAL_LABEL: &str = "list account credential";

#[async_trait]
impl UserListAccountRepository for ListStore {
    async fn create(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        let key = self.encryption_key()?;
        let args = account_args(&account, key.as_ref())?;
        SqlRuntime::execute_write(
            &self.datastore,
            "create_user_list_account",
            &format!(
                "INSERT INTO user_list_accounts ({ACCOUNT_COLUMNS}) VALUES ({})",
                placeholders(13)
            ),
            args,
        )
        .await?;
        Ok(account)
    }

    async fn update(&self, account: UserListAccount) -> AppResult<UserListAccount> {
        let key = self.encryption_key()?;
        let credential = encrypt_credential(&account.credential, key.as_ref())?;
        let changed = SqlRuntime::execute_write(
            &self.datastore,
            "update_user_list_account",
            "UPDATE user_list_accounts
                SET username = {}, display_name = {}, credential_encrypted = {}, status = {},
                    error_message = {}, last_used_at = {}, last_refresh_at = {}, updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(account.username.clone()),
                SqlArg::OptText(account.display_name.clone()),
                SqlArg::Text(credential),
                SqlArg::Text(account.status.as_str().to_string()),
                SqlArg::OptText(account.error_message.clone()),
                SqlArg::OptTimestamp(account.last_used_at),
                SqlArg::OptTimestamp(account.last_refresh_at),
                SqlArg::Timestamp(account.updated_at),
                SqlArg::Text(account.id.clone()),
            ],
        )
        .await?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("list account {}", account.id)));
        }
        Ok(account)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserListAccount>> {
        let key = self.encryption_key()?;
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT {ACCOUNT_COLUMNS} FROM user_list_accounts WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(|row| row_to_account(row, key.as_ref()))
        .transpose()
    }

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserListAccount>> {
        let key = self.encryption_key()?;
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACCOUNT_COLUMNS} FROM user_list_accounts
                  WHERE user_id = {{}} ORDER BY provider, linked_at, id"
            ),
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter()
            .map(|row| row_to_account(row, key.as_ref()))
            .collect()
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let changed = SqlRuntime::execute_write(
            &self.datastore,
            "delete_user_list_account",
            "DELETE FROM user_list_accounts WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("list account {id}")));
        }
        Ok(())
    }
}

#[async_trait]
impl UserListPolicyRepository for ListStore {
    async fn get(&self, user_id: &str) -> AppResult<Option<UserListPolicy>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT user_id, policy, updated_by_user_id, updated_at
               FROM user_list_policies WHERE user_id = {}",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_policy)
        .transpose()
    }

    async fn set(&self, policy: UserListPolicy) -> AppResult<UserListPolicy> {
        SqlRuntime::execute_write(
            &self.datastore,
            "set_user_list_policy",
            "INSERT INTO user_list_policies (user_id, policy, updated_by_user_id, updated_at)
             VALUES ({}, {}, {}, {})
             ON CONFLICT (user_id) DO UPDATE SET
                 policy = excluded.policy,
                 updated_by_user_id = excluded.updated_by_user_id,
                 updated_at = excluded.updated_at",
            vec![
                SqlArg::Text(policy.user_id.clone()),
                SqlArg::Text(policy.policy.as_str().to_string()),
                SqlArg::OptText(policy.updated_by_user_id.clone()),
                SqlArg::Timestamp(policy.updated_at),
            ],
        )
        .await?;
        Ok(policy)
    }

    async fn list(&self) -> AppResult<Vec<UserListPolicy>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT user_id, policy, updated_by_user_id, updated_at
               FROM user_list_policies ORDER BY user_id",
            &[],
        )
        .await?;
        rows.iter().map(row_to_policy).collect()
    }
}

fn encrypt_credential(
    credential: &ListAccountCredential,
    key: Option<&EncryptionKey>,
) -> AppResult<String> {
    let plain = serde_json::to_string(credential).map_err(|error| {
        AppError::Repository(format!("failed to encode {CREDENTIAL_LABEL}: {error}"))
    })?;
    maybe_encrypt_value(key, &plain)
}

fn decrypt_credential(
    stored: String,
    key: Option<&EncryptionKey>,
) -> AppResult<ListAccountCredential> {
    let plain = decrypt_value(key, stored, CREDENTIAL_LABEL, true)?;
    serde_json::from_str(&plain)
        .map_err(|error| AppError::Repository(format!("invalid {CREDENTIAL_LABEL}: {error}")))
}

fn account_args(account: &UserListAccount, key: Option<&EncryptionKey>) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(account.id.clone()),
        SqlArg::Text(account.user_id.clone()),
        SqlArg::Text(account.provider.clone()),
        SqlArg::Text(account.external_user_id.clone()),
        SqlArg::Text(account.username.clone()),
        SqlArg::OptText(account.display_name.clone()),
        SqlArg::Text(encrypt_credential(&account.credential, key)?),
        SqlArg::Text(account.status.as_str().to_string()),
        SqlArg::OptText(account.error_message.clone()),
        SqlArg::Timestamp(account.linked_at),
        SqlArg::OptTimestamp(account.last_used_at),
        SqlArg::OptTimestamp(account.last_refresh_at),
        SqlArg::Timestamp(account.updated_at),
    ])
}

fn row_to_account(row: &SqlRow, key: Option<&EncryptionKey>) -> AppResult<UserListAccount> {
    Ok(UserListAccount {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        provider: row.text("provider")?,
        external_user_id: row.text("external_user_id")?,
        username: row.text("username")?,
        display_name: row.opt_text("display_name")?,
        credential: decrypt_credential(row.text("credential_encrypted")?, key)?,
        status: parse_or_repo_err("status", &row.text("status")?, UserListAccountStatus::parse)?,
        error_message: row.opt_text("error_message")?,
        linked_at: row.timestamp("linked_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        last_refresh_at: row.opt_timestamp("last_refresh_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_policy(row: &SqlRow) -> AppResult<UserListPolicy> {
    Ok(UserListPolicy {
        user_id: row.text("user_id")?,
        policy: parse_or_repo_err("policy", &row.text("policy")?, ListPolicy::parse)?,
        updated_by_user_id: row.opt_text("updated_by_user_id")?,
        updated_at: row.timestamp("updated_at")?,
    })
}
