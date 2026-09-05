use super::*;

/// A movie library with two roots plus a title parked on the first one, which
/// is what every FR-077 direct-root-write case needs before it can ask for a
/// second root.
async fn bootstrap_two_root_movie_library(
    name: &str,
) -> (AppUseCase, User, Arc<MockMediaFileRepo>, Library, Title) {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files.clone(),
    );
    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            name.to_string(),
            vec![
                LibraryRootDraft {
                    path: format!("/Volumes/Media/{name}/RootA"),
                    is_default: true,
                },
                LibraryRootDraft {
                    path: format!("/Volumes/Media/{name}/RootB"),
                    is_default: false,
                },
            ],
            None,
        )
        .await
        .expect("two-root movie library should be created");
    let title = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: format!("{name} Title"),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
            library.id.clone(),
        )
        .await
        .expect("title should be created")
        .title;
    (app, user, media_files, library, title)
}

fn other_root_id(library: &Library, current_root_folder_id: &str) -> String {
    library
        .roots
        .iter()
        .find(|root| root.id != current_root_folder_id)
        .expect("library should have a second root")
        .id
        .clone()
}

/// FR-077/SC-009: a title with content on disk cannot have its root rewritten
/// through the options path; the refusal is typed and names the move workflow.
#[tokio::test]
async fn direct_root_write_is_refused_for_a_title_with_tracked_files() {
    let (app, user, media_files, library, title) =
        bootstrap_two_root_movie_library("RootWriteRefused").await;
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/Volumes/Media/RootWriteRefused/RootA/Tracked.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert tracked file");
    let destination_root_id = other_root_id(&library, &title.root_folder_id);

    let error = app
        .update_title_metadata_with_root_folder_id(
            &user,
            &title.id,
            None,
            None,
            None,
            Some(Some(destination_root_id)),
        )
        .await
        .expect_err("a tracked-file title must refuse a direct root write");

    assert!(
        matches!(
            &error,
            AppError::DirectRootWriteRetired { message, title_id }
                if title_id == &title.id
                    && message.contains("locationOperationPreview")
                    && message.contains("startLocationOperation")
        ),
        "unexpected error: {error:?}"
    );
    let stored = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(stored.root_folder_id, title.root_folder_id);
}

/// Re-submitting the root the title already sits on is not a move, so clients
/// that echo the whole options object back on every save keep working.
#[tokio::test]
async fn resubmitting_the_current_root_on_a_tracked_title_is_not_a_move() {
    let (app, user, media_files, _library, title) =
        bootstrap_two_root_movie_library("RootWriteNoop").await;
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/Volumes/Media/RootWriteNoop/RootA/Tracked.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert tracked file");

    let updated = app
        .update_title_metadata_with_root_folder_id(
            &user,
            &title.id,
            Some("Renamed While Root Unchanged".into()),
            None,
            None,
            Some(Some(title.root_folder_id.clone())),
        )
        .await
        .expect("a no-op root write must still apply the rest of the patch");

    assert_eq!(updated.name, "Renamed While Root Unchanged");
    assert_eq!(updated.root_folder_id, title.root_folder_id);
}

/// FR-076/FR-077: a fileless title's root is a catalog pointer, so the direct
/// path stays open for it.
#[tokio::test]
async fn fileless_title_keeps_the_direct_root_write_path() {
    let (app, user, _media_files, library, title) =
        bootstrap_two_root_movie_library("RootWriteFileless").await;
    let destination_root_id = other_root_id(&library, &title.root_folder_id);

    let updated = app
        .update_title_metadata_with_root_folder_id(
            &user,
            &title.id,
            None,
            None,
            None,
            Some(Some(destination_root_id.clone())),
        )
        .await
        .expect("a fileless title may still be reassigned directly");

    assert_eq!(updated.root_folder_id, destination_root_id);
}

