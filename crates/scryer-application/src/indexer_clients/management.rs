use super::*;
use std::collections::HashSet;

pub struct CreatedIndexerClient {
    pub client: IndexerClientRecord,
    pub raw_key: String,
}

impl std::fmt::Debug for CreatedIndexerClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreatedIndexerClient")
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct AuthenticatedIndexerClient {
    pub client: IndexerClientRecord,
}

impl AuthenticatedIndexerClient {
    pub fn principal(&self) -> crate::SearchPrincipal {
        crate::SearchPrincipal::IndexerClient {
            client_id: self.client.id.clone(),
            key_generation: self.client.key_generation,
            grants_generation: self.client.revision.to_string(),
        }
    }
}

fn invalid_key() -> AppError {
    AppError::unauthorized_reason(reasons::KEY_INVALID, "Invalid indexer client key")
}

fn changed() -> AppError {
    AppError::validation_refused(
        reasons::CONFIGURATION_CHANGED,
        "Indexer client configuration changed; reload it",
    )
}

fn increment(value: u64) -> AppResult<u64> {
    value
        .checked_add(1)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| {
            AppError::validation_refused(
                reasons::REVISION_EXHAUSTED,
                "Indexer client revision capacity is exhausted",
            )
        })
}

impl AppUseCase {
    pub(crate) fn indexer_client_repository(&self) -> AppResult<&dyn IndexerClientRepository> {
        self.services
            .integrations
            .indexer_clients
            .available()
            .map(|repository| repository.as_ref())
            .ok_or_else(|| {
                AppError::temporary_unavailable_reason(
                    reasons::UNAVAILABLE,
                    "Indexer client management is unavailable",
                    None,
                )
            })
    }

    pub async fn list_indexer_clients(
        &self,
        actor: &User,
        management_session: bool,
    ) -> AppResult<Vec<IndexerClientRecord>> {
        self.require_indexer_client_management_session(actor, management_session)
            .await?;
        self.indexer_client_repository()?.list().await
    }

    pub async fn create_indexer_client(
        &self,
        actor: &User,
        management_session: bool,
        step_up_verified_until: Option<i64>,
        configuration: IndexerClientConfiguration,
    ) -> AppResult<CreatedIndexerClient> {
        self.require_indexer_client_key_management(
            actor,
            management_session,
            step_up_verified_until,
        )
        .await?;
        let configuration = self
            .validate_indexer_client_configuration(configuration)
            .await?;
        let key = issue_indexer_client_key(&self.auth.jwt_signing_salt)?;
        let now = self.runtime.environment.now();
        let record = IndexerClientRecord {
            id: scryer_domain::Id::new().0,
            configuration,
            lookup_id: Some(key.lookup_id),
            secret_hash: Some(key.secret_hash),
            key_generation: 1,
            revision: 1,
            last_used_at: None,
            last_outcome_code: None,
            created_at: now,
            updated_at: now,
        };
        if !self
            .indexer_client_repository()?
            .save(&record, None)
            .await?
        {
            return Err(changed());
        }
        Ok(CreatedIndexerClient {
            client: record,
            raw_key: key.raw_key,
        })
    }

    pub async fn update_indexer_client(
        &self,
        actor: &User,
        management_session: bool,
        step_up_verified_until: Option<i64>,
        id: &str,
        expected_revision: u64,
        configuration: IndexerClientConfiguration,
    ) -> AppResult<IndexerClientRecord> {
        self.require_indexer_client_key_management(
            actor,
            management_session,
            step_up_verified_until,
        )
        .await?;
        let configuration = self
            .validate_indexer_client_configuration(configuration)
            .await?;
        let mut record = self
            .indexer_client_for_update(id, expected_revision)
            .await?;
        if configuration.enabled && record.secret_hash.is_none() {
            return Err(AppError::validation_refused(
                reasons::ENABLE_REQUIRES_KEY,
                "Rotate in a new key before enabling this client; its key was revoked",
            ));
        }
        record.configuration = configuration;
        self.save_indexer_client_update(record, expected_revision)
            .await
    }

