use super::*;

#[tokio::test]
async fn create_collection_and_episode() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "The Odes".into(),
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

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let episodes = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
}

#[tokio::test]
async fn series_hydration_persists_and_clears_episode_image_url() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Still Frames".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "880088".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 880_001,
        number: 1,
        label: "Season 1".into(),
        episode_type: "official".into(),
    }];
    let mut episodes = vec![EpisodeMetadata {
        tvdb_id: 880_101,
        episode_number: 1,
        name: "A Still Frame".into(),
        aired: "2026-01-01".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: "A frame is captured.".into(),
        absolute_number: "1".into(),
        season_number: 1,
        image_url: " https://image.tmdb.org/t/p/original/still-a.jpg ".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let collection = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections")
        .into_iter()
        .next()
        .expect("collection created");
    let hydrated = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");
    assert_eq!(
        hydrated[0].image_url.as_deref(),
        Some("https://image.tmdb.org/t/p/original/still-a.jpg")
    );

    episodes[0].image_url = "https://image.tmdb.org/t/p/original/still-b.jpg".into();
    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let updated = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes after image update");
    assert_eq!(
        updated[0].image_url.as_deref(),
        Some("https://image.tmdb.org/t/p/original/still-b.jpg")
    );

    episodes[0].image_url = "not-a-url".into();
    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let cleared = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes after image clear");
    assert_eq!(cleared[0].image_url, None);
}

#[tokio::test]
async fn anime_hybrid_movie_mapping_creates_series_movie_link() {
    let (app, user) = bootstrap();
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                131_963,
                MovieMetadata {
                    target_key: None,
                    smg_id: None,
                    primary_source: "tvdb".into(),
                    tvdb_id: Some(131_963),
                    name: "Iron Rail".into(),
                    slug: "iron-rail".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "A train mission.".into(),
                    poster_url: "https://example.com/iron-rail.jpg".into(),
                    background_url: None,
                    language: "eng".into(),
                    original_language: Some("jpn".into()),
                    runtime_minutes: 117,
                    sort_title: "Iron Rail".into(),
                    imdb_id: "tt11032374".into(),
                    tmdb_id: None,
                    popularity: None,
                    anidb_id: None,
                    canonical_tags: vec![],
                    studio: "ufotable".into(),
                    tmdb_release_date: Some("2020-10-16".into()),
                    ratings: Default::default(),
                    credits: Vec::new(),
                    ..Default::default()
                },
            )]),
        }))
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blade Summit".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "348545".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 1001,
            episode_number: 1,
            name: "Cruelty".into(),
            aired: "2019-04-06".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 1002,
            episode_number: 26,
            name: "New Mission".into(),
            aired: "2019-09-28".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 26".into(),
            absolute_number: "26".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 2001,
            episode_number: 1,
            name: "Iron Rail".into(),
            aired: "2020-10-10".into(),
            runtime_minutes: 117,
            is_filler: false,
            is_recap: false,
            overview: "Special cut".into(),
            absolute_number: String::new(),
            season_number: 0,
            image_url: String::new(),
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(40456),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(348545),
        themoviedb_id: Some(438759),
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: Some(131_963),
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];
    let anime_movies = vec![AnimeMovie {
        movie_tvdb_id: Some(131_963),
        movie_tmdb_id: Some(438759),
        movie_imdb_id: Some("tt11032374".into()),
        movie_mal_id: Some(40456),
        movie_anidb_id: None,
        name: "Iron Rail".into(),
        slug: "iron-rail".into(),
        year: Some(2020),
        content_status: "released".into(),
        overview: "Blade Summit: Ember Rail".into(),
        poster_url: "poster".into(),
        language: "eng".into(),
        runtime_minutes: 117,
        sort_title: "Iron Rail".into(),
        imdb_id: "tt11032374".into(),
        studio: "ufotable".into(),
        digital_release_date: Some("2020-10-16".into()),
        association_confidence: "high".into(),
        continuity_status: "canon".into(),
        movie_form: "movie".into(),
        placement: "ordered".into(),
        confidence: "high".into(),
        signal_summary: "TVDB marked special as critical to story".into(),
    }];

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        &anime_movies,
    )
    .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    assert!(
        collections
            .iter()
            .all(|collection| collection.label.as_deref() != Some("Iron Rail"))
    );
    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.movie.title, "Iron Rail");
    assert_eq!(link.movie.tvdb_id.as_deref(), Some("131963"));
    assert_eq!(link.movie.imdb_id.as_deref(), Some("tt11032374"));
    assert_eq!(link.continuity_status.as_deref(), Some("canon"));
    assert_eq!(link.association_confidence.as_deref(), Some("high"));

    let specials = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Specials)
        .expect("specials collection should exist");
    let specials_episodes = app
        .list_episodes(&user, &specials.id)
        .await
        .expect("list specials episodes");
    assert_eq!(specials_episodes.len(), 1);
    assert_eq!(specials_episodes[0].title.as_deref(), Some("Iron Rail"));
    assert_eq!(
        link.linked_episode_id.as_deref(),
        Some(specials_episodes[0].id.as_str())
    );
}

