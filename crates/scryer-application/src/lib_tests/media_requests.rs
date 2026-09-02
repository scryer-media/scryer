use super::*;
use crate::media_requests::normalize_media_request_external_ids;

#[derive(Default)]
struct MediaRequestMetadataGateway {
    movies: HashMap<i64, MovieMetadata>,
    series: HashMap<i64, SeriesMetadata>,
    fail_detail: bool,
}

#[async_trait]
impl MetadataGateway for MediaRequestMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        if self.fail_detail {
            return Err(AppError::Repository("movie metadata unavailable".into()));
        }
        self.movies
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        if self.fail_detail {
            return Err(AppError::Repository("series metadata unavailable".into()));
        }
        self.series
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("series {tvdb_id}")))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let movies = movie_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.movies
                    .get(tvdb_id)
                    .cloned()
                    .map(|movie| (*tvdb_id, movie))
            })
            .collect();
        let series = series_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.series
                    .get(tvdb_id)
                    .cloned()
                    .map(|series| (*tvdb_id, series))
            })
            .collect();
        Ok(BulkMetadataResult { movies, series })
    }

    async fn get_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        _language: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        if self.fail_detail {
            return Err(AppError::Repository("movie metadata unavailable".into()));
        }
        let mut result = MovieTitleBulkResult::default();
        for (ref_index, movie_ref) in refs.iter().enumerate() {
            let movie = self.movies.values().find(|movie| {
                movie_ref
                    .smg_id
                    .is_some_and(|smg_id| movie.smg_id == Some(smg_id))
                    || movie_ref
                        .tvdb_id
                        .is_some_and(|tvdb_id| movie.tvdb_id == Some(tvdb_id))
                    || movie_ref
                        .tmdb_id
                        .is_some_and(|tmdb_id| movie.tmdb_id == Some(tmdb_id))
                    || movie_ref
                        .imdb_id
                        .as_deref()
                        .is_some_and(|imdb_id| movie.imdb_id == imdb_id)
            });
            if let Some(movie) = movie {
                result.by_ref_index.insert(ref_index, movie.clone());
            } else {
                result.missing_ref_indexes.push(ref_index);
            }
        }
        Ok(result)
    }
}

fn external_id(source: &str, value: impl ToString) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

fn assert_external_ids(external_ids: &[ExternalId], expected: &[(&str, &str)]) {
    let actual = external_ids
        .iter()
        .map(|external_id| (external_id.source.as_str(), external_id.value.as_str()))
        .collect::<HashSet<_>>();
    let expected = expected.iter().copied().collect::<HashSet<_>>();
    assert_eq!(actual, expected);
}

fn make_series_metadata(tvdb_id: i64, name: &str) -> SeriesMetadata {
    SeriesMetadata {
        target_key: None,
        tvdb_id,
        name: name.to_string(),
        sort_name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "Continuing".to_string(),
        first_aired: "2026-01-01".to_string(),
        overview: format!("{name} overview"),
        network: "Test Network".to_string(),
        runtime_minutes: 24,
        poster_url: format!("https://example.com/{tvdb_id}.jpg"),
        background_url: None,
        original_language: Some("jpn".to_string()),
        country: "JP".to_string(),
        canonical_tags: vec![],
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        seasons: Vec::new(),
        episodes: Vec::new(),
        anime_mappings: Vec::new(),
        anime_movies: Vec::new(),
        ratings: Default::default(),
        credits: Vec::new(),
    }
}

#[test]
fn library_permission_request_shadowing_expands_and_normalizes_masks() {
    let request = scryer_domain::LibraryPermissionMask::from_permissions([
        scryer_domain::LibraryPermission::Request,
    ]);
    assert!(request.is_strictly_requestable());
    assert_eq!(request.normalized_for_storage(), request);

    let auto_approve = scryer_domain::LibraryPermissionMask::from_permissions([
        scryer_domain::LibraryPermission::AutoApproveRequests,
    ]);
    assert!(auto_approve.is_strictly_requestable());
    assert!(auto_approve.can_auto_approve_requests());
    assert!(
        auto_approve
            .with_request_shadowing()
            .contains(scryer_domain::LibraryPermissionMask::REQUEST)
    );
    assert_eq!(auto_approve.normalized_for_storage(), auto_approve);

    let manage_titles = scryer_domain::LibraryPermissionMask::from_permissions([
        scryer_domain::LibraryPermission::ManageTitles,
        scryer_domain::LibraryPermission::AutoApproveRequests,
        scryer_domain::LibraryPermission::Request,
    ]);
    assert!(!manage_titles.is_strictly_requestable());
    assert!(!manage_titles.can_auto_approve_requests());
    assert!(
        manage_titles
            .with_request_shadowing()
            .contains(scryer_domain::LibraryPermissionMask::AUTO_APPROVE_REQUESTS)
    );
    assert_eq!(
        manage_titles.normalized_for_storage(),
        scryer_domain::LibraryPermissionMask::MANAGE_TITLES
    );
}

