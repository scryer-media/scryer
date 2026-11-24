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
    assert_eq!(group.label_key.as_deref(), Some("trending_now"));
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
fn discovery_projected_context_preserves_identity_provenance_and_medium_mix() {
    let mut animation = test_title(
        "animation",
        "Animated",
        MediaFacet::Movie,
        vec![("tmdb_movie", "10")],
    );
    animation.canonical_tags = vec![provider_genre_tag(" Animation ")];
    let mut duplicate = animation.clone();
    duplicate.id = "duplicate".into();
    duplicate.library_id = "other-library".into();
    let mut anime = test_title(
        "anime",
        "Anime",
        MediaFacet::Series,
        vec![("thetvdb", "tvdb:123")],
    );
    anime.canonical_tags = vec![provider_genre_tag("Animation"), provider_genre_tag("ANIME")];
    let titles = vec![
        animation,
        duplicate,
        anime,
        test_title("unresolved", "Unresolved", MediaFacet::Anime, vec![]),
        test_title("live", "Live", MediaFacet::Series, vec![("tmdb_tv", "456")]),
    ];
    let defaults = DiscoveryContextDefaults::default();
    let original = build_discovery_library_context(&titles, defaults.clone());
    let projected = build_projected_discovery_library_context(
        &titles
            .iter()
            .rev()
            .map(crate::DiscoveryContextTitle::from)
            .collect::<Vec<_>>(),
        defaults.clone(),
    );
    assert_eq!(projected.fingerprint, original.fingerprint);
    assert_eq!(projected.medium_mix, original.medium_mix);
    assert_eq!(
        serde_json::to_value(projected.snapshot_submit_input(&defaults)).unwrap(),
        serde_json::to_value(original.snapshot_submit_input(&defaults)).unwrap()
    );
    assert_eq!(
        projected.submitted_subject_records("run").unwrap(),
        original.submitted_subject_records("run").unwrap()
    );
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
            label_key: None,
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

    let profile = affinity_profile(&["Adventure", "Fantasy"], &[], library_mix(60, 30, 10));
    let items = extend_with_fillers(
        vec![
            discovery_item("1", "Shared Match", &["Adventure", "Fantasy"], 100.0, 1),
            discovery_item("2", "Unlinked Fantasy", &["Fantasy"], 95.0, 0),
            discovery_item("3", "Adventure Match", &["Adventure"], 90.0, 1),
            discovery_item("4", "Fantasy Match", &["Fantasy"], 80.0, 1),
        ],
        vec![
            affinity_filler_items("adv", "movie", &["canonical:genre:adventure"], 8),
            affinity_filler_items("fan", "movie", &["canonical:genre:fantasy"], 8),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let adventure = sections
        .iter()
        .find(|section| section.title == "Because you like adventure")
        .expect("adventure section");
    let fantasy = sections
        .iter()
        .find(|section| section.title == "Because you like fantasy")
        .expect("fantasy section");
    assert_eq!(adventure.label_key.as_deref(), Some("adventure"));
    assert_eq!(fantasy.label_key.as_deref(), Some("fantasy"));

    assert_eq!(
        affinity_section_lead(adventure, 2),
        vec!["Shared Match", "Adventure Match"]
    );
    assert_eq!(affinity_section_lead(fantasy, 1), vec!["Fantasy Match"]);
    assert!(
        !fantasy
            .items
            .iter()
            .any(|item| item.display_title == "Unlinked Fantasy"),
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
    item.affinity_signals = facet_terms
        .iter()
        .filter_map(|term| term.strip_prefix("canonical:theme:"))
        .map(|key| crate::DiscoveryAffinitySignalRecord {
            affinity_key: format!("affinity:theme:{key}"),
            category: "theme".to_string(),
            tier: "strong".to_string(),
            confidence: 0.95,
            sources: vec!["anilist".to_string()],
            rail_eligible: true,
            ..Default::default()
        })
        .collect();
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
        &HashSet::new(),
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
    personalized_section_results(
        items,
        profile,
        &HashMap::new(),
        &HashSet::new(),
        settings,
        true,
        limit,
    )
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
    // "Because you like isekai" theme rail must not be eaten first by the
    // far broader "Because you like fantasy" genre rail.
    let profile = affinity_profile(&["Fantasy"], &["Isekai"], library_mix(60, 20, 20));
    let items = extend_with_fillers(
        vec![
            affinity_test_item(
                "1",
                "movie",
                &["canonical:genre:fantasy", "canonical:theme:isekai"],
            ),
            affinity_test_item("2", "movie", &["canonical:genre:fantasy"]),
        ],
        vec![
            affinity_filler_items(
                "isekai",
                "movie",
                &["canonical:genre:fantasy", "canonical:theme:isekai"],
                8,
            ),
            affinity_filler_items("fan", "movie", &["canonical:genre:fantasy"], 8),
        ],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let isekai = sections
        .iter()
        .find(|section| section.title == "Because you like isekai")
        .expect("isekai theme section");
    assert_eq!(isekai.section_type, "BECAUSE_YOU_LIKE_TAG");
    assert_eq!(isekai.label_key.as_deref(), Some("isekai"));
    assert_eq!(affinity_section_lead(isekai, 1), vec!["Title 1"]);

    let fantasy = sections
        .iter()
        .find(|section| section.title == "Because you like fantasy")
        .expect("fantasy genre section");
    assert_eq!(fantasy.section_type, "BECAUSE_YOU_LIKE_GENRE");
    assert_eq!(fantasy.label_key.as_deref(), Some("fantasy"));
    assert_eq!(affinity_section_lead(fantasy, 1), vec!["Title 2"]);
    assert!(
        !fantasy
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
fn affinity_label_rails_never_name_a_medium() {
    // Medium is owned by the facet chips and the medium gate, so no rail is
    // ever named after one. Even a profile that somehow carries "Animation"
    // and "Anime" composes no reason rail from them; the titles fall through
    // to the generic rail instead.
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
    assert!(
        sections
            .iter()
            .all(|section| section.section_type != "BECAUSE_YOU_LIKE_GENRE"
                && section.label_key.is_none()),
        "a medium must never name a rail: {:?}",
        sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        sections
            .iter()
            .any(|section| section.section_type == "FOR_YOU"),
        "the titles still reach the generic rail"
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
        affinity_signals: Vec::new(),
        key: "canonical:theme:heist".to_string(),
        category: "theme".to_string(),
        name: "Heist".to_string(),
        confidence: None,
        sources: Vec::new(),
        source_tag_keys: Vec::new(),
        is_adult: false,
        is_spoiler: false,
    }];

    let titles = vec![bag_only.clone(), bag_only.clone(), bag_only];
    let theme_labels = top_owned_title_labels(
        &titles,
        DiscoveryRailLabelFamily::Theme,
        |title| canonical_tag_labels(&title.canonical_tags, "theme"),
        2,
        discovery_rail_label_support_floor(titles.len()),
    );
    assert_eq!(theme_labels, vec!["Heist".to_string()]);
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
            "heist",
            "series",
            &["canonical:theme:heist"],
        )],
        vec![affinity_filler_items(
            "heist",
            "series",
            &["canonical:theme:heist"],
            8,
        )],
    );
    let sections = compose_affinity_sections(&items, &profile, 10);
    assert!(
        sections
            .iter()
            .any(|section| section.title == "Because you like heists"
                && section.label_key.as_deref() == Some("heist"))
    );
}

#[test]
fn library_profile_skips_unsayable_labels_for_the_next_allowlisted_pair() {
    // An anime library's most-carried theme is `school` and its most-carried
    // genre is `anime`. Neither is something a person says out loud, so
    // neither may name a rail: the profile skips them and takes the next
    // allowlisted labels the library supports instead.
    fn theme_tag(name: &str) -> CanonicalMediaTag {
        let slug = normalize_discovery_affinity_key(name).replace(' ', "-");
        CanonicalMediaTag {
            key: format!("canonical:theme:{slug}"),
            category: "theme".into(),
            name: name.into(),
            confidence: Some(0.95),
            sources: vec!["anilist".into()],
            affinity_signals: vec![crate::DiscoveryAffinitySignalRecord {
                affinity_key: format!("affinity:theme:{slug}"),
                category: "theme".into(),
                tier: "strong".into(),
                confidence: 0.95,
                sources: vec!["anilist".into()],
                rail_eligible: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    fn anime_title(id: &str, genres: &[&str], themes: &[&str]) -> Title {
        let mut title = test_title(id, id, MediaFacet::Series, Vec::new());
        title.canonical_tags = genres
            .iter()
            .map(|genre| provider_genre_tag(genre))
            .chain(themes.iter().map(|theme| theme_tag(theme)))
            .collect();
        title
    }

    let mut titles = Vec::new();
    for index in 0..6 {
        titles.push(anime_title(
            &format!("a{index}"),
            &["Anime", "Animation", "Drama", "Fantasy"],
            &["School", "Coming of Age", "Isekai"],
        ));
    }
    for index in 0..4 {
        titles.push(anime_title(
            &format!("b{index}"),
            &["Anime", "Animation", "Drama"],
            &["School", "Coming of Age"],
        ));
    }

    let profile = discovery_library_affinity_profile_from_titles(&titles, true);
    assert_eq!(profile.genre_labels, vec!["Drama", "Fantasy"]);
    assert_eq!(profile.theme_labels, vec!["Coming of Age", "Isekai"]);

    // ...and the rails composed from that profile read as sentences.
    let items = extend_with_fillers(
        vec![
            corroborated_genre_item("drama", "anime", &["Anime", "Drama"]),
            affinity_test_item(
                "coming",
                "anime",
                &["canonical:genre:anime", "canonical:theme:coming-of-age"],
            ),
        ],
        vec![
            corroborated_genre_fillers("drama", "anime", &["Anime", "Drama"], 8),
            affinity_filler_items(
                "coming",
                "anime",
                &["canonical:genre:anime", "canonical:theme:coming-of-age"],
                8,
            ),
        ],
    );
    let profile = affinity_profile(
        &profile
            .genre_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        &profile
            .theme_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        library_mix(0, 0, 10),
    );
    let sections = compose_affinity_sections(&items, &profile, 10);
    let coming_of_age = sections
        .iter()
        .find(|section| section.label_key.as_deref() == Some("coming-of-age"))
        .expect("coming-of-age theme section");
    assert_eq!(
        coming_of_age.title,
        "Because you like coming-of-age stories"
    );
    let drama = sections
        .iter()
        .find(|section| section.label_key.as_deref() == Some("drama"))
        .expect("drama genre section");
    assert_eq!(drama.title, "Because you like drama");
    assert!(sections.iter().all(|section| !matches!(
        section.label_key.as_deref(),
        Some("anime" | "animation" | "school")
    )));
}

/// A canonical genre in the shape SMG produces from a provider's own genre
/// list: high confidence and a `genre:` source key. This is the shape the
/// corroboration gate is supposed to accept.
fn provider_genre_tag(name: &str) -> CanonicalMediaTag {
    let slug = name.to_ascii_lowercase();
    CanonicalMediaTag {
        affinity_signals: Vec::new(),
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
        affinity_signals: Vec::new(),
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
fn anime_without_content_type_does_not_leak_into_an_animation_library_rail() {
    // `discovery_item_media_kind` falls back to target_kind, so a
    // content_type-less anime reports as a plain "series". Only the canonical
    // anime genre facet classifies it as anime, which is what keeps it off the
    // rails of a library that owns Western animation but no anime.
    let mut untyped_anime = test_discovery_item("untyped", "series", None);
    untyped_anime.target_key = "mal:anime:untyped".to_string();
    untyped_anime.display_title = "Title untyped".to_string();
    untyped_anime.sort_title = Some("Title untyped".to_string());
    untyped_anime.facet_terms = vec![
        "canonical:genre:animation".to_string(),
        "canonical:genre:anime".to_string(),
        "canonical:genre:fantasy".to_string(),
    ];
    untyped_anime.canonical_tags = vec![
        provider_genre_tag("Animation"),
        provider_genre_tag("Anime"),
        provider_genre_tag("Fantasy"),
    ];
    untyped_anime.matched_subject_count = 1;
    untyped_anime.rank_score = Some(5.0);
    assert_eq!(discovery_item_media_kind(&untyped_anime), Some("series"));
    assert_eq!(
        discovery_item_medium(&untyped_anime),
        DiscoveryMedium::Anime
    );

    let profile = affinity_profile(&["Fantasy"], &[], library_mix(40, 60, 0));
    let items = extend_with_fillers(
        vec![
            affinity_test_item(
                "western",
                "movie",
                &["canonical:genre:animation", "canonical:genre:fantasy"],
            ),
            untyped_anime,
        ],
        vec![affinity_filler_items(
            "west",
            "movie",
            &["canonical:genre:animation", "canonical:genre:fantasy"],
            8,
        )],
    );

    let sections = compose_affinity_sections(&items, &profile, 10);
    let fantasy = sections
        .iter()
        .find(|section| section.title == "Because you like fantasy")
        .expect("fantasy genre section");
    assert_eq!(affinity_section_lead(fantasy, 1), vec!["Title western"]);
    assert!(
        sections
            .iter()
            .flat_map(|section| section.items.iter())
            .all(|item| item.display_title != "Title untyped"),
        "an anime without a content type must not reach a library that owns no anime"
    );
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
            .map(|(source, value)| ExternalId::new(source.to_string(), value.to_string()))
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
        movie_release_dates: None,
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

pub(super) fn test_discovery_item(
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
        affinity_signals: Vec::new(),
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
        label_key: None,
    }
}

#[test]
fn discovery_theme_admission_follows_smg_rail_eligibility() {
    let mut item = affinity_test_item("theme-evidence", "series", &["canonical:theme:friendship"]);
    assert!(discovery_item_matches_affinity_label(
        &item,
        "Friendship",
        "theme",
        false
    ));
    // SMG's verdict is final: a rejected signal stays rejected however strong
    // its tier, confidence, or source count look to Scryer.
    item.affinity_signals[0].rail_eligible = false;
    item.affinity_signals[0].tier = "key".to_string();
    item.affinity_signals[0].confidence = 1.0;
    item.affinity_signals[0].sources = vec!["anilist".to_string(), "mal".to_string()];
    assert!(!discovery_item_matches_affinity_label(
        &item,
        "Friendship",
        "theme",
        false
    ));
    // An admitted signal is accepted without any local threshold.
    item.affinity_signals[0].rail_eligible = true;
    item.affinity_signals[0].tier = "support".to_string();
    item.affinity_signals[0].confidence = 0.1;
    item.affinity_signals[0].sources.clear();
    assert!(discovery_item_matches_affinity_label(
        &item,
        "Friendship",
        "theme",
        false
    ));
    // Eligibility for a different theme does not transfer.
    item.affinity_signals[0].affinity_key = "affinity:theme:revenge".to_string();
    assert!(!discovery_item_matches_affinity_label(
        &item,
        "Friendship",
        "theme",
        false
    ));
    item.affinity_signals.clear();
    assert!(!discovery_item_matches_affinity_label(
        &item,
        "Friendship",
        "theme",
        false
    ));
    assert!(
        canonical_affinity_labels_for_profile(&[item], &["Friendship".to_string()], "theme")
            .is_empty()
    );
}

#[test]
fn discovery_owned_themes_require_the_same_evidence_as_candidates() {
    let mut title = test_title("owned-theme", "Owned Theme", MediaFacet::Movie, vec![]);
    title.canonical_tags = vec![CanonicalMediaTag {
        key: "canonical:theme:found-family".into(),
        category: "theme".into(),
        name: "Found Family".into(),
        confidence: Some(0.95),
        sources: vec!["tmdb".into()],
        affinity_signals: vec![crate::DiscoveryAffinitySignalRecord {
            affinity_key: "affinity:theme:found-family".into(),
            category: "theme".into(),
            tier: "key".into(),
            confidence: 1.0,
            sources: vec!["tmdb".into(), "anilist".into()],
            rail_eligible: false,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let profile = |title: &Title| {
        discovery_library_affinity_profile_from_titles(&vec![title.clone(); 10], true)
    };
    assert!(profile(&title).theme_labels.is_empty());
    title.canonical_tags[0].affinity_signals[0].rail_eligible = true;
    assert_eq!(profile(&title).theme_labels, vec!["Found Family"]);
    title.canonical_tags[0].affinity_signals.clear();
    assert!(profile(&title).theme_labels.is_empty());
}

/// The gateway registers the `lgbtq` theme tag with the older signal key
/// `affinity:theme:lgbt`, so a matcher that compares the signal slug with the
/// tag name never sees it. These fixtures are shaped the way the gateway
/// serves that tag.
fn smg_lgbtq_signal(rail_eligible: bool) -> crate::DiscoveryAffinitySignalRecord {
    crate::DiscoveryAffinitySignalRecord {
        affinity_key: "affinity:theme:lgbt".into(),
        category: "theme".into(),
        tier: "support".into(),
        confidence: 0.9,
        sources: vec!["anilist".into()],
        rail_eligible,
        ..Default::default()
    }
}

fn smg_lgbtq_pool_item(item: DiscoveryItemRecord) -> DiscoveryItemRecord {
    let mut item = item;
    item.affinity_signals = vec![smg_lgbtq_signal(true)];
    item
}

#[test]
fn owned_lgbtq_titles_reach_the_profile_through_the_gateway_signal_key() {
    let mut title = test_title("owned-lgbtq", "Owned Example", MediaFacet::Movie, vec![]);
    title.canonical_tags = vec![CanonicalMediaTag {
        key: "canonical:theme:lgbtq".into(),
        category: "theme".into(),
        name: "LGBTQ".into(),
        confidence: Some(0.9),
        sources: vec!["anilist".into()],
        affinity_signals: vec![smg_lgbtq_signal(true)],
        ..Default::default()
    }];
    let profile = |title: &Title| {
        discovery_library_affinity_profile_from_titles(&vec![title.clone(); 10], true)
    };
    assert_eq!(profile(&title).theme_labels, vec!["LGBTQ"]);

    // The gateway's verdict still decides.
    title.canonical_tags[0].affinity_signals[0].rail_eligible = false;
    assert!(profile(&title).theme_labels.is_empty());

    // A secondary signal folded into the same tag is not the label's signal.
    title.canonical_tags[0].affinity_signals[0] = crate::DiscoveryAffinitySignalRecord {
        affinity_key: "affinity:theme:coming-out".into(),
        ..smg_lgbtq_signal(true)
    };
    assert!(profile(&title).theme_labels.is_empty());
}

#[test]
fn lgbtq_rail_composes_from_gateway_shaped_pool_items() {
    let profile = affinity_profile(&[], &["LGBTQ"], balanced_library_mix());
    let lead = smg_lgbtq_pool_item(affinity_test_item(
        "lgbtq-lead",
        "series",
        &["canonical:theme:lgbtq"],
    ));
    let items = extend_with_fillers(
        vec![lead],
        vec![
            affinity_filler_items("lgbtq", "series", &["canonical:theme:lgbtq"], 8)
                .into_iter()
                .map(smg_lgbtq_pool_item)
                .collect(),
        ],
    );
    let sections = compose_affinity_sections(&items, &profile, 10);
    let rail = sections
        .iter()
        .find(|section| section.label_key.as_deref() == Some("lgbtq"))
        .expect("the lgbtq rail composes from the gateway's signal key");
    assert_eq!(rail.title, "Because you like LGBTQ+ stories");
    assert_eq!(affinity_section_lead(rail, 1), vec!["Title lgbtq-lead"]);
}

#[test]
fn a_label_that_cannot_name_a_rail_never_spends_the_label_budget() {
    let candidates = HashMap::new();
    let context = DiscoveryRailContext {
        candidates_by_id: &candidates,
        settings: DiscoveryRailCompositionSettings::default(),
        medium_mix: balanced_library_mix(),
        median_base_rank: None,
        limit: 10,
    };
    let items = extend_with_fillers(
        vec![affinity_test_item(
            "budget-heist",
            "series",
            &["canonical:theme:heist"],
        )],
        vec![
            affinity_filler_items("budget-school", "series", &["canonical:theme:school"], 9),
            affinity_filler_items("budget-heist", "series", &["canonical:theme:heist"], 8),
        ],
    );
    let sections = label_affinity_sections(
        &items,
        &["School".to_string(), "Heist".to_string()],
        "theme",
        "BECAUSE_YOU_LIKE_TAG",
        "because_you_like_tag",
        1,
        &context,
        &mut HashSet::new(),
    );
    assert_eq!(
        sections
            .iter()
            .map(|section| section.label_key.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("heist")]
    );
}

#[test]
fn catalog_affinity_groups_carry_the_rail_label_and_skip_unsayable_labels() {
    let profile = affinity_profile(
        &["Anime", "Science Fiction"],
        &["School", "Heist"],
        balanced_library_mix(),
    );
    let items = extend_with_fillers(
        Vec::new(),
        vec![
            affinity_filler_items("catalog-school", "series", &["canonical:theme:school"], 8),
            affinity_filler_items("catalog-heist", "series", &["canonical:theme:heist"], 8),
            affinity_filler_items("catalog-anime", "series", &["canonical:genre:anime"], 8),
            affinity_filler_items(
                "catalog-science-fiction",
                "series",
                &["canonical:genre:science-fiction"],
                8,
            ),
        ],
    );
    let mut groups = Vec::new();
    catalog_personalized_groups(
        &mut groups,
        &items,
        &profile,
        DiscoveryRailCompositionSettings::default(),
        6,
        6,
        &mut HashSet::new(),
    );
    let affinity_groups = groups
        .iter()
        .filter(|group| {
            matches!(
                group.kind,
                CatalogDiscoveryGroupKind::ThemeAffinity | CatalogDiscoveryGroupKind::GenreAffinity
            )
        })
        .map(|group| {
            (
                group.id.as_str(),
                group.label_value.as_deref(),
                group.label_key.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        affinity_groups,
        vec![
            ("theme_heist", Some("Heist"), Some("heist")),
            (
                "genre_science_fiction",
                Some("Science Fiction"),
                Some("science-fiction")
            ),
        ]
    );
    assert!(
        groups
            .iter()
            .filter(|group| group.label_key.is_none())
            .all(|group| !matches!(
                group.kind,
                CatalogDiscoveryGroupKind::ThemeAffinity | CatalogDiscoveryGroupKind::GenreAffinity
            ))
    );
}

fn watchlist_member(
    kind: MediaFacet,
    ids: &[(&str, &str)],
    first_seen_at: i64,
) -> DiscoveryWatchlistMember {
    DiscoveryWatchlistMember {
        kind,
        external_ids: ids
            .iter()
            .map(|(source, value)| ExternalId::new(*source, *value))
            .collect(),
        display_title: Some(format!("Row {}", ids.first().map(|id| id.1).unwrap_or("?"))),
        year: None,
        smg_title_id: None,
        first_seen_at: Utc.timestamp_opt(first_seen_at, 0).unwrap(),
        rank: None,
        declined: false,
    }
}

fn pending_watchlist_request(
    facet: MediaFacet,
    ids: &[(&str, &str)],
) -> scryer_domain::MediaRequest {
    let now = Utc.timestamp_opt(0, 0).unwrap();
    scryer_domain::MediaRequest {
        id: "request-fixture".to_string(),
        library_id: "library-fixture".to_string(),
        facet,
        status: scryer_domain::MediaRequestStatus::Pending,
        identity_fingerprint: "fingerprint-fixture".to_string(),
        title: "Fixture Request".to_string(),
        sort_title: None,
        slug: None,
        poster_url: None,
        background_url: None,
        year: None,
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        rating_summary: scryer_domain::TitleRatingSummary::default(),
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        requested_monitor_selection: None,
        resolved_by_user_id: None,
        resolved_at: None,
        created_title_id: None,
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        requested_lease_days: None,
        approved_lease_days: None,
        decision_id: None,
        decided_by_rule_set_ids: Vec::new(),
        policy_tags: Vec::new(),
        metadata_snapshot_json: "{}".to_string(),
        external_ids: ids
            .iter()
            .map(|(source, value)| ExternalId::new(*source, *value))
            .collect(),
        requesters: Vec::new(),
        origin: scryer_domain::MediaRequestOrigin::default(),
        created_by_user_id: "requester-fixture".to_string(),
        created_at: now,
        updated_at: now,
        requested_required_audio_languages: None,
        requested_preferred_audio_languages: None,
    }
}

struct WatchlistFixture {
    owned: CatalogOwnedVisibility,
    requests: Vec<scryer_domain::MediaRequest>,
    admitted: Option<HashSet<String>>,
    filters: crate::ports::DiscoveryHomeFilters,
    row_kinds: Vec<String>,
    include_unresolved: bool,
    limit: usize,
}

impl Default for WatchlistFixture {
    fn default() -> Self {
        Self {
            owned: CatalogOwnedVisibility::default(),
            requests: Vec::new(),
            admitted: None,
            filters: crate::ports::DiscoveryHomeFilters::default(),
            row_kinds: vec![
                "anime".to_string(),
                "movie".to_string(),
                "series".to_string(),
            ],
            include_unresolved: true,
            limit: 10,
        }
    }
}

impl WatchlistFixture {
    fn compose(
        &self,
        members: &[DiscoveryWatchlistMember],
        items: Vec<DiscoveryItemRecord>,
    ) -> DiscoveryWatchlistComposition {
        compose_discovery_watchlist(
            members,
            items,
            &DiscoveryWatchlistRules {
                owned_visibility: &self.owned,
                pending_requests: &self.requests,
                admitted_item_keys: self.admitted.as_ref(),
                filters: &self.filters,
                row_card_media_kinds: &self.row_kinds,
                include_unresolved: self.include_unresolved,
                limit: self.limit,
            },
        )
    }
}

fn owned_movie(tmdb_id: &str) -> CatalogOwnedVisibility {
    CatalogOwnedVisibility::from_titles(&[test_title(
        "owned-fixture",
        "Owned Fixture",
        MediaFacet::Movie,
        vec![("tmdb", tmdb_id)],
    )])
}

fn watchlist_section_titles(composition: &DiscoveryWatchlistComposition) -> Vec<&str> {
    composition
        .section
        .as_ref()
        .map(affinity_section_titles)
        .unwrap_or_default()
}

fn claimed(keys: &[&str]) -> HashSet<String> {
    keys.iter().map(|key| key.to_string()).collect()
}

#[test]
fn watchlist_member_target_keys_follow_the_member_kind() {
    let movie = watchlist_member(
        MediaFacet::Movie,
        &[("tmdb", "9101"), ("imdb", "tt9900101")],
        0,
    );
    assert_eq!(
        discovery_watchlist_member_target_keys(&movie),
        vec!["tmdb:movie:9101", "imdb:title:tt9900101"]
    );
    let series = watchlist_member(MediaFacet::Series, &[("tvdb", "9201"), ("tmdb", "9202")], 0);
    assert_eq!(
        discovery_watchlist_member_target_keys(&series),
        vec!["tvdb:series:9201", "tmdb:series:9202"]
    );
    let unusable = watchlist_member(MediaFacet::Movie, &[("tmdb", "not-a-number")], 0);
    assert!(discovery_watchlist_member_target_keys(&unusable).is_empty());
}

#[test]
fn watchlist_members_sort_by_provider_rank_then_newest_first_seen() {
    let ranked = |id: &str, rank: Option<i64>, first_seen_at: i64| DiscoveryWatchlistMember {
        rank,
        ..watchlist_member(MediaFacet::Movie, &[("tmdb", id)], first_seen_at)
    };
    let mut members = vec![
        ranked("1", None, 10),
        ranked("2", Some(2), 90),
        ranked("3", None, 30),
        ranked("4", Some(1), 0),
        ranked("5", Some(2), 95),
    ];
    sort_discovery_watchlist_members(&mut members);
    assert_eq!(
        members
            .iter()
            .map(|member| member.external_ids[0].value.as_str())
            .collect::<Vec<_>>(),
        vec!["4", "5", "2", "3", "1"]
    );
}

#[test]
fn watchlist_sources_match_each_providers_watch_later_list_only() {
    let source = |provider: &str, source_type: &str, params: &[(&str, &str)]| {
        let mut subscription = crate::lists::test_support::subscription("watchlist-source");
        subscription.source.provider = provider.to_string();
        subscription.source.source_type = source_type.to_string();
        subscription.source.params = params
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        discovery_watchlist_source_matches(&subscription)
    };
    assert!(source("plex", "watchlist", &[]));
    assert!(source("trakt", "watchlist", &[]));
    assert!(source("tmdb", "watchlist", &[]));
    assert!(source("mdblist", "watchlist", &[]));
    assert!(source("simkl", "plantowatch", &[]));
    assert!(source("mal", "status:plan_to_watch", &[]));
    assert!(source("anilist", "status", &[("status", "planning")]));
    assert!(source("anilist", "status", &[("status", "PLANNING")]));
    // A feed URL anyone could paste, read without the member's account.
    assert!(!source("plex", "watchlist_rss", &[]));
    assert!(!source("anilist", "status", &[("status", "current")]));
    assert!(!source("anilist", "status", &[]));
    assert!(!source("mal", "status:watching", &[]));
    assert!(!source("trakt", "user_list", &[("list", "watchlist")]));
    assert!(!source("custom", "watchlist", &[]));
}

#[test]
fn watchlist_composition_offers_stored_cards_then_row_cards_in_watchlist_order() {
    let members = vec![
        watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 50),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "71")], 40),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "2")], 30),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "72")], 20),
    ];
    let items = vec![
        affinity_test_item("2", "movie", &[]),
        affinity_test_item("1", "movie", &[]),
    ];

    let composition = WatchlistFixture::default().compose(&members, items);

    assert_eq!(
        watchlist_section_titles(&composition),
        vec!["Title 1", "Title 2", "Row 71", "Row 72"]
    );
    let section = composition.section.as_ref().expect("watchlist section");
    assert_eq!(section.section_type, DISCOVERY_WATCHLIST_SECTION_TYPE);
    assert_eq!(section.surface, "personalized");
    assert_eq!(section.label_key, None);
    assert_eq!(section.total_count, 4);
    // Only stored items are claimed from the reason rails.
    assert_eq!(
        composition.claimed_item_keys,
        claimed(&["tmdb:movie:1", "tmdb:movie:2"])
    );
}

#[test]
fn watchlist_composition_caps_cards_but_counts_and_claims_every_member() {
    let members = (1..=6)
        .map(|index| {
            watchlist_member(
                MediaFacet::Movie,
                &[("tmdb", &index.to_string())],
                100 - index,
            )
        })
        .collect::<Vec<_>>();
    let items = (1..=4)
        .map(|index| affinity_test_item(&index.to_string(), "movie", &[]))
        .collect::<Vec<_>>();

    let composition = WatchlistFixture {
        limit: 3,
        ..WatchlistFixture::default()
    }
    .compose(&members, items.clone());

    let section = composition.section.as_ref().expect("watchlist section");
    assert_eq!(
        watchlist_section_titles(&composition),
        vec!["Title 1", "Title 2", "Title 3"]
    );
    assert_eq!(section.total_count, 6);
    assert_eq!(composition.claimed_item_keys.len(), 4);

    let everything = WatchlistFixture {
        limit: usize::MAX,
        ..WatchlistFixture::default()
    }
    .compose(&members, items);
    assert_eq!(
        everything.section.expect("watchlist section").items.len(),
        6
    );
}

#[test]
fn watchlist_composition_leaves_out_owned_requested_and_declined_members() {
    let mut declined = watchlist_member(MediaFacet::Movie, &[("tmdb", "4")], 40);
    declined.declined = true;
    let mut declined_row = watchlist_member(MediaFacet::Movie, &[("tmdb", "74")], 35);
    declined_row.declined = true;
    let mut untitled = watchlist_member(MediaFacet::Movie, &[("tmdb", "75")], 34);
    untitled.display_title = None;
    let members = vec![
        watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 60),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "2")], 50),
        declined,
        declined_row,
        untitled,
        watchlist_member(MediaFacet::Movie, &[("tmdb", "5")], 30),
        // Same number as the pending movie request, but a series: a movie
        // request does not answer for it.
        watchlist_member(MediaFacet::Series, &[("tmdb", "2")], 20),
        // Owned, with no stored item: the row card checks the catalog too.
        watchlist_member(MediaFacet::Movie, &[("tmdb", "73")], 10),
        // Requested, with no stored item.
        watchlist_member(MediaFacet::Movie, &[("tmdb", "76")], 5),
    ];
    let items = vec![
        affinity_test_item("1", "movie", &[]),
        affinity_test_item("2", "movie", &[]),
        affinity_test_item("4", "movie", &[]),
        affinity_test_item("5", "movie", &[]),
        affinity_test_item("2", "series", &[]),
    ];
    let owned = CatalogOwnedVisibility::from_titles(&[
        test_title(
            "owned-fixture-a",
            "Owned Fixture A",
            MediaFacet::Movie,
            vec![("tmdb", "1")],
        ),
        test_title(
            "owned-fixture-b",
            "Owned Fixture B",
            MediaFacet::Movie,
            vec![("tmdb", "73")],
        ),
    ]);

    let composition = WatchlistFixture {
        owned,
        requests: vec![
            pending_watchlist_request(MediaFacet::Movie, &[("tmdb", "2")]),
            pending_watchlist_request(MediaFacet::Movie, &[("tmdb", "76")]),
        ],
        ..WatchlistFixture::default()
    }
    .compose(&members, items);

    assert_eq!(
        watchlist_section_titles(&composition),
        vec!["Title 5", "Title 2"]
    );
    let section = composition.section.as_ref().expect("watchlist section");
    assert_eq!(section.items[1].target_key, "tmdb:series:2");
    // Requested and declined members stay on the watchlist, so the reason
    // rails still may not offer them; the owned one is released.
    assert_eq!(
        composition.claimed_item_keys,
        claimed(&[
            "tmdb:movie:2",
            "tmdb:movie:4",
            "tmdb:movie:5",
            "tmdb:series:2"
        ])
    );
}

