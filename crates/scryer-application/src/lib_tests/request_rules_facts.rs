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

use crate::media_requests::snapshot::MediaRequestMetadataSnapshot;
use crate::request_rules::facts::{
    RequestCatalogContext, RequestInputContext, RequestQualityContext,
    RequestRequesterHistoryContext, build_request_input,
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
            claim_side: "nominee".to_string(),
        }],
        is_adult: false,
    }
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
            created_at: None,
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
            lease_forever: false,
            lease_days: Some(14),
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
const EVERY_FACT: [&str; 38] = [
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
];

#[test]
fn a_full_snapshot_answers_every_group() {
    let document = document(context_with(full_snapshot()));

    // Two facts are legitimately absent from this fixture rather than known:
    // a movie has no network, and a movie has no first-aired date.
    let expected_absent = ["network", "first_aired"];
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
    assert_eq!(document["facts"]["release_age_days"], 10);
    assert_eq!(document["facts"]["days_since_last_request"], 4);
    assert_eq!(document["facts"]["quality_profile_max_resolution"], 1080);
    assert_eq!(document["facts"]["library_title_count"], 412);
    assert_eq!(document["facts"]["ratings_by_source"]["imdb"], 0.74);
    assert_eq!(document["now"]["weekday"], "tuesday");
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