/// FR-077 keeps root selection at creation a direct assignment; the refusal is
/// the update path's alone.
#[tokio::test]
async fn creating_a_title_still_assigns_the_requested_root_directly() {
    let (app, user, _media_files, library, title) =
        bootstrap_two_root_movie_library("RootWriteCreate").await;
    let destination_root_id = other_root_id(&library, &title.root_folder_id);

    let created = app
        .add_title_with_options_patch_outcome_in_library(
            &user,
            // What the GraphQL layer builds from `TitleOptionsInput.rootFolderId`
            // on the creation path: the resolved root rides on the request and
            // the patch carries it too.
            NewTitle {
                name: "Created On The Second Root".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                root_folder_id: Some(destination_root_id.clone()),
                ..Default::default()
            },
            library.id.clone(),
            TitleOptionsPatch {
                root_folder_id: Some(Some(destination_root_id.clone())),
                ..Default::default()
            },
        )
        .await
        .expect("creation may assign a root directly");

    assert_eq!(created.title.root_folder_id, destination_root_id);
    assert_ne!(created.title.id, title.id);
}

#[tokio::test]
async fn update_title_metadata_changes_name_and_tags() {
    let (app, user) = bootstrap();
    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Original".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["SciFi".into()],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    // The whole-bag write is registry-gated now, so the vocabulary has to exist
    // before a title can be given it.
    for label in ["action", "drama"] {
        app.create_title_tag_definition(&user, label, None)
            .await
            .expect("tag should be defined");
    }

    let updated = app
        .update_title_metadata(
            &user,
            &created.id,
            Some("Updated Name".into()),
            None,
            Some(vec!["Action".into(), "Drama".into(), "Action".into()]),
        )
        .await
        .expect("update title metadata");

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(
        updated.tags,
        vec!["action".to_string(), "drama".to_string()]
    );
    let events = title_updated_events(&app, &created.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
    assert!(matches!(
        &events[0].payload,
        DomainEventPayload::TitleUpdated(_)
    ));
}

#[tokio::test]
async fn fix_title_match_conflicts_are_scoped_to_the_title_library_and_facet() {
    let (app, user) = bootstrap();
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let second_library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Rematch Movies B".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/RematchMoviesB".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("second movie library should be created");
    let target = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Rematch Target".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "111001".to_string(),
                }],
                year: Some(2020),
                overview: Some("metadata that should reset".to_string()),
                ..Default::default()
            },
            default_library_id.clone(),
        )
        .await
        .expect("rematch target should be created")
        .title;
    let other_library_copy = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Other Library Identity".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "222002".to_string(),
                }],
                ..Default::default()
            },
            second_library.id.clone(),
        )
        .await
        .expect("other library copy should be created")
        .title;
    let same_library_conflict = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Same Library Conflict".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "333003".to_string(),
                }],
                ..Default::default()
            },
            default_library_id,
        )
        .await
        .expect("same-library conflict fixture should be created")
        .title;

    let cross_library_result = app
        .fix_title_match(&user, &target.id, Some("222002"), None)
        .await
        .expect("an identity in another library must not block rematch");
    assert!(!cross_library_result.hydrated);
    assert!(!cross_library_result.warnings.is_empty());
    let reset_target = app
        .services
        .catalog
        .titles
        .get_by_id(&target.id)
        .await
        .expect("rematch target should load")
        .expect("rematch target should exist");
    let untouched_copy = app
        .services
        .catalog
        .titles
        .get_by_id(&other_library_copy.id)
        .await
        .expect("other library copy should load")
        .expect("other library copy should exist");
    assert!(
        reset_target
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tvdb" && external_id.value == "222002" })
    );
    assert!(
        untouched_copy
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tvdb" && external_id.value == "222002" })
    );

    let same_library_error = app
        .fix_title_match(&user, &target.id, Some("333003"), None)
        .await
        .expect_err("same-library duplicate must be rejected");
    assert!(matches!(
        same_library_error,
        AppError::Validation(message)
            if message.contains("already assigned")
                && message.contains(&same_library_conflict.name)
    ));
}

