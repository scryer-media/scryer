use super::*;

#[tokio::test]
async fn graphql_activity_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ activityEvents { id kind severity } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["activityEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleHistory(filter: { limit: 10 }) { items { id eventType sourceTitle } totalCount } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 0);
    assert!(body["data"]["titleHistory"]["items"].is_array());
}

#[tokio::test]
async fn graphql_title_history_rejects_unsupported_event_type_filters() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleHistory(filter: { eventTypes: [DOWNLOAD_COMPLETED], limit: 10 }) { totalCount } }"#,
        json!({}),
    )
    .await;

    let errors = body["errors"].as_array().expect("graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(message.contains("unsupported title history event type `download_completed`"));
    assert!(message.contains("grabbed"));
    assert!(message.contains("rematched"));
}

#[tokio::test]
async fn graphql_title_history_includes_download_ignored_events() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Ignored Download History Fixture",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::User,
            actor_user_id: Some("user-1".to_string()),
            actor_display_name: "Fixture User".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Movie),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::DownloadIgnored(scryer_domain::DownloadIgnoredEventData {
                title: Some(TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                }),
                download_client_item_id: "ignored-job-1".to_string(),
                client_id: Some("client-1".to_string()),
                client_type: Some("nzbget".to_string()),
                source_provider: Some("Fixture Indexer".to_string()),
                source_title: Some("Fixture.Release.2026.1080p.WEB-DL".to_string()),
            }),
        })
        .await
        .expect("append download ignored event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], eventTypes: [DOWNLOAD_IGNORED], limit: 10 }) {
            totalCount
            items { eventType downloadId sourceTitle displayTitle sourceProvider sourceHint }
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 1);
    let record = &body["data"]["titleHistory"]["items"][0];
    assert_eq!(record["eventType"], "download_ignored");
    assert_eq!(record["downloadId"], "ignored-job-1");
    assert_eq!(record["sourceTitle"], "Fixture.Release.2026.1080p.WEB-DL");
    assert_eq!(record["displayTitle"], "Fixture.Release.2026.1080p.WEB-DL");
    assert_eq!(record["sourceProvider"], "Fixture Indexer");
    assert_eq!(record["sourceHint"], "Fixture Indexer");
}

