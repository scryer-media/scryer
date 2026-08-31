use super::*;

#[tokio::test]
async fn list_cutoff_unmet_titles_normalizes_lowercase_cutoff_tier() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-lowercase""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-lowercase",
            "720p",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Case".into(),
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

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Case.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id, None);
    assert_eq!(items[0].current_tier, "480P");
    assert_eq!(items[0].target_tier, "720P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_returns_episode_scoped_rows_for_series() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-series""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-series",
            "1080P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Episodes".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Episodes/Season 01/Cutoff Episodes - S01E01.mkv"
                .to_string(),
            size_bytes: 1_000,
            quality_label: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link media file to episode");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Series), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(items[0].current_tier, "720P");
    assert_eq!(items[0].target_tier, "1080P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_skips_legacy_titles_with_stale_profile_tags() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-global""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-global",
            "720P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, titles) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Tag".into(),
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
    titles
        .store
        .lock()
        .await
        .iter_mut()
        .find(|stored| stored.id == title.id)
        .expect("stored title")
        .tags
        .push("scryer:quality-profile:missing-profile".to_string());

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Stale Tag.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert!(items.is_empty());
}

#[tokio::test]
async fn search_titles_supports_facet_filter() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Movie A".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create movie");

    app.add_title(
        &user,
        NewTitle {
            name: "Show B".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create series");

    let tvs = app
        .list_titles_unpaged(&user, Some(MediaFacet::Series), None, None)
        .await
        .expect("list titles");

    assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
}

#[tokio::test]
async fn search_indexers_for_title_keeps_direct_nab_searches_uncategorized_when_routing_is_empty() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingCategoriesIndexerClient::new(
        "Generic.Release.2026.1080p.WEB-DL",
    ));
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings, recording_client.clone());

    let movie = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Movie".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    let series = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    let anime = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Anime".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    app.search_indexers_for_title(
        &user,
        movie.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("movie search should succeed");
    app.search_indexers_for_title(
        &user,
        series.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("series search should succeed");
    app.search_indexers_for_title(
        &user,
        anime.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("anime search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].newznab_categories, None);
    assert_eq!(calls[1].newznab_categories, None);
    assert_eq!(calls[2].newznab_categories, None);
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_series_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-series", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Signal".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: Some("tvdb-series-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create series episode");

    app.search_indexers_for_episode(
        &user,
        title.id.clone(),
        "2".to_string(),
        "11".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("series episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Signal S02E11".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: None,
        }]
    );
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_anime_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-anime", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Atlas".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime season");

    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("35".to_string()),
            overview: None,
            tvdb_id: Some("tvdb-anime-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime episode");

    app.search_indexers_for_episode(
        &user,
        title.id.clone(),
        "2".to_string(),
        "11".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("anime episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Atlas 035".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: Some(35),
        }]
    );
}

#[tokio::test]
async fn search_indexers_anime_required_original_and_english_accepts_dual_audio_release() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Anime.Show.S01E01.1080p.WEB-DL.DUAL.H.265",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    app.set_facet_required_audio_languages(
        &user,
        "anime",
        vec!["original".to_string(), "English".to_string()],
    )
    .await
    .expect("set anime required audio");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Anime Show".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                language: Some("jpn".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let results = app
        .search_indexers_for_title(
            &user,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    let parsed = results[0]
        .parsed_release_metadata
        .as_ref()
        .expect("search result should be parsed");
    assert_eq!(
        parsed.languages_audio,
        vec!["eng".to_string(), "jpn".to_string()]
    );
    let decision = results[0]
        .quality_profile_decision
        .as_ref()
        .expect("search result should be scored");
    assert!(decision.allowed);
    assert!(
        decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == "required_audio_languages_match")
    );
}

#[tokio::test]
async fn search_indexers_for_title_uses_tagged_aliases_for_auto_evaluation() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Nightfall.Heavy.Chorus.Dark.Lantern.S01E01.1080p.NF.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let search_user = create_user_with_permissions(
        &app,
        &user,
        "title_search_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create search user");
    let search_token = app
        .issue_access_token(&search_user)
        .await
        .expect("issue search token");
    let authed_search_user = app
        .authenticate_token(&search_token)
        .await
        .expect("authenticate search user");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Nightfall!!".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "1309".to_string(),
                }],
                year: Some(2022),
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &title.id,
            TitleMetadataUpdate {
                tagged_aliases: vec![scryer_domain::TaggedAlias {
                    name: "Nightfall Heavy Chorus Dark Lantern".to_string(),
                    language: "eng".to_string(),
                }],
                ..Default::default()
            },
        )
        .await
        .expect("persist tagged aliases");

    let results = app
        .search_indexers_for_title(
            &authed_search_user,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].auto_eligible, Some(true));
    assert_eq!(results[0].auto_decision_code.as_deref(), Some("eligible"));
    assert!(results[0].candidate_token.is_some());
}