#[tokio::test]
async fn set_primary_movie_file_promotes_selected_and_demotes_same_folder_files() {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Primary Switch".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, "/movies/Primary Switch (2026)")
        .await
        .expect("set folder path");

    let old_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch (2026)/Primary Switch 1080p.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert old primary");
    let new_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch (2026)/Primary Switch 2160p.mkv".into(),
            size_bytes: 2_000,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert additional file");
    let out_of_folder_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch Copy/Primary Switch 720p.mkv".into(),
            size_bytes: 500,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert polluted out-of-folder file");

    app.set_primary_movie_file(&user, &title.id, &new_primary_id)
        .await
        .expect("promote primary movie file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list files");
    let role_for = |file_id: &str| {
        files
            .iter()
            .find(|file| file.id == file_id)
            .map(|file| file.role)
            .expect("file role")
    };
    assert_eq!(role_for(&new_primary_id), MediaFileRole::Primary);
    assert_eq!(role_for(&old_primary_id), MediaFileRole::Additional);
    assert_eq!(role_for(&out_of_folder_id), MediaFileRole::Primary);

    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_primary_movie_file_scopes_series_movie_promotion_to_linked_files() {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Movie Primary Switch".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Series Movie Primary Switch: The Movie",
            Some(2026),
            None,
            None,
        ))
        .await
        .expect("create series movie link");

    let old_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Specials/primary.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert old primary");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&old_primary_id, &link.id)
        .await
        .expect("link old primary");
    let new_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Specials/additional.mkv".into(),
            size_bytes: 2_000,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert additional file");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&new_primary_id, &link.id)
        .await
        .expect("link additional file");
    let unrelated_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Season 01/episode.mkv".into(),
            size_bytes: 500,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert unrelated primary");

    app.set_primary_movie_file(&user, &title.id, &new_primary_id)
        .await
        .expect("promote series movie primary file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list files");
    let role_for = |file_id: &str| {
        files
            .iter()
            .find(|file| file.id == file_id)
            .map(|file| file.role)
            .expect("file role")
    };
    assert_eq!(role_for(&new_primary_id), MediaFileRole::Primary);
    assert_eq!(role_for(&old_primary_id), MediaFileRole::Additional);
    assert_eq!(role_for(&unrelated_primary_id), MediaFileRole::Primary);
}

#[tokio::test]
async fn set_primary_movie_file_scopes_episode_promotion_to_linked_files() {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Episode Primary Switch".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    let old_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/series/Episode Primary Switch/Season 01/primary.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert old primary");
    app.services
        .library
        .media_files
        .link_file_to_episode(&old_primary_id, "episode-1")
        .await
        .expect("link old primary to episode");
    let new_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/series/Episode Primary Switch/Season 01/additional.mkv".into(),
            size_bytes: 2_000,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert additional file");
    app.services
        .library
        .media_files
        .link_file_to_episode(&new_primary_id, "episode-1")
        .await
        .expect("link additional file to episode");
    let unrelated_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/series/Episode Primary Switch/Season 01/unrelated.mkv".into(),
            size_bytes: 500,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert unrelated primary");
    app.services
        .library
        .media_files
        .link_file_to_episode(&unrelated_primary_id, "episode-2")
        .await
        .expect("link unrelated primary to episode");

    app.set_primary_movie_file(&user, &title.id, &new_primary_id)
        .await
        .expect("promote episode primary file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list files");
    let role_for = |file_id: &str| {
        files
            .iter()
            .find(|file| file.id == file_id)
            .map(|file| file.role)
            .expect("file role")
    };
    assert_eq!(role_for(&new_primary_id), MediaFileRole::Primary);
    assert_eq!(role_for(&old_primary_id), MediaFileRole::Additional);
    assert_eq!(role_for(&unrelated_primary_id), MediaFileRole::Primary);
}

