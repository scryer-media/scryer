use super::*;
use chrono::Utc;
use scryer_application::{
    TitleImageBlob, TitleImageKind, TitleImageReplacement, TitleImageRepository,
    TitleImageStorageMode, TitleImageVariantRecord, TitleRepository, UserRepository,
};
use scryer_domain::{
    Collection, Entitlement, Episode, InterstitialMovieMetadata, MediaFacet, Title,
};
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

fn make_test_title(id: &str, poster_url: Option<&str>) -> Title {
    Title {
        id: id.to_string(),
        name: "Poster Test".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2026),
        overview: Some("overview".to_string()),
        poster_url: poster_url.map(str::to_string),
        banner_url: None,
        background_url: None,
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
    }
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
async fn title_queries_prefer_local_cached_poster_url() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let title = make_test_title("title-1", Some("https://tvdb.example/poster.jpg"));
    <SqliteServices as TitleRepository>::create(&services, title.clone())
        .await
        .expect("title should insert");

    let before_cache = <SqliteServices as TitleRepository>::get_by_id(&services, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        before_cache.poster_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );

    <SqliteServices as TitleImageRepository>::replace_title_image(
        &services,
        &title.id,
        TitleImageReplacement {
            kind: TitleImageKind::Poster,
            source_url: "https://tvdb.example/poster.jpg".to_string(),
            source_etag: Some("\"etag-1\"".to_string()),
            source_last_modified: None,
            source_format: "jpeg".to_string(),
            source_width: 1000,
            source_height: 1500,
            storage_mode: TitleImageStorageMode::AvifMaster,
            master_format: "avif".to_string(),
            master_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            master_width: 1000,
            master_height: 1500,
            master_bytes: vec![1, 2, 3],
            variants: vec![TitleImageVariantRecord {
                variant_key: "w500".to_string(),
                format: "avif".to_string(),
                width: 500,
                height: 750,
                bytes: vec![7, 8, 9],
                sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            }],
        },
    )
    .await
    .expect("title image should insert");

    let after_cache = <SqliteServices as TitleRepository>::get_by_id(&services, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        after_cache.poster_url.as_deref(),
        Some("/images/titles/title-1/poster/w500?v=bbbbbbbbbbbbbbbb")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_change_local_version_when_cached_poster_changes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_version_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let title = make_test_title("title-2", Some("https://tvdb.example/poster-a.jpg"));
    <SqliteServices as TitleRepository>::create(&services, title.clone())
        .await
        .expect("title should insert");

    for (source_url, sha) in [
        (
            "https://tvdb.example/poster-a.jpg",
            "11111111111111111111111111111111",
        ),
        (
            "https://tvdb.example/poster-b.jpg",
            "22222222222222222222222222222222",
        ),
    ] {
        <SqliteServices as TitleImageRepository>::replace_title_image(
            &services,
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: source_url.to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: sha.to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![7, 8, 9],
                    sha256: sha.to_string(),
                }],
            },
        )
        .await
        .expect("title image should upsert");

        sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
            .bind(source_url)
            .bind(&title.id)
            .execute(&services.pool)
            .await
            .expect("source url should update");
    }

    let updated = <SqliteServices as TitleRepository>::get_by_id(&services, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-2/poster/w500?v=2222222222222222")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_use_local_original_url_for_original_storage_mode() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_original_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let title = make_test_title("title-3", Some("https://tvdb.example/poster-original.jpg"));
    <SqliteServices as TitleRepository>::create(&services, title.clone())
        .await
        .expect("title should insert");

    <SqliteServices as TitleImageRepository>::replace_title_image(
        &services,
        &title.id,
        TitleImageReplacement {
            kind: TitleImageKind::Poster,
            source_url: "https://tvdb.example/poster-original.jpg".to_string(),
            source_etag: None,
            source_last_modified: None,
            source_format: "jpeg".to_string(),
            source_width: 400,
            source_height: 600,
            storage_mode: TitleImageStorageMode::Original,
            master_format: "jpeg".to_string(),
            master_sha256: "cccccccccccccccccccccccccccccccc".to_string(),
            master_width: 400,
            master_height: 600,
            master_bytes: vec![3, 2, 1],
            variants: Vec::new(),
        },
    )
    .await
    .expect("title image should insert");

    let updated = <SqliteServices as TitleRepository>::get_by_id(&services, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-3/poster/original?v=cccccccccccccccc")
    );

    let original = <SqliteServices as TitleImageRepository>::get_title_image_blob(
        &services,
        &title.id,
        TitleImageKind::Poster,
        "original",
    )
    .await
    .expect("original blob lookup should succeed");
    assert_eq!(
        original,
        Some(TitleImageBlob {
            content_type: "image/jpeg".to_string(),
            etag: "cccccccccccccccccccccccccccccccc".to_string(),
            bytes: vec![3, 2, 1],
        })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_fall_back_to_original_when_w500_variant_is_missing() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_incomplete_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let title = make_test_title(
        "title-4",
        Some("https://tvdb.example/poster-incomplete.jpg"),
    );
    <SqliteServices as TitleRepository>::create(&services, title.clone())
        .await
        .expect("title should insert");

    <SqliteServices as TitleImageRepository>::replace_title_image(
        &services,
        &title.id,
        TitleImageReplacement {
            kind: TitleImageKind::Poster,
            source_url: "https://tvdb.example/poster-incomplete.jpg".to_string(),
            source_etag: None,
            source_last_modified: None,
            source_format: "jpeg".to_string(),
            source_width: 1000,
            source_height: 1500,
            storage_mode: TitleImageStorageMode::AvifMaster,
            master_format: "avif".to_string(),
            master_sha256: "dddddddddddddddddddddddddddddddd".to_string(),
            master_width: 1000,
            master_height: 1500,
            master_bytes: vec![9, 8, 7],
            variants: Vec::new(),
        },
    )
    .await
    .expect("title image should insert");

    let updated = <SqliteServices as TitleRepository>::get_by_id(&services, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-4/poster/original?v=dddddddddddddddd")
    );

    let pending = <SqliteServices as TitleImageRepository>::list_titles_requiring_image_refresh(
        &services,
        TitleImageKind::Poster,
        10,
    )
    .await
    .expect("list pending poster refresh should succeed");
    assert!(
        pending.iter().any(|task| task.title_id == title.id),
        "incomplete AVIF cache rows should be re-queued for repair"
    );

    let _ = std::fs::remove_file(db);
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
        banner_url: None,
        background_url: None,
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
        interstitial_movie: Some(InterstitialMovieMetadata {
            tvdb_id: "12345".into(),
            name: "Test Movie".into(),
            slug: "test-movie".into(),
            year: Some(2024),
            content_status: "released".into(),
            overview: "Interstitial overview".into(),
            poster_url: "https://example.com/poster.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 97,
            sort_title: "Test Movie".into(),
            imdb_id: "tt1234567".into(),
            genres: vec!["Action".into(), "Anime".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2024-01-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("canon".into()),
            movie_form: Some("movie".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB marked special as critical to story".into()),
        }),
        specials_movies: vec![InterstitialMovieMetadata {
            tvdb_id: "67890".into(),
            name: "Recap Movie".into(),
            slug: "recap-movie".into(),
            year: Some(2014),
            content_status: "released".into(),
            overview: "Recap of the first half.".into(),
            poster_url: "https://example.com/recap.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 90,
            sort_title: "Recap Movie".into(),
            imdb_id: "tt7654321".into(),
            genres: vec!["Action".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2014-11-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("unknown".into()),
            movie_form: Some("recap".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB special category marks this as a recap".into()),
        }],
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
    assert_eq!(
        collections[0]
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.as_str()),
        Some("Test Movie")
    );
    let loaded_collection =
        <SqliteServices as scryer_application::ShowRepository>::get_collection_by_id(
            &services,
            &collection.id,
        )
        .await
        .expect("get collection by id")
        .expect("collection should exist");
    assert_eq!(loaded_collection.id, collection.id);
    assert_eq!(
        loaded_collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.imdb_id.as_str()),
        Some("tt1234567")
    );
    assert_eq!(loaded_collection.specials_movies.len(), 1);
    assert_eq!(
        loaded_collection.specials_movies[0].movie_form.as_deref(),
        Some("recap")
    );
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
    let loaded_episode = <SqliteServices as scryer_application::ShowRepository>::get_episode_by_id(
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
