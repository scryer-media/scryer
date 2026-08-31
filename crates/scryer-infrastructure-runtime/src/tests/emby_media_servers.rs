use std::sync::{Arc, RwLock};

use chrono::Utc;
use scryer_application::MediaServerConnectionRepository;
use scryer_domain::{
    AppPermission, AppPermissionMask, LibraryPermission, LibraryPermissionMask,
    MediaServerConnection, MediaServerDefaultLibraryGrant, MediaServerPathMapping,
    MediaServerProvider,
};
use sqlx::Row;

use super::*;
use crate::encryption::EncryptionKey;
use crate::media_server_connection_store::MediaServerConnectionStore;

fn emby_connection() -> MediaServerConnection {
    let now = Utc::now();
    MediaServerConnection {
        id: "emby-round-trip".to_string(),
        provider: MediaServerProvider::Emby,
        display_name: "Emby Round Trip".to_string(),
        base_url: "https://emby.example.test/reverse-proxy/emby".to_string(),
        external_url: None,
        enabled: true,
        login_enabled: true,
        linking_enabled: true,
        auto_add_enabled: true,
        default_app_permissions: AppPermissionMask::from_permissions([
            AppPermission::ManageCatalogSettings,
        ]),
        default_library_grants: vec![MediaServerDefaultLibraryGrant {
            library_id: "library-1".to_string(),
            permissions: LibraryPermissionMask::from_permissions([LibraryPermission::View]),
        }],
        machine_id: None,
        api_key: Some("emby-static-api-key".to_string()),
        emby_server_id: Some("emby-system-id".to_string()),
        emby_connect_enabled: true,
        path_mappings: vec![MediaServerPathMapping {
            source_path: "/downloads".to_string(),
            destination_path: "/media".to_string(),
            sort_order: 0,
        }],
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn sqlite_emby_details_round_trip_encrypted_key_identity_toggle_grants_and_mappings() {
    let (services, db) = temp_services("scryer_emby_media_server_round_trip").await;
    let encryption_key = EncryptionKey::from_bytes([0x5a; 32]);
    *services
        .encryption_key_state()
        .write()
        .expect("test encryption-key lock") = Some(encryption_key);
    let store =
        MediaServerConnectionStore::new(services.datastore(), services.encryption_key_state());
    let expected = emby_connection();
    sqlx::query(
        "INSERT INTO libraries
            (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ('library-1', 'movie', 'Emby Movies', 'emby-movies', 0, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
    )
    .execute(services.pool())
    .await
    .expect("insert library required by default grant");

    store
        .create(expected.clone())
        .await
        .expect("create Emby connection");
    let raw = sqlx::query(
        "SELECT api_key, server_id, connect_enabled
           FROM emby_media_server_details
          WHERE connection_id = ?",
    )
    .bind(&expected.id)
    .fetch_one(services.pool())
    .await
    .expect("load raw Emby details");
    let stored_key: String = raw.try_get("api_key").expect("stored API key");
    assert_ne!(stored_key, "emby-static-api-key");
    assert!(!stored_key.contains("emby-static-api-key"));
    assert_eq!(
        raw.try_get::<String, _>("server_id").unwrap(),
        "emby-system-id"
    );
    assert!(raw.try_get::<bool, _>("connect_enabled").unwrap());

    let loaded = store
        .get_by_id(&expected.id)
        .await
        .expect("load Emby connection")
        .expect("Emby connection exists");
    assert_eq!(loaded, expected);

    let mut updated = loaded;
    updated.api_key = Some("rotated-emby-static-key".to_string());
    updated.emby_server_id = Some("rotated-emby-system-id".to_string());
    updated.emby_connect_enabled = false;
    updated.updated_at = Utc::now();
    store
        .update(updated.clone())
        .await
        .expect("rotate Emby credentials");
    assert_eq!(
        store
            .get_by_id(&updated.id)
            .await
            .expect("reload rotated Emby connection"),
        Some(updated.clone())
    );

    assert!(
        store
            .compare_and_set_emby_base_url(
                &updated.id,
                &updated.base_url,
                "rotated-emby-system-id",
                "https://fresh-emby.example.test/emby",
            )
            .await
            .expect("compare-and-set Emby address")
    );
    assert_eq!(
        store
            .get_by_id(&updated.id)
            .await
            .expect("load refreshed Emby connection")
            .expect("refreshed Emby connection")
            .base_url,
        "https://fresh-emby.example.test/emby"
    );

    services.pool().close().await;
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_emby_detail_defaults_preserve_legacy_null_identity_and_disabled_connect() {
    let (services, db) = temp_services("scryer_emby_media_server_legacy_defaults").await;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO media_server_connections (
             id, provider, display_name, base_url, enabled, login_enabled,
             linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
         ) VALUES (?, 'emby', 'Legacy Emby', 'https://legacy-emby.example.test', 1, 0, 0, 0, 0, ?, ?)",
    )
    .bind("legacy-emby")
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("insert legacy connection");
    sqlx::query(
        "INSERT INTO emby_media_server_details (connection_id, api_key, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind("legacy-emby")
    .bind("legacy-api-key")
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("new Emby columns must retain migration defaults");

    let store = MediaServerConnectionStore::new(services.datastore(), Arc::new(RwLock::new(None)));
    let loaded = store
        .get_by_id("legacy-emby")
        .await
        .expect("load legacy Emby row")
        .expect("legacy Emby row exists");
    assert_eq!(loaded.api_key.as_deref(), Some("legacy-api-key"));
    assert_eq!(loaded.emby_server_id, None);
    assert!(!loaded.emby_connect_enabled);

    services.pool().close().await;
    let _ = std::fs::remove_file(db);
}