#[tokio::test]
async fn list_libraries_for_manage_titles_ignores_app_settings_override() {
    let harness = bootstrap_media_request_app();
    let actor = test_user_with_app_permissions(
        "catalog-settings-only",
        AppPermissionMask::MANAGE_CATALOG_SETTINGS,
    );

    let manageable_libraries = harness
        .app
        .list_libraries_for_permission(
            &actor,
            Some(MediaFacet::Movie),
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await
        .expect("manageable libraries should load");
    assert!(
        manageable_libraries.is_empty(),
        "app-level catalog settings must not imply title queue management"
    );

    let visible_libraries = harness
        .app
        .list_libraries_for_permission(
            &actor,
            Some(MediaFacet::Movie),
            scryer_domain::LibraryPermission::View,
        )
        .await
        .expect("view libraries should still use app-level settings override");
    assert!(
        !visible_libraries.is_empty(),
        "catalog settings override should continue to expose visible libraries"
    );
}

#[tokio::test]
async fn submit_media_request_creates_request_requester_and_domain_event() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let outcome = harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9010))
        .await
        .expect("request submission should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.id, outcome.request_id);
    assert_eq!(request.library_id, library_id);
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert_eq!(
        request.requested_quality_profile_id.as_deref(),
        Some(crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID),
        "unconfigured requests snapshot the built-in default profile"
    );
    assert_eq!(
        request.requested_quality_profile_name.as_deref(),
        Some("1080P")
    );
    assert!(request.requested_monitor_type.is_none());
    assert_eq!(request.created_by_user_id, harness.user.id);
    assert_eq!(request.requesters.len(), 1);
    assert_eq!(request.requesters[0].user_id, harness.user.id);
    assert_eq!(
        request.external_ids,
        vec![
            ExternalId {
                source: "imdb".to_string(),
                value: "tt0009010".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt1234567".to_string(),
            },
            ExternalId {
                source: "tvdb".to_string(),
                value: "9010".to_string(),
            },
        ]
    );

    let events = harness.domain_events.events.lock().await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].actor_user_id.as_deref(),
        Some(harness.user.id.as_str())
    );
    match &events[0].payload {
        DomainEventPayload::MediaRequestSubmitted(data) => {
            assert_eq!(data.request_id, request.id);
            assert_eq!(data.library_id, library_id);
            assert_eq!(data.title_name, "Glass Harbor");
            assert_eq!(data.external_ids, request.external_ids);
            assert_eq!(
                data.requested_quality_profile_id.as_deref(),
                Some(crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID)
            );
            assert_eq!(
                data.requested_quality_profile_name.as_deref(),
                Some("1080P")
            );
            assert!(data.requested_monitor_type.is_none());
        }
        other => panic!("unexpected event payload: {other:?}"),
    }
}

#[tokio::test]
async fn pending_media_request_blocks_profile_removal_after_library_allowlist_changes() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.manager,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("allow 4k before request submission");
    let mut input = media_request_input(library_id.clone(), 9088);
    input.requested_quality_profile_id = Some("4k".to_string());
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("submit pending request with explicit profile");

    harness
        .app
        .update_library_settings(
            &harness.manager,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("library allowlist can subsequently remove the request profile");

    let error = harness
        .app
        .delete_quality_profile(&harness.manager, "4k")
        .await
        .expect_err("persisted pending request must remain a deletion reference");

    assert!(
        matches!(error, AppError::Validation(message) if message.contains("pending media request"))
    );
}

#[tokio::test]
async fn submit_media_request_auto_approves_for_requester_with_auto_approve_permission() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requester = library_permission_user(
        "auto-approved-requester",
        &library_id,
        &[scryer_domain::LibraryPermission::AutoApproveRequests],
    );

    let outcome = harness
        .app
        .submit_media_request(&requester, media_request_input(library_id.clone(), 9029))
        .await
        .expect("request submission should auto-approve");

    assert!(!outcome.request_id.is_empty());
    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1);
    let title_id = titles[0].id.clone();
    assert_eq!(titles[0].name, "Glass Harbor");
    assert_eq!(titles[0].library_id, library_id);
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.status, MediaRequestStatus::Approved);
    assert_eq!(request.created_title_id.as_deref(), Some(title_id.as_str()));
    assert_eq!(
        request.approved_quality_profile_id.as_deref(),
        Some(crate::BUILTIN_DEFAULT_QUALITY_PROFILE_ID),
        "unconfigured approvals snapshot the built-in default profile"
    );
    assert_eq!(
        request.approved_quality_profile_name.as_deref(),
        Some("1080P")
    );
    assert_eq!(
        request.resolved_by_user_id.as_deref(),
        Some(requester.id.as_str())
    );
    assert!(request.resolved_at.is_some());

    let events = harness.domain_events.events.lock().await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, DomainEventPayload::MediaRequestSubmitted(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, DomainEventPayload::MediaRequestApproved(_)))
    );
}

