use super::*;

#[test]
fn graphql_fix_title_match_movie_updates_identity_and_history() {
    run_large_stack_graphql_test(
        "graphql_fix_title_match_movie_updates_identity_and_history",
        || async {
            let ctx = TestContext::new().await;
            mount_smg_mocks(&ctx, "smg/titles_movie.json").await;

            let title = create_catalog_title(
                &ctx,
                "Broken Movie Match",
                MediaFacet::Movie,
                vec![
                    ExternalId {
                        source: "tvdb".to_string(),
                        value: "999".to_string(),
                    },
                    ExternalId {
                        source: "imdb".to_string(),
                        value: "tt0000999".to_string(),
                    },
                    ExternalId {
                        source: "tmdb".to_string(),
                        value: "4444".to_string(),
                    },
                ],
                vec!["scryer:quality-profile:4k".to_string()],
                true,
            )
            .await;

            let body = gql(
                &ctx,
                r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan { scanned }
            title {
              id
              name
              slug
              imdbId
              metadataFetchedAt
              tags
              externalIds { source value }
            }
          }
        }
        "#,
                json!({ "input": { "titleId": title.id, "smgId": 101 } }),
            )
            .await;
            assert_no_errors(&body);

            let payload = &body["data"]["fixTitleMatch"];
            assert_eq!(payload["hydrated"], true);
            assert_eq!(payload["warnings"], json!([]));
            assert!(payload["libraryScan"].is_null());
            assert_eq!(payload["title"]["name"], "Test Movie Title");
            assert_eq!(payload["title"]["slug"], "test-movie-title");
            assert_eq!(payload["title"]["imdbId"], "tt1234567");
            assert!(payload["title"]["metadataFetchedAt"].is_string());

            let tags = payload["title"]["tags"].as_array().expect("tags array");
            assert!(tags.contains(&json!("scryer:quality-profile:4k")));

            let external_ids = payload["title"]["externalIds"]
                .as_array()
                .expect("external ids array");
            assert!(
                external_ids
                    .iter()
                    .any(|value| { value["source"] == "smg" && value["value"] == "101" })
            );
            assert!(
                external_ids
                    .iter()
                    .any(|value| { value["source"] == "tvdb" && value["value"] == "123456" })
            );
            assert!(
                external_ids
                    .iter()
                    .any(|value| { value["source"] == "imdb" && value["value"] == "tt1234567" })
            );
            assert!(
                !external_ids
                    .iter()
                    .any(|value| { value["source"] == "tvdb" && value["value"] == "999" })
            );
            assert!(
                !external_ids
                    .iter()
                    .any(|value| { value["source"] == "tmdb" && value["value"] == "4444" })
            );
            assert!(
                external_ids
                    .iter()
                    .any(|value| { value["source"] == "tmdb" && value["value"] == "111" })
            );

            let events = gql(
                &ctx,
                r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], limit: 10 }) {
            items {
              eventType
              dataJson
            }
          }
        }
        "#,
                json!({ "titleId": title.id }),
            )
            .await;
            assert_no_errors(&events);
            let rematch_events = events["data"]["titleHistory"]["items"]
                .as_array()
                .expect("title events array");
            let rematch_event = rematch_events
                .iter()
                .find(|event| event["eventType"] == "rematched")
                .expect("rematched history event");
            let data_value = rematch_event["dataJson"].clone();
            assert!(data_value.is_object(), "dataJson should be a JSON object");
            assert_eq!(data_value["old_tvdb_id"], "999");
            assert_eq!(data_value["new_tvdb_id"], "123456");
            assert_eq!(data_value["smg_id"], 101);
            assert_eq!(data_value["tmdb_id"], 111);
            assert_eq!(data_value["source"], "manual");

            let history = gql(
                &ctx,
                r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], eventTypes: [REMATCHED], limit: 10 }) {
            totalCount
            items {
              eventType
            }
          }
        }
        "#,
                json!({ "titleId": title.id }),
            )
            .await;
            assert_no_errors(&history);
            assert_eq!(history["data"]["titleHistory"]["totalCount"], 1);
            assert_eq!(
                history["data"]["titleHistory"]["items"][0]["eventType"],
                "rematched"
            );

            let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
            assert!(
                activity_kinds
                    .iter()
                    .any(|kind| kind == "METADATA_HYDRATION_STARTED")
            );
            assert!(
                activity_kinds
                    .iter()
                    .any(|kind| kind == "METADATA_HYDRATION_COMPLETED")
            );
            assert!(activity_kinds.iter().any(|kind| kind == "TITLE_UPDATED"));
        },
    );
}

