use super::StoredSettingsRepo;
use super::support_bootstrap_fixtures::{
    TestPermissionPreset, bootstrap_with_metadata_gateway_and_titles,
    bootstrap_with_metadata_gateway_settings_and_titles, create_authenticated_user,
    library_permission_user, library_permission_user_with_grants, test_series_movie_link,
};
use crate::ports::{
    CatalogDiscoveryCandidatesRecord, CatalogDiscoverySectionCandidatesRecord,
    DiscoveryHomeCandidate, DiscoveryHomeSectionCandidatesRecord,
    DiscoveryItemLibraryProvenanceRecord, DiscoveryItemsPageRecord, DiscoveryItemsStorageQuery,
    DiscoverySourceTagRecord,
};
use crate::settings::keys::{DISCOVERY_REGION_KEY, METADATA_LANGUAGE_KEY, SETTINGS_SCOPE_SYSTEM};
use crate::{
    AppError, AppResult, BulkMetadataResult, CatalogDiscoveryGroupKind, CatalogDiscoveryQuery,
    CatalogDiscoverySurface, DiscoveryContextChangeType, DiscoveryContextChangesInput,
    DiscoveryContextChangesResult, DiscoveryContextIncrementalCommit,
    DiscoveryContextSnapshotAckResult, DiscoveryContextSnapshotCommit,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotStatusResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryContextSnapshotSubmitResult,
    DiscoveryDashboardResult, DiscoveryDashboardSection, DiscoveryFacetRecord,
    DiscoveryHomeFilterOptions, DiscoveryHomeFilters, DiscoveryHomeQuery, DiscoveryItemDetailQuery,
    DiscoveryItemRecord, DiscoveryItemsQuery, DiscoveryPendingContextChangeRecord,
    DiscoveryPublicFeedCommit, DiscoveryPublicFeedInput, DiscoveryRelatedResult,
    DiscoveryRepository, DiscoverySectionRecord, DiscoverySnapshotFacetGroup,
    DiscoverySnapshotFacetValue, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord, DiscoveryTitle, DomainEventRepository, JobCategory, JobKey, JobRun,
    JobRunStatus, JobSection, JobTriggerSource, LibraryRootDraft, MetadataGateway,
    MetadataSearchItem, MetadataSearchQuery, MovieMetadata, MultiMetadataSearchResult,
    RichMetadataSearchItem, SeriesMetadata, TitleRecommendationsInput,
};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use scryer_domain::{
    CanonicalMediaTag, DomainEventPayload, DomainExternalIds, ExternalId, JobRunStartedEventData,
    LibraryScanCanceledEventData, LibraryScanCompletedEventData, LibraryScanFailedEventData,
    LibraryScanStartedEventData, MediaFacet, Title, TitleContextSnapshot, TitleUpdatedEventData,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

fn canonical_genre_tags(labels: &[&str]) -> Vec<CanonicalMediaTag> {
    labels
        .iter()
        .map(|label| CanonicalMediaTag {
            key: format!(
                "canonical:genre:{}",
                label.to_ascii_lowercase().replace(' ', "-")
            ),
            category: "genre".to_string(),
            name: (*label).to_string(),
            confidence: Some(1.0),
            sources: Vec::new(),
            source_tag_keys: Vec::new(),
            is_adult: false,
            is_spoiler: false,
        })
        .collect()
}

#[tokio::test]
async fn discovery_sync_status_returns_state_recent_runs_and_pending_count() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    let visible_movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut owned_drama_sci_fi = test_title(
        "owned-1",
        "Owned Drama Sci-Fi",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    owned_drama_sci_fi.library_id = visible_movie_library_id.clone();
    owned_drama_sci_fi.canonical_tags = canonical_genre_tags(&["Drama", "Sci-Fi"]);
    owned_drama_sci_fi.tags = vec!["horror".to_string(), "isekai".to_string()];
    let mut owned_drama = test_title(
        "owned-2",
        "Owned Drama",
        MediaFacet::Movie,
        vec![("tmdb_movie", "604")],
    );
    owned_drama.library_id = visible_movie_library_id.clone();
    owned_drama.canonical_tags = canonical_genre_tags(&["Drama"]);
    owned_drama.tags = vec!["horror".to_string()];
    let mut owned_sci_fi = test_title(
        "owned-3",
        "Owned Sci-Fi",
        MediaFacet::Movie,
        vec![("tmdb_movie", "605")],
    );
    owned_sci_fi.library_id = visible_movie_library_id.clone();
    owned_sci_fi.canonical_tags = canonical_genre_tags(&["Sci-Fi"]);
    owned_sci_fi.tags = vec!["isekai".to_string()];
    titles
        .store
        .lock()
        .await
        .extend([owned_drama_sci_fi, owned_drama, owned_sci_fi]);

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("run-current".to_string()),
        last_seen_domain_event_sequence: Some(42),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.runs.lock().await.extend([
        discovery_run_record("run-old", observed_at, "complete"),
        discovery_run_record("run-current", observed_at, "complete"),
    ]);
    discovery.pending_changes.lock().await.extend([
        discovery_pending_change_record("change-default", crate::DISCOVERY_DEFAULT_SCOPE_KEY),
        discovery_pending_change_record("change-other", "other-scope"),
    ]);

    let status = app
        .discovery_sync_status(&admin)
        .await
        .expect("discovery status should be readable");

    assert_eq!(
        status.state.last_success_generation_id.as_deref(),
        Some("run-current")
    );
    assert_eq!(status.state.last_seen_domain_event_sequence, Some(42));
    assert_eq!(status.pending_context_change_count, 1);
    assert_eq!(status.recent_runs.len(), 2);
    assert_eq!(status.recent_runs[0].id, "run-current");
}

#[tokio::test]
async fn discovery_sync_recovers_committed_unacked_snapshot_before_new_submit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("run-unacked".to_string()),
        last_public_feed_generation_id: Some("public-current".to_string()),
        last_subject_fingerprint: Some("existing-fingerprint".to_string()),
        next_context_snapshot_eligible_at: Some(observed_at + chrono::Duration::hours(24)),
        next_incremental_reload_eligible_at: Some(observed_at + chrono::Duration::hours(4)),
        next_public_feed_eligible_at: Some(observed_at + chrono::Duration::hours(24)),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    let mut run = discovery_run_record("run-unacked", observed_at, "complete");
    run.smg_request_id = Some("request-unacked".to_string());
    run.acknowledged_at = None;
    discovery.runs.lock().await.push(run);

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should recover ack");

    assert_eq!(
        gateway.ack_requests.lock().await.as_slice(),
        ["request-unacked"]
    );
    assert!(
        gateway.submitted_inputs.lock().await.is_empty(),
        "ack recovery should run before any new context submit"
    );
    let runs = discovery.runs.lock().await;
    let recovered = runs
        .iter()
        .find(|run| run.id == "run-unacked")
        .expect("unacked run should remain in ledger");
    assert_eq!(recovered.status, "complete");
    assert!(recovered.acknowledged_at.is_some());
    assert!(recovered.error_text.is_none());
}

