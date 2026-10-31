use super::*;
use scryer_application::indexer_clients::IndexerClientProtocol;
use std::sync::Arc;

async fn store() -> IndexerClientStore {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(
        "PRAGMA foreign_keys = ON; CREATE TABLE indexers (id TEXT PRIMARY KEY);
        INSERT INTO indexers VALUES ('source-a'), ('source-b');",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0315_indexer_api_clients.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    IndexerClientStore::new(StoreDatastore::sqlite(
        pool,
        Arc::new(tokio::sync::Mutex::new(())),
    ))
}

fn record() -> IndexerClientRecord {
    let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    IndexerClientRecord {
        id: "client-a".into(),
        configuration: IndexerClientConfiguration {
            name: "Test client".into(),
            enabled: true,
            protocols: vec![
                IndexerClientProtocol::Newznab,
                IndexerClientProtocol::Torznab,
            ],
            grants: vec![
                IndexerClientGrant {
                    indexer_id: "source-a".into(),
                    categories: None,
                },
                IndexerClientGrant {
                    indexer_id: "source-b".into(),
                    categories: Some(vec![]),
                },
            ],
        },
        lookup_id: Some("lookup-a".into()),
        secret_hash: Some("verification-material".into()),
        key_generation: 1,
        revision: 1,
        last_used_at: None,
        last_outcome_code: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn grants_round_trip_and_stale_writes_do_not_replace_them() {
    let store = store().await;
    let original = record();
    assert!(store.save(&original, None).await.unwrap());
    assert!(!store.save(&original, None).await.unwrap());
    let stored = store.get_by_lookup_id("lookup-a").await.unwrap().unwrap();
    assert_eq!(stored.configuration, original.configuration);
    assert_eq!(stored.created_at, original.created_at);
    let mut changed = original.clone();
    changed.revision = 2;
    changed.configuration.grants = vec![IndexerClientGrant {
        indexer_id: "source-b".into(),
        categories: Some(vec!["100123".into(), "software".into()]),
    }];
    assert!(store.save(&changed, Some(1)).await.unwrap());
    let mut stale = original;
    stale.revision = 2;
    stale.configuration.grants.clear();
    assert!(!store.save(&stale, Some(1)).await.unwrap());
    let stored = store.list().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].configuration, changed.configuration);
    assert_eq!(stored[0].revision, 2);
}

#[tokio::test]
async fn failed_grant_replacement_rolls_back_key_rotation_and_configuration() {
    let store = store().await;
    let original = record();
    assert!(store.save(&original, None).await.unwrap());
    let mut changed = original.clone();
    changed.revision = 2;
    changed.key_generation = 2;
    changed.lookup_id = Some("rotated-lookup".into());
    changed.configuration.name = "Changed name".into();
    changed.configuration.grants.push(IndexerClientGrant {
        indexer_id: "missing-source".into(),
        categories: None,
    });
    assert!(store.save(&changed, Some(1)).await.is_err());
    assert!(
        store
            .get_by_lookup_id("rotated-lookup")
            .await
            .unwrap()
            .is_none()
    );
    let stored = store.get_by_lookup_id("lookup-a").await.unwrap().unwrap();
    assert_eq!(stored.configuration, original.configuration);
    assert_eq!(stored.revision, 1);
    assert_eq!(stored.key_generation, 1);
}

#[tokio::test]
async fn revoked_keys_and_stale_request_status_cannot_reappear() {
    let store = store().await;
    let mut client = record();
    assert!(store.save(&client, None).await.unwrap());
    let at = client.created_at;
    store
        .record_use(&client.id, 1, at, "PARTIAL")
        .await
        .unwrap();
    client.revision = 2;
    client.key_generation = 2;
    client.lookup_id = None;
    client.secret_hash = None;
    client.configuration.grants.clear();
    assert!(store.save(&client, Some(1)).await.unwrap());
    store
        .record_use(&client.id, 1, at + chrono::Duration::seconds(20), "STALE")
        .await
        .unwrap();
    store
        .record_use(&client.id, 2, at + chrono::Duration::seconds(10), "CURRENT")
        .await
        .unwrap();
    store
        .record_use(&client.id, 2, at + chrono::Duration::seconds(5), "OLDER")
        .await
        .unwrap();
    assert!(store.get_by_lookup_id("lookup-a").await.unwrap().is_none());
    let stored = store.get_by_id(&client.id).await.unwrap().unwrap();
    assert!(stored.configuration.grants.is_empty());
    assert!(stored.secret_hash.is_none());
    assert_eq!(stored.last_outcome_code.as_deref(), Some("CURRENT"));
    assert_eq!(
        stored.last_used_at,
        Some(at + chrono::Duration::seconds(10))
    );
}
