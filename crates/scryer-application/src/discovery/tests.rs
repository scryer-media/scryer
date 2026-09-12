use super::*;
use crate::types::TitleExternalRating;
use chrono::{TimeZone, Utc};
use scryer_domain::{ExternalId, MediaFacet};

#[test]
fn catalog_public_top_section_prefers_anime_this_week() {
    let mut sections = vec![
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "trending_now".to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: CATALOG_ANIME_WEEKLY_SECTION_ID.to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "popular_right_now".to_string(),
            ..Default::default()
        },
    ];

    let top = catalog_public_top_section(&mut sections, "anime")
        .expect("anime weekly section should be selected");

    assert_eq!(top.section_id, CATALOG_ANIME_WEEKLY_SECTION_ID);
    assert_eq!(
        sections
            .iter()
            .map(|section| section.section_id.as_str())
            .collect::<Vec<_>>(),
        vec!["trending_now", "popular_right_now"]
    );
}

#[test]
fn catalog_anime_public_policy_never_falls_back_to_generic_trending() {
    let mut sections = vec![
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "trending_now".to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "popular_series".to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "popular_right_now".to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "new_on_streaming".to_string(),
            ..Default::default()
        },
    ];

    catalog_filter_anime_public_sections(&mut sections);
    let top = catalog_public_top_section(&mut sections, "anime")
        .expect("a remaining Anime public section should become the lead");

    assert_eq!(top.section_id, "popular_right_now");
    assert_eq!(
        sections
            .iter()
            .map(|section| section.section_id.as_str())
            .collect::<Vec<_>>(),
        vec!["new_on_streaming"]
    );
}

#[test]
fn catalog_public_top_section_keeps_first_section_for_non_anime() {
    let mut sections = vec![
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "trending_now".to_string(),
            ..Default::default()
        },
        CatalogDiscoverySectionCandidatesRecord {
            section_id: CATALOG_ANIME_WEEKLY_SECTION_ID.to_string(),
            ..Default::default()
        },
    ];

    let top = catalog_public_top_section(&mut sections, "movie")
        .expect("first public section should be selected");

    assert_eq!(top.section_id, "trending_now");
}

#[test]
fn catalog_public_top_group_keeps_source_label_and_refills_after_deduplication() {
    let duplicate = test_discovery_item("already-shown", "movie", Some("movie"));
    let first_unique = test_discovery_item("first-unique", "movie", Some("movie"));
    let second_unique = test_discovery_item("second-unique", "movie", Some("movie"));
    let mut emitted_item_keys =
        HashSet::from([discovery_item_identity_key(&duplicate).to_string()]);

    let group = catalog_public_top_group(
        CatalogDiscoverySectionCandidatesRecord {
            section_id: "trending_now".to_string(),
            section_type: "TRENDING_NOW".to_string(),
            title: Some("Trending Now".to_string()),
            total_count: 3,
            items: vec![duplicate, first_unique, second_unique],
        },
        "movie",
        2,
        &mut emitted_item_keys,
    )
    .expect("remaining candidates should produce a group");

    assert_eq!(group.label_value.as_deref(), Some("Trending Now"));
    assert_eq!(group.total_count, 3);
    assert_eq!(group.items.len(), 2);
    assert_eq!(
        group
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>(),
        vec!["movie:first-unique", "movie:second-unique"]
    );
}

#[test]
fn pending_context_change_coalescing_drops_add_then_delete() {
    let existing = test_pending_change("change-1", "added", 1, 10);
    let incoming = test_pending_change("change-1", "removed", 2, 10);

    let merged = coalesce_pending_context_change(Some(&existing), incoming)
        .expect("coalescing should succeed");

    assert!(merged.is_none());
}

#[test]
fn pending_context_change_coalescing_preserves_added_and_first_seen() {
    let existing = test_pending_change("change-1", "added", 1, 10);
    let incoming = test_pending_change("change-1", "updated", 4, 11);

    let merged = coalesce_pending_context_change(Some(&existing), incoming)
        .expect("coalescing should succeed")
        .expect("change should remain pending");

    assert_eq!(merged.change_type, "added");
    assert_eq!(merged.first_seen_sequence, Some(1));
    assert_eq!(merged.last_seen_sequence, Some(4));
    assert_eq!(merged.previous_subject_key, None);
}

#[test]
fn pending_context_change_coalescing_update_then_delete_becomes_removed() {
    let existing = test_pending_change("change-1", "updated", 1, 10);
    let incoming = test_pending_change("change-1", "removed", 4, 10);

    let merged = coalesce_pending_context_change(Some(&existing), incoming)
        .expect("coalescing should succeed")
        .expect("change should remain pending");

    assert_eq!(merged.change_type, "removed");
    assert_eq!(
        merged.previous_subject_key.as_deref(),
        Some("tmdb:movie:10")
    );
    assert_eq!(merged.first_seen_sequence, Some(1));
    assert_eq!(merged.last_seen_sequence, Some(4));
}

#[test]
fn pending_context_change_coalescing_rematch_preserves_previous_subject() {
    let existing = test_pending_change("change-1", "updated", 1, 10);
    let mut incoming = test_pending_change("change-1", "rematched", 4, 11);
    incoming.previous_subject_key = Some("tmdb:movie:10".to_string());
    incoming.raw_previous_subject_json = existing.raw_subject_json.clone();

    let merged = coalesce_pending_context_change(Some(&existing), incoming)
        .expect("coalescing should succeed")
        .expect("change should remain pending");

    assert_eq!(merged.change_type, "rematched");
    assert_eq!(merged.subject_key.as_deref(), Some("tmdb:movie:11"));
    assert_eq!(
        merged.previous_subject_key.as_deref(),
        Some("tmdb:movie:10")
    );
    assert_eq!(merged.first_seen_sequence, Some(1));
    assert_eq!(merged.last_seen_sequence, Some(4));
}

#[test]
fn discovery_context_fingerprint_is_stable_across_title_and_external_id_order() {
    let left = build_discovery_library_context(
        &[
            test_title(
                "series",
                "The Example Show",
                MediaFacet::Series,
                vec![("tmdb_tv", "456"), ("thetvdb", "tvdb:123")],
            ),
            test_title(
                "anime",
                "Example Anime",
                MediaFacet::Anime,
                vec![("myanimelist", "7"), ("anilist:anime", "9")],
            ),
        ],
        DiscoveryContextDefaults::default(),
    );
    let right = build_discovery_library_context(
        &[
            test_title(
                "anime",
                "Example Anime",
                MediaFacet::Anime,
                vec![("anilist_anime", "9"), ("mal", "7")],
            ),
            test_title(
                "series",
                "The Example Show",
                MediaFacet::Series,
                vec![("tvdb_series", "123"), ("themoviedb", "456")],
            ),
        ],
        DiscoveryContextDefaults::default(),
    );

    assert_eq!(left.fingerprint, right.fingerprint);
    assert_eq!(left.subjects, right.subjects);
}

#[test]
fn discovery_context_only_builds_subjects_with_smg_supported_ids() {
    let mut imdb_only = test_title(
        "imdb-only",
        "IMDb Only",
        MediaFacet::Movie,
        vec![("imdb", "tt0133093")],
    );
    imdb_only.imdb_id = Some("tt0133093".to_string());

    let context = build_discovery_library_context(
        &[
            imdb_only,
            test_title(
                "unsupported",
                "Unsupported",
                MediaFacet::Movie,
                vec![("otherdb", "100")],
            ),
            test_title(
                "movie",
                "The Example Movie",
                MediaFacet::Movie,
                vec![("tmdb_movie", "movie:603")],
            ),
        ],
        DiscoveryContextDefaults::default(),
    );

    assert_eq!(context.subjects.len(), 1);
    assert_eq!(context.subjects[0].title_id, "movie");
    assert_eq!(context.subjects[0].subject_key, "tmdb:movie:603");
    assert_eq!(context.subjects[0].subject.tmdb_id, Some(603));
    assert_eq!(
        context.subjects[0].subject.external_ids,
        vec![DiscoveryExternalIdInput {
            source: "tmdb".to_string(),
            value: "603".to_string(),
        }]
    );
}

#[test]
fn discovery_context_uses_unique_typed_ids_and_keeps_external_ids() {
    let context = build_discovery_library_context(
        &[test_title(
            "series",
            "Series",
            MediaFacet::Series,
            vec![("tvdb", "10"), ("thetvdb", "11"), ("tmdb", "20")],
        )],
        DiscoveryContextDefaults::default(),
    );

    let subject = &context.subjects[0].subject;
    assert_eq!(context.subjects[0].subject_key, "tmdb:series:20");
    assert_eq!(subject.tvdb_id, None);
    assert_eq!(subject.tmdb_id, Some(20));
    assert_eq!(subject.kind.as_deref(), Some("series"));
    assert_eq!(subject.facet.as_deref(), Some("series"));
    assert_eq!(
        subject.external_ids,
        vec![
            DiscoveryExternalIdInput {
                source: "tmdb".to_string(),
                value: "20".to_string(),
            },
            DiscoveryExternalIdInput {
                source: "tvdb".to_string(),
                value: "10".to_string(),
            },
            DiscoveryExternalIdInput {
                source: "tvdb".to_string(),
                value: "11".to_string(),
            },
        ]
    );
}

#[test]
fn discovery_context_uses_series_resolver_kind_for_anime_subjects() {
    let context = build_discovery_library_context(
        &[test_title(
            "anime",
            "Anime",
            MediaFacet::Anime,
            vec![("tvdb", "100"), ("mal", "200")],
        )],
        DiscoveryContextDefaults::default(),
    );

    let subject = &context.subjects[0].subject;
    assert_eq!(context.subjects[0].subject_key, "tvdb:series:100");
    assert_eq!(subject.kind.as_deref(), Some("series"));
    assert_eq!(subject.facet.as_deref(), Some("anime"));
    assert_eq!(subject.tvdb_id, Some(100));
    assert_eq!(subject.mal_id, Some(200));
}

