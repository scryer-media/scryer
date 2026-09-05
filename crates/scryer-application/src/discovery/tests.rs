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
fn discovery_item_records_wire_canonical_genre_and_theme_terms() {
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
        item.rank_score = Some(rank_score);
        item.matched_subject_count = matched_subject_count;
        item
    }

    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels: vec!["Adventure".to_string(), "Animation".to_string()],
        theme_labels: Vec::new(),
    };
    let items = vec![
        discovery_item("1", "Shared Match", &["Adventure", "Animation"], 100.0, 1),
        discovery_item("2", "Unlinked Animation", &["Animation"], 95.0, 0),
        discovery_item("3", "Adventure Match", &["Adventure"], 90.0, 1),
        discovery_item("4", "Animation Match", &["Animation"], 80.0, 1),
    ];

    let sections = personalized_section_results(&items, &profile, true, 10);
    let adventure = sections
        .iter()
        .find(|section| section.title == "Because You Like Adventure")
        .expect("adventure section");
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation section");

    assert_eq!(
        adventure
            .items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect::<Vec<_>>(),
        vec!["Shared Match", "Adventure Match"]
    );
    assert_eq!(
        animation
            .items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect::<Vec<_>>(),
        vec!["Animation Match"]
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
    item
}

fn affinity_section_titles(section: &DiscoverySectionResult) -> Vec<&str> {
    section
        .items
        .iter()
        .map(|item| item.display_title.as_str())
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
    let profile = DiscoveryLibraryAffinityProfile::default();
    let mut items = Vec::new();
    for index in 0..8 {
        items.push(affinity_test_item(&format!("m{index}"), "movie", &[]));
        items.push(affinity_test_item(&format!("s{index}"), "series", &[]));
        items.push(affinity_test_item(&format!("a{index}"), "anime", &[]));
    }

    let sections = personalized_section_results(&items, &profile, true, 5);
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
    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels: vec!["Animation".to_string()],
        theme_labels: vec!["Isekai".to_string()],
    };
    let items = vec![
        affinity_test_item(
            "1",
            "movie",
            &["canonical:genre:animation", "canonical:theme:isekai"],
        ),
        affinity_test_item("2", "movie", &["canonical:genre:animation"]),
    ];

    let sections = personalized_section_results(&items, &profile, true, 10);
    let isekai = sections
        .iter()
        .find(|section| section.title == "Because You Like Isekai")
        .expect("isekai theme section");
    assert_eq!(isekai.section_type, "BECAUSE_YOU_LIKE_TAG");
    assert_eq!(affinity_section_titles(isekai), vec!["Title 1"]);

    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(animation.section_type, "BECAUSE_YOU_LIKE_GENRE");
    assert_eq!(affinity_section_titles(animation), vec!["Title 2"]);

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
    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels: vec!["Animation".to_string(), "Anime".to_string()],
        theme_labels: Vec::new(),
    };
    let items = vec![
        affinity_test_item("western", "movie", &["canonical:genre:animation"]),
        affinity_test_item(
            "shonen",
            "anime",
            &["canonical:genre:animation", "canonical:genre:anime"],
        ),
    ];

    let sections = personalized_section_results(&items, &profile, true, 10);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_titles(animation), vec!["Title western"]);

    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section");
    assert_eq!(affinity_section_titles(anime), vec!["Title shonen"]);
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

    let titles = vec![bag_only.clone(), bag_only];
    let theme_labels = top_owned_title_labels(
        &titles,
        |title| canonical_tag_labels(&title.canonical_tags, "theme"),
        2,
    );
    assert_eq!(theme_labels, vec!["Slow Burn".to_string()]);
    assert!(
        !theme_labels.iter().any(|label| label == "isekai"),
        "a user tag must never reach the affinity profile: {theme_labels:?}"
    );

    // And the profile struct itself no longer has anywhere to put one.
    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels: Vec::new(),
        theme_labels,
    };
    let items = vec![affinity_test_item(
        "slow",
        "series",
        &["canonical:theme:slow-burn"],
    )];
    let sections = personalized_section_results(&items, &profile, true, 10);
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
            .map(|name| CanonicalMediaTag {
                key: format!("canonical:genre:{}", name.to_ascii_lowercase()),
                category: "genre".to_string(),
                name: name.to_string(),
                confidence: None,
                sources: Vec::new(),
                source_tag_keys: Vec::new(),
                is_adult: false,
                is_spoiler: false,
            })
            .collect();
        title
    }

    let titles = vec![anime_title("a1"), anime_title("a2")];
    let genre_labels = top_owned_title_labels(
        &titles,
        |title| canonical_tag_labels(&title.canonical_tags, "genre"),
        2,
    );
    assert!(
        genre_labels.iter().any(|label| label == "Anime"),
        "anime-heavy library produced no Anime label: {genre_labels:?}"
    );

    // ...and the label, once reachable, splits the two traditions apart.
    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels,
        theme_labels: Vec::new(),
    };
    let items = vec![
        affinity_test_item("western", "movie", &["canonical:genre:animation"]),
        affinity_test_item(
            "shonen",
            "anime",
            &["canonical:genre:animation", "canonical:genre:anime"],
        ),
    ];
    let sections = personalized_section_results(&items, &profile, true, 10);
    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section should be reachable from a real profile");
    assert_eq!(affinity_section_titles(anime), vec!["Title shonen"]);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_titles(animation), vec!["Title western"]);
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
    untyped_anime.matched_subject_count = 1;
    assert_eq!(discovery_item_media_kind(&untyped_anime), Some("series"));

    let profile = DiscoveryLibraryAffinityProfile {
        genre_labels: vec!["Animation".to_string(), "Anime".to_string()],
        theme_labels: Vec::new(),
    };
    let items = vec![
        affinity_test_item("western", "movie", &["canonical:genre:animation"]),
        untyped_anime,
    ];

    let sections = personalized_section_results(&items, &profile, true, 10);
    let animation = sections
        .iter()
        .find(|section| section.title == "Because You Like Animation")
        .expect("animation genre section");
    assert_eq!(affinity_section_titles(animation), vec!["Title western"]);
    let anime = sections
        .iter()
        .find(|section| section.title == "Because You Like Anime")
        .expect("anime genre section");
    assert_eq!(affinity_section_titles(anime), vec!["Title untyped"]);
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
