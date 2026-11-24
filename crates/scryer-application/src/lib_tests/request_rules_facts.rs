//! The pure request-rule fact builder (spec 0003 FR-015, plan §3.2).
//!
//! `build_request_input` takes a fully-read context and returns a document, so
//! every one of these runs without a harness, a repository, or a clock. What is
//! under test is the *unknown vs. absent* distinction the whole safety story
//! rests on: a fact Scryer could not observe holds the rules that read it, and a
//! fact a source answered "nothing" to is a real answer a rule may act on.

use chrono::{DateTime, TimeZone, Utc};
use scryer_domain::{CanonicalMediaTag, TitleExternalRating, TitleRatingSummary};
use scryer_rules::request::{RequestDoc, RequestLibraryDoc, RequestRequesterDoc};
use std::collections::BTreeMap;

use crate::media_requests::snapshot::{
    MediaRequestMetadataSnapshot, SnapshotReleaseDates, SnapshotSeriesShape,
};
use crate::request_rules::facts::{
    RequestCatalogContext, RequestFactReads, RequestInputContext, RequestQualityContext,
    RequestRequesterHistoryContext, RequestStorageContext, RequestStorageFigures,
    RequestWatchlistContext, build_request_input, fact_reason,
};
use crate::types::{ContentCertification, ContentRating, MdblistSummary};

const EVALUATED_AT: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z, a Tuesday.

fn evaluation_time() -> DateTime<Utc> {
    Utc.timestamp_opt(EVALUATED_AT, 0)
        .single()
        .expect("instant")
}

fn us_rating(label: &str, age: i32) -> ContentRating {
    ContentRating {
        country: "usa".to_string(),
        certifications: vec![ContentCertification {
            value: label.to_string(),
            source: "tmdb".to_string(),
            release_type: Some(3),
        }],
        age_rating: Some(age),
        age_rating_source: Some("tmdb".to_string()),
    }
}

fn canonical_tag(key: &str, category: &str, name: &str, is_adult: bool) -> CanonicalMediaTag {
    CanonicalMediaTag {
        affinity_signals: Vec::new(),
        key: key.to_string(),
        category: category.to_string(),
        name: name.to_string(),
        confidence: Some(0.9),
        sources: vec!["fixture".to_string()],
        source_tag_keys: Vec::new(),
        is_adult,
        is_spoiler: false,
    }
}

/// A snapshot with every group populated, so a test only has to say what it
/// wants to take away.
fn full_snapshot() -> MediaRequestMetadataSnapshot {
    MediaRequestMetadataSnapshot {
        schema_version: 1,
        captured_at: Some(evaluation_time()),
        source: Some("smg_titles".to_string()),
        partial: false,
        missing: Vec::new(),
        genres: vec!["Drama".to_string(), "Thriller".to_string()],
        canonical_tags: vec![
            canonical_tag("canonical:genre:drama", "genre", "Drama", false),
            canonical_tag("canonical:theme:heist", "theme", "Heist", false),
        ],
        content_ratings: vec![us_rating("PG-13", 13)],
        mdblist: Some(MdblistSummary {
            mdblist_id: "mdb-fixture".to_string(),
            trakt_id: Some(4242),
            score: Some(70.0),
            score_average: Some(68.5),
            age_rating: Some(13),
            certification: "PG-13".to_string(),
            commonsense: Some(true),
        }),
        ratings: TitleRatingSummary {
            rating: Some(7.4),
            rating_sources: vec!["imdb".to_string()],
            external_ratings: vec![TitleExternalRating {
                source: "IMDb".to_string(),
                value: Some(7.4),
                score: Some(74.0),
                normalized: 0.74,
                votes: Some(1234),
                url: String::new(),
            }],
        },
        tmdb_vote_average: Some(7.25),
        tmdb_vote_count: Some(4321),
        popularity: Some(19.5),
        runtime_minutes: Some(100),
        original_language: Some("eng".to_string()),
        country: Some("us".to_string()),
        network: None,
        studio: Some("Fixture Studio".to_string()),
        content_status: Some("Released".to_string()),
        // Ten days before the evaluation instant.
        release_date: Some("2023-11-04".to_string()),
        first_aired: None,
        awards: vec![crate::types::TitleAward {
            award_qid: "Q100".to_string(),
            award_label: "Fixture Prize".to_string(),
            year: Some(2023),
            recipient_qid: "Q200".to_string(),
            recipient_label: "Fixture Subject".to_string(),
            claim_side: "work".to_string(),
        }],
        is_adult: false,
        series_shape: None,
        release_dates: Some(SnapshotReleaseDates {
            fetched: true,
            theatrical: Some("2023-11-04".to_string()),
            digital: Some("2023-11-10T00:00:00Z".to_string()),
            physical: None,
        }),
    }
}

/// A series snapshot: two seasons, five regular episodes, three aired by
/// capture and two scheduled after it.
fn series_snapshot() -> MediaRequestMetadataSnapshot {
    let mut snapshot = full_snapshot();
    snapshot.release_dates = None;
    snapshot.release_date = None;
    snapshot.first_aired = Some("2023-01-10".to_string());
    snapshot.series_shape = Some(SnapshotSeriesShape {
        season_count: 2,
        episode_count: 5,
        aired_at_capture: 3,
        upcoming_air_dates: vec!["2023-11-14".to_string(), "2023-11-21".to_string()],
    });
    snapshot
}

fn series_context(snapshot: MediaRequestMetadataSnapshot) -> RequestInputContext {
    let mut context = context_with(snapshot);
    context.library.facet = "series".to_string();
    context
}