#[test]
fn discovery_home_public_sections_filter_owned_catalog_titles() {
    let owned_visibility = CatalogOwnedVisibility::from_titles(&[test_title(
        "house-of-ravens",
        "House of Ravens",
        MediaFacet::Series,
        vec![("tvdb", "371572")],
    )]);
    let mut owned_item = test_discovery_item("owned", "series", Some("series"));
    owned_item.target_key = "tvdb:series:371572".to_string();
    owned_item.display_title = "House of Ravens".to_string();
    let mut visible_item = test_discovery_item("visible", "series", Some("series"));
    visible_item.target_key = "tmdb:series:100".to_string();
    visible_item.display_title = "Visible".to_string();
    let mut refill_item = test_discovery_item("refill", "series", Some("series"));
    refill_item.target_key = "tmdb:series:101".to_string();
    refill_item.display_title = "Refill".to_string();
    let visibility = DiscoveryVisibility {
        allowed_media_kinds: HashSet::from(["series"]),
        ..DiscoveryVisibility::default()
    };

    let sections = filter_discovery_sections_for_owned_items(
        vec![DiscoverySectionResult {
            section_id: "trending_now".to_string(),
            section_type: "TRENDING_NOW".to_string(),
            title: "Top Series This Week".to_string(),
            surface: "public".to_string(),
            total_count: 3,
            items: vec![owned_item, visible_item, refill_item],
        }],
        &owned_visibility,
        &visibility,
        2,
    );

    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].total_count, 2);
    assert_eq!(
        sections[0]
            .items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect::<Vec<_>>(),
        vec!["Visible", "Refill"]
    );
}

#[test]
fn discovery_home_top_rated_prefers_external_rating_provenance_and_dedupes() {
    let mut scalar_only = test_discovery_item("scalar", "movie", Some("movie"));
    scalar_only.source_run_kind = "public_feed".to_string();
    scalar_only.target_key = "tmdb:movie:scalar".to_string();
    scalar_only.rating = Some(10.0);
    scalar_only.rank_score = Some(100.0);

    let mut weaker_public_duplicate = test_discovery_item("shared-public", "movie", Some("movie"));
    weaker_public_duplicate.source_run_kind = "public_feed".to_string();
    weaker_public_duplicate.target_key = "tmdb:movie:shared".to_string();
    weaker_public_duplicate.rating = Some(6.0);
    weaker_public_duplicate.rank_score = Some(1.0);

    let mut external_rated = test_discovery_item("shared-context", "movie", Some("movie"));
    external_rated.target_key = "tmdb:movie:shared".to_string();
    external_rated.rating = Some(5.0);
    external_rated.external_ratings = vec![TitleExternalRating {
        source: "imdb".to_string(),
        value: Some(8.8),
        score: Some(8.8),
        normalized: 0.88,
        votes: Some(100_000),
        url: "https://imdb.com/title/tt0000001".to_string(),
    }];

    let section = top_rated_discovery_home_section(
        &[scalar_only, weaker_public_duplicate, external_rated],
        &[],
        true,
        10,
    )
    .expect("top rated section");

    assert_eq!(section.section_type, "TOP_RATED");
    assert_eq!(section.total_count, 2);
    assert_eq!(
        section
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>(),
        vec!["tmdb:movie:shared", "tmdb:movie:scalar"]
    );
}

#[test]
fn discovery_home_top_rated_keeps_short_sections() {
    let mut only_item = test_discovery_item("only", "series", Some("series"));
    only_item.source_run_kind = "public_feed".to_string();
    only_item.target_key = "tmdb:series:only".to_string();
    only_item.rating = Some(7.0);

    let section =
        top_rated_discovery_home_section(&[only_item], &[], true, 6).expect("top rated section");

    assert_eq!(section.total_count, 1);
    assert_eq!(section.items.len(), 1);
    assert_eq!(section.items[0].target_key, "tmdb:series:only");
}