#[tokio::test]
async fn set_title_monitored_emits_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Monitor Fixture".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let updated = app
        .set_title_monitored(&user, &title.id, false)
        .await
        .expect("update monitored");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_collection_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, collection, _) =
        create_series_with_collection_and_episode(&app, &user, "Collection Monitor Fixture").await;

    let updated = app
        .set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("update collection monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_episode_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, _, episode) =
        create_series_with_collection_and_episode(&app, &user, "Episode Monitor Fixture").await;

    let updated = app
        .set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("update episode monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn external_import_monitor_snapshots_are_scoped_and_retryable_per_library() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let second_library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Snapshot Movies B".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/SnapshotMoviesB".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("second movie library should be created");

    let first = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Snapshot Identity A".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tmdb".to_string(),
                    value: "998731".to_string(),
                }],
                ..Default::default()
            },
            default_library_id.clone(),
        )
        .await
        .expect("default-library title should be created")
        .title;
    let second = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Snapshot Identity B".to_string(),
                facet: MediaFacet::Movie,
                monitored: false,
                external_ids: vec![ExternalId {
                    source: "tmdb".to_string(),
                    value: "998731".to_string(),
                }],
                ..Default::default()
            },
            second_library.id.clone(),
        )
        .await
        .expect("second-library identity copy should be created")
        .title;

    append_movie_monitor_snapshot_chunk_for_library(
        &app,
        &user,
        &default_library_id,
        vec![ExternalImportMonitorMovieEntry {
            tmdb_id: Some("998731".to_string()),
            imdb_id: None,
            path: None,
            monitored: false,
        }],
    )
    .await;
    append_movie_monitor_snapshot_chunk_for_library(
        &app,
        &user,
        &second_library.id,
        vec![ExternalImportMonitorMovieEntry {
            tmdb_id: Some("998731".to_string()),
            imdb_id: None,
            path: None,
            monitored: true,
        }],
    )
    .await;

    assert!(
        app.apply_pending_external_import_monitor_snapshot_for_library(
            &MediaFacet::Movie,
            &default_library_id,
        )
        .await
        .expect("default-library snapshot should apply")
    );
    let first_after = app
        .services
        .catalog
        .titles
        .get_by_id(&first.id)
        .await
        .expect("first title should load")
        .expect("first title should exist");
    let second_before = app
        .services
        .catalog
        .titles
        .get_by_id(&second.id)
        .await
        .expect("second title should load")
        .expect("second title should exist");
    assert!(!first_after.monitored);
    assert!(!second_before.monitored);
    assert!(
        !app.apply_pending_external_import_monitor_snapshot_for_library(
            &MediaFacet::Movie,
            &default_library_id,
        )
        .await
        .expect("consumed default-library snapshot should be absent")
    );

    assert!(
        app.apply_pending_external_import_monitor_snapshot_for_library(
            &MediaFacet::Movie,
            &second_library.id,
        )
        .await
        .expect("second-library snapshot should apply")
    );
    let second_after = app
        .services
        .catalog
        .titles
        .get_by_id(&second.id)
        .await
        .expect("second title should load")
        .expect("second title should exist");
    assert!(second_after.monitored);

    append_movie_monitor_snapshot_chunk_for_library(
        &app,
        &user,
        &second_library.id,
        vec![ExternalImportMonitorMovieEntry {
            tmdb_id: Some("777001".to_string()),
            imdb_id: None,
            path: None,
            monitored: false,
        }],
    )
    .await;
    assert!(
        app.apply_pending_external_import_monitor_snapshot_for_library(
            &MediaFacet::Movie,
            &second_library.id,
        )
        .await
        .is_err(),
        "an unresolved snapshot should remain retryable"
    );
    let retry_title = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Snapshot Retry Identity".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tmdb".to_string(),
                    value: "777001".to_string(),
                }],
                ..Default::default()
            },
            second_library.id.clone(),
        )
        .await
        .expect("retry title should be created")
        .title;
    assert!(
        app.apply_pending_external_import_monitor_snapshot_for_library(
            &MediaFacet::Movie,
            &second_library.id,
        )
        .await
        .expect("retained snapshot should apply on retry")
    );
    let retry_after = app
        .services
        .catalog
        .titles
        .get_by_id(&retry_title.id)
        .await
        .expect("retry title should load")
        .expect("retry title should exist");
    assert!(!retry_after.monitored);
}

#[tokio::test]
async fn external_import_monitor_snapshot_emits_title_updated_without_actor() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Monitor Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "4242".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert!(events.iter().all(|event| event.actor_user_id.is_none()));

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let reapplied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("reapply monitor snapshot");

    assert!(reapplied);
    let replay_events = title_updated_events(&app, &title.id).await;
    assert_eq!(replay_events.len(), 1);
    assert!(
        replay_events
            .iter()
            .all(|event| event.actor_user_id.is_none())
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_applies_series_child_monitoring() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Child Monitor Fixture".into(),
                facet: MediaFacet::Series,
                monitored: false,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "5252".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("5252".to_string()),
            path: None,
            monitored: true,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let stored_title = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");

    assert!(stored_title.monitored);
    assert!(collections.iter().any(|collection| collection.monitored));
    assert!(episodes.iter().any(|episode| episode.monitored));
}