#[tokio::test]
async fn discovery_home_and_items_use_local_rows_and_library_view_rbac() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let (_public_user, public_actor) = create_authenticated_user(
        &app,
        &admin,
        "discovery-public",
        "password",
        vec![TestPermissionPreset::MediaRequest],
    )
    .await;
    let (_viewer, viewer_actor) = create_authenticated_user(
        &app,
        &admin,
        "discovery-viewer",
        "password",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::MediaRequest,
        ],
    )
    .await;
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    let visible_movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let hidden_movie_library_id = "hidden-movie-library".to_string();
    let mut owned_drama_sci_fi = test_title(
        "title-603",
        "Owned Drama Sci-Fi",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    owned_drama_sci_fi.library_id = visible_movie_library_id.clone();
    owned_drama_sci_fi.canonical_tags = canonical_genre_tags(&["Drama", "Sci-Fi"]);
    owned_drama_sci_fi.tags = vec!["horror".to_string(), "isekai".to_string()];
    let mut owned_drama = test_title(
        "owned-2",
        "Owned Drama",
        MediaFacet::Movie,
        vec![("tmdb_movie", "604")],
    );
    owned_drama.library_id = visible_movie_library_id.clone();
    owned_drama.canonical_tags = canonical_genre_tags(&["Drama"]);
    owned_drama.tags = vec!["horror".to_string()];
    let mut owned_sci_fi = test_title(
        "owned-3",
        "Owned Sci-Fi",
        MediaFacet::Movie,
        vec![("tmdb_movie", "605")],
    );
    owned_sci_fi.library_id = visible_movie_library_id.clone();
    owned_sci_fi.canonical_tags = canonical_genre_tags(&["Sci-Fi"]);
    owned_sci_fi.tags = vec!["isekai".to_string()];
    titles
        .store
        .lock()
        .await
        .extend([owned_drama_sci_fi, owned_drama, owned_sci_fi]);

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .sections
        .lock()
        .await
        .push(discovery_section_record(
            "public-run",
            "trending",
            "TRENDING",
            "public",
        ));
    discovery.submitted_subjects.lock().await.extend([
        DiscoverySubmittedSubjectRecord {
            run_id: "context-run".to_string(),
            subject_key: "tmdb:movie:603".to_string(),
            title_id: Some("title-603".to_string()),
            library_id: Some(visible_movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Local Example Movie".to_string()),
            external_ids_json: serde_json::json!([{"source": "tmdb", "value": "603"}]).to_string(),
            raw_subject_json: serde_json::json!({"tmdbId": 603}).to_string(),
        },
        DiscoverySubmittedSubjectRecord {
            run_id: "context-run".to_string(),
            subject_key: "tmdb:movie:604".to_string(),
            title_id: Some("owned-2".to_string()),
            library_id: Some(visible_movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Owned Drama".to_string()),
            external_ids_json: serde_json::json!([{"source": "tmdb", "value": "604"}]).to_string(),
            raw_subject_json: serde_json::json!({"tmdbId": 604}).to_string(),
        },
        DiscoverySubmittedSubjectRecord {
            run_id: "context-run".to_string(),
            subject_key: "tmdb:movie:605".to_string(),
            title_id: Some("owned-3".to_string()),
            library_id: Some(visible_movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Owned Sci-Fi".to_string()),
            external_ids_json: serde_json::json!([{"source": "tmdb", "value": "605"}]).to_string(),
            raw_subject_json: serde_json::json!({"tmdbId": 605}).to_string(),
        },
        DiscoverySubmittedSubjectRecord {
            run_id: "context-run".to_string(),
            subject_key: "tmdb:movie:999".to_string(),
            title_id: Some("hidden-title".to_string()),
            library_id: Some(hidden_movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Hidden Movie".to_string()),
            external_ids_json: serde_json::json!([{"source": "tmdb", "value": "999"}]).to_string(),
            raw_subject_json: serde_json::json!({"tmdbId": 999}).to_string(),
        },
    ]);
    let mut private_recommendation = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:200",
        "Private Recommendation",
        "movie",
        90.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    private_recommendation.matched_subject_keys =
        vec!["tmdb:movie:603".to_string(), "tmdb:movie:999".to_string()];
    private_recommendation.matched_subject_titles = vec!["SMG should not leak this".to_string()];
    private_recommendation.matched_subject_count = 2;
    private_recommendation.library_provenance = vec![
        DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:603".to_string(),
            title_id: Some("title-603".to_string()),
            library_id: Some(visible_movie_library_id.clone()),
        },
        DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:999".to_string(),
            title_id: Some("hidden-title".to_string()),
            library_id: Some(hidden_movie_library_id.clone()),
        },
    ];
    let linked_subject_keys = vec!["tmdb:movie:603".to_string()];
    let top_context_terms = vec!["weekly".to_string(), "popular".to_string()];
    let mut sci_fi_one = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:300",
        "Sci-Fi One",
        "movie",
        89.0,
        &["Sci-Fi"],
        &[],
        false,
        true,
    );
    sci_fi_one.matched_subject_keys = linked_subject_keys.clone();
    sci_fi_one.context_terms = top_context_terms.clone();
    let mut sci_fi_two = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:301",
        "Sci-Fi Two",
        "movie",
        88.0,
        &["Sci-Fi"],
        &[],
        false,
        true,
    );
    sci_fi_two.matched_subject_keys = linked_subject_keys.clone();
    sci_fi_two.context_terms = top_context_terms.clone();
    let mut drama_one = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:302",
        "Drama One",
        "movie",
        87.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    drama_one.matched_subject_keys = linked_subject_keys.clone();
    drama_one.context_terms = top_context_terms.clone();
    let mut drama_two = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:303",
        "Drama Two",
        "movie",
        86.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    drama_two.matched_subject_keys = linked_subject_keys.clone();
    drama_two.rating = Some(8.7);
    let mut high_rated_sci_fi = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:304",
        "High-Rated Sci-Fi",
        "movie",
        85.0,
        &["Sci-Fi"],
        &[],
        false,
        true,
    );
    high_rated_sci_fi.matched_subject_keys = linked_subject_keys.clone();
    high_rated_sci_fi.rating = Some(9.1);
    let mut horror_item = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:305",
        "Horror Match",
        "movie",
        84.0,
        &["Horror"],
        &[],
        false,
        true,
    );
    horror_item.matched_subject_keys = linked_subject_keys.clone();
    let mut isekai_item = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:306",
        "Isekai Match",
        "movie",
        83.0,
        &["Fantasy"],
        &[],
        false,
        true,
    );
    isekai_item.matched_subject_keys = linked_subject_keys.clone();
    isekai_item
        .facet_terms
        .push("canonical:theme:isekai".to_string());
    isekai_item.source_tags = vec![DiscoverySourceTagRecord {
        category: Some("theme".to_string()),
        name: Some("Isekai".to_string()),
        values: vec!["mal".to_string(), "theme".to_string(), "Isekai".to_string()],
    }];
    let mut weekly_series_one = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tvdb:series:400",
        "Weekly Series One",
        "series",
        81.0,
        &["Mystery"],
        &[],
        false,
        true,
    );
    weekly_series_one.context_terms = top_context_terms.clone();
    let mut weekly_series_two = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tvdb:series:401",
        "Weekly Series Two",
        "series",
        80.0,
        &["Sci-Fi"],
        &[],
        false,
        true,
    );
    weekly_series_two.context_terms = top_context_terms.clone();
    let mut weekly_anime_one = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "mal:anime:500",
        "Weekly Anime One",
        "anime",
        79.0,
        &["Fantasy"],
        &[],
        false,
        true,
    );
    weekly_anime_one.context_terms = top_context_terms.clone();
    let mut weekly_anime_two = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "mal:anime:501",
        "Weekly Anime Two",
        "anime",
        78.0,
        &["Adventure"],
        &[],
        false,
        true,
    );
    weekly_anime_two.context_terms = top_context_terms.clone();
    let mut high_rated_mystery = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:308",
        "High-Rated Mystery",
        "movie",
        82.5,
        &["Mystery"],
        &[],
        false,
        true,
    );
    high_rated_mystery.rating = Some(9.2);
    let mut collection_signal_movie = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:307",
        "Collection Signal Movie",
        "movie",
        82.0,
        &["Adventure"],
        &["tmdb.collection"],
        false,
        true,
    );
    collection_signal_movie.tmdb_collection_id = None;
    collection_signal_movie.tmdb_collection_name = None;
    let mut hidden_recommendation = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:998",
        "Hidden Recommendation",
        "movie",
        96.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    hidden_recommendation.matched_subject_keys = vec!["tmdb:movie:999".to_string()];
    hidden_recommendation.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
        subject_key: "tmdb:movie:999".to_string(),
        title_id: Some("hidden-title".to_string()),
        library_id: Some("hidden-movie-library".to_string()),
    }];
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending"),
            "tmdb:movie:100",
            "Public Movie",
            "movie",
            50.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        private_recommendation,
        sci_fi_one,
        sci_fi_two,
        drama_one,
        drama_two,
        high_rated_sci_fi,
        horror_item,
        isekai_item,
        high_rated_mystery,
        weekly_series_one,
        weekly_series_two,
        weekly_anime_one,
        weekly_anime_two,
        collection_signal_movie,
        hidden_recommendation,
        discovery_item_record(
            "context-run",
            "context-run",
            None,
            "tmdb:movie:201",
            "Collection Movie",
            "movie",
            95.0,
            &["Adventure"],
            &["tmdb.collection"],
            false,
            true,
        ),
        discovery_item_record(
            "context-run",
            "context-run",
            None,
            "tmdb:movie:202",
            "Owned Movie",
            "movie",
            99.0,
            &["Drama"],
            &[],
            true,
            true,
        ),
    ]);
    discovery.facets.lock().await.push(DiscoveryFacetRecord {
        run_id: "context-run".to_string(),
        facet_name: "genre".to_string(),
        facet_value: "Drama".to_string(),
        smg_count: Some(20),
        local_count: None,
    });

    let public_home = app
        .discovery_home(
            &public_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: false,
                limit_per_section: 10,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("public discovery home should load");
    assert!(!public_home.can_view_personalized);
    let public_section_types = public_home
        .public_sections
        .iter()
        .map(|section| section.section_type.as_str())
        .collect::<Vec<_>>();
    assert!(public_section_types.contains(&"TOP_RATED"));
    assert!(public_home.public_sections.iter().any(|section| {
        section.section_type != "TOP_RATED"
            && section
                .items
                .iter()
                .any(|item| item.display_title == "Public Movie")
    }));
    assert!(public_home.personalized_sections.is_empty());
    assert!(public_home.complete_collection.is_none());
    assert!(
        public_home
            .status
            .state
            .last_success_generation_id
            .is_none()
    );

    let viewer_home = app
        .discovery_home(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: false,
                limit_per_section: 10,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("viewer discovery home should load");
    assert!(viewer_home.can_view_personalized);
    assert!(viewer_home.complete_collection.is_some());
    let complete_collection = viewer_home
        .complete_collection
        .as_ref()
        .expect("complete collection should be present");
    assert!(complete_collection.items.iter().any(|item| {
        item.display_title == "Collection Signal Movie"
            && item.tmdb_collection_id.is_none()
            && item.tmdb_collection_name.is_none()
    }));
    let drama_facet = viewer_home
        .facets
        .iter()
        .find(|facet| facet.facet_name == "genre" && facet.facet_value == "Drama")
        .expect("canonical drama facet should be present");
    assert_eq!(drama_facet.local_count, Some(3));
    assert_eq!(drama_facet.smg_count, None);
    assert!(
        viewer_home
            .facets
            .iter()
            .any(|facet| facet.facet_name == "genre" && facet.facet_value == "Sci Fi")
    );
    assert!(
        viewer_home
            .facets
            .iter()
            .any(|facet| facet.facet_name == "theme" && facet.facet_value == "Isekai")
    );
    assert!(viewer_home.facets.iter().all(|facet| {
        !facet.facet_value.contains(':')
            && facet.facet_value != "Sci-Fi"
            && facet.smg_count.is_none()
    }));
    let personalized_section_types = viewer_home
        .personalized_sections
        .iter()
        .map(|section| section.section_type.as_str())
        .collect::<Vec<_>>();
    assert!(!personalized_section_types.contains(&"TOP_MOVIES_THIS_WEEK"));
    assert!(!personalized_section_types.contains(&"TOP_SERIES_THIS_WEEK"));
    assert!(!personalized_section_types.contains(&"TOP_ANIME_THIS_WEEK"));
    assert!(!personalized_section_types.contains(&"MORE_FROM_SOURCE"));
    assert!(personalized_section_types.contains(&"BECAUSE_YOU_LIKE_TAG"));
    assert!(viewer_home.personalized_sections.iter().any(|section| {
        section.section_type == "BECAUSE_YOU_LIKE_GENRE"
            && section.title == "Because You Like Sci Fi"
    }));
    assert!(viewer_home.personalized_sections.iter().any(|section| {
        section.section_type == "BECAUSE_YOU_LIKE_GENRE"
            && section.title == "Because You Like Drama"
    }));
    assert!(!viewer_home.personalized_sections.iter().any(|section| {
        section.section_type == "BECAUSE_YOU_LIKE_TAG" && section.title == "Because You Like Horror"
    }));
    assert!(viewer_home.personalized_sections.iter().any(|section| {
        section.section_type == "BECAUSE_YOU_LIKE_TAG" && section.title == "Because You Like Isekai"
    }));

    let filtered = app
        .discovery_items(
            &viewer_actor,
            DiscoveryItemsQuery {
                relation_subtypes: vec!["tmdb.collection".to_string()],
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("viewer discovery items should load");
    assert_eq!(filtered.total_count, 2);
    assert!(
        filtered
            .items
            .iter()
            .any(|item| item.display_title == "Collection Movie")
    );
    assert!(
        filtered
            .items
            .iter()
            .any(|item| item.display_title == "Collection Signal Movie")
    );

    let matched_context = app
        .discovery_items(
            &viewer_actor,
            DiscoveryItemsQuery {
                query: Some("Private".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("viewer matched context should load");
    assert_eq!(matched_context.total_count, 1);
    assert_eq!(matched_context.items[0].matched_subject_count, 1);
    assert_eq!(
        matched_context.items[0].matched_subject_titles,
        vec!["Local Example Movie".to_string()]
    );

    let hidden_context = app
        .discovery_items(
            &viewer_actor,
            DiscoveryItemsQuery {
                query: Some("Hidden".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("viewer hidden context should load");
    assert_eq!(hidden_context.total_count, 0);

    let public_items = app
        .discovery_items(
            &public_actor,
            DiscoveryItemsQuery {
                query: Some("Public".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("public discovery items should load");
    assert_eq!(public_items.total_count, 1);
    assert_eq!(public_items.items[0].display_title, "Public Movie");

    let public_catalog = app
        .catalog_discovery(
            &public_actor,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: Vec::new(),
                include_unresolved: false,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("public catalog discovery should load");
    assert!(!public_catalog.can_view_personalized);
    assert_eq!(public_catalog.groups.len(), 1);
    assert_eq!(
        public_catalog.groups[0].kind,
        CatalogDiscoveryGroupKind::PublicTop
    );
    assert_eq!(
        public_catalog.groups[0].items[0].display_title,
        "Public Movie"
    );

    let visible_catalog = app
        .catalog_discovery(
            &viewer_actor,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: vec![visible_movie_library_id.clone()],
                include_unresolved: false,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("visible catalog discovery should load");
    assert!(visible_catalog.can_view_personalized);
    let visible_catalog_titles = visible_catalog
        .groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.display_title.as_str()))
        .collect::<Vec<_>>();
    assert!(visible_catalog_titles.contains(&"Private Recommendation"));
    assert!(!visible_catalog_titles.contains(&"Hidden Recommendation"));

    let hidden_catalog = app
        .catalog_discovery(
            &viewer_actor,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: vec![hidden_movie_library_id.clone()],
                include_unresolved: false,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("hidden catalog discovery should load public data");
    assert!(!hidden_catalog.can_view_personalized);
    assert_eq!(hidden_catalog.groups.len(), 1);
    assert_eq!(
        hidden_catalog.groups[0].kind,
        CatalogDiscoveryGroupKind::PublicTop
    );
    assert!(hidden_catalog.groups.iter().all(|group| {
        group
            .items
            .iter()
            .all(|item| item.display_title != "Hidden Recommendation")
    }));
    assert_eq!(
        *discovery.generation_list_calls.lock().await,
        0,
        "normal discovery home/items reads should not hydrate full generations"
    );
}

#[tokio::test]
async fn discovery_filters_every_read_path_by_request_or_manage_facet() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(1_500, 0).unwrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .sections
        .lock()
        .await
        .push(discovery_section_record(
            "public-run",
            "mixed_public",
            "TRENDING_NOW",
            "public",
        ));
    let mut movie_item = discovery_item_record(
        "public-run",
        "public-run",
        Some("mixed_public"),
        "tmdb:movie:100",
        "Visible Movie",
        "movie",
        100.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    movie_item.background_url =
        Some("https://example.invalid/visible-movie-backdrop.jpg".to_string());
    let series_item = discovery_item_record(
        "public-run",
        "public-run",
        Some("mixed_public"),
        "tvdb:series:200",
        "Visible Series",
        "series",
        90.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    let anime_item = discovery_item_record(
        "public-run",
        "public-run",
        Some("mixed_public"),
        "mal:anime:300",
        "Hidden Anime",
        "anime",
        80.0,
        &["Fantasy"],
        &[],
        false,
        true,
    );
    let mut unknown_item = discovery_item_record(
        "public-run",
        "public-run",
        Some("mixed_public"),
        "tmdb:movie:400",
        "Hidden Unknown",
        "movie",
        70.0,
        &["Documentary"],
        &[],
        false,
        true,
    );
    unknown_item.content_type = Some("documentary".to_string());
    discovery
        .items
        .lock()
        .await
        .extend([movie_item, series_item, anime_item, unknown_item]);

    let request_permissions = [scryer_domain::LibraryPermission::Request];
    let requester = library_permission_user_with_grants(
        "movie-series-requester",
        &[
            (movie_library_id.as_str(), request_permissions.as_slice()),
            (series_library_id.as_str(), request_permissions.as_slice()),
        ],
    );
    let home = app
        .discovery_home(
            &requester,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 10,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("request-scoped discovery home should load");
    let home_titles = home
        .public_sections
        .iter()
        .flat_map(|section| section.items.iter().map(|item| item.display_title.as_str()))
        .collect::<HashSet<_>>();
    assert_eq!(
        home_titles,
        HashSet::from(["Visible Movie", "Visible Series"])
    );
    assert!(!home.can_view_personalized);
    assert!(home.personalized_sections.is_empty());
    assert!(home.complete_collection.is_none());
    assert!(home.facets.is_empty());
    assert!(home.hero_item.as_ref().is_some_and(|item| {
        matches!(
            recording_discovery_item_media_kind(item).as_deref(),
            Some("movie" | "series")
        )
    }));

    let first_page = app
        .discovery_items(
            &requester,
            DiscoveryItemsQuery {
                include_public: true,
                include_unresolved: true,
                limit: 1,
                offset: 0,
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("request-scoped discovery items should load");
    assert_eq!(first_page.total_count, 2);
    assert_eq!(first_page.items.len(), 1);
    let second_page = app
        .discovery_items(
            &requester,
            DiscoveryItemsQuery {
                include_public: true,
                include_unresolved: true,
                limit: 1,
                offset: 1,
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("second request-scoped discovery page should load");
    assert_eq!(second_page.total_count, 2);
    assert_eq!(second_page.items.len(), 1);

    let movie_detail = app
        .discovery_item_detail(
            &requester,
            DiscoveryItemDetailQuery {
                target_key: "tmdb:movie:100".to_string(),
                include_unresolved: true,
            },
        )
        .await
        .expect("visible movie detail should load");
    assert!(movie_detail.is_some());
    for target_key in ["mal:anime:300", "tmdb:movie:400"] {
        let hidden_detail = app
            .discovery_item_detail(
                &requester,
                DiscoveryItemDetailQuery {
                    target_key: target_key.to_string(),
                    include_unresolved: true,
                },
            )
            .await
            .expect("hidden detail query should succeed without returning an item");
        assert!(hidden_detail.is_none());
    }

    let movie_catalog = app
        .catalog_discovery(
            &requester,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 10,
                max_groups: 3,
            },
        )
        .await
        .expect("requestable movie catalog discovery should load");
    assert!(!movie_catalog.groups.is_empty());
    assert!(!movie_catalog.can_view_personalized);
    let anime_catalog = app
        .catalog_discovery(
            &requester,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Anime,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 10,
                max_groups: 3,
            },
        )
        .await
        .expect("unauthorized anime catalog discovery should return an empty result");
    assert!(anime_catalog.groups.is_empty());
    assert!(!anime_catalog.can_view_personalized);

    let view_only = library_permission_user(
        "anime-viewer",
        &anime_library_id,
        &[scryer_domain::LibraryPermission::View],
    );
    let view_only_home = app
        .discovery_home(&view_only, DiscoveryHomeQuery::default())
        .await
        .expect("view-only discovery home should load");
    assert!(view_only_home.public_sections.is_empty());
    assert!(view_only_home.hero_item.is_none());

    let anime_manager = library_permission_user(
        "anime-manager",
        &anime_library_id,
        &[scryer_domain::LibraryPermission::ManageTitles],
    );
    let manager_home = app
        .discovery_home(&anime_manager, DiscoveryHomeQuery::default())
        .await
        .expect("manager discovery home should load");
    let manager_titles = manager_home
        .public_sections
        .iter()
        .flat_map(|section| section.items.iter().map(|item| item.display_title.as_str()))
        .collect::<HashSet<_>>();
    assert_eq!(manager_titles, HashSet::from(["Hidden Anime"]));
}

#[tokio::test]
async fn discovery_home_hydrates_only_selected_cards_from_large_candidate_set() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let (_viewer, viewer_actor) = create_authenticated_user(
        &app,
        &admin,
        "discovery-selected-hydration",
        "password",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::MediaRequest,
        ],
    )
    .await;
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut library_title = test_title(
        "selected-hydration-library-title",
        "Selected Hydration Library Title",
        MediaFacet::Movie,
        vec![("tmdb_movie", "9000")],
    );
    library_title.library_id = movie_library_id.clone();
    titles.store.lock().await.push(library_title);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("selected-hydration-run".to_string()),
        updated_at: Utc.timestamp_opt(3_000, 0).unwrap(),
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .submitted_subjects
        .lock()
        .await
        .push(DiscoverySubmittedSubjectRecord {
            run_id: "selected-hydration-run".to_string(),
            subject_key: "tmdb:movie:9000".to_string(),
            title_id: Some("selected-hydration-library-title".to_string()),
            library_id: Some(movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Selected Hydration Library Title".to_string()),
            external_ids_json: "[]".to_string(),
            raw_subject_json: "{}".to_string(),
        });
    let mut candidates = Vec::new();
    for index in 0..128 {
        let mut item = discovery_item_record(
            "selected-hydration-run",
            "selected-hydration-run",
            None,
            &format!("tmdb:movie:{}", 10_000 + index),
            &format!("Candidate {index}"),
            "movie",
            1_000.0 - index as f64,
            &["Drama"],
            &[],
            false,
            true,
        );
        item.background_url = Some(format!(
            "https://example.invalid/candidate-{index}-backdrop.jpg"
        ));
        item.matched_subject_keys = vec!["tmdb:movie:9000".to_string()];
        item.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:9000".to_string(),
            title_id: Some("selected-hydration-library-title".to_string()),
            library_id: Some(movie_library_id.clone()),
        }];
        item.context_terms = (0..64)
            .map(|term_index| format!("term-{index}-{term_index}"))
            .collect();
        candidates.push(item);
    }
    discovery.items.lock().await.extend(candidates);

    let home = app
        .discovery_home(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: false,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 1,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("large discovery home should load");
    let returned_ids = home
        .public_sections
        .iter()
        .chain(home.personalized_sections.iter())
        .flat_map(|section| section.items.iter())
        .chain(
            home.complete_collection
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .chain(home.hero_item.iter())
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    assert!(!returned_ids.is_empty());
    assert!(returned_ids.len() < 128);
    let hydration_batches = discovery.hydrated_home_candidate_ids.lock().await;
    assert_eq!(hydration_batches.len(), 1);
    assert_eq!(
        hydration_batches[0].iter().cloned().collect::<HashSet<_>>(),
        returned_ids
    );
    assert!(
        home.personalized_sections
            .iter()
            .flat_map(|section| section.items.iter())
            .all(|item| item.context_terms.len() == 64)
    );

    drop(hydration_batches);
    discovery.hydrated_home_candidate_ids.lock().await.clear();
    *discovery.personalized_facet_calls.lock().await = 0;
    let card_home = app
        .discovery_home_cards(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: false,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 1,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("discovery home cards should load");
    let card_returned_ids = card_home
        .public_sections
        .iter()
        .chain(card_home.personalized_sections.iter())
        .flat_map(|section| section.items.iter())
        .chain(
            card_home
                .complete_collection
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .chain(card_home.hero_item.iter())
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(card_returned_ids, returned_ids);
    let card_hero = card_home
        .hero_item
        .as_ref()
        .expect("card-only home should retain its selected hero");
    assert_eq!(card_hero.matched_subject_count, 1);
    assert_eq!(
        *discovery.hydrated_home_hero_ids.lock().await,
        vec![card_hero.id.clone()],
        "card-only discovery home must hydrate presentation metadata for only the hero"
    );
    assert!(
        discovery
            .hydrated_home_candidate_ids
            .lock()
            .await
            .is_empty(),
        "card-only discovery home must not hydrate selected item details"
    );
    assert_eq!(
        *discovery.personalized_facet_calls.lock().await,
        0,
        "card-only discovery home must not load unused personalized facets"
    );
}

#[tokio::test]
async fn discovery_home_cards_personalized_hero_keeps_presentation_hydration() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let (_viewer, viewer_actor) = create_authenticated_user(
        &app,
        &admin,
        "hero-presentation-viewer",
        "password",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::MediaRequest,
        ],
    )
    .await;
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut library_title = test_title(
        "hero-presentation-library-title",
        "Hero Presentation Library Title",
        MediaFacet::Movie,
        vec![("tmdb_movie", "9100")],
    );
    library_title.library_id = movie_library_id.clone();
    titles.store.lock().await.push(library_title);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("hero-presentation-run".to_string()),
        updated_at: Utc.timestamp_opt(3_000, 0).unwrap(),
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .submitted_subjects
        .lock()
        .await
        .push(DiscoverySubmittedSubjectRecord {
            run_id: "hero-presentation-run".to_string(),
            subject_key: "tmdb:movie:9100".to_string(),
            title_id: Some("hero-presentation-library-title".to_string()),
            library_id: Some(movie_library_id.clone()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Hero Presentation Library Title".to_string()),
            external_ids_json: "[]".to_string(),
            raw_subject_json: "{}".to_string(),
        });
    // Personalized hero candidate as production serves it: the lean home
    // projection leaves overview empty (NULL AS overview) — only the dedicated
    // hero hydration reads presentation columns from the title store.
    let mut item = discovery_item_record(
        "hero-presentation-run",
        "hero-presentation-run",
        None,
        "tmdb:movie:9101",
        "Hero Presentation Recommendation",
        "movie",
        900.0,
        &["Animation"],
        &[],
        false,
        true,
    );
    item.background_url =
        Some("https://example.invalid/hero-presentation-backdrop.jpg".to_string());
    item.matched_subject_keys = vec!["tmdb:movie:9100".to_string()];
    item.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
        subject_key: "tmdb:movie:9100".to_string(),
        title_id: Some("hero-presentation-library-title".to_string()),
        library_id: Some(movie_library_id.clone()),
    }];
    let hero_item_id = item.id.clone();
    discovery.items.lock().await.push(item);
    discovery.home_hero_presentation.lock().await.insert(
        hero_item_id.clone(),
        (
            Some("https://example.invalid/hero-presentation-backdrop.jpg".to_string()),
            Some("A hero-grade synopsis.".to_string()),
        ),
    );

    let card_home = app
        .discovery_home_cards(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: false,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 5,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("discovery home cards should load");

    let hero = card_home
        .hero_item
        .as_ref()
        .expect("personalized hero should be selected");
    assert_eq!(hero.id, hero_item_id);
    assert_eq!(
        hero.overview.as_deref(),
        Some("A hero-grade synopsis."),
        "subject resolution must not clobber the hero's presentation hydration"
    );
    assert_eq!(
        hero.background_url.as_deref(),
        Some("https://example.invalid/hero-presentation-backdrop.jpg")
    );
    assert_eq!(hero.matched_subject_count, 1);
    assert_eq!(
        hero.matched_subject_titles,
        vec!["Hero Presentation Library Title".to_string()]
    );
}

#[tokio::test]
async fn discovery_provenance_keeps_duplicate_subject_keys_across_libraries() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_000, 0).unwrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    let mut movie_copy = test_title(
        "shared-movie-title",
        "Movie Library Copy",
        MediaFacet::Movie,
        vec![("tmdb_movie", "777")],
    );
    movie_copy.library_id = movie_library_id.clone();
    let mut series_copy = test_title(
        "shared-series-library-title",
        "Series Library Copy",
        MediaFacet::Movie,
        vec![("tmdb_movie", "777")],
    );
    series_copy.library_id = series_library_id.clone();
    titles
        .store
        .lock()
        .await
        .extend([movie_copy.clone(), series_copy.clone()]);

    let library_context = crate::discovery::build_discovery_library_context(
        &[movie_copy, series_copy],
        crate::discovery::DiscoveryContextDefaults::default(),
    );
    let subject_provenance = library_context.subject_provenance_by_key();
    assert_eq!(
        subject_provenance
            .get("tmdb:movie:777")
            .expect("shared subject provenance should exist")
            .len(),
        2
    );
    let recommendation = DiscoveryTitle {
        target_key: "tmdb:movie:778".to_string(),
        target_kind: "movie".to_string(),
        content_type: "movie".to_string(),
        resolved: true,
        display_title: "Shared Recommendation".to_string(),
        matched_subject_keys: vec!["tmdb:movie:777".to_string()],
        ..DiscoveryTitle::default()
    };
    let item = crate::discovery::snapshot_item_records(
        "context-run",
        "context-run",
        &[recommendation],
        &subject_provenance,
        observed_at,
    )
    .expect("snapshot item should build")
    .into_iter()
    .next()
    .expect("snapshot item should exist");
    assert_eq!(item.library_provenance.len(), 2);

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.submitted_subjects.lock().await.extend(
        library_context
            .submitted_subject_records("context-run")
            .unwrap(),
    );
    discovery.items.lock().await.push(item);

    let movie_actor = library_permission_user(
        "movie-library-viewer",
        &movie_library_id,
        &[
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::Request,
        ],
    );
    let series_discovery_permissions = [
        scryer_domain::LibraryPermission::View,
        scryer_domain::LibraryPermission::Request,
    ];
    let movie_request_permissions = [scryer_domain::LibraryPermission::Request];
    let series_actor = library_permission_user_with_grants(
        "series-library-viewer",
        &[
            (
                series_library_id.as_str(),
                series_discovery_permissions.as_slice(),
            ),
            (
                movie_library_id.as_str(),
                movie_request_permissions.as_slice(),
            ),
        ],
    );

    let movie_items = app
        .discovery_items(
            &movie_actor,
            DiscoveryItemsQuery {
                query: Some("Shared Recommendation".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("movie viewer discovery items should load");
    assert_eq!(movie_items.total_count, 1);
    assert_eq!(movie_items.items[0].matched_subject_count, 1);
    assert_eq!(
        movie_items.items[0].matched_subject_titles,
        vec!["Movie Library Copy".to_string()]
    );

    let series_items = app
        .discovery_items(
            &series_actor,
            DiscoveryItemsQuery {
                query: Some("Shared Recommendation".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("series viewer discovery items should load");
    assert_eq!(series_items.total_count, 1);
    assert_eq!(series_items.items[0].matched_subject_count, 1);
    assert_eq!(
        series_items.items[0].matched_subject_titles,
        vec!["Series Library Copy".to_string()]
    );
}

#[tokio::test]
async fn catalog_discovery_returns_public_groups_without_personalized_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_100, 0).unwrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let viewer = library_permission_user(
        "movie-library-viewer",
        &movie_library_id,
        &[
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::Request,
        ],
    );

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.sections.lock().await.extend([
        discovery_section_record("public-run", "trending_now", "TRENDING_NOW", "public"),
        discovery_section_record("public-run", "popular_movies", "POPULAR_MOVIES", "public"),
    ]);
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tmdb:movie:999999",
            "Public Movie",
            "movie",
            10.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("popular_movies"),
            "tmdb:movie:888888",
            "Popular Public Movie",
            "movie",
            9.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
    ]);

    let result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("catalog discovery should return public data");

    assert!(result.can_view_personalized);
    assert_eq!(result.groups.len(), 2);
    assert_eq!(result.groups[0].kind, CatalogDiscoveryGroupKind::PublicTop);
    assert_eq!(result.groups[0].surface, CatalogDiscoverySurface::Public);
    assert_eq!(result.groups[0].items[0].display_title, "Public Movie");
    assert_eq!(
        result.groups[1].kind,
        CatalogDiscoveryGroupKind::PublicSection
    );
    assert_eq!(result.groups[1].surface, CatalogDiscoverySurface::Public);
    assert_eq!(
        result.groups[1].label_value.as_deref(),
        Some("popular_movies")
    );
    assert_eq!(
        result.groups[1].items[0].display_title,
        "Popular Public Movie"
    );
    assert_eq!(
        *discovery.generation_list_calls.lock().await,
        0,
        "catalog discovery should not hydrate full generations"
    );
}

#[tokio::test]
async fn catalog_discovery_prioritizes_anime_public_rails_before_personalization() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_125, 0).unwrap();
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let viewer = library_permission_user(
        "anime-library-viewer",
        &anime_library_id,
        &[
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::Request,
        ],
    );

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.sections.lock().await.extend([
        discovery_section_record("public-run", "trending_now", "TRENDING_NOW", "public"),
        discovery_section_record("public-run", "popular_series", "POPULAR_SERIES", "public"),
        discovery_section_record("public-run", "anime_this_week", "ANIME_THIS_WEEK", "public"),
        discovery_section_record(
            "public-run",
            "new_on_streaming",
            "NEW_ON_STREAMING",
            "public",
        ),
        discovery_section_record(
            "public-run",
            "most_anticipated_anime",
            "MOST_ANTICIPATED_ANIME",
            "public",
        ),
        discovery_section_record(
            "public-run",
            "popular_right_now",
            "POPULAR_RIGHT_NOW",
            "public",
        ),
    ]);
    let mut personalized = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tvdb:series:700006",
        "Personalized Anime",
        "anime",
        7.5,
        &[],
        &[],
        false,
        true,
    );
    personalized.library_provenance[0].library_id = Some(anime_library_id);
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tvdb:series:700001",
            "Generic Anime Trend",
            "anime",
            10.0,
            &[],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("popular_series"),
            "tvdb:series:700002",
            "Anime From Popular Series",
            "anime",
            9.0,
            &[],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("anime_this_week"),
            "tvdb:series:700003",
            "Weekly Anime Blend",
            "anime",
            8.0,
            &[],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("new_on_streaming"),
            "tvdb:series:700004",
            "New Streaming Anime",
            "anime",
            7.0,
            &[],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("most_anticipated_anime"),
            "tvdb:series:700005",
            "Anticipated Anime",
            "anime",
            6.0,
            &[],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("popular_right_now"),
            "tvdb:series:700007",
            "Popular Anime",
            "anime",
            5.0,
            &[],
            &[],
            false,
            true,
        ),
        personalized,
    ]);

    let result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Anime,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("catalog discovery should return prioritized anime rails");

    assert_eq!(
        result
            .groups
            .iter()
            .map(|group| group.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "public_top_anime",
            "public_section_new_on_streaming",
            "public_section_most_anticipated_anime",
            "fallback",
            "public_section_popular_right_now",
        ]
    );
    assert_eq!(
        result.groups[0].label_value.as_deref(),
        Some("Trending Now")
    );
    assert!(result.groups.iter().all(|group| {
        group.id != "public_section_trending_now" && group.id != "public_section_popular_series"
    }));
}

#[tokio::test]
async fn catalog_discovery_backfills_public_groups_after_personalization() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_150, 0).unwrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let viewer = library_permission_user(
        "movie-personalized-viewer",
        &movie_library_id,
        &[
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::Request,
        ],
    );

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.sections.lock().await.extend([
        discovery_section_record("public-run", "trending_now", "TRENDING_NOW", "public"),
        discovery_section_record("public-run", "popular_movies", "POPULAR_MOVIES", "public"),
    ]);
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tmdb:movie:999999",
            "Public Lead",
            "movie",
            10.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("popular_movies"),
            "tmdb:movie:888888",
            "Public Backfill",
            "movie",
            9.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "context-run",
            "context-run",
            None,
            "tmdb:movie:777777",
            "Personalized Pick",
            "movie",
            8.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
    ]);

    let result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 6,
                max_groups: 3,
            },
        )
        .await
        .expect("catalog discovery should backfill public groups");

    assert_eq!(result.groups.len(), 3);
    assert_eq!(result.groups[0].kind, CatalogDiscoveryGroupKind::PublicTop);
    assert_eq!(
        result.groups[0].label_value.as_deref(),
        Some("trending_now")
    );
    assert_eq!(
        result.groups[1].surface,
        CatalogDiscoverySurface::Personalized
    );
    assert_eq!(result.groups[1].items[0].display_title, "Personalized Pick");
    assert_eq!(
        result.groups[2].kind,
        CatalogDiscoveryGroupKind::PublicSection
    );
    assert_eq!(result.groups[2].items[0].display_title, "Public Backfill");
}

#[tokio::test]
async fn catalog_discovery_excludes_public_rows_owned_by_normalized_external_id() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_200, 0).unwrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let viewer = library_permission_user_with_grants(
        "movie-public-owned-viewer",
        &[
            (
                &movie_library_id,
                &[
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::Request,
                ],
            ),
            (
                &series_library_id,
                &[scryer_domain::LibraryPermission::View],
            ),
        ],
    );
    let mut owned_movie = test_title(
        "owned-matrix",
        "The Meridian",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    owned_movie.library_id = movie_library_id.clone();
    let mut owned_series = test_title(
        "owned-series",
        "Owned Series",
        MediaFacet::Series,
        vec![("tvdb", "900")],
    );
    owned_series.library_id = series_library_id.clone();
    titles
        .store
        .lock()
        .await
        .extend([owned_movie, owned_series]);
    app.services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            "owned-series",
            "Owned Series Movie",
            Some(2026),
            None,
            Some("604"),
        ))
        .await
        .expect("create owned series movie link");

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .sections
        .lock()
        .await
        .push(discovery_section_record(
            "public-run",
            "trending_now",
            "TRENDING_NOW",
            "public",
        ));
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tmdb:movie:603",
            "Owned Public Movie",
            "movie",
            100.0,
            &["Action"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tvdb:movie:604",
            "Owned Series Movie",
            "movie",
            95.0,
            &["Action"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending_now"),
            "tmdb:movie:999999",
            "Fresh Public Movie",
            "movie",
            90.0,
            &["Action"],
            &[],
            false,
            true,
        ),
    ]);

    let result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: Vec::new(),
                include_unresolved: true,
                limit_per_group: 2,
                max_groups: 1,
            },
        )
        .await
        .expect("catalog discovery should exclude owned public rows");

    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].items.len(), 1);
    assert_eq!(
        result.groups[0].items[0].display_title,
        "Fresh Public Movie"
    );

    let home = app
        .discovery_home(
            &viewer,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: false,
                include_unresolved: true,
                limit_per_section: 10,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("discovery home should exclude owned series movies");
    let home_item_titles = home
        .public_sections
        .iter()
        .flat_map(|section| section.items.iter())
        .map(|item| item.display_title.as_str())
        .collect::<Vec<_>>();
    assert!(home_item_titles.contains(&"Fresh Public Movie"));
    assert!(!home_item_titles.contains(&"Owned Public Movie"));
    assert!(!home_item_titles.contains(&"Owned Series Movie"));
}