#[test]
fn watchlist_requests_match_through_the_resolved_items_own_ids() {
    // The member and the request name the title by ids that never meet
    // directly; the stored item carries both.
    let members = vec![watchlist_member(MediaFacet::Movie, &[("tmdb", "9301")], 0)];
    let mut item = affinity_test_item("9301", "movie", &[]);
    item.external_ids = vec![crate::ports::DiscoveryExternalIdRecord {
        source: "imdb".to_string(),
        kind: "movie".to_string(),
        id: "tt9909301".to_string(),
        key: "imdb:movie:tt9909301".to_string(),
    }];

    let composition = WatchlistFixture {
        requests: vec![pending_watchlist_request(
            MediaFacet::Movie,
            &[("imdb", "tt9909301")],
        )],
        ..WatchlistFixture::default()
    }
    .compose(&members, vec![item]);

    assert!(composition.section.is_none());
    assert_eq!(composition.claimed_item_keys, claimed(&["tmdb:movie:9301"]));
}

#[test]
fn watchlist_title_on_two_lists_is_offered_unless_every_row_declined_it() {
    let mut declined = watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 90);
    declined.declined = true;
    let wanted = watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 10);
    let items = vec![affinity_test_item("1", "movie", &[])];

    let composition =
        WatchlistFixture::default().compose(&[declined.clone(), wanted], items.clone());
    assert_eq!(watchlist_section_titles(&composition), vec!["Title 1"]);

    let composition = WatchlistFixture::default().compose(&[declined.clone(), declined], items);
    assert!(composition.section.is_none());
    assert_eq!(composition.claimed_item_keys, claimed(&["tmdb:movie:1"]));
}

