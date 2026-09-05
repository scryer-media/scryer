use super::*;

#[tokio::test]
async fn queued_delete_stale_recovery_only_recovers_stale_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_delete_recovery_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadQueueCommandStore::new(services.datastore());

    let stale = workflow_store
        .queue_delete_command(None, "nzbget", "job-stale", false, Some("admin"))
        .await
        .expect("stale delete should queue");
    let fresh = workflow_store
        .queue_delete_command(None, "nzbget", "job-fresh", true, Some("admin"))
        .await
        .expect("fresh delete should queue");

    workflow_store
        .mark_delete_command_running(&stale.id)
        .await
        .expect("stale delete should mark running");
    workflow_store
        .mark_delete_command_running(&fresh.id)
        .await
        .expect("fresh delete should mark running");

    let stale_updated_at = (Utc::now() - chrono::Duration::seconds(300)).to_rfc3339();
    sqlx::query("UPDATE download_queue_commands SET updated_at = ? WHERE id = ?")
        .bind(&stale_updated_at)
        .bind(&stale.id)
        .execute(&services.pool)
        .await
        .expect("age stale running delete");

    let recovered = workflow_store
        .recover_stale_running_delete_commands(120)
        .await
        .expect("stale recovery should succeed");
    assert_eq!(recovered, 1);

    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, started_at
         FROM download_queue_commands
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind(&fresh.id)
    .bind(&stale.id)
    .fetch_all(&services.pool)
    .await
    .expect("load delete rows after stale recovery");

    assert_eq!(rows.len(), 2);
    let fresh_row = rows
        .iter()
        .find(|row| row.0 == fresh.id)
        .expect("fresh row should exist");
    assert_eq!(fresh_row.1, "running");
    assert!(
        fresh_row.2.is_some(),
        "fresh running delete should remain running"
    );
    let stale_row = rows
        .iter()
        .find(|row| row.0 == stale.id)
        .expect("stale row should exist");
    assert_eq!(stale_row, &(stale.id, "queued".to_string(), None));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn unique_constraints_enforce_settings_and_user_permission_masks() {
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

    sqlx::query("INSERT INTO users (id, username) VALUES (?, ?)")
        .bind("user-1")
        .bind("constraint_user")
        .execute(&pool)
        .await
        .expect("insert user");

    sqlx::query(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
        VALUES (?, ?, ?)",
    )
    .bind("user-1")
    .bind(1_i64)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert first app permission mask");

    let duplicate_app_permission_mask = sqlx::query(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
        VALUES (?, ?, ?)",
    )
    .bind("user-1")
    .bind(1_i64)
    .bind(&now)
    .execute(&pool)
    .await;
    assert!(duplicate_app_permission_mask.is_err());

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
    let users = user_store(&services);

    let created = UserRepository::create(
        &users,
        scryer_domain::User {
            id: "u-1".to_string(),
            username: "editor".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        },
    )
    .await
    .expect("create user");

    let from_db = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("query by id")
        .expect("id should exist");
    assert_eq!(from_db.username, created.username);
    assert_eq!(
        from_db.login_status(),
        scryer_domain::UserLoginStatus::Enabled
    );

    let updated = UserRepository::update_password_and_invalidate_sessions(
        &users,
        &created.id,
        "hashed-password".to_string(),
        false,
        "session-1",
    )
    .await
    .expect("update password hash");
    assert_eq!(updated.password_hash.as_deref(), Some("hashed-password"));
    assert_eq!(
        UserRepository::auth_session_version(&users, &created.id)
            .await
            .expect("load password update session version")
            .as_deref(),
        Some("session-1")
    );

    let disabled = UserRepository::update_login_status_and_rotate_session(
        &users,
        &created.id,
        scryer_domain::UserLoginStatus::Disabled,
        "session-2",
    )
    .await
    .expect("disable user login");
    assert_eq!(
        disabled.login_status(),
        scryer_domain::UserLoginStatus::Disabled
    );
    assert_eq!(disabled.password_hash.as_deref(), Some("hashed-password"));
    assert_eq!(
        UserRepository::auth_session_version(&users, &created.id)
            .await
            .expect("load session version")
            .as_deref(),
        Some("session-2")
    );

    UserRepository::delete(&users, &created.id)
        .await
        .expect("delete user");
    let missing = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("query after delete");
    assert!(missing.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn conditional_own_password_updates_allow_one_initial_claim_and_reject_stale_hashes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_conditional_password_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let users = user_store(&services);
    let created = UserRepository::create(
        &users,
        scryer_domain::User {
            id: "conditional-password-user".to_string(),
            username: "conditional-password-user".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: Default::default(),
            authorization: Default::default(),
        },
    )
    .await
    .expect("create passwordless user");

    let first_claim = UserRepository::update_own_password_and_invalidate_sessions(
        &users,
        &created.id,
        "first-hash".to_string(),
        false,
        "first-session",
        None,
    );
    let second_claim = UserRepository::update_own_password_and_invalidate_sessions(
        &users,
        &created.id,
        "second-hash".to_string(),
        false,
        "second-session",
        None,
    );
    let (first_claim, second_claim) = tokio::join!(first_claim, second_claim);
    assert_eq!(
        usize::from(first_claim.is_ok()) + usize::from(second_claim.is_ok()),
        1,
        "exactly one initial password claim must win"
    );
    for result in [&first_claim, &second_claim] {
        if let Err(error) = result {
            assert!(matches!(error, AppError::ReauthenticationRequired(_)));
        }
    }

    let winner = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("load initial password winner")
        .expect("user should remain present");
    let winner_hash = winner.password_hash.expect("winning password hash");
    let winner_session = UserRepository::auth_session_version(&users, &created.id)
        .await
        .expect("load winning authentication epoch")
        .expect("winning authentication epoch");

    let stale = UserRepository::update_own_password_and_invalidate_sessions(
        &users,
        &created.id,
        "stale-hash".to_string(),
        false,
        "stale-session",
        Some("superseded-hash"),
    )
    .await;
    assert!(matches!(stale, Err(AppError::ReauthenticationRequired(_))));
    let after_stale = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("load user after stale write")
        .expect("user should remain present");
    assert_eq!(
        after_stale.password_hash.as_deref(),
        Some(winner_hash.as_str())
    );
    assert_eq!(
        UserRepository::auth_session_version(&users, &created.id)
            .await
            .expect("load authentication epoch after stale write")
            .as_deref(),
        Some(winner_session.as_str())
    );

    let final_user = UserRepository::update_own_password_and_invalidate_sessions(
        &users,
        &created.id,
        "final-hash".to_string(),
        false,
        "final-session",
        Some(&winner_hash),
    )
    .await
    .expect("matching stored hash should permit replacement");
    assert_eq!(final_user.password_hash.as_deref(), Some("final-hash"));
    assert_eq!(
        UserRepository::auth_session_version(&users, &created.id)
            .await
            .expect("load final authentication epoch")
            .as_deref(),
        Some("final-session")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_show_queries_roundtrip() {
    let db = std::env::temp_dir().join(format!(
        "scryer_show_roundtrip_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let catalog = title_store(&services);
    let shows = show_store(&services);

    let title = Title {
        id: "title-show-1".into(),
        name: "Sample Show".into(),
        facet: MediaFacet::Series,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("insert title");

    let collection = Collection {
        id: "collection-show-1".into(),
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".into(),
        label: Some("Season One".into()),
        ordered_path: None,
        narrative_order: Some("1".into()),
        first_episode_number: Some("1".into()),
        last_episode_number: Some("12".into()),
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&shows, collection.clone())
        .await
        .expect("insert collection");
    let movie_link = ShowRepository::upsert_series_movie_link(
        &shows,
        scryer_domain::SeriesMovieLink {
            id: "series-movie-link-1".into(),
            series_title_id: title.id.clone(),
            movie: scryer_domain::MovieEntity {
                id: "movie-entity-1".into(),
                title: "Test Movie".into(),
                sort_title: Some("Test Movie".into()),
                slug: Some("test-movie".into()),
                year: Some(2024),
                overview: Some("Series movie overview".into()),
                poster_url: Some("https://example.com/poster.jpg".into()),
                background_url: None,
                language: Some("eng".into()),
                runtime_minutes: Some(97),
                content_status: Some("released".into()),
                studio: Some("Studio Test".into()),
                digital_release_date: Some("2024-01-01".into()),
                imdb_id: Some("tt1234567".into()),
                tvdb_id: Some("12345".into()),
                tmdb_id: Some("99001".into()),
                mal_id: Some("5001".into()),
                anidb_id: None,
                ratings: None,
                credits: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            placement: Some("ordered".into()),
            narrative_order: Some("1.5".into()),
            after_season: Some(1),
            before_season: None,
            linked_episode_id: None,
            association_confidence: Some("high".into()),
            continuity_status: Some("canon".into()),
            movie_form: Some("movie".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB marked special as critical to story".into()),
            source: Some("test".into()),
            monitoring_override: None,
            metadata_active: true,
            monitored: true,
            legacy_collection_id: None,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await
    .expect("insert series movie link");
    ShowRepository::upsert_series_movie_link(
        &shows,
        scryer_domain::SeriesMovieLink {
            id: "series-movie-link-2".into(),
            series_title_id: title.id.clone(),
            movie: scryer_domain::MovieEntity {
                id: "movie-entity-2".into(),
                title: "Recap Movie".into(),
                sort_title: Some("Recap Movie".into()),
                slug: Some("recap-movie".into()),
                year: Some(2014),
                overview: Some("Recap of the first half.".into()),
                poster_url: Some("https://example.com/recap.jpg".into()),
                background_url: None,
                language: Some("eng".into()),
                runtime_minutes: Some(90),
                content_status: Some("released".into()),
                studio: Some("Studio Test".into()),
                digital_release_date: Some("2014-11-01".into()),
                imdb_id: Some("tt7654321".into()),
                tvdb_id: Some("67890".into()),
                tmdb_id: None,
                mal_id: None,
                anidb_id: None,
                ratings: None,
                credits: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            placement: Some("specials".into()),
            narrative_order: Some("0.1".into()),
            after_season: Some(0),
            before_season: None,
            linked_episode_id: None,
            association_confidence: Some("high".into()),
            continuity_status: Some("unknown".into()),
            movie_form: Some("recap".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB special category marks this as a recap".into()),
            source: Some("test".into()),
            monitoring_override: None,
            metadata_active: true,
            monitored: true,
            legacy_collection_id: None,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await
    .expect("insert recap series movie link");

    let episode = Episode {
        id: "episode-show-1".into(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
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
        tvdb_id: None,
        image_url: Some("https://cdn.example.test/episode-created.jpg".into()),
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_episode(&shows, episode.clone())
        .await
        .expect("insert episode");

    let collections = ShowRepository::list_collections_for_title(&shows, &title.id)
        .await
        .expect("list collections");
    let episodes = ShowRepository::list_episodes_for_collection(&shows, &collection.id)
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    let loaded_collection = ShowRepository::get_collection_by_id(&shows, &collection.id)
        .await
        .expect("get collection by id")
        .expect("collection should exist");
    assert_eq!(loaded_collection.id, collection.id);
    let series_movie_links = ShowRepository::list_series_movie_links_for_title(&shows, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(series_movie_links.len(), 2);
    assert!(series_movie_links.iter().any(|link| {
        link.id == movie_link.id
            && link.movie.imdb_id.as_deref() == Some("tt1234567")
            && link.continuity_status.as_deref() == Some("canon")
    }));
    assert_eq!(
        series_movie_links
            .iter()
            .find(|link| link.movie.title == "Recap Movie")
            .and_then(|link| link.movie_form.as_deref()),
        Some("recap")
    );
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
    let loaded_episode = ShowRepository::get_episode_by_id(&shows, &episode.id)
        .await
        .expect("get episode by id")
        .expect("episode should exist");
    assert_eq!(loaded_episode.id, episode.id);
    assert_eq!(
        loaded_episode.image_url,
        Some("https://cdn.example.test/episode-created.jpg".into())
    );

    let updated_collection = ShowRepository::update_collection(
        &shows,
        &collection.id,
        CollectionUpdate {
            collection_type: Some(CollectionType::Arc),
            collection_index: Some("1.1".into()),
            label: Some("Arc One".into()),
            ordered_path: Some("arc/season".into()),
            last_episode_number: Some("12".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update collection");
    assert_eq!(updated_collection.collection_type, CollectionType::Arc);
    assert_eq!(updated_collection.collection_index, "1.1");
    assert_eq!(updated_collection.label, Some("Arc One".into()));
    assert_eq!(updated_collection.ordered_path, Some("arc/season".into()));
    assert_eq!(updated_collection.last_episode_number, Some("12".into()));

    let updated_episode = ShowRepository::update_episode(
        &shows,
        &episode.id,
        EpisodeUpdate {
            episode_type: Some(scryer_domain::EpisodeType::Special),
            episode_number: Some("E1".into()),
            season_number: Some("2".into()),
            episode_label: Some("Special".into()),
            title: Some("Pilot Special".into()),
            air_date: Some("2026-01-01".into()),
            duration_seconds: Some(2_400),
            has_multi_audio: Some(true),
            has_subtitle: Some(false),
            collection_id: Some(collection.id.clone()),
            overview: Some("Updated overview".into()),
            tvdb_id: Some("349232".into()),
            image_url: Some("https://cdn.example.test/episode-updated.jpg".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update episode");
    assert_eq!(
        updated_episode.episode_type,
        scryer_domain::EpisodeType::Special
    );
    assert_eq!(updated_episode.episode_number, Some("E1".into()));
    assert_eq!(updated_episode.season_number, Some("2".into()));
    assert_eq!(updated_episode.episode_label, Some("Special".into()));
    assert_eq!(updated_episode.title, Some("Pilot Special".into()));
    assert_eq!(updated_episode.air_date, Some("2026-01-01".into()));
    assert_eq!(updated_episode.duration_seconds, Some(2_400));
    assert!(updated_episode.has_multi_audio);
    assert!(!updated_episode.has_subtitle);
    assert_eq!(
        updated_episode.image_url,
        Some("https://cdn.example.test/episode-updated.jpg".into())
    );

    let cleared_episode = ShowRepository::update_episode(
        &shows,
        &episode.id,
        EpisodeUpdate {
            clear_image_url: true,
            ..Default::default()
        },
    )
    .await
    .expect("clear episode image url");
    assert_eq!(cleared_episode.image_url, None);

    ShowRepository::delete_episode(&shows, &episode.id)
        .await
        .expect("delete episode");
    let episodes_after_delete =
        ShowRepository::list_episodes_for_collection(&shows, &collection.id)
            .await
            .expect("list episodes after delete");
    assert!(episodes_after_delete.is_empty());
    let missing_episode = ShowRepository::get_episode_by_id(&shows, &episode.id)
        .await
        .expect("get episode by id after delete");
    assert!(missing_episode.is_none());

    ShowRepository::delete_collection(&shows, &collection.id)
        .await
        .expect("delete collection");
    let collections_after_delete = ShowRepository::list_collections_for_title(&shows, &title.id)
        .await
        .expect("list collections after delete");
    assert!(collections_after_delete.is_empty());
    let missing_collection = ShowRepository::get_collection_by_id(&shows, &collection.id)
        .await
        .expect("get collection by id after delete");
    assert!(missing_collection.is_none());

    let _ = std::fs::remove_file(db);
}