#[tokio::test]
async fn title_more_like_this_filters_readable_library_titles_and_refills_limit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let viewer = library_permission_user_with_grants(
        "title-more-like-this-viewer",
        &[
            (&movie_library_id, &[scryer_domain::LibraryPermission::View]),
            (
                &series_library_id,
                &[scryer_domain::LibraryPermission::View],
            ),
        ],
    );

    let mut source_title = test_title(
        "source-title",
        "Source Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "100")],
    );
    source_title.library_id = movie_library_id.clone();
    let mut owned_title = test_title(
        "owned-title",
        "Owned Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    owned_title.library_id = movie_library_id.clone();
    let mut owned_series = test_title(
        "owned-series",
        "Owned Series",
        MediaFacet::Series,
        vec![("tvdb", "900")],
    );
    owned_series.library_id = series_library_id;
    titles
        .store
        .lock()
        .await
        .extend([source_title, owned_title, owned_series]);
    app.services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            "owned-series",
            "Owned Series Movie",
            Some(2026),
            None,
            Some("604"),
        ))
        .await
        .expect("create owned series movie link");

    let mut cached_items = vec![
        discovery_item_record(
            "title-more-like-this-run",
            "title-more-like-this-run",
            None,
            "tmdb:movie:603",
            "Owned Recommendation",
            "movie",
            100.0,
            &["Action"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "title-more-like-this-run",
            "title-more-like-this-run",
            None,
            "tvdb:movie:604",
            "Owned Series Movie Recommendation",
            "movie",
            95.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "title-more-like-this-run",
            "title-more-like-this-run",
            None,
            "tmdb:movie:605",
            "Fresh Recommendation One",
            "movie",
            90.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        discovery_item_record(
            "title-more-like-this-run",
            "title-more-like-this-run",
            None,
            "tmdb:movie:606",
            "Fresh Recommendation Two",
            "movie",
            80.0,
            &["Comedy"],
            &[],
            false,
            true,
        ),
    ];
    let fresh_updated_at = Utc::now();
    for item in &mut cached_items {
        item.updated_at = fresh_updated_at;
    }
    discovery
        .title_more_like_this_items
        .lock()
        .await
        .insert("source-title".to_string(), cached_items);

    let items = app
        .title_more_like_this(&viewer, "source-title", 2)
        .await
        .expect("title more-like-this should load");

    assert_eq!(
        items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect::<Vec<_>>(),
        vec!["Fresh Recommendation One", "Fresh Recommendation Two"]
    );
    assert!(items.iter().all(|item| !item.owned_in_input));
    assert!(items.iter().all(|item| item.resolved_title_id.is_none()));
}

#[tokio::test]
async fn title_more_like_this_queues_empty_cache_refresh_off_the_read_path() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let recommendation_gate = Arc::new(Notify::new());
    *gateway.title_recommendation_gate.lock().await = Some(recommendation_gate.clone());
    let mut malformed_recommendation = test_discovery_title();
    malformed_recommendation.target_key = "tvdb:movie:".to_string();
    malformed_recommendation.display_title.clear();
    malformed_recommendation.original_title.clear();
    malformed_recommendation.year = None;
    malformed_recommendation.poster_url.clear();
    malformed_recommendation.background_url.clear();
    malformed_recommendation.overview.clear();
    malformed_recommendation.content_type.clear();
    let mut recommendation = test_discovery_title();
    recommendation.target_key = "tmdb:movie:604".to_string();
    recommendation.display_title = "Fresh Gateway Recommendation".to_string();
    gateway
        .title_recommendation_results
        .lock()
        .await
        .extend([malformed_recommendation, recommendation]);
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let viewer = library_permission_user(
        "title-more-like-this-refresh-viewer",
        &movie_library_id,
        &[scryer_domain::LibraryPermission::View],
    );

    let mut source_title = test_title(
        "source-title",
        "Source Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "100"), ("tvdb", "200")],
    );
    source_title.library_id = movie_library_id.clone();
    titles.store.lock().await.push(source_title);

    let items = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app.title_more_like_this(&viewer, "source-title", 12),
    )
    .await
    .expect("title more-like-this should not await the metadata gateway")
    .expect("title more-like-this should load the empty cache");

    assert!(items.is_empty());
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        gateway.title_recommendation_started.notified(),
    )
    .await
    .expect("queued recommendation refresh should reach the metadata gateway");

    let inputs = gateway.title_recommendation_inputs.lock().await.clone();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].subject.tvdb_id, Some(200));
    assert_eq!(inputs[0].subject.tmdb_id, Some(100));
    assert_eq!(inputs[0].subject.facet.as_deref(), Some("movie"));
    assert!(inputs[0].include_unresolved);

    recommendation_gate.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if discovery
                .title_more_like_this_items
                .lock()
                .await
                .get("source-title")
                .is_some_and(|items| !items.is_empty())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued recommendation refresh should populate the cache");

    let items = app
        .title_more_like_this(&viewer, "source-title", 12)
        .await
        .expect("title more-like-this should load the refreshed cache");

    assert_eq!(
        items
            .iter()
            .map(|item| item.display_title.as_str())
            .collect::<Vec<_>>(),
        vec!["Fresh Gateway Recommendation"]
    );
    assert_eq!(
        *discovery.title_more_like_this_limits.lock().await,
        vec![1, 48, 1, 48]
    );
}