fn context_with(snapshot: MediaRequestMetadataSnapshot) -> RequestInputContext {
    RequestInputContext {
        evaluation_time: evaluation_time(),
        requester: RequestRequesterDoc {
            user_id: "user-1".to_string(),
            username: "operator".to_string(),
            account_kind: "local".to_string(),
            app_permissions: vec!["manage_catalog_settings".to_string()],
            library_permissions: vec!["view".to_string(), "request".to_string()],
            linked_providers: vec!["jellyfin".to_string()],
            created_at: Some("2022-01-01T00:00:00Z".to_string()),
            last_login_at: Some("2023-11-13T00:00:00Z".to_string()),
            account_age_days: Some(682),
        },
        library: RequestLibraryDoc {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
            facet: "movie".to_string(),
            is_default: true,
        },
        request: RequestDoc {
            origin: "manual".to_string(),
            title: "Glass Harbor".to_string(),
            year: Some(2023),
            external_ids: BTreeMap::from([("tvdb".to_string(), "9001".to_string())]),
            quality_profile_id: Some("1080p".to_string()),
            quality_profile_name: Some("1080P".to_string()),
            monitor_type: None,
            monitor_selection_season_count: None,
            monitor_selection_seasons: None,
            lease_forever: false,
            lease_days: Some(14),
            required_audio_languages: None,
            preferred_audio_languages: None,
            list: None,
        },
        snapshot,
        quality: Some(RequestQualityContext {
            tiers: vec!["1080P".to_string(), "720P".to_string()],
            allow_upgrades: true,
        }),
        catalog: RequestCatalogContext {
            exists_in_library_ids: vec!["library-2".to_string()],
            previous_request_count: 2,
            previously_denied: true,
            previously_approved: false,
            other_requester_count: 2,
            pending_by_other_requester: true,
            readable: true,
        },
        history: RequestRequesterHistoryContext {
            pending_request_count: 1,
            approved_last_30d: 2,
            denied_last_30d: 0,
            total_approved: 9,
            active_lease_count: Some(3),
            last_request_at: Some(evaluation_time() - chrono::Duration::days(4)),
            readable: true,
        },
        library_title_count: Some(412),
        watchlist: RequestWatchlistContext::Known {
            first_seen_at: Some(evaluation_time() - chrono::Duration::days(6)),
        },
        list_excluded: Some(false),
        requester_titles_in_library_count: Some(5),
        storage: RequestStorageContext {
            root_id: Some("root-fixture".to_string()),
            capacity: Ok(RequestStorageFigures {
                sampled_at: evaluation_time() - chrono::Duration::minutes(3),
                available_bytes: 500,
                total_bytes: 2_000,
                available_percent: 25.0,
            }),
        },
        fact_reads: RequestFactReads::ALL,
    }
}

/// The serialized document, which is what the engine actually reads.
fn document(context: RequestInputContext) -> serde_json::Value {
    serde_json::to_value(build_request_input(context)).expect("input serializes")
}

fn status_of(document: &serde_json::Value, fact: &str) -> String {
    document["observations"][fact]["status"]
        .as_str()
        .unwrap_or_else(|| panic!("fact {fact} has no status"))
        .to_string()
}

fn reason_of(document: &serde_json::Value, fact: &str) -> String {
    document["observations"][fact]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("fact {fact} has no reason"))
        .to_string()
}

/// Every fact the contract declares, so a fact added without a builder entry
/// fails here rather than silently reading unknown forever.
const EVERY_FACT: [&str; 59] = [
    "age_rating",
    "certifications",
    "certification_label",
    "certification_rank",
    "commonsense_recommended",
    "genres",
    "canonical_tag_keys",
    "themes",
    "is_adult",
    "rating",
    "ratings_by_source",
    "tmdb_vote_average",
    "tmdb_vote_count",
    "popularity",
    "runtime_minutes",
    "original_language",
    "country",
    "network",
    "studio",
    "content_status",
    "release_date",
    "first_aired",
    "release_age_days",
    "award_count",
    "awards",
    "quality_profile_tiers",
    "quality_profile_max_resolution",
    "quality_profile_allows_upgrades",
    "exists_in_library_ids",
    "previous_request_count",
    "previously_denied",
    "previously_approved",
    "pending_request_count",
    "approved_last_30d",
    "denied_last_30d",
    "total_approved",
    "active_lease_count",
    "days_since_last_request",
    "library_title_count",
    "season_count",
    "episode_count",
    "aired_episode_count",
    "unaired_episode_count",
    "next_episode_air_date",
    "theatrical_release_date",
    "digital_release_date",
    "physical_release_date",
    "home_release_available",
    "days_until_release",
    "other_requester_count",
    "pending_by_other_requester",
    "on_requester_watchlist",
    "requester_watchlist_days",
    "list_excluded",
    "requester_titles_in_library_count",
    "storage_root_id",
    "storage_available_bytes",
    "storage_total_bytes",
    "storage_available_percent",
];