    pub async fn rotate_indexer_client_key(
        &self,
        actor: &User,
        management_session: bool,
        step_up_verified_until: Option<i64>,
        id: &str,
        expected_revision: u64,
    ) -> AppResult<CreatedIndexerClient> {
        self.require_indexer_client_key_management(
            actor,
            management_session,
            step_up_verified_until,
        )
        .await?;
        let mut record = self
            .indexer_client_for_update(id, expected_revision)
            .await?;
        let key = issue_indexer_client_key(&self.auth.jwt_signing_salt)?;
        record.lookup_id = Some(key.lookup_id);
        record.secret_hash = Some(key.secret_hash);
        record.key_generation = increment(record.key_generation)?;
        let record = self
            .save_indexer_client_update(record, expected_revision)
            .await?;
        Ok(CreatedIndexerClient {
            client: record,
            raw_key: key.raw_key,
        })
    }

    pub async fn revoke_indexer_client_key(
        &self,
        actor: &User,
        management_session: bool,
        step_up_verified_until: Option<i64>,
        id: &str,
        expected_revision: u64,
    ) -> AppResult<IndexerClientRecord> {
        self.require_indexer_client_key_management(
            actor,
            management_session,
            step_up_verified_until,
        )
        .await?;
        let mut record = self
            .indexer_client_for_update(id, expected_revision)
            .await?;
        record.lookup_id = None;
        record.secret_hash = None;
        // A client with no key cannot be used, so it is switched off with the
        // key. Rotating a key in later does not switch it back on.
        record.configuration.enabled = false;
        record.key_generation = increment(record.key_generation)?;
        self.save_indexer_client_update(record, expected_revision)
            .await
    }

    async fn indexer_client_for_update(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> AppResult<IndexerClientRecord> {
        let record = self
            .indexer_client_repository()?
            .get_by_id(id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(reasons::NOT_FOUND, "Indexer client no longer exists")
            })?;
        if record.revision != expected_revision {
            return Err(changed());
        }
        Ok(record)
    }

    async fn save_indexer_client_update(
        &self,
        mut record: IndexerClientRecord,
        previous: u64,
    ) -> AppResult<IndexerClientRecord> {
        record.revision = increment(previous)?;
        record.updated_at = self.runtime.environment.now();
        if !self
            .indexer_client_repository()?
            .save(&record, Some(previous))
            .await?
        {
            return Err(changed());
        }
        Ok(record)
    }