#[test]
fn watchlist_composition_with_nothing_to_offer_has_no_section() {
    let members = vec![watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 0)];

    let composition = WatchlistFixture {
        owned: owned_movie("1"),
        ..WatchlistFixture::default()
    }
    .compose(&members, vec![affinity_test_item("1", "movie", &[])]);

    assert!(composition.section.is_none());
    assert!(composition.claimed_item_keys.is_empty());
    assert!(
        WatchlistFixture::default()
            .compose(&[], Vec::new())
            .section
            .is_none()
    );
}

#[test]
fn watchlist_composition_honours_the_filter_bar() {
    let mut old_row = watchlist_member(MediaFacet::Movie, &[("tmdb", "71")], 20);
    old_row.year = Some(1990);
    let mut recent_row = watchlist_member(MediaFacet::Movie, &[("tmdb", "72")], 10);
    recent_row.year = Some(2030);
    let undated_row = watchlist_member(MediaFacet::Movie, &[("tmdb", "73")], 5);
    let members = vec![
        watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 40),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "2")], 30),
        old_row,
        recent_row,
        undated_row,
    ];
    let items = vec![
        affinity_test_item("1", "movie", &[]),
        affinity_test_item("2", "movie", &[]),
    ];

    let composition = WatchlistFixture {
        // Storage admitted only the second stored item.
        admitted: Some(claimed(&["tmdb:movie:2"])),
        filters: crate::ports::DiscoveryHomeFilters {
            minimum_year: Some(2000),
            ..Default::default()
        },
        ..WatchlistFixture::default()
    }
    .compose(&members, items);

    assert_eq!(
        watchlist_section_titles(&composition),
        vec!["Title 2", "Row 72", "Row 73"]
    );
    // A filtered title is still the watchlist's, so the reason rails stay off it.
    assert_eq!(
        composition.claimed_item_keys,
        claimed(&["tmdb:movie:1", "tmdb:movie:2"])
    );
}

