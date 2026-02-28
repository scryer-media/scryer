use super::*;
use chrono::Utc;
use scryer_application::UserRepository;
use scryer_domain::{Collection, Entitlement, Episode, MediaFacet, Title};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_can_initialize() {
    let db = std::env::temp_dir().join(format!(
        "scryer_store_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let users = services
        .list_all()
        .await
        .expect("query should return users after initialization");

    assert!(!users.is_empty());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn nzbget_client_is_sendable() {
    let client = NzbgetDownloadClient::new(
        "http://127.0.0.1:6789".to_string(),
        Some("user".into()),
        Some("pass".into()),
        "SCORE".to_string(),
    );
    // We only validate that it can be built and is callable in type system.
    let _ = client.endpoint();
}

#[tokio::test]
async fn migration_validate_mode_rejects_pending_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    assert!(
        result.is_err(),
        "validate mode should reject unapplied migrations"
    );
    let err = match result {
        Ok(_) => panic!("validate mode should reject unapplied migrations"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("pending migration"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_rejects_unknown_or_newer_schema_history() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_compat_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let too_new_key = "999999_too_new";
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(Vec::<u8>::new())
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("tamper first migration checksum");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
        (version, description, installed_on, success, checksum, execution_time)
        VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(999999i64)
    .bind(too_new_key)
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await
    .expect("insert new migration");

    let result = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::Apply).await;
    assert!(result.is_err());
    let err = match result {
        Ok(_) => panic!("bad migration history should fail compatibility check"),
        Err(err) => err,
    };

    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(message.contains("migrations newer than supported"));
    assert!(message.contains("Please update scryer"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_apply_then_validate_is_idempotent() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_then_apply_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    drop(services);

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("applied DB should pass validate mode");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn unique_constraints_enforce_settings_and_user_entitlements() {
    let db = std::env::temp_dir().join(format!(
        "scryer_unique_constraints_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO settings_definitions
        (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sd-settings")
    .bind("app")
    .bind("global")
    .bind("theme")
    .bind("string")
    .bind("{}")
    .bind(0)
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert settings definition");

    sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-1")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert first settings value");

    let duplicate_setting_value = sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-2")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await;
    assert!(duplicate_setting_value.is_err());

    sqlx::query("INSERT INTO users (id, username, entitlements) VALUES (?, ?, ?)")
        .bind("user-1")
        .bind("constraint_user")
        .bind("[]")
        .execute(&pool)
        .await
        .expect("insert user");

    sqlx::query("INSERT INTO entitlements (code, description, category) VALUES (?, ?, ?)")
        .bind("ent.code.manage")
        .bind("Manage")
        .bind("admin")
        .execute(&pool)
        .await
        .expect("insert entitlement");

    sqlx::query(
        "INSERT INTO user_entitlements (user_id, entitlement_code, granted_by_user_id, granted_at, expires_at)
        VALUES (?, ?, ?, ?, ?)",
    )
    .bind("user-1")
    .bind("ent.code.manage")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(Option::<String>::None)
    .execute(&pool)
    .await
    .expect("insert first user entitlement");

    let duplicate_user_entitlement = sqlx::query(
        "INSERT INTO user_entitlements (user_id, entitlement_code, granted_by_user_id, granted_at, expires_at)
        VALUES (?, ?, ?, ?, ?)",
    )
    .bind("user-1")
    .bind("ent.code.manage")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(Option::<String>::None)
    .execute(&pool)
    .await;
    assert!(duplicate_user_entitlement.is_err());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn user_crud_queries_work() {
    let db = std::env::temp_dir().join(format!(
        "scryer_user_queries_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let created = <SqliteServices as UserRepository>::create(
        &services,
        scryer_domain::User {
            id: "u-1".to_string(),
            username: "editor".to_string(),
            entitlements: vec![Entitlement::ViewCatalog],
            password_hash: None,
        },
    )
    .await
    .expect("create user");

    let from_db = <SqliteServices as UserRepository>::get_by_id(&services, &created.id)
        .await
        .expect("query by id")
        .expect("id should exist");
    assert_eq!(from_db.username, created.username);

    let updated = <SqliteServices as UserRepository>::update_entitlements(
        &services,
        &created.id,
        vec![Entitlement::ManageTitle, Entitlement::ViewHistory],
    )
    .await
    .expect("update entitlements");
    assert!(updated.entitlements.contains(&Entitlement::ManageTitle));

    <SqliteServices as UserRepository>::delete(&services, &created.id)
        .await
        .expect("delete user");
    let missing = <SqliteServices as UserRepository>::get_by_id(&services, &created.id)
        .await
        .expect("query after delete");
    assert!(missing.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_show_queries_roundtrip() {
    let db = std::env::temp_dir().join(format!(
        "scryer_show_roundtrip_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();

    let title = Title {
        id: "title-show-1".into(),
        name: "Sample Show".into(),
        facet: MediaFacet::Tv,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
    };
    <SqliteServices as scryer_application::TitleRepository>::create(&services, title.clone())
        .await
        .expect("insert title");

    let collection = Collection {
        id: "collection-show-1".into(),
        title_id: title.id.clone(),
        collection_type: "season".into(),
        collection_index: "1".into(),
        label: Some("Season One".into()),
        ordered_path: None,
        narrative_order: Some("1".into()),
        first_episode_number: Some("1".into()),
        last_episode_number: Some("12".into()),
        monitored: true,
        created_at: Utc::now(),
    };
    <SqliteServices as scryer_application::ShowRepository>::create_collection(
        &services,
        collection.clone(),
    )
    .await
    .expect("insert collection");

    let episode = Episode {
        id: "episode-show-1".into(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: "episode".into(),
        episode_number: Some("1".into()),
        season_number: Some("1".into()),
        episode_label: Some("Pilot".into()),
        title: Some("Pilot".into()),
        air_date: None,
        duration_seconds: Some(1000),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: Some("The pilot episode.".into()),
        monitored: true,
        created_at: Utc::now(),
    };
    <SqliteServices as scryer_application::ShowRepository>::create_episode(
        &services,
        episode.clone(),
    )
    .await
    .expect("insert episode");

    let collections =
        <SqliteServices as scryer_application::ShowRepository>::list_collections_for_title(
            &services, &title.id,
        )
        .await
        .expect("list collections");
    let episodes =
        <SqliteServices as scryer_application::ShowRepository>::list_episodes_for_collection(
            &services,
            &collection.id,
        )
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    let loaded_collection =
        <SqliteServices as scryer_application::ShowRepository>::get_collection_by_id(
            &services,
            &collection.id,
        )
        .await
        .expect("get collection by id")
        .expect("collection should exist");
    assert_eq!(loaded_collection.id, collection.id);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
    let loaded_episode =
        <SqliteServices as scryer_application::ShowRepository>::get_episode_by_id(
            &services,
            &episode.id,
        )
        .await
        .expect("get episode by id")
        .expect("episode should exist");
    assert_eq!(loaded_episode.id, episode.id);

    let updated_collection =
        <SqliteServices as scryer_application::ShowRepository>::update_collection(
            &services,
            &collection.id,
            Some("arc".into()),
            Some("1.1".into()),
            Some("Arc One".into()),
            Some("arc/season".into()),
            None,
            Some("12".into()),
            None,
        )
        .await
        .expect("update collection");
    assert_eq!(updated_collection.collection_type, "arc");
    assert_eq!(updated_collection.collection_index, "1.1");
    assert_eq!(updated_collection.label, Some("Arc One".into()));
    assert_eq!(updated_collection.ordered_path, Some("arc/season".into()));
    assert_eq!(updated_collection.last_episode_number, Some("12".into()));

    let updated_episode = <SqliteServices as scryer_application::ShowRepository>::update_episode(
        &services,
        &episode.id,
        Some("special".into()),
        Some("E1".into()),
        Some("2".into()),
        Some("Special".into()),
        Some("Pilot Special".into()),
        Some("2026-01-01".into()),
        Some(2_400),
        Some(true),
        Some(false),
        None,
        Some(collection.id.clone()),
    )
    .await
    .expect("update episode");
    assert_eq!(updated_episode.episode_type, "special");
    assert_eq!(updated_episode.episode_number, Some("E1".into()));
    assert_eq!(updated_episode.season_number, Some("2".into()));
    assert_eq!(updated_episode.episode_label, Some("Special".into()));
    assert_eq!(updated_episode.title, Some("Pilot Special".into()));
    assert_eq!(updated_episode.air_date, Some("2026-01-01".into()));
    assert_eq!(updated_episode.duration_seconds, Some(2_400));
    assert!(updated_episode.has_multi_audio);
    assert!(!updated_episode.has_subtitle);

    <SqliteServices as scryer_application::ShowRepository>::delete_episode(&services, &episode.id)
        .await
        .expect("delete episode");
    let episodes_after_delete =
        <SqliteServices as scryer_application::ShowRepository>::list_episodes_for_collection(
            &services,
            &collection.id,
        )
        .await
        .expect("list episodes after delete");
    assert!(episodes_after_delete.is_empty());
    let missing_episode =
        <SqliteServices as scryer_application::ShowRepository>::get_episode_by_id(
            &services,
            &episode.id,
        )
        .await
        .expect("get episode by id after delete");
    assert!(missing_episode.is_none());

    <SqliteServices as scryer_application::ShowRepository>::delete_collection(
        &services,
        &collection.id,
    )
    .await
    .expect("delete collection");
    let collections_after_delete =
        <SqliteServices as scryer_application::ShowRepository>::list_collections_for_title(
            &services, &title.id,
        )
        .await
        .expect("list collections after delete");
    assert!(collections_after_delete.is_empty());
    let missing_collection =
        <SqliteServices as scryer_application::ShowRepository>::get_collection_by_id(
            &services,
            &collection.id,
        )
        .await
        .expect("get collection by id after delete");
    assert!(missing_collection.is_none());

    let _ = std::fs::remove_file(db);
}