#[tokio::test]
async fn search_indexers_for_title_returns_results_when_candidate_token_attachment_fails() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Failure.Recovery.2026.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery".into(),
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

    let mut ghost_actor = User {
        id: "ghost-search-user".to_string(),
        username: "ghost".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    ghost_actor.authorization = scryer_domain::UserAuthorization {
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    let results = app
        .search_indexers_for_title(
            &ghost_actor,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search should still succeed without candidate signing key");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].candidate_token, None);
}

#[tokio::test]
async fn list_cutoff_unmet_titles_page_bounds_and_total() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p720""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p720", "720p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    // Three monitored movies, each with a below-cutoff (480p vs 720p) file.
    for name in ["Alpha", "Bravo", "Charlie"] {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: name.into(),
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
        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/{name}.mkv"),
                size_bytes: 1_000,
                quality_label: Some("480p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
    }

    // First page of 2 of 3, with the full total reported.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 2, 0)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].title_name, "Alpha");
    assert_eq!(page.items[1].title_name, "Bravo");

    // Second page: remainder.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 2, 2)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].title_name, "Charlie");

    // limit == 0 returns just the total.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 0, 0)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert!(page.items.is_empty());
}

/// The Wanted page reads `currentScore` off the state row attached to each
/// derived view. Those rows come straight from the repository, where the bar is
/// always unset because it is resolved on read — so the page rendered null for
/// every occupied scope while the `Title.wantedItems` relation, which went
/// through the decorator, showed a number (D10/A2).
///
/// The number must also be the *re-derived* bar rather than the persisted
/// `acquisition_score`, which is display history: the fixture stores a
/// deliberately impossible score so a read-back would be unmistakable.
#[tokio::test]
async fn wanted_scope_views_report_the_re_derived_landed_bar() {
    const STORED_LIE: i32 = -4_242;

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p720""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p720", "720p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let acquisition_scope_states = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());
    let app = app.with_test_overrides(|services| {
        services.with_acquisition_scope_states(acquisition_scope_states)
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Landed Bar".into(),
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

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Landed Bar.mkv".to_string(),
            size_bytes: 4 * 1024 * 1024 * 1024,
            quality_label: Some("480p".to_string()),
            scene_name: Some("Landed.Bar.2024.480p.WEB-DL.H.264-GROUP".to_string()),
            acquisition_score: Some(STORED_LIE),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let list = async || {
        app.list_wanted_scope_views(
            &user,
            WantedKind::CutoffUpgrade,
            Some(MediaFacet::Movie),
            Vec::new(),
            None,
            50,
            0,
        )
        .await
        .expect("wanted scope views should resolve")
    };

    // **No state row yet.** The Wanted page derives its rows from the
    // projection, so a scope a library scan filled below cutoff appears here
    // without ever having been searched or grabbed. Keying the bar on the state
    // row left `currentScore` null for exactly this case.
    let (views, total) = list().await;
    assert_eq!(total, 1);
    assert!(
        views[0].state.is_none(),
        "fixture precondition: this scope must have no state row yet"
    );
    let bar = views[0]
        .landed_bar
        .expect("an occupied scope reports the bar its file sets, row or no row");
    assert_ne!(
        bar, STORED_LIE,
        "the bar must be re-derived from the row, not read back from acquisition_score"
    );

    // With a row, the same number, and mirrored onto the row so the
    // `Title.wantedItems` relation agrees.
    let state_view = app.new_wanted_state_view(&title, "movie", None, None, None, None);
    app.services
        .workflow
        .acquisition_scope_states
        .ensure_acquisition_scope_state(&state_view)
        .await
        .expect("materialize the scope's state row");

    let (views, total) = list().await;
    assert_eq!(total, 1);
    assert_eq!(views[0].landed_bar, Some(bar));
    assert_eq!(
        views[0]
            .state
            .as_ref()
            .expect("the scope's state row is attached")
            .landed_bar,
        Some(bar)
    );
}

// ── Per-title release blocklist (search-time exclusion) ─────────────────────

/// Interactive-search fixture with an in-memory blocklist repo: a fixed
/// indexer answering `release_title` for every query and one NZBGet client so
/// NZB results survive the download-capability filter.
async fn bootstrap_interactive_search_with_blocklist(
    release_title: &str,
) -> (AppUseCase, User, Arc<MockBlocklistRepo>) {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);
    let blocklist = Arc::new(MockBlocklistRepo::default());
    let app = app.with_test_overrides(|services| services.with_blocklist_repo(blocklist.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    (app, user, blocklist)
}

async fn add_movie_title_for_search(app: &AppUseCase, user: &User, name: &str) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: name.into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
    )
    .await
    .expect("create title")
}

async fn interactive_search_titles(app: &AppUseCase, user: &User, title_id: &str) -> Vec<String> {
    app.search_indexers_for_title(
        user,
        title_id.to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("search indexers for title")
    .into_iter()
    .map(|result| result.title)
    .collect()
}

fn blocklist_entry_for(title_id: &str, release_name: &str, reason: &str) -> NewBlocklistEntry {
    NewBlocklistEntry {
        title_id: title_id.to_string(),
        release_name: release_name.to_string(),
        indexer_id: String::new(),
        info_hash: None,
        reason: Some(reason.to_string()),
    }
}

#[tokio::test]
async fn interactive_search_blocklist_is_per_title_and_removal_reallows_the_release() {
    let release_title = "Blocklisted.Movie.2024.1080p.WEB-DL-GRP";
    let (app, user, blocklist) = bootstrap_interactive_search_with_blocklist(release_title).await;
    let blocked = add_movie_title_for_search(&app, &user, "Blocklisted Movie").await;
    let other = add_movie_title_for_search(&app, &user, "Other Movie").await;

    // Grab-path writers keep the indexer casing (and may carry whitespace); the
    // read side normalizes both sides, so this must still exclude the release.
    blocklist
        .block(&blocklist_entry_for(
            &blocked.id,
            "  BLOCKLISTED.Movie.2024.1080p.WEB-DL-GRP ",
            "download client failure: corrupt archive",
        ))
        .await
        .expect("seed blocklist entry");
    let entry_id = blocklist
        .list_for_title(&blocked.id, 10)
        .await
        .expect("list blocklist")
        .first()
        .map(|entry| entry.id.clone())
        .expect("the seeded block is listed");

    assert!(
        interactive_search_titles(&app, &user, &blocked.id)
            .await
            .is_empty(),
        "a blocklisted release must not be offered for its title"
    );
    assert_eq!(
        interactive_search_titles(&app, &user, &other.id).await,
        vec![release_title.to_string()],
        "an entry for one title must never hide the same release from another title"
    );

    // Removal from the UI re-allows the release immediately; no retention purge
    // of the failed-attempt log is involved.
    app.clear_title_release_blocklist_entry(&user, &entry_id)
        .await
        .expect("clear blocklist entry");
    assert_eq!(
        interactive_search_titles(&app, &user, &blocked.id).await,
        vec![release_title.to_string()],
        "removing the entry must re-allow the release for its title"
    );
}

#[tokio::test]
async fn interactive_search_ignores_failed_attempt_history_without_a_blocklist_entry() {
    // The failed-attempt log is history/audit only: a Failed attempt with no
    // blocklist entry (e.g. one whose entry the operator removed) must not gate.
    let release_title = "Attempted.Movie.2024.1080p.WEB-DL-GRP";
    let (app, user, blocklist) = bootstrap_interactive_search_with_blocklist(release_title).await;
    let title = add_movie_title_for_search(&app, &user, "Attempted Movie").await;

    app.services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            Some("https://example.invalid/attempted.nzb".to_string()),
            Some(release_title.to_string()),
            ReleaseDownloadAttemptOutcome::Failed,
            Some("download client failure: corrupt archive".to_string()),
            None,
        )
        .await
        .expect("record failed attempt");
    assert!(
        blocklist
            .list_for_title(&title.id, 10)
            .await
            .expect("list blocklist")
            .is_empty()
    );

    assert_eq!(
        interactive_search_titles(&app, &user, &title.id).await,
        vec![release_title.to_string()],
        "a failed attempt without a blocklist entry must not exclude the release"
    );
}

#[tokio::test]
async fn clearing_a_blocklist_entry_reallows_the_release() {
    let release_title = "Duplicated.Movie.2024.1080p.WEB-DL-GRP";
    let (app, user, blocklist) = bootstrap_interactive_search_with_blocklist(release_title).await;
    let title = add_movie_title_for_search(&app, &user, "Duplicated Movie").await;
    let other = add_movie_title_for_search(&app, &user, "Other Movie").await;

    blocklist
        .block(&blocklist_entry_for(
            &title.id,
            release_title,
            "grab failed: rejected by client",
        ))
        .await
        .expect("seed grab-path entry");
    // The same failure recorded again from the client-failure path -- differing
    // only in casing -- writes nothing. There is no sibling row to sweep up,
    // which is why clearing is a single delete.
    assert!(
        !blocklist
            .block(&blocklist_entry_for(
                &title.id,
                &release_title.to_ascii_lowercase(),
                "download client failure: corrupt archive",
            ))
            .await
            .expect("second record of the same failure")
    );
    // Unrelated blocks that must survive: a different release on this title,
    // and the same release on another title.
    blocklist
        .block(&blocklist_entry_for(
            &title.id,
            "Unrelated.Movie.2024.720p.WEB-DL-GRP",
            "manual_replacement",
        ))
        .await
        .expect("seed unrelated entry");
    blocklist
        .block(&blocklist_entry_for(
            &other.id,
            release_title,
            "grab failed: rejected by client",
        ))
        .await
        .expect("seed other-title entry");

    let entries = blocklist
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(entries.len(), 2, "the duplicate failure added no row");
    assert!(
        interactive_search_titles(&app, &user, &title.id)
            .await
            .is_empty()
    );

    let blocked_id = entries
        .iter()
        .find(|entry| entry.normalized_release_name == release_title.to_ascii_lowercase())
        .map(|entry| entry.id.clone())
        .expect("the blocked release is listed");
    app.clear_title_release_blocklist_entry(&user, &blocked_id)
        .await
        .expect("clear blocklist entry");

    assert_eq!(
        blocklist
            .list_for_title(&title.id, 10)
            .await
            .expect("list blocklist")
            .len(),
        1,
        "only the cleared release goes"
    );
    assert_eq!(
        blocklist
            .list_for_title(&other.id, 10)
            .await
            .expect("list other blocklist")
            .len(),
        1,
        "another title's block on the same release is untouched"
    );
    assert_eq!(
        interactive_search_titles(&app, &user, &title.id).await,
        vec![release_title.to_string()],
        "the release is searchable again once its block is cleared"
    );
}

/// A blocklist repository whose every call fails, to prove search fails open.
struct FailingBlocklistRepo;

#[async_trait]
impl BlocklistRepository for FailingBlocklistRepo {
    async fn block(&self, _: &NewBlocklistEntry) -> AppResult<bool> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn list_for_title(&self, _: &str, _: usize) -> AppResult<Vec<BlocklistEntry>> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn list_all(&self, _: usize, _: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn get(&self, _: &str) -> AppResult<Option<BlocklistEntry>> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn is_blocked(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> AppResult<bool> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn remove(&self, _: &str) -> AppResult<()> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
    async fn delete_for_indexer(&self, _: &str) -> AppResult<()> {
        Err(AppError::Repository("blocklist unavailable".to_string()))
    }
}

#[tokio::test]
async fn interactive_search_fails_open_when_the_blocklist_repository_errors() {
    let release_title = "Unblocked.Movie.2024.1080p.WEB-DL-GRP";
    let (app, user, _) = bootstrap_interactive_search_with_blocklist(release_title).await;
    let app = app.with_test_overrides(|services| {
        services.with_blocklist_repo(Arc::new(FailingBlocklistRepo))
    });
    let title = add_movie_title_for_search(&app, &user, "Unblocked Movie").await;

    assert_eq!(
        interactive_search_titles(&app, &user, &title.id).await,
        vec![release_title.to_string()],
        "a blocklist read failure must warn and exclude nothing, not fail the search"
    );
}

#[tokio::test]
async fn title_release_blocklist_signatures_are_per_title_and_normalized() {
    let release_title = "Signature.Movie.2024.1080p.WEB-DL-GRP";
    let (app, user, blocklist) = bootstrap_interactive_search_with_blocklist(release_title).await;
    let title = add_movie_title_for_search(&app, &user, "Normalized Movie").await;
    let other = add_movie_title_for_search(&app, &user, "Other Movie").await;

    // Release names compare trimmed + lowercased on both sides.
    blocklist
        .block(&blocklist_entry_for(
            &title.id,
            "  Mixed.Case.Release.2024.1080p ",
            "grab failed",
        ))
        .await
        .expect("seed mixed-case entry");
    // A nameless block is not a block: there is nothing to match on.
    assert!(
        !blocklist
            .block(&blocklist_entry_for(&title.id, "   ", "empty"))
            .await
            .expect("whitespace-only name is refused")
    );
    blocklist
        .block(&blocklist_entry_for(
            &other.id,
            "Other.Title.Release.2024.1080p",
            "grab failed",
        ))
        .await
        .expect("seed other-title entry");

    let signatures = app.load_title_release_blocklist_signatures(&title.id).await;
    assert_eq!(
        signatures.release_names,
        HashSet::from([(String::new(), "mixed.case.release.2024.1080p".to_string())])
    );
    assert!(signatures.info_hashes.is_empty());

    let other_signatures = app.load_title_release_blocklist_signatures(&other.id).await;
    assert_eq!(
        other_signatures.release_names,
        HashSet::from([(String::new(), "other.title.release.2024.1080p".to_string())])
    );
}

// ── One season-pack gate (D8) ─────────────────────────────────────────────

/// The pack subject is the collection's **monitored** episodes, and a monitored
/// member that has not aired refuses the pack outright.
///
/// Both halves were wrong before. The subject used every episode
/// `list_episodes_for_collection` returned, so a partially-monitored season had
/// "missing" members nobody wanted and admitted every pack; and nothing checked
/// air dates, so a mid-season pack was fetched on every cycle, arrived partial,
/// and then blocked the per-episode searches that would have filled it (Sonarr's
/// `FullSeasonSpecification`).
#[tokio::test]
async fn a_pack_subject_covers_monitored_members_and_refuses_an_unaired_season() {
    use crate::admission::{AdmissionPolicy, CandidateFacts, evaluate_admission};

    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Pack Gate".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("3".into()),
        )
        .await
        .expect("create collection");

    let mut episode_ids = Vec::new();
    // Two aired members, plus one that airs in a week.
    for (number, air_date) in [
        ("1", (Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
        ("2", (Utc::now() - chrono::Duration::days(23)).to_rfc3339()),
        ("3", (Utc::now() + chrono::Duration::days(7)).to_rfc3339()),
    ] {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some(air_date),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    let profile = test_quality_profile("pack-gate");
    let scope = crate::SubmissionScope::Collection {
        collection_id: collection.id.clone(),
    };
    let scoring_context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let policy = AdmissionPolicy::not_a_downgrade();
    let candidate = CandidateFacts::new(Some(0), 0, 900);

    let subject = app
        .admission_subject_for_scope(
            &title,
            &scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    let verdict = evaluate_admission(&subject, candidate, &policy);
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(crate::admission::AdmissionRejectionReason::SeasonIncomplete)
        ),
        "a season with an unaired monitored member cannot be packed: {verdict:?}"
    );

    // Unmonitor the unaired member: it is no longer part of the scope, so it
    // neither blocks the pack nor counts as a member it must fill.
    app.set_episode_monitored(&user, &episode_ids[2], false)
        .await
        .expect("unmonitor the unaired episode");
    let subject = app
        .admission_subject_for_scope(
            &title,
            &scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        evaluate_admission(&subject, candidate, &policy).is_admitted(),
        "an unmonitored episode is not part of the scope"
    );

    // …and with every member unmonitored there is nothing a pack could fill, so
    // the pack is refused rather than fetched for a season nobody wants.
    for episode_id in &episode_ids[..2] {
        app.set_episode_monitored(&user, episode_id, false)
            .await
            .expect("unmonitor the aired episodes");
    }
    let subject = app
        .admission_subject_for_scope(
            &title,
            &scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        subject.is_unoccupied(),
        "an entirely unmonitored season resolves to an empty subject"
    );
    let verdict = evaluate_admission(&subject, candidate, &policy);
    assert!(
        !verdict.is_admitted(),
        "an empty pack scope must not admit a whole season: {verdict:?}"
    );
}

/// **MA4.** The Wanted page's `currentScore` is the bar the gate compares
/// against (D10), which means the same file has to be measured against the same
/// runtime on both paths.
///
/// The listing joins the file-episode table, so a two-episode file arrives as
/// two rows. Scoring a row measured a 48-minute file against one 24-minute
/// episode — twice the modelled size, a different size band, a displayed number
/// the gate never uses.
#[tokio::test]
async fn a_multi_episode_files_landed_bar_matches_the_gates_incumbent_bar() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p1080""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p1080", "1080p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Double Bill".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("2".into()),
        )
        .await
        .expect("create collection");

    let mut episode_ids = Vec::new();
    for number in 1..=2 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some((Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
                // 24 minutes each, so the span is 48.
                Some(1_440),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    // One file covering both episodes, sized for 48 minutes of 1080p WEB-DL.
    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Double Bill/Season 01/Double Bill - S01E01-E02.mkv".to_string(),
            size_bytes: 2_700_000_000,
            quality_label: Some("1080p".to_string()),
            scene_name: Some("Double.Bill.S01E01-E02.1080p.WEB-DL.H.264-GRP".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode_ids[0])
        .await
        .expect("link first episode");
    // The store joins one row per link; the mock's `link_file_to_episode`
    // overwrites, so the second membership is seeded the way the join emits it.
    {
        let mut store = media_files.store.lock().await;
        let mut second = store
            .iter()
            .find(|entry| entry.id == file_id)
            .cloned()
            .expect("seeded file");
        second.episode_id = Some(episode_ids[1].clone());
        store.push(second);
    }

    let profile = app
        .resolve_quality_profile_for_title(&title)
        .await
        .expect("resolve profile");
    let scoring_context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let subject = app
        .admission_subject_for_scope(
            &title,
            &crate::SubmissionScope::EpisodeSet {
                episode_ids: episode_ids.clone(),
            },
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Import,
        )
        .await;
    let incumbent = subject
        .incumbents()
        .first()
        .expect("the two-episode file occupies the span");
    assert_eq!(
        incumbent.covers.len(),
        2,
        "the gate must see the file's whole span"
    );

    let bars = app
        .landed_bars_for_scopes(&[crate::acquisition_workflow::LandedBarScope {
            title_id: title.id.clone(),
            episode_id: Some(episode_ids[0].clone()),
            collection_id: None,
            series_movie_link_id: None,
        }])
        .await;
    assert_eq!(
        bars[0],
        Some(incumbent.score),
        "the displayed bar must be the number the gate compares against"
    );
}

/// **F-3b-2 / D18.** A queued release is scored with the size it announced, so
/// the pseudo-incumbent and the candidate beside it are measured on the same
/// terms.
///
/// Scoring the in-flight release with no size left it without a size term while
/// the candidate had one: a candidate identical to the queued release but
/// announced in a larger band beat it by the band weight and was grabbed as a
/// duplicate.
#[tokio::test]
async fn a_queued_releases_announced_size_is_part_of_its_score() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Sized Queue".into(),
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

    let profile = test_quality_profile("sized-queue");
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let release = "Sized.Queue.2024.1080p.WEB-DL.H.264-GRP";
    let score_at = |size: Option<i64>| {
        crate::quality::canonical_context::score_parked_release_title(
            &title,
            release,
            size,
            &[],
            &[],
            &context,
        )
        .score
    };

    let size_less = score_at(None);
    let plausible = score_at(Some(7_000_000_000));
    let tiny = score_at(Some(200 * 1024 * 1024));
    assert_ne!(
        plausible, size_less,
        "the announced size has to reach the score, or a queued release is \
         compared on different terms than the candidate beside it"
    );
    assert!(
        plausible > tiny,
        "a plausibly sized queued release must out-score a tiny one \
         ({plausible} vs {tiny})"
    );

    // …and the parity that matters at the gate: the *same release* announced a
    // little larger is not an upgrade over the copy already downloading.
    use crate::admission::{
        AdmissionPolicy, AdmissionScope, AdmissionSubject, CandidateFacts, QueuedRelease,
        evaluate_admission,
    };
    let queued_facts = crate::quality::canonical_context::score_parked_release_title(
        &title,
        release,
        Some(7_000_000_000),
        &[],
        &[],
        &context,
    );
    let candidate_facts = crate::quality::canonical_context::score_parked_release_title(
        &title,
        release,
        Some(9_000_000_000),
        &[],
        &[],
        &context,
    );
    let subject =
        AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![QueuedRelease {
            title: release.to_string(),
            covers: Vec::new(),
            tier_index: queued_facts.tier_index,
            revision: queued_facts.revision,
            score: queued_facts.score,
        }]);
    let grab_policy = AdmissionPolicy {
        allow_upgrades: true,
        min_delta: 200,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: true,
    };

    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(
            candidate_facts.tier_index,
            candidate_facts.revision,
            candidate_facts.score,
        ),
        &grab_policy,
    );
    assert!(
        !verdict.is_admitted(),
        "the same release one size band up is not worth a second download \
         (queued {} vs candidate {}): {verdict:?}",
        queued_facts.score,
        candidate_facts.score
    );

    // A genuinely better tier still gets through.
    let better_tier = crate::quality::canonical_context::score_parked_release_title(
        &title,
        "Sized.Queue.2024.2160p.WEB-DL.H.265-GRP",
        Some(20_000_000_000),
        &[],
        &[],
        &context,
    );
    assert!(
        evaluate_admission(
            &subject,
            CandidateFacts::new(
                better_tier.tier_index,
                better_tier.revision,
                better_tier.score
            ),
            &grab_policy,
        )
        .is_admitted(),
        "a better tier is not a duplicate"
    );
}

/// **Final review M2.** A queued release the *current* profile vetoes keeps its
/// honest, block-free score on the ladder — like an incumbent's bar (I5) — so an
/// equal candidate is still refused instead of the veto's −10 000 quietly
/// switching the queue gate off and fetching a duplicate.
#[tokio::test]
async fn a_queued_release_the_profile_now_vetoes_still_holds_its_scope() {
    use crate::admission::{
        AdmissionPolicy, AdmissionScope, AdmissionSubject, CandidateFacts, QueuedRelease,
        evaluate_admission,
    };

    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Vetoed Queue".into(),
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

    // The profile blocks H.264 now — edited after the release was grabbed.
    let profile = crate::QualityProfile::parse(
        r#"{"id":"vq","name":"VQ","criteria":{"quality_tiers":["2160P","1080P"],"allow_upgrades":true,"video_codec_blocklist":["H.264"]}}"#,
    )
    .expect("profile fixture parses");
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let release = "Vetoed.Queue.2024.1080p.WEB-DL.H.264-GRP";
    let facts = crate::quality::canonical_context::score_parked_release_title(
        &title,
        release,
        Some(7_000_000_000),
        &[],
        &[],
        &context,
    );
    assert!(
        !facts.allowed,
        "fixture precondition: the current profile vetoes the release"
    );
    assert!(
        facts.score > crate::quality_profile::BLOCK_SCORE / 2,
        "the queued score must be block-free, got {}",
        facts.score
    );

    let subject =
        AdmissionSubject::new(AdmissionScope::Title, []).with_queued(vec![QueuedRelease {
            title: release.to_string(),
            covers: Vec::new(),
            tier_index: facts.tier_index,
            revision: facts.revision,
            score: facts.score,
        }]);
    let policy = AdmissionPolicy {
        allow_upgrades: true,
        min_delta: 200,
        cutoff_score: None,
        manual_override: false,
        applies_to_queue: true,
    };
    let verdict = evaluate_admission(
        &subject,
        CandidateFacts::new(facts.tier_index, facts.revision, facts.score),
        &policy,
    );
    assert!(
        !verdict.is_admitted(),
        "an equal release must not be fetched twice because the queued copy is vetoed: {verdict:?}"
    );
}

/// **N2.** A file's role is per episode: it can be the primary occupant of one
/// episode it covers and merely an additional copy of another. The landed bar
/// belongs to the scope it is primary for, and to no other.
#[tokio::test]
async fn a_landed_bar_follows_the_per_episode_role_not_the_row_order() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p1080""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p1080", "1080p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Split Role".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("2".into()),
        )
        .await
        .expect("create collection");
    let mut episode_ids = Vec::new();
    for number in 1..=2 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some((Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
                Some(1_440),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    // One file covering both episodes: **additional** for E01 (some other file
    // holds it) and primary for E02. The additional row is seeded first, so a
    // first-row read of the role would call the whole file additional.
    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Split Role/Season 01/Split Role - S01E01-E02.mkv".to_string(),
            size_bytes: 2_700_000_000,
            quality_label: Some("1080p".to_string()),
            role: crate::MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode_ids[0])
        .await
        .expect("link first episode");
    {
        let mut store = media_files.store.lock().await;
        let mut primary_row = store
            .iter()
            .find(|entry| entry.id == file_id)
            .cloned()
            .expect("seeded file");
        primary_row.episode_id = Some(episode_ids[1].clone());
        primary_row.role = crate::MediaFileRole::Primary;
        store.push(primary_row);
    }

    let bars = app
        .landed_bars_for_scopes(&[
            crate::acquisition_workflow::LandedBarScope {
                title_id: title.id.clone(),
                episode_id: Some(episode_ids[0].clone()),
                collection_id: None,
                series_movie_link_id: None,
            },
            crate::acquisition_workflow::LandedBarScope {
                title_id: title.id.clone(),
                episode_id: Some(episode_ids[1].clone()),
                collection_id: None,
                series_movie_link_id: None,
            },
        ])
        .await;

    assert_eq!(
        bars[0], None,
        "E01 has no primary file, so it has no bar and is still a target"
    );
    assert!(
        bars[1].is_some(),
        "E02 is held by this file as its primary, so it has a bar"
    );
}

/// **MA2.** A season with one missing episode has not reached cutoff, whatever
/// the anchor row's `grabbed_release` says.
///
/// `analyzed_cutoff_quality_for_scope` answers `None` the moment one member is
/// empty — "a season has reached cutoff only when all of it has". That `None`
/// used to fall through to parsing the *anchor* row's grabbed release, so a
/// season whose E01 had been grabbed at 1080p reported cutoff reached, the pack
/// lane returned, and the pack that would have filled the missing episode was
/// never evaluated.
#[tokio::test]
async fn a_season_with_a_missing_episode_has_not_reached_cutoff() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p1080""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p1080", "1080p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Nearly Complete".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("3".into()),
        )
        .await
        .expect("create collection");

    let mut episode_ids = Vec::new();
    for number in 1..=3 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some((Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    // E01 and E02 are at cutoff; E03 was never grabbed.
    for (index, episode_id) in episode_ids.iter().take(2).enumerate() {
        let file_id = media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!(
                    "/library/Nearly Complete/Season 01/Nearly Complete - S01E0{}.mkv",
                    index + 1
                ),
                size_bytes: 1_500_000_000,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        media_files
            .link_file_to_episode(&file_id, episode_id)
            .await
            .expect("link media file to episode");
    }

    let scope = crate::SubmissionScope::Collection {
        collection_id: collection.id.clone(),
    };
    let existing_files = media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let cutoff_scope = app.cutoff_scope_for(&scope).await;
    let analyzed = crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
        &existing_files,
        &cutoff_scope,
    );
    assert_eq!(
        analyzed, None,
        "a season with an empty member has no cutoff quality"
    );

    let context = app
        .resolve_upgrade_context_for_title_with_category_and_quality(&title, None, analyzed)
        .await
        .expect("resolve upgrade context");
    assert!(
        !context.cutoff_reached,
        "the pack that would fill the missing episode must still be evaluated"
    );

    // Fill the last member and the season really has reached cutoff.
    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Nearly Complete/Season 01/Nearly Complete - S01E03.mkv"
                .to_string(),
            size_bytes: 1_500_000_000,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode_ids[2])
        .await
        .expect("link media file to episode");
    let existing_files = media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let analyzed = crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
        &existing_files,
        &cutoff_scope,
    );
    let context = app
        .resolve_upgrade_context_for_title_with_category_and_quality(&title, None, analyzed)
        .await
        .expect("resolve upgrade context");
    assert!(context.cutoff_reached, "every member is at cutoff");
}