#[tokio::test]
async fn external_import_monitor_snapshot_emits_title_updated_for_child_only_changes() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Child Activity Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "6262".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");
    app.set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("disable episode");

    let events_before_apply = title_updated_events(&app, &title.id).await.len();

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("6262".to_string()),
            path: None,
            monitored: true,
            seasons: vec![ExternalImportMonitorSeasonEntry {
                season_number: 1,
                monitored: true,
            }],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_episode.monitored);

    let events_after_apply = title_updated_events(&app, &title.id).await;
    assert_eq!(events_after_apply.len(), events_before_apply + 1);
    assert!(
        events_after_apply
            .last()
            .expect("latest event")
            .actor_user_id
            .is_none()
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_enables_collection_for_monitored_episode_override() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Episode Override Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "7373".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");
    app.set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("disable collection");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("7373".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    // The snapshot reconciles monitoring state; acquisition targets are
    // derived from that state, no wanted rows are materialized on apply.
    let updated_collection = app
        .get_collection(&user, &collection.id)
        .await
        .expect("get collection")
        .expect("collection exists");
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_collection.monitored);
    assert!(updated_episode.monitored);
}

// ---------------------------------------------------------------------------
// Title tags (registry-gated user tags)
// ---------------------------------------------------------------------------

/// A movie title in the default movie library, plus the tags it should carry.
async fn create_tagged_movie(app: &AppUseCase, actor: &User, name: &str) -> Title {
    app.add_title(
        actor,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            ..Default::default()
        },
    )
    .await
    .expect("title should be created")
}

/// A minimal enabled delay profile whose only interesting field is its tag list.
fn tagged_delay_profile(tags: Vec<String>) -> crate::DelayProfile {
    crate::DelayProfile {
        id: "profile-one".to_string(),
        name: "Tagged".to_string(),
        usenet_delay_minutes: 0,
        torrent_delay_minutes: 0,
        preferred_protocol: crate::PreferredProtocol::Usenet,
        min_age_minutes: 0,
        bypass_score_threshold: None,
        enable_usenet: true,
        enable_torrent: true,
        bypass_if_highest_quality: false,
        applies_to_facets: Vec::new(),
        tags,
        priority: 0,
        enabled: true,
    }
}

async fn stored_title_tags(app: &AppUseCase, title_id: &str) -> Vec<String> {
    app.services
        .catalog
        .titles
        .get_by_id(title_id)
        .await
        .expect("title should load")
        .expect("title should exist")
        .tags
}

#[tokio::test]
async fn the_raw_tag_write_refuses_a_label_the_registry_does_not_define() {
    let (app, user) = bootstrap();
    let title = create_tagged_movie(&app, &user, "Registry Gate").await;

    let error = app
        .update_title_metadata(
            &user,
            &title.id,
            None,
            None,
            Some(vec!["not defined".to_string()]),
        )
        .await
        .expect_err("an unregistered label must be refused");
    assert!(
        error.to_string().contains("not defined"),
        "the refusal must name the label: {error}"
    );
    assert!(stored_title_tags(&app, &title.id).await.is_empty());

    // The same write succeeds once an administrator defines the tag.
    app.create_title_tag_definition(&user, "not defined", None)
        .await
        .expect("tag should be defined");
    let updated = app
        .update_title_metadata(
            &user,
            &title.id,
            None,
            None,
            Some(vec!["not defined".to_string()]),
        )
        .await
        .expect("a defined label is accepted");
    assert_eq!(updated.tags, vec!["not defined".to_string()]);
}

#[tokio::test]
async fn the_tag_patch_refuses_the_reserved_namespace_and_undefined_labels() {
    let (app, user) = bootstrap();
    let title = create_tagged_movie(&app, &user, "Reserved Namespace").await;
    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:all".to_string()]),
    )
    .await
    .expect("structured settings entries stay writable");

    // A reserved entry cannot be applied as if it were a user tag, and it
    // cannot be stripped through the tag door either.
    let reserved = app
        .update_title_tags(
            &user,
            &[title.id.clone()],
            &["scryer:monitor-type:none".to_string()],
            &[],
        )
        .await
        .expect_err("the reserved prefix must be refused");
    assert!(reserved.to_string().contains("scryer:"), "{reserved}");

    let undefined = app
        .update_title_tags(&user, &[title.id.clone()], &["keep".to_string()], &[])
        .await
        .expect_err("an undefined label must be refused");
    assert!(undefined.to_string().contains("keep"), "{undefined}");

    assert_eq!(
        stored_title_tags(&app, &title.id).await,
        vec!["scryer:monitor-type:all".to_string()]
    );
}