#[tokio::test]
async fn series_season_zero_creates_canonical_specials_collection() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Halted Ambitions".into(),
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

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 80,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 81,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 8001,
            episode_number: 1,
            name: "Special Episode".into(),
            aired: "2003-11-01".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: "Special".into(),
            absolute_number: String::new(),
            season_number: 0,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 8101,
            episode_number: 1,
            name: "Pilot".into(),
            aired: "2003-11-02".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let specials = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Specials
                || (collection.collection_type == CollectionType::Season
                    && collection.collection_index == "0")
        })
        .expect("specials collection should exist");
    assert_eq!(specials.collection_type, CollectionType::Specials);
    assert_eq!(specials.collection_index, "0");
    assert!(!specials.monitored);
}

#[tokio::test]
async fn new_regular_season_without_episodes_is_monitored_when_title_is_monitored() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Future Season Show".into(),
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

    let seasons = vec![SeasonMetadata {
        tvdb_id: 92,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert!(
        season.monitored,
        "new regular seasons should auto-monitor for monitored titles even before episodes exist"
    );
}

#[tokio::test]
async fn new_regular_season_without_episodes_is_not_monitored_when_monitor_type_is_none() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Season Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec!["scryer:monitor-type:none".into()],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 93,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert!(
        !season.monitored,
        "monitor-type:none should keep new empty regular seasons unmonitored"
    );
}

#[tokio::test]
async fn rehydrating_existing_regular_season_preserves_manual_unmonitored_state() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Existing Season Show".into(),
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

    let existing_collection = app
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
            first_episode_number: None,
            last_episode_number: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("seed existing season collection");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 94,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert_eq!(season.id, existing_collection.id);
    assert!(
        !season.monitored,
        "rehydration should not retroactively flip existing manually unmonitored seasons"
    );
}

#[tokio::test]
async fn series_rollout_reuses_legacy_season_zero_specials_collection() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Legacy Specials Show".into(),
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

    let legacy_specials = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "0".into(),
            Some("Season 0".into()),
            None,
            None,
            None,
        )
        .await
        .expect("create legacy season zero collection");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 90,
        number: 0,
        label: "Specials".into(),
        episode_type: "special".into(),
    }];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 9001,
        episode_number: 1,
        name: "Pilot Special".into(),
        aired: "2004-01-01".into(),
        runtime_minutes: 22,
        is_filler: false,
        is_recap: false,
        overview: "Legacy special".into(),
        absolute_number: String::new(),
        season_number: 0,
        image_url: String::new(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let logical_specials: Vec<&Collection> = collections
        .iter()
        .filter(|collection| {
            collection.collection_type == CollectionType::Specials
                || (collection.collection_type == CollectionType::Season
                    && collection.collection_index == "0")
        })
        .collect();
    assert_eq!(logical_specials.len(), 1);
    assert_eq!(logical_specials[0].id, legacy_specials.id);
    assert_eq!(logical_specials[0].collection_type, CollectionType::Season);

    let episodes = app
        .list_episodes(&user, &legacy_specials.id)
        .await
        .expect("list legacy season zero episodes");
    assert_eq!(episodes.len(), 1);
}