#[test]
fn watchlist_row_cards_follow_the_media_kind_chips() {
    let members = vec![
        watchlist_member(MediaFacet::Movie, &[("tmdb", "71")], 20),
        watchlist_member(MediaFacet::Anime, &[("tvdb", "9401")], 10),
    ];
    let composition = WatchlistFixture {
        row_kinds: vec!["anime".to_string()],
        ..WatchlistFixture::default()
    }
    .compose(&members, Vec::new());
    assert_eq!(watchlist_section_titles(&composition), vec!["Row 9401"]);
}

#[test]
fn watchlist_row_cards_need_an_id_a_request_can_add() {
    let members = vec![
        watchlist_member(MediaFacet::Anime, &[("anilist", "9411")], 30),
        watchlist_member(MediaFacet::Anime, &[("mal", "9412"), ("anidb", "9413")], 20),
        watchlist_member(
            MediaFacet::Anime,
            &[("anilist", "9414"), ("tvdb", "9415")],
            10,
        ),
    ];

    let composition = WatchlistFixture::default().compose(&members, Vec::new());

    let section = composition.section.as_ref().expect("watchlist section");
    assert_eq!(section.total_count, 1);
    assert_eq!(watchlist_section_titles(&composition), vec!["Row 9414"]);
    assert_eq!(section.items[0].target_key, "tvdb:series:9415");

    // An AniList-only row still resolves once a stored item names it.
    let mut stored = affinity_test_item("9411", "series", &[]);
    stored.target_key = "anilist:anime:9411".to_string();
    stored.display_title = "Stored 9411".to_string();
    let resolved = WatchlistFixture::default().compose(&members[..1], vec![stored]);
    assert_eq!(watchlist_section_titles(&resolved), vec!["Stored 9411"]);
}