#[test]
fn a_full_snapshot_answers_every_group() {
    let document = document(context_with(full_snapshot()));

    // These are legitimately absent from this movie fixture rather than known:
    // a movie has no network, first-aired date or series shape; this one has
    // no physical release and is already out.
    let expected_absent = [
        "network",
        "first_aired",
        "season_count",
        "episode_count",
        "aired_episode_count",
        "unaired_episode_count",
        "next_episode_air_date",
        "physical_release_date",
        "days_until_release",
    ];
    for fact in EVERY_FACT {
        let status = status_of(&document, fact);
        if expected_absent.contains(&fact) {
            assert_eq!(status, "absent", "{fact} should be an answered absence");
        } else {
            assert_eq!(status, "known", "{fact} should be known");
        }
    }

    assert_eq!(document["facts"]["certification_label"], "PG-13");
    assert_eq!(document["facts"]["certification_rank"], 2);
    assert_eq!(document["facts"]["age_rating"], 13);
    assert_eq!(document["facts"]["commonsense_recommended"], true);
    assert_eq!(document["facts"]["themes"], serde_json::json!(["Heist"]));
    assert_eq!(document["facts"]["is_adult"], false);
    assert_eq!(document["facts"]["award_count"], 1);
    assert_eq!(
        document["facts"]["awards"],
        serde_json::json!([{
            "award": "Fixture Prize",
            "award_id": "Q100",
            "year": 2023,
            "recipient": "Fixture Subject",
            "recipient_id": "Q200",
            "given_to": "work",
        }])
    );
    assert_eq!(document["facts"]["release_age_days"], 10);
    assert_eq!(document["facts"]["days_since_last_request"], 4);
    assert_eq!(document["facts"]["quality_profile_max_resolution"], 1080);
    assert_eq!(document["facts"]["library_title_count"], 412);
    assert_eq!(document["facts"]["ratings_by_source"]["imdb"], 0.74);
    assert_eq!(document["now"]["weekday"], "tuesday");
    assert_eq!(document["facts"]["theatrical_release_date"], "2023-11-04");
    assert_eq!(document["facts"]["digital_release_date"], "2023-11-10");
    assert_eq!(document["facts"]["home_release_available"], true);
    assert_eq!(
        reason_of(&document, "days_until_release"),
        fact_reason::ALREADY_RELEASED
    );
    assert_eq!(
        reason_of(&document, "season_count"),
        fact_reason::NOT_A_SERIES
    );
    assert_eq!(document["facts"]["other_requester_count"], 2);
    assert_eq!(document["facts"]["pending_by_other_requester"], true);
    assert_eq!(document["facts"]["on_requester_watchlist"], true);
    assert_eq!(document["facts"]["requester_watchlist_days"], 6);
    assert_eq!(document["facts"]["list_excluded"], false);
    assert_eq!(document["facts"]["requester_titles_in_library_count"], 5);
    assert_eq!(document["facts"]["storage_root_id"], "root-fixture");
    assert_eq!(document["facts"]["storage_available_bytes"], 500);
    assert_eq!(document["facts"]["storage_total_bytes"], 2_000);
    assert_eq!(document["facts"]["storage_available_percent"], 25.0);
    assert_eq!(
        document["observations"]["storage_available_bytes"]["observed_at"],
        (evaluation_time() - chrono::Duration::minutes(3)).to_rfc3339()
    );
    assert_eq!(document["requester"]["account_age_days"], 682);
}

#[test]
fn a_series_reads_its_shape_on_the_evaluation_day() {
    let document = document(series_context(series_snapshot()));

    assert_eq!(document["facts"]["season_count"], 2);
    assert_eq!(document["facts"]["episode_count"], 5);
    // The episode airing on the evaluation day counts as aired.
    assert_eq!(document["facts"]["aired_episode_count"], 4);
    assert_eq!(document["facts"]["unaired_episode_count"], 1);
    assert_eq!(document["facts"]["next_episode_air_date"], "2023-11-21");
    for fact in [
        "theatrical_release_date",
        "digital_release_date",
        "physical_release_date",
        "home_release_available",
        "days_until_release",
    ] {
        assert_eq!(status_of(&document, fact), "absent", "{fact}");
        assert_eq!(
            reason_of(&document, fact),
            fact_reason::NOT_A_MOVIE,
            "{fact}"
        );
    }
}

#[test]
fn a_series_with_nothing_scheduled_has_no_next_air_date() {
    let mut snapshot = series_snapshot();
    snapshot.series_shape.as_mut().unwrap().upcoming_air_dates = vec!["2023-11-01".to_string()];
    let document = document(series_context(snapshot));
    assert_eq!(status_of(&document, "next_episode_air_date"), "absent");
    assert_eq!(
        reason_of(&document, "next_episode_air_date"),
        fact_reason::NO_SCHEDULED_EPISODE
    );
    assert_eq!(document["facts"]["unaired_episode_count"], 1);
}

#[test]
fn a_snapshot_written_before_the_new_groups_decodes_with_them_unknown() {
    let mut json = serde_json::to_value(full_snapshot()).expect("snapshot serializes");
    let object = json.as_object_mut().expect("snapshot object");
    object.remove("series_shape");
    object.remove("release_dates");
    let old: MediaRequestMetadataSnapshot =
        serde_json::from_value(json).expect("an old snapshot still decodes");
    assert!(old.series_shape.is_none());
    assert!(old.release_dates.is_none());

    let movie = document(context_with(old.clone()));
    for fact in [
        "theatrical_release_date",
        "digital_release_date",
        "physical_release_date",
        "home_release_available",
        "days_until_release",
    ] {
        assert_eq!(status_of(&movie, fact), "unknown", "{fact}");
        assert_eq!(
            reason_of(&movie, fact),
            fact_reason::METADATA_UNAVAILABLE,
            "{fact}"
        );
    }
    let series = document(series_context(old));
    for fact in [
        "season_count",
        "episode_count",
        "aired_episode_count",
        "unaired_episode_count",
        "next_episode_air_date",
    ] {
        assert_eq!(status_of(&series, fact), "unknown", "{fact}");
        assert_eq!(
            reason_of(&series, fact),
            fact_reason::METADATA_UNAVAILABLE,
            "{fact}"
        );
    }
}