#[test]
fn discovery_home_hero_prefers_visible_personalized_item() {
    let mut public_item = test_discovery_item("public", "movie", Some("movie"));
    public_item.target_key = "tmdb:movie:public".to_string();
    public_item.rating = Some(10.0);
    public_item.rank_score = Some(99.0);
    public_item.source_count = Some(9);
    public_item.background_url = Some("https://images.example/public.jpg".to_string());

    let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
    personalized_item.target_key = "tmdb:movie:personalized".to_string();
    personalized_item.rating = Some(1.0);
    personalized_item.rank_score = Some(1.0);
    personalized_item.matched_subject_count = 1;
    personalized_item.background_url = Some("https://images.example/personalized.jpg".to_string());

    let hero = select_discovery_home_hero(
        &[test_discovery_section("public", vec![public_item])],
        &[test_discovery_section(
            "personalized",
            vec![personalized_item],
        )],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:movie:personalized");
}

#[test]
fn discovery_home_hero_ignores_public_items_inside_mixed_personalized_sections() {
    let mut public_item = test_discovery_item("public", "movie", Some("movie"));
    public_item.source_run_kind = "public_feed".to_string();
    public_item.target_key = "tmdb:movie:public".to_string();
    public_item.rating = Some(10.0);
    public_item.rank_score = Some(100.0);
    public_item.background_url = Some("https://images.example/public.jpg".to_string());

    let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
    personalized_item.source_run_kind = "context_snapshot".to_string();
    personalized_item.target_key = "tmdb:movie:personalized".to_string();
    personalized_item.matched_subject_count = 1;
    personalized_item.rating = Some(1.0);
    personalized_item.rank_score = Some(1.0);
    personalized_item.background_url = Some("https://images.example/personalized.jpg".to_string());

    let hero = select_discovery_home_hero(
        &[],
        &[test_discovery_section(
            "top_rated",
            vec![public_item, personalized_item],
        )],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:movie:personalized");
}

#[test]
fn discovery_home_hero_skips_owned_personalized_items() {
    let mut owned_item = test_discovery_item("owned", "series", Some("series"));
    owned_item.target_key = "tmdb:series:owned".to_string();
    owned_item.owned_in_input = true;
    owned_item.matched_subject_count = 100;
    owned_item.background_url = Some("https://images.example/owned.jpg".to_string());

    let mut visible_item = test_discovery_item("visible", "series", Some("series"));
    visible_item.target_key = "tmdb:series:visible".to_string();
    visible_item.matched_subject_count = 1;
    visible_item.background_url = Some("https://images.example/visible.jpg".to_string());

    let hero = select_discovery_home_hero(
        &[],
        &[test_discovery_section(
            "personalized",
            vec![owned_item, visible_item],
        )],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:series:visible");
}

#[test]
fn discovery_home_hero_falls_back_to_highest_ranked_public_item() {
    // The public hero now mirrors the personalized philosophy: rank_score
    // leads and a bare rating only breaks ties. A strongly ranked item wins
    // even against a rival carrying a higher but un-corroborated rating, so a
    // lone inflated rating can no longer commandeer the hero slot.
    let mut higher_ranked = test_discovery_item("higher-rank", "movie", Some("movie"));
    higher_ranked.target_key = "tmdb:movie:higher-rank".to_string();
    higher_ranked.rating = Some(6.0);
    higher_ranked.rank_score = Some(100.0);
    higher_ranked.background_url = Some("https://images.example/higher-rank.jpg".to_string());

    let mut higher_rating = test_discovery_item("higher-rating", "movie", Some("movie"));
    higher_rating.target_key = "tmdb:movie:higher-rating".to_string();
    higher_rating.rating = Some(8.5);
    higher_rating.rank_score = Some(1.0);
    higher_rating.background_url = Some("https://images.example/higher-rating.jpg".to_string());

    let hero = select_discovery_home_hero(
        &[test_discovery_section(
            "public",
            vec![higher_rating, higher_ranked],
        )],
        &[],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:movie:higher-rank");
}

#[test]
fn discovery_home_hero_breaks_public_rank_ties_by_rating() {
    // With equal rank_score the credible-rating tiebreak decides, so a
    // healthy rating still wins when the ranking signal is level.
    let mut lower_rated = test_discovery_item("lower", "movie", Some("movie"));
    lower_rated.source_run_kind = "public_feed".to_string();
    lower_rated.target_key = "tmdb:movie:lower".to_string();
    lower_rated.rating = Some(6.0);
    lower_rated.rank_score = Some(10.0);
    lower_rated.background_url = Some("https://images.example/lower.jpg".to_string());

    let mut higher_rated = test_discovery_item("higher", "movie", Some("movie"));
    higher_rated.source_run_kind = "public_feed".to_string();
    higher_rated.target_key = "tmdb:movie:higher".to_string();
    higher_rated.rating = Some(8.5);
    higher_rated.rank_score = Some(10.0);
    higher_rated.background_url = Some("https://images.example/higher.jpg".to_string());

    let hero = select_public_discovery_home_hero(&[test_discovery_section(
        "public",
        vec![lower_rated, higher_rated],
    )])
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:movie:higher");
}

#[test]
fn discovery_home_hero_treats_blank_backdrop_as_missing() {
    let mut blank_backdrop = test_discovery_item("blank", "movie", Some("movie"));
    blank_backdrop.target_key = "tmdb:movie:blank".to_string();
    blank_backdrop.rating = Some(10.0);
    blank_backdrop.rank_score = Some(100.0);
    blank_backdrop.background_url = Some("   ".to_string());

    let mut real_backdrop = test_discovery_item("real", "movie", Some("movie"));
    real_backdrop.target_key = "tmdb:movie:real".to_string();
    real_backdrop.rating = Some(1.0);
    real_backdrop.rank_score = Some(1.0);
    real_backdrop.background_url = Some("https://images.example/real.jpg".to_string());

    let hero = select_discovery_home_hero(
        &[test_discovery_section(
            "public",
            vec![blank_backdrop, real_backdrop],
        )],
        &[],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:movie:real");
}

#[test]
fn discovery_home_hero_requires_a_backdrop() {
    let item = test_discovery_item("poster-only", "movie", Some("movie"));

    let hero = select_discovery_home_hero(&[test_discovery_section("public", vec![item])], &[]);

    assert!(hero.is_none());
}

#[test]
fn discovery_home_hero_tie_breaks_by_target_key_without_raw_labels() {
    let mut later_key = test_discovery_item("later", "anime", Some("anime"));
    later_key.target_key = "tmdb:anime:z".to_string();
    later_key.background_url = Some("https://images.example/z.jpg".to_string());
    later_key.source_tags = vec![DiscoverySourceTagRecord {
        category: Some("theme".to_string()),
        name: Some("Isekai".to_string()),
        values: vec!["Isekai".to_string()],
    }];

    let mut earlier_key = test_discovery_item("earlier", "anime", Some("anime"));
    earlier_key.target_key = "tmdb:anime:a".to_string();
    earlier_key.background_url = Some("https://images.example/a.jpg".to_string());
    earlier_key.facet_terms = vec!["canonical:theme:isekai".to_string()];

    let hero = select_discovery_home_hero(
        &[test_discovery_section(
            "public",
            vec![later_key, earlier_key],
        )],
        &[],
    )
    .expect("hero item");

    assert_eq!(hero.target_key, "tmdb:anime:a");
}

#[test]
fn discovery_context_deduplicates_identical_subjects() {
    let context = build_discovery_library_context(
        &[
            test_title(
                "library-a",
                "Movie A",
                MediaFacet::Movie,
                vec![("tmdb", "603")],
            ),
            test_title(
                "library-b",
                "Movie B",
                MediaFacet::Movie,
                vec![("tmdb_movie", "603")],
            ),
        ],
        DiscoveryContextDefaults::default(),
    );

    assert_eq!(context.subjects.len(), 1);
    assert_eq!(context.subjects[0].title_id, "library-a");
}

#[test]
fn discovery_context_fallback_key_uses_external_id_priority_after_ambiguous_typed_ids() {
    let context = build_discovery_library_context(
        &[test_title(
            "anime",
            "Anime",
            MediaFacet::Anime,
            vec![
                ("mal", "200"),
                ("myanimelist", "201"),
                ("anidb", "10"),
                ("anidb", "11"),
            ],
        )],
        DiscoveryContextDefaults::default(),
    );

    let subject = &context.subjects[0].subject;
    assert_eq!(subject.mal_id, None);
    assert_eq!(subject.anidb_id, None);
    assert_eq!(context.subjects[0].subject_key, "anidb:anime:10");
}

#[test]
fn title_recommendations_subject_prefers_tvdb_then_tmdb_then_imdb() {
    let title = test_title(
        "movie",
        "Movie",
        MediaFacet::Movie,
        vec![("imdb", "tt0133093"), ("tmdb", "603"), ("tvdb", "78874")],
    );

    let (subject, source_target_keys) =
        title_recommendations_subject(&title, &[]).expect("subject should build");

    assert_eq!(subject.key.as_deref(), Some("tvdb:movie:78874"));
    assert_eq!(subject.tvdb_id, Some(78874));
    assert_eq!(subject.tmdb_id, Some(603));
    assert!(
        source_target_keys
            .iter()
            .any(|key| key == "imdb:title:tt0133093")
    );
    assert!(
        subject
            .external_ids
            .iter()
            .any(|external_id| external_id.source == "imdb")
    );
}

#[test]
fn title_recommendations_subject_uses_anime_ids_after_tvdb_tmdb() {
    let tvdb_title = test_title(
        "anime-tvdb",
        "Anime",
        MediaFacet::Anime,
        vec![("mal", "200"), ("anidb", "10"), ("tvdb", "100")],
    );
    let (subject, _) =
        title_recommendations_subject(&tvdb_title, &[]).expect("subject should build");
    assert_eq!(subject.key.as_deref(), Some("tvdb:series:100"));

    let anime_id_title = test_title(
        "anime-mal",
        "Anime",
        MediaFacet::Anime,
        vec![("anidb", "10"), ("mal", "200"), ("anilist", "300")],
    );
    let (subject, _) =
        title_recommendations_subject(&anime_id_title, &[]).expect("subject should build");
    assert_eq!(subject.key.as_deref(), Some("mal:anime:200"));
}

#[test]
fn discovery_item_records_do_not_persist_smg_resolved_title_id_as_local_fk() {
    let now = Utc.timestamp_opt(0, 0).unwrap();
    let item = DiscoveryTitle {
        target_key: "tmdb:movie:603".to_string(),
        target_kind: "movie".to_string(),
        resolved: true,
        resolved_title_id: "smg-title-603".to_string(),
        display_title: "The Example".to_string(),
        ..DiscoveryTitle::default()
    };

    let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
        .expect("discovery item records should build");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].resolved_title_id, None);
}

#[test]
fn discovery_item_records_derive_local_sort_title_from_human_title() {
    let now = Utc.timestamp_opt(0, 0).unwrap();
    let item = DiscoveryTitle {
        target_key: "tvdb:movie:603".to_string(),
        target_kind: "movie".to_string(),
        resolved: true,
        display_title: "tvdb:movie:603".to_string(),
        original_title: "\u{ff34}\u{ff48}\u{ff45} Matrix".to_string(),
        ..DiscoveryTitle::default()
    };

    let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
        .expect("discovery item records should build");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].display_title, "\u{ff34}\u{ff48}\u{ff45} Matrix");
    assert_eq!(records[0].sort_title.as_deref(), Some("Matrix"));
}

#[test]
fn discovery_item_records_wire_canonical_tags_and_theme_terms() {
    let now = Utc.timestamp_opt(0, 0).unwrap();
    let item = DiscoveryTitle {
        target_key: "tmdb:movie:603".to_string(),
        target_kind: "movie".to_string(),
        resolved: true,
        display_title: "The Example".to_string(),
        source_tags: vec![
            serde_json::json!({
                "source": "mal",
                "category": "theme",
                "name": "mal:theme:psychological",
                "canonical": "canonical:theme:psychological"
            }),
            serde_json::json!("canonical:theme:survival"),
        ],
        canonical_tags: vec![
            serde_json::json!({
                "key": "canonical:genre:action",
                "category": "genre",
                "name": "action",
                "confidence": 1.0,
            }),
            serde_json::json!({
                "key": "canonical:genre:drama",
                "category": "genre",
                "name": "Drama",
                "confidence": 1.0,
            }),
            serde_json::json!({
                "key": "canonical:theme:isekai",
                "category": "theme",
                "name": "Isekai",
                "confidence": 1.0,
            }),
            serde_json::json!({
                "key": "adult-cast",
                "category": "theme",
                "name": "Adult Cast",
                "confidence": 1.0,
            }),
        ],
        facet_terms: vec![
            "raw:compat".to_string(),
            "canonical:genre:drama".to_string(),
        ],
        ..DiscoveryTitle::default()
    };

    let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
        .expect("discovery item records should build");

    assert_eq!(records.len(), 1);
    assert!(records[0].facet_terms.contains(&"raw:compat".to_string()));
    assert!(
        records[0]
            .facet_terms
            .contains(&"canonical:genre:action".to_string())
    );
    assert!(
        records[0]
            .facet_terms
            .contains(&"canonical:genre:drama".to_string())
    );
    assert_eq!(
        records[0]
            .facet_terms
            .iter()
            .filter(|term| term.as_str() == "canonical:genre:action")
            .count(),
        1
    );
    assert!(
        records[0]
            .facet_terms
            .contains(&"canonical:theme:isekai".to_string())
    );
    assert!(
        records[0]
            .facet_terms
            .contains(&"canonical:theme:adult-cast".to_string())
    );
    assert!(
        !records[0]
            .facet_terms
            .contains(&"canonical:theme:psychological".to_string())
    );
}

#[test]
fn discovery_item_genre_query_uses_canonical_facet_terms() {
    fn matches_genre(item: &DiscoveryItemRecord, genre: &str) -> bool {
        item_matches_discovery_items_query(
            item,
            &DiscoveryItemsQuery {
                genres: vec![genre.to_string()],
                include_unresolved: false,
                ..DiscoveryItemsQuery::default()
            },
        )
    }

    let mut item = test_discovery_item("canonical", "movie", Some("movie"));
    item.facet_terms = vec!["canonical:genre:action".to_string()];

    assert!(matches_genre(&item, "Action"));
    assert!(matches_genre(&item, "canonical:genre:action"));
    assert!(!matches_genre(&item, "Drama"));
}

#[test]
fn discovery_item_media_kind_uses_v1_content_type_contract() {
    fn matches_target_kind(item: &DiscoveryItemRecord, target_kind: &str) -> bool {
        item_matches_discovery_items_query(
            item,
            &DiscoveryItemsQuery {
                target_kinds: vec![target_kind.to_string()],
                include_unresolved: false,
                ..DiscoveryItemsQuery::default()
            },
        )
    }

    let anime = test_discovery_item("anime", "series", Some("anime"));
    assert!(matches_target_kind(&anime, "anime"));
    assert!(!matches_target_kind(&anime, "series"));

    let series = test_discovery_item("series", "series", Some("series"));
    assert!(matches_target_kind(&series, "series"));
    assert!(!matches_target_kind(&series, "anime"));

    let movie = test_discovery_item("movie", "movie", Some("movie"));
    assert!(matches_target_kind(&movie, "movie"));
    assert!(!matches_target_kind(&movie, "series"));

    let fallback = test_discovery_item("fallback", "anime", Some(""));
    assert!(matches_target_kind(&fallback, "anime"));
    assert!(!matches_target_kind(&fallback, "series"));

    let unknown = test_discovery_item("unknown", "series", Some("tv"));
    assert!(!matches_target_kind(&unknown, "series"));
    assert!(!matches_target_kind(&unknown, "anime"));
}

#[test]
fn personalized_sections_dedupe_derived_items_and_require_subject_match() {
    fn discovery_item(
        id: &str,
        title: &str,
        genre_labels: &[&str],
        rank_score: f64,
        matched_subject_count: i32,
    ) -> DiscoveryItemRecord {
        let mut item = test_discovery_item(id, "movie", Some("movie"));
        item.target_key = format!("tmdb:movie:{id}");
        item.display_title = title.to_string();
        item.sort_title = Some(title.to_string());
        item.facet_terms = genre_labels
            .iter()
            .map(|genre| format!("canonical:genre:{}", genre.to_ascii_lowercase()))
            .collect();
        item.canonical_tags = genre_labels
            .iter()
            .map(|genre| provider_genre_tag(genre))
            .collect();
        item.rank_score = Some(rank_score);
        item.matched_subject_count = matched_subject_count;
        item
    }

    let profile = affinity_profile(&["Adventure", "Animation"], &[], library_mix(60, 30, 10));
    let items = extend_with_fillers(
        vec![
            discovery_item("1", "Shared Match", &["Adventure", "Animation"], 100.0, 1),
            discovery_item("2", "Unlinked Animation", &["Animation"], 95.0, 0),
            discovery_item("3", "Adventure Match", &["Adventure"], 90.0, 1),
            discovery_item("4", "Animation Match", &["Animation"], 80.0, 1),
        ],
        vec![
            affinity_filler_items("adv", "movie", &["canonical:genre:adventure"], 8),
            affinity_filler_items("ani", "movie", &["canonical:genre:animation"], 8),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let adventure = sections
        .iter()
        .find(|section| section.title == "Because You Like Adventure")
        .expect("adventure section");
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation section");

    assert_eq!(
        affinity_section_lead(adventure, 2),
        vec!["Shared Match", "Adventure Match"]
    );
    assert_eq!(affinity_section_lead(animation, 1), vec!["Animation Match"]);
    assert!(
        !animation
            .items
            .iter()
            .any(|item| item.display_title == "Unlinked Animation"),
        "an item with no matched subject has no business on a reason rail"
    );

    let mut seen = HashSet::new();
    for item in sections.iter().flat_map(|section| section.items.iter()) {
        assert!(
            seen.insert(discovery_item_identity_key(item).to_string()),
            "duplicate discovery item {} in derived sections",
            item.display_title
        );
    }
}

fn affinity_test_item(id: &str, content_type: &str, facet_terms: &[&str]) -> DiscoveryItemRecord {
    let target_kind = if content_type == "movie" {
        "movie"
    } else {
        "series"
    };
    let mut item = test_discovery_item(id, target_kind, Some(content_type));
    item.target_key = format!("tmdb:{content_type}:{id}");
    item.display_title = format!("Title {id}");
    item.sort_title = Some(format!("Title {id}"));
    item.facet_terms = facet_terms.iter().map(|term| (*term).to_string()).collect();
    item.matched_subject_count = 1;
    // Above the evidence-less band, so the fixture is about the rule under test
    // rather than about the evidence floor.
    item.rank_score = Some(5.0);
    // The pool loader hydrates genre-category canonical tags alongside the
    // facet terms, because a facet term carries no provenance and the
    // corroboration gate needs some. Fixtures mirror that, and the tests that
    // are *about* corroboration override these tags to make their point.
    item.canonical_tags = facet_terms
        .iter()
        .filter_map(|term| canonical_discovery_facet_label(term, "genre"))
        .map(|label| provider_genre_tag(&label))
        .collect();
    item
}

fn affinity_section_titles(section: &DiscoverySectionResult) -> Vec<&str> {
    section
        .items
        .iter()
        .map(|item| item.display_title.as_str())
        .collect()
}

/// A library that could plausibly have earned the rails under test: a medium
/// mix that admits every medium the fixture uses, and enough owned titles to
/// open the full ladder. Fixtures say this explicitly rather than inheriting a
/// default mix of all zeros, which the medium law would (correctly) read as a
/// library that has rejected everything.
fn affinity_profile(
    genre_labels: &[&str],
    theme_labels: &[&str],
    mix: DiscoveryLibraryMediumMix,
) -> DiscoveryLibraryAffinityProfile {
    DiscoveryLibraryAffinityProfile {
        genre_labels: genre_labels.iter().map(|label| label.to_string()).collect(),
        theme_labels: theme_labels.iter().map(|label| label.to_string()).collect(),
        owned_title_count: mix.total(),
        medium_mix: mix,
    }
}

fn library_mix(live_action: usize, animation: usize, anime: usize) -> DiscoveryLibraryMediumMix {
    DiscoveryLibraryMediumMix {
        live_action,
        animation,
        anime,
    }
}

fn balanced_library_mix() -> DiscoveryLibraryMediumMix {
    library_mix(40, 30, 30)
}

fn compose_affinity_sections(
    items: &[DiscoveryItemRecord],
    profile: &DiscoveryLibraryAffinityProfile,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    personalized_section_results(
        items,
        profile,
        &HashMap::new(),
        DiscoveryRailCompositionSettings::default(),
        true,
        limit,
    )
}

fn compose_affinity_sections_with_settings(
    items: &[DiscoveryItemRecord],
    profile: &DiscoveryLibraryAffinityProfile,
    settings: DiscoveryRailCompositionSettings,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    personalized_section_results(items, profile, &HashMap::new(), settings, true, limit)
}

/// Rails ship only when they have enough to say, so a fixture that wants a rail
/// has to give it enough items to clear [`DISCOVERY_RAIL_MIN_ITEMS`]. Fillers
/// carry the rail's labels and edge evidence but deliberately weak scores, so
/// they always sort behind whatever the test is asserting about.
fn affinity_filler_items(
    prefix: &str,
    content_type: &str,
    facet_terms: &[&str],
    count: usize,
) -> Vec<DiscoveryItemRecord> {
    (0..count)
        .map(|index| {
            let mut item = affinity_test_item(
                &format!("{prefix}-filler-{index}"),
                content_type,
                facet_terms,
            );
            item.display_title = format!("Filler {prefix} {index}");
            item.sort_title = Some(item.display_title.clone());
            item.rank_score = Some(2.0);
            item
        })
        .collect()
}

fn extend_with_fillers(
    mut items: Vec<DiscoveryItemRecord>,
    fillers: Vec<Vec<DiscoveryItemRecord>>,
) -> Vec<DiscoveryItemRecord> {
    for filler in fillers {
        items.extend(filler);
    }
    items
}

/// The leading `count` titles of a section, which is where an ordering rule is
/// observable; fillers occupy the tail by construction.
fn affinity_section_lead(section: &DiscoverySectionResult, count: usize) -> Vec<&str> {
    affinity_section_titles(section)
        .into_iter()
        .take(count)
        .collect()
}

#[test]
fn personalized_sections_retire_medium_and_library_rails() {
    // Medium is owned by the dashboard's facet chips, so the per-medium
    // "For You" rails and BECAUSE_YOU_HAVE are gone. Eight items per medium
    // clears the retired medium rails' old six-item floor, every item has a
    // matched subject so BECAUSE_YOU_HAVE would have qualified too, and the
    // small limit leaves plenty of unclaimed items for them: this fixture
    // emitted all four retired sections before the change.
    let profile = affinity_profile(&[], &[], balanced_library_mix());
    let mut items = Vec::new();
    for index in 0..8 {
        items.push(affinity_test_item(&format!("m{index}"), "movie", &[]));
        items.push(affinity_test_item(&format!("s{index}"), "series", &[]));
        items.push(affinity_test_item(
            &format!("a{index}"),
            "anime",
            &["canonical:genre:anime"],
        ));
    }

    let sections = compose_affinity_sections(&items, &profile, 5);
    let section_types = sections
        .iter()
        .map(|section| section.section_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(section_types, vec!["FOR_YOU"]);
}

#[test]
fn personalized_sections_prefer_specific_tag_rail_over_broad_genre_rail() {
    // Composition order is dedupe priority. A title that earns the narrow
    // "Because You Like Isekai" theme rail must not be eaten first by the
    // far broader "Because You Like Animation" genre rail.
    let profile = affinity_profile(&["Animation"], &["Isekai"], library_mix(20, 60, 20));
    let items = extend_with_fillers(
        vec![
            affinity_test_item(
                "1",
                "movie",
                &["canonical:genre:animation", "canonical:theme:isekai"],
            ),
            affinity_test_item("2", "movie", &["canonical:genre:animation"]),
        ],
        vec![
            affinity_filler_items(
                "isekai",
                "movie",
                &["canonical:genre:animation", "canonical:theme:isekai"],
                8,
            ),
            affinity_filler_items("anim", "movie", &["canonical:genre:animation"], 8),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let isekai = sections
        .iter()
        .find(|section| section.title == "Because You Like Isekai")
        .expect("isekai theme section");
    assert_eq!(isekai.section_type, "BECAUSE_YOU_LIKE_TAG");
    assert_eq!(affinity_section_lead(isekai, 1), vec!["Title 1"]);

    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(animation.section_type, "BECAUSE_YOU_LIKE_GENRE");
    assert_eq!(affinity_section_lead(animation, 1), vec!["Title 2"]);
    assert!(
        !animation
            .items
            .iter()
            .any(|item| item.display_title == "Title 1"),
        "the narrow theme rail must claim the shared title first"
    );

    let tag_index = sections
        .iter()
        .position(|section| section.section_type == "BECAUSE_YOU_LIKE_TAG")
        .expect("tag section index");
    let genre_index = sections
        .iter()
        .position(|section| section.section_type == "BECAUSE_YOU_LIKE_GENRE")
        .expect("genre section index");
    assert!(tag_index < genre_index);
}

#[test]
fn affinity_label_rails_keep_animation_and_anime_apart() {
    // Animation is a medium, anime is a tradition. Both titles carry the
    // canonical `animation` genre facet, so only the media-kind guard keeps
    // Western animation and anime out of each other's rails.
    let profile = affinity_profile(&["Animation", "Anime"], &[], balanced_library_mix());
    let items = extend_with_fillers(
        vec![
            affinity_test_item("western", "movie", &["canonical:genre:animation"]),
            affinity_test_item(
                "shonen",
                "anime",
                &["canonical:genre:animation", "canonical:genre:anime"],
            ),
        ],
        vec![
            affinity_filler_items("west", "movie", &["canonical:genre:animation"], 8),
            affinity_filler_items(
                "shonen",
                "anime",
                &["canonical:genre:animation", "canonical:genre:anime"],
                8,
            ),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_lead(animation, 1), vec!["Title western"]);
    assert!(
        animation
            .items
            .iter()
            .all(|item| discovery_item_medium(item) == DiscoveryMedium::Animation),
        "the Animation rail must contain only Western animation: {:?}",
        affinity_section_titles(animation)
    );

    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section");
    assert_eq!(affinity_section_lead(anime, 1), vec!["Title shonen"]);
    assert!(
        anime
            .items
            .iter()
            .all(|item| discovery_item_medium(item) == DiscoveryMedium::Anime),
        "the Anime rail must contain only anime: {:?}",
        affinity_section_titles(anime)
    );
}

#[test]
fn affinity_theme_labels_come_from_canonical_tags_not_from_the_user_tag_bag() {
    // The affinity profile used to read `title.tags` for its theme rails, which
    // put an operator's private tag vocabulary into discovery. User tags are
    // catalog-local and never leave the instance, so the profile is built from
    // canonical theme tags only. A title whose bag says "isekai" but whose
    // canonical tags say nothing contributes no theme label at all.
    let mut bag_only = test_title("bag-only", "Bag Only", MediaFacet::Series, Vec::new());
    bag_only.tags = vec!["isekai".to_string(), "keep".to_string()];
    bag_only.canonical_tags = vec![CanonicalMediaTag {
        key: "canonical:theme:slow-burn".to_string(),
        category: "theme".to_string(),
        name: "Slow Burn".to_string(),
        confidence: None,
        sources: Vec::new(),
        source_tag_keys: Vec::new(),
        is_adult: false,
        is_spoiler: false,
    }];

    let titles = vec![bag_only.clone(), bag_only.clone(), bag_only];
    let theme_labels = top_owned_title_labels(
        &titles,
        |title| canonical_tag_labels(&title.canonical_tags, "theme"),
        2,
        discovery_rail_label_support_floor(titles.len()),
    );
    assert_eq!(theme_labels, vec!["Slow Burn".to_string()]);
    assert!(
        !theme_labels.iter().any(|label| label == "isekai"),
        "a user tag must never reach the affinity profile: {theme_labels:?}"
    );

    // And the profile struct itself no longer has anywhere to put one.
    let profile = affinity_profile(
        &[],
        &theme_labels.iter().map(String::as_str).collect::<Vec<_>>(),
        balanced_library_mix(),
    );
    let items = extend_with_fillers(
        vec![affinity_test_item(
            "slow",
            "series",
            &["canonical:theme:slow-burn"],
        )],
        vec![affinity_filler_items(
            "slow",
            "series",
            &["canonical:theme:slow-burn"],
            8,
        )],
    );
    let sections = compose_affinity_sections(&items, &profile, 10);
    assert!(
        sections
            .iter()
            .any(|section| section.title == "Because You Like Slow Burn")
    );
}

#[test]
fn anime_affinity_label_survives_the_generic_label_filter() {
    // Reachability guard: the boundary's "anime" arm is dead code unless a
    // real profile can actually carry the label. The profile is built by
    // `top_owned_title_labels` over owned titles, which drops labels that
    // `discovery_affinity_label_is_generic` rejects - "anime" must not be
    // among them, or "Because You Like Anime" can never exist.
    fn anime_title(id: &str) -> Title {
        let mut title = test_title(id, id, MediaFacet::Series, Vec::new());
        title.canonical_tags = ["Anime", "Animation"]
            .into_iter()
            .map(provider_genre_tag)
            .collect();
        title
    }

    let titles = vec![anime_title("a1"), anime_title("a2"), anime_title("a3")];
    let genre_labels = top_owned_title_labels(
        &titles,
        |title| corroborated_canonical_genre_labels(&title.canonical_tags, true),
        2,
        discovery_rail_label_support_floor(titles.len()),
    );
    assert!(
        genre_labels.iter().any(|label| label == "Anime"),
        "anime-heavy library produced no Anime label: {genre_labels:?}"
    );

    // ...and the label, once reachable, splits the two traditions apart.
    let profile = affinity_profile(
        &genre_labels.iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
        balanced_library_mix(),
    );
    let items = extend_with_fillers(
        vec![
            corroborated_genre_item("western", "movie", &["Animation"]),
            corroborated_genre_item("shonen", "anime", &["Animation", "Anime"]),
        ],
        vec![
            corroborated_genre_fillers("west", "movie", &["Animation"], 8),
            corroborated_genre_fillers("shonen", "anime", &["Animation", "Anime"], 8),
        ],
    );
    let sections = compose_affinity_sections(&items, &profile, 10);
    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section should be reachable from a real profile");
    assert_eq!(affinity_section_lead(anime, 1), vec!["Title shonen"]);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_lead(animation, 1), vec!["Title western"]);
}

/// A canonical genre in the shape SMG produces from a provider's own genre
/// list: high confidence and a `genre:` source key. This is the shape the
/// corroboration gate is supposed to accept.
fn provider_genre_tag(name: &str) -> CanonicalMediaTag {
    let slug = name.to_ascii_lowercase();
    CanonicalMediaTag {
        key: format!("canonical:genre:{slug}"),
        category: "genre".to_string(),
        name: name.to_string(),
        confidence: Some(0.95),
        sources: vec!["genre".to_string()],
        source_tag_keys: vec![format!("genre:{slug}")],
        is_adult: false,
        is_spoiler: false,
    }
}

/// The shape a single community tag produces: confident enough on its own, but
/// carried by one free-text tag key. This is what the gate exists to reject.
fn community_tag_genre_tag(name: &str) -> CanonicalMediaTag {
    let slug = name.to_ascii_lowercase();
    CanonicalMediaTag {
        key: format!("canonical:genre:{slug}"),
        category: "genre".to_string(),
        name: name.to_string(),
        confidence: Some(0.95),
        sources: vec!["anilist".to_string()],
        source_tag_keys: vec![format!("anilist:tag:{slug}")],
        is_adult: false,
        is_spoiler: false,
    }
}

/// A pool item carrying both halves of what a genre rail now needs: the
/// canonical facet term the matcher keys on, and the hydrated canonical tag
/// that proves the genre is corroborated.
fn corroborated_genre_item(id: &str, content_type: &str, genres: &[&str]) -> DiscoveryItemRecord {
    let facet_terms = genres
        .iter()
        .map(|genre| format!("canonical:genre:{}", genre.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    affinity_test_item(
        id,
        content_type,
        &facet_terms.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn corroborated_genre_fillers(
    prefix: &str,
    content_type: &str,
    genres: &[&str],
    count: usize,
) -> Vec<DiscoveryItemRecord> {
    (0..count)
        .map(|index| {
            let mut item =
                corroborated_genre_item(&format!("{prefix}-filler-{index}"), content_type, genres);
            item.display_title = format!("Filler {prefix} {index}");
            item.sort_title = Some(item.display_title.clone());
            item.rank_score = Some(2.0);
            item
        })
        .collect()
}

#[test]
fn anime_without_content_type_does_not_leak_into_the_animation_rail() {
    // `discovery_item_media_kind` falls back to target_kind, so a
    // content_type-less anime reports as a plain "series". Only the canonical
    // anime genre facet keeps it out of the Western-animation rail.
    let mut untyped_anime = test_discovery_item("untyped", "series", None);
    untyped_anime.target_key = "mal:anime:untyped".to_string();
    untyped_anime.display_title = "Title untyped".to_string();
    untyped_anime.sort_title = Some("Title untyped".to_string());
    untyped_anime.facet_terms = vec![
        "canonical:genre:animation".to_string(),
        "canonical:genre:anime".to_string(),
    ];
    untyped_anime.canonical_tags =
        vec![provider_genre_tag("Animation"), provider_genre_tag("Anime")];
    untyped_anime.matched_subject_count = 1;
    untyped_anime.rank_score = Some(5.0);
    assert_eq!(discovery_item_media_kind(&untyped_anime), Some("series"));
    assert_eq!(
        discovery_item_medium(&untyped_anime),
        DiscoveryMedium::Anime
    );

    let profile = affinity_profile(&["Animation", "Anime"], &[], balanced_library_mix());
    let items = extend_with_fillers(
        vec![
            affinity_test_item("western", "movie", &["canonical:genre:animation"]),
            untyped_anime,
        ],
        vec![
            affinity_filler_items("west", "movie", &["canonical:genre:animation"], 8),
            affinity_filler_items(
                "untyped",
                "anime",
                &["canonical:genre:animation", "canonical:genre:anime"],
                8,
            ),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_lead(animation, 1), vec!["Title western"]);
    assert!(
        !animation
            .items
            .iter()
            .any(|item| item.display_title == "Title untyped"),
        "an anime without a content type must not leak into the animation rail"
    );
    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section");
    assert_eq!(affinity_section_lead(anime, 1), vec!["Title untyped"]);
}

// ── Medium law ───────────────────────────────────────────────────────────────

/// Every medium the fixtures need, in the shape the pool produces.
fn medium_item(id: &str, medium: DiscoveryMedium) -> DiscoveryItemRecord {
    match medium {
        DiscoveryMedium::LiveAction => affinity_test_item(id, "movie", &["canonical:genre:crime"]),
        DiscoveryMedium::Animation => {
            affinity_test_item(id, "movie", &["canonical:genre:animation"])
        }
        DiscoveryMedium::Anime => affinity_test_item(
            id,
            "anime",
            &["canonical:genre:animation", "canonical:genre:anime"],
        ),
    }
}

fn medium_items(prefix: &str, medium: DiscoveryMedium, count: usize) -> Vec<DiscoveryItemRecord> {
    (0..count)
        .map(|index| {
            let mut item = medium_item(&format!("{prefix}-{index}"), medium);
            item.display_title = format!("{prefix} {index}");
            item.sort_title = Some(item.display_title.clone());
            item
        })
        .collect()
}

fn all_medium_items(per_medium: usize) -> Vec<DiscoveryItemRecord> {
    let mut items = medium_items("live", DiscoveryMedium::LiveAction, per_medium);
    items.extend(medium_items(
        "western",
        DiscoveryMedium::Animation,
        per_medium,
    ));
    items.extend(medium_items("anime", DiscoveryMedium::Anime, per_medium));
    items
}

fn composed_mediums(sections: &[DiscoverySectionResult]) -> Vec<DiscoveryMedium> {
    sections
        .iter()
        .flat_map(|section| section.items.iter())
        .map(discovery_item_medium)
        .collect()
}

#[test]
fn a_library_with_no_anime_is_never_shown_anime() {
    // Absence is evidence. No genre, theme or edge signal outranks a library
    // that has declined an entire medium.
    let profile = affinity_profile(&[], &[], library_mix(40, 20, 0));
    let sections = compose_affinity_sections(&all_medium_items(12), &profile, 20);

    assert!(!sections.is_empty(), "the live-action rails should survive");
    assert!(
        !composed_mediums(&sections).contains(&DiscoveryMedium::Anime),
        "a zero-anime library was shown anime"
    );
}

#[test]
fn a_library_with_one_anime_is_shown_at_most_the_slot_floor() {
    // Above zero the medium is admitted, but only in proportion. One anime out
    // of a hundred buys the two-slot floor, not a rail full of anime.
    let profile = affinity_profile(&[], &[], library_mix(60, 39, 1));
    let sections = compose_affinity_sections(&all_medium_items(12), &profile, 20);
    let anime_count = composed_mediums(&sections)
        .into_iter()
        .filter(|medium| *medium == DiscoveryMedium::Anime)
        .count();

    assert_eq!(
        anime_count, DISCOVERY_MEDIUM_SHARE_MIN_SLOTS,
        "one owned anime must buy exactly the slot floor"
    );
}

#[test]
fn an_anime_library_is_not_shown_western_animation() {
    // The mirror of the zero rule: sharing the `animation` genre facet does not
    // make Pixar an acceptable substitute for anime.
    let profile = affinity_profile(&[], &[], library_mix(0, 0, 50));
    let sections = compose_affinity_sections(&all_medium_items(12), &profile, 20);

    assert!(!sections.is_empty(), "the anime rails should survive");
    assert!(
        !composed_mediums(&sections).contains(&DiscoveryMedium::Animation),
        "an anime-only library was shown Western animation"
    );
}

#[test]
fn a_pixar_library_is_not_shown_anime() {
    let profile = affinity_profile(&[], &[], library_mix(0, 50, 0));
    let sections = compose_affinity_sections(&all_medium_items(12), &profile, 20);

    assert!(!sections.is_empty(), "the animation rails should survive");
    assert!(
        !composed_mediums(&sections).contains(&DiscoveryMedium::Anime),
        "a Western-animation library was shown anime"
    );
}

#[test]
fn a_zero_anime_crime_library_gets_crime_rails_with_no_anime() {
    // The plan's worked example: a library of live-action crime drama. The
    // Crime rail is exactly what this user should get, and anime is exactly
    // what they should not.
    let owned = [
        "Peaky Blinders",
        "Narcos",
        "Better Call Saul",
        "The Wire",
        "Heat",
        "Sicario",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| {
        let mut title = test_title(
            &format!("crime-{index}"),
            name,
            MediaFacet::Series,
            Vec::new(),
        );
        title.canonical_tags = vec![provider_genre_tag("Crime"), provider_genre_tag("Drama")];
        title
    })
    .collect::<Vec<_>>();
    let profile = discovery_library_affinity_profile_from_titles(&owned, true);

    assert_eq!(profile.medium_mix, library_mix(6, 0, 0));
    assert!(profile.genre_labels.iter().any(|label| label == "Crime"));

    let mut items = medium_items("crime", DiscoveryMedium::LiveAction, 12);
    items.extend(medium_items("anime", DiscoveryMedium::Anime, 12));
    let sections = compose_affinity_sections(&items, &profile, 20);

    assert_eq!(
        sections
            .iter()
            .map(|section| section.section_type.as_str())
            .collect::<Vec<_>>(),
        vec!["FOR_YOU"],
        "six owned titles are below the reason-rail budget"
    );
    assert!(
        !composed_mediums(&sections).contains(&DiscoveryMedium::Anime),
        "a zero-anime crime library was shown anime"
    );
}

#[test]
fn the_medium_gate_can_be_rolled_back_with_a_setting() {
    // The levers exist because a gate this strong should be reversible without a
    // downgrade, not because the gate is in doubt.
    let profile = affinity_profile(&[], &[], library_mix(40, 20, 0));
    let items = all_medium_items(12);

    let gated = compose_affinity_sections(&items, &profile, 20);
    assert!(!composed_mediums(&gated).contains(&DiscoveryMedium::Anime));

    let ungated = compose_affinity_sections_with_settings(
        &items,
        &profile,
        DiscoveryRailCompositionSettings {
            medium_affinity_gate: false,
            ..DiscoveryRailCompositionSettings::default()
        },
        20,
    );
    assert!(
        composed_mediums(&ungated).contains(&DiscoveryMedium::Anime),
        "discovery.medium_affinity_gate=false must restore the old behaviour"
    );
}

// ── Ordering ─────────────────────────────────────────────────────────────────

/// The plan's section 3 fixture, in fixture form: four tag-inflated anime whose
/// only strength is a single strong edge, three live-action crime titles SMG
/// actually rates relevant, and an evidence-less alphabetical tail.
fn tag_inflation_fixture() -> Vec<DiscoveryItemRecord> {
    let mut items = Vec::new();
    for (index, title) in ["Aoi", "Akira Nights", "Ayakashi", "Azumi"]
        .iter()
        .enumerate()
    {
        let mut item = medium_item(&format!("inflated-{index}"), DiscoveryMedium::Anime);
        item.display_title = (*title).to_string();
        item.sort_title = Some((*title).to_string());
        // A single strong edge, and nothing SMG's blended relevance believes in.
        item.rank_score = Some(90.0);
        item.recommendation_score = Some(0.05);
        item.matched_subject_count = 1;
        items.push(item);
    }
    for (index, title) in ["Sicario", "Heat", "The Departed"].iter().enumerate() {
        let mut item = medium_item(&format!("crime-{index}"), DiscoveryMedium::LiveAction);
        item.display_title = (*title).to_string();
        item.sort_title = Some((*title).to_string());
        item.rank_score = Some(12.0);
        item.recommendation_score = Some(0.9 - index as f64 * 0.01);
        item.matched_subject_count = 3;
        items.push(item);
    }
    // Ordinary edge-backed live action, so the rail clears its minimum without
    // reaching into the tail.
    for index in 0..6 {
        let mut item = medium_item(&format!("filler-{index}"), DiscoveryMedium::LiveAction);
        item.display_title = format!("Zed {index}");
        item.sort_title = Some(item.display_title.clone());
        item.rank_score = Some(8.0);
        item.recommendation_score = Some(0.4 - index as f64 * 0.01);
        item.matched_subject_count = 2;
        items.push(item);
    }
    // The alphabetical tail: reached by a bare semantic match, no ratings, no
    // standing. This is the band that used to fill rails to the limit.
    for (index, title) in ["Aaron", "Abacus", "Abandon", "Abbey", "Abide"]
        .iter()
        .enumerate()
    {
        let mut item = medium_item(&format!("tail-{index}"), DiscoveryMedium::LiveAction);
        item.display_title = (*title).to_string();
        item.sort_title = Some((*title).to_string());
        item.rank_score = Some(1.0);
        item.recommendation_score = Some(0.0);
        item.matched_subject_count = 1;
        items.push(item);
    }
    items
}

#[test]
fn relevance_order_leads_with_edge_backed_titles_not_tag_inflated_ones() {
    let profile = affinity_profile(&[], &[], library_mix(60, 10, 10));
    let sections = compose_affinity_sections(&tag_inflation_fixture(), &profile, 12);
    let for_you = sections
        .iter()
        .find(|section| section.section_type == "FOR_YOU")
        .expect("for you rail");

    assert_eq!(
        affinity_section_lead(for_you, 3),
        vec!["Sicario", "Heat", "The Departed"],
        "relevance, not a single strong edge, must lead the rail"
    );
    assert!(
        for_you
            .items
            .iter()
            .all(|item| !item.display_title.starts_with("Ab")),
        "the evidence-less alphabetical tail must not be on the rail: {:?}",
        affinity_section_titles(for_you)
    );
}

#[test]
fn no_rail_falls_into_alphabetical_order_when_scores_differ() {
    // A rail sorted by title is a rail that has given up. Whenever the items
    // carry distinguishable scores, something other than the title must have
    // decided the order.
    let profile = affinity_profile(&[], &[], library_mix(60, 10, 10));
    let sections = compose_affinity_sections(&tag_inflation_fixture(), &profile, 12);

    assert!(!sections.is_empty());
    for section in &sections {
        let titles = affinity_section_titles(section)
            .into_iter()
            .take(10)
            .collect::<Vec<_>>();
        let scores_differ = section
            .items
            .iter()
            .take(10)
            .map(|item| comparable_finite_f64(item.recommendation_score).to_bits())
            .collect::<HashSet<_>>()
            .len()
            > 1;
        if !scores_differ {
            continue;
        }
        let mut sorted = titles.clone();
        sorted.sort_unstable();
        assert_ne!(
            titles, sorted,
            "section {} fell into alphabetical order",
            section.section_id
        );
    }
}

// ── Corroboration ────────────────────────────────────────────────────────────

#[test]
fn a_genre_rail_rejects_a_tag_only_genre_and_accepts_a_provider_one() {
    let mut tag_only = affinity_test_item("tag-only", "series", &["canonical:genre:crime"]);
    tag_only.canonical_tags = vec![community_tag_genre_tag("Crime")];
    assert!(
        !discovery_item_matches_affinity_label(&tag_only, "Crime", "genre", true),
        "a lone community tag must not put a title on a genre rail"
    );
    assert!(
        discovery_item_matches_affinity_label(&tag_only, "Crime", "genre", false),
        "and the rollback lever must restore the old behaviour"
    );

    let provider = affinity_test_item("provider", "series", &["canonical:genre:crime"]);
    assert!(
        discovery_item_matches_affinity_label(&provider, "Crime", "genre", true),
        "a provider genre list is corroboration"
    );

    let mut two_sources = affinity_test_item("two-sources", "series", &["canonical:genre:crime"]);
    two_sources.canonical_tags = vec![CanonicalMediaTag {
        sources: vec!["anilist".to_string(), "mal".to_string()],
        source_tag_keys: vec!["anilist:tag:crime".to_string(), "mal:tag:crime".to_string()],
        ..community_tag_genre_tag("Crime")
    }];
    assert!(
        discovery_item_matches_affinity_label(&two_sources, "Crime", "genre", true),
        "two independent sources are corroboration too"
    );
}

#[test]
fn a_low_confidence_genre_is_never_corroborated() {
    let guess = CanonicalMediaTag {
        confidence: Some(0.85),
        ..provider_genre_tag("Crime")
    };
    assert!(
        !canonical_genre_tag_is_corroborated(&guess),
        "a phrase-sequence guess must not reach a rail"
    );
}

#[test]
fn an_anilist_tag_only_genre_never_becomes_a_rail_label() {
    let mut titles = Vec::new();
    for index in 0..20 {
        let mut title = test_title(
            &format!("t{index}"),
            &format!("Title {index}"),
            MediaFacet::Series,
            Vec::new(),
        );
        title.canonical_tags = vec![
            community_tag_genre_tag("Crime"),
            provider_genre_tag("Drama"),
        ];
        titles.push(title);
    }

    let profile = discovery_library_affinity_profile_from_titles(&titles, true);
    assert!(
        !profile.genre_labels.iter().any(|label| label == "Crime"),
        "an uncorroborated genre became a rail label: {:?}",
        profile.genre_labels
    );
    assert!(
        profile.genre_labels.iter().any(|label| label == "Drama"),
        "the corroborated genre should still be there: {:?}",
        profile.genre_labels
    );
}

// ── Evidence floors ──────────────────────────────────────────────────────────

#[test]
fn a_label_below_the_support_floor_is_skipped_for_the_next_one() {
    // Three crime titles in a thirty-title library is not a crime library.
    let mut titles = Vec::new();
    for index in 0..30 {
        let mut title = test_title(
            &format!("t{index}"),
            &format!("Title {index}"),
            MediaFacet::Movie,
            Vec::new(),
        );
        title.canonical_tags = vec![provider_genre_tag("Drama")];
        if index < 3 {
            title.canonical_tags.push(provider_genre_tag("Crime"));
        }
        if index < 10 {
            title.canonical_tags.push(provider_genre_tag("Thriller"));
        }
        titles.push(title);
    }

    assert_eq!(
        discovery_rail_label_support_floor(titles.len()),
        DISCOVERY_RAIL_LABEL_SUPPORT_MIN_CARRIERS
    );
    let profile = discovery_library_affinity_profile_from_titles(&titles, true);
    assert_eq!(
        profile.genre_labels,
        vec!["Drama".to_string(), "Thriller".to_string()],
        "the under-supported label must be skipped, not promoted"
    );
}

#[test]
fn the_support_floor_scales_with_the_library() {
    assert_eq!(discovery_rail_label_support_floor(0), 3);
    assert_eq!(discovery_rail_label_support_floor(10), 3);
    assert_eq!(discovery_rail_label_support_floor(50), 5);
    assert_eq!(discovery_rail_label_support_floor(300), 30);
}

#[test]
fn a_rail_is_emitted_short_rather_than_padded_and_dropped_below_the_minimum() {
    let profile = affinity_profile(&[], &[], library_mix(60, 0, 0));

    // Nine credible items and a limit of twenty: the rail ships short rather
    // than reaching into the evidence-less tail to fill itself.
    let mut items = medium_items("live", DiscoveryMedium::LiveAction, 9);
    items.extend(
        medium_items("tail", DiscoveryMedium::LiveAction, 10)
            .into_iter()
            .map(|mut item| {
                item.rank_score = Some(1.0);
                item.matched_subject_count = 1;
                item
            }),
    );
    let sections = compose_affinity_sections(&items, &profile, 20);
    let for_you = sections
        .iter()
        .find(|section| section.section_type == "FOR_YOU")
        .expect("for you rail");
    assert_eq!(for_you.items.len(), 9, "the rail must not pad itself");

    // Seven is below the floor, so there is no rail at all.
    let sections = compose_affinity_sections(
        &medium_items("live", DiscoveryMedium::LiveAction, 7),
        &profile,
        20,
    );
    assert!(
        sections.is_empty(),
        "a rail below the minimum must be dropped, not shipped: {:?}",
        sections
            .iter()
            .map(|section| section.section_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_obscure_item_is_excluded_and_a_credible_one_is_kept() {
    let mut obscure = medium_item("obscure", DiscoveryMedium::LiveAction);
    obscure.rank_score = Some(1.0);
    obscure.rating = Some(9.4);
    obscure.rating_sources = vec!["trakt".to_string()];
    obscure.external_ratings = vec![TitleExternalRating {
        source: "trakt".to_string(),
        normalized: 9.4,
        votes: Some(15),
        ..TitleExternalRating::default()
    }];
    assert!(
        !discovery_home_item_passes_evidence_floor(&obscure, &HashMap::new(), None),
        "fifteen votes on one provider is not evidence"
    );

    let mut credible = obscure.clone();
    credible.external_ratings = vec![TitleExternalRating {
        source: "imdb".to_string(),
        normalized: 7.8,
        votes: Some(90_000),
        ..TitleExternalRating::default()
    }];
    credible.rating_sources = vec!["imdb".to_string()];
    assert!(
        discovery_home_item_passes_evidence_floor(&credible, &HashMap::new(), None),
        "a well-voted rating is evidence even without an edge"
    );
}

#[test]
fn popularity_at_or_above_the_pool_median_clears_the_floor() {
    let mut items = Vec::new();
    for (index, base_rank) in [10.0, 20.0, 30.0, 40.0, 50.0].iter().enumerate() {
        let mut item = medium_item(&format!("pop-{index}"), DiscoveryMedium::LiveAction);
        item.rank_score = Some(1.0);
        item.base_rank = Some(*base_rank);
        items.push(item);
    }
    let median = discovery_pool_median_base_rank(&items).expect("median");
    assert_eq!(median, 30.0);

    assert!(discovery_home_item_passes_evidence_floor(
        &items[2],
        &HashMap::new(),
        Some(median)
    ));
    assert!(!discovery_home_item_passes_evidence_floor(
        &items[0],
        &HashMap::new(),
        Some(median)
    ));
}

#[test]
fn a_library_below_the_budget_gets_for_you_and_nothing_else() {
    let mut titles = Vec::new();
    for index in 0..10 {
        let mut title = test_title(
            &format!("t{index}"),
            &format!("Title {index}"),
            MediaFacet::Movie,
            Vec::new(),
        );
        title.canonical_tags = vec![provider_genre_tag("Crime")];
        titles.push(title);
    }

    // Ten titles: one theme rail and one genre rail are allowed.
    let profile = discovery_library_affinity_profile_from_titles(&titles, true);
    assert_eq!(discovery_rail_label_budget(profile.owned_title_count), 1);

    // Nine: FOR_YOU only, whatever the labels say.
    titles.pop();
    let profile = discovery_library_affinity_profile_from_titles(&titles, true);
    assert_eq!(discovery_rail_label_budget(profile.owned_title_count), 0);
    let sections = compose_affinity_sections(
        &medium_items("crime", DiscoveryMedium::LiveAction, 12),
        &profile,
        20,
    );
    assert_eq!(
        sections
            .iter()
            .map(|section| section.section_type.as_str())
            .collect::<Vec<_>>(),
        vec!["FOR_YOU"],
        "a nine-title library has not earned a reason rail"
    );
}

#[test]
fn the_rail_budget_widens_only_as_the_library_earns_it() {
    assert_eq!(discovery_rail_label_budget(0), 0);
    assert_eq!(discovery_rail_label_budget(9), 0);
    assert_eq!(discovery_rail_label_budget(10), 1);
    assert_eq!(discovery_rail_label_budget(49), 1);
    assert_eq!(
        discovery_rail_label_budget(50),
        DISCOVERY_RAIL_LADDER_LABELS
    );
}

#[test]
fn library_growth_earns_a_fresh_snapshot_only_when_it_changes_the_library() {
    // A fresh install: one subject at the first snapshot, six by the time the
    // scheduler next wakes. That is a different library.
    assert!(discovery_library_growth_warrants_snapshot(1, 6));
    // Growth past the label-support floor, in absolute terms.
    assert!(discovery_library_growth_warrants_snapshot(200, 240));
    // A twentieth of a large library is neither a floor nor a quarter.
    assert!(!discovery_library_growth_warrants_snapshot(200, 210));
    // ...or a quarter of what was submitted last time.
    assert!(discovery_library_growth_warrants_snapshot(8, 10));
    // One new title out of ten is not.
    assert!(!discovery_library_growth_warrants_snapshot(10, 11));
    // Neither is standing still, nor shrinking.
    assert!(!discovery_library_growth_warrants_snapshot(10, 10));
    assert!(!discovery_library_growth_warrants_snapshot(10, 4));
    // A library arriving from nothing always warrants one.
    assert!(discovery_library_growth_warrants_snapshot(0, 3));
}

// ── Context fingerprint ──────────────────────────────────────────────────────

#[test]
fn the_context_fingerprint_changes_when_the_medium_mix_changes() {
    let live_action = test_title(
        "t1",
        "Live Action",
        MediaFacet::Movie,
        vec![("tmdb_movie", "1")],
    );
    let mut anime = test_title("t2", "Anime", MediaFacet::Anime, vec![("tmdb_movie", "2")]);
    anime.canonical_tags = vec![provider_genre_tag("Anime")];

    let defaults = DiscoveryContextDefaults::default();
    let without_anime =
        build_discovery_library_context(std::slice::from_ref(&live_action), defaults.clone());
    let with_anime = build_discovery_library_context(&[live_action, anime], defaults);

    assert_eq!(without_anime.medium_mix, library_mix(1, 0, 0));
    assert_eq!(with_anime.medium_mix, library_mix(1, 0, 1));
    assert_ne!(
        without_anime.fingerprint, with_anime.fingerprint,
        "a library that gains its first anime must invalidate the cached run"
    );
}

#[test]
fn the_medium_mix_is_sent_on_every_submission() {
    let mut anime = test_title("t2", "Anime", MediaFacet::Anime, vec![("tmdb_movie", "2")]);
    anime.canonical_tags = vec![provider_genre_tag("Anime")];
    let defaults = DiscoveryContextDefaults::default();
    let context = build_discovery_library_context(
        &[
            test_title(
                "t1",
                "Live Action",
                MediaFacet::Movie,
                vec![("tmdb_movie", "1")],
            ),
            anime,
        ],
        defaults.clone(),
    );

    let submit = context.snapshot_submit_input(&defaults);
    assert_eq!(
        submit.medium_mix,
        DiscoveryContextMediumMixInput {
            live_action: 1,
            animation: 0,
            anime: 1,
        }
    );

    let changes = context
        .incremental_changes_input(&defaults, &[], "blake3:previous")
        .expect("incremental input");
    assert_eq!(changes.medium_mix, submit.medium_mix);
}

#[test]
fn a_gateway_title_without_relevance_fields_still_deserializes() {
    // `DiscoveryTitle` is shared by every gateway document. Only the context
    // snapshot and context changes fragments request `recommendation_score`
    // and `base_rank`; the public feed, More Like This and collection
    // completion documents do not, and a required field there would take
    // those surfaces down with a deserialization error the mocked gateways in
    // this suite never produce.
    let mut value = serde_json::to_value(DiscoveryTitle {
        recommendation_score: Some(0.7),
        base_rank: Some(12.0),
        ..DiscoveryTitle::default()
    })
    .expect("discovery title should serialize");
    let object = value
        .as_object_mut()
        .expect("discovery title should serialize as an object");
    object.remove("recommendation_score");
    object.remove("base_rank");

    let title: DiscoveryTitle =
        serde_json::from_value(value).expect("a title without relevance fields must deserialize");
    assert_eq!(title.recommendation_score, None);
    assert_eq!(title.base_rank, None);

    // And the context documents, which do request them, carry them through
    // to the record the composer orders by.
    let record = discovery_item_record(
        "run",
        "run",
        "context_snapshot",
        None,
        0,
        &DiscoveryTitle {
            recommendation_score: Some(0.7),
            base_rank: Some(12.0),
            ..DiscoveryTitle::default()
        },
        &HashMap::new(),
        Utc::now(),
    )
    .expect("discovery item record should build");
    assert_eq!(record.recommendation_score, Some(0.7));
    assert_eq!(record.base_rank, Some(12.0));
}

fn test_pending_change(
    id: &str,
    change_type: &str,
    sequence: i64,
    tmdb_id: i64,
) -> DiscoveryPendingContextChangeRecord {
    let observed_at = Utc.timestamp_opt(sequence, 0).unwrap();
    DiscoveryPendingContextChangeRecord {
        id: id.to_string(),
        scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
        subject_key: Some(format!("tmdb:movie:{tmdb_id}")),
        previous_subject_key: None,
        change_type: change_type.to_string(),
        title_id: Some(id.to_string()),
        previous_title_id: None,
        library_facet: Some("movie".to_string()),
        raw_subject_json: Some(
            serde_json::json!({
                "tmdbId": tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": tmdb_id.to_string()}]
            })
            .to_string(),
        ),
        raw_previous_subject_json: None,
        first_seen_sequence: Some(sequence),
        last_seen_sequence: Some(sequence),
        first_seen_at: observed_at,
        last_seen_at: observed_at,
    }
}

fn test_title(id: &str, name: &str, facet: MediaFacet, external_ids: Vec<(&str, &str)>) -> Title {
    Title {
        id: id.to_string(),
        library_id: "library".to_string(),
        name: name.to_string(),
        facet,
        monitored: true,
        tags: Vec::new(),
        canonical_tags: vec![],
        external_ids: external_ids
            .into_iter()
            .map(|(source, value)| ExternalId {
                source: source.to_string(),
                value: value.to_string(),
            })
            .collect(),
        root_folder_id: "root".to_string(),
        created_by: None,
        created_at: Utc.timestamp_opt(0, 0).unwrap(),
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
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

#[test]
fn discovery_home_public_hero_prefers_multi_source_rating_over_single_source_fossil() {
    // Both carry a hero backdrop and an equal, healthy rank_score, so ordering
    // falls through to the credible-rating tiebreak. The single-source 10.0
    // must lose to the multi-source 9.4.
    let mut fossil = test_discovery_item("fossil", "movie", Some("movie"));
    fossil.source_run_kind = "public_feed".to_string();
    fossil.target_key = "tmdb:movie:fossil".to_string();
    fossil.rating = Some(10.0);
    fossil.rating_sources = vec!["trakt".to_string()];
    fossil.rank_score = Some(50.0);
    fossil.background_url = Some("https://images.example/fossil.jpg".to_string());

    let mut credible = test_discovery_item("credible", "movie", Some("movie"));
    credible.source_run_kind = "public_feed".to_string();
    credible.target_key = "tmdb:movie:credible".to_string();
    credible.rating = Some(9.4);
    credible.rating_sources = vec!["imdb".to_string(), "tmdb".to_string(), "trakt".to_string()];
    credible.rank_score = Some(50.0);
    credible.background_url = Some("https://images.example/credible.jpg".to_string());

    let hero = select_public_discovery_home_hero(&[test_discovery_section(
        "public",
        vec![fossil, credible],
    )])
    .expect("public hero item");

    assert_eq!(hero.target_key, "tmdb:movie:credible");
}

#[test]
fn discovery_home_top_rated_demotes_single_source_fossil_below_multi_source() {
    let mut fossil = test_discovery_item("fossil", "movie", Some("movie"));
    fossil.source_run_kind = "public_feed".to_string();
    fossil.target_key = "tmdb:movie:fossil".to_string();
    fossil.rating = Some(10.0);
    fossil.rating_sources = vec!["trakt".to_string()];
    fossil.external_ratings = vec![TitleExternalRating {
        source: "trakt".to_string(),
        value: Some(10.0),
        score: Some(10.0),
        normalized: 1.0,
        votes: Some(1),
        url: String::new(),
    }];

    let mut credible = test_discovery_item("credible", "movie", Some("movie"));
    credible.source_run_kind = "public_feed".to_string();
    credible.target_key = "tmdb:movie:credible".to_string();
    credible.rating = Some(9.4);
    credible.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];
    credible.external_ratings = vec![
        TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(9.4),
            score: Some(9.4),
            normalized: 0.94,
            votes: Some(500_000),
            url: String::new(),
        },
        TitleExternalRating {
            source: "tmdb".to_string(),
            value: Some(9.2),
            score: Some(9.2),
            normalized: 0.92,
            votes: Some(120_000),
            url: String::new(),
        },
    ];

    let section = top_rated_discovery_home_section(&[fossil, credible], &[], true, 10)
        .expect("top rated section");

    assert_eq!(
        section
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>(),
        vec!["tmdb:movie:credible", "tmdb:movie:fossil"]
    );
}

#[test]
fn discovery_home_top_rated_keeps_vote_backed_single_source_above_multi_source() {
    // A MAL-only anime score built from hundreds of thousands of votes is
    // credible evidence: it must not be demoted below a lower multi-source
    // score just because only one provider carries it.
    let mut mal_only = test_discovery_item("mal-only", "series", Some("anime"));
    mal_only.source_run_kind = "public_feed".to_string();
    mal_only.target_key = "tvdb:series:mal-only".to_string();
    mal_only.rating = Some(9.3);
    mal_only.rating_sources = vec!["mal".to_string()];
    mal_only.external_ratings = vec![TitleExternalRating {
        source: "mal".to_string(),
        value: Some(9.3),
        score: Some(9.3),
        normalized: 0.93,
        votes: Some(500_000),
        url: String::new(),
    }];

    let mut multi_source = test_discovery_item("multi", "movie", Some("movie"));
    multi_source.source_run_kind = "public_feed".to_string();
    multi_source.target_key = "tmdb:movie:multi".to_string();
    multi_source.rating = Some(7.8);
    multi_source.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];
    multi_source.external_ratings = vec![
        TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(7.8),
            score: Some(7.8),
            normalized: 0.78,
            votes: Some(90_000),
            url: String::new(),
        },
        TitleExternalRating {
            source: "tmdb".to_string(),
            value: Some(7.6),
            score: Some(7.6),
            normalized: 0.76,
            votes: Some(4_000),
            url: String::new(),
        },
    ];

    let section = top_rated_discovery_home_section(&[multi_source, mal_only], &[], true, 10)
        .expect("top rated section");

    assert_eq!(
        section
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>(),
        vec!["tvdb:series:mal-only", "tmdb:movie:multi"]
    );
}

#[test]
fn discovery_home_top_rated_prefers_vote_backed_score_over_scoreless_source_names() {
    // During the per-source rating rollout an item can carry source names and
    // a summary rating but no external rating rows. Its bare source-name
    // count must not hoist it above a vote-backed real external score.
    let mut names_only = test_discovery_item("names-only", "movie", Some("movie"));
    names_only.source_run_kind = "public_feed".to_string();
    names_only.target_key = "tmdb:movie:names-only".to_string();
    names_only.rating = Some(7.0);
    names_only.rating_sources = vec!["imdb".to_string(), "tmdb".to_string()];

    let mut vote_backed = test_discovery_item("vote-backed", "movie", Some("movie"));
    vote_backed.source_run_kind = "public_feed".to_string();
    vote_backed.target_key = "tmdb:movie:vote-backed".to_string();
    vote_backed.rating = Some(9.2);
    vote_backed.rating_sources = vec!["imdb".to_string()];
    vote_backed.external_ratings = vec![TitleExternalRating {
        source: "imdb".to_string(),
        value: Some(9.2),
        score: Some(9.2),
        normalized: 0.92,
        votes: Some(80_000),
        url: String::new(),
    }];

    let section = top_rated_discovery_home_section(&[names_only, vote_backed], &[], true, 10)
        .expect("top rated section");

    assert_eq!(
        section
            .items
            .iter()
            .map(|item| item.target_key.as_str())
            .collect::<Vec<_>>(),
        vec!["tmdb:movie:vote-backed", "tmdb:movie:names-only"]
    );
}

#[test]
fn discovery_rating_source_aliases_collapse_to_one_provider() {
    // Alias spellings of a single provider must not fake corroboration.
    let mut aliased = test_discovery_item("aliased", "series", Some("anime"));
    aliased.rating = Some(10.0);
    aliased.rating_sources = vec![
        "mal".to_string(),
        "MyAnimeList".to_string(),
        "MyAnimeList.net".to_string(),
    ];

    assert_eq!(discovery_item_distinct_rating_source_count(&aliased), 1);
    assert!(!discovery_item_has_credible_rating_evidence(&aliased));
}

#[test]
fn discovery_comparators_tolerate_nan_scores() {
    // A NaN score must collapse to the missing-value ordering instead of
    // producing a non-total comparator (which can panic sort_by).
    assert_eq!(
        compare_optional_f64_desc(Some(f64::NAN), Some(5.0)),
        compare_optional_f64_desc(Some(0.0), Some(5.0))
    );
    assert_eq!(
        compare_optional_f64_desc(Some(f64::NAN), Some(f64::NAN)),
        Ordering::Equal
    );

    let mut nan_rated = test_discovery_item("nan-rated", "movie", Some("movie"));
    nan_rated.rating = Some(f64::NAN);
    assert_eq!(discovery_item_comparable_rating(&nan_rated), 0.0);
}

fn test_discovery_item(
    id: &str,
    target_kind: &str,
    content_type: Option<&str>,
) -> DiscoveryItemRecord {
    let now = Utc.timestamp_opt(0, 0).unwrap();
    DiscoveryItemRecord {
        id: id.to_string(),
        run_id: "run-1".to_string(),
        base_generation_id: Some("run-1".to_string()),
        source_run_kind: "context_snapshot".to_string(),
        section_id: None,
        sort_index: 0,
        target_key: format!("{target_kind}:{id}"),
        target_kind: target_kind.to_string(),
        resolved: true,
        resolved_title_id: None,
        display_title: "Example".to_string(),
        original_title: None,
        sort_title: Some("Example".to_string()),
        year: None,
        poster_path: None,
        poster_url: None,
        background_url: None,
        overview: None,
        content_type: content_type.map(str::to_string),
        canonical_tags: vec![],
        is_adult: false,
        content_ratings: Vec::new(),
        rating: None,
        rating_sources: Vec::new(),
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: Vec::new(),
        best_source: None,
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: None,
        edge_count: None,
        relation_count: None,
        source_subject_count: None,
        rank_score: None,
        recommendation_score: None,
        base_rank: None,
        matched_subject_keys: Vec::new(),
        matched_subject_titles: Vec::new(),
        matched_subject_count: 0,
        library_provenance: Vec::new(),
        tmdb_collection_id: None,
        tmdb_collection_name: None,
        owned_in_input: false,
        studio_slug: None,
        person_ids: Vec::new(),
        facet_terms: Vec::new(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: now,
        updated_at: now,
    }
}

fn test_discovery_section(
    surface: &str,
    items: Vec<DiscoveryItemRecord>,
) -> DiscoverySectionResult {
    DiscoverySectionResult {
        section_id: format!("{surface}_section"),
        section_type: "TEST".to_string(),
        title: "Test".to_string(),
        surface: surface.to_string(),
        total_count: items.len() as i64,
        items,
    }
}