#[test]
fn watchlist_rows_naming_one_title_merge_into_one_row_card() {
    // Two lists know one untracked series by different ids; a shared tvdb id
    // joins them.
    let mut imdb_and_tvdb = watchlist_member(
        MediaFacet::Series,
        &[("imdb", "tt9909801"), ("tvdb", "9801")],
        30,
    );
    imdb_and_tvdb.display_title = Some("Fixture Merged Series".to_string());
    imdb_and_tvdb.year = Some(2032);
    let mut tvdb_and_tmdb = watchlist_member(
        MediaFacet::Series,
        &[("tvdb", "9801"), ("tmdb", "9802")],
        20,
    );
    tvdb_and_tmdb.smg_title_id = Some(9803);
    let members = vec![
        imdb_and_tvdb.clone(),
        tvdb_and_tmdb.clone(),
        watchlist_member(MediaFacet::Series, &[("tvdb", "9811")], 10),
    ];

    let composition = WatchlistFixture::default().compose(&members, Vec::new());

    let section = composition.section.as_ref().expect("watchlist section");
    assert_eq!(section.total_count, 2);
    assert_eq!(
        watchlist_section_titles(&composition),
        vec!["Fixture Merged Series", "Row 9811"]
    );
    let merged = &section.items[0];
    // tmdb leads the key preference whichever row named the title first.
    assert_eq!(merged.target_key, "tmdb:series:9802");
    assert_eq!(merged.year, Some(2032));
    assert_eq!(
        merged
            .external_ids
            .iter()
            .map(|id| (id.source.as_str(), id.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("imdb", "tt9909801"),
            ("tvdb", "9801"),
            ("tmdb", "9802"),
            ("smg", "9803")
        ]
    );

    // Declined on one list, wanted on the other: still offered once.
    let mut declined = imdb_and_tvdb.clone();
    declined.declined = true;
    let one_declined =
        WatchlistFixture::default().compose(&[declined.clone(), tvdb_and_tmdb.clone()], Vec::new());
    assert_eq!(
        watchlist_section_titles(&one_declined),
        vec!["Fixture Merged Series"]
    );

    // Declined on every list: not offered.
    let mut also_declined = tvdb_and_tmdb;
    also_declined.declined = true;
    let all_declined = WatchlistFixture::default().compose(&[declined, also_declined], Vec::new());
    assert!(all_declined.section.is_none());
}

#[test]
fn watchlist_unresolved_cards_follow_include_unresolved() {
    let members = vec![
        watchlist_member(MediaFacet::Movie, &[("tmdb", "1")], 30),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "2")], 20),
        watchlist_member(MediaFacet::Movie, &[("tmdb", "71")], 10),
    ];
    let mut unresolved = affinity_test_item("2", "movie", &[]);
    unresolved.resolved = false;
    let items = vec![affinity_test_item("1", "movie", &[]), unresolved];

    let shown = WatchlistFixture::default().compose(&members, items.clone());
    assert_eq!(
        watchlist_section_titles(&shown),
        vec!["Title 1", "Title 2", "Row 71"]
    );

    let hidden = WatchlistFixture {
        include_unresolved: false,
        ..WatchlistFixture::default()
    }
    .compose(&members, items);
    assert_eq!(watchlist_section_titles(&hidden), vec!["Title 1"]);
}