#[test]
fn release_dates_smg_has_not_collected_are_unknown_not_missing() {
    let mut snapshot = full_snapshot();
    snapshot.release_dates = Some(SnapshotReleaseDates::default());
    let document = document(context_with(snapshot));
    for fact in [
        "theatrical_release_date",
        "home_release_available",
        "days_until_release",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&document, fact), fact_reason::NOT_YET_COLLECTED);
    }
}

#[test]
fn an_upcoming_movie_counts_the_days_to_its_first_release() {
    let mut snapshot = full_snapshot();
    snapshot.release_dates = Some(SnapshotReleaseDates {
        fetched: true,
        theatrical: Some("2023-12-01".to_string()),
        digital: None,
        physical: Some("2024-02-01".to_string()),
    });
    let document = document(context_with(snapshot));
    assert_eq!(document["facts"]["days_until_release"], 17);
    assert_eq!(document["facts"]["home_release_available"], false);
    assert_eq!(
        reason_of(&document, "digital_release_date"),
        fact_reason::NO_DIGITAL_RELEASE_DATE
    );
}

#[test]
fn demand_facts_follow_the_catalog_read() {
    let mut context = context_with(full_snapshot());
    context.catalog.readable = false;
    let document = document(context);
    for fact in ["other_requester_count", "pending_by_other_requester"] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&document, fact), fact_reason::CATALOG_UNAVAILABLE);
    }
}

#[test]
fn watchlist_facts_say_why_they_have_no_value() {
    let mut context = context_with(full_snapshot());
    context.watchlist = RequestWatchlistContext::NotLinked;
    let doc = document(context);
    assert_eq!(status_of(&doc, "on_requester_watchlist"), "absent");
    assert_eq!(
        reason_of(&doc, "on_requester_watchlist"),
        fact_reason::NO_WATCHLIST_LINKED
    );

    let mut context = context_with(full_snapshot());
    context.watchlist = RequestWatchlistContext::Unavailable(fact_reason::WATCHLIST_STALE);
    let doc = document(context);
    for fact in ["on_requester_watchlist", "requester_watchlist_days"] {
        assert_eq!(status_of(&doc, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&doc, fact), fact_reason::WATCHLIST_STALE);
    }

    let mut context = context_with(full_snapshot());
    context.watchlist = RequestWatchlistContext::Known {
        first_seen_at: None,
    };
    let doc = document(context);
    assert_eq!(doc["facts"]["on_requester_watchlist"], false);
    assert_eq!(
        reason_of(&doc, "requester_watchlist_days"),
        fact_reason::NOT_ON_A_WATCHLIST
    );
}

#[test]
fn unreadable_exclusions_and_title_counts_are_unknown_not_false_or_zero() {
    let mut context = context_with(full_snapshot());
    context.list_excluded = None;
    context.requester_titles_in_library_count = None;
    let document = document(context);
    assert_eq!(
        reason_of(&document, "list_excluded"),
        fact_reason::LIST_EXCLUSIONS_UNAVAILABLE
    );
    assert_eq!(
        reason_of(&document, "requester_titles_in_library_count"),
        fact_reason::REQUESTER_TITLES_UNAVAILABLE
    );
}

#[test]
fn storage_without_a_current_sample_is_unknown_never_zero() {
    for reason in [
        fact_reason::STORAGE_NOT_SAMPLED,
        fact_reason::STORAGE_SAMPLE_STALE,
        fact_reason::STORAGE_CAPACITY_UNAVAILABLE,
    ] {
        let mut context = context_with(full_snapshot());
        context.storage.capacity = Err(reason);
        let document = document(context);
        assert_eq!(document["facts"]["storage_root_id"], "root-fixture");
        for fact in [
            "storage_available_bytes",
            "storage_total_bytes",
            "storage_available_percent",
        ] {
            assert_eq!(status_of(&document, fact), "unknown", "{fact}");
            assert_eq!(reason_of(&document, fact), reason, "{fact}");
        }
    }

    let mut context = context_with(full_snapshot());
    context.storage = RequestStorageContext::no_root();
    let document = document(context);
    assert_eq!(
        reason_of(&document, "storage_root_id"),
        fact_reason::STORAGE_ROOT_UNAVAILABLE
    );
}

#[test]
fn fact_reason_codes_are_distinct() {
    let mut seen = std::collections::HashSet::new();
    for code in fact_reason::ALL {
        assert!(seen.insert(*code), "{code} repeated");
        assert!(
            code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{code}"
        );
    }
}

#[test]
fn a_missing_snapshot_group_makes_only_its_own_facts_unknown() {
    let mut snapshot = full_snapshot();
    snapshot.partial = true;
    snapshot.missing = vec!["content_ratings".to_string()];
    let document = document(context_with(snapshot));

    for fact in [
        "age_rating",
        "certifications",
        "certification_label",
        "certification_rank",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&document, fact), "metadata_unavailable", "{fact}");
        // An unknown fact is absent from the bare namespace, so a rule reading
        // it matches nothing — and the engine holds it before it ever runs.
        assert!(document["facts"].get(fact).is_none(), "{fact}");
    }
    // Untouched groups stay known.
    assert_eq!(status_of(&document, "genres"), "known");
    assert_eq!(status_of(&document, "rating"), "known");
}