#[tokio::test]
async fn graphql_title_history_includes_download_failed_and_blocklisted_events() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Download Outcome History Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("download-outcome-episode-1".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let title_context = TitleContextSnapshot {
        title_name: title.name.clone(),
        facet: title.facet,
        external_ids: DomainExternalIds::default(),
        poster_url: title.poster_url.clone(),
        year: title.year,
    };

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                title: Some(title_context.clone()),
                source_title: Some("Fixture.S01.1080p.WEB-DL".to_string()),
                source_hint: Some("https://indexer.example/release".to_string()),
                download_id: Some("job-123".to_string()),
                client_id: Some("client-1".to_string()),
                client_name: Some("Primary".to_string()),
                client_type: Some("nzbget".to_string()),
                quality: Some("1080P".to_string()),
                reason: Some(
                    "download failed for 'Fixture.S01.1080p.WEB-DL': CORRUPT ARCHIVE".to_string(),
                ),
                episode_ids: vec![episode.id.clone()],
                collection_id: Some(collection.id.clone()),
            }),
        })
        .await
        .expect("append download failed event");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ReleaseBlocklisted(ReleaseBlocklistedEventData {
                title: Some(title_context),
                source_title: Some("Fixture.S01.1080p.WEB-DL".to_string()),
                source_hint: Some("https://indexer.example/release".to_string()),
                download_id: Some("job-123".to_string()),
                client_id: Some("client-1".to_string()),
                client_name: Some("Primary".to_string()),
                client_type: Some("nzbget".to_string()),
                quality: Some("1080P".to_string()),
                reason: Some("download client failure: CORRUPT ARCHIVE".to_string()),
                episode_ids: vec![episode.id.clone()],
                collection_id: Some(collection.id.clone()),
            }),
        })
        .await
        .expect("append release blocklisted event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], eventTypes: [DOWNLOAD_FAILED, BLOCKLISTED], limit: 10 }) {
            totalCount
            items {
              eventType
              sourceTitle
              downloadId
              clientId
              clientName
              failureReason
              blocklistReason
              episodeId
              collectionId
            }
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["titleHistory"]["totalCount"], 2);
    let records = body["data"]["titleHistory"]["items"]
        .as_array()
        .expect("title history items array");

    let download_failed = records
        .iter()
        .find(|record| record["eventType"] == "download_failed")
        .expect("download_failed record");
    assert_eq!(download_failed["sourceTitle"], "Fixture.S01.1080p.WEB-DL");
    assert_eq!(download_failed["downloadId"], "job-123");
    assert_eq!(download_failed["clientId"], "client-1");
    assert_eq!(download_failed["clientName"], "Primary");
    assert_eq!(download_failed["episodeId"], episode.id);
    assert_eq!(download_failed["collectionId"], collection.id);
    assert!(
        download_failed["failureReason"]
            .as_str()
            .is_some_and(|value| value.contains("CORRUPT ARCHIVE"))
    );

    let blocklisted = records
        .iter()
        .find(|record| record["eventType"] == "blocklisted")
        .expect("blocklisted record");
    assert_eq!(blocklisted["downloadId"], "job-123");
    assert_eq!(blocklisted["clientId"], "client-1");
    assert_eq!(blocklisted["clientName"], "Primary");
    assert_eq!(blocklisted["episodeId"], episode.id);
    assert_eq!(blocklisted["collectionId"], collection.id);
    assert_eq!(
        blocklisted["blocklistReason"],
        "download client failure: CORRUPT ARCHIVE"
    );
}

#[tokio::test]
async fn graphql_title_history_filters_by_episode_id() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Episode Scoped History Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode_one = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("episode-history-1".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create first episode");
    let episode_two = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode Two".to_string()),
            air_date: Some("2024-01-08".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("2".to_string()),
            overview: None,
            tvdb_id: Some("episode-history-2".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create second episode");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                },
                media_updates: vec![
                    MediaPathUpdate {
                        path: "/library/Episode Scoped History Fixture/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                    MediaPathUpdate {
                        path: "/library/Episode Scoped History Fixture/S01E02.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                ],
                imported_count: 2,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: None,
                source_path: None,
                dest_path: None,
                quality: None,
                episode_ids: vec![episode_one.id.clone(), episode_two.id.clone()],
                size_bytes: None,
            }),
        })
        .await
        .expect("append import completed event");

    let scanned_path = "/library/Episode Scoped History Fixture/S01E01.mkv";
    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::MediaFileAnalyzed(MediaFileAnalyzedEventData {
                title: TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                },
                media_updates: vec![MediaPathUpdate {
                    path: scanned_path.to_string(),
                    update_type: MediaUpdateType::Modified,
                }],
                file_id: "scanned-file-1".to_string(),
                analysis_status: "scanned".to_string(),
                episode_ids: vec![episode_one.id.clone()],
            }),
        })
        .await
        .expect("append media file analyzed event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!, $episodeId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], episodeId: $episodeId, limit: 10 }) {
            totalCount
            items {
              eventType
              episodeId
              sourceTitle
            }
          }
        }
        "#,
        json!({ "titleId": title.id, "episodeId": episode_one.id }),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["titleHistory"]["totalCount"], 2);
    let records = body["data"]["titleHistory"]["items"]
        .as_array()
        .expect("title history items array");
    assert_eq!(records.len(), 2);
    let scanned = records
        .iter()
        .find(|record| record["eventType"] == "scanned")
        .expect("scanned history event");
    assert_eq!(scanned["episodeId"], episode_one.id);
    assert_eq!(scanned["sourceTitle"], scanned_path);
}

