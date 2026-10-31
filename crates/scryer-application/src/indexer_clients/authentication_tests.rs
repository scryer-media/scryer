use super::*;
use std::sync::{Arc, Mutex};

struct ClientFixture(Mutex<Option<IndexerClientRecord>>);

#[async_trait::async_trait]
impl IndexerClientRepository for ClientFixture {
    async fn list(&self) -> AppResult<Vec<IndexerClientRecord>> {
        Ok(self.0.lock().unwrap().iter().cloned().collect())
    }
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerClientRecord>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|client| client.id == id)
            .cloned())
    }
    async fn get_by_lookup_id(&self, lookup: &str) -> AppResult<Option<IndexerClientRecord>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|client| client.lookup_id.as_deref() == Some(lookup))
            .cloned())
    }
    async fn save(&self, _: &IndexerClientRecord, _: Option<u64>) -> AppResult<bool> {
        panic!("authentication cannot modify client configuration")
    }
    async fn record_use(
        &self,
        _: &str,
        _: u64,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<()> {
        panic!("authentication cannot modify usage status")
    }
}

#[tokio::test]
async fn authentication_and_delivery_revalidation_reject_changed_authority() {
    let (mut app, _) = crate::lib_tests::bootstrap();
    let issued = issue_indexer_client_key(&app.auth.jwt_signing_salt).unwrap();
    let now = app.runtime.environment.now();
    let original = IndexerClientRecord {
        id: "client-a".into(),
        configuration: IndexerClientConfiguration {
            name: "Client fixture".into(),
            enabled: true,
            protocols: vec![IndexerClientProtocol::Newznab],
            grants: vec![IndexerClientGrant {
                indexer_id: "source-a".into(),
                categories: None,
            }],
        },
        lookup_id: Some(issued.lookup_id),
        secret_hash: Some(issued.secret_hash),
        key_generation: 1,
        revision: 1,
        last_used_at: None,
        last_outcome_code: None,
        created_at: now,
        updated_at: now,
    };
    let repository = Arc::new(ClientFixture(Mutex::new(Some(original.clone()))));
    app.services.integrations.indexer_clients = crate::RuntimeFeature::Enabled(repository.clone());
    let authenticated = app
        .authenticate_indexer_client(&issued.raw_key)
        .await
        .unwrap();
    assert!(matches!(
        authenticated.principal(),
        crate::SearchPrincipal::IndexerClient {
            key_generation: 1,
            ..
        }
    ));
    assert!(app.revalidate_indexer_client(&authenticated).await.is_ok());
    assert!(app.authenticate_api_key(&issued.raw_key).await.is_err());

    let other = issue_indexer_client_key(&app.auth.jwt_signing_salt).unwrap();
    assert_eq!(
        app.authenticate_indexer_client(&other.raw_key)
            .await
            .unwrap_err()
            .reason(),
        Some(reasons::KEY_INVALID)
    );

    for mutation in 0..6 {
        let mut changed = original.clone();
        match mutation {
            0 => changed.configuration.enabled = false,
            1 => changed.key_generation += 1,
            2 => changed.revision += 1,
            3 => {
                changed.lookup_id = None;
                changed.secret_hash = None;
            }
            // A deleted source cascades its grant without a client edit.
            4 => changed.configuration.grants.clear(),
            _ => changed.configuration.protocols.clear(),
        }
        *repository.0.lock().unwrap() = Some(changed);
        assert_eq!(
            app.revalidate_indexer_client(&authenticated)
                .await
                .unwrap_err()
                .reason(),
            Some(reasons::KEY_INVALID)
        );
        if matches!(mutation, 0 | 3) {
            assert_eq!(
                app.authenticate_indexer_client(&issued.raw_key)
                    .await
                    .unwrap_err()
                    .reason(),
                Some(reasons::KEY_INVALID)
            );
        }
    }
    *repository.0.lock().unwrap() = None;
    assert_eq!(
        app.revalidate_indexer_client(&authenticated)
            .await
            .unwrap_err()
            .reason(),
        Some(reasons::KEY_INVALID)
    );
}