#[test]
fn a_wholly_unavailable_snapshot_makes_every_metadata_fact_unknown() {
    let document = document(context_with(MediaRequestMetadataSnapshot::unavailable(
        "enrichment_failed",
    )));

    for fact in [
        "age_rating",
        "certification_rank",
        "commonsense_recommended",
        "genres",
        "themes",
        "is_adult",
        "rating",
        "popularity",
        "runtime_minutes",
        "release_date",
        "release_age_days",
        "award_count",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
    }
    // The non-metadata facts are unaffected: the catalog, the profile and the
    // requester's history were all read successfully.
    assert_eq!(status_of(&document, "quality_profile_tiers"), "known");
    assert_eq!(status_of(&document, "previous_request_count"), "known");
    assert_eq!(status_of(&document, "pending_request_count"), "known");
}

#[test]
fn only_a_us_certification_is_ranked() {
    let mut snapshot = full_snapshot();
    snapshot.content_ratings = vec![ContentRating {
        country: "de".to_string(),
        certifications: vec![ContentCertification {
            value: "FSK 12".to_string(),
            source: "tmdb".to_string(),
            release_type: None,
        }],
        age_rating: Some(12),
        age_rating_source: Some("tmdb".to_string()),
    }];
    let document = document(context_with(snapshot));

    // The ladder is the US one, so a German label leaves both the label and the
    // rank *absent* — the source answered, Scryer's ladder just does not place
    // it — while the flattened certification list still carries the row.
    assert_eq!(status_of(&document, "certification_label"), "absent");
    assert_eq!(
        reason_of(&document, "certification_label"),
        "no_us_certification"
    );
    assert_eq!(status_of(&document, "certification_rank"), "absent");
    assert_eq!(status_of(&document, "certifications"), "known");
    // The age rating falls back to the non-US row rather than going absent: an
    // age is an age, whatever body published it.
    assert_eq!(document["facts"]["age_rating"], 12);
}

#[test]
fn an_unrankable_us_label_is_an_absence_not_an_unknown() {
    let mut snapshot = full_snapshot();
    snapshot.content_ratings = vec![us_rating("NR", 0)];
    let document = document(context_with(snapshot));

    assert_eq!(document["facts"]["certification_label"], "NR");
    assert_eq!(status_of(&document, "certification_rank"), "absent");
    assert_eq!(
        reason_of(&document, "certification_rank"),
        "unrankable_certification"
    );
}

#[test]
fn days_since_last_request_is_absent_for_a_first_time_requester() {
    let mut context = context_with(full_snapshot());
    context.history.last_request_at = None;
    let document = document(context);

    assert_eq!(status_of(&document, "days_since_last_request"), "absent");
    assert_eq!(
        reason_of(&document, "days_since_last_request"),
        "never_requested"
    );
}

#[test]
fn an_unreadable_history_makes_the_counters_unknown_not_zero() {
    let mut context = context_with(full_snapshot());
    context.history.readable = false;
    context.history.active_lease_count = None;
    let document = document(context);

    for fact in [
        "pending_request_count",
        "approved_last_30d",
        "denied_last_30d",
        "total_approved",
        "days_since_last_request",
        "active_lease_count",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
    }
    assert_eq!(
        reason_of(&document, "active_lease_count"),
        "lifecycle_claims_unavailable"
    );
}

#[test]
fn an_unresolvable_quality_profile_leaves_the_quality_facts_unknown() {
    let mut context = context_with(full_snapshot());
    context.quality = None;
    let document = document(context);

    for fact in [
        "quality_profile_tiers",
        "quality_profile_max_resolution",
        "quality_profile_allows_upgrades",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&document, fact), "quality_profile_unavailable");
    }
}

#[test]
fn exists_in_library_ids_carries_only_other_libraries() {
    // The assembler excludes the target library before it ever reaches the
    // builder; this pins that the builder passes the list through untouched, so
    // a request whose subject is nowhere else reads as an empty *known* list
    // rather than an unknown.
    let mut context = context_with(full_snapshot());
    context.catalog.exists_in_library_ids = Vec::new();
    let document = document(context);

    assert_eq!(status_of(&document, "exists_in_library_ids"), "known");
    assert_eq!(
        document["facts"]["exists_in_library_ids"],
        serde_json::json!([])
    );
}

#[test]
fn an_unreadable_catalog_makes_the_catalog_facts_unknown() {
    let mut context = context_with(full_snapshot());
    context.catalog.readable = false;
    let document = document(context);

    for fact in [
        "exists_in_library_ids",
        "previous_request_count",
        "previously_denied",
        "previously_approved",
    ] {
        assert_eq!(status_of(&document, fact), "unknown", "{fact}");
        assert_eq!(reason_of(&document, fact), "catalog_unavailable", "{fact}");
    }
}

#[test]
fn a_library_with_no_collected_count_reads_unknown() {
    let mut context = context_with(full_snapshot());
    context.library_title_count = None;
    let document = document(context);

    assert_eq!(status_of(&document, "library_title_count"), "unknown");
    assert_eq!(
        reason_of(&document, "library_title_count"),
        "not_yet_collected"
    );
}

#[test]
fn a_series_measures_its_age_from_the_first_air_date() {
    let mut snapshot = full_snapshot();
    snapshot.release_date = None;
    snapshot.first_aired = Some("2023-10-15T00:00:00Z".to_string());
    let document = document(context_with(snapshot));

    assert_eq!(status_of(&document, "release_date"), "absent");
    assert_eq!(document["facts"]["release_age_days"], 30);
}

#[test]
fn an_unparseable_release_date_is_an_absence() {
    let mut snapshot = full_snapshot();
    snapshot.release_date = Some("sometime in the eighties".to_string());
    snapshot.first_aired = None;
    let document = document(context_with(snapshot));

    assert_eq!(status_of(&document, "release_age_days"), "absent");
    assert_eq!(
        reason_of(&document, "release_age_days"),
        "unparseable_release_date"
    );
}