#[tokio::test]
async fn submit_media_request_uses_library_request_quality_profile_allowlist() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    let mut input = media_request_input(library_id.clone(), 9026);
    input.requested_quality_profile_id = Some("1080p".to_string());
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("allowlisted request profile should be accepted");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
    assert_eq!(
        requests[0].requested_quality_profile_name.as_deref(),
        Some("1080P")
    );
}

#[tokio::test]
async fn submit_media_request_rejects_profiles_outside_library_request_allowlist() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    let mut input = media_request_input(library_id, 9027);
    input.requested_quality_profile_id = Some("4k".to_string());
    let error = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("request profile outside allowlist should fail");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("not allowed")),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_defaults_missing_profile_to_library_request_default() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9028))
        .await
        .expect("missing profile should use request default");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
}

#[tokio::test]
async fn media_request_activity_is_visible_to_library_viewers() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9021))
        .await
        .expect("request submission should succeed");

    let viewer = library_permission_user(
        "request-activity-viewer",
        &library_id,
        &[scryer_domain::LibraryPermission::View],
    );
    let activities = harness
        .app
        .recent_activity(&viewer, 10, 0)
        .await
        .expect("request activity should be visible");

    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::SystemNotice);
    assert!(
        activities[0].message.contains("Requested 'Glass Harbor'"),
        "unexpected activity message: {}",
        activities[0].message
    );
}

#[tokio::test]
async fn submit_media_request_duplicate_same_user_creates_separate_submission_and_event() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9011);

    let first = harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    let second = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed opaquely");

    assert_ne!(first.request_id, second.request_id);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.requesters.len() == 1));
    let request_ids = requests
        .iter()
        .map(|request| request.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(request_ids.len(), 2);
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| matches!(event.payload, DomainEventPayload::MediaRequestSubmitted(_)))
    );
}

#[tokio::test]
async fn submit_media_request_second_user_creates_private_submission_without_exposing_prior_request()
 {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id.clone(), 9012);
    let second_user = library_permission_user(
        "requester-two",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );

    let first = harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    let second = harness
        .app
        .submit_media_request(&second_user, input)
        .await
        .expect("second request should attach opaquely");

    assert_ne!(first.request_id, second.request_id);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.requesters.len() == 1));
    let requester_ids = requests
        .iter()
        .flat_map(|request| request.requesters.iter().map(|entry| entry.user_id.clone()))
        .collect::<HashSet<_>>();
    assert!(requester_ids.contains(&harness.user.id));
    assert!(requester_ids.contains(&second_user.id));
    assert_eq!(requester_ids.len(), 2);
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    let request_ids = events
        .iter()
        .map(|event| match &event.payload {
            DomainEventPayload::MediaRequestSubmitted(data) => data.request_id.clone(),
            other => panic!("unexpected event payload: {other:?}"),
        })
        .collect::<HashSet<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(request_ids.len(), 2);
}

#[tokio::test]
async fn submit_media_request_accepts_search_correlation_id_without_tvdb() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9019);
    input.external_ids = vec![ExternalId {
        source: "imdb".to_string(),
        value: "tt7654321".to_string(),
    }];

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("imdb-backed search request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].external_ids,
        vec![ExternalId {
            source: "imdb".to_string(),
            value: "tt7654321".to_string(),
        }]
    );
}