#[tokio::test]
async fn anime_mapping_without_movie_link_does_not_create_series_movie_link() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Given".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "361218".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 20,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 21,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 3001,
            episode_number: 1,
            name: "Kids in the Chorus".into(),
            aired: "2019-07-12".into(),
            runtime_minutes: 23,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 3002,
            episode_number: 1,
            name: "OVA".into(),
            aired: "2020-02-01".into(),
            runtime_minutes: 23,
            is_filler: false,
            is_recap: false,
            overview: "Special".into(),
            absolute_number: String::new(),
            season_number: 0,
            image_url: String::new(),
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(40421),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(361218),
        themoviedb_id: None,
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: None,
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &anime_mappings, &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    assert!(
        collections
            .iter()
            .all(|collection| collection.collection_type != CollectionType::Movie)
    );
    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert!(links.is_empty(), "unexpected series movie link created");
}

#[tokio::test]
async fn anime_hydration_persists_scoped_anibridge_ids_for_episode_and_full_season() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "The Apiary Almanac".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "431162".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 4_311_622,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];
    let episodes = (1..=24)
        .map(|episode_number| EpisodeMetadata {
            tvdb_id: 431_162_200 + i64::from(episode_number),
            episode_number,
            name: format!("Episode {episode_number}"),
            aired: "2025-01-10".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: String::new(),
            absolute_number: episode_number.to_string(),
            season_number: 2,
            image_url: String::new(),
        })
        .collect::<Vec<_>>();
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(58514),
        mal_dub_id: Some(999_58514),
        anilist_id: Some(176301),
        anidb_id: Some(18562),
        kitsu_id: Some(48924),
        simkl_id: Some(231_001),
        thetvdb_id: Some(431162),
        themoviedb_id: Some(156_067),
        imdb_id: Some(2_024_544),
        trakt_id: Some(314_159),
        alt_tvdb_id: None,
        thetvdb_season: Some(2),
        thetvdb_part: Some(1),
        score: Some(1.0),
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "current".into(),
        mapping_type: "R".into(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 2,
            episode_start: 1,
            episode_end: 24,
        }],
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &anime_mappings, &[])
        .await;

    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    let season_two = collections
        .iter()
        .find(|collection| collection.collection_index == "2")
        .expect("season two collection");
    let collection_ids = app
        .services
        .catalog
        .shows
        .list_collection_external_ids(&season_two.id)
        .await
        .expect("list collection external ids");
    assert!(
        collection_ids.iter().any(|id| {
            id.source == "anilist"
                && id.external_id == "176301"
                && id.source_scope.as_deref() == Some("R")
        }),
        "expected full-season scoped AniList ID"
    );
    assert!(
        collection_ids
            .iter()
            .any(|id| id.source == "simkl" && id.external_id == "231001"),
        "expected all available AniBridge ID sources to be persisted"
    );

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    let episode_23 = episodes
        .iter()
        .find(|episode| {
            episode.season_number.as_deref() == Some("2")
                && episode.episode_number.as_deref() == Some("23")
        })
        .expect("season two episode 23");
    let episode_ids = app
        .services
        .catalog
        .shows
        .list_episode_external_ids(&episode_23.id)
        .await
        .expect("list episode external ids");
    assert!(
        episode_ids.iter().any(|id| {
            id.source == "anilist"
                && id.external_id == "176301"
                && id.source_scope.as_deref() == Some("R")
        }),
        "expected episode-scoped AniList ID"
    );
}

