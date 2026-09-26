//! Dual-dialect storage for Lists.
//!
//! One store type implements the five list repository ports, one per file:
//! subscriptions (with routes and sync runs), memberships, exclusions, and
//! member accounts with member policies. Every table the store writes was
//! created by one migration, and every row that belongs to a person cascades
//! from `users`, so the store itself never has to clean up after a deleted
//! member.
//!
//! The account credential is the only secret here. It is encrypted with the
//! datastore key on the way in and decrypted on the way out through the same
//! helpers every other at-rest secret uses, so an assembly without a key stores
//! it in the clear exactly as it would a download client's password, and an
//! assembly with one never writes plaintext.

mod accounts;
mod exclusions;
mod memberships;
mod subscriptions;
#[cfg(test)]
mod tests;

use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use serde::de::DeserializeOwned;

use crate::config_store::current_encryption_key;
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlRow, StoreDatastore};

#[derive(Clone)]
pub struct ListStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl ListStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

/// `{}, {}, ...` for an `IN (...)` list of `count` values.
pub(super) fn placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn json_column<T: DeserializeOwned>(
    row: &SqlRow,
    column: &str,
    fallback: &str,
) -> AppResult<T> {
    let raw = row.opt_text(column)?;
    let text = raw
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback);
    serde_json::from_str(text)
        .map_err(|error| AppError::Repository(format!("invalid JSON in {column}: {error}")))
}

pub(super) fn json_arg<T: serde::Serialize>(value: &T) -> AppResult<SqlArg> {
    serde_json::to_string(value)
        .map(SqlArg::Text)
        .map_err(|error| AppError::Repository(format!("failed to encode JSON column: {error}")))
}

pub(super) fn parse_or_repo_err<T>(
    column: &str,
    raw: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> AppResult<T> {
    parse(raw).ok_or_else(|| AppError::Repository(format!("unknown {column} value '{raw}'")))
}