#[tokio::test]
async fn submit_media_request_enriches_movie_external_ids_from_metadata() {
    let harness = bootstrap_media_request_app();
    let tvdb_id = 91_001;
    let mut movie = make_movie_metadata(tvdb_id, "External Movie");
    movie.imdb_id = "tt9100100".to_string();
    movie.tmdb_id = Some(810_010);
    movie.anidb_id = Some(710_010);
    movie.overview = "Hydrated overview".to_string();
    movie.ratings = TitleRatingSummary {
        rating: Some(8.6),
        rating_sources: vec!["imdb".to_string(), "tmdb".to_string()],
        external_ratings: vec![TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(8.6),
            score: None,
            normalized: 8.6,
            votes: Some(12_345),
            url: "https://www.imdb.com/title/tt9100100/".to_string(),
        }],
    };
    let expected_rating_summary = movie.ratings.clone();
    let hydration_result =
        crate::catalog::facets::handler::movie_to_hydration_result(movie.clone(), "eng");
    let expected_external_ids = normalize_media_request_external_ids(
        crate::catalog::facets::handler::external_ids_from_hydration_metadata(
            vec![external_id("TVDB", tvdb_id)],
            &hydration_result.metadata_update,
        ),
    )
    .expect("hydration external ids should normalize");
    let app = harness.app.with_test_overrides(|builder| {
        builder.with_metadata_gateway(Arc::new(MediaRequestMetadataGateway {
            movies: HashMap::from([(tvdb_id, movie)]),
            ..Default::default()
        }))
    });
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, tvdb_id);
    input.title = "External Movie".to_string();
    input.external_ids = vec![external_id("TVDB", tvdb_id)];
    input.overview = Some("Submitted overview".to_string());

    app.submit_media_request(&harness.user, input)
        .await
        .expect("TVDB-backed movie request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].overview.as_deref(), Some("Hydrated overview"));
    assert_eq!(requests[0].rating_summary, expected_rating_summary);
    assert_eq!(requests[0].external_ids, expected_external_ids);
    assert_external_ids(
        &requests[0].external_ids,
        &[
            ("anidb", "710010"),
            ("imdb", "tt9100100"),
            ("tmdb", "810010"),
            ("tvdb", "91001"),
        ],
    );
}

#[tokio::test]
async fn submit_media_request_enriches_tmdb_only_movie_from_title_ref() {
    let harness = bootstrap_media_request_app();
    let tvdb_id = 91_021;
    let tmdb_id = 810_021;
    let mut movie = make_movie_metadata(tvdb_id, "TMDB Request Movie");
    movie.smg_id = Some(1_810_021);
    movie.tmdb_id = Some(tmdb_id);
    movie.imdb_id = "tt8100021".to_string();
    let app = harness.app.with_test_overrides(|builder| {
        builder.with_metadata_gateway(Arc::new(MediaRequestMetadataGateway {
            movies: HashMap::from([(tvdb_id, movie)]),
            ..Default::default()
        }))
    });
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, tvdb_id);
    input.title = "TMDB Request Movie".to_string();
    input.external_ids = vec![external_id("tmdb", tmdb_id)];

    app.submit_media_request(&harness.user, input)
        .await
        .expect("TMDB-only movie request should enrich");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_external_ids(
        &requests[0].external_ids,
        &[
            ("imdb", "tt8100021"),
            ("smg", "1810021"),
            ("tmdb", "810021"),
            ("tvdb", "91021"),
        ],
    );
}