#[tokio::test]
async fn anime_movies_create_series_movie_links_without_collection_metadata() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stoneguard".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec!["scryer:monitor-specials:false".into()],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "267440".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 50,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 51,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
        SeasonMetadata {
            tvdb_id: 52,
            number: 2,
            label: "Season 2".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 5001,
            episode_number: 1,
            name: "To You, in 2000 Winters".into(),
            aired: "2013-04-07".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 6001,
            episode_number: 1,
            name: "Iron Colossus".into(),
            aired: "2017-04-01".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "26".into(),
            season_number: 2,
            image_url: String::new(),
        },
    ];

    let anime_movies = vec![
        AnimeMovie {
            movie_tvdb_id: Some(379088),
            movie_tmdb_id: Some(379088),
            movie_imdb_id: Some("tt3865768".into()),
            movie_mal_id: Some(23775),
            movie_anidb_id: None,
            name: "Stoneguard: Amber Bow and Quiver".into(),
            slug: "amber-bow-and-quiver".into(),
            year: Some(2014),
            content_status: "released".into(),
            overview: "Recap of episodes 1-13.".into(),
            poster_url: "poster-stoneguard".into(),
            language: "eng".into(),
            runtime_minutes: 120,
            sort_title: "Amber Bow and Quiver".into(),
            imdb_id: "tt3865768".into(),
            studio: "WIT Studio".into(),
            digital_release_date: Some("2014-11-22".into()),
            association_confidence: "high".into(),
            continuity_status: "unknown".into(),
            movie_form: "recap".into(),
            placement: "specials".into(),
            confidence: "high".into(),
            signal_summary: "TVDB special category marks this as a recap".into(),
        },
        AnimeMovie {
            movie_tvdb_id: Some(131963),
            movie_tmdb_id: Some(438759),
            movie_imdb_id: Some("tt11032374".into()),
            movie_mal_id: Some(40456),
            movie_anidb_id: None,
            name: "Iron Rail".into(),
            slug: "iron-rail".into(),
            year: Some(2020),
            content_status: "released".into(),
            overview: "Canon bridge movie".into(),
            poster_url: "poster-ds".into(),
            language: "eng".into(),
            runtime_minutes: 117,
            sort_title: "Iron Rail".into(),
            imdb_id: "tt11032374".into(),
            studio: "ufotable".into(),
            digital_release_date: Some("2020-10-16".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "TVDB marked special as critical to story".into(),
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &anime_movies)
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let specials = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Specials)
        .expect("specials collection should exist");
    assert!(!specials.monitored);
    assert!(
        collections
            .iter()
            .all(|collection| collection.collection_type != CollectionType::Movie)
    );
    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(links.len(), 2);
    let recap = links
        .iter()
        .find(|link| link.movie.title == "Stoneguard: Amber Bow and Quiver")
        .expect("recap movie link");
    assert_eq!(recap.movie_form.as_deref(), Some("recap"));
    assert!(recap.linked_episode_id.is_none());
    assert!(
        !recap.monitored,
        "recaps require an explicit operator choice"
    );
    let explicitly_disabled_recap = app
        .set_series_movie_monitored(&user, &recap.id, false)
        .await
        .expect("record explicit disabled choice");
    assert_eq!(explicitly_disabled_recap.monitoring_override, Some(false));
    let ordered = links
        .iter()
        .find(|link| link.movie.title == "Iron Rail")
        .expect("ordered movie link");
    assert_eq!(ordered.continuity_status.as_deref(), Some("canon"));
    assert!(ordered.linked_episode_id.is_none());
    assert!(
        !ordered.monitored,
        "derived links stay unmonitored until the title explicitly uses All or Missing"
    );

    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:allepisodes".into()]),
    )
    .await
    .expect("set explicit all monitor mode");
    let all_links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list policy-selected links");
    let all_ordered = all_links
        .iter()
        .find(|link| link.movie.title == "Iron Rail")
        .expect("ordered movie link");
    let all_recap = all_links
        .iter()
        .find(|link| link.movie.title == "Stoneguard: Amber Bow and Quiver")
        .expect("recap movie link");
    assert!(all_ordered.monitored);
    assert!(!all_recap.monitored);

    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:missingandfutureepisodes".into()]),
    )
    .await
    .expect("set explicit missing monitor mode");
    let missing_links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list missing-policy links");
    let missing_ordered = missing_links
        .iter()
        .find(|link| link.movie.title == "Iron Rail")
        .expect("ordered movie link");
    assert!(missing_ordered.monitored);

    app.set_series_movie_monitored(&user, &missing_ordered.id, false)
        .await
        .expect("explicitly disable canonical movie");
    app.update_title_metadata(
        &user,
        &title.id,
        None,
        None,
        Some(vec!["scryer:monitor-type:allepisodes".into()]),
    )
    .await
    .expect("refresh explicit all monitor mode");
    let overridden = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list overridden links")
        .into_iter()
        .find(|link| link.id == missing_ordered.id)
        .expect("canonical movie link");
    assert_eq!(overridden.monitoring_override, Some(false));
    assert!(!overridden.monitored);
}