#[test]
fn watchlist_row_card_is_an_unresolved_item_with_the_members_ids() {
    let mut member = watchlist_member(
        MediaFacet::Series,
        &[("tvdb", "9501"), ("imdb", "tt9909501")],
        0,
    );
    member.display_title = Some("Fixture Row Series".to_string());
    member.year = Some(2031);
    member.smg_title_id = Some(9601);

    let composition = WatchlistFixture::default().compose(&[member], Vec::new());

    let card = &composition.section.expect("watchlist section").items[0];
    assert!(!card.resolved);
    assert_eq!(card.target_key, "tvdb:series:9501");
    assert_eq!(card.target_kind, "series");
    assert_eq!(card.content_type.as_deref(), Some("series"));
    assert_eq!(card.display_title, "Fixture Row Series");
    assert_eq!(card.year, Some(2031));
    assert_eq!(card.poster_url, None);
    assert_eq!(card.background_url, None);
    assert_eq!(
        card.external_ids
            .iter()
            .map(|id| (id.source.as_str(), id.id.as_str()))
            .collect::<Vec<_>>(),
        vec![("tvdb", "9501"), ("imdb", "tt9909501"), ("smg", "9601")]
    );
    assert!(composition.claimed_item_keys.is_empty());
}

#[test]
fn watchlist_lookup_ignores_target_key_case() {
    let members = vec![watchlist_member(
        MediaFacet::Movie,
        &[("imdb", "tt9909701")],
        0,
    )];
    let mut item = affinity_test_item("9701", "movie", &[]);
    item.target_key = "IMDB:title:TT9909701".to_string();

    let composition = WatchlistFixture::default().compose(&members, vec![item]);

    assert_eq!(watchlist_section_titles(&composition), vec!["Title 9701"]);
    assert_eq!(
        composition.claimed_item_keys,
        claimed(&["IMDB:title:TT9909701"])
    );
}

