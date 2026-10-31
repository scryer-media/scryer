//! Indexer clients have their own key family and never authenticate as users.

use crate::api_keys::{
    generate_api_key_material, hash_key_material, parse_key_material, verify_key_material,
};
use crate::error_reasons::indexer_clients as reasons;
use crate::{AppError, AppResult, AppUseCase};
use scryer_domain::{AppPermissionMask, User};

pub const INDEXER_CLIENT_KEY_PREFIX: &str = "sck";
const KEY_DOMAIN: &str = "indexer-client-key";

mod model;
pub use model::*;
mod management;
pub use management::{AuthenticatedIndexerClient, CreatedIndexerClient};
mod search;
pub use scryer_plugin_sdk::PluginNabOperation as IndexerClientSearchOperation;
pub use search::IndexerClientSearchRequest;
mod artifact;
mod caps;
pub use artifact::IndexerClientArtifactReference;
pub mod usage;
pub use usage::IndexerClientUseOutcome;

#[cfg(test)]
mod authentication_tests;

pub struct IssuedIndexerClientKey {
    pub lookup_id: String,
    pub secret_hash: String,
    /// Returned once to the operator, never stored in the client record.
    pub raw_key: String,
}

impl std::fmt::Debug for IssuedIndexerClientKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedIndexerClientKey")
            .finish_non_exhaustive()
    }
}

pub fn issue_indexer_client_key(signing_salt: &str) -> AppResult<IssuedIndexerClientKey> {
    let (lookup_id, secret) = generate_api_key_material()?;
    Ok(IssuedIndexerClientKey {
        secret_hash: hash_key_material(signing_salt, KEY_DOMAIN, &lookup_id, &secret),
        raw_key: format!("{INDEXER_CLIENT_KEY_PREFIX}_{lookup_id}.{secret}"),
        lookup_id,
    })
}

/// Only the lookup portion is exposed before verification; no user resolution
/// or personal-key fallback participates in this authentication family.
pub fn indexer_client_key_lookup(raw_key: &str) -> Option<String> {
    parse_key_material(raw_key, INDEXER_CLIENT_KEY_PREFIX).map(|(lookup, _)| lookup)
}

pub fn verify_indexer_client_key(signing_salt: &str, raw_key: &str, stored_hash: &str) -> bool {
    let Some((lookup, secret)) = parse_key_material(raw_key, INDEXER_CLIENT_KEY_PREFIX) else {
        return false;
    };
    verify_key_material(signing_salt, KEY_DOMAIN, &lookup, &secret, stored_hash)
}

impl AppUseCase {
    /// Key changes are a step-up action and use the same step-up as personal
    /// API keys: a user with MFA set up must verify, a user without it is not
    /// asked to enroll.
    pub async fn require_indexer_client_key_management(
        &self,
        actor: &User,
        management_session: bool,
        step_up_verified_until: Option<i64>,
    ) -> AppResult<()> {
        self.require_indexer_client_management_session(actor, management_session)
            .await?;
        self.require_api_key_mfa_step_up(actor, step_up_verified_until)
            .await
    }

    /// The interface derives `management_session` from trusted request
    /// context, never from a mutation input.
    pub async fn require_indexer_client_management_session(
        &self,
        actor: &User,
        management_session: bool,
    ) -> AppResult<()> {
        if !management_session {
            return Err(AppError::unauthorized_reason(
                reasons::INTERACTIVE_SESSION_REQUIRED,
                "Sign in to manage indexer clients; the local network login bypass and API keys cannot manage them",
            ));
        }
        if !self
            .authorization_for_actor(actor)
            .await?
            .app
            .contains(AppPermissionMask::ALL)
        {
            return Err(AppError::unauthorized_reason(
                reasons::ALL_PERMISSIONS_REQUIRED,
                "Every application permission is required for indexer client management",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_management_rejects_bypass_and_any_missing_permission() {
        let (app, mut actor) = crate::lib_tests::bootstrap();
        actor.authorization.loaded = true;
        actor.authorization.app = AppPermissionMask::ALL;
        let bypass = app
            .require_indexer_client_key_management(&actor, false, Some(i64::MAX))
            .await
            .unwrap_err();
        assert_eq!(bypass.reason(), Some(reasons::INTERACTIVE_SESSION_REQUIRED));
        for permission in scryer_domain::AppPermission::ALL {
            actor.authorization.app = AppPermissionMask::from_bits_retain(
                AppPermissionMask::ALL.bits()
                    & !AppPermissionMask::from_permission(*permission).bits(),
            );
            let denied = app
                .require_indexer_client_key_management(&actor, true, Some(i64::MAX))
                .await
                .unwrap_err();
            assert_eq!(denied.reason(), Some(reasons::ALL_PERMISSIONS_REQUIRED));
        }
        actor.authorization.app = AppPermissionMask::ALL;
        assert!(
            app.require_indexer_client_key_management(&actor, true, None)
                .await
                .is_ok()
        );
    }

    #[test]
    fn client_keys_are_separate_from_personal_keys_and_domains() {
        let issued = issue_indexer_client_key("fixture-signing-salt").unwrap();
        assert!(crate::parse_api_key(&issued.raw_key).is_none());
        assert_eq!(
            indexer_client_key_lookup(&issued.raw_key).as_deref(),
            Some(issued.lookup_id.as_str())
        );
        assert!(verify_indexer_client_key(
            "fixture-signing-salt",
            &issued.raw_key,
            &issued.secret_hash
        ));
        assert!(!verify_indexer_client_key(
            "other-salt",
            &issued.raw_key,
            &issued.secret_hash
        ));
        let (_, secret) = parse_key_material(&issued.raw_key, INDEXER_CLIENT_KEY_PREFIX).unwrap();
        let personal_hash = hash_key_material(
            "fixture-signing-salt",
            "api-key",
            &issued.lookup_id,
            &secret,
        );
        assert!(!verify_indexer_client_key(
            "fixture-signing-salt",
            &issued.raw_key,
            &personal_hash
        ));
        let personal_key = format!("ska_{}.{}", issued.lookup_id, secret);
        assert!(crate::parse_api_key(&personal_key).is_some());
        assert!(indexer_client_key_lookup(&personal_key).is_none());
        assert!(!verify_indexer_client_key(
            "fixture-signing-salt",
            &personal_key,
            &issued.secret_hash
        ));
        let debug = format!("{issued:?}");
        assert!(!debug.contains(&secret) && !debug.contains(&issued.raw_key));
    }

    #[test]
    fn malformed_or_tampered_client_keys_do_not_verify() {
        let first = issue_indexer_client_key("fixture-signing-salt").unwrap();
        let second = issue_indexer_client_key("fixture-signing-salt").unwrap();
        assert!(!verify_indexer_client_key(
            "fixture-signing-salt",
            &second.raw_key,
            &first.secret_hash
        ));
        for key in ["", "sck_", "sck_x.y", "Bearer sck_x.y", "sck_x.y.z"] {
            assert!(indexer_client_key_lookup(key).is_none());
            assert!(!verify_indexer_client_key(
                "fixture-signing-salt",
                key,
                &first.secret_hash
            ));
        }
        assert!(!verify_indexer_client_key(
            "fixture-signing-salt",
            &first.raw_key,
            "invalid-hash"
        ));
    }
}