#[tokio::test]
async fn catalog_discovery_scopes_personalized_rows_to_selected_readable_library() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(2_300, 0).unwrap();
    let library_a_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let library_b = app
        .create_library(
            &admin,
            MediaFacet::Movie,
            "Movie Library B".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/MoviesB".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("second movie library should be created");
    let discovery_permissions = [
        scryer_domain::LibraryPermission::View,
        scryer_domain::LibraryPermission::Request,
    ];
    let viewer = library_permission_user_with_grants(
        "movie-multi-library-viewer",
        &[
            (library_a_id.as_str(), discovery_permissions.as_slice()),
            (library_b.id.as_str(), discovery_permissions.as_slice()),
        ],
    );
    let mut title_a = test_title(
        "title-library-a",
        "Library A Title",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    title_a.library_id = library_a_id.clone();
    title_a.canonical_tags = canonical_genre_tags(&["Drama"]);
    let mut title_b = test_title(
        "title-library-b",
        "Library B Title",
        MediaFacet::Movie,
        vec![("tmdb_movie", "604")],
    );
    title_b.library_id = library_b.id.clone();
    title_b.canonical_tags = canonical_genre_tags(&["Drama"]);
    titles.store.lock().await.extend([title_a, title_b]);

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    let mut library_a_recommendation = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:700",
        "Library A Recommendation",
        "movie",
        100.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    library_a_recommendation.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
        subject_key: "tmdb:movie:603".to_string(),
        title_id: Some("title-library-a".to_string()),
        library_id: Some(library_a_id.clone()),
    }];
    library_a_recommendation.matched_subject_keys = vec!["tmdb:movie:603".to_string()];
    library_a_recommendation.matched_subject_count = 1;
    let mut library_b_recommendation = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:800",
        "Library B Recommendation",
        "movie",
        90.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    library_b_recommendation.library_provenance = vec![DiscoveryItemLibraryProvenanceRecord {
        subject_key: "tmdb:movie:604".to_string(),
        title_id: Some("title-library-b".to_string()),
        library_id: Some(library_b.id.clone()),
    }];
    library_b_recommendation.matched_subject_keys = vec!["tmdb:movie:604".to_string()];
    library_b_recommendation.matched_subject_count = 1;
    discovery
        .items
        .lock()
        .await
        .extend([library_a_recommendation, library_b_recommendation]);

    let library_a_result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: vec![library_a_id],
                include_unresolved: true,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("library A catalog discovery should load");
    let library_a_titles = library_a_result
        .groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.display_title.as_str()))
        .collect::<Vec<_>>();
    assert!(library_a_titles.contains(&"Library A Recommendation"));
    assert!(!library_a_titles.contains(&"Library B Recommendation"));

    let library_b_result = app
        .catalog_discovery(
            &viewer,
            CatalogDiscoveryQuery {
                facet: MediaFacet::Movie,
                library_ids: vec![library_b.id],
                include_unresolved: true,
                limit_per_group: 6,
                max_groups: 6,
            },
        )
        .await
        .expect("library B catalog discovery should load");
    let library_b_titles = library_b_result
        .groups
        .iter()
        .flat_map(|group| group.items.iter().map(|item| item.display_title.as_str()))
        .collect::<Vec<_>>();
    assert!(library_b_titles.contains(&"Library B Recommendation"));
    assert!(!library_b_titles.contains(&"Library A Recommendation"));
}

#[tokio::test]
async fn discovery_home_never_uses_live_public_feed_when_snapshots_are_missing() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let (_viewer, viewer_actor) = create_authenticated_user(
        &app,
        &admin,
        "discovery-live-public-viewer",
        "password",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::MediaRequest,
        ],
    )
    .await;

    let result = app
        .discovery_home(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 18,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("discovery home should stay local when snapshots are missing");

    assert!(result.public_sections.is_empty());
    assert!(result.personalized_sections.is_empty());
    assert!(result.complete_collection.is_none());
    assert!(result.status.state.last_success_generation_id.is_none());
    assert!(result.status.state.last_public_feed_generation_id.is_none());
    let card_result = app
        .discovery_home_cards(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: true,
                limit_per_section: 18,
                filters: DiscoveryHomeFilters::default(),
            },
        )
        .await
        .expect("card-only discovery home should stay local when snapshots are missing");
    assert!(card_result.public_sections.is_empty());
    assert!(card_result.personalized_sections.is_empty());
    assert!(card_result.hero_item.is_none());
    assert!(gateway.public_feed_inputs.lock().await.is_empty());
    assert!(discovery.public_feed_commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_initial_snapshot_submits_smg_and_commits_local_generation() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].subjects.len(), 1);
    assert_eq!(submitted_inputs[0].subjects[0].tmdb_id, Some(603));
    assert!(submitted_inputs[0].context_fingerprint.is_some());
    drop(submitted_inputs);

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert_eq!(gateway.ack_requests.lock().await.as_slice(), ["request-1"]);

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(commit.run.status, "complete");
    assert_eq!(commit.run.smg_request_id.as_deref(), Some("request-1"));
    assert_eq!(
        commit.state.last_success_generation_id,
        Some(commit.run.id.clone())
    );
    assert_eq!(
        commit.state.last_subject_fingerprint,
        commit.run.subject_fingerprint
    );
    assert_eq!(commit.submitted_subjects.len(), 1);
    assert_eq!(commit.submitted_subjects[0].subject_key, "tmdb:movie:603");
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].target_key, "tmdb:movie:604");
    assert_eq!(commit.facets.len(), 1);
    assert_eq!(commit.facets[0].facet_name, "genre");
    drop(commits);

    let runs = discovery.runs.lock().await;
    assert!(
        runs.iter().any(|run| run.acknowledged_at.is_some()),
        "ack timestamp should be written back to the run ledger"
    );
}

#[tokio::test]
async fn discovery_sync_uses_configured_metadata_language() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, "jpn")
        .await;
    let (app, _admin, titles) =
        bootstrap_with_metadata_gateway_settings_and_titles(gateway.clone(), settings);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].language, "jpn");
    drop(submitted_inputs);

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.language, "jpn");
}

#[tokio::test]
async fn discovery_sync_uses_configured_region() {
    // Region routes through settings (defaults to "US" when unset).
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, DISCOVERY_REGION_KEY, "CA")
        .await;
    let (app, _admin, titles) =
        bootstrap_with_metadata_gateway_settings_and_titles(gateway.clone(), settings);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].region, "CA");
    drop(submitted_inputs);

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.region, "CA");
}

#[tokio::test]
async fn metadata_language_change_refreshes_public_discovery_feed() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, "eng")
        .await;
    let (app, admin, titles) =
        bootstrap_with_metadata_gateway_settings_and_titles(gateway.clone(), settings);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc::now();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("snapshot-old".to_string()),
        last_public_feed_generation_id: Some("public-old".to_string()),
        next_context_snapshot_eligible_at: Some(now - chrono::Duration::minutes(1)),
        next_incremental_reload_eligible_at: Some(now + chrono::Duration::hours(4)),
        next_public_feed_eligible_at: Some(now + chrono::Duration::hours(24)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });
    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.rehydrate_all_metadata(&admin, "jpn")
        .await
        .expect("language change should be accepted");

    for _ in 0..50 {
        if !gateway.public_feed_inputs.lock().await.is_empty()
            && !gateway.submitted_inputs.lock().await.is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].language, "jpn");
    drop(submitted_inputs);

    let public_feed_inputs = gateway.public_feed_inputs.lock().await;
    assert_eq!(public_feed_inputs.len(), 1);
    assert_eq!(public_feed_inputs[0].language, "jpn");
    drop(public_feed_inputs);

    let runs = discovery.runs.lock().await;
    let public_run = runs
        .iter()
        .find(|run| run.kind == "public_feed")
        .expect("public discovery feed should run");
    assert_eq!(public_run.language, "jpn");
    assert_eq!(
        public_run.trigger_source,
        JobTriggerSource::SystemInternal.as_str()
    );
}

#[tokio::test]
async fn discovery_sync_snapshot_polling_status_resumes_existing_request_and_commits() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    {
        let mut statuses = gateway.snapshot_status_queue.lock().await;
        statuses.push_back(polling_snapshot_status("request-1", "RUNNING"));
        statuses.push_back(complete_snapshot_status("request-1"));
    }
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("first discovery sync should defer while snapshot builds");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());

    {
        let mut state = discovery.state.lock().await;
        let state = state.as_mut().expect("state should persist");
        assert!(state.inflight_context_snapshot_run_id.is_some());
        assert!(state.backoff_until.is_some());
        state.backoff_until = Some(Utc::now() - chrono::Duration::minutes(1));
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("second discovery sync should resume and commit");

    assert_eq!(
        gateway.submitted_inputs.lock().await.len(),
        1,
        "resume must not submit a second snapshot request"
    );
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1", "request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert_eq!(discovery.commits.lock().await.len(), 1);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
}

#[tokio::test]
async fn discovery_sync_snapshot_queue_full_sets_backoff_without_commit_or_pages() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.snapshot_status_override.lock().await = Some(queue_full_snapshot_status("request-1"));
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer on queue full");

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(gateway.ack_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be persisted");
    assert!(state.backoff_until.is_some());
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "deferred");
    assert_eq!(run.smg_status.as_deref(), Some("QUEUE_FULL"));
    assert_eq!(run.item_count, Some(0));
}

#[tokio::test]
async fn discovery_sync_snapshot_terminal_failure_clears_inflight_without_commit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.snapshot_status_override.lock().await = Some(failed_snapshot_status("request-1"));
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-previous".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-previous".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should handle terminal snapshot failure");

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(gateway.ack_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(
        state.last_success_generation_id.as_deref(),
        Some("generation-previous")
    );
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
    assert!(state.backoff_until.is_some());

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "failed");
    assert_eq!(run.smg_status.as_deref(), Some("FAILED"));
}

#[tokio::test]
async fn discovery_sync_snapshot_page_failure_preserves_inflight_for_retry() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_snapshot_page.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer on page fetch failure");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert!(discovery.commits.lock().await.is_empty());
    {
        let mut state = discovery.state.lock().await;
        let state = state.as_mut().expect("state should persist");
        assert!(state.inflight_context_snapshot_run_id.is_some());
        assert!(state.backoff_until.is_some());
        state.backoff_until = Some(Utc::now() - chrono::Duration::minutes(1));
    }
    *gateway.fail_snapshot_page.lock().await = false;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should retry page fetch and commit");

    assert_eq!(
        gateway.submitted_inputs.lock().await.len(),
        1,
        "page retry must reuse the accepted snapshot request"
    );
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1", "request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1), ("request-1".to_string(), 1)]
    );
    assert_eq!(discovery.commits.lock().await.len(), 1);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.inflight_context_snapshot_run_id.is_none());
}

#[tokio::test]
async fn discovery_sync_ack_failure_after_commit_schedules_retry() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_ack.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should commit snapshot even if ack fails");

    assert_eq!(gateway.ack_requests.lock().await.as_slice(), ["request-1"]);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be persisted");
    assert!(
        state.backoff_until.is_some(),
        "ack failure should schedule prompt retry"
    );
    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "warning");
    assert!(run.acknowledged_at.is_none());
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|text| text.contains("ack failed"))
    );
}

#[tokio::test]
async fn discovery_sync_existing_library_upgrade_schedules_first_snapshot_promptly() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    // Every discovery wake time is jittered from the persisted scheduler seed,
    // which the app mints as a fresh UUID on first use. Pin it so the exact
    // times this test asserts are the same on every run instead of a lottery
    // (CI 0.18.15: a random seed put the incremental-reload bucket 37 s after
    // `now`, ahead of the first-snapshot window it should never pre-empt).
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            crate::jobs::jobs::SCHEDULER_INSTANCE_ID_KEY,
            "stable-scheduler-seed",
        )
        .await;
    let (app, _admin, titles) =
        bootstrap_with_metadata_gateway_settings_and_titles(gateway.clone(), settings);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be written");
    assert!(state.bootstrap_started_at.is_none());
    assert!(state.bootstrap_quiet_until.is_none());
    assert!(state.last_success_generation_id.is_none());
    let first_due = state
        .next_context_snapshot_eligible_at
        .expect("first personalized snapshot should be scheduled");
    assert!(first_due >= now + chrono::Duration::minutes(1));
    assert!(first_due <= now + chrono::Duration::minutes(6));

    let next_run_at = app
        .runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::DiscoverySync)
        .await
        .expect("discovery sync should be scheduled promptly");
    assert_eq!(next_run_at, first_due);

    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(first_due));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("due discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(discovery.commits.lock().await.len(), 1);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be written");
    assert!(state.last_success_generation_id.is_some());
}

#[tokio::test]
async fn discovery_sync_empty_new_install_does_not_submit_personalized_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be written");
    assert!(state.last_success_generation_id.is_none());
    assert!(state.next_context_snapshot_eligible_at.is_none());
}