#[tokio::test]
async fn the_tag_patch_preserves_structured_entries_and_emits_one_event_per_title() {
    let (app, user) = bootstrap();
    let title = create_tagged_movie(&app, &user, "Patch Preserves").await;
    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:all".to_string()]),
    )
    .await
    .expect("set a structured entry");
    for label in ["keep", "needs review"] {
        app.create_title_tag_definition(&user, label, None)
            .await
            .expect("tag should be defined");
    }

    let updated = app
        .update_title_tags(
            &user,
            &[title.id.clone()],
            &["Keep".to_string(), " Needs  Review ".to_string()],
            &[],
        )
        .await
        .expect("tags should apply");
    assert_eq!(updated.len(), 1);
    assert_eq!(
        updated[0].tags,
        vec![
            "scryer:monitor-type:all".to_string(),
            "keep".to_string(),
            "needs review".to_string(),
        ],
        "the structured entry survives and the labels arrive normalized"
    );

    // Removing one leaves the other and the structured entry alone.
    app.update_title_tags(
        &user,
        &[title.id.clone()],
        &[],
        &["needs review".to_string()],
    )
    .await
    .expect("tags should be removed");
    assert_eq!(
        stored_title_tags(&app, &title.id).await,
        vec!["scryer:monitor-type:all".to_string(), "keep".to_string()]
    );

    // One title-updated event per patch call, so the catalog refreshes once.
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 3, "one per metadata write plus one per patch");
}