#[test]
fn graphql_fix_title_match_series_rebuilds_and_relinks_library() {
    run_large_stack_graphql_test(
        "graphql_fix_title_match_series_rebuilds_and_relinks_library",
        || async {
            let ctx = TestContext::new().await;
            mount_smg_mocks(&ctx, "smg/metadata_bulk_series.json").await;

            let media_root = tempfile::tempdir().expect("media root tempdir");
            configure_default_library_root(&ctx, MediaFacet::Series, media_root.path()).await;
            let title_name = "Broken Series Match";
            let title = create_catalog_title(
                &ctx,
                title_name,
                MediaFacet::Series,
                vec![
                    ExternalId {
                        source: "tvdb".to_string(),
                        value: "999".to_string(),
                    },
                    ExternalId {
                        source: "mal".to_string(),
                        value: "5555".to_string(),
                    },
                ],
                vec![
                    "scryer:season-folder:enabled".to_string(),
                    "scryer:anime-status:finished".to_string(),
                ],
                true,
            )
            .await;

            let old_collection = ctx
                .shows
                .create_collection(Collection {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    collection_type: scryer_domain::CollectionType::Season,
                    collection_index: "99".to_string(),
                    label: Some("Legacy Season".to_string()),
                    ordered_path: None,
                    narrative_order: None,
                    first_episode_number: Some("1".to_string()),
                    last_episode_number: Some("1".to_string()),
                    monitored: true,
                    created_at: chrono::Utc::now(),
                })
                .await
                .expect("create old collection");

            let old_episode = ctx
                .shows
                .create_episode(Episode {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    collection_id: Some(old_collection.id.clone()),
                    episode_type: scryer_domain::EpisodeType::Standard,
                    episode_number: Some("1".to_string()),
                    season_number: Some("99".to_string()),
                    episode_label: Some("S99E01".to_string()),
                    title: Some("Legacy Pilot".to_string()),
                    air_date: None,
                    duration_seconds: Some(1440),
                    has_multi_audio: false,
                    has_subtitle: false,
                    is_filler: false,
                    is_recap: false,
                    absolute_number: None,
                    overview: Some("Legacy episode".to_string()),
                    tvdb_id: Some("9999001".to_string()),
                    image_url: None,
                    monitored: true,
                    created_at: chrono::Utc::now(),
                })
                .await
                .expect("create old episode");

            let show_dir = media_root.path().join(title_name);
            let season_dir = show_dir.join("Season 01");
            std::fs::create_dir_all(&season_dir).expect("create season dir");
            set_title_folder_path(&ctx, &title.id, &show_dir).await;
            let file_path = season_dir.join("Broken.Series.Match.S01E01.1080p.WEB-DL.mkv");
            std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");
            let file_id = ctx
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: file_path.to_string_lossy().to_string(),
                    size_bytes: 1024,
                    quality_label: Some("1080p".to_string()),
                    ..Default::default()
                })
                .await
                .expect("insert media file");
            ctx.media_files
                .link_file_to_episode(&file_id, &old_episode.id)
                .await
                .expect("link file to legacy episode");

            let body = gql(
                &ctx,
                r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan {
              scanned
              matched
              imported
              skipped
              unmatched
            }
            title {
              id
              name
              tags
              externalIds { source value }
              collections {
                id
                collectionIndex
                episodes {
                  id
                  seasonNumber
                  episodeNumber
                  title
                }
              }
              mediaFiles {
                episodeId
                filePath
              }
            }
          }
        }
        "#,
                json!({ "input": { "titleId": title.id, "tvdbId": "345678" } }),
            )
            .await;
            assert_no_errors(&body);

            let payload = &body["data"]["fixTitleMatch"];
            assert_eq!(payload["hydrated"], true);
            assert_eq!(payload["warnings"], json!([]));
            assert_eq!(payload["title"]["name"], "Test Show Name");
            assert_eq!(payload["libraryScan"]["scanned"], 1);
            assert_eq!(payload["libraryScan"]["unmatched"], 0);

            let tags = payload["title"]["tags"].as_array().expect("tags array");
            assert!(tags.contains(&json!("scryer:season-folder:enabled")));
            assert!(
                !tags
                    .iter()
                    .filter_map(|tag| tag.as_str())
                    .any(|tag| tag.starts_with("scryer:root-folder:"))
            );
            assert!(!tags.contains(&json!("scryer:anime-status:finished")));

            let external_ids = payload["title"]["externalIds"]
                .as_array()
                .expect("external ids array");
            assert!(
                external_ids
                    .iter()
                    .any(|value| { value["source"] == "tvdb" && value["value"] == "345678" })
            );
            assert!(!external_ids.iter().any(|value| value["source"] == "mal"));

            let collections = payload["title"]["collections"]
                .as_array()
                .expect("collections array");
            assert_eq!(collections.len(), 2);
            assert!(
                !collections
                    .iter()
                    .any(|collection| collection["id"] == old_collection.id)
            );
            let rebuilt_episode_count: usize = collections
                .iter()
                .map(|collection| {
                    collection["episodes"]
                        .as_array()
                        .expect("episodes array")
                        .len()
                })
                .sum();
            assert_eq!(rebuilt_episode_count, 3);

            let media_files = payload["title"]["mediaFiles"]
                .as_array()
                .expect("media files array");
            assert_eq!(media_files.len(), 1);
            assert_eq!(
                media_files[0]["filePath"],
                file_path.to_string_lossy().to_string()
            );
            let relinked_episode_id = media_files[0]["episodeId"]
                .as_str()
                .expect("media file should relink to rebuilt episode");
            assert_ne!(relinked_episode_id, old_episode.id);

            let events = gql(
                &ctx,
                r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], limit: 10 }) {
            items {
              eventType
              dataJson
            }
          }
        }
        "#,
                json!({ "titleId": title.id }),
            )
            .await;
            assert_no_errors(&events);
            let rematch_events = events["data"]["titleHistory"]["items"]
                .as_array()
                .expect("title events array");
            let rematch_event = rematch_events
                .iter()
                .find(|event| event["eventType"] == "rematched")
                .expect("rematched history event");
            let data_value = rematch_event["dataJson"].clone();
            assert!(data_value.is_object(), "dataJson should be a JSON object");
            assert_eq!(data_value["old_tvdb_id"], "999");
            assert_eq!(data_value["new_tvdb_id"], "345678");

            let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
            assert!(
                activity_kinds
                    .iter()
                    .any(|kind| kind == "METADATA_HYDRATION_STARTED")
            );
            assert!(
                activity_kinds
                    .iter()
                    .any(|kind| kind == "METADATA_HYDRATION_COMPLETED")
            );
            assert!(activity_kinds.iter().any(|kind| kind == "TITLE_UPDATED"));
        },
    );
}

#[test]
fn graphql_fix_title_match_rejects_duplicate_target_tvdb_id() {
    run_large_stack_graphql_test(
        "graphql_fix_title_match_rejects_duplicate_target_tvdb_id",
        || async {
            let ctx = TestContext::new().await;
            let existing = create_catalog_title(
                &ctx,
                "Existing Correct Match",
                MediaFacet::Movie,
                vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                vec![],
                true,
            )
            .await;
            let broken = create_catalog_title(
                &ctx,
                "Broken Match",
                MediaFacet::Movie,
                vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "999".to_string(),
                }],
                vec![],
                true,
            )
            .await;
            let existing_name = existing.name;
            let broken_id = broken.id;

            let body = gql(
                &ctx,
                r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            title { id }
          }
        }
        "#,
                json!({ "input": { "titleId": broken_id, "tvdbId": "123456" } }),
            )
            .await;

            assert!(
                body.get("errors").is_some(),
                "expected graphql errors: {body}"
            );
            let message = body["errors"][0]["message"]
                .as_str()
                .expect("graphql error message");
            assert!(message.contains("tvdb id 123456 is already assigned to title"));
            assert!(message.contains(&existing_name));
        },
    );
}
