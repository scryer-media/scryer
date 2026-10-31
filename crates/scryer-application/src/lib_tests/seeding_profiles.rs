use super::*;

fn new_seeding_profile(name: &str) -> NewSeedingProfile {
    NewSeedingProfile {
        name: name.to_string(),
        ratio: Some(1.5),
        seed_time_minutes: Some(4320),
        season_pack_mode: scryer_domain::SeasonPackSeedMode::Inherit,
        season_pack_ratio: None,
        season_pack_seed_time_minutes: None,
        honor_tracker_minimums: true,
        goal_met_action: scryer_domain::SeedGoalMetAction::RemoveEntry,
        never_remove: false,
        minimum_seeders: None,
        post_import_tracking: scryer_domain::PostImportTracking::Park,
    }
}

async fn bootstrap() -> (AppUseCase, User) {
    bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(MockIndexerClient),
        vec![
            synthetic_direct_nab_indexer_config("idx-usenet", "nzbgeek"),
            synthetic_direct_nab_indexer_config("idx-torrent", "torrent_rss"),
            synthetic_direct_nab_indexer_config("idx-unknown", "generic"),
        ],
    )
}

#[tokio::test]
async fn seeding_profile_create_validation_rejects_bad_goals() {
    let (app, admin) = bootstrap().await;

    let mut blank = new_seeding_profile("   ");
    blank.ratio = None;
    let error = app
        .create_seeding_profile(&admin, blank)
        .await
        .expect_err("blank names are rejected");
    assert!(
        matches!(error, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("name is required"))
    );

    let mut zero_ratio = new_seeding_profile("Zero ratio");
    zero_ratio.ratio = Some(0.0);
    let error = app
        .create_seeding_profile(&admin, zero_ratio)
        .await
        .expect_err("non-positive ratios are rejected");
    assert!(
        matches!(error, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("greater than 0")),
    );

    let mut zero_time = new_seeding_profile("Zero time");
    zero_time.seed_time_minutes = Some(0);
    let error = app
        .create_seeding_profile(&admin, zero_time)
        .await
        .expect_err("non-positive seed times are rejected");
    assert!(
        matches!(error, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("greater than 0"))
    );

    // Inherit mode normalizes stray season-pack goals instead of rejecting them.
    let mut stray = new_seeding_profile("Stray season pack");
    stray.season_pack_ratio = Some(2.0);
    stray.season_pack_seed_time_minutes = Some(60);
    let created = app
        .create_seeding_profile(&admin, stray)
        .await
        .expect("inherit mode normalizes season pack goals");
    assert_eq!(created.season_pack_ratio, None);
    assert_eq!(created.season_pack_seed_time_minutes, None);
}

#[tokio::test]
async fn indexer_seeding_profile_assignment_requires_torrent_support() {
    let (app, admin) = bootstrap().await;
    let profile = app
        .create_seeding_profile(&admin, new_seeding_profile("Private tracker"))
        .await
        .expect("profile should be created");

    let unknown_profile = app
        .set_indexer_seeding_profile(&admin, "idx-torrent", Some("missing-profile"))
        .await
        .expect_err("unknown profiles are rejected");
    assert!(
        matches!(unknown_profile, AppError::NotFound(message) | AppError::NotFoundReason { message, .. } if message.contains("missing-profile"))
    );

    let usenet = app
        .set_indexer_seeding_profile(&admin, "idx-usenet", Some(&profile.id))
        .await
        .expect_err("Usenet indexers cannot carry seeding profiles");
    assert!(
        matches!(usenet, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("does not support torrents")),
    );

    let unknown_protocol = app
        .set_indexer_seeding_profile(&admin, "idx-unknown", Some(&profile.id))
        .await
        .expect_err("indexers without a declared protocol are rejected");
    assert!(
        matches!(unknown_protocol, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("does not declare")),
    );

    let assigned = app
        .set_indexer_seeding_profile(&admin, "idx-torrent", Some(&format!("  {}  ", profile.id)))
        .await
        .expect("torrent indexers accept seeding profiles");
    assert_eq!(
        assigned.seeding_profile_id.as_deref(),
        Some(profile.id.as_str())
    );
    assert_eq!(
        app.indexer_seeding_profile_id("idx-torrent")
            .await
            .expect("assignment should read back")
            .as_deref(),
        Some(profile.id.as_str())
    );

    let cleared = app
        .set_indexer_seeding_profile(&admin, "idx-torrent", Some("   "))
        .await
        .expect("blank ids clear the assignment");
    assert_eq!(cleared.seeding_profile_id, None);
}