#[tokio::test]
async fn discovery_sync_first_successful_scan_with_titles_accelerates_first_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("empty discovery sync should run");
    assert!(gateway.submitted_inputs.lock().await.is_empty());

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    let completed_at = now + chrono::Duration::seconds(30);
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(completed_at));
    let mut event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanCompleted(test_library_scan_completed_event("scan-1", 1)),
    );
    event.occurred_at = completed_at;
    app.append_domain_event(event)
        .await
        .expect("scan completion event should append");

    let next_run_at = app
        .runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::DiscoverySync)
        .await
        .expect("first successful scan should schedule discovery");
    assert!(next_run_at >= completed_at + chrono::Duration::minutes(1));
    assert!(next_run_at <= completed_at + chrono::Duration::minutes(6));

    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(next_run_at));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("accelerated discovery sync should run");

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].subjects.len(), 1);
    assert_eq!(submitted_inputs[0].subjects[0].tmdb_id, Some(603));
    drop(submitted_inputs);
    assert_eq!(discovery.commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_successful_scan_after_first_snapshot_does_not_accelerate() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        next_context_snapshot_eligible_at: Some(now + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(now + chrono::Duration::hours(4)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    let mut event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanCompleted(test_library_scan_completed_event("scan-1", 1)),
    );
    event.occurred_at = now;
    app.append_domain_event(event)
        .await
        .expect("scan completion event should append");

    assert!(
        app.runtime
            .jobs
            .job_run_tracker
            .next_run_at(JobKey::DiscoverySync)
            .await
            .is_none()
    );
    assert!(gateway.submitted_inputs.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_failed_canceled_or_empty_scan_does_not_accelerate_first_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    let mut empty_event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-empty",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanCompleted(test_library_scan_completed_event(
            "scan-empty",
            0,
        )),
    );
    empty_event.occurred_at = now;
    app.append_domain_event(empty_event)
        .await
        .expect("empty scan completion event should append");

    let mut failed_event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-failed",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanFailed(LibraryScanFailedEventData {
            session_id: "scan-failed".to_string(),
            error_message: "scan failed".to_string(),
        }),
    );
    failed_event.occurred_at = now;
    app.append_domain_event(failed_event)
        .await
        .expect("failed scan event should append");

    let mut canceled_event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-canceled",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanCanceled(LibraryScanCanceledEventData {
            session_id: "scan-canceled".to_string(),
            status: "canceled".to_string(),
            found_titles: 1,
            title_match_completed: 1,
            title_match_total_known: true,
            titles_completed: 1,
            titles_total: Some(1),
            files_completed: 0,
            files_total: Some(0),
            summary: None,
        }),
    );
    canceled_event.occurred_at = now;
    app.append_domain_event(canceled_event)
        .await
        .expect("canceled scan event should append");

    assert!(
        app.runtime
            .jobs
            .job_run_tracker
            .next_run_at(JobKey::DiscoverySync)
            .await
            .is_none()
    );
    assert!(gateway.submitted_inputs.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_initial_snapshot_waits_for_backoff_before_resubmitting() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        backoff_until: Some(Utc::now() + chrono::Duration::hours(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_startup_snapshot_respects_backoff() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        backoff_until: Some(Utc::now() + chrono::Duration::hours(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledStartup)
        .await
        .expect("startup discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_initial_snapshot_waits_for_bootstrap_quiet_window() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    let quiet_until = now + chrono::Duration::minutes(5);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(now - chrono::Duration::minutes(1)),
        bootstrap_quiet_until: Some(quiet_until),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    let next_context_at = state
        .next_context_snapshot_eligible_at
        .expect("first snapshot should remain scheduled");
    assert!(next_context_at >= quiet_until);
    let next_run_at = app
        .runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::DiscoverySync)
        .await
        .expect("discovery sync should stay scheduled");
    assert!(next_run_at >= quiet_until);
}

#[tokio::test]
async fn discovery_sync_manual_trigger_bypasses_bootstrap_quiet_window() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(now - chrono::Duration::minutes(1)),
        bootstrap_quiet_until: Some(now + chrono::Duration::minutes(5)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(discovery.commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_incremental_reload_calls_smg_and_commits_patch() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(
        change_inputs[0].previous_context_fingerprint.as_deref(),
        Some("fingerprint-generation-1")
    );
    assert_eq!(
        change_inputs[0].context_subject_keys,
        vec!["tmdb:movie:603"]
    );
    assert_eq!(change_inputs[0].changed_subjects.len(), 1);
    drop(change_inputs);

    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_incremental");
    assert_eq!(commit.run.status, "complete");
    assert_eq!(
        commit.run.base_generation_id.as_deref(),
        Some("generation-1")
    );
    assert_eq!(commit.tombstone_target_keys, vec!["tmdb:movie:604"]);
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].source_run_kind, "context_incremental");
    assert_eq!(commit.clear_pending_through_sequence, Some(12));
    assert_eq!(commit.state.last_seen_domain_event_sequence, Some(12));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_incremental_queue_full_sets_backoff_without_commit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.context_changes_override.lock().await = Some(queue_full_context_changes_result());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(
        change_inputs[0].previous_context_fingerprint.as_deref(),
        Some("fingerprint-generation-1")
    );
    drop(change_inputs);

    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.backoff_until.is_some());
    assert_eq!(state.dirty_since, Some(due_at));
    assert_eq!(state.dirty_reason_mask, 1);

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_incremental")
        .expect("incremental run should be recorded");
    assert_eq!(run.status, "deferred");
    assert_eq!(run.smg_status.as_deref(), Some("QUEUE_FULL"));
    assert_eq!(run.item_count, Some(0));
}

#[tokio::test]
async fn discovery_sync_incremental_transport_failure_sets_backoff_and_keeps_pending_dirty() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_context_changes.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer transport failure");

    assert_eq!(gateway.change_inputs.lock().await.len(), 1);
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.backoff_until.is_some());
    assert_eq!(state.dirty_since, Some(due_at));
    assert_eq!(state.dirty_reason_mask, 1);

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_incremental")
        .expect("incremental run should be recorded");
    assert_eq!(run.status, "deferred");
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|error| error.contains("forced incremental failure"))
    );
}

#[tokio::test]
async fn discovery_sync_too_many_incremental_changes_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(
            crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS as i64 + 1,
        ),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..=crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS {
        let mut change = discovery_pending_change_record(
            &format!("change-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.first_seen_sequence = Some(index as i64 + 1);
        change.last_seen_sequence = Some(index as i64 + 1);
        change.first_seen_at = due_at;
        change.last_seen_at = due_at;
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(
        commit.clear_pending_through_sequence,
        Some(crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS as i64 + 1)
    );
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_rematch_without_previous_subject_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(20),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "rematched".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(20),
            last_seen_sequence: Some(20),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.kind, "context_snapshot");
    assert_eq!(commits[0].clear_pending_through_sequence, Some(20));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_rematch_resolved_key_limit_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    let change_count = crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS / 2 + 1;
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(change_count as i64),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..change_count {
        let current_tmdb_id = 10_000 + index as i64;
        let previous_tmdb_id = 20_000 + index as i64;
        let mut change = discovery_pending_change_record(
            &format!("change-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.subject_key = Some(format!("tmdb:movie:{current_tmdb_id}"));
        change.previous_subject_key = Some(format!("tmdb:movie:{previous_tmdb_id}"));
        change.change_type = "rematched".to_string();
        change.raw_subject_json = Some(
            serde_json::json!({
                "tmdbId": current_tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": current_tmdb_id.to_string()}]
            })
            .to_string(),
        );
        change.raw_previous_subject_json = Some(
            serde_json::json!({
                "tmdbId": previous_tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": previous_tmdb_id.to_string()}]
            })
            .to_string(),
        );
        change.first_seen_sequence = Some(index as i64 + 1);
        change.last_seen_sequence = Some(index as i64 + 1);
        change.first_seen_at = due_at;
        change.last_seen_at = due_at;
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending rematch should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(
        change_count < crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS,
        "test must stay below the row-count guard"
    );
    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.kind, "context_snapshot");
    assert_eq!(
        commits[0].clear_pending_through_sequence,
        Some(change_count as i64)
    );
}

#[tokio::test]
async fn discovery_sync_daily_snapshot_takes_precedence_and_clears_pending_changes() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(due_at + chrono::Duration::days(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(commit.clear_pending_through_sequence, Some(12));
    assert_eq!(
        commit.state.last_success_generation_id,
        Some(commit.run.id.clone())
    );
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_public_feed_runs_while_scan_and_context_backoff_are_active_and_filters_collection_section()
 {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    let context_backoff_until = Utc::now() + chrono::Duration::hours(6);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        next_public_feed_eligible_at: Some(due_at),
        backoff_until: Some(context_backoff_until),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(test_active_library_scan_run(due_at))
        .await;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    let public_feed_inputs = gateway.public_feed_inputs.lock().await;
    assert_eq!(public_feed_inputs.len(), 1);
    assert!(
        public_feed_inputs[0].section_types.is_empty(),
        "public feed input should let SMG choose its public default sections"
    );
    drop(public_feed_inputs);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.public_feed_commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "public_feed");
    assert_eq!(commit.sections.len(), 1);
    assert_eq!(commit.sections[0].section_type, "TRENDING_NOW");
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].source_run_kind, "public_feed");
    assert!(commit.items[0].matched_subject_keys.is_empty());
    assert!(commit.items[0].matched_subject_titles.is_empty());
    assert_eq!(commit.items[0].matched_subject_count, 0);
    assert_eq!(
        commit.state.last_public_feed_generation_id,
        Some(commit.run.id.clone())
    );
}

#[tokio::test]
async fn discovery_sync_manual_run_forces_public_feed_when_fresh() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(future_at),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_startup_refreshes_public_feed_immediately_without_personalized_work() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(future_at),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledStartup)
        .await
        .expect("startup discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_manual_public_feed_runs_while_context_backoff_is_active() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(future_at),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        backoff_until: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_defers_smg_work_while_library_scan_is_active() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(due_at + chrono::Duration::days(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(test_active_library_scan_run(due_at))
        .await;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_defers_smg_work_for_projected_active_scan() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let domain_events = Arc::new(super::MockDomainEventRepo::default());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_discovery_store(discovery.clone())
            .with_domain_events(domain_events.clone())
    });
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");
    let mut job_event = crate::domain_events::new_job_run_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "scan-1".to_string(),
            job_key: JobKey::LibraryScanMovies.as_str().to_string(),
            operation_type: JobKey::LibraryScanMovies.as_str().to_string(),
            trigger_source: JobTriggerSource::Manual.as_str().to_string(),
        }),
    );
    job_event.occurred_at = due_at;
    domain_events
        .append(job_event)
        .await
        .expect("scan job start event should append");
    let mut event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
            session_id: "scan-1".to_string(),
            library_id: Some("library".to_string()),
            mode: "full".to_string(),
        }),
    );
    event.occurred_at = due_at;
    domain_events
        .append(event)
        .await
        .expect("scan start event should append");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_catches_up_title_events_before_incremental_reload() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let domain_events = Arc::new(super::MockDomainEventRepo::default());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_discovery_store(discovery.clone())
            .with_domain_events(domain_events.clone())
    });
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        last_seen_domain_event_sequence: Some(0),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    let title = test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    titles.store.lock().await.push(title.clone());

    let mut event = crate::domain_events::new_title_domain_event(
        crate::domain_events::DomainEventActor::system(),
        &title,
        DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
            title: test_title_context_snapshot(&title),
        }),
    );
    event.occurred_at = due_at;
    domain_events
        .append(event)
        .await
        .expect("domain event should append");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(change_inputs[0].changed_subjects.len(), 1);
    let changed_subject = &change_inputs[0].changed_subjects[0];
    assert_eq!(changed_subject.subject.tmdb_id, Some(603));
    assert_eq!(
        changed_subject.change_type,
        DiscoveryContextChangeType::Updated
    );
    drop(change_inputs);

    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(1));
    assert_eq!(commits[0].state.last_seen_domain_event_sequence, Some(1));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_incremental_success_preserves_dirty_when_newer_sequence_seen() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now),
        next_context_snapshot_eligible_at: Some(now + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(20),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    change.first_seen_at = now - chrono::Duration::hours(1);
    change.last_seen_at = now - chrono::Duration::hours(1);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.change_inputs.lock().await.len(), 1);
    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(12));
    assert_eq!(commits[0].state.dirty_since, Some(now));
    assert_eq!(commits[0].state.dirty_reason_mask, 1);
    assert_eq!(commits[0].state.last_seen_domain_event_sequence, Some(20));
}

#[tokio::test]
async fn discovery_sync_snapshot_dirty_clear_requires_inflight_fingerprint_match() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-previous".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-previous".to_string()),
        inflight_context_snapshot_run_id: Some("run-inflight".to_string()),
        inflight_subject_fingerprint: Some("fingerprint-stale".to_string()),
        inflight_domain_event_sequence: Some(10),
        dirty_since: Some(now),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(10),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });
    let mut run = discovery_run_record("run-inflight", now, "deferred");
    run.smg_request_id = Some("request-1".to_string());
    run.subject_fingerprint = Some("fingerprint-stale".to_string());
    run.completed_at = None;
    discovery.runs.lock().await.push(run);

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should resume and commit");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(10));
    assert_eq!(commits[0].state.dirty_since, Some(now));
    assert_eq!(commits[0].state.dirty_reason_mask, 1);
    assert!(commits[0].state.inflight_context_snapshot_run_id.is_none());
}

#[tokio::test]
async fn discovery_sync_unchanged_fingerprint_clears_pending_without_smg() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    let fingerprint = crate::discovery::build_discovery_library_context(
        &[],
        crate::discovery::DiscoveryContextDefaults::default(),
    )
    .fingerprint;
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some(fingerprint),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(now),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    change.first_seen_at = now - chrono::Duration::hours(1);
    change.last_seen_at = now - chrono::Duration::hours(1);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should clean unchanged dirty state");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert!(discovery.pending_changes.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.dirty_since.is_none());
    assert_eq!(state.dirty_reason_mask, 0);
    assert_eq!(state.last_seen_domain_event_sequence, Some(12));
}

