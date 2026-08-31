use super::*;

/// Migration 0163 gave `pending_releases` the four tracker-minimum columns a
/// delayed grab needs, and 0169 added the seeder count promotion re-judges
/// against. Proves both directions through the real migrated schema: a parked
/// row keeps its minimums and its seeders, and a row written without them (the
/// shape every pre-0165 row has) reads back as `None` rather than failing the
/// map.
#[tokio::test]
async fn pending_release_tracker_minimums_round_trip_and_legacy_rows_read_back_as_none() {
    let (services, db) = temp_services("scryer_pending_release_seed_minimums").await;
    let pending_store =
        PendingReleaseStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now().to_rfc3339();

    let mut parked = scryer_application::PendingRelease {
        id: "pending-with-minimums".to_string(),
        wanted_item_id: "wanted-minimums".to_string(),
        title_id: "title-minimums".to_string(),
        release_title: "Tracker.Minimums.2024.1080p-GRP".to_string(),
        release_url: Some("https://example.invalid/minimums.torrent".to_string()),
        source_kind: None,
        release_size_bytes: Some(2_048),
        release_score: 1200,
        scoring_log_json: None,
        indexer_source: Some("private-tracker".to_string()),
        indexer_id: None,
        release_guid: Some("guid-minimums".to_string()),
        added_at: now.clone(),
        last_observed_at: now.clone(),
        delay_until: now.clone(),
        status: scryer_application::PendingReleaseStatus::Waiting,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
        seed_minimums: Default::default(),
        seeders: Some(37),
        release_identity: "guid:private-tracker:guid-minimums".to_string(),
        coverage_identity: "scope:wanted-minimums".to_string(),
        role: scryer_application::PendingReleaseRole::Primary,
        last_decision_code: None,
        release_age_unknown: false,
    };
    parked.seed_minimums = scryer_application::ReleaseSeedMinimums {
        min_seed_ratio: Some(1.5),
        min_seed_time_minutes: Some(4_320),
        season_pack_seed_ratio: Some(2.5),
        season_pack_seed_time_minutes: Some(10_080),
    };
    PendingReleaseRepository::insert_pending_release(&pending_store, &parked)
        .await
        .expect("pending release with minimums should insert");

    let loaded =
        PendingReleaseRepository::get_pending_release(&pending_store, "pending-with-minimums")
            .await
            .expect("pending release should load")
            .expect("pending release should exist");
    assert_eq!(loaded.seed_minimums, parked.seed_minimums);
    assert_eq!(
        loaded.seeders,
        Some(37),
        "promotion re-judges the swarm against the count captured at park time"
    );

    // Written the way every row parked before 0165 was: the four columns absent
    // from the INSERT, so the migration's defaults (NULL) apply.
    sqlx::query(
        "INSERT INTO pending_releases
         (id, wanted_item_id, title_id, release_title, release_url, release_size_bytes,
          source_kind, release_score, scoring_log_json, indexer_source, indexer_id, release_guid,
          added_at, delay_until, status, grabbed_at, source_password, published_at, info_hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("pending-legacy")
    .bind("wanted-legacy")
    .bind("title-legacy")
    .bind("Legacy.Pending.2024.1080p-GRP")
    .bind(None::<String>)
    .bind(None::<i64>)
    .bind(None::<String>)
    .bind(900_i32)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind("guid-legacy")
    .bind(&now)
    .bind(&now)
    .bind("waiting")
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .bind(None::<String>)
    .execute(services.pool())
    .await
    .expect("legacy pending release should insert");

    let legacy = PendingReleaseRepository::get_pending_release(&pending_store, "pending-legacy")
        .await
        .expect("legacy pending release should load")
        .expect("legacy pending release should exist");
    assert_eq!(
        legacy.seed_minimums,
        scryer_application::ReleaseSeedMinimums::default()
    );
    assert_eq!(
        legacy.seeders, None,
        "a row parked before 0169 reads as unknown, which stays eligible"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_database_maintenance_runs_without_command_bus() {
    let (services, db) = temp_services("scryer_sqlite_database_maintenance").await;
    let housekeeping = housekeeping_store(&services);
    let wal_path = std::path::PathBuf::from(format!("{}-wal", db.display()));

    let mut connection = services.pool().acquire().await.expect("acquire sqlite");
    sqlx::query("PRAGMA wal_autocheckpoint = 0")
        .execute(&mut *connection)
        .await
        .expect("disable automatic checkpoint");
    sqlx::query("CREATE TABLE maintenance_wal_probe (value TEXT NOT NULL)")
        .execute(&mut *connection)
        .await
        .expect("create WAL probe table");
    sqlx::query("INSERT INTO maintenance_wal_probe (value) VALUES ('probe')")
        .execute(&mut *connection)
        .await
        .expect("write WAL probe row");
    drop(connection);

    let freelist_pages: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(services.pool())
        .await
        .expect("load freelist count");
    assert!(freelist_pages < 2_000, "test should not trigger VACUUM");
    assert!(
        std::fs::metadata(&wal_path).is_ok_and(|metadata| metadata.len() > 0),
        "probe write should populate the WAL"
    );

    housekeeping
        .run_database_maintenance()
        .await
        .expect("database maintenance should complete");

    assert!(
        std::fs::metadata(&wal_path).is_ok_and(|metadata| metadata.len() == 0),
        "maintenance should truncate the WAL"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_database_maintenance_truncates_wal_after_threshold_vacuum() {
    let (services, db) = temp_services("scryer_sqlite_database_vacuum").await;
    let housekeeping = housekeeping_store(&services);
    let wal_path = std::path::PathBuf::from(format!("{}-wal", db.display()));

    let mut connection = services.pool().acquire().await.expect("acquire sqlite");
    sqlx::query("PRAGMA wal_autocheckpoint = 0")
        .execute(&mut *connection)
        .await
        .expect("disable automatic checkpoint");
    sqlx::query("CREATE TABLE maintenance_vacuum_probe (payload BLOB NOT NULL)")
        .execute(&mut *connection)
        .await
        .expect("create VACUUM probe table");
    sqlx::query("INSERT INTO maintenance_vacuum_probe (payload) VALUES (zeroblob(12582912))")
        .execute(&mut *connection)
        .await
        .expect("allocate VACUUM probe pages");
    sqlx::query("DELETE FROM maintenance_vacuum_probe")
        .execute(&mut *connection)
        .await
        .expect("free VACUUM probe pages");
    drop(connection);

    let freelist_before: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(services.pool())
        .await
        .expect("load freelist count before maintenance");
    let pages_before: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(services.pool())
        .await
        .expect("load page count before maintenance");
    assert!(freelist_before >= 2_000);
    assert!(freelist_before as f64 / pages_before as f64 >= 0.10);

    housekeeping
        .run_database_maintenance()
        .await
        .expect("database maintenance should vacuum and checkpoint");

    let pages_after: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(services.pool())
        .await
        .expect("load page count after maintenance");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(services.pool())
        .await
        .expect("run integrity check");
    assert!(
        pages_after < pages_before,
        "VACUUM should reclaim free pages"
    );
    assert_eq!(integrity, "ok");
    assert!(
        std::fs::metadata(&wal_path).is_ok_and(|metadata| metadata.len() == 0),
        "maintenance should truncate the post-VACUUM WAL"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn external_subtitle_probe_cache_round_trips_replace_and_delete() {
    let (services, db) = temp_services("scryer_external_subtitle_probe_cache").await;
    let catalog = title_store(&services);
    let media_files = media_file_store(&services);
    let subtitles = subtitle_download_store(&services);

    let title = make_test_title("title-probe-cache", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let media_file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Example.Movie.mkv".to_string(),
            size_bytes: 4_096,
            ..Default::default()
        })
        .await
        .expect("media file should insert");

    let initial = ExternalSubtitleProbeCacheEntry {
        media_file_id: media_file_id.clone(),
        file_path: "/tmp/Example.Movie.srt".to_string(),
        size_bytes: 512,
        modified_at: Some("2026-04-29T00:00:00Z".to_string()),
        language: None,
        hearing_impaired: None,
        detection_source_language: ExternalSubtitleDetectionSource::Unknown,
        detection_source_hi: ExternalSubtitleDetectionSource::Unknown,
        probe_version: 2,
        updated_at: "2026-04-29T00:00:01Z".to_string(),
    };

    subtitles
        .upsert_probe_cache_entry(&initial)
        .await
        .expect("initial probe cache row should insert");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("probe cache rows should list");
    assert_eq!(listed, vec![initial.clone()]);

    let replaced = ExternalSubtitleProbeCacheEntry {
        language: Some("eng".to_string()),
        hearing_impaired: Some(true),
        detection_source_language: ExternalSubtitleDetectionSource::Content,
        detection_source_hi: ExternalSubtitleDetectionSource::Content,
        updated_at: "2026-04-29T00:00:02Z".to_string(),
        ..initial
    };

    subtitles
        .upsert_probe_cache_entry(&replaced)
        .await
        .expect("probe cache row should replace");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("replaced probe cache row should list");
    assert_eq!(listed, vec![replaced.clone()]);

    subtitles
        .delete_probe_cache_entry(&media_file_id, "/tmp/Example.Movie.srt")
        .await
        .expect("probe cache row should delete");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("probe cache rows should list after delete");
    assert!(listed.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn scoped_anibridge_external_ids_round_trip_for_collections_and_episodes() {
    let (services, db) = temp_services("scryer_scoped_anibridge_ids").await;
    let catalog = title_store(&services);
    let shows = show_store(&services);

    let mut title = make_test_title("title-anime", None);
    title.facet = MediaFacet::Anime;
    title.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    title.external_ids = vec![ExternalId {
        source: "tvdb_id".to_string(),
        value: "431162".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let collection = Collection {
        id: "season-2".to_string(),
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "2".to_string(),
        label: Some("Season 2".to_string()),
        ordered_path: None,
        narrative_order: Some("2".to_string()),
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("24".to_string()),
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&shows, collection.clone())
        .await
        .expect("collection should insert");

    let episode = Episode {
        id: "episode-s02e23".to_string(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("23".to_string()),
        season_number: Some("2".to_string()),
        episode_label: Some("S02E23".to_string()),
        title: Some("Episode 23".to_string()),
        air_date: Some("2025-06-13".to_string()),
        duration_seconds: Some(1_440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: Some("47".to_string()),
        overview: None,
        tvdb_id: Some("1234567".to_string()),
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_episode(&shows, episode.clone())
        .await
        .expect("episode should insert");

    ShowRepository::replace_anibridge_scoped_external_ids_for_title(
        &shows,
        &title.id,
        vec![ScopedExternalId {
            scope_id: collection.id.clone(),
            source: "anilist".to_string(),
            external_id: "176301".to_string(),
            provenance: "anibridge".to_string(),
            source_scope: Some("R".to_string()),
        }],
        vec![ScopedExternalId {
            scope_id: episode.id.clone(),
            source: "anidb".to_string(),
            external_id: "18562".to_string(),
            provenance: "anibridge".to_string(),
            source_scope: Some("R".to_string()),
        }],
    )
    .await
    .expect("replace scoped ids should succeed");

    let collection_ids = ShowRepository::list_collection_external_ids(&shows, &collection.id)
        .await
        .expect("collection ids should load");
    assert_eq!(collection_ids.len(), 1);
    assert_eq!(collection_ids[0].scope_id, collection.id);
    assert_eq!(collection_ids[0].source, "anilist");
    assert_eq!(collection_ids[0].external_id, "176301");
    assert_eq!(collection_ids[0].source_scope.as_deref(), Some("R"));

    let episode_ids = ShowRepository::list_episode_external_ids(&shows, &episode.id)
        .await
        .expect("episode ids should load");
    assert_eq!(episode_ids.len(), 1);
    assert_eq!(episode_ids[0].scope_id, episode.id);
    assert_eq!(episode_ids[0].source, "anidb");
    assert_eq!(episode_ids[0].external_id, "18562");
    assert_eq!(episode_ids[0].source_scope.as_deref(), Some("R"));

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            extra_external_ids: vec![ExternalId {
                source: "anidb".to_string(),
                value: "18562".to_string(),
            }],
            ..TitleMetadataUpdate::default()
        },
    )
    .await
    .expect("hydrated title metadata should persist title-level AniDB");

    let hydrated_title = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("hydrated title should load")
        .expect("hydrated title should exist");
    assert!(
        hydrated_title
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "anidb" && external_id.value == "18562" })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn release_metadata_enum_canonicalization_migration_normalizes_legacy_values() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE media_files (
            id TEXT PRIMARY KEY,
            source_type TEXT,
            video_codec TEXT,
            video_codec_parsed TEXT,
            audio_codec_parsed TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("media_files fixture table should be created");

    for statement in [
        "CREATE TABLE quality_profile_source_allowlist (
            profile_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, source)
        )",
        "CREATE TABLE quality_profile_source_blocklist (
            profile_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, source)
        )",
        "CREATE TABLE quality_profile_audio_codec_allowlist (
            profile_id TEXT NOT NULL,
            codec TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, codec)
        )",
        "CREATE TABLE quality_profile_audio_codec_blocklist (
            profile_id TEXT NOT NULL,
            codec TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, codec)
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("quality profile fixture table should be created");
    }

    sqlx::query(
        "INSERT INTO media_files
            (id, source_type, video_codec, video_codec_parsed, audio_codec_parsed)
         VALUES
            ('known', 'webdl', 'x264', 'HEVC', 'DTS-HD MA'),
            ('unknown', 'mystery-source', 'mystery-video', 'mystery-parsed', 'mystery-audio')",
    )
    .execute(&pool)
    .await
    .expect("media file fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_source_allowlist(profile_id, source, created_at)
         VALUES
            ('p1', 'webdl', '2026-01-01T00:00:00Z'),
            ('p1', 'WEB-DL', '2026-01-02T00:00:00Z'),
            ('p1', 'not-a-source', '2026-01-03T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("source allowlist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_source_blocklist(profile_id, source, created_at)
         VALUES
            ('p1', 'bdmv', '2026-01-01T00:00:00Z'),
            ('p1', 'BRDISK', '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("source blocklist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_audio_codec_allowlist(profile_id, codec, created_at)
         VALUES
            ('p1', 'DTS-HD MA', '2026-01-01T00:00:00Z'),
            ('p1', 'DTSMA', '2026-01-02T00:00:00Z'),
            ('p1', 'not-a-codec', '2026-01-03T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("audio allowlist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_audio_codec_blocklist(profile_id, codec, created_at)
         VALUES
            ('p1', 'DD+', '2026-01-01T00:00:00Z'),
            ('p1', 'DDP', '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("audio blocklist fixture rows should be inserted");

    run_embedded_migration(
        &pool,
        rolled_up_migration_section(
            include_str!(
                "../../../scryer/src/db/migrations/0125_0_16_release_rollup_pre_notification_target_hook.sql"
            ),
            "migrations/0125_release_metadata_enum_canonicalization.sql",
        ),
    )
    .await;

    let known_media: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT source_type, video_codec, video_codec_parsed, audio_codec_parsed
               FROM media_files
              WHERE id = 'known'",
    )
    .fetch_one(&pool)
    .await
    .expect("known media row should remain");
    assert_eq!(
        known_media,
        (
            Some("WEB-DL".to_string()),
            Some("H.264".to_string()),
            Some("H.265".to_string()),
            Some("DTSMA".to_string())
        )
    );

    let unknown_media: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT source_type, video_codec, video_codec_parsed, audio_codec_parsed
               FROM media_files
              WHERE id = 'unknown'",
    )
    .fetch_one(&pool)
    .await
    .expect("unknown media row should remain");
    assert_eq!(
        unknown_media,
        (
            Some("mystery-source".to_string()),
            Some("mystery-video".to_string()),
            Some("mystery-parsed".to_string()),
            Some("mystery-audio".to_string())
        )
    );

    let source_allowlist: Vec<String> =
        sqlx::query_scalar("SELECT source FROM quality_profile_source_allowlist ORDER BY source")
            .fetch_all(&pool)
            .await
            .expect("source allowlist should query");
    assert_eq!(source_allowlist, vec!["WEB-DL".to_string()]);

    let source_blocklist: Vec<String> =
        sqlx::query_scalar("SELECT source FROM quality_profile_source_blocklist ORDER BY source")
            .fetch_all(&pool)
            .await
            .expect("source blocklist should query");
    assert_eq!(source_blocklist, vec!["BRDISK".to_string()]);

    let audio_allowlist: Vec<String> = sqlx::query_scalar(
        "SELECT codec FROM quality_profile_audio_codec_allowlist ORDER BY codec",
    )
    .fetch_all(&pool)
    .await
    .expect("audio allowlist should query");
    assert_eq!(audio_allowlist, vec!["DTSMA".to_string()]);

    let audio_blocklist: Vec<String> = sqlx::query_scalar(
        "SELECT codec FROM quality_profile_audio_codec_blocklist ORDER BY codec",
    )
    .fetch_all(&pool)
    .await
    .expect("audio blocklist should query");
    assert_eq!(audio_blocklist, vec!["DDP".to_string()]);
}

#[test]
fn embedded_migration_bundle_includes_external_import_monitor_snapshot_chunk_table() {
    let keys = crate::migrations::list_embedded_migration_keys();
    assert!(
        keys.iter()
            .any(|key| key == "0117_external_import_monitor_snapshot_chunks"),
        "embedded migration bundle is missing 0117_external_import_monitor_snapshot_chunks: {keys:?}"
    );
    assert!(
        keys.iter().any(|key| key == "0140_0.17_release_rollup"),
        "embedded migration bundle is missing 0140_0.17_release_rollup: {keys:?}"
    );
}

#[tokio::test]
async fn additional_managed_file_role_migration_defaults_existing_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE media_files (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            file_path TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            quality_label TEXT,
            scan_status TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy media_files should be created");
    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            submitted_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy download_submissions should be created");
    sqlx::query(
        "INSERT INTO media_files
         (id, title_id, file_path, size_bytes, scan_status, created_at)
         VALUES ('file-1', 'title-1', '/library/Movie.mkv', 1024, 'scanned', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("legacy media file should insert");
    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_type, download_client_item_id, submitted_at)
         VALUES ('submission-1', 'title-1', 'movie', 'nzbget', 'job-1', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("legacy submission should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0129_additional_managed_file_roles.sql"),
    )
    .await;

    let role: String = sqlx::query_scalar("SELECT role FROM media_files WHERE id = 'file-1'")
        .fetch_one(&pool)
        .await
        .expect("media file role should load");
    assert_eq!(role, "primary");

    let purpose: String =
        sqlx::query_scalar("SELECT purpose FROM download_submissions WHERE id = 'submission-1'")
            .fetch_one(&pool)
            .await
            .expect("download submission purpose should load");
    assert_eq!(purpose, "standard");
}

#[tokio::test]
async fn review_regression_download_client_identity_migration_deduplicates_legacy_submissions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            source_title TEXT,
            submitted_at TEXT NOT NULL,
            collection_id TEXT,
            tracked_state TEXT,
            tracked_state_at TEXT,
            source_hint TEXT,
            source_kind TEXT,
            request_signature TEXT,
            episode_id TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy download_submissions should be created");
    sqlx::query(
        "CREATE TABLE download_queue_commands (
            id TEXT PRIMARY KEY,
            action TEXT NOT NULL,
            client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            is_history INTEGER NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy download_queue_commands should be created");

    for (id, submitted_at) in [
        ("old-submission", "2025-01-01T00:00:00Z"),
        ("new-submission", "2025-01-02T00:00:00Z"),
    ] {
        sqlx::query(
            "INSERT INTO download_submissions
             (id, title_id, facet, download_client_type, download_client_item_id, submitted_at)
             VALUES (?, 'title-1', 'series', 'sabnzbd', 'native-id-1', ?)",
        )
        .bind(id)
        .bind(submitted_at)
        .execute(&pool)
        .await
        .expect("legacy submission should insert");
    }

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0087_download_queue_client_identity.sql"),
    )
    .await;

    let kept_id: String = sqlx::query_scalar("SELECT id FROM download_submissions")
        .fetch_one(&pool)
        .await
        .expect("migrated submission should exist");
    assert_eq!(kept_id, "new-submission");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions")
        .fetch_one(&pool)
        .await
        .expect("migrated submission count should load");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn review_regression_release_name_blocklist_watershed_resets_legacy_failed_state() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE blocklist (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            source_title TEXT,
            source_hint TEXT,
            quality TEXT,
            download_id TEXT,
            reason TEXT,
            data_json TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("blocklist table should be created");
    sqlx::query(
        "CREATE TABLE release_download_attempts (
            id TEXT PRIMARY KEY,
            title_id TEXT,
            source_hint TEXT,
            source_title TEXT,
            outcome TEXT NOT NULL,
            error_message TEXT,
            source_password TEXT,
            attempted_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("release attempts table should be created");
    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            download_client_id TEXT NOT NULL DEFAULT '',
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            source_title TEXT,
            submitted_at TEXT NOT NULL,
            collection_id TEXT,
            tracked_state TEXT,
            tracked_state_at TEXT,
            source_hint TEXT,
            source_kind TEXT,
            request_signature TEXT,
            episode_id TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("download submissions table should be created");

    sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, source_title, created_at)
         VALUES ('block-1', 'title-1', 'pals.s05.720p.bluray.dd5.1.x264-ntb', '2025-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("blocklist row should insert");
    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, attempted_at)
         VALUES
         ('failed-1', 'title-1', 'weaver-1', 'pals.s05.720p.bluray.dd5.1.x264-ntb', 'failed', '2025-01-01T00:00:00Z'),
         ('success-1', 'title-1', 'weaver-1', 'pals.s05.720p.bluray.dd5.1.x264-ntb', 'success', '2025-01-01T01:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("release attempts should insert");
    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_title, submitted_at, tracked_state, tracked_state_at, source_hint, request_signature)
         VALUES
         ('stub-failed', '', '', 'primary', 'weaver', 'job-1', NULL, '2025-01-01T00:00:00Z', 'failed', '2025-01-01T00:05:00Z', NULL, NULL),
         ('rich-failed', 'title-1', 'series', 'primary', 'weaver', 'job-2', 'Pals.S05.720p.BluRay.DD5.1.x264-NTb', '2025-01-01T00:00:00Z', 'failed', '2025-01-01T00:05:00Z', 'weaver://job-2', 'sig-2')",
    )
    .execute(&pool)
    .await
    .expect("download submissions should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0102_release_name_blocklist_watershed.sql"),
    )
    .await;

    let blocklist_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocklist")
        .fetch_one(&pool)
        .await
        .expect("blocklist count should load");
    assert_eq!(blocklist_count, 0);

    let failed_attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM release_download_attempts WHERE outcome = 'failed'",
    )
    .fetch_one(&pool)
    .await
    .expect("failed attempt count should load");
    assert_eq!(failed_attempt_count, 0);

    let successful_attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM release_download_attempts WHERE outcome = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("successful attempt count should load");
    assert_eq!(successful_attempt_count, 1);

    let blank_stub_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions WHERE id = 'stub-failed'")
            .fetch_one(&pool)
            .await
            .expect("blank stub count should load");
    assert_eq!(blank_stub_count, 0);

    let rich_failed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions WHERE id = 'rich-failed'")
            .fetch_one(&pool)
            .await
            .expect("rich failed submission count should load");
    assert_eq!(rich_failed_count, 1);
}

#[tokio::test]
async fn review_regression_download_submission_episode_links_cascade_with_parent_records() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys should enable");
    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            download_client_id TEXT NOT NULL DEFAULT '',
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            UNIQUE(download_client_id, download_client_type, download_client_item_id)
        )",
    )
    .execute(&pool)
    .await
    .expect("download_submissions should be created");
    sqlx::query("CREATE TABLE episodes (id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("episodes should be created");

    run_embedded_migration(
        &pool,
        include_str!(
            "../../../scryer/src/db/migrations/0089_download_submission_episode_links.sql"
        ),
    )
    .await;

    sqlx::query(
        "INSERT INTO download_submissions
         (id, download_client_id, download_client_type, download_client_item_id)
         VALUES ('submission-1', 'client-1', 'sabnzbd', 'native-id-1')",
    )
    .execute(&pool)
    .await
    .expect("submission should insert");
    sqlx::query("INSERT INTO episodes (id) VALUES ('episode-1')")
        .execute(&pool)
        .await
        .expect("episode should insert");
    sqlx::query(
        "INSERT INTO download_submission_episode_links
         (download_client_id, download_client_type, download_client_item_id, episode_id)
         VALUES ('client-1', 'sabnzbd', 'native-id-1', 'episode-1')",
    )
    .execute(&pool)
    .await
    .expect("episode link should insert");

    sqlx::query("DELETE FROM download_submissions WHERE id = 'submission-1'")
        .execute(&pool)
        .await
        .expect("submission should delete");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submission_episode_links")
        .fetch_one(&pool)
        .await
        .expect("link count should load");
    assert_eq!(count, 0);

    sqlx::query(
        "INSERT INTO download_submissions
         (id, download_client_id, download_client_type, download_client_item_id)
         VALUES ('submission-2', 'client-1', 'sabnzbd', 'native-id-1')",
    )
    .execute(&pool)
    .await
    .expect("submission should reinsert");
    sqlx::query(
        "INSERT INTO download_submission_episode_links
         (download_client_id, download_client_type, download_client_item_id, episode_id)
         VALUES ('client-1', 'sabnzbd', 'native-id-1', 'episode-1')",
    )
    .execute(&pool)
    .await
    .expect("episode link should reinsert");
    sqlx::query("DELETE FROM episodes WHERE id = 'episode-1'")
        .execute(&pool)
        .await
        .expect("episode should delete");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submission_episode_links")
        .fetch_one(&pool)
        .await
        .expect("link count should load");
    // Episode deletion does not cascade — the link table is a submission-time
    // audit record and outlives episode catalog churn. Cascade applies only
    // to the download_submissions parent.
    assert_eq!(
        count, 1,
        "link survives episode deletion: episode_id has no FK cascade"
    );
}

#[tokio::test]
async fn review_regression_subtitle_provider_update_sets_and_clears_disabled_until() {
    let (services, db) = single_connection_services("scryer_subtitle_disabled_until").await;
    let store =
        SubtitleProviderConfigStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();
    let config = SubtitleProviderConfig {
        id: "subtitle-provider-1".to_string(),
        name: "Subtitles".to_string(),
        provider_type: "mock".to_string(),
        config_json: "{}".to_string(),
        enabled_facets: vec!["movie".to_string()],
        is_enabled: true,
        last_health_status: None,
        last_error: None,
        last_error_at: None,
        disabled_until: None,
        created_at: now,
        updated_at: now,
    };
    SubtitleProviderConfigRepository::create(&store, config)
        .await
        .expect("subtitle provider should be created");

    let disabled_until = chrono::DateTime::parse_from_rfc3339("2030-01-02T03:04:05Z")
        .expect("fixed timestamp should parse")
        .with_timezone(&Utc);
    let updated = SubtitleProviderConfigRepository::update(
        &store,
        SubtitleProviderConfigUpdate {
            id: "subtitle-provider-1".to_string(),
            disabled_until: Some(Some(disabled_until)),
            ..Default::default()
        },
    )
    .await
    .expect("subtitle provider disabled_until should update");
    assert_eq!(updated.disabled_until, Some(disabled_until));

    let updated = SubtitleProviderConfigRepository::update(
        &store,
        SubtitleProviderConfigUpdate {
            id: "subtitle-provider-1".to_string(),
            disabled_until: Some(None),
            ..Default::default()
        },
    )
    .await
    .expect("subtitle provider disabled_until should clear");
    assert_eq!(updated.disabled_until, None);

    let _ = std::fs::remove_file(db);
}