#[test]
fn the_lease_the_requester_asked_for_reaches_the_document() {
    let document = document(context_with(full_snapshot()));
    assert_eq!(document["request"]["lease_forever"], false);
    assert_eq!(document["request"]["lease_days"], 14);
}

fn list_draft(origin: scryer_domain::MediaRequestOrigin) -> crate::request_rules::RequestDraft {
    crate::request_rules::RequestDraft {
        facet: scryer_domain::MediaFacet::Movie,
        title: "Glass Harbor".to_string(),
        year: Some(2023),
        external_ids: Vec::new(),
        identity_fingerprint: "fixture-fingerprint".to_string(),
        quality_profile_id: None,
        quality_profile_name: None,
        monitor_type: None,
        monitor_selection: None,
        requested_lease_days: None,
        requested_required_audio_languages: None,
        requested_preferred_audio_languages: None,
        origin,
        list_rank: Some(3),
    }
}

/// The `request` section a draft assembles into against the harness's list
/// store.
async fn assembled_request(
    harness: &super::MediaRequestTestHarness,
    origin: scryer_domain::MediaRequestOrigin,
) -> serde_json::Value {
    let context = harness
        .app
        .assemble_request_input_context(
            &harness.user,
            &super::custom_movie_library("library-movies", "Movies"),
            &list_draft(origin),
            full_snapshot(),
            evaluation_time(),
            None,
            RequestFactReads::ALL,
        )
        .await
        .expect("a list read failure must not stop the request from evaluating");
    document(context)["request"].clone()
}

#[tokio::test]
async fn a_public_list_request_names_its_list() {
    let harness = super::bootstrap_media_request_app();
    let mut public = crate::lists::test_support::subscription("list-public");
    public.owner_user_id = harness.user.id.clone();
    *harness.lists.subscriptions.lock().unwrap() = vec![public];
    harness
        .lists
        .policies
        .lock()
        .unwrap()
        .push(scryer_domain::UserListPolicy {
            user_id: harness.user.id.clone(),
            policy: scryer_domain::ListPolicy::Auto,
            updated_by_user_id: None,
            updated_at: evaluation_time(),
        });

    let request = assembled_request(
        &harness,
        scryer_domain::MediaRequestOrigin::PublicList {
            subscription_id: "list-public".to_string(),
        },
    )
    .await;

    assert_eq!(request["origin"], "public_list");
    assert_eq!(
        request["list"],
        serde_json::json!({
            "subscription_id": "list-public",
            "name": "Fixture list list-public",
            "scope": "public",
            "mode": "add",
            "provider": crate::lists::test_support::PROVIDER,
            "owner_is_requester": true,
            "owner_list_policy": "auto",
            "rank": 3,
            "is_watchlist": false,
            "source_type": "user_list",
        })
    );
}

#[tokio::test]
async fn a_personal_list_request_never_names_its_list() {
    let harness = super::bootstrap_media_request_app();
    let mut personal = crate::lists::test_support::subscription("list-personal");
    personal.scope = scryer_domain::ListScope::Personal;
    personal.name = "Private picks of a member".to_string();
    *harness.lists.subscriptions.lock().unwrap() = vec![personal];

    let request = assembled_request(
        &harness,
        scryer_domain::MediaRequestOrigin::PersonalList {
            subscription_id: "list-personal".to_string(),
        },
    )
    .await;

    assert_eq!(request["origin"], "personal_list");
    let list = request["list"].as_object().expect("a list section");
    assert!(!list.contains_key("name"), "a personal list is never named");
    assert!(
        !request.to_string().contains("Private picks of a member"),
        "the personal list name leaked into the request section"
    );
    assert_eq!(list["scope"], "personal");
    assert_eq!(
        list["owner_is_requester"], false,
        "the fixture list belongs to another member"
    );
    assert_eq!(
        list["owner_list_policy"], "approval",
        "an owner with no stored policy is on the default"
    );
}

#[tokio::test]
async fn a_manual_request_has_no_list_section() {
    let harness = super::bootstrap_media_request_app();

    let request = assembled_request(&harness, scryer_domain::MediaRequestOrigin::Manual).await;

    assert_eq!(request["origin"], "manual");
    assert!(
        request.get("list").is_none(),
        "the section is absent, not null"
    );
}

#[tokio::test]
async fn an_unreadable_list_store_drops_the_section_and_still_evaluates() {
    let harness = super::bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() =
        vec![crate::lists::test_support::subscription("list-public")];
    *harness.lists.subscription_reads_left.lock().unwrap() = Some(0);

    let request = assembled_request(
        &harness,
        scryer_domain::MediaRequestOrigin::PublicList {
            subscription_id: "list-public".to_string(),
        },
    )
    .await;

    assert_eq!(request["origin"], "public_list");
    assert!(request.get("list").is_none());
}

/// The whole facts document an assembled draft produces against the harness.
async fn assembled_document(
    harness: &super::MediaRequestTestHarness,
    draft: &crate::request_rules::RequestDraft,
) -> serde_json::Value {
    assembled_document_reading(harness, draft, RequestFactReads::ALL).await
}

async fn assembled_document_reading(
    harness: &super::MediaRequestTestHarness,
    draft: &crate::request_rules::RequestDraft,
    reads: RequestFactReads,
) -> serde_json::Value {
    let context = harness
        .app
        .assemble_request_input_context(
            &harness.user,
            &super::custom_movie_library("library-movies", "Movies"),
            draft,
            full_snapshot(),
            evaluation_time(),
            None,
            reads,
        )
        .await
        .expect("the request evaluates");
    document(context)
}