#[tokio::test]
async fn submit_media_request_enriches_anime_external_ids_from_hydration_metadata() {
    let harness = bootstrap_media_request_app();
    let tvdb_id = 91_101;
    let mut series = make_series_metadata(tvdb_id, "Mapped Anime");
    series.anime_mappings = vec![AnimeMapping {
        mal_id: Some(111_001),
        mal_dub_id: None,
        anilist_id: Some(222_002),
        anidb_id: Some(333_003),
        kitsu_id: Some(444_004),
        simkl_id: Some(555_005),
        thetvdb_id: Some(tvdb_id),
        themoviedb_id: Some(666_006),
        imdb_id: Some(777_007),
        trakt_id: Some(888_008),
        alt_tvdb_id: None,
        thetvdb_season: None,
        thetvdb_part: None,
        score: None,
        anime_media_type: "tv".to_string(),
        global_media_type: "series".to_string(),
        status: "current".to_string(),
        mapping_type: "default".to_string(),
        episode_mappings: Vec::new(),
    }];
    let hydration_result =
        crate::catalog::facets::handler::series_to_hydration_result(series.clone(), "eng");
    let expected_external_ids = normalize_media_request_external_ids(
        crate::catalog::facets::handler::external_ids_from_hydration_metadata(
            vec![external_id("tvdb", tvdb_id)],
            &hydration_result.metadata_update,
        ),
    )
    .expect("hydration external ids should normalize");
    let app = harness.app.with_test_overrides(|builder| {
        builder.with_metadata_gateway(Arc::new(MediaRequestMetadataGateway {
            series: HashMap::from([(tvdb_id, series)]),
            ..Default::default()
        }))
    });
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let mut input = media_request_input(library_id, tvdb_id);
    input.facet = MediaFacet::Anime;
    input.title = "Mapped Anime".to_string();
    input.external_ids = vec![external_id("tvdb", tvdb_id)];

    app.submit_media_request(&harness.user, input)
        .await
        .expect("TVDB-backed anime request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].external_ids, expected_external_ids);
    assert_external_ids(
        &requests[0].external_ids,
        &[
            ("anidb", "333003"),
            ("anilist", "222002"),
            ("imdb", "tt777007"),
            ("kitsu", "444004"),
            ("mal", "111001"),
            ("simkl", "555005"),
            ("tmdb", "666006"),
            ("trakt", "888008"),
            ("tvdb", "91101"),
        ],
    );
}

#[tokio::test]
async fn submit_media_request_keeps_original_ids_when_metadata_enrichment_fails() {
    let harness = bootstrap_media_request_app();
    let app = harness.app.with_test_overrides(|builder| {
        builder.with_metadata_gateway(Arc::new(MediaRequestMetadataGateway {
            fail_detail: true,
            ..Default::default()
        }))
    });
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 91_201);
    input.external_ids = vec![external_id("TVDB", 91_201)];
    input.overview = Some("Submitted overview".to_string());
    let submitted_rating_summary = TitleRatingSummary {
        rating: Some(7.4),
        rating_sources: vec!["tmdb".to_string()],
        external_ratings: vec![TitleExternalRating {
            source: "tmdb".to_string(),
            value: Some(7.4),
            score: None,
            normalized: 7.4,
            votes: Some(9_876),
            url: "https://www.themoviedb.org/movie/91201".to_string(),
        }],
    };
    input.rating_summary = submitted_rating_summary.clone();

    app.submit_media_request(&harness.user, input)
        .await
        .expect("metadata failure should fail open");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].overview.as_deref(), Some("Submitted overview"));
    assert_eq!(requests[0].rating_summary, submitted_rating_summary);
    assert_external_ids(&requests[0].external_ids, &[("tvdb", "91201")]);
}

#[tokio::test]
async fn submit_media_request_checks_existing_titles_against_enriched_ids() {
    let harness = bootstrap_media_request_app();
    let tvdb_id = 91_301;
    let tmdb_id = 813_010;
    let mut movie = make_movie_metadata(tvdb_id, "Existing Via Tmdb");
    movie.tmdb_id = Some(tmdb_id);
    let app = harness.app.with_test_overrides(|builder| {
        builder.with_metadata_gateway(Arc::new(MediaRequestMetadataGateway {
            movies: HashMap::from([(tvdb_id, movie)]),
            ..Default::default()
        }))
    });
    let mut existing = make_due_hydration_title("existing-tmdb", MediaFacet::Movie, tvdb_id);
    existing.external_ids = vec![external_id("tmdb", tmdb_id)];
    harness.titles.store.lock().await.push(existing);

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, tvdb_id);
    input.external_ids = vec![external_id("tvdb", tvdb_id)];

    let error = app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("enriched tmdb identity should block existing title request");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("already exists")),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_rejects_ids_that_cannot_correlate_to_smg_search() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9020);
    input.external_ids = vec![ExternalId {
        source: "unknown".to_string(),
        value: "opaque".to_string(),
    }];

    let error = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("unsupported identity should fail");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("searchable SMG identifier")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn submit_media_request_allows_same_identity_in_different_libraries() {
    let harness = bootstrap_media_request_app();
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let alternate_library_id = "movie-library-alt".to_string();
    harness
        .libraries
        .libraries
        .lock()
        .await
        .push(custom_movie_library(&alternate_library_id, "Movie Alt"));

    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(default_library_id.clone(), 9013),
        )
        .await
        .expect("default library request should succeed");
    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(alternate_library_id.clone(), 9013),
        )
        .await
        .expect("alternate library request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .any(|request| request.library_id == default_library_id)
    );
    assert!(
        requests
            .iter()
            .any(|request| request.library_id == alternate_library_id)
    );
}

#[tokio::test]
async fn submit_media_request_blocks_existing_title_in_target_library() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "existing-movie",
            MediaFacet::Movie,
            9014,
        ));

    let error = harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9014))
        .await
        .expect_err("existing title identity should block request");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("already exists")),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
    assert!(harness.domain_events.events.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_requires_request_permission_and_matching_facet() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requestless_user = library_permission_user("viewer", &library_id, &[]);

    let permission_error = harness
        .app
        .submit_media_request(
            &requestless_user,
            media_request_input(library_id.clone(), 9015),
        )
        .await
        .expect_err("request permission should be required");
    assert!(
        matches!(permission_error, AppError::Unauthorized(_)),
        "unexpected permission error: {permission_error:?}"
    );

    let mut mismatched = media_request_input(library_id, 9016);
    mismatched.facet = MediaFacet::Series;
    let facet_error = harness
        .app
        .submit_media_request(&harness.user, mismatched)
        .await
        .expect_err("facet mismatch should fail");
    assert!(
        matches!(facet_error, AppError::Validation(ref message) if message.contains("facet")),
        "unexpected facet error: {facet_error:?}"
    );
}