#[tokio::test]
async fn a_bulk_tag_patch_writes_nothing_when_one_library_is_denied() {
    let (app, admin) = bootstrap();
    app.create_title_tag_definition(&admin, "keep", None)
        .await
        .expect("tag should be defined");

    let allowed_title = create_tagged_movie(&app, &admin, "Allowed Movie").await;
    let restricted_library = app
        .create_library(
            &admin,
            MediaFacet::Movie,
            "Restricted Movies".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/RestrictedMovies".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("restricted library should be created");
    let restricted_title = app
        .create_title_without_hydration_in_library(
            &admin,
            NewTitle {
                name: "Restricted Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
            restricted_library.id.clone(),
        )
        .await
        .expect("restricted title should be created")
        .title;

    // The editor manages the default libraries only; the restricted library is
    // deliberately absent from the grant list.
    let (created_editor, editor) = create_authenticated_user(
        &app,
        &admin,
        "tag-editor",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    app.set_user_library_permissions(
        &admin,
        &created_editor.id,
        test_library_grants_from_presets(&[
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ]),
    )
    .await
    .expect("grants should apply");

    let error = app
        .update_title_tags(
            &editor,
            &[allowed_title.id.clone(), restricted_title.id.clone()],
            &["keep".to_string()],
            &[],
        )
        .await
        .expect_err("a denied library must refuse the whole batch");
    assert!(matches!(error, AppError::Unauthorized(_)), "{error}");

    // All-or-nothing: the title the editor *could* manage is untouched too.
    assert!(
        stored_title_tags(&app, &allowed_title.id).await.is_empty(),
        "a partially authorized batch must write nothing"
    );
    assert!(
        stored_title_tags(&app, &restricted_title.id)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn renaming_a_tag_rewrites_titles_and_delay_profiles() {
    let (app, user) = bootstrap();
    let definition = app
        .create_title_tag_definition(&user, "Keep", Some("  hold on to this  ".to_string()))
        .await
        .expect("tag should be defined");
    assert_eq!(definition.label, "keep");
    assert_eq!(definition.description.as_deref(), Some("hold on to this"));

    let first = create_tagged_movie(&app, &user, "Rename One").await;
    let second = create_tagged_movie(&app, &user, "Rename Two").await;
    app.update_title_tags(
        &user,
        &[first.id.clone(), second.id.clone()],
        &["keep".to_string()],
        &[],
    )
    .await
    .expect("tags should apply");

    app.upsert_delay_profile(&user, tagged_delay_profile(vec!["keep".to_string()]))
        .await
        .expect("delay profile should save");

    let renamed = app
        .update_title_tag_definition(&user, &definition.id, Some("Archive".to_string()), None)
        .await
        .expect("rename should apply");
    assert_eq!(renamed.definition.label, "archive");
    assert_eq!(renamed.counts.titles, 2);
    assert_eq!(renamed.counts.delay_profiles, 1);
    // No rules are configured in this fixture, so nothing references the old
    // label; the counts exist so the UI can warn that Rego is never rewritten.
    assert_eq!(renamed.counts.maintenance_rule_sets, 0);
    assert_eq!(renamed.counts.release_rule_sets, 0);

    assert_eq!(
        stored_title_tags(&app, &first.id).await,
        vec!["archive".to_string()]
    );
    assert_eq!(
        stored_title_tags(&app, &second.id).await,
        vec!["archive".to_string()]
    );
    let profiles = app
        .get_delay_profiles(&user)
        .await
        .expect("delay profiles should load");
    assert_eq!(profiles[0].tags, vec!["archive".to_string()]);

    // The registry read reports the label and its live membership count.
    let registry = app
        .title_tag_definitions(&user)
        .await
        .expect("registry should load");
    assert_eq!(registry.len(), 1);
    assert_eq!(registry[0].definition.label, "archive");
    assert_eq!(registry[0].title_count, 2);
}

#[tokio::test]
async fn deleting_a_tag_strips_it_from_titles_and_delay_profiles() {
    let (app, user) = bootstrap();
    let definition = app
        .create_title_tag_definition(&user, "keep", None)
        .await
        .expect("tag should be defined");
    let title = create_tagged_movie(&app, &user, "Delete Cleanup").await;
    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:all".to_string()]),
    )
    .await
    .expect("set a structured entry");
    app.update_title_tags(&user, &[title.id.clone()], &["keep".to_string()], &[])
        .await
        .expect("tags should apply");
    app.upsert_delay_profile(
        &user,
        tagged_delay_profile(vec!["keep".to_string(), "other".to_string()]),
    )
    .await
    .expect("delay profile should save");

    let counts = app
        .delete_title_tag_definition(&user, &definition.id)
        .await
        .expect("delete should apply");
    assert_eq!(counts.titles, 1);
    assert_eq!(counts.delay_profiles, 1);

    assert_eq!(
        stored_title_tags(&app, &title.id).await,
        vec!["scryer:monitor-type:all".to_string()],
        "deleting a tag must not disturb structured settings entries"
    );
    let profiles = app
        .get_delay_profiles(&user)
        .await
        .expect("delay profiles should load");
    assert_eq!(profiles[0].tags, vec!["other".to_string()]);
    assert!(
        app.title_tag_definitions(&user)
            .await
            .expect("registry should load")
            .is_empty()
    );
}

#[tokio::test]
async fn the_tag_patch_enforces_the_per_title_ceiling() {
    let (app, user) = bootstrap();
    let title = create_tagged_movie(&app, &user, "Ceiling").await;

    let labels = (0..=crate::MAX_USER_TAGS_PER_TITLE)
        .map(|index| format!("tag {index}"))
        .collect::<Vec<_>>();
    for label in &labels {
        app.create_title_tag_definition(&user, label, None)
            .await
            .expect("tag should be defined");
    }

    app.update_title_tags(
        &user,
        &[title.id.clone()],
        &labels[..crate::MAX_USER_TAGS_PER_TITLE],
        &[],
    )
    .await
    .expect("the ceiling itself is allowed");
    assert_eq!(
        stored_title_tags(&app, &title.id).await.len(),
        crate::MAX_USER_TAGS_PER_TITLE
    );

    let error = app
        .update_title_tags(
            &user,
            &[title.id.clone()],
            &[labels[crate::MAX_USER_TAGS_PER_TITLE].clone()],
            &[],
        )
        .await
        .expect_err("one past the ceiling must be refused");
    assert!(
        error
            .to_string()
            .contains(&crate::MAX_USER_TAGS_PER_TITLE.to_string()),
        "{error}"
    );
    assert_eq!(
        stored_title_tags(&app, &title.id).await.len(),
        crate::MAX_USER_TAGS_PER_TITLE,
        "a refused patch leaves the bag as it was"
    );
}