#[test]
fn personalized_sections_never_offer_watchlist_claimed_titles() {
    let profile = affinity_profile(&["Fantasy"], &[], library_mix(60, 20, 20));
    let items = extend_with_fillers(
        vec![
            affinity_test_item("1", "movie", &["canonical:genre:fantasy"]),
            affinity_test_item("2", "movie", &["canonical:genre:fantasy"]),
        ],
        vec![affinity_filler_items(
            "fan",
            "movie",
            &["canonical:genre:fantasy"],
            8,
        )],
    );
    let unclaimed = compose_affinity_sections(&items, &profile, 10);
    assert!(
        unclaimed
            .iter()
            .flat_map(|section| section.items.iter())
            .any(|item| item.target_key == "tmdb:movie:1"),
        "fixture must offer the title before it is claimed"
    );

    let claimed_keys = claimed(&["tmdb:movie:1"]);
    let sections = personalized_section_results(
        &items,
        &profile,
        &HashMap::new(),
        &claimed_keys,
        DiscoveryRailCompositionSettings::default(),
        true,
        10,
    );

    assert!(!sections.is_empty());
    assert!(
        sections
            .iter()
            .flat_map(|section| section.items.iter())
            .all(|item| item.target_key != "tmdb:movie:1")
    );
    assert!(
        sections
            .iter()
            .flat_map(|section| section.items.iter())
            .any(|item| item.target_key == "tmdb:movie:2")
    );
}