#[tokio::test]
async fn submit_media_request_rejects_manage_titles_shadowed_request_permission() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let manager = library_permission_user(
        "manage-title-requester",
        &library_id,
        &[
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::Request,
            scryer_domain::LibraryPermission::AutoApproveRequests,
        ],
    );

    let error = harness
        .app
        .submit_media_request(&manager, media_request_input(library_id, 9037))
        .await
        .expect_err("manage titles should suppress personal request submission");

    assert!(
        matches!(error, AppError::Unauthorized(_)),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_allows_mixed_manage_and_request_libraries() {
    let harness = bootstrap_media_request_app();
    let managed_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let request_library_id = "movie-library-requestable".to_string();
    harness
        .libraries
        .libraries
        .lock()
        .await
        .push(custom_movie_library(
            &request_library_id,
            "Movie Requestable",
        ));
    let actor = library_permission_user_with_grants(
        "mixed-requester",
        &[
            (
                managed_library_id.as_str(),
                &[scryer_domain::LibraryPermission::ManageTitles][..],
            ),
            (
                request_library_id.as_str(),
                &[scryer_domain::LibraryPermission::Request][..],
            ),
        ],
    );

    harness
        .app
        .submit_media_request(
            &actor,
            media_request_input(request_library_id.clone(), 9038),
        )
        .await
        .expect("request-only library should accept submission");
    let managed_error = harness
        .app
        .submit_media_request(&actor, media_request_input(managed_library_id, 9039))
        .await
        .expect_err("managed library should reject personal submission");

    assert!(
        matches!(managed_error, AppError::Unauthorized(_)),
        "unexpected managed-library error: {managed_error:?}"
    );
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].library_id, request_library_id);
    drop(requests);

    let requestable_libraries = harness
        .app
        .list_libraries_for_permission(
            &actor,
            Some(MediaFacet::Movie),
            scryer_domain::LibraryPermission::Request,
        )
        .await
        .expect("requestable libraries should load");
    assert_eq!(requestable_libraries.len(), 1);
    assert_eq!(requestable_libraries[0].id, request_library_id);
}

#[tokio::test]
async fn list_media_requests_filters_by_facet_and_manageable_libraries() {
    let harness = bootstrap_media_request_app();
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let alternate_library_id = "movie-library-queue".to_string();
    harness
        .libraries
        .libraries
        .lock()
        .await
        .push(custom_movie_library(&alternate_library_id, "Movie Queue"));

    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(default_library_id.clone(), 9017),
        )
        .await
        .expect("default library request should succeed");
    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(alternate_library_id.clone(), 9018),
        )
        .await
        .expect("alternate library request should succeed");

    let queue_manager = library_permission_user(
        "queue-manager",
        &alternate_library_id,
        &[scryer_domain::LibraryPermission::ManageTitles],
    );
    let requests = harness
        .app
        .list_media_requests(
            &queue_manager,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: Some(vec![default_library_id, alternate_library_id.clone()]),
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("request list should load");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].library_id, alternate_library_id);
    assert_eq!(requests[0].requesters.len(), 1);
}

#[tokio::test]
async fn list_my_media_requests_filters_to_requester_owned_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let second_user = library_permission_user(
        "requester-owned-list",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9031))
        .await
        .expect("first user's request should succeed");
    harness
        .app
        .submit_media_request(&second_user, media_request_input(library_id, 9032))
        .await
        .expect("second user's request should succeed");

    let requests = harness
        .app
        .list_my_media_requests(
            &second_user,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: None,
                status: None,
            },
        )
        .await
        .expect("own requests should load");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].created_by_user_id, second_user.id);
    assert!(
        requests[0]
            .requesters
            .iter()
            .any(|requester| requester.user_id == second_user.id)
    );
}