#[tokio::test]
async fn discovery_sync_reads_more_than_1000_pending_rows_for_incremental_eligibility() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(now + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(1_001),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..1_001 {
        let mut change = discovery_pending_change_record(
            &format!("change-shared-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.subject_key = Some("tmdb:movie:603".to_string());
        change.raw_subject_json = Some(
            serde_json::json!({
                "tmdbId": 603,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": "603"}]
            })
            .to_string(),
        );
        change.first_seen_sequence = Some(index + 1);
        change.last_seen_sequence = Some(index + 1);
        change.first_seen_at = now - chrono::Duration::hours(1);
        change.last_seen_at = now - chrono::Duration::hours(1);
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(change_inputs[0].changed_subjects.len(), 1_001);
    drop(change_inputs);
    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(1_001));
}

#[tokio::test]
async fn discovery_sync_transient_backoff_escalates_and_resets_after_success() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_context_changes.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let t0 = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(t0));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(t0 - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(t0 + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(t0),
        dirty_since: Some(t0 - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(t0 + chrono::Duration::days(1)),
        updated_at: t0,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("first transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 1);
    assert_eq!(
        state.backoff_until,
        Some(t0 + chrono::Duration::minutes(15))
    );

    let t1 = t0 + chrono::Duration::minutes(31);
    app.runtime.environment.set_fixed_now_for_tests(Some(t1));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("second transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 2);
    assert_eq!(state.backoff_until, Some(t1 + chrono::Duration::hours(1)));

    let t2 = t1 + chrono::Duration::minutes(61);
    app.runtime.environment.set_fixed_now_for_tests(Some(t2));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("third transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 3);
    assert_eq!(state.backoff_until, Some(t2 + chrono::Duration::hours(6)));

    let t3 = t2 + chrono::Duration::hours(6) + chrono::Duration::minutes(1);
    app.runtime.environment.set_fixed_now_for_tests(Some(t3));
    *gateway.fail_context_changes.lock().await = false;
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("successful incremental should reset transient failure count");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 0);
    assert!(state.backoff_until.is_none());
    assert_eq!(discovery.incremental_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_manual_context_cooldown_defers_context_but_allows_public_feed() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::minutes(5)),
        next_context_snapshot_eligible_at: Some(now),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::minutes(5)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&discovery_pending_change_record(
            "change-1",
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        ))
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_manual_context_cooldown_allows_first_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(now - chrono::Duration::minutes(20)),
        bootstrap_quiet_until: Some(now - chrono::Duration::minutes(1)),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should submit first snapshot");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(discovery.commits.lock().await.len(), 1);
}

#[derive(Default)]
struct SnapshotMetadataGateway {
    submitted_inputs: Mutex<Vec<DiscoveryContextSnapshotSubmitInput>>,
    change_inputs: Mutex<Vec<DiscoveryContextChangesInput>>,
    public_feed_inputs: Mutex<Vec<DiscoveryPublicFeedInput>>,
    title_recommendation_inputs: Mutex<Vec<TitleRecommendationsInput>>,
    title_recommendation_results: Mutex<Vec<DiscoveryTitle>>,
    title_recommendation_gate: Mutex<Option<Arc<Notify>>>,
    title_recommendation_started: Notify,
    status_requests: Mutex<Vec<String>>,
    page_requests: Mutex<Vec<(String, i32)>>,
    ack_requests: Mutex<Vec<String>>,
    fail_ack: Mutex<bool>,
    snapshot_status_override: Mutex<Option<DiscoveryContextSnapshotStatusResult>>,
    snapshot_status_queue: Mutex<VecDeque<DiscoveryContextSnapshotStatusResult>>,
    fail_snapshot_page: Mutex<bool>,
    context_changes_override: Mutex<Option<DiscoveryContextChangesResult>>,
    fail_context_changes: Mutex<bool>,
}

#[async_trait]
impl MetadataGateway for SnapshotMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(unused_gateway_call())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(unused_gateway_call())
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(unused_gateway_call())
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Err(unused_gateway_call())
    }

    async fn title_recommendations(
        &self,
        input: &TitleRecommendationsInput,
    ) -> AppResult<DiscoveryRelatedResult> {
        self.title_recommendation_inputs
            .lock()
            .await
            .push(input.clone());
        if let Some(gate) = self.title_recommendation_gate.lock().await.clone() {
            self.title_recommendation_started.notify_one();
            gate.notified().await;
        }
        Ok(DiscoveryRelatedResult {
            subject_key: input.subject.key.clone().unwrap_or_default(),
            query: input.query.clone(),
            generated_at: "2026-06-25T00:00:04Z".to_string(),
            results: self.title_recommendation_results.lock().await.clone(),
        })
    }

    async fn discover_public_feed(
        &self,
        input: &DiscoveryPublicFeedInput,
    ) -> AppResult<DiscoveryDashboardResult> {
        self.public_feed_inputs.lock().await.push(input.clone());
        Ok(DiscoveryDashboardResult {
            subject_keys: Vec::new(),
            generated_at: "2026-06-25T00:00:04Z".to_string(),
            sections: vec![
                DiscoveryDashboardSection {
                    section_id: "trending_now".to_string(),
                    section_type: "TRENDING_NOW".to_string(),
                    title: "Trending Now".to_string(),
                    source_signals: vec!["popular".to_string()],
                    facets: Vec::new(),
                    items: vec![test_discovery_title()],
                },
                DiscoveryDashboardSection {
                    section_id: "collection".to_string(),
                    section_type: "COMPLETE_THE_COLLECTION".to_string(),
                    title: "Complete the Collection".to_string(),
                    source_signals: Vec::new(),
                    facets: Vec::new(),
                    items: vec![test_discovery_title()],
                },
            ],
        })
    }

    async fn submit_discovery_context_snapshot(
        &self,
        input: &DiscoveryContextSnapshotSubmitInput,
    ) -> AppResult<DiscoveryContextSnapshotSubmitResult> {
        self.submitted_inputs.lock().await.push(input.clone());
        Ok(DiscoveryContextSnapshotSubmitResult {
            request_id: Some("request-1".to_string()),
            status: "ACCEPTED".to_string(),
            subject_count: input.subjects.len() as i32,
            retry_after_seconds: 1,
            expires_at: "2026-06-25T00:00:00Z".to_string(),
        })
    }

    async fn discovery_context_snapshot_status(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotStatusResult> {
        self.status_requests
            .lock()
            .await
            .push(request_id.to_string());
        if let Some(result) = self.snapshot_status_queue.lock().await.pop_front() {
            return Ok(result);
        }
        if let Some(result) = self.snapshot_status_override.lock().await.clone() {
            return Ok(result);
        }
        Ok(DiscoveryContextSnapshotStatusResult {
            request_id: request_id.to_string(),
            status: "COMPLETE".to_string(),
            phase: "complete".to_string(),
            subject_count: 1,
            item_count: 1,
            page_count: 1,
            facet_count: 1,
            lazy_hydration_queued_count: 0,
            lazy_hydration_sources: Vec::new(),
            discovery_index_watermark: "watermark-1".to_string(),
            retry_after_seconds: 1,
            created_at: "2026-06-25T00:00:00Z".to_string(),
            started_at: "2026-06-25T00:00:00Z".to_string(),
            completed_at: "2026-06-25T00:00:01Z".to_string(),
            expires_at: "2026-06-26T00:00:00Z".to_string(),
            last_error: String::new(),
        })
    }

    async fn discovery_context_snapshot_page(
        &self,
        request_id: &str,
        page: i32,
    ) -> AppResult<DiscoveryContextSnapshotPageResult> {
        self.page_requests
            .lock()
            .await
            .push((request_id.to_string(), page));
        if *self.fail_snapshot_page.lock().await {
            return Err(AppError::Repository("forced page failure".to_string()));
        }
        Ok(DiscoveryContextSnapshotPageResult {
            request_id: request_id.to_string(),
            page,
            page_count: 1,
            generated_at: "2026-06-25T00:00:01Z".to_string(),
            discovery_index_watermark: "watermark-1".to_string(),
            facets: vec![DiscoverySnapshotFacetGroup {
                name: "genre".to_string(),
                values: vec![DiscoverySnapshotFacetValue {
                    value: "sci-fi".to_string(),
                    count: 1,
                }],
            }],
            items: vec![test_discovery_title()],
        })
    }

    async fn discovery_context_changes(
        &self,
        input: &DiscoveryContextChangesInput,
    ) -> AppResult<DiscoveryContextChangesResult> {
        self.change_inputs.lock().await.push(input.clone());
        if *self.fail_context_changes.lock().await {
            return Err(AppError::Repository(
                "forced incremental failure".to_string(),
            ));
        }
        if let Some(result) = self.context_changes_override.lock().await.clone() {
            return Ok(result);
        }
        Ok(DiscoveryContextChangesResult {
            status: "COMPLETE".to_string(),
            retry_after_seconds: 1,
            generated_at: "2026-06-25T00:00:03Z".to_string(),
            context_fingerprint: input.context_fingerprint.clone().unwrap_or_default(),
            previous_context_fingerprint: input
                .previous_context_fingerprint
                .clone()
                .unwrap_or_default(),
            discovery_index_watermark: "watermark-incremental".to_string(),
            context_subject_count: input.context_subject_keys.len() as i32,
            changed_subject_count: input.changed_subjects.len() as i32,
            resolved_changed_subject_keys: vec!["tmdb:movie:603".to_string()],
            removed_subject_keys: Vec::new(),
            affected_target_keys: vec!["tmdb:movie:604".to_string()],
            items: vec![test_discovery_title()],
        })
    }

    async fn acknowledge_discovery_context_snapshot(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotAckResult> {
        self.ack_requests.lock().await.push(request_id.to_string());
        if *self.fail_ack.lock().await {
            return Err(AppError::Repository("forced ack failure".to_string()));
        }
        Ok(DiscoveryContextSnapshotAckResult {
            request_id: request_id.to_string(),
            status: "EXPIRED".to_string(),
            acknowledged_at: "2026-06-25T00:00:02Z".to_string(),
        })
    }
}

type HomeHeroPresentation = HashMap<String, (Option<String>, Option<String>)>;

#[derive(Default)]
struct RecordingDiscoveryRepository {
    state: Mutex<Option<DiscoverySyncStateRecord>>,
    runs: Mutex<Vec<DiscoverySyncRunRecord>>,
    commits: Mutex<Vec<DiscoveryContextSnapshotCommit>>,
    incremental_commits: Mutex<Vec<DiscoveryContextIncrementalCommit>>,
    public_feed_commits: Mutex<Vec<DiscoveryPublicFeedCommit>>,
    pending_changes: Mutex<Vec<DiscoveryPendingContextChangeRecord>>,
    sections: Mutex<Vec<DiscoverySectionRecord>>,
    items: Mutex<Vec<DiscoveryItemRecord>>,
    facets: Mutex<Vec<DiscoveryFacetRecord>>,
    submitted_subjects: Mutex<Vec<DiscoverySubmittedSubjectRecord>>,
    title_more_like_this_items: Mutex<HashMap<String, Vec<DiscoveryItemRecord>>>,
    title_more_like_this_limits: Mutex<Vec<i64>>,
    generation_list_calls: Mutex<usize>,
    hydrated_home_candidate_ids: Mutex<Vec<Vec<String>>>,
    hydrated_home_hero_ids: Mutex<Vec<String>>,
    // Title-store surrogate for hero presentation hydration, keyed by item id:
    // (background_url, overview). Mirrors production, where home candidates are
    // loaded with the lean projection and only the dedicated hero hydration
    // reads these columns from discovery_titles.
    home_hero_presentation: Mutex<HomeHeroPresentation>,
    personalized_facet_calls: Mutex<usize>,
}

#[async_trait]
impl DiscoveryRepository for RecordingDiscoveryRepository {
    async fn get_discovery_sync_state(
        &self,
        _scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>> {
        Ok(self.state.lock().await.clone())
    }

    async fn upsert_discovery_sync_state(&self, state: &DiscoverySyncStateRecord) -> AppResult<()> {
        *self.state.lock().await = Some(state.clone());
        Ok(())
    }

    async fn try_acquire_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut state = self.state.lock().await;
        let mut next = state.clone().unwrap_or_default();
        next.scope_key = scope_key.to_string();
        let available = next.lease_owner_id.is_none()
            || next
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= now)
            || next.lease_owner_id.as_deref() == Some(owner_id);
        if available {
            next.lease_owner_id = Some(owner_id.to_string());
            next.lease_expires_at = Some(lease_expires_at);
            next.updated_at = now;
            *state = Some(next);
        }
        Ok(available)
    }

    async fn renew_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut state = self.state.lock().await;
        let Some(existing) = state.as_mut() else {
            return Ok(false);
        };
        if existing.scope_key == scope_key && existing.lease_owner_id.as_deref() == Some(owner_id) {
            existing.lease_expires_at = Some(lease_expires_at);
            existing.updated_at = now;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn release_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.as_mut()
            && existing.scope_key == scope_key
            && existing.lease_owner_id.as_deref() == Some(owner_id)
        {
            existing.lease_owner_id = None;
            existing.lease_expires_at = None;
            existing.updated_at = now;
        }
        Ok(())
    }

    async fn get_discovery_sync_run(&self, id: &str) -> AppResult<Option<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .rev()
            .find(|run| run.id == id)
            .cloned())
    }

    async fn upsert_discovery_sync_run(&self, run: &DiscoverySyncRunRecord) -> AppResult<()> {
        let mut runs = self.runs.lock().await;
        if let Some(existing) = runs.iter_mut().find(|existing| existing.id == run.id) {
            *existing = run.clone();
        } else {
            runs.push(run.clone());
        }
        Ok(())
    }

    async fn list_recent_discovery_sync_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .rev()
            .take(limit.clamp(1, 100) as usize)
            .cloned()
            .collect())
    }

    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .filter(|run| run.kind == "context_snapshot")
            .filter(|run| run.status == "complete" || run.status == "warning")
            .filter(|run| run.smg_request_id.is_some())
            .filter(|run| run.acknowledged_at.is_none())
            .take(limit.clamp(1, 100) as usize)
            .cloned()
            .collect())
    }

    async fn commit_discovery_context_snapshot(
        &self,
        commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        self.items
            .lock()
            .await
            .retain(|item| item.run_id != commit.run.id);
        self.items.lock().await.extend(commit.items.clone());
        self.facets
            .lock()
            .await
            .retain(|facet| facet.run_id != commit.run.id);
        self.facets.lock().await.extend(commit.facets.clone());
        self.submitted_subjects
            .lock()
            .await
            .retain(|subject| subject.run_id != commit.run.id);
        self.submitted_subjects
            .lock()
            .await
            .extend(commit.submitted_subjects.clone());
        self.commits.lock().await.push(commit.clone());
        if let Some(sequence) = commit.clear_pending_through_sequence {
            self.pending_changes
                .lock()
                .await
                .retain(|change| change.last_seen_sequence.is_none_or(|seen| seen > sequence));
        }
        Ok(())
    }

    async fn commit_discovery_context_incremental(
        &self,
        commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        let tombstoned_at = commit.run.completed_at.unwrap_or(commit.run.updated_at);
        {
            let mut items = self.items.lock().await;
            for item in items.iter_mut() {
                if commit.tombstone_target_keys.contains(&item.target_key)
                    && item.base_generation_id.as_deref()
                        == commit.run.base_generation_id.as_deref()
                    && item.tombstoned_at.is_none()
                {
                    item.tombstoned_by_run_id = Some(commit.run.id.clone());
                    item.tombstoned_at = Some(tombstoned_at);
                }
            }
            items.extend(commit.items.clone());
        }
        self.incremental_commits.lock().await.push(commit.clone());
        if let Some(sequence) = commit.clear_pending_through_sequence {
            self.pending_changes
                .lock()
                .await
                .retain(|change| change.last_seen_sequence.is_none_or(|seen| seen > sequence));
        }
        Ok(())
    }

    async fn commit_discovery_public_feed(
        &self,
        commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        self.sections
            .lock()
            .await
            .retain(|section| section.run_id != commit.run.id);
        self.sections.lock().await.extend(commit.sections.clone());
        self.items
            .lock()
            .await
            .retain(|item| item.run_id != commit.run.id);
        self.items.lock().await.extend(commit.items.clone());
        self.public_feed_commits.lock().await.push(commit.clone());
        Ok(())
    }

    async fn replace_discovery_submitted_subjects(
        &self,
        run_id: &str,
        subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()> {
        self.submitted_subjects
            .lock()
            .await
            .retain(|subject| subject.run_id != run_id);
        self.submitted_subjects
            .lock()
            .await
            .extend(subjects.to_vec());
        Ok(())
    }

    async fn list_discovery_submitted_subjects(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        Ok(self
            .submitted_subjects
            .lock()
            .await
            .iter()
            .filter(|subject| subject.run_id == run_id)
            .cloned()
            .collect())
    }

    async fn upsert_pending_discovery_context_change(
        &self,
        change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()> {
        let mut pending_changes = self.pending_changes.lock().await;
        if let Some(existing) = pending_changes
            .iter_mut()
            .find(|existing| existing.id == change.id)
        {
            *existing = change.clone();
        } else {
            pending_changes.push(change.clone());
        }
        Ok(())
    }

    async fn get_pending_discovery_context_change(
        &self,
        id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .find(|change| change.id == id)
            .cloned())
    }

    async fn delete_pending_discovery_context_change(&self, id: &str) -> AppResult<u64> {
        let mut pending_changes = self.pending_changes.lock().await;
        let before = pending_changes.len();
        pending_changes.retain(|change| change.id != id);
        Ok((before - pending_changes.len()) as u64)
    }

    async fn list_all_pending_discovery_context_changes(
        &self,
        scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .cloned()
            .collect())
    }

    async fn list_pending_discovery_context_changes(
        &self,
        scope_key: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn count_pending_discovery_context_changes(&self, scope_key: &str) -> AppResult<i64> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .count() as i64)
    }

    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        scope_key: &str,
        last_seen_sequence: i64,
    ) -> AppResult<u64> {
        let mut pending_changes = self.pending_changes.lock().await;
        let before = pending_changes.len();
        pending_changes.retain(|change| {
            change.scope_key != scope_key
                || change
                    .last_seen_sequence
                    .is_none_or(|seen| seen > last_seen_sequence)
        });
        Ok((before - pending_changes.len()) as u64)
    }

    async fn replace_discovery_sections(
        &self,
        run_id: &str,
        sections: &[crate::DiscoverySectionRecord],
    ) -> AppResult<()> {
        self.sections
            .lock()
            .await
            .retain(|section| section.run_id != run_id);
        self.sections.lock().await.extend(sections.to_vec());
        Ok(())
    }

    async fn replace_discovery_items(
        &self,
        run_id: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        self.items.lock().await.retain(|item| item.run_id != run_id);
        self.items.lock().await.extend(items.to_vec());
        Ok(())
    }

    async fn replace_discovery_facets(
        &self,
        run_id: &str,
        facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()> {
        self.facets
            .lock()
            .await
            .retain(|facet| facet.run_id != run_id);
        self.facets.lock().await.extend(facets.to_vec());
        Ok(())
    }

    async fn list_discovery_sections(
        &self,
        run_id: &str,
        surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>> {
        Ok(self
            .sections
            .lock()
            .await
            .iter()
            .filter(|section| section.run_id == run_id)
            .filter(|section| surface.is_none_or(|surface| section.surface == surface))
            .cloned()
            .collect())
    }

    async fn list_public_discovery_section_items(
        &self,
        run_id: &str,
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        limit_per_section: i64,
    ) -> AppResult<Vec<DiscoveryHomeSectionCandidatesRecord>> {
        let sections = self.list_discovery_sections(run_id, Some("public")).await?;
        let all_items = self.items.lock().await.clone();
        let mut records = Vec::new();
        for section in sections {
            if section
                .section_type
                .trim()
                .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
            {
                continue;
            }
            let mut items = all_items
                .iter()
                .filter(|item| item.base_generation_id.as_deref() == Some(run_id))
                .filter(|item| item.tombstoned_at.is_none())
                .filter(|item| item.section_id.as_deref() == Some(section.section_id.as_str()))
                .filter(|item| !item.owned_in_input)
                .filter(|item| include_unresolved || item.resolved)
                .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
                .cloned()
                .collect::<Vec<_>>();
            recording_dedupe_preserving_order(&mut items);
            let total_count = items.len() as i64;
            items.truncate(limit_per_section.max(1) as usize);
            if !items.is_empty() {
                records.push(DiscoveryHomeSectionCandidatesRecord {
                    section,
                    total_count,
                    items: items.into_iter().map(recording_home_candidate).collect(),
                });
            }
        }
        Ok(records)
    }

    async fn list_personalized_discovery_home_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        let mut items = recording_visible_personalized_items(
            &self.items.lock().await,
            run_id,
            readable_library_ids,
            include_unresolved,
        )
        .into_iter()
        .filter(|item| !item.owned_in_input)
        .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
        .collect::<Vec<_>>();
        recording_dedupe_and_sort(&mut items);
        items.truncate(limit.max(1) as usize);
        Ok(items.into_iter().map(recording_home_candidate).collect())
    }

    async fn list_personalized_complete_collection_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        let mut items = recording_visible_personalized_items(
            &self.items.lock().await,
            run_id,
            readable_library_ids,
            include_unresolved,
        )
        .into_iter()
        .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
        .filter(|item| recording_item_media_kind(item) == Some("movie"))
        .filter(|item| !item.owned_in_input)
        .filter(recording_item_has_collection_signal)
        .collect::<Vec<_>>();
        recording_dedupe_and_sort(&mut items);
        items.truncate(limit.max(1) as usize);
        Ok(items.into_iter().map(recording_home_candidate).collect())
    }

    async fn list_personalized_discovery_facets(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        include_unresolved: bool,
    ) -> AppResult<Vec<DiscoveryFacetRecord>> {
        *self.personalized_facet_calls.lock().await += 1;
        let items = recording_visible_personalized_items(
            &self.items.lock().await,
            run_id,
            readable_library_ids,
            include_unresolved,
        )
        .into_iter()
        .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
        .collect::<Vec<_>>();
        Ok(recording_canonical_facet_records(run_id, &items))
    }

    async fn list_discovery_home_top_rated_items(
        &self,
        public_run_id: Option<&str>,
        context_run_id: Option<&str>,
        readable_library_ids: &[String],
        allowed_media_kinds: &[String],
        _owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        let excluded_identity_keys = excluded_identity_keys
            .iter()
            .map(|key| key.trim().to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let all_items = self.items.lock().await.clone();
        let mut items = Vec::new();
        if let Some(run_id) = public_run_id {
            items.extend(
                all_items
                    .iter()
                    .filter(|item| item.base_generation_id.as_deref() == Some(run_id))
                    .filter(|item| item.tombstoned_at.is_none())
                    .filter(|item| !item.owned_in_input)
                    .filter(|item| include_unresolved || item.resolved)
                    .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
                    .filter(|item| {
                        !excluded_identity_keys.contains(
                            &recording_item_identity_key(item)
                                .trim()
                                .to_ascii_lowercase(),
                        )
                    })
                    .cloned(),
            );
        }
        if let Some(run_id) = context_run_id {
            items.extend(
                recording_visible_personalized_items(
                    &all_items,
                    run_id,
                    readable_library_ids,
                    include_unresolved,
                )
                .into_iter()
                .filter(|item| !item.owned_in_input)
                .filter(|item| recording_item_is_allowed(item, allowed_media_kinds))
                .filter(|item| {
                    !excluded_identity_keys.contains(
                        &recording_item_identity_key(item)
                            .trim()
                            .to_ascii_lowercase(),
                    )
                }),
            );
        }
        items.sort_by(recording_compare_top_rated_items);
        recording_dedupe_preserving_order(&mut items);
        items.truncate(limit.max(1) as usize);
        Ok(items.into_iter().map(recording_home_candidate).collect())
    }

    async fn hydrate_discovery_home_candidates(
        &self,
        candidates: &mut [DiscoveryHomeCandidate],
    ) -> AppResult<()> {
        self.hydrated_home_candidate_ids.lock().await.push(
            candidates
                .iter()
                .map(|candidate| candidate.item.id.clone())
                .collect(),
        );
        Ok(())
    }

    async fn hydrate_discovery_home_hero(
        &self,
        candidate: &mut DiscoveryHomeCandidate,
    ) -> AppResult<()> {
        self.hydrated_home_hero_ids
            .lock()
            .await
            .push(candidate.item.id.clone());
        if let Some((background_url, overview)) = self
            .home_hero_presentation
            .lock()
            .await
            .get(&candidate.item.id)
            .cloned()
        {
            candidate.item.background_url = background_url;
            candidate.item.overview = overview;
        }
        Ok(())
    }

    async fn list_discovery_home_filter_options(
        &self,
        _public_run_id: Option<&str>,
        _context_run_id: Option<&str>,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
    ) -> AppResult<DiscoveryHomeFilterOptions> {
        Ok(DiscoveryHomeFilterOptions::default())
    }

    async fn list_catalog_public_discovery_items(
        &self,
        run_id: &str,
        _owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        let excluded_identity_keys = excluded_identity_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut items = self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| item.base_generation_id.as_deref() == Some(run_id))
            .filter(|item| item.tombstoned_at.is_none())
            .filter(|item| !item.owned_in_input)
            .filter(|item| {
                let identity_key = if item.target_key.trim().is_empty() {
                    item.id.as_str()
                } else {
                    item.target_key.as_str()
                };
                !excluded_identity_keys.contains(identity_key.trim().to_ascii_lowercase().as_str())
            })
            .filter(|item| include_unresolved || item.resolved)
            .filter(|item| recording_discovery_item_media_kind(item).as_deref() == Some(media_kind))
            .cloned()
            .collect::<Vec<_>>();
        recording_dedupe_and_sort(&mut items);
        let total_count = items.len() as i64;
        let limit = limit.max(0) as usize;
        items.truncate(limit);
        Ok(CatalogDiscoveryCandidatesRecord { items, total_count })
    }

    async fn list_catalog_public_discovery_sections(
        &self,
        run_id: &str,
        _owned_library_ids: &[String],
        excluded_identity_keys: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit_per_section: i64,
    ) -> AppResult<Vec<CatalogDiscoverySectionCandidatesRecord>> {
        let excluded_identity_keys = excluded_identity_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let sections = self.list_discovery_sections(run_id, Some("public")).await?;
        let all_items = self.items.lock().await.clone();
        let mut records = Vec::new();
        for section in sections {
            if section
                .section_type
                .trim()
                .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
            {
                continue;
            }
            let mut items = all_items
                .iter()
                .filter(|item| item.base_generation_id.as_deref() == Some(run_id))
                .filter(|item| item.tombstoned_at.is_none())
                .filter(|item| item.section_id.as_deref() == Some(section.section_id.as_str()))
                .filter(|item| !item.owned_in_input)
                .filter(|item| {
                    let identity_key = if item.target_key.trim().is_empty() {
                        item.id.as_str()
                    } else {
                        item.target_key.as_str()
                    };
                    !excluded_identity_keys
                        .contains(identity_key.trim().to_ascii_lowercase().as_str())
                })
                .filter(|item| include_unresolved || item.resolved)
                .filter(|item| {
                    recording_discovery_item_media_kind(item).as_deref() == Some(media_kind)
                })
                .cloned()
                .collect::<Vec<_>>();
            recording_dedupe_preserving_order(&mut items);
            let total_count = items.len() as i64;
            items.truncate(limit_per_section.max(1) as usize);
            if !items.is_empty() {
                records.push(CatalogDiscoverySectionCandidatesRecord {
                    section_id: section.section_id,
                    section_type: section.section_type,
                    title: Some(section.title),
                    total_count,
                    items,
                });
            }
        }
        Ok(records)
    }

    async fn list_catalog_personalized_discovery_items(
        &self,
        run_id: &str,
        readable_library_ids: &[String],
        media_kind: &str,
        include_unresolved: bool,
        limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        let mut items = recording_visible_personalized_items(
            &self.items.lock().await,
            run_id,
            readable_library_ids,
            include_unresolved,
        );
        items.retain(|item| {
            recording_discovery_item_media_kind(item).as_deref() == Some(media_kind)
        });
        recording_dedupe_and_sort(&mut items);
        let total_count = items.len() as i64;
        let limit = limit.max(0) as usize;
        items.truncate(limit);
        Ok(CatalogDiscoveryCandidatesRecord { items, total_count })
    }

    async fn query_discovery_items(
        &self,
        query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<DiscoveryItemsPageRecord> {
        let all_items = self.items.lock().await.clone();
        let mut items = Vec::new();
        if let Some(context_run_id) = query.context_run_id.as_deref() {
            items.extend(recording_visible_personalized_items(
                &all_items,
                context_run_id,
                &query.readable_library_ids,
                true,
            ));
        }
        if let Some(public_run_id) = query.public_run_id.as_deref() {
            items.extend(
                all_items
                    .iter()
                    .filter(|item| item.base_generation_id.as_deref() == Some(public_run_id))
                    .filter(|item| item.tombstoned_at.is_none())
                    .cloned(),
            );
        }
        items.retain(|item| {
            recording_discovery_item_media_kind(item).is_some_and(|media_kind| {
                query
                    .allowed_media_kinds
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&media_kind))
            }) && recording_item_matches_query(item, &query.filters)
        });
        recording_dedupe_and_sort(&mut items);
        let total_count = items.len() as i64;
        let offset = query.offset.min(items.len());
        let limit = query.limit.min(items.len().saturating_sub(offset));
        let items = items.into_iter().skip(offset).take(limit).collect();
        Ok(DiscoveryItemsPageRecord { items, total_count })
    }

    async fn replace_title_more_like_this_items(
        &self,
        title_id: &str,
        _language: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        self.title_more_like_this_items
            .lock()
            .await
            .insert(title_id.to_string(), items.to_vec());
        Ok(())
    }

    async fn list_title_more_like_this_items(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        self.title_more_like_this_limits.lock().await.push(limit);
        let limit = limit.max(0) as usize;
        let mut items = self
            .title_more_like_this_items
            .lock()
            .await
            .get(title_id)
            .cloned()
            .unwrap_or_default();
        items.truncate(limit);
        Ok(items)
    }

    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        *self.generation_list_calls.lock().await += 1;
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| item.base_generation_id.as_deref() == Some(base_generation_id))
            .filter(|item| item.tombstoned_at.is_none())
            .cloned()
            .collect())
    }

    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        Ok(self
            .facets
            .lock()
            .await
            .iter()
            .filter(|facet| facet.run_id == run_id)
            .cloned()
            .collect())
    }

    async fn prune_discovery_history(
        &self,
        _scope_key: &str,
        _retain_successful_per_kind: usize,
        diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<crate::DiscoveryPruneReport> {
        let mut runs = self.runs.lock().await;
        let before = runs.len();
        runs.retain(|run| {
            run.updated_at >= diagnostic_cutoff
                || run.status == "complete"
                || run.status == "warning"
                || run.status == "running"
        });
        Ok(crate::DiscoveryPruneReport {
            runs_deleted: (before - runs.len()) as u64,
        })
    }
}