#[tokio::test]
async fn anime_series_movie_refresh_updates_localized_movie_entity_metadata() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Vanguard Academy".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 1001,
            episode_number: 1,
            name: "Episode 1".into(),
            aired: "2018-04-03".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 2001,
            episode_number: 1,
            name: "Twin Sentinels".into(),
            aired: "2018-08-03".into(),
            runtime_minutes: 96,
            is_filler: false,
            is_recap: false,
            overview: "Movie special".into(),
            absolute_number: String::new(),
            season_number: 0,
            image_url: String::new(),
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(36665),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(305074),
        themoviedb_id: Some(505262),
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: Some(149921),
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];

    let japanese_movie = AnimeMovie {
        movie_tvdb_id: Some(149921),
        movie_tmdb_id: Some(505262),
        movie_imdb_id: Some("tt5626028".into()),
        movie_mal_id: Some(36665),
        movie_anidb_id: None,
        name: "星界学園 THE MOVIE ～二人の英雄～".into(),
        slug: "my-hero-academia-the-movie-two-heroes".into(),
        year: Some(2018),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 96,
        sort_title: "星界学園 THE MOVIE ～二人の英雄～".into(),
        imdb_id: "tt5626028".into(),
        studio: "Bones".into(),
        digital_release_date: Some("2018-08-03".into()),
        association_confidence: "high".into(),
        continuity_status: "canon".into(),
        movie_form: "movie".into(),
        placement: "ordered".into(),
        confidence: "high".into(),
        signal_summary: "TVDB special linked to movie".into(),
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        std::slice::from_ref(&japanese_movie),
    )
    .await;

    let english_movie = AnimeMovie {
        name: "Vanguard Academy: Twin Sentinels".into(),
        overview: "English overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "Vanguard Academy: Twin Sentinels".into(),
        ..japanese_movie.clone()
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        std::slice::from_ref(&english_movie),
    )
    .await;

    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(links.len(), 1);
    let link = &links[0];

    assert_eq!(
        link.movie.title.as_str(),
        "Vanguard Academy: Twin Sentinels"
    );
    assert_eq!(link.movie.overview.as_deref(), Some("English overview"));
    assert_eq!(link.movie.language.as_deref(), Some("eng"));
}

#[tokio::test]
async fn anime_specials_refresh_updates_localized_series_movie_metadata() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stoneguard".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 1001,
        episode_number: 1,
        name: "Episode 1".into(),
        aired: "2013-04-07".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: "Episode 1".into(),
        absolute_number: "1".into(),
        season_number: 1,
        image_url: String::new(),
    }];

    let japanese_special = AnimeMovie {
        movie_tvdb_id: Some(379088),
        movie_tmdb_id: Some(379088),
        movie_imdb_id: Some("tt3865768".into()),
        movie_mal_id: Some(23775),
        movie_anidb_id: None,
        name: "石衛 前編～紅蓮の弓矢～".into(),
        slug: "amber-bow-and-quiver".into(),
        year: Some(2014),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 120,
        sort_title: "石衛 前編～紅蓮の弓矢～".into(),
        imdb_id: "tt3865768".into(),
        studio: "WIT Studio".into(),
        digital_release_date: Some("2014-11-22".into()),
        association_confidence: "high".into(),
        continuity_status: "unknown".into(),
        movie_form: "recap".into(),
        placement: "specials".into(),
        confidence: "high".into(),
        signal_summary: "TVDB special category marks this as a recap".into(),
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &[],
        std::slice::from_ref(&japanese_special),
    )
    .await;

    let english_special = AnimeMovie {
        name: "Stoneguard: Amber Bow and Quiver".into(),
        overview: "English recap overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "Stoneguard: Amber Bow and Quiver".into(),
        ..japanese_special.clone()
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &[],
        std::slice::from_ref(&english_special),
    )
    .await;

    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(links.len(), 1);
    let link = &links[0];

    assert_eq!(link.movie.title, "Stoneguard: Amber Bow and Quiver");
    assert_eq!(
        link.movie.overview.as_deref(),
        Some("English recap overview")
    );
    assert_eq!(link.movie.language.as_deref(), Some("eng"));
}