#[tokio::test]
async fn request_only_user_can_list_submitted_bluey_series_request() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let requester = library_permission_user(
        "bluey-requester-owned-list",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );
    let mut input = media_request_input(library_id.clone(), 353546);
    input.facet = MediaFacet::Series;
    input.title = "Bluey".to_string();
    input.sort_title = Some("Bluey".to_string());
    input.slug = Some("bluey".to_string());
    input.year = Some(2018);
    input.content_status = Some("Continuing".to_string());
    input.requested_monitor_type = Some("allEpisodes".to_string());
    input.external_ids = vec![
        ExternalId {
            source: "tvdb".to_string(),
            value: "353546".to_string(),
        },
        ExternalId {
            source: "imdb".to_string(),
            value: "tt7678620".to_string(),
        },
    ];

    harness
        .app
        .submit_media_request(&requester, input)
        .await
        .expect("request-only Bluey submission should succeed");

    let requests = harness
        .app
        .list_my_media_requests(
            &requester,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Series),
                library_ids: None,
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("requester should list own Bluey request");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].library_id, library_id);
    assert_eq!(requests[0].facet, MediaFacet::Series);
    assert_eq!(requests[0].title, "Bluey");
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    assert_eq!(requests[0].created_by_user_id, requester.id);
    assert_eq!(
        requests[0].requested_monitor_type.as_deref(),
        Some("allepisodes")
    );
    assert!(
        requests[0]
            .requesters
            .iter()
            .any(|entry| entry.user_id == requester.id)
    );
}

#[tokio::test]
async fn requester_can_update_pending_request_preferences() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");
    let mut input = media_request_input(library_id, 9033);
    input.facet = MediaFacet::Series;

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();

    let updated = harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id,
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: Some("allEpisodes".to_string()),
            },
        )
        .await
        .expect("requester should update pending request");

    assert_eq!(
        updated.requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
    assert_eq!(
        updated.requested_quality_profile_name.as_deref(),
        Some("1080P")
    );
    assert_eq!(
        updated.requested_monitor_type.as_deref(),
        Some("allepisodes")
    );

    let events = harness.domain_events.events.lock().await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, DomainEventPayload::MediaRequestUpdated(_)))
    );
}

#[tokio::test]
async fn requester_can_cancel_pending_request_without_resolving_overlapping_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9034);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();

    let canceled = harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect("requester should cancel pending request");

    assert_eq!(canceled, 1);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].status, MediaRequestStatus::Canceled);
    assert_eq!(requests[1].status, MediaRequestStatus::Pending);
}

#[tokio::test]
async fn requester_cannot_update_or_cancel_after_manager_resolution() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9035))
        .await
        .expect("request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    harness
        .app
        .dismiss_media_request(&harness.manager, &request_id)
        .await
        .expect("manager should reject request");

    let update_error = harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id: request_id.clone(),
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: None,
            },
        )
        .await
        .expect_err("resolved request cannot be updated");
    assert!(
        matches!(update_error, AppError::Validation(ref message) if message.contains("no longer pending")),
        "unexpected update error: {update_error:?}"
    );

    let cancel_error = harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect_err("resolved request cannot be canceled");
    assert!(
        matches!(cancel_error, AppError::Validation(ref message) if message.contains("no longer pending")),
        "unexpected cancel error: {cancel_error:?}"
    );
}

#[tokio::test]
async fn approve_media_request_creates_title_and_resolves_overlapping_pending_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id.clone(), 9022);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    let outcome = harness
        .app
        .approve_media_request(&harness.manager, &request_id, "1080p", None)
        .await
        .expect("approval should create the title");

    assert!(outcome.search_error.is_none());
    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1);
    let title = &titles[0];
    assert_eq!(outcome.title_id, title.id);
    assert_eq!(title.name, "Glass Harbor");
    assert_eq!(title.library_id, library_id);
    assert_eq!(title.year, Some(2026));
    assert_eq!(
        title.poster_url.as_deref(),
        Some("https://example.com/9022.jpg")
    );
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:quality-profile:1080p")
    );
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.status == MediaRequestStatus::Approved)
    );
    assert!(requests.iter().all(|request| {
        request.created_title_id.as_deref() == Some(outcome.title_id.as_str())
            && request.approved_quality_profile_id.as_deref() == Some("1080p")
            && request.approved_quality_profile_name.as_deref() == Some("1080P")
            && request.resolved_by_user_id.as_deref() == Some(harness.manager.id.as_str())
            && request.resolved_at.is_some()
    }));
}

#[tokio::test]
async fn approve_media_request_accepts_legacy_case_profile_id_and_persists_canonical_tag() {
    let harness = bootstrap_media_request_app();
    harness
        .quality_profiles
        .set_profiles(vec![test_quality_profile("wizard-SERIES")])
        .await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    // The catalog holds only the wizard profile, so the request allowlist must
    // name it explicitly: the implicit default allowlist follows the built-in
    // default profile, which is absent from this deliberately reduced catalog.
    harness
        .app
        .update_library_settings(
            &harness.manager,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["wizard-SERIES".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("allow the wizard profile for requests");
    let mut input = media_request_input(library_id, 9024);
    input.facet = MediaFacet::Series;
    input.requested_quality_profile_id = Some("wizard-SERIES".to_string());

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("request should accept the configured profile");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    let outcome = harness
        .app
        .approve_media_request(&harness.manager, &request_id, "wizard-series", None)
        .await
        .expect("approval should resolve profile ids case-insensitively");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == outcome.title_id)
        .expect("approved title should exist");
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:quality-profile:wizard-SERIES")
    );
}