#[tokio::test]
async fn referenced_seeding_profiles_cannot_be_deleted() {
    let (app, admin) = bootstrap().await;
    let profile = app
        .create_seeding_profile(&admin, new_seeding_profile("Private tracker"))
        .await
        .expect("profile should be created");

    app.set_indexer_seeding_profile(&admin, "idx-torrent", Some(&profile.id))
        .await
        .expect("assignment should succeed");
    let blocked = app
        .delete_seeding_profile(&admin, &profile.id)
        .await
        .expect_err("indexer assignment blocks deletion");
    assert!(
        matches!(blocked, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("indexer"))
    );

    app.set_indexer_seeding_profile(&admin, "idx-torrent", None)
        .await
        .expect("assignment should clear");
    app.update_download_client_routing(
        &admin,
        "movie",
        vec![DownloadClientRoutingSettingsEntry {
            client_id: "client-1".to_string(),
            enabled: true,
            category: None,
            recent_queue_priority: None,
            older_queue_priority: None,
            remove_completed: true,
            remove_failed: false,
            seeding_profile_id: Some(profile.id.clone()),
        }],
    )
    .await
    .expect("routing should save");
    let blocked = app
        .delete_seeding_profile(&admin, &profile.id)
        .await
        .expect_err("routing assignment blocks deletion");
    assert!(
        matches!(blocked, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("routing entry"))
    );
    assert_eq!(
        app.routing_seeding_profile_id("movie", "client-1")
            .await
            .expect("routing assignment should read back")
            .as_deref(),
        Some(profile.id.as_str())
    );

    app.update_download_client_routing(&admin, "movie", Vec::new())
        .await
        .expect("routing should clear");
    app.set_default_seeding_profile(&admin, Some(&profile.id))
        .await
        .expect("global default should save");
    let blocked = app
        .delete_seeding_profile(&admin, &profile.id)
        .await
        .expect_err("global default blocks deletion");
    assert!(
        matches!(blocked, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("global default"))
    );

    app.set_default_seeding_profile(&admin, None)
        .await
        .expect("global default should clear");
    app.delete_seeding_profile(&admin, &profile.id)
        .await
        .expect("unreferenced profiles delete");
    assert!(
        app.list_seeding_profiles(&admin)
            .await
            .expect("list should load")
            .is_empty()
    );
}

#[tokio::test]
async fn seeding_profile_update_clears_and_preserves_goals() {
    let (app, admin) = bootstrap().await;
    let profile = app
        .create_seeding_profile(&admin, new_seeding_profile("Private tracker"))
        .await
        .expect("profile should be created");

    let unchanged = app
        .update_seeding_profile(
            &admin,
            SeedingProfileUpdate {
                id: profile.id.clone(),
                name: Some("Renamed".to_string()),
                ..SeedingProfileUpdate::default()
            },
        )
        .await
        .expect("rename should succeed");
    assert_eq!(unchanged.name, "Renamed");
    assert_eq!(unchanged.ratio, Some(1.5));
    assert_eq!(unchanged.seed_time_minutes, Some(4320));

    let cleared = app
        .update_seeding_profile(
            &admin,
            SeedingProfileUpdate {
                id: profile.id.clone(),
                ratio: Some(None),
                season_pack_mode: Some(scryer_domain::SeasonPackSeedMode::Override),
                season_pack_ratio: Some(Some(3.0)),
                never_remove: Some(true),
                ..SeedingProfileUpdate::default()
            },
        )
        .await
        .expect("clearing goals should succeed");
    assert_eq!(cleared.ratio, None);
    assert_eq!(cleared.seed_time_minutes, Some(4320));
    assert_eq!(cleared.season_pack_ratio, Some(3.0));
    assert!(cleared.never_remove);
    assert_eq!(cleared.effective_ratio(true), Some(3.0));
    assert_eq!(cleared.effective_ratio(false), None);

    let empty = app
        .update_seeding_profile(
            &admin,
            SeedingProfileUpdate {
                id: profile.id.clone(),
                ..SeedingProfileUpdate::default()
            },
        )
        .await
        .expect_err("empty patches are rejected");
    assert!(
        matches!(empty, AppError::Validation(message) | AppError::ValidationRefused { message, .. } | AppError::ValidationRefusedWithCount { message, .. } if message.contains("at least one"))
    );
}