fn unused_gateway_call() -> AppError {
    AppError::Repository("unexpected metadata gateway call in discovery sync test".to_string())
}

#[expect(
    clippy::too_many_arguments,
    reason = "test status fixture mirrors the SMG snapshot status fields under test"
)]
fn snapshot_status_result(
    request_id: &str,
    status: &str,
    phase: &str,
    retry_after_seconds: i32,
    page_count: i32,
    item_count: i32,
    facet_count: i32,
    last_error: &str,
) -> DiscoveryContextSnapshotStatusResult {
    DiscoveryContextSnapshotStatusResult {
        request_id: request_id.to_string(),
        status: status.to_string(),
        phase: phase.to_string(),
        subject_count: 1,
        item_count,
        page_count,
        facet_count,
        lazy_hydration_queued_count: 0,
        lazy_hydration_sources: Vec::new(),
        discovery_index_watermark: "watermark-1".to_string(),
        retry_after_seconds,
        created_at: "2026-06-25T00:00:00Z".to_string(),
        started_at: "2026-06-25T00:00:00Z".to_string(),
        completed_at: if status == "COMPLETE" {
            "2026-06-25T00:00:01Z".to_string()
        } else {
            String::new()
        },
        expires_at: "2026-06-26T00:00:00Z".to_string(),
        last_error: last_error.to_string(),
    }
}

fn polling_snapshot_status(request_id: &str, status: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(request_id, status, "building", 60, 0, 0, 0, "")
}

fn complete_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(request_id, "COMPLETE", "complete", 1, 1, 1, 1, "")
}

fn failed_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(
        request_id,
        "FAILED",
        "failed",
        600,
        0,
        0,
        0,
        "forced snapshot failure",
    )
}

fn queue_full_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    DiscoveryContextSnapshotStatusResult {
        request_id: request_id.to_string(),
        status: "QUEUE_FULL".to_string(),
        phase: "queued".to_string(),
        subject_count: 1,
        item_count: 0,
        page_count: 0,
        facet_count: 0,
        lazy_hydration_queued_count: 0,
        lazy_hydration_sources: Vec::new(),
        discovery_index_watermark: String::new(),
        retry_after_seconds: 600,
        created_at: "2026-06-25T00:00:00Z".to_string(),
        started_at: String::new(),
        completed_at: String::new(),
        expires_at: "2026-06-26T00:00:00Z".to_string(),
        last_error: String::new(),
    }
}

fn queue_full_context_changes_result() -> DiscoveryContextChangesResult {
    DiscoveryContextChangesResult {
        status: "QUEUE_FULL".to_string(),
        retry_after_seconds: 900,
        generated_at: "2026-06-25T00:00:03Z".to_string(),
        context_fingerprint: "fingerprint-current".to_string(),
        previous_context_fingerprint: "fingerprint-previous".to_string(),
        discovery_index_watermark: String::new(),
        context_subject_count: 1,
        changed_subject_count: 1,
        resolved_changed_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        affected_target_keys: Vec::new(),
        items: Vec::new(),
    }
}