/// A synthetic personalized pool shaped like a large library's: every item
/// edge-backed, spread over several allowlisted genres and themes and all
/// three mediums, with strictly falling relevance so pool order is SMG order.
fn timing_pool(size: usize) -> Vec<DiscoveryItemRecord> {
    const GENRES: &[&str] = &[
        "crime", "drama", "comedy", "thriller", "fantasy", "horror", "mystery", "romance",
    ];
    const THEMES: &[&str] = &[
        "heist",
        "revenge",
        "survival",
        "dystopia",
        "space",
        "detective",
    ];
    const MEDIUMS: &[&str] = &["movie", "series", "anime"];
    (0..size)
        .map(|index| {
            let genre_a = GENRES[index % GENRES.len()];
            let genre_b = GENRES[(index / GENRES.len()) % GENRES.len()];
            let theme = THEMES[index % THEMES.len()];
            let facet_terms = [
                format!("canonical:genre:{genre_a}"),
                format!("canonical:genre:{genre_b}"),
                format!("canonical:theme:{theme}"),
            ];
            let mut item = affinity_test_item(
                &format!("pool-{index}"),
                MEDIUMS[index % MEDIUMS.len()],
                &facet_terms.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            item.recommendation_score = Some((size - index) as f64);
            item.base_rank = Some(((index * 7919) % 1000) as f64);
            item.sort_index = index as i32;
            item
        })
        .collect()
}

/// Wall-clock cost of composing the personalized shelf at the home's pool
/// size and at a section page's. Ignored by default because it reads the
/// clock; when run it asserts the shelf composes and that the slowest round
/// stays inside a deliberately generous bound, and prints the timings. Run it
/// with
/// `cargo nextest run --locked -p scryer-application --run-ignored only
/// discovery_home_composition_timing --no-capture`.
#[test]
#[ignore = "timing measurement, run on demand"]
fn discovery_home_composition_timing() {
    let pool = timing_pool(2_500);
    let profile = affinity_profile(
        &["crime", "drama"],
        &["heist", "revenge"],
        balanced_library_mix(),
    );
    const ROUNDS: usize = 20;
    for pool_size in [1_000usize, 2_500] {
        let items = &pool[..pool_size];
        let mut samples = Vec::with_capacity(ROUNDS);
        let mut rails = 0;
        let candidates_by_id = HashMap::new();
        for _ in 0..ROUNDS {
            let started = Instant::now();
            // The same steps the home runs: visibility, the shared rail
            // context, then the shelf.
            let visible_items = discovery_home_visible_items(items, true);
            let context = DiscoveryRailContext::new(
                &candidates_by_id,
                DiscoveryRailCompositionSettings::default(),
                &profile,
                &visible_items,
                18,
            );
            let sections = personalized_section_results_in_context(
                visible_items,
                &profile,
                &context,
                &HashSet::new(),
            );
            samples.push(started.elapsed());
            rails = sections.len();
        }
        samples.sort();
        assert!(rails > 0, "the synthetic pool composes a shelf");
        assert!(
            samples[ROUNDS - 1] < std::time::Duration::from_secs(5),
            "composing a {pool_size}-title pool took {:?}",
            samples[ROUNDS - 1]
        );
        println!(
            "discovery home composition: pool={pool_size} rails={rails} median={:?} max={:?}",
            samples[ROUNDS / 2],
            samples[ROUNDS - 1]
        );
    }
}

#[test]
fn section_reading_order_matches_the_web_contract() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SectionOrderContract {
        personalized_shelf: Vec<String>,
        generic_for_you_fallback: Vec<String>,
        non_library_personalized: Vec<String>,
        public_promotion: Vec<String>,
        world_shelf: Vec<String>,
        curated: Vec<String>,
        suppressed_legacy: Vec<String>,
        merged_section_targets: Vec<(String, String)>,
        anime_only_public: Vec<String>,
        anime_excluding_public: Vec<String>,
        min_public_rail_items: usize,
    }
    let contract: SectionOrderContract = serde_json::from_str(include_str!(
        "../../../../apps/scryer-web/lib/contracts/discovery-section-order.json"
    ))
    .expect("section order contract is JSON");
    let owned = |list: &[&str]| {
        list.iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        contract.personalized_shelf,
        owned(DISCOVERY_PERSONALIZED_SHELF_ORDER)
    );
    assert_eq!(
        contract.generic_for_you_fallback,
        owned(DISCOVERY_GENERIC_FOR_YOU_FALLBACK_TYPES)
    );
    assert_eq!(
        contract.non_library_personalized,
        owned(DISCOVERY_NON_LIBRARY_PERSONALIZED_TYPES)
    );
    assert_eq!(
        contract.public_promotion,
        owned(DISCOVERY_PUBLIC_PROMOTION_TYPES)
    );
    assert_eq!(contract.world_shelf, owned(DISCOVERY_WORLD_SHELF_ORDER));
    assert_eq!(contract.curated, owned(DISCOVERY_CURATED_TYPES));
    assert_eq!(
        contract.suppressed_legacy,
        owned(DISCOVERY_SUPPRESSED_LEGACY_TYPES)
    );
    assert_eq!(
        contract.merged_section_targets,
        DISCOVERY_PUBLIC_FOLDED_SECTION_TYPES
            .iter()
            .map(|(source, target)| (source.to_string(), target.to_string()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        contract.anime_only_public,
        owned(DISCOVERY_PUBLIC_ANIME_ONLY_SECTION_TYPES)
    );
    assert_eq!(
        contract.anime_excluding_public,
        owned(DISCOVERY_PUBLIC_ANIME_EXCLUDING_SECTION_TYPES)
    );
    assert_eq!(
        contract.min_public_rail_items,
        DISCOVERY_MIN_PUBLIC_RAIL_ITEMS
    );
}

fn rendered_test_item(key: &str, content_type: &str) -> DiscoveryItemRecord {
    let target_kind = if content_type == "movie" {
        "movie"
    } else {
        "series"
    };
    let mut item = test_discovery_item(key, target_kind, Some(content_type));
    item.target_key = key.to_string();
    item
}

fn rendered_test_section(
    section_id: &str,
    section_type: &str,
    surface: &str,
    keys: &[&str],
) -> DiscoverySectionResult {
    let items = keys
        .iter()
        .map(|key| rendered_test_item(key, "movie"))
        .collect::<Vec<_>>();
    DiscoverySectionResult {
        section_id: section_id.to_string(),
        section_type: section_type.to_string(),
        title: section_id.to_string(),
        surface: surface.to_string(),
        total_count: items.len() as i64,
        items,
        label_key: None,
    }
}

fn rendered_test_home(
    personalized_sections: Vec<DiscoverySectionResult>,
    public_sections: Vec<DiscoverySectionResult>,
) -> DiscoveryHomeResult {
    DiscoveryHomeResult {
        status: DiscoverySyncStatus {
            state: crate::DiscoverySyncStateRecord::default(),
            recent_runs: Vec::new(),
            pending_context_change_count: 0,
        },
        hero_item: None,
        public_sections,
        personalized_sections,
        complete_collection: None,
        facets: Vec::new(),
        can_view_personalized: true,
        browse_labels: Vec::new(),
    }
}

fn rendered_keys(section: &DiscoverySectionResult) -> Vec<&str> {
    section
        .items
        .iter()
        .map(|item| item.target_key.as_str())
        .collect()
}

#[test]
fn rendered_window_drops_what_earlier_rails_showed_in_the_dashboard_order() {
    let home = rendered_test_home(
        vec![
            // Delivered after Top Rated, read before it.
            rendered_test_section("top_rated", "TOP_RATED", "mixed", &["b", "d", "e"]),
            rendered_test_section(
                "because_you_like_genre_drama",
                "BECAUSE_YOU_LIKE_GENRE",
                "personalized",
                &["a", "b", "c"],
            ),
        ],
        vec![
            rendered_test_section("trending", "TRENDING_NOW", "public", &["x", "y"]),
            rendered_test_section("thin", "NEW_ON_STREAMING", "public", &["d", "z"]),
            rendered_test_section("popular", "POPULAR_RIGHT_NOW", "public", &["p", "a"]),
        ],
    );

    let top_rated = discovery_rendered_window(&home, "top_rated", None).expect("top rated renders");
    assert_eq!(rendered_keys(&top_rated.section), ["d", "e"]);
    assert_eq!(
        top_rated.preceding_keys,
        ["MOVIE:a", "MOVIE:b", "MOVIE:c"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>()
    );

    // The trending source folds behind its target, and a title an earlier
    // rail showed drops out of the folded rail.
    let popular = discovery_rendered_window(&home, "popular", None).expect("popular renders");
    assert_eq!(rendered_keys(&popular.section), ["p", "x", "y"]);
    assert!(discovery_rendered_window(&home, "trending", None).is_none());

    // A public rail left under the floor is hidden, but what it would have
    // shown still counts as shown.
    assert!(discovery_rendered_window(&home, "thin", None).is_none());
    let continued = continue_discovery_rendered_window(
        discovery_rendered_window(&home, "popular", None),
        Some(rendered_test_section(
            "popular",
            "POPULAR_RIGHT_NOW",
            "public",
            &["p", "a", "x", "z", "q"],
        )),
    )
    .expect("the section continues");
    assert_eq!(rendered_keys(&continued), ["p", "x", "y", "q"]);
}

#[test]
fn rendered_window_counts_a_shown_hero_as_shown_before_every_rail() {
    let mut home = rendered_test_home(
        Vec::new(),
        vec![
            rendered_test_section(
                "popular",
                "POPULAR_RIGHT_NOW",
                "public",
                &["h", "a", "b", "c"],
            ),
            rendered_test_section(
                "next",
                "UPCOMING_NEXT_SEASON",
                "public",
                &["d", "h", "e", "f"],
            ),
        ],
    );
    home.hero_item = Some(rendered_test_item("h", "movie"));
    let hero_key = discovery_home_hero_key(&home);
    assert_eq!(hero_key.as_deref(), Some("MOVIE:h"));

    let popular =
        discovery_rendered_window(&home, "popular", hero_key.as_deref()).expect("popular renders");
    assert_eq!(rendered_keys(&popular.section), ["a", "b", "c"]);
    assert_eq!(popular.section.total_count, 3);
    assert!(popular.preceding_keys.contains("MOVIE:h"));
    let continued = continue_discovery_rendered_window(
        Some(popular),
        Some(rendered_test_section(
            "popular",
            "POPULAR_RIGHT_NOW",
            "public",
            &["h", "a", "b", "c", "g"],
        )),
    )
    .expect("the section continues");
    assert_eq!(rendered_keys(&continued), ["a", "b", "c", "g"]);

    // A hero no rendered rail shows is not shown, so nothing is taken out.
    home.hero_item = Some(rendered_test_item("elsewhere", "movie"));
    let hero_key = discovery_home_hero_key(&home);
    let next = discovery_rendered_window(&home, "next", hero_key.as_deref()).expect("next renders");
    assert_eq!(rendered_keys(&next.section), ["d", "e", "f"]);
    assert!(!next.preceding_keys.contains("MOVIE:elsewhere"));
}

#[test]
fn folded_public_section_page_one_is_each_rail_window_first() {
    let mut anime = rendered_test_section("trending", "TRENDING_NOW", "public", &[]);
    anime.items = vec![
        rendered_test_item("t1", "movie"),
        rendered_test_item("t-anime", "anime"),
        rendered_test_item("t2", "movie"),
        rendered_test_item("t3", "movie"),
    ];
    let folded = fold_discovery_public_section(
        "popular",
        vec![
            rendered_test_section(
                "popular",
                "POPULAR_RIGHT_NOW",
                "public",
                &["p1", "p2", "p3", "p4"],
            ),
            anime,
        ],
        2,
    )
    .expect("the folded section has items");
    // Window of two from each rail, narrowed after the cut, then the rest.
    assert_eq!(
        rendered_keys(&folded),
        ["p1", "p2", "t1", "p3", "p4", "t2", "t3"]
    );
    assert_eq!(folded.section_id, "popular");
    assert_eq!(folded.total_count, 7);
}

#[test]
fn requested_titles_match_by_media_kind_as_well_as_external_id() {
    let now = Utc.timestamp_opt(3_000, 0).unwrap();
    let request = scryer_domain::MediaRequest {
        background_url: None,
        requested_monitor_selection: None,
        requested_lease_days: None,
        approved_lease_days: None,
        decision_id: None,
        decided_by_rule_set_ids: Vec::new(),
        policy_tags: Vec::new(),
        metadata_snapshot_json: "{}".to_string(),
        rating_summary: scryer_domain::TitleRatingSummary::default(),
        id: "requested-kind".to_string(),
        library_id: "movies".to_string(),
        facet: MediaFacet::Movie,
        status: scryer_domain::MediaRequestStatus::Pending,
        identity_fingerprint: "requested-kind".to_string(),
        title: "Requested Kind".to_string(),
        sort_title: None,
        slug: None,
        poster_url: None,
        year: None,
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        resolved_by_user_id: None,
        resolved_at: None,
        created_title_id: None,
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        external_ids: vec![ExternalId {
            source: "tmdb".to_string(),
            kind: None,
            value: "4100".to_string(),
        }],
        requesters: Vec::new(),
        created_by_user_id: "requester".to_string(),
        origin: Default::default(),
        created_at: now,
        updated_at: now,
        requested_required_audio_languages: None,
        requested_preferred_audio_languages: None,
    };
    let requested = DiscoveryRequestedVisibility::from_media_requests(&[request]);
    assert!(requested.item_is_requested(&rendered_test_item("tmdb:movie:4100", "movie")));
    assert!(!requested.item_is_requested(&rendered_test_item("tmdb:series:4100", "series")));
    assert!(!requested.item_is_requested(&rendered_test_item("tmdb:movie:4101", "movie")));
}