#[tokio::test]
async fn request_evaluation_reads_storage_from_the_sampler_and_never_the_disk() {
    let harness = super::bootstrap_media_request_app();
    let capacity = &harness.app.runtime.health.storage_capacity;
    // The movie library's only root, sampled two minutes before evaluation.
    capacity.record_for_test(
        "/data/movies",
        evaluation_time() - chrono::Duration::minutes(2),
        Some(crate::helpers::FilesystemSpace {
            total_bytes: 4_000,
            available_bytes: 1_000,
        }),
    );
    let calls_before = capacity.disk_calls();

    let document = assembled_document(
        &harness,
        &list_draft(scryer_domain::MediaRequestOrigin::Manual),
    )
    .await;

    assert_eq!(
        capacity.disk_calls(),
        calls_before,
        "evaluation made a disk call"
    );
    assert_eq!(
        document["facts"]["storage_root_id"],
        scryer_domain::root_folder_id_for_path("/data/movies")
    );
    assert_eq!(document["facts"]["storage_available_bytes"], 1_000);
    assert_eq!(document["facts"]["storage_total_bytes"], 4_000);
    assert_eq!(document["facts"]["storage_available_percent"], 25.0);
}

#[tokio::test]
async fn a_root_sampled_too_long_ago_reads_stale_and_still_makes_no_call() {
    let harness = super::bootstrap_media_request_app();
    let capacity = &harness.app.runtime.health.storage_capacity;
    capacity.record_for_test(
        "/data/movies",
        evaluation_time() - chrono::Duration::hours(1),
        Some(crate::helpers::FilesystemSpace {
            total_bytes: 4_000,
            available_bytes: 1_000,
        }),
    );
    let calls_before = capacity.disk_calls();

    let document = assembled_document(
        &harness,
        &list_draft(scryer_domain::MediaRequestOrigin::Manual),
    )
    .await;

    assert_eq!(capacity.disk_calls(), calls_before);
    assert_eq!(status_of(&document, "storage_available_bytes"), "unknown");
    assert_eq!(
        reason_of(&document, "storage_available_bytes"),
        fact_reason::STORAGE_SAMPLE_STALE
    );
}

fn linked_watchlist(
    owner: &str,
    last_at: chrono::DateTime<chrono::Utc>,
) -> scryer_domain::ListSubscription {
    let mut watchlist = crate::lists::test_support::subscription("watchlist-own");
    watchlist.scope = scryer_domain::ListScope::Personal;
    watchlist.owner_user_id = owner.to_string();
    watchlist.source.provider = "tmdb".to_string();
    watchlist.source.source_type = "watchlist".to_string();
    watchlist.credential_id = Some("credential-fixture".to_string());
    watchlist.sync.state = scryer_domain::ListSyncState::Ok;
    watchlist.sync.last_at = Some(last_at);
    watchlist
}

fn watchlisted_draft() -> crate::request_rules::RequestDraft {
    let mut draft = list_draft(scryer_domain::MediaRequestOrigin::Manual);
    draft.external_ids = vec![crate::lists::test_support::tmdb("glass-harbor")];
    draft
}

#[tokio::test]
async fn a_title_on_the_requesters_synced_watchlist_counts_its_days() {
    let harness = super::bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![linked_watchlist(
        &harness.user.id,
        evaluation_time() - chrono::Duration::hours(2),
    )];
    let mut row = crate::lists::test_support::membership(
        "watchlist-own",
        "glass-harbor",
        scryer_domain::ListMembershipState::Pending,
    );
    row.external_ids = vec![crate::lists::test_support::tmdb("glass-harbor")];
    row.first_seen_at = evaluation_time() - chrono::Duration::days(9);
    // The same id on a series row is another title, not this movie.
    let mut series_row = row.clone();
    series_row.item_key = "glass-harbor-series".to_string();
    series_row.kind = scryer_domain::MediaFacet::Series;
    series_row.first_seen_at = evaluation_time() - chrono::Duration::days(40);
    harness.lists.insert_rows(vec![row, series_row]);

    let document = assembled_document(&harness, &watchlisted_draft()).await;

    assert_eq!(document["facts"]["on_requester_watchlist"], true);
    assert_eq!(document["facts"]["requester_watchlist_days"], 9);
}

#[tokio::test]
async fn skipping_the_watchlist_read_changes_only_the_watchlist_facts() {
    let harness = super::bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![linked_watchlist(
        &harness.user.id,
        evaluation_time() - chrono::Duration::hours(2),
    )];
    let mut row = crate::lists::test_support::membership(
        "watchlist-own",
        "glass-harbor",
        scryer_domain::ListMembershipState::Pending,
    );
    row.external_ids = vec![crate::lists::test_support::tmdb("glass-harbor")];
    row.first_seen_at = evaluation_time() - chrono::Duration::days(9);
    harness.lists.insert_rows(vec![row]);

    let read =
        assembled_document_reading(&harness, &watchlisted_draft(), RequestFactReads::ALL).await;
    let skipped = assembled_document_reading(
        &harness,
        &watchlisted_draft(),
        RequestFactReads {
            watchlist: false,
            ..RequestFactReads::ALL
        },
    )
    .await;

    // A rule that names a watchlist fact always gets the read, and with it the
    // same values as before.
    assert_eq!(read["facts"]["on_requester_watchlist"], true);
    assert_eq!(read["facts"]["requester_watchlist_days"], 9);
    for fact in crate::request_rules::facts::WATCHLIST_FACT_NAMES {
        assert_eq!(status_of(&skipped, fact), "unknown");
        assert_eq!(reason_of(&skipped, fact), fact_reason::NOT_YET_COLLECTED);
    }

    // Everything no rule can be denied by is identical either way.
    let without_watchlist = |mut document: serde_json::Value| {
        for section in ["facts", "observations"] {
            if let Some(object) = document
                .get_mut(section)
                .and_then(serde_json::Value::as_object_mut)
            {
                for fact in crate::request_rules::facts::WATCHLIST_FACT_NAMES {
                    object.remove(*fact);
                }
            }
        }
        document
    };
    assert_eq!(without_watchlist(read), without_watchlist(skipped));
}