fn discovery_run_record(
    id: &str,
    observed_at: DateTime<Utc>,
    status: &str,
) -> DiscoverySyncRunRecord {
    DiscoverySyncRunRecord {
        id: id.to_string(),
        kind: "context_snapshot".to_string(),
        status: status.to_string(),
        trigger_source: "scheduled_interval".to_string(),
        region: "US".to_string(),
        language: "en".to_string(),
        subject_count: 10,
        subject_fingerprint: Some(format!("{id}-fingerprint")),
        previous_subject_fingerprint: None,
        base_generation_id: None,
        changed_subject_count: 0,
        affected_target_count: 0,
        smg_request_id: Some(format!("{id}-request")),
        smg_status: Some(status.to_string()),
        discovery_index_watermark: None,
        page_count: Some(1),
        item_count: Some(5),
        facet_count: Some(2),
        acknowledged_at: None,
        error_text: None,
        started_at: Some(observed_at),
        completed_at: Some(observed_at),
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn discovery_pending_change_record(
    id: &str,
    scope_key: &str,
) -> DiscoveryPendingContextChangeRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    let tmdb_id = id
        .rsplit('-')
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(603);
    DiscoveryPendingContextChangeRecord {
        id: id.to_string(),
        scope_key: scope_key.to_string(),
        subject_key: Some(format!("tmdb:movie:{tmdb_id}")),
        previous_subject_key: None,
        change_type: "updated".to_string(),
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
        first_seen_sequence: Some(1),
        last_seen_sequence: Some(1),
        first_seen_at: observed_at,
        last_seen_at: observed_at,
    }
}

fn discovery_section_record(
    run_id: &str,
    section_id: &str,
    section_type: &str,
    surface: &str,
) -> DiscoverySectionRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    DiscoverySectionRecord {
        id: format!("{run_id}:section:{section_id}"),
        run_id: run_id.to_string(),
        section_id: section_id.to_string(),
        section_type: section_type.to_string(),
        surface: surface.to_string(),
        title: section_id.to_string(),
        sort_index: 0,
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn recording_home_candidate(item: DiscoveryItemRecord) -> DiscoveryHomeCandidate {
    let has_hero_backdrop = item
        .background_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty());
    let rating_source_count = item
        .rating_sources
        .iter()
        .chain(item.external_ratings.iter().map(|rating| &rating.source))
        .map(|source| source.trim().to_ascii_lowercase())
        .filter(|source| !source.is_empty())
        .collect::<HashSet<_>>()
        .len() as i32;
    let best_external_rating = item
        .external_ratings
        .iter()
        .filter_map(|rating| {
            rating
                .normalized
                .is_finite()
                .then_some(if rating.normalized <= 1.0 {
                    rating.normalized * 10.0
                } else {
                    rating.normalized
                })
        })
        .filter(|rating| *rating > 0.0)
        .max_by(f64::total_cmp);
    let best_external_rating_votes = item
        .external_ratings
        .iter()
        .filter_map(|rating| rating.votes)
        .max()
        .unwrap_or_default();
    DiscoveryHomeCandidate {
        discovery_title_id: format!("recording:{}", item.id),
        matched_subject_keys: item.matched_subject_keys.clone(),
        affinity_terms: item.facet_terms.clone(),
        has_hero_backdrop,
        rating_source_count,
        best_external_rating,
        best_external_rating_votes,
        item,
    }
}

fn recording_visible_personalized_items(
    items: &[DiscoveryItemRecord],
    run_id: &str,
    readable_library_ids: &[String],
    include_unresolved: bool,
) -> Vec<DiscoveryItemRecord> {
    let readable_library_ids = readable_library_ids.iter().collect::<HashSet<_>>();
    items
        .iter()
        .filter(|item| item.base_generation_id.as_deref() == Some(run_id))
        .filter(|item| item.tombstoned_at.is_none())
        .filter(|item| include_unresolved || item.resolved)
        .filter(|item| {
            item.library_provenance.iter().any(|provenance| {
                provenance
                    .library_id
                    .as_ref()
                    .is_some_and(|library_id| readable_library_ids.contains(library_id))
            })
        })
        .cloned()
        .collect()
}

fn recording_item_matches_query(item: &DiscoveryItemRecord, query: &DiscoveryItemsQuery) -> bool {
    if !query.include_owned && item.owned_in_input {
        return false;
    }
    if !query.include_unresolved && !item.resolved {
        return false;
    }
    if !query.target_keys.is_empty()
        && !query
            .target_keys
            .iter()
            .any(|target_key| target_key.eq_ignore_ascii_case(&item.target_key))
    {
        return false;
    }
    if let Some(query_text) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let query_text = query_text.to_ascii_lowercase();
        let matches_text = [
            Some(item.display_title.as_str()),
            item.original_title.as_deref(),
            item.sort_title.as_deref(),
            item.overview.as_deref(),
            item.tmdb_collection_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.to_ascii_lowercase().contains(&query_text));
        if !matches_text {
            return false;
        }
    }
    if !query.target_kinds.is_empty()
        && !recording_item_media_kind(item).is_some_and(|kind| {
            query
                .target_kinds
                .iter()
                .any(|target_kind| target_kind.eq_ignore_ascii_case(kind))
        })
    {
        return false;
    }
    recording_text_matches(&item.sources, item.best_source.as_deref(), &query.sources)
        && recording_values_match(&item.relation_types, &query.relation_types)
        && recording_values_match(&item.relation_subtypes, &query.relation_subtypes)
        && recording_canonical_facet_values_match(item, "genre", &query.genres)
        && recording_values_match(&item.status_tags, &query.status_tags)
        && recording_values_match(&item.facet_terms, &query.facet_terms)
}

fn recording_values_match(values: &[String], filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| {
            values
                .iter()
                .any(|value| value.eq_ignore_ascii_case(filter))
        })
}

fn recording_canonical_facet_values_match(
    item: &DiscoveryItemRecord,
    kind: &str,
    filters: &[String],
) -> bool {
    filters.is_empty()
        || filters.iter().any(|filter| {
            let filter_key = recording_canonical_label_key(filter);
            item.facet_terms.iter().any(|term| {
                recording_canonical_facet_display_value(term).is_some_and(
                    |(facet_name, facet_value)| {
                        facet_name.eq_ignore_ascii_case(kind)
                            && (term.eq_ignore_ascii_case(filter)
                                || recording_canonical_label_key(&facet_value) == filter_key)
                    },
                )
            })
        })
}

fn recording_text_matches(values: &[String], text: Option<&str>, filters: &[String]) -> bool {
    filters.is_empty()
        || text.is_some_and(|text| {
            filters
                .iter()
                .any(|filter| text.eq_ignore_ascii_case(filter))
        })
        || filters.iter().any(|filter| {
            values
                .iter()
                .any(|value| value.eq_ignore_ascii_case(filter))
        })
}

fn recording_item_media_kind(item: &DiscoveryItemRecord) -> Option<&str> {
    match item.content_type.as_deref().map(str::trim) {
        Some(content_type) if !content_type.is_empty() => {
            recording_normalized_media_kind(Some(content_type))
        }
        _ => recording_normalized_media_kind(Some(item.target_kind.as_str())),
    }
}

fn recording_item_is_allowed(item: &DiscoveryItemRecord, allowed_media_kinds: &[String]) -> bool {
    recording_item_media_kind(item).is_some_and(|media_kind| {
        allowed_media_kinds
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(media_kind))
    })
}

fn recording_normalized_media_kind(value: Option<&str>) -> Option<&'static str> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

fn recording_item_has_collection_signal(item: &DiscoveryItemRecord) -> bool {
    item.tmdb_collection_id.is_some()
        || item
            .tmdb_collection_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
        || item
            .relation_types
            .iter()
            .chain(item.relation_subtypes.iter())
            .any(|value| {
                let value = value.trim().to_ascii_lowercase();
                value == "tmdb.collection"
                    || value.contains("collection")
                    || value.contains("franchise")
            })
}

fn recording_canonical_facet_records(
    run_id: &str,
    items: &[DiscoveryItemRecord],
) -> Vec<DiscoveryFacetRecord> {
    let mut counts = BTreeMap::<(String, String), i64>::new();
    for item in items.iter().filter(|item| !item.owned_in_input) {
        let mut seen_item_terms = HashSet::new();
        for term in &item.facet_terms {
            let Some((facet_name, facet_value)) = recording_canonical_facet_display_value(term)
            else {
                continue;
            };
            if seen_item_terms.insert((facet_name.clone(), facet_value.clone())) {
                *counts.entry((facet_name, facet_value)).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .map(
            |((facet_name, facet_value), local_count)| DiscoveryFacetRecord {
                run_id: run_id.to_string(),
                facet_name,
                facet_value,
                smg_count: None,
                local_count: Some(local_count),
            },
        )
        .collect()
}

fn recording_canonical_facet_display_value(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    let kind = parts.next()?.trim();
    if !kind.eq_ignore_ascii_case("genre") && !kind.eq_ignore_ascii_case("theme") {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some((
        kind.to_ascii_lowercase(),
        recording_canonical_label_from_slug(tail),
    ))
}

fn recording_canonical_label_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn recording_discovery_item_media_kind(item: &DiscoveryItemRecord) -> Option<String> {
    item.content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| Some(item.target_kind.trim()))
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "anime" => Some("anime".to_string()),
            "movie" => Some("movie".to_string()),
            "series" => Some("series".to_string()),
            _ => None,
        })
}

fn recording_canonical_label_from_slug(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn recording_dedupe_preserving_order(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(recording_item_identity_key(item).to_string()));
}

fn recording_dedupe_and_sort(items: &mut Vec<DiscoveryItemRecord>) {
    recording_dedupe_preserving_order(items);
    items.sort_by(|left, right| {
        right
            .rank_score
            .partial_cmp(&left.rank_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.sort_title
                    .as_deref()
                    .unwrap_or(&left.display_title)
                    .cmp(right.sort_title.as_deref().unwrap_or(&right.display_title))
            })
            .then_with(|| left.target_key.cmp(&right.target_key))
    });
}

fn recording_compare_top_rated_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> std::cmp::Ordering {
    let left_external = recording_external_rating_score(left);
    let right_external = recording_external_rating_score(right);
    right_external
        .is_some()
        .cmp(&left_external.is_some())
        .then_with(|| recording_compare_option_f64_desc(left_external, right_external))
        .then_with(|| {
            recording_external_rating_votes(right).cmp(&recording_external_rating_votes(left))
        })
        .then_with(|| recording_compare_option_f64_desc(left.rating, right.rating))
        .then_with(|| recording_compare_option_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| right.source_count.cmp(&left.source_count))
        .then_with(|| left.target_key.cmp(&right.target_key))
        .then_with(|| left.id.cmp(&right.id))
}

fn recording_compare_option_f64_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn recording_external_rating_score(item: &DiscoveryItemRecord) -> Option<f64> {
    item.external_ratings
        .iter()
        .filter_map(|rating| {
            let normalized = rating.normalized;
            (normalized > 0.0).then_some(if normalized <= 1.0 {
                normalized * 10.0
            } else {
                normalized
            })
        })
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn recording_external_rating_votes(item: &DiscoveryItemRecord) -> i32 {
    item.external_ratings
        .iter()
        .filter_map(|rating| rating.votes)
        .max()
        .unwrap_or_default()
}

fn recording_item_identity_key(item: &DiscoveryItemRecord) -> &str {
    if item.target_key.trim().is_empty() {
        item.id.as_str()
    } else {
        item.target_key.as_str()
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "test discovery item fixture keeps each varied assertion field explicit"
)]
fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    section_id: Option<&str>,
    target_key: &str,
    display_title: &str,
    target_kind: &str,
    rank_score: f64,
    genre_labels: &[&str],
    relation_subtypes: &[&str],
    owned_in_input: bool,
    resolved: bool,
) -> DiscoveryItemRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    let library_provenance = if run_id == base_generation_id && run_id.starts_with("public") {
        Vec::new()
    } else {
        vec![DiscoveryItemLibraryProvenanceRecord {
            subject_key: "tmdb:movie:603".to_string(),
            title_id: Some("title-603".to_string()),
            library_id: Some(scryer_domain::default_library_id_for_facet(
                &MediaFacet::Movie,
            )),
        }]
    };
    DiscoveryItemRecord {
        id: format!("{run_id}:item:{target_key}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: if run_id == base_generation_id && run_id.starts_with("public") {
            "public_feed".to_string()
        } else {
            "context_snapshot".to_string()
        },
        section_id: section_id.map(str::to_string),
        sort_index: 0,
        target_key: target_key.to_string(),
        target_kind: target_kind.to_string(),
        resolved,
        resolved_title_id: None,
        display_title: display_title.to_string(),
        original_title: None,
        sort_title: Some(display_title.to_string()),
        year: Some(2026),
        poster_path: None,
        poster_url: None,
        background_url: None,
        overview: None,
        content_type: Some(target_kind.to_string()),
        canonical_tags: canonical_genre_tags(genre_labels),
        is_adult: false,
        content_ratings: Vec::new(),
        rating: Some(7.5),
        rating_sources: Vec::new(),
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        status_tags: Vec::new(),
        source_tags: Vec::new(),
        sources: vec!["smg".to_string()],
        best_source: Some("smg".to_string()),
        relation_types: Vec::new(),
        relation_subtypes: relation_subtypes
            .iter()
            .map(|subtype| (*subtype).to_string())
            .collect(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: Some(1),
        edge_count: Some(1),
        relation_count: Some(relation_subtypes.len() as i32),
        source_subject_count: Some(1),
        rank_score: Some(rank_score),
        matched_subject_keys: Vec::new(),
        matched_subject_titles: Vec::new(),
        matched_subject_count: 0,
        library_provenance,
        tmdb_collection_id: relation_subtypes
            .contains(&"tmdb.collection")
            .then(|| "123".to_string()),
        tmdb_collection_name: relation_subtypes
            .contains(&"tmdb.collection")
            .then(|| "Example Collection".to_string()),
        owned_in_input,
        studio_slug: None,
        person_ids: Vec::new(),
        facet_terms: genre_labels
            .iter()
            .map(|genre| {
                format!(
                    "canonical:genre:{}",
                    genre.trim().to_ascii_lowercase().replace(' ', "_")
                )
            })
            .collect(),
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn test_library_scan_completed_event(
    session_id: &str,
    found_titles: i64,
) -> LibraryScanCompletedEventData {
    LibraryScanCompletedEventData {
        session_id: session_id.to_string(),
        status: "completed".to_string(),
        found_titles,
        title_match_completed: found_titles,
        title_match_total_known: true,
        titles_completed: found_titles,
        titles_total: Some(found_titles),
        files_completed: 0,
        files_total: Some(0),
        summary: None,
        warning_message: None,
    }
}

fn test_active_library_scan_run(started_at: chrono::DateTime<Utc>) -> JobRun {
    JobRun {
        id: "active-scan-run".to_string(),
        operation_type: JobKey::LibraryScanMovies.as_str().to_string(),
        actor_user_id: None,
        job_key: JobKey::LibraryScanMovies,
        display_name: "Scan Movies".to_string(),
        category: JobCategory::Library,
        section: JobSection::Primary,
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::Manual,
        started_at,
        completed_at: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        progress_json: None,
        library_scan_progress: None,
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

fn test_title_context_snapshot(title: &Title) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        external_ids: DomainExternalIds {
            imdb_id: title.imdb_id.clone(),
            tmdb_id: test_external_id(title, &["tmdb_movie", "tmdb"]),
            tvdb_id: test_external_id(title, &["tvdb_series", "tvdb_movie", "tvdb"]),
            anidb_id: test_external_id(title, &["anidb"]),
        },
        poster_url: title.poster_url.clone(),
        year: title.year,
    }
}

fn test_external_id(title: &Title, sources: &[&str]) -> Option<String> {
    title
        .external_ids
        .iter()
        .find(|external_id| sources.iter().any(|source| external_id.source == *source))
        .map(|external_id| external_id.value.clone())
}

#[test]
fn discovery_title_deserializes_studio_slug_and_person_ids_with_defaults() {
    // The SMG feed uses snake_case studio_slug / person_ids and
    // absence must default tolerantly (no deny_unknown_fields, no strict enums).
    let mut base = serde_json::to_value(test_discovery_title())
        .expect("fixture discovery title should serialize");
    let object = base.as_object_mut().expect("discovery title is an object");

    // Absent: drop both keys and inject an unknown future field.
    object.remove("studio_slug");
    object.remove("person_ids");
    object.insert(
        "unexpected_future_field".to_string(),
        serde_json::json!({"anything": true}),
    );
    let absent: DiscoveryTitle = serde_json::from_value(serde_json::Value::Object(object.clone()))
        .expect("missing studio_slug/person_ids should default, unknown fields tolerated");
    assert_eq!(absent.studio_slug, None);
    assert!(absent.person_ids.is_empty());

    // Present: snake_case keys parse onto the typed fields.
    object.insert("studio_slug".to_string(), serde_json::json!("a24"));
    object.insert("person_ids".to_string(), serde_json::json!([101, 202]));
    let present: DiscoveryTitle = serde_json::from_value(serde_json::Value::Object(object.clone()))
        .expect("snake_case studio_slug/person_ids should deserialize");
    assert_eq!(present.studio_slug.as_deref(), Some("a24"));
    assert_eq!(present.person_ids, vec![101, 202]);
}

fn test_discovery_title() -> DiscoveryTitle {
    DiscoveryTitle {
        target_key: "tmdb:movie:604".to_string(),
        target_kind: "movie".to_string(),
        resolved: false,
        resolved_title_id: String::new(),
        display_title: "Another Example Movie".to_string(),
        original_title: String::new(),
        year: Some(2026),
        poster_path: String::new(),
        poster_url: String::new(),
        overview: "A fixture discovery title".to_string(),
        content_type: "movie".to_string(),
        rating: Some(7.5),
        rating_sources: vec!["smg".to_string()],
        external_ratings: Vec::new(),
        external_ids: Vec::new(),
        rating_provenance: Vec::new(),
        status_tags: Vec::new(),
        background_url: String::new(),
        source_tags: Vec::new(),
        canonical_tags: Vec::new(),
        is_adult: false,
        content_ratings: Vec::new(),
        sources: vec!["popular".to_string()],
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: 1,
        edge_count: 1,
        relation_count: 0,
        source_subject_count: 1,
        rank_score: 0.8,
        best_source: "popular".to_string(),
        matched_subject_keys: vec!["tmdb:movie:603".to_string()],
        matched_subject_titles: vec!["The Example Movie".to_string()],
        matched_subject_count: 1,
        tmdb_collection_id: None,
        tmdb_collection_name: String::new(),
        owned_in_input: false,
        studio_slug: None,
        person_ids: Vec::new(),
        facet_terms: vec!["movie".to_string()],
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
    }
}