/// **BL1 / D8 as amended.** A batch is a pack at grab: judged per member, so
/// four missing episodes are reason enough to fetch it even though the fifth
/// already holds a better file.
///
/// This is the shape the span-scoped subject got wrong. `evaluate_admission`'s
/// span loop sees the E05 incumbent, finds a better tier, and refuses
/// `LowerQualityTier` — on every RSS cycle, forever — because `has_missing_member`
/// only exists inside `evaluate_any_member`.
#[tokio::test]
async fn a_batch_that_fills_missing_episodes_is_admitted_over_a_better_member() {
    use crate::admission::{AdmissionPolicy, CandidateFacts, evaluate_admission};

    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user) = bootstrap();
    let app = app.with_test_overrides(|services| services.with_media_files(media_files.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Batch Filler".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("5".into()),
        )
        .await
        .expect("create collection");

    let mut episode_ids = Vec::new();
    for number in 1..=5 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some((Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    // Only E05 is filled, and by a better tier than the batch.
    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Batch Filler/Season 01/Batch Filler - S01E05.mkv".to_string(),
            size_bytes: 4 * 1024 * 1024 * 1024,
            quality_label: Some("2160p".to_string()),
            scene_name: Some("Batch.Filler.S01E05.2160p.WEB-DL.H.265-GRP".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode_ids[4])
        .await
        .expect("link media file to episode");

    let profile = test_quality_profile("batch-filler");
    let scoring_context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let batch_scope = crate::SubmissionScope::EpisodeSet {
        episode_ids: episode_ids.clone(),
    };
    // A 1080p batch: worse than the 2160p file holding E05, better than nothing
    // for E01–E04.
    let candidate = CandidateFacts::new(Some(1), 0, 900);

    let grab_subject = app
        .admission_subject_for_scope(
            &title,
            &batch_scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        !grab_subject.is_unoccupied(),
        "the fixture must actually seed the E05 incumbent, or the test cannot go red"
    );
    let verdict = evaluate_admission(
        &grab_subject,
        candidate,
        &AdmissionPolicy::not_a_downgrade(),
    );
    assert!(
        verdict.is_admitted(),
        "a batch filling four missing episodes must be fetched: {verdict:?}"
    );
    assert!(
        verdict.superseded().is_empty(),
        "the 2160p member is not improvable by a 1080p batch, so nothing is displaced: {verdict:?}"
    );

    // …and the *import* side of the same scope keeps span semantics: one file
    // covering E01-E05 has to beat everything it displaces, or E05 is silently
    // downgraded.
    let import_subject = app
        .admission_subject_for_scope(
            &title,
            &batch_scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Import,
        )
        .await;
    let verdict = evaluate_admission(
        &import_subject,
        candidate,
        &AdmissionPolicy::not_a_downgrade(),
    );
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(crate::admission::AdmissionRejectionReason::LowerQualityTier)
        ),
        "one landed file spanning E01-E05 must not downgrade E05: {verdict:?}"
    );
}