#[tokio::test]
async fn read_collection_by_id_returns_item() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Read Collection".into(),
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

    let found = app
        .get_collection(&user, &collection.id)
        .await
        .expect("get collection")
        .expect("found collection");

    assert_eq!(found.id, collection.id);
    assert_eq!(found.collection_index, collection.collection_index);
}

#[tokio::test]
async fn read_episode_by_id_returns_item() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Read Episode".into(),
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

    let found = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("found episode");

    assert_eq!(found.id, episode.id);
    assert_eq!(found.episode_number, episode.episode_number);
}

#[tokio::test]
async fn delete_collection_removes_collection_entry() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Collection Delete".into(),
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

    app.delete_collection(&user, &collection.id)
        .await
        .expect("delete collection");

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    assert!(collections.is_empty());
}

#[tokio::test]
async fn delete_episode_removes_episode_entry() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Episode Delete".into(),
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

    app.delete_episode(&user, &episode.id)
        .await
        .expect("delete episode");

    let episodes = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");
    assert!(episodes.is_empty(), "expected episode to be deleted");
}

#[tokio::test]
async fn update_collection_changes_fields() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Update Collection".into(),
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
            Some("s1".into()),
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let updated = app
        .update_collection(
            &user,
            collection.id.clone(),
            Some("arc".into()),
            None,
            Some("Arc One".into()),
            Some("arc-one".into()),
            None,
            Some("13".into()),
            None,
        )
        .await
        .expect("update collection");

    assert_eq!(updated.collection_type, CollectionType::Arc);
    assert_eq!(updated.label, Some("Arc One".into()));
    assert_eq!(updated.ordered_path, Some("arc-one".into()));
    assert_eq!(updated.last_episode_number, Some("13".into()));
    assert_eq!(updated.collection_index, "1");
}

#[tokio::test]
async fn update_episode_changes_fields() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Update Episode".into(),
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

    let updated = app
        .update_episode(
            &user,
            episode.id.clone(),
            Some("special".into()),
            Some("E01".into()),
            None,
            None,
            Some("Pilot Updated".into()),
            Some("2026-01-01".into()),
            Some(1_800),
            Some(true),
            None,
            None,
            Some(collection.id.clone()),
            Some("Updated overview".into()),
        )
        .await
        .expect("update episode");

    assert_eq!(updated.episode_type, scryer_domain::EpisodeType::Special);
    assert_eq!(updated.episode_number, Some("E01".into()));
    assert_eq!(updated.title, Some("Pilot Updated".into()));
    assert_eq!(updated.air_date, Some("2026-01-01".into()));
    assert_eq!(updated.overview, Some("Updated overview".into()));
    assert_eq!(updated.duration_seconds, Some(1_800));
    assert!(updated.has_multi_audio);
    assert!(!updated.has_subtitle);
}