#[tokio::test]
async fn approve_series_media_request_applies_requested_monitor_type() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut input = media_request_input(library_id.clone(), 9030);
    input.facet = MediaFacet::Series;
    input.requested_monitor_type = Some("missingAndFutureEpisodes".to_string());

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("series request should succeed");

    let request = harness.media_requests.requests.lock().await[0].clone();
    assert_eq!(
        request.requested_monitor_type.as_deref(),
        Some("missingandfutureepisodes")
    );

    let outcome = harness
        .app
        .approve_media_request(&harness.manager, &request.id, "1080p", None)
        .await
        .expect("approval should create the series title");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == outcome.title_id)
        .expect("approved title should be stored");
    assert!(title.monitored);
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:monitor-type:missingandfutureepisodes")
    );
}

#[tokio::test]
async fn approve_series_media_request_can_override_requested_monitor_type() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut input = media_request_input(library_id, 9036);
    input.facet = MediaFacet::Series;
    input.requested_monitor_type = Some("allEpisodes".to_string());

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("series request should succeed");

    let request = harness.media_requests.requests.lock().await[0].clone();
    let outcome = harness
        .app
        .approve_media_request(
            &harness.manager,
            &request.id,
            "1080p",
            Some("none".to_string()),
        )
        .await
        .expect("approval should create the series title");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == outcome.title_id)
        .expect("approved title should be stored");
    assert!(!title.monitored);
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:monitor-type:none")
    );
}

#[tokio::test]
async fn dismiss_media_request_resolves_overlapping_pending_requests_without_title() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9023);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    let mut notification_wakes = harness
        .app
        .runtime
        .events
        .notification_event_broadcast
        .subscribe();
    let removed = harness
        .app
        .dismiss_media_request(&harness.manager, &request_id)
        .await
        .expect("dismiss should remove the request group");

    assert_eq!(removed, 2);
    timeout(Duration::from_millis(250), notification_wakes.recv())
        .await
        .expect("rejected media request should wake notifications")
        .expect("notification wake sender should remain open");
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.status == MediaRequestStatus::Rejected)
    );
    assert!(requests.iter().all(|request| {
        request.created_title_id.is_none()
            && request.approved_quality_profile_id.is_none()
            && request.resolved_by_user_id.as_deref() == Some(harness.manager.id.as_str())
            && request.resolved_at.is_some()
    }));
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
}

#[tokio::test]
async fn pending_media_request_counts_deduplicate_duplicate_identity_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9024);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let counts = harness
        .app
        .pending_media_request_counts(&harness.manager)
        .await
        .expect("request counts should load");

    assert_eq!(counts.movie, 1);
    assert_eq!(counts.series, 0);
    assert_eq!(counts.anime, 0);
}

#[tokio::test]
async fn media_request_admin_surfaces_require_manage_titles_library_permission() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9025))
        .await
        .expect("request submission should succeed");

    let config_admin = test_user_with_app_permissions(
        "catalog-config-admin",
        AppPermissionMask::MANAGE_CATALOG_SETTINGS,
    );

    let listed = harness
        .app
        .list_media_requests(
            &config_admin,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: None,
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("request list should load");
    assert!(listed.is_empty());

    let counts = harness
        .app
        .pending_media_request_counts(&config_admin)
        .await
        .expect("request counts should load");
    assert_eq!(counts.movie, 0);
    assert_eq!(counts.series, 0);
    assert_eq!(counts.anime, 0);
    assert!(
        !harness
            .app
            .can_manage_media_requests(&config_admin)
            .await
            .expect("permission check should load")
    );

    let events = harness
        .app
        .list_media_request_lifecycle_events_for_manager(&config_admin, 0, 10)
        .await
        .expect("request event list should load");
    assert!(events.is_empty());

    let request_manager = library_permission_user(
        "request-manager",
        &library_id,
        &[scryer_domain::LibraryPermission::ManageTitles],
    );
    let manager_events = harness
        .app
        .list_media_request_lifecycle_events_for_manager(&request_manager, 0, 10)
        .await
        .expect("manager request events should load");
    assert_eq!(manager_events.len(), 1);
}