/// A batch reaching into an episode that has not aired is refused exactly like a
/// mid-season pack — counted over the batch's **own** members, not the season's.
#[tokio::test]
async fn a_batch_covering_an_unaired_episode_is_refused() {
    use crate::admission::{AdmissionPolicy, CandidateFacts, evaluate_admission};

    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Batch Too Early".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("3".into()),
        )
        .await
        .expect("create collection");

    let mut episode_ids = Vec::new();
    for (number, air_date) in [
        ("1", (Utc::now() - chrono::Duration::days(14)).to_rfc3339()),
        ("2", (Utc::now() - chrono::Duration::days(7)).to_rfc3339()),
        ("3", (Utc::now() + chrono::Duration::days(7)).to_rfc3339()),
    ] {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some(air_date),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    let profile = test_quality_profile("batch-too-early");
    let scoring_context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let candidate = CandidateFacts::new(Some(0), 0, 900);
    let policy = AdmissionPolicy::not_a_downgrade();

    // E01-E03: the finale has not aired, so no batch can contain it.
    let reaching = app
        .admission_subject_for_scope(
            &title,
            &crate::SubmissionScope::EpisodeSet {
                episode_ids: episode_ids.clone(),
            },
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    let verdict = evaluate_admission(&reaching, candidate, &policy);
    assert!(
        matches!(
            verdict.rejection().map(|rejection| &rejection.reason),
            Some(crate::admission::AdmissionRejectionReason::SeasonIncomplete)
        ),
        "a batch claiming an unaired episode is a guaranteed partial fetch: {verdict:?}"
    );

    // E01-E02 covers only what exists, and the season's finale is not its
    // business.
    let aired_only = app
        .admission_subject_for_scope(
            &title,
            &crate::SubmissionScope::EpisodeSet {
                episode_ids: episode_ids[..2].to_vec(),
            },
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(evaluate_admission(&aired_only, candidate, &policy).is_admitted());
}

/// **B2 at the subject level.** A partial multi-episode batch and a full-season
/// pack are different scopes, and only the second one answers to the season's
/// air schedule.
///
/// The RSS lane routes both through the same function; giving the batch the
/// season's Collection scope refused `Show - 01-02` for a currently-airing
/// season, and kept refusing every batch for the season's whole run. Sonarr's
/// `FullSeasonSpecification` only applies to a release parsed as a full season.
#[tokio::test]
async fn a_partial_batch_is_admitted_while_the_season_is_still_airing() {
    use crate::admission::{AdmissionPolicy, CandidateFacts, evaluate_admission};

    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Airing Season".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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
            Some("3".into()),
        )
        .await
        .expect("create collection");

    // Two aired, one airing in a week: the season is mid-run.
    let mut episode_ids = Vec::new();
    for (number, air_date) in [
        ("1", (Utc::now() - chrono::Duration::days(14)).to_rfc3339()),
        ("2", (Utc::now() - chrono::Duration::days(7)).to_rfc3339()),
        ("3", (Utc::now() + chrono::Duration::days(7)).to_rfc3339()),
    ] {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some(air_date),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    let profile = test_quality_profile("airing-season");
    let scoring_context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let policy = AdmissionPolicy::not_a_downgrade();
    let candidate = CandidateFacts::new(Some(0), 0, 900);

    // The batch covers only the aired episodes; the season's schedule is not
    // its business.
    let batch_scope = crate::SubmissionScope::EpisodeSet {
        episode_ids: episode_ids[..2].to_vec(),
    };
    let batch_subject = app
        .admission_subject_for_scope(
            &title,
            &batch_scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        evaluate_admission(&batch_subject, candidate, &policy).is_admitted(),
        "a partial batch must not be refused for the season's unaired members"
    );

    // The full-season pack is refused while a member has not aired.
    let season_scope = crate::SubmissionScope::Collection {
        collection_id: collection.id.clone(),
    };
    let season_subject = app
        .admission_subject_for_scope(
            &title,
            &season_scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        !evaluate_admission(&season_subject, candidate, &policy).is_admitted(),
        "a full-season pack cannot be complete while the season is airing"
    );

    // Once the finale has aired, the same pack is admitted.
    app.update_episode(
        &user,
        episode_ids[2].clone(),
        None,
        None,
        None,
        None,
        None,
        Some((Utc::now() - chrono::Duration::days(1)).to_rfc3339()),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("air the finale");
    let finished_subject = app
        .admission_subject_for_scope(
            &title,
            &season_scope,
            &scoring_context,
            None,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await;
    assert!(
        evaluate_admission(&finished_subject, candidate, &policy).is_admitted(),
        "a finished season's pack is a normal pack"
    );
}