    async fn validate_indexer_client_configuration(
        &self,
        mut configuration: IndexerClientConfiguration,
    ) -> AppResult<IndexerClientConfiguration> {
        configuration.name = configuration.name.trim().to_string();
        if configuration.name.is_empty()
            || configuration.name.len() > 120
            || configuration.name.chars().any(char::is_control)
        {
            return Err(AppError::validation_refused(
                reasons::NAME_INVALID,
                "Indexer client name must contain 1 to 120 bytes without control characters",
            ));
        }
        let mut protocols = HashSet::new();
        configuration
            .protocols
            .retain(|protocol| protocols.insert(*protocol));
        let mut sources = HashSet::new();
        for grant in &mut configuration.grants {
            if !sources.insert(grant.indexer_id.clone()) {
                return Err(AppError::validation_refused(
                    reasons::GRANT_INVALID,
                    "Each indexer may be granted only once",
                ));
            }
            let source = self
                .services
                .integrations
                .indexer_configs
                .get_by_id(&grant.indexer_id)
                .await?
                .ok_or_else(|| {
                    AppError::validation_refused(
                        reasons::GRANT_INVALID,
                        "A granted indexer no longer exists",
                    )
                })?;
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::validation_refused(
                        reasons::UNAVAILABLE,
                        "Indexer provider is unavailable",
                    )
                })?;
            if provider
                .management_capabilities_for_provider(&source.provider_type)
                .supports_managed_children_sync
            {
                return Err(AppError::validation_refused(
                    reasons::GRANT_INVALID,
                    "Select managed indexers individually",
                ));
            }
            if let Some(categories) = &mut grant.categories {
                for category in categories.iter_mut() {
                    *category = category.trim().to_string();
                    if category.is_empty()
                        || category.chars().any(char::is_control)
                        || ((source.is_direct_nab() || source.is_prowlarr_nab_proxy())
                            && category
                                .parse::<u32>()
                                .ok()
                                .filter(|value| *value > 0)
                                .is_none())
                    {
                        return Err(AppError::validation_refused(
                            reasons::GRANT_INVALID,
                            "Category restrictions must contain valid category values",
                        ));
                    }
                }
                categories.sort();
                categories.dedup();
            }
        }
        configuration
            .grants
            .sort_by(|left, right| left.indexer_id.cmp(&right.indexer_id));
        Ok(configuration)
    }

    pub async fn authenticate_indexer_client(
        &self,
        raw_key: &str,
    ) -> AppResult<AuthenticatedIndexerClient> {
        let lookup = indexer_client_key_lookup(raw_key).ok_or_else(invalid_key)?;
        let client = self
            .indexer_client_repository()?
            .get_by_lookup_id(&lookup)
            .await?
            .ok_or_else(invalid_key)?;
        if !client.configuration.enabled
            || !client.secret_hash.as_ref().is_some_and(|hash| {
                verify_indexer_client_key(&self.auth.jwt_signing_salt, raw_key, hash)
            })
        {
            return Err(invalid_key());
        }
        Ok(AuthenticatedIndexerClient { client })
    }

    /// Recheck before delivering a retained page. A grant change, rotation,
    /// disablement or revocation during source execution denies stale access.
    pub async fn revalidate_indexer_client(
        &self,
        authenticated: &AuthenticatedIndexerClient,
    ) -> AppResult<()> {
        let current = self
            .indexer_client_repository()?
            .get_by_id(&authenticated.client.id)
            .await?
            .ok_or_else(invalid_key)?;
        if !current.configuration.enabled
            || current.secret_hash.is_none()
            || current.configuration != authenticated.client.configuration
            || current.revision != authenticated.client.revision
            || current.key_generation != authenticated.client.key_generation
        {
            return Err(invalid_key());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Provider(bool);
    impl crate::IndexerPluginProvider for Provider {
        fn client_for_provider(
            &self,
            _: &scryer_domain::IndexerConfig,
        ) -> Option<Arc<dyn crate::IndexerClient>> {
            None
        }
        fn available_provider_types(&self) -> Vec<String> {
            vec!["newznab".into()]
        }
        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
        fn management_capabilities_for_provider(
            &self,
            _: &str,
        ) -> scryer_domain::IndexerManagementCapabilities {
            scryer_domain::IndexerManagementCapabilities {
                supports_managed_children_sync: self.0,
                ..Default::default()
            }
        }
    }

    #[tokio::test]
    async fn grants_fail_closed_without_provider_and_refuse_parent_or_nonnumeric_nab_categories() {
        let (mut app, _) = crate::lib_tests::bootstrap();
        let now = app.runtime.environment.now();
        app.services
            .integrations
            .indexer_configs
            .create(
                serde_json::from_value(serde_json::json!({
                    "id": "grant-source", "name": "Grant fixture", "provider_type": "newznab",
                    "base_url": "https://fixture.invalid", "is_enabled": true,
                    "enable_interactive_search": true, "enable_auto_search": false,
                    "created_at": now, "updated_at": now
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let mut configuration = IndexerClientConfiguration {
            name: "Grant fixture".into(),
            enabled: true,
            protocols: vec![IndexerClientProtocol::Newznab],
            grants: vec![IndexerClientGrant {
                indexer_id: "grant-source".into(),
                categories: None,
            }],
        };
        app.services.integrations.plugin_provider = crate::RuntimeFeature::Disabled;
        assert_eq!(
            app.validate_indexer_client_configuration(configuration.clone())
                .await
                .unwrap_err()
                .reason(),
            Some(reasons::UNAVAILABLE)
        );
        app.services.integrations.plugin_provider =
            crate::RuntimeFeature::Enabled(Arc::new(Provider(true)));
        assert_eq!(
            app.validate_indexer_client_configuration(configuration.clone())
                .await
                .unwrap_err()
                .reason(),
            Some(reasons::GRANT_INVALID)
        );
        app.services.integrations.plugin_provider =
            crate::RuntimeFeature::Enabled(Arc::new(Provider(false)));
        for category in ["software", "0", "4294967296"] {
            configuration.grants[0].categories = Some(vec![category.into()]);
            assert_eq!(
                app.validate_indexer_client_configuration(configuration.clone())
                    .await
                    .unwrap_err()
                    .reason(),
                Some(reasons::GRANT_INVALID)
            );
        }
        configuration.grants[0].categories = Some(vec!["100123".into()]);
        assert!(
            app.validate_indexer_client_configuration(configuration)
                .await
                .is_ok()
        );
    }
}