#[tokio::test]
async fn graphql_title_history_filters_skipped_import_by_episode_id() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Skipped Episode History Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("The Skipped One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("skipped-episode-history-1".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ImportRejected(scryer_domain::ImportRejectedEventData {
                title: Some(TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                }),
                status: scryer_domain::ImportStatus::Skipped,
                import_id: Some("skipped-episode-import".to_string()),
                source_system: Some("weaver".to_string()),
                source_ref: Some("10028".to_string()),
                source_title: Some("Skipped Episode Release".to_string()),
                source_path: Some(
                    "/weaver-downloads/complete/anime/Skipped Episode Release#10028".to_string(),
                ),
                dest_path: None,
                quality: None,
                reason: Some("duplicate file already exists".to_string()),
                skip_reason: Some(scryer_domain::ImportSkipReason::DuplicateFile),
                episode_ids: vec![episode.id.clone()],
            }),
        })
        .await
        .expect("append skipped import event");

    let body = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!, $episodeId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], episodeId: $episodeId, limit: 10 }) {
            totalCount
            items {
              eventType
              episodeId
              episodeIds
              failureReason
              skipReason
            }
          }
        }
        "#,
        json!({ "titleId": title.id, "episodeId": episode.id }),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["titleHistory"]["totalCount"], 1);
    let records = body["data"]["titleHistory"]["items"]
        .as_array()
        .expect("title history items array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["eventType"], "import_skipped");
    assert_eq!(records[0]["episodeId"], episode.id);
    assert_eq!(records[0]["episodeIds"], json!([episode.id.clone()]));
    assert_eq!(records[0]["failureReason"], "duplicate file already exists");
    assert_eq!(records[0]["skipReason"], "duplicate_file");
}

#[tokio::test]
async fn graphql_episode_history_omits_ambiguous_source_path_for_multi_file_events() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "History Projection Fixture",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create collection");
    let episode_one = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode One".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("history-episode-1".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create first episode");
    let episode_two = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode Two".to_string()),
            air_date: Some("2024-01-08".to_string()),
            duration_seconds: Some(1500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("2".to_string()),
            overview: None,
            tvdb_id: Some("history-episode-2".to_string()),
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create second episode");

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_kind: DomainEventActorKind::System,
            actor_user_id: None,
            actor_display_name: "System".to_string(),
            title_id: Some(title.id.clone()),
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::Title {
                title_id: title.id.clone(),
            },
            payload: DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: TitleContextSnapshot {
                    title_name: title.name.clone(),
                    facet: title.facet,
                    external_ids: DomainExternalIds::default(),
                    poster_url: title.poster_url.clone(),
                    year: title.year,
                },
                media_updates: vec![
                    MediaPathUpdate {
                        path: "/library/History Projection Fixture/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                    MediaPathUpdate {
                        path: "/library/History Projection Fixture/S01E02.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    },
                ],
                imported_count: 2,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: None,
                source_path: None,
                dest_path: None,
                quality: None,
                episode_ids: vec![episode_one.id.clone(), episode_two.id.clone()],
                size_bytes: None,
            }),
        })
        .await
        .expect("append import completed event");

    let body = gql(
        &ctx,
        r#"
        query EpisodeHistory($episodeId: ID!) {
          titleHistory(filter: { episodeId: $episodeId, limit: 10 }) {
            items {
              eventType
              sourceTitle
            }
          }
        }
        "#,
        json!({ "episodeId": episode_one.id }),
    )
    .await;
    assert_no_errors(&body);

    let records = body["data"]["titleHistory"]["items"]
        .as_array()
        .expect("episode history array");
    let imported = records
        .iter()
        .find(|record| record["eventType"] == "imported")
        .expect("imported event");
    assert!(imported["sourceTitle"].is_null());
}

#[tokio::test]
async fn graphql_search_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_titles.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: MediaFacetValue!) {
            searchMetadata(query: $query, type: $type) {
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "MOVIE" }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "Test Movie Title");
}