/// Remove `names` from a document's facts and observations.
fn without_facts(mut document: serde_json::Value, names: &[&str]) -> serde_json::Value {
    for section in ["facts", "observations"] {
        if let Some(object) = document
            .get_mut(section)
            .and_then(serde_json::Value::as_object_mut)
        {
            for name in names {
                object.remove(*name);
            }
        }
    }
    document
}

#[test]
fn an_unread_family_reads_unknown_and_leaves_every_other_fact_alone() {
    use crate::request_rules::facts::{
        CATALOG_FACT_NAMES, LIBRARY_TITLE_COUNT_FACT_NAMES, LIST_EXCLUSION_FACT_NAMES,
        QUALITY_FACT_NAMES, REQUESTER_HISTORY_FACT_NAMES, REQUESTER_TITLES_FACT_NAMES,
        WATCHLIST_FACT_NAMES,
    };
    let full = document(context_with(full_snapshot()));
    type DisableFactFamily = fn(&mut RequestFactReads);
    let families: [(&[&str], DisableFactFamily); 7] = [
        (QUALITY_FACT_NAMES, |reads| reads.quality = false),
        (CATALOG_FACT_NAMES, |reads| reads.catalog = false),
        (REQUESTER_HISTORY_FACT_NAMES, |reads| {
            reads.requester_history = false
        }),
        (LIBRARY_TITLE_COUNT_FACT_NAMES, |reads| {
            reads.library_title_count = false
        }),
        (WATCHLIST_FACT_NAMES, |reads| reads.watchlist = false),
        (LIST_EXCLUSION_FACT_NAMES, |reads| {
            reads.list_exclusion = false
        }),
        (REQUESTER_TITLES_FACT_NAMES, |reads| {
            reads.requester_titles = false
        }),
    ];
    for (names, unread) in families {
        let mut context = context_with(full_snapshot());
        unread(&mut context.fact_reads);
        let skipped = document(context);
        for name in names {
            // Every name is a real fact, known in the full document.
            assert_eq!(status_of(&full, name), "known", "{name}");
            assert_eq!(status_of(&skipped, name), "unknown", "{name}");
            assert_eq!(
                reason_of(&skipped, name),
                fact_reason::NOT_YET_COLLECTED,
                "{name}"
            );
        }
        assert_eq!(
            without_facts(full.clone(), names),
            without_facts(skipped, names)
        );
    }
}

#[tokio::test]
async fn assembling_with_every_family_unread_matches_the_full_document_elsewhere() {
    let harness = super::bootstrap_media_request_app();
    let read = assembled_document(&harness, &watchlisted_draft()).await;
    let skipped =
        assembled_document_reading(&harness, &watchlisted_draft(), RequestFactReads::NONE).await;
    let unread: Vec<&str> = [
        crate::request_rules::facts::QUALITY_FACT_NAMES,
        crate::request_rules::facts::CATALOG_FACT_NAMES,
        crate::request_rules::facts::REQUESTER_HISTORY_FACT_NAMES,
        crate::request_rules::facts::LIBRARY_TITLE_COUNT_FACT_NAMES,
        crate::request_rules::facts::WATCHLIST_FACT_NAMES,
        crate::request_rules::facts::LIST_EXCLUSION_FACT_NAMES,
        crate::request_rules::facts::REQUESTER_TITLES_FACT_NAMES,
    ]
    .concat();
    for name in &unread {
        assert_eq!(
            reason_of(&skipped, name),
            fact_reason::NOT_YET_COLLECTED,
            "{name}"
        );
    }
    assert_eq!(
        without_facts(read, &unread),
        without_facts(skipped, &unread)
    );
}

#[tokio::test]
async fn a_watchlist_that_has_not_synced_lately_is_unknown_not_false() {
    let harness = super::bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![linked_watchlist(
        &harness.user.id,
        evaluation_time() - chrono::Duration::days(5),
    )];

    let document = assembled_document(&harness, &watchlisted_draft()).await;

    assert_eq!(status_of(&document, "on_requester_watchlist"), "unknown");
    assert_eq!(
        reason_of(&document, "on_requester_watchlist"),
        fact_reason::WATCHLIST_STALE
    );
}

#[tokio::test]
async fn another_members_watchlist_is_not_the_requesters() {
    let harness = super::bootstrap_media_request_app();
    *harness.lists.subscriptions.lock().unwrap() = vec![linked_watchlist(
        "someone-else",
        evaluation_time() - chrono::Duration::hours(2),
    )];

    let document = assembled_document(&harness, &watchlisted_draft()).await;

    assert_eq!(status_of(&document, "on_requester_watchlist"), "absent");
    assert_eq!(
        reason_of(&document, "on_requester_watchlist"),
        fact_reason::NO_WATCHLIST_LINKED
    );
}