async fn advanced_title(
    app: &AppUseCase,
    user: &User,
    name: &str,
    facet: MediaFacet,
    selection: scryer_domain::MonitorSelection,
) -> Title {
    let library_id = scryer_domain::default_library_id_for_facet(&facet);
    app.add_title_with_options_patch_outcome_in_library(
        user,
        NewTitle {
            name: name.into(),
            facet,
            monitored: true,
            tags: vec!["scryer:monitor-type:advanced".into()],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
        library_id,
        TitleOptionsPatch {
            monitor_selection: Some(Some(selection)),
            ..TitleOptionsPatch::default()
        },
    )
    .await
    .expect("create advanced title")
    .title
}

#[tokio::test]
async fn advanced_monitoring_only_monitors_selected_seasons_and_their_episodes() {
    let (app, user) = bootstrap();
    let title = advanced_title(
        &app,
        &user,
        "Advanced Show",
        MediaFacet::Series,
        scryer_domain::MonitorSelection {
            seasons: vec![2],
            series_movies: vec![],
        },
    )
    .await;

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 101,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
        SeasonMetadata {
            tvdb_id: 102,
            number: 2,
            label: "Season 2".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 10101,
            episode_number: 1,
            name: "S1E1".into(),
            // Already aired: proves advanced ignores air-date policy entirely.
            aired: "2001-01-01".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: String::new(),
            absolute_number: "1".into(),
            season_number: 1,
            image_url: String::new(),
        },
        EpisodeMetadata {
            tvdb_id: 10201,
            episode_number: 1,
            name: "S2E1".into(),
            aired: "2002-01-01".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: String::new(),
            absolute_number: "2".into(),
            season_number: 2,
            image_url: String::new(),
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season_one = collections
        .iter()
        .find(|collection| collection.collection_index == "1")
        .expect("season 1 collection");
    let season_two = collections
        .iter()
        .find(|collection| collection.collection_index == "2")
        .expect("season 2 collection");
    assert!(!season_one.monitored, "unselected season stays unmonitored");
    assert!(season_two.monitored, "selected season is monitored");

    let season_one_episodes = app
        .list_episodes(&user, &season_one.id)
        .await
        .expect("list season 1 episodes");
    let season_two_episodes = app
        .list_episodes(&user, &season_two.id)
        .await
        .expect("list season 2 episodes");
    assert!(season_one_episodes.iter().all(|episode| !episode.monitored));
    assert!(
        !season_two_episodes.is_empty()
            && season_two_episodes.iter().all(|episode| episode.monitored)
    );
}

#[tokio::test]
async fn advanced_monitoring_monitors_only_the_selected_series_movies() {
    let (app, user) = bootstrap();
    let title = advanced_title(
        &app,
        &user,
        "Advanced Anime",
        MediaFacet::Anime,
        scryer_domain::MonitorSelection {
            seasons: vec![1],
            series_movies: vec![scryer_domain::MonitorSelectionMovie {
                name: "Iron Rail".into(),
                external_ids: vec![ExternalId {
                    source: "tvdb".into(),
                    value: "131963".into(),
                }],
            }],
        },
    )
    .await;

    let seasons = vec![SeasonMetadata {
        tvdb_id: 201,
        number: 1,
        label: "Season 1".into(),
        episode_type: "official".into(),
    }];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 20101,
        episode_number: 1,
        name: "S1E1".into(),
        aired: "2013-04-07".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: String::new(),
        absolute_number: "1".into(),
        season_number: 1,
        image_url: String::new(),
    }];
    let anime_movies = vec![
        AnimeMovie {
            movie_tvdb_id: Some(131963),
            movie_tmdb_id: Some(438759),
            movie_imdb_id: Some("tt11032374".into()),
            movie_mal_id: Some(40456),
            movie_anidb_id: None,
            name: "Iron Rail".into(),
            slug: "iron-rail".into(),
            year: Some(2020),
            content_status: "released".into(),
            overview: "Canon bridge movie".into(),
            poster_url: "poster-ds".into(),
            language: "eng".into(),
            runtime_minutes: 117,
            sort_title: "Iron Rail".into(),
            imdb_id: "tt11032374".into(),
            studio: "ufotable".into(),
            digital_release_date: Some("2020-10-16".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "canon".into(),
        },
        AnimeMovie {
            movie_tvdb_id: Some(222222),
            movie_tmdb_id: Some(222222),
            movie_imdb_id: Some("tt2222222".into()),
            movie_mal_id: Some(2222),
            movie_anidb_id: None,
            name: "Unselected Canon Movie".into(),
            slug: "unselected".into(),
            year: Some(2021),
            content_status: "released".into(),
            overview: "Also canon, but not picked".into(),
            poster_url: "poster-unselected".into(),
            language: "eng".into(),
            runtime_minutes: 100,
            sort_title: "Unselected Canon Movie".into(),
            imdb_id: "tt2222222".into(),
            studio: "ufotable".into(),
            digital_release_date: Some("2021-10-16".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "canon".into(),
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &anime_movies)
        .await;

    let links = app
        .list_series_movie_links(&user, &title.id)
        .await
        .expect("list series movie links");
    assert_eq!(links.len(), 2);
    let selected = links
        .iter()
        .find(|link| link.movie.title == "Iron Rail")
        .expect("selected movie link");
    let unselected = links
        .iter()
        .find(|link| link.movie.title == "Unselected Canon Movie")
        .expect("unselected movie link");
    assert!(selected.monitored, "selected canon movie is monitored");
    assert!(
        !unselected.monitored,
        "canon movies outside the selection stay unmonitored"
    );
}
