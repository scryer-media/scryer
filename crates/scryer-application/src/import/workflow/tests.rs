#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::is_sample_file;
    use super::{
        CompletedDownloadSubmissionMatch, CompletedDownloadSubmissionResolution,
        CompletedImportEvidenceInputs, CompletedImportEvidenceSource,
        CompletedImportIdentityPolicy, CompletedImportRequestPayload,
        IMPORT_TRANSFER_HEARTBEAT_INTERVAL, ManualImportCandidateMapping, ReleaseEvidence,
        SelectedCompletedImportEvidence, StoredCompletedImportRequestPayload,
        completed_import_error_message_is_retryable, completed_import_status_for_result,
        discover_manual_import_video_candidates, download_submission_persistence_may_be_in_flight,
        manual_episode_suggestion_for_grabbed_scope, parse_import_release_for_title,
        qualify_manual_import_video_candidate, resolved_episode_ids_are_within_expected,
        sanitized_title_folder_component,
        select_completed_import_evidence, should_persist_import_transfer_heartbeat,
        skip_reason_for_import_check_code, stamp_scryer_submission_origin,
        submission_has_scryer_origin, validate_manual_import_candidate_mapping_targets,
        validate_manual_import_source_under_trusted_root,
    };
    use crate::{DownloadSubmission, DownloadSubmissionPurpose, SubmissionScope};
    use chrono::Utc;
    use scryer_domain::MediaFacet;
    use scryer_domain::{
        CompletedDownload, ImportDecision, ImportResult, ImportSkipReason, ImportStatus,
    };
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    /// Test-side view of the stamp step: what the provenance resolver does with
    /// a live submission lookup for a Scryer-origin row (stamped copy) versus
    /// anything else (no Scryer origin).
    #[derive(Clone, Debug)]
    enum CompletedDownloadOriginResolution {
        Ready(Box<CompletedDownload>),
        NoScryerOrigin,
    }

    fn resolve_completed_download_origin(
        completed: &CompletedDownload,
        resolution: &CompletedDownloadSubmissionResolution,
    ) -> CompletedDownloadOriginResolution {
        match resolution {
            CompletedDownloadSubmissionResolution::Matched(matched)
                if submission_has_scryer_origin(&matched.submission) =>
            {
                CompletedDownloadOriginResolution::Ready(Box::new(stamp_scryer_submission_origin(
                    completed,
                    &matched.submission,
                )))
            }
            _ => CompletedDownloadOriginResolution::NoScryerOrigin,
        }
    }

    #[test]
    fn title_folder_component_falls_back_when_sanitized_empty() {
        assert_eq!(sanitized_title_folder_component("///...___---"), "untitled");
    }

    #[test]
    fn title_folder_component_keeps_nonempty_values() {
        assert_eq!(
            sanitized_title_folder_component("Movie Title (2024)"),
            "Movie Title (2024)"
        );
    }

    #[test]
    fn retryable_import_errors_match_locked_as_a_word() {
        assert!(!completed_import_error_message_is_retryable(
            "post-download rule(s) blocked import: language policy"
        ));
        assert!(completed_import_error_message_is_retryable(
            "source file is locked by another process"
        ));
    }

    #[test]
    fn grabbed_release_gate_allows_only_expected_episode_ids() {
        let expected = HashSet::from(["ep-1".to_string()]);

        assert!(resolved_episode_ids_are_within_expected(
            &["ep-1".to_string()],
            &expected
        ));
        assert!(!resolved_episode_ids_are_within_expected(
            &["ep-1".to_string(), "ep-2".to_string()],
            &expected
        ));
        // A file that binds to no episode is not within the grabbed release
        // either; the importer rejects it as `episode_not_found_for_title`.
        assert!(!resolved_episode_ids_are_within_expected(&[], &expected));
    }

    #[test]
    fn manual_preview_starts_from_single_grabbed_episode_only_without_a_file_parse() {
        let grabbed = HashSet::from(["episode-3".to_string()]);

        // The largest video with no episode parse of its own takes the single
        // grabbed episode.
        assert_eq!(
            manual_episode_suggestion_for_grabbed_scope(None, &grabbed, true).as_deref(),
            Some("episode-3")
        );
        // A file that parsed to an episode outside the grabbed scope never
        // gets the grabbed episode substituted for its own parse.
        assert_eq!(
            manual_episode_suggestion_for_grabbed_scope(
                Some("episode-4".to_string()),
                &grabbed,
                false,
            ),
            None
        );
    }

    #[test]
    fn manual_preview_leaves_non_grabbed_ambiguous_files_unselected() {
        let grabbed = HashSet::from(["episode-3".to_string(), "episode-4".to_string()]);

        assert_eq!(
            manual_episode_suggestion_for_grabbed_scope(
                Some("episode-8".to_string()),
                &grabbed,
                false,
            ),
            None
        );
        assert_eq!(
            manual_episode_suggestion_for_grabbed_scope(
                Some("episode-4".to_string()),
                &grabbed,
                false,
            )
            .as_deref(),
            Some("episode-4")
        );
    }

    fn completed_download_with_parameters(parameters: Vec<(&str, &str)>) -> CompletedDownload {
        CompletedDownload {
            client_type: "sabnzbd".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "item-1".to_string(),
            download_id: Some("download-1".to_string()),
            name: "Release".to_string(),
            release_name: None,
            dest_dir: "/downloads/release".to_string(),
            category: Some("anime".to_string()),
            size_bytes: Some(1024),
            completed_at: None,
            parameters: parameters
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn completed_import_request_round_trip_preserves_release_evidence() {
        let completed = completed_download_with_parameters(Vec::new());
        let payload = CompletedImportRequestPayload {
            completed,
            release_evidence: ReleaseEvidence::ScryerSubmission {
                title_id: "title-1".to_string(),
                facet: "series".to_string(),
                source_title: Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb".to_string()),
                observed_release_name: None,
                release_size_bytes: None,
                purpose: DownloadSubmissionPurpose::Standard,
                scope: SubmissionScope::Episode {
                    episode_id: "episode-3".to_string(),
                },
            },
            target_title_id: Some("title-1".to_string()),
        };

        let encoded = serde_json::to_string(&payload).expect("serialize current import request");
        let decoded: StoredCompletedImportRequestPayload =
            serde_json::from_str(&encoded).expect("deserialize current import request");

        let StoredCompletedImportRequestPayload::Current(decoded) = decoded else {
            panic!("current import request must not deserialize as legacy");
        };
        assert_eq!(decoded.target_title_id.as_deref(), Some("title-1"));
        let ReleaseEvidence::ScryerSubmission {
            title_id,
            source_title,
            scope,
            ..
        } = decoded.release_evidence
        else {
            panic!("Scryer evidence must survive retry serialization");
        };
        assert_eq!(title_id, "title-1");
        assert_eq!(
            source_title.as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
        assert!(matches!(
            scope,
            SubmissionScope::Episode { episode_id } if episode_id == "episode-3"
        ));
    }

    #[test]
    fn completed_import_request_reads_legacy_completion_payload() {
        let completed = completed_download_with_parameters(Vec::new());
        let encoded = serde_json::to_string(&completed).expect("serialize legacy completion");
        let decoded: StoredCompletedImportRequestPayload =
            serde_json::from_str(&encoded).expect("deserialize legacy completion");

        let StoredCompletedImportRequestPayload::Legacy(decoded) = decoded else {
            panic!("legacy completion must remain readable");
        };
        assert_eq!(decoded.download_client_item_id, "item-1");
        assert_eq!(decoded.release_name, None);
    }

    #[test]
    fn recent_missing_download_submission_gets_bounded_visibility_grace() {
        let now = Utc::now();
        let resolution = CompletedDownloadSubmissionResolution::MissingDownloadId {
            identity: crate::DownloadSubmissionIdentity {
                download_id: Some("scryer-download:pending".to_string()),
            },
        };
        let mut completed = completed_download_with_parameters(Vec::new());
        completed.completed_at = Some(now - chrono::Duration::seconds(1));

        assert!(download_submission_persistence_may_be_in_flight(
            &completed,
            &resolution,
            now,
        ));

        completed.completed_at = Some(now - chrono::Duration::seconds(16));
        assert!(!download_submission_persistence_may_be_in_flight(
            &completed,
            &resolution,
            now,
        ));

        completed.completed_at = None;
        assert!(!download_submission_persistence_may_be_in_flight(
            &completed,
            &resolution,
            now,
        ));
    }

    fn matched_submission(
        title_id: &str,
        facet: &str,
        scope: SubmissionScope,
    ) -> CompletedDownloadSubmissionResolution {
        matched_submission_with_source_title(
            title_id,
            facet,
            scope,
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb"),
        )
    }

    fn matched_submission_with_source_title(
        title_id: &str,
        facet: &str,
        scope: SubmissionScope,
        source_title: Option<&str>,
    ) -> CompletedDownloadSubmissionResolution {
        CompletedDownloadSubmissionResolution::Matched(Box::new(CompletedDownloadSubmissionMatch {
            submission: DownloadSubmission {
    download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title_id.to_string(),
                facet: facet.to_string(),
                download_client_id: Some("client-1".to_string()),
                download_client_type: "sabnzbd".to_string(),
                download_client_item_id: "item-1".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: source_title.map(str::to_string),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                purpose: DownloadSubmissionPurpose::Standard,
                scope,
            },
            identity: None,
        }))
    }

    fn parameter_value<'a>(parameters: &'a [(String, String)], key: &str) -> Option<&'a str> {
        parameters
            .iter()
            .find(|(candidate_key, _)| candidate_key == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn completed_origin_resolution_keeps_matching_complete_params() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
            ("*scryer_series_movie_link_id", "series-movie-link-1"),
        ]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(resolved.parameters, completed.parameters);
    }

    #[test]
    fn completed_origin_resolution_writes_series_movie_scope_without_existing_params() {
        let completed = completed_download_with_parameters(vec![]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_title_id"),
            Some("title-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_facet"),
            Some("anime")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_series_movie_link_id"),
            Some("series-movie-link-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_collection_id"),
            None
        );
    }

    #[test]
    fn completed_origin_resolution_replaces_stale_scope_parameters() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
            ("*scryer_collection_id", "legacy-collection-1"),
        ]);
        let resolution = matched_submission(
            "title-1",
            "anime",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_collection_id"),
            None
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_series_movie_link_id"),
            Some("series-movie-link-1")
        );
    }

    #[test]
    fn completed_origin_resolution_replaces_untrusted_title_facet_and_scope_parameters() {
        for (completed, resolution) in [
            (
                completed_download_with_parameters(vec![("*scryer_title_id", "title-2")]),
                matched_submission("title-1", "anime", SubmissionScope::Title),
            ),
            (
                completed_download_with_parameters(vec![
                    ("*scryer_title_id", "title-1"),
                    ("*scryer_facet", "movie"),
                ]),
                matched_submission("title-1", "anime", SubmissionScope::Title),
            ),
            (
                completed_download_with_parameters(vec![
                    ("*scryer_title_id", "title-1"),
                    ("*scryer_facet", "anime"),
                    ("*scryer_series_movie_link_id", "series-movie-link-1"),
                ]),
                matched_submission(
                    "title-1",
                    "anime",
                    SubmissionScope::Collection {
                        collection_id: "collection-1".to_string(),
                    },
                ),
            ),
        ] {
            let CompletedDownloadOriginResolution::Ready(resolved) =
                resolve_completed_download_origin(&completed, &resolution)
            else {
                panic!("expected durable submission to win");
            };
            assert_eq!(
                parameter_value(&resolved.parameters, "*scryer_title_id"),
                Some("title-1")
            );
            assert_eq!(
                parameter_value(&resolved.parameters, "*scryer_facet"),
                Some("anime")
            );
        }
    }

    #[test]
    fn completed_origin_resolution_treats_stub_submission_as_observation() {
        let completed = completed_download_with_parameters(vec![
            ("*scryer_title_id", "title-1"),
            ("*scryer_facet", "anime"),
        ]);
        let resolution = matched_submission(
            "",
            "",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        assert!(matches!(
            resolve_completed_download_origin(&completed, &resolution),
            CompletedDownloadOriginResolution::NoScryerOrigin
        ));
    }

    #[test]
    fn completed_origin_resolution_ignores_stub_submission_without_params() {
        let completed = completed_download_with_parameters(vec![]);
        let resolution = matched_submission(
            "",
            "",
            SubmissionScope::SeriesMovie {
                series_movie_link_id: "series-movie-link-1".to_string(),
            },
        );

        assert!(matches!(
            resolve_completed_download_origin(&completed, &resolution),
            CompletedDownloadOriginResolution::NoScryerOrigin
        ));
    }

    #[test]
    fn qbit_scryer_submission_uses_durable_release_not_display_label() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.client_type = "qbittorrent".to_string();
        completed.name = "Tokan — S01E03 2160p WEB-DL".to_string();

        let resolution = matched_submission("title-1", "series", SubmissionScope::Title);
        let evidence = super::release_evidence_for_resolution(&completed, &resolution);

        let title = series_title("title-1", "Tōkan", &["Tokan"], Some(2024));
        let parsed = super::build_augmented_episode_import_metadata_for_title(
            std::path::Path::new("/downloads/Tokan.S01E03.2160p.WEB-DL.mkv"),
            &evidence,
            &title,
            false,
        );
        assert_eq!(
            evidence.release_title(None).as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
        assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    }

    fn series_title(
        id: &str,
        name: &str,
        aliases: &[&str],
        year: Option<i32>,
    ) -> scryer_domain::Title {
        scryer_domain::Title {
            id: id.to_string(),
            name: name.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year,
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
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    // ── A1: a Scryer submission without a persisted release title stays importable ──

    #[test]
    fn scryer_submission_without_source_title_keeps_identity_and_uses_client_release_name() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.client_type = "nzbget".to_string();
        completed.name = "Paper Lantern (display label)".to_string();
        completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());
        let resolution =
            matched_submission_with_source_title("title-1", "movie", SubmissionScope::Title, None);

        let evidence = super::release_evidence_for_resolution(&completed, &resolution);

        let ReleaseEvidence::ScryerSubmission {
            title_id,
            source_title,
            observed_release_name,
            ..
        } = &evidence
        else {
            panic!("a Scryer submission without a release title must keep its identity");
        };
        assert_eq!(title_id, "title-1");
        assert_eq!(source_title, &None);
        assert_eq!(
            observed_release_name.as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL")
        );
        assert_eq!(evidence.title_id(), Some("title-1"));
        assert_eq!(evidence.facet(), Some("movie"));
        assert_eq!(evidence.scope(), Some(&SubmissionScope::Title));
        // The name degrades to the client-reported release name, never the label.
        assert_eq!(
            evidence.release_title(None).as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL")
        );

        // A blank persisted title is the same as none.
        let blank = matched_submission_with_source_title(
            "title-1",
            "movie",
            SubmissionScope::Title,
            Some("   "),
        );
        let evidence = super::release_evidence_for_resolution(&completed, &blank);
        assert_eq!(
            evidence.release_title(None).as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL")
        );

        // A persisted title still wins over the client-reported name.
        let persisted = matched_submission("title-1", "movie", SubmissionScope::Title);
        let evidence = super::release_evidence_for_resolution(&completed, &persisted);
        assert_eq!(
            evidence.release_title(None).as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
    }

    #[test]
    fn scryer_submission_without_any_release_name_falls_back_to_source_video_stem() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.release_name = None;
        let resolution =
            matched_submission_with_source_title("title-1", "movie", SubmissionScope::Title, None);

        let evidence = super::release_evidence_for_resolution(&completed, &resolution);

        assert_eq!(evidence.title_id(), Some("title-1"));
        assert_eq!(evidence.release_title(None), None);
        assert_eq!(
            evidence
                .release_title(Some(std::path::Path::new(
                    "/downloads/paper/Paper.Lantern.2012.1080p.WEB-DL.mkv"
                )))
                .as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL")
        );
    }

    #[test]
    fn legacy_scryer_submission_payload_with_string_source_title_still_deserializes() {
        // Exactly the shape persisted before `source_title` became optional and
        // `observed_release_name`/`target_title_id` existed.
        let completed = completed_download_with_parameters(vec![]);
        let legacy = serde_json::json!({
            "completed": completed,
            "release_evidence": {
                "ScryerSubmission": {
                    "title_id": "title-1",
                    "facet": "series",
                    "source_title": "Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb",
                    "purpose": serde_json::to_value(DownloadSubmissionPurpose::Standard).unwrap(),
                    "scope": serde_json::to_value(SubmissionScope::Episode {
                        episode_id: "episode-3".to_string(),
                    })
                    .unwrap(),
                }
            },
            "manual_title_id": "title-1",
        })
        .to_string();

        let decoded: StoredCompletedImportRequestPayload =
            serde_json::from_str(&legacy).expect("legacy payload must deserialize");
        let StoredCompletedImportRequestPayload::Current(decoded) = decoded else {
            panic!("legacy current payload must not fall through to the completion-only shape");
        };
        assert_eq!(decoded.target_title_id.as_deref(), Some("title-1"));
        let ReleaseEvidence::ScryerSubmission {
            source_title,
            observed_release_name,
            title_id,
            ..
        } = decoded.release_evidence
        else {
            panic!("legacy Scryer evidence must decode as a Scryer submission");
        };
        assert_eq!(title_id, "title-1");
        assert_eq!(
            source_title.as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
        assert_eq!(observed_release_name, None);

        // The stand-alone evidence snapshot (manual import selections) decodes too.
        let evidence: ReleaseEvidence = serde_json::from_value(serde_json::json!({
            "ScryerSubmission": {
                "title_id": "title-1",
                "facet": "series",
                "source_title": "Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb",
                "purpose": serde_json::to_value(DownloadSubmissionPurpose::Standard).unwrap(),
                "scope": serde_json::to_value(SubmissionScope::Title).unwrap(),
            }
        }))
        .expect("legacy evidence snapshot must deserialize");
        assert_eq!(
            evidence.release_title(None).as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
    }

    #[test]
    fn completed_origin_resolution_keeps_client_release_name_when_submission_has_none() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());

        let without_title =
            matched_submission_with_source_title("title-1", "movie", SubmissionScope::Title, None);
        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &without_title)
        else {
            panic!("expected ready completed download");
        };
        assert_eq!(
            resolved.release_name.as_deref(),
            Some("Paper.Lantern.2012.1080p.WEB-DL"),
            "a submission without a persisted title must not blank the client-reported name"
        );

        let with_title = matched_submission("title-1", "movie", SubmissionScope::Title);
        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &with_title)
        else {
            panic!("expected ready completed download");
        };
        assert_eq!(
            resolved.release_name.as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb"),
            "the persisted indexer release title is THE name for a Scryer grab"
        );
    }

    // ── A2/A3: evidence and target selection for an import attempt ──

    fn scryer_evidence(title_id: &str, source_title: &str) -> ReleaseEvidence {
        ReleaseEvidence::ScryerSubmission {
            title_id: title_id.to_string(),
            facet: "movie".to_string(),
            source_title: Some(source_title.to_string()),
            observed_release_name: None,
            release_size_bytes: None,
            purpose: DownloadSubmissionPurpose::Standard,
            scope: SubmissionScope::Title,
        }
    }

    fn select(
        identity_policy: CompletedImportIdentityPolicy,
        fresh_resolution: Option<&CompletedDownloadSubmissionResolution>,
        release_evidence_override: Option<&ReleaseEvidence>,
        persisted_release_evidence: Option<&ReleaseEvidence>,
        persisted_target_title_id: Option<&str>,
        requested_target_title_id: Option<&str>,
        completed: &CompletedDownload,
    ) -> SelectedCompletedImportEvidence {
        select_completed_import_evidence(CompletedImportEvidenceInputs {
            identity_policy,
            fresh_resolution,
            release_evidence_override,
            persisted_release_evidence,
            persisted_target_title_id,
            requested_target_title_id,
            completed,
        })
    }

    #[test]
    fn live_scryer_submission_row_beats_persisted_evidence_and_stale_target() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.release_name = Some("Client.Release.Name".to_string());
        let fresh = matched_submission("title-fresh", "movie", SubmissionScope::Title);
        let persisted = scryer_evidence("title-stale", "Stale.Release");

        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&fresh),
            None,
            Some(&persisted),
            Some("title-stale"),
            None,
            &completed,
        );

        assert_eq!(selected.source, CompletedImportEvidenceSource::FreshRow);
        assert_eq!(selected.release_evidence.title_id(), Some("title-fresh"));
        assert_eq!(
            selected.release_evidence.release_title(None).as_deref(),
            Some("Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb")
        );
        assert_eq!(
            selected.target_title_id, None,
            "the submission is the target"
        );
    }

    #[test]
    fn live_titled_row_without_scryer_origin_beats_persisted_scryer_submission_and_target() {
        // A live titled row that carries no Scryer origin (defensive: the store
        // reads titled rows back with a real scope, so this only occurs before
        // a round trip) still names the operator's choice; a retry must land
        // there, not in the old title the persisted evidence remembers.
        let mut completed = completed_download_with_parameters(vec![]);
        completed.release_name = Some("Client.Release.Name".to_string());
        let reassigned =
            matched_submission_with_source_title("title-b", "movie", SubmissionScope::Orphan, None);
        let persisted = scryer_evidence("title-a", "Old.Grab.Release");

        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&reassigned),
            None,
            Some(&persisted),
            Some("title-a"),
            None,
            &completed,
        );

        assert_eq!(selected.source, CompletedImportEvidenceSource::FreshRow);
        assert!(matches!(
            selected.release_evidence,
            ReleaseEvidence::DownloaderObservation { ref release_name }
                if release_name.as_deref() == Some("Client.Release.Name")
        ));
        assert_eq!(selected.target_title_id.as_deref(), Some("title-b"));
    }

    #[test]
    fn persisted_scryer_submission_is_used_only_when_the_row_is_gone() {
        let mut completed = completed_download_with_parameters(vec![]);
        completed.release_name = Some("Client.Release.Name".to_string());
        let persisted = scryer_evidence("title-a", "Old.Grab.Release");
        let no_row = CompletedDownloadSubmissionResolution::DownloaderObservation;

        // Row lost: the persisted evidence and target still drive the import.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            Some(&persisted),
            Some("title-a"),
            None,
            &completed,
        );
        assert_eq!(selected.source, CompletedImportEvidenceSource::Persisted);
        assert_eq!(selected.release_evidence.title_id(), Some("title-a"));
        assert_eq!(selected.target_title_id, None);

        // Transient lookup failure: same fallback.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            None,
            None,
            Some(&persisted),
            None,
            None,
            &completed,
        );
        assert_eq!(selected.source, CompletedImportEvidenceSource::Persisted);
        assert_eq!(selected.release_evidence.title_id(), Some("title-a"));

        // Nothing persisted and no row: the client-reported observation.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            Some("title-b"),
            None,
            &completed,
        );
        assert_eq!(
            selected.source,
            CompletedImportEvidenceSource::FreshObservation
        );
        assert!(matches!(
            selected.release_evidence,
            ReleaseEvidence::DownloaderObservation { ref release_name }
                if release_name.as_deref() == Some("Client.Release.Name")
        ));
        assert_eq!(selected.target_title_id.as_deref(), Some("title-b"));
    }

    #[test]
    fn requested_target_is_the_import_target_for_observation_evidence() {
        let completed = completed_download_with_parameters(vec![]);
        let stub_row = matched_submission_with_source_title("", "", SubmissionScope::Orphan, None);

        // The tracked download's validated title outranks a persisted target.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&stub_row),
            None,
            None,
            Some("title-persisted"),
            Some("title-tracked"),
            &completed,
        );
        assert_eq!(selected.release_evidence.title_id(), None);
        assert_eq!(selected.target_title_id.as_deref(), Some("title-tracked"));

        // A stub orphan row (no title) names nothing, so the persisted target
        // is still honored on retry.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&stub_row),
            None,
            None,
            Some("title-persisted"),
            None,
            &completed,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-persisted"));

        // Blank requested/persisted targets are no targets.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&stub_row),
            None,
            None,
            Some("  "),
            Some(""),
            &completed,
        );
        assert_eq!(selected.target_title_id, None);
    }

    #[test]
    fn scryer_title_id_parameter_is_the_last_resort_target_for_observation_evidence() {
        // Both the submission row and the tracked state are gone; only Scryer's
        // own add-time stamp names the title.
        let stamped = completed_download_with_parameters(vec![
            ("*scryer_title_id", "  title-param  "),
            ("*scryer_facet", "movie"),
        ]);
        let no_row = CompletedDownloadSubmissionResolution::DownloaderObservation;

        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            None,
            None,
            &stamped,
        );
        assert_eq!(selected.release_evidence.title_id(), None);
        assert_eq!(selected.target_title_id.as_deref(), Some("title-param"));

        // Every earlier source still outranks it: the requested target …
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            None,
            Some("title-tracked"),
            &stamped,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-tracked"));

        // … a live titled row without Scryer origin …
        let reassigned =
            matched_submission_with_source_title("title-b", "movie", SubmissionScope::Orphan, None);
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&reassigned),
            None,
            None,
            None,
            None,
            &stamped,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-b"));

        // … and the persisted target.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            Some("title-persisted"),
            None,
            &stamped,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-persisted"));

        // A stub orphan row names nothing, so the stamp still applies.
        let stub_row = matched_submission_with_source_title("", "", SubmissionScope::Orphan, None);
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&stub_row),
            None,
            None,
            None,
            None,
            &stamped,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-param"));

        // It never overrides a Scryer submission title, whether the row is
        // live or the persisted evidence is what is being replayed.
        let fresh = matched_submission("title-submission", "movie", SubmissionScope::Title);
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&fresh),
            None,
            None,
            None,
            None,
            &stamped,
        );
        assert_eq!(
            selected.release_evidence.title_id(),
            Some("title-submission")
        );
        assert_eq!(selected.target_title_id, None);
        let persisted = scryer_evidence("title-a", "Old.Grab.Release");
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            Some(&persisted),
            None,
            None,
            &stamped,
        );
        assert_eq!(selected.release_evidence.title_id(), Some("title-a"));
        assert_eq!(selected.target_title_id, None);

        // A blank or absent stamp is no target.
        let blank = completed_download_with_parameters(vec![("*scryer_title_id", "  ")]);
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            None,
            None,
            &blank,
        );
        assert_eq!(selected.target_title_id, None);
        let unstamped = completed_download_with_parameters(vec![]);
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&no_row),
            None,
            None,
            None,
            None,
            &unstamped,
        );
        assert_eq!(selected.target_title_id, None);
    }

    #[test]
    fn scryer_submission_settles_a_disagreeing_target_by_policy() {
        let completed = completed_download_with_parameters(vec![]);
        let fresh = matched_submission("title-submission", "movie", SubmissionScope::Title);

        // Automatic import: the submission wins; the tracked title is dropped.
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&fresh),
            None,
            None,
            None,
            Some("title-tracked"),
            &completed,
        );
        assert_eq!(
            selected.release_evidence.title_id(),
            Some("title-submission")
        );
        assert_eq!(selected.target_title_id, None);

        // Manual review: the operator's choice is passed through so the
        // import's own guard can reject a title outside the submission.
        let selected = select(
            CompletedImportIdentityPolicy::AllowUnresolved,
            Some(&fresh),
            None,
            None,
            None,
            Some("title-operator"),
            &completed,
        );
        assert_eq!(selected.target_title_id.as_deref(), Some("title-operator"));

        // A caller-resolved override is used as-is.
        let override_evidence = scryer_evidence("title-override", "Override.Release");
        let selected = select(
            CompletedImportIdentityPolicy::RequireSubmission,
            Some(&fresh),
            Some(&override_evidence),
            None,
            Some("title-persisted"),
            None,
            &completed,
        );
        assert_eq!(selected.source, CompletedImportEvidenceSource::Override);
        assert_eq!(selected.release_evidence.title_id(), Some("title-override"));
        assert_eq!(selected.target_title_id, None);
    }

    #[test]
    fn invalid_and_sample_check_codes_are_permanent_policy_mismatches() {
        assert_eq!(
            skip_reason_for_import_check_code(crate::import_checks::ImportCheckCode::InvalidExtension),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code(crate::import_checks::ImportCheckCode::SampleFile),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code(
                crate::import_checks::ImportCheckCode::SampleDirectory,
            ),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code(crate::import_checks::ImportCheckCode::StillUnpacking),
            ImportSkipReason::DownloadInProgress
        );
    }

    #[test]
    fn import_transfer_heartbeat_refreshes_when_progress_stalls() {
        assert!(should_persist_import_transfer_heartbeat(None));
        assert!(!should_persist_import_transfer_heartbeat(Some(
            Instant::now()
        )));

        let stale_emit =
            Instant::now() - IMPORT_TRANSFER_HEARTBEAT_INTERVAL - Duration::from_secs(1);
        assert!(should_persist_import_transfer_heartbeat(Some(stale_emit)));
    }

    #[test]
    fn retryable_completed_import_results_remain_pending_without_terminal_status() {
        let source = tempfile::tempdir().expect("source tempdir");
        let mut result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::NoVideoFiles),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("item-1".to_string()),
            source_title: Some("Release".to_string()),
            source_path: source.path().to_string_lossy().into_owned(),
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            file_size_bytes: None,
            link_type: None,
            error_message: None,
            release_burned: false,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Pending
        );

        result.skip_reason = Some(ImportSkipReason::UnparseableEpisode);
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Skipped
        );

        result.skip_reason = Some(ImportSkipReason::PolicyMismatch);
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Skipped
        );

        result.error_message = Some("source changed during copy".to_string());
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Pending
        );

        // These execution races lack a structured outcome and remain the only
        // message-based retry hints. Import-check rejection reasons are typed.
        for transient in ["source changed during copy", "destination temporarily unavailable"] {
            result.error_message = Some(transient.to_string());
            assert_eq!(
                completed_import_status_for_result(&result, ImportStatus::Failed),
                ImportStatus::Pending,
                "{transient}"
            );
        }

        // Decision-phase skips stay terminal whatever the text — there is no OS
        // error catalogue any more; execution failures are retried by phase.
        for permanent in [
            "failed to move file: Invalid argument (os error 22)",
            "failed to move file: Is a directory (os error 21)",
            "failed to move file: Device or resource busy (os error 16)",
            "failed to remove source: The process cannot access the file because it is being used by another process. (os error 32)",
            "archive requires a password",
        ] {
            result.error_message = Some(permanent.to_string());
            assert_eq!(
                completed_import_status_for_result(&result, ImportStatus::Failed),
                ImportStatus::Failed,
                "{permanent}"
            );
        }

        // Sonarr's phase rule: an approved import that failed to *execute* is
        // retried regardless of the error text — no catalogue can be complete.
        result.decision = ImportDecision::Failed;
        result.skip_reason = None;
        for execution_failure in [
            "failed to move file: Invalid argument (os error 22)",
            "failed to remove source: The process cannot access the file because it is being used by another process. (os error 32)",
            "0 imported, 0 skipped, 0 rejected, 1 failed. Last error: something novel",
            "database is locked",
        ] {
            result.error_message = Some(execution_failure.to_string());
            assert_eq!(
                completed_import_status_for_result(&result, ImportStatus::Failed),
                ImportStatus::Pending,
                "{execution_failure}"
            );
        }
        result.error_message = None;
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Pending
        );

        // ...except a password-protected archive, which cannot succeed without
        // operator input.
        result.skip_reason = Some(ImportSkipReason::PasswordRequired);
        result.error_message = Some("archive requires a password".to_string());
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Failed
        );

        result.skip_reason = Some(ImportSkipReason::ArchiveExtractionTimedOut);
        result.error_message = Some("archive plugin timed out after 3600 seconds".to_string());
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Failed
        );

        // Environmental skips clear on their own and are retried.
        result.decision = ImportDecision::Skipped;
        result.error_message = Some("not enough room".to_string());
        for environmental in [
            ImportSkipReason::DownloadInProgress,
            ImportSkipReason::DiskFull,
            ImportSkipReason::PermissionDenied,
        ] {
            result.skip_reason = Some(environmental);
            assert_eq!(
                completed_import_status_for_result(&result, ImportStatus::Skipped),
                ImportStatus::Pending,
                "{}",
                result
                    .skip_reason
                    .as_ref()
                    .map(ImportSkipReason::as_str)
                    .unwrap_or("none")
            );
        }

        // Rejections are decision-phase and stay terminal.
        result.decision = ImportDecision::Rejected;
        result.skip_reason = Some(ImportSkipReason::PolicyMismatch);
        result.error_message = Some("existing file is equal or better".to_string());
        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Failed),
            ImportStatus::Failed
        );
    }

    #[test]
    fn held_replacement_import_result_stays_blocked_instead_of_retrying() {
        // The replace guard parks an implausible overwrite for manual resolution.
        // Its message must not read as a transient failure, or the tracked
        // download would be scheduled for retry instead of landing blocked.
        let mut analysis = crate::post_download_gate::build_stream_pointer_media_file_analysis();
        analysis.duration_seconds = Some(1_495);
        let accepted = crate::post_download_gate::ImportedFileAcceptance {
            analysis: Some(analysis),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        };
        let message = crate::post_download_gate::replace_runtime_band_block(
            crate::post_download_gate::RuntimeSampleValidation::automatic(None),
            &accepted,
            crate::post_download_gate::incumbent_replace_runtime_seconds([Some(3_300)]),
        )
        .expect("implausible replacement should be held");

        assert_eq!(
            crate::post_download_gate::REPLACE_BLOCKED_RUNTIME_MISMATCH_CODE,
            "replace_blocked_runtime_mismatch"
        );

        let source = tempfile::tempdir().expect("source tempdir");
        let result = ImportResult {
            import_id: "import-1".to_string(),
            decision: ImportDecision::Skipped,
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            title_id: Some("title-1".to_string()),
            source_system: Some("nzbget".to_string()),
            source_ref: Some("item-1".to_string()),
            source_title: Some("Release".to_string()),
            source_path: source.path().to_string_lossy().into_owned(),
            dest_path: None,
            quality: None,
            episode_ids: Vec::new(),
            file_size_bytes: None,
            link_type: None,
            error_message: Some(message),
            release_burned: false,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        assert_eq!(
            completed_import_status_for_result(&result, ImportStatus::Skipped),
            ImportStatus::Skipped
        );
    }

    #[test]
    fn manual_import_source_validation_rejects_files_outside_trusted_root() {
        let source = tempfile::tempdir().expect("source tempdir");
        let other = tempfile::tempdir().expect("other tempdir");
        let inside = source.path().join("episode.mkv");
        let outside = other.path().join("episode.mkv");
        std::fs::write(&inside, b"video").expect("write inside file");
        std::fs::write(&outside, b"video").expect("write outside file");

        assert!(
            validate_manual_import_source_under_trusted_root(
                &inside,
                &std::fs::canonicalize(source.path()).expect("canonical source root"),
            )
            .is_ok(),
        );

        let err = validate_manual_import_source_under_trusted_root(
            &outside,
            &std::fs::canonicalize(source.path()).expect("canonical source root"),
        )
        .expect_err("outside file should be rejected");
        assert!(err.to_string().contains("outside the trusted source root"));
    }

    #[cfg(feature = "runtime-media-analysis")]
    fn copy_mediainfo_fixture(destination: &std::path::Path) {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scryer-mediainfo/tests/media/hevc_hdr10plus.mkv");
        std::fs::copy(fixture, destination).expect("copy MediaInfo fixture");
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn manual_import_content_probe_accepts_large_extensionless_video_source() {
        let source = tempfile::tempdir().expect("source tempdir");
        let opaque_file = source.path().join("dXRUKoYEAJ58jradJdxMKKxgczVTvt");
        copy_mediainfo_fixture(&opaque_file);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&opaque_file)
            .expect("open fixture")
            .set_len(16 * 1024 * 1024)
            .expect("pad fixture");
        let trusted_root = std::fs::canonicalize(&opaque_file).expect("canonical source file");

        let candidate = qualify_manual_import_video_candidate(&opaque_file, &trusted_root)
            .await
            .expect("qualify candidate")
            .expect("extensionless video candidate");

        assert_eq!(candidate.canonical_path, trusted_root);
        let facts = candidate.video_facts.expect("video facts");
        assert_eq!(facts.container_format.as_deref(), Some("matroska"));
        assert_eq!(facts.video_codec.as_deref(), Some("hevc"));
        assert!(facts.duration_seconds.is_some_and(|duration| duration > 0));
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn manual_import_known_extensions_skip_content_probing() {
        let source = tempfile::tempdir().expect("source tempdir");
        let known_video = source.path().join("Example.Show.S01E02.1080p.mkv");
        copy_mediainfo_fixture(&known_video);
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");

        let candidate = qualify_manual_import_video_candidate(&known_video, &trusted_root)
            .await
            .expect("qualify candidate")
            .expect("known-extension video candidate");

        assert!(candidate.video_facts.is_none());
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn manual_import_unrecognized_extensions_skip_content_probing() {
        let source = tempfile::tempdir().expect("source tempdir");
        let unrecognized_extension = source.path().join("Example.Show.S01E02.1080p.download");
        copy_mediainfo_fixture(&unrecognized_extension);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&unrecognized_extension)
            .expect("open fixture")
            .set_len(16 * 1024 * 1024)
            .expect("pad fixture");
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");

        assert!(
            qualify_manual_import_video_candidate(&unrecognized_extension, &trusted_root)
                .await
                .expect("qualify candidate")
                .is_none()
        );
    }

    #[cfg(all(feature = "runtime-media-analysis", unix))]
    #[tokio::test]
    async fn manual_import_discovery_surfaces_an_unavailable_only_candidate() {
        use std::os::unix::fs::PermissionsExt;

        let source = tempfile::tempdir().expect("source tempdir");
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");
        let unavailable = source.path().join("unavailable.mkv");
        copy_mediainfo_fixture(&unavailable);
        std::fs::set_permissions(&unavailable, std::fs::Permissions::from_mode(0o000))
            .expect("make candidate unavailable");

        let result = discover_manual_import_video_candidates(&trusted_root).await;

        std::fs::set_permissions(&unavailable, std::fs::Permissions::from_mode(0o644))
            .expect("restore candidate permissions");
        let error = result.expect_err("unavailable-only candidate should surface an error");
        assert!(error.to_string().contains("not accessible"));
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn manual_import_content_probe_rejects_small_opaque_files() {
        let source = tempfile::tempdir().expect("source tempdir");
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");
        let small_opaque = source.path().join("opaque");
        copy_mediainfo_fixture(&small_opaque);

        assert!(
            qualify_manual_import_video_candidate(&small_opaque, &trusted_root)
                .await
                .expect("qualify small opaque file")
                .is_none()
        );
    }

    #[cfg(all(feature = "runtime-media-analysis", unix))]
    #[tokio::test]
    async fn manual_import_discovery_preserves_symlink_entry_name_and_rejects_escape() {
        let source = tempfile::tempdir().expect("source tempdir");
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");
        let opaque_target = source.path().join("opaque-target");
        copy_mediainfo_fixture(&opaque_target);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&opaque_target)
            .expect("open fixture")
            .set_len(16 * 1024 * 1024)
            .expect("pad fixture");
        let release_entry = source.path().join("Example.Show.S01E02.1080p.mkv");
        std::os::unix::fs::symlink(&opaque_target, &release_entry)
            .expect("create contained source symlink");

        let outside = tempfile::tempdir().expect("outside tempdir");
        copy_mediainfo_fixture(&outside.path().join("Outside.Show.S01E03.mkv"));
        std::os::unix::fs::symlink(outside.path(), source.path().join("outside-link"))
            .expect("create escaping directory symlink");

        let candidates = discover_manual_import_video_candidates(&trusted_root)
            .await
            .expect("discover manual import candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].source_entry_path.file_name(),
            release_entry.file_name()
        );
        assert_eq!(
            candidates[0].canonical_path,
            std::fs::canonicalize(&opaque_target).expect("canonical target")
        );
        assert!(candidates[0].video_facts.is_none());
    }

    #[cfg(all(feature = "runtime-media-analysis", unix))]
    #[tokio::test]
    async fn manual_import_content_probe_accepts_symlinked_trusted_root() {
        let actual_parent = tempfile::tempdir().expect("actual parent tempdir");
        let actual_root = actual_parent.path().join("source");
        std::fs::create_dir(&actual_root).expect("create source root");
        let actual_file = actual_root.join("Example.Show.S01E02.1080p.mkv");
        copy_mediainfo_fixture(&actual_file);

        let alias_parent = tempfile::tempdir().expect("alias parent tempdir");
        let alias_root = alias_parent.path().join("source-alias");
        std::os::unix::fs::symlink(&actual_root, &alias_root).expect("create source root alias");

        let candidate = qualify_manual_import_video_candidate(
            &alias_root.join("Example.Show.S01E02.1080p.mkv"),
            &alias_root,
        )
        .await
        .expect("qualify symlinked source root")
        .expect("valid video candidate");

        assert_eq!(
            candidate.canonical_path,
            std::fs::canonicalize(&actual_file).expect("canonical source file")
        );
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn manual_import_content_probe_keeps_known_extension_parse_failures() {
        let source = tempfile::tempdir().expect("source tempdir");
        let trusted_root = std::fs::canonicalize(source.path()).expect("canonical source root");
        let stream_pointer = source.path().join("movie.strm");
        std::fs::write(&stream_pointer, b"https://media.example/movie").expect("write pointer");

        let candidate = qualify_manual_import_video_candidate(&stream_pointer, &trusted_root)
            .await
            .expect("qualify stream pointer")
            .expect("recognized extension fallback");
        assert!(candidate.video_facts.is_none());

        let malformed = source.path().join("broken.mkv");
        std::fs::write(&malformed, b"not a Matroska file").expect("write malformed fixture");
        let candidate = qualify_manual_import_video_candidate(&malformed, &trusted_root)
            .await
            .expect("qualify malformed known-extension file")
            .expect("manual fallback for a known extension");
        assert!(candidate.video_facts.is_none());

        let empty = source.path().join("empty.mkv");
        std::fs::write(&empty, b"").expect("write empty fixture");
        assert!(
            qualify_manual_import_video_candidate(&empty, &trusted_root)
                .await
                .expect("qualify empty file")
                .is_none()
        );

        let opaque_malformed = source.path().join("opaque");
        std::fs::write(&opaque_malformed, [0x1A, 0x45, 0xDF, 0xA3])
            .expect("write malformed opaque fixture");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&opaque_malformed)
            .expect("open malformed opaque fixture")
            .set_len(16 * 1024 * 1024)
            .expect("pad malformed opaque fixture");
        assert!(
            qualify_manual_import_video_candidate(&opaque_malformed, &trusted_root)
                .await
                .expect("qualify malformed opaque file")
                .is_none()
        );
    }

    #[test]
    fn manual_import_candidate_mapping_validation_requires_unique_candidate_and_exactly_one_target()
    {
        let neither = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: None,
            series_movie_link_id: None,
        };
        let err = validate_manual_import_candidate_mapping_targets(&[neither], &MediaFacet::Series)
            .expect_err("missing target should be rejected");
        assert!(err.to_string().contains("requires episode_id"));

        let both = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: Some("episode-1".to_string()),
            series_movie_link_id: Some("series-movie-link-1".to_string()),
        };
        let err = validate_manual_import_candidate_mapping_targets(&[both], &MediaFacet::Series)
            .expect_err("ambiguous target should be rejected");
        assert!(err.to_string().contains("cannot include both"));

        let series_movie = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: None,
            series_movie_link_id: Some("series-movie-link-1".to_string()),
        };
        validate_manual_import_candidate_mapping_targets(&[series_movie], &MediaFacet::Series)
            .expect("series movie target should be accepted");

        // A MOVIE has no sub-target to name, so "neither id" is the only shape
        // its mapping can take. Rejecting it left completed movies awaiting
        // manual import with no action that could ever succeed: the UI sends
        // exactly this and the server refused it.
        let movie = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: None,
            series_movie_link_id: None,
        };
        validate_manual_import_candidate_mapping_targets(&[movie], &MediaFacet::Movie)
            .expect("a movie maps to its title, so it needs no explicit target");

        // The facet only relaxes the missing-target rule; a contradictory
        // mapping is still rejected.
        let movie_with_both = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: Some("episode-1".to_string()),
            series_movie_link_id: Some("series-movie-link-1".to_string()),
        };
        let err = validate_manual_import_candidate_mapping_targets(
            &[movie_with_both],
            &MediaFacet::Movie,
        )
        .expect_err("ambiguous target should still be rejected for movies");
        assert!(err.to_string().contains("cannot include both"));

        let duplicate = ManualImportCandidateMapping {
            candidate_id: "candidate-1".to_string(),
            episode_id: Some("episode-1".to_string()),
            series_movie_link_id: None,
        };
        let err = validate_manual_import_candidate_mapping_targets(
            &[duplicate.clone(), duplicate],
            &MediaFacet::Series,
        )
        .expect_err("duplicate candidates should be rejected");
        assert!(err.to_string().contains("must be unique"));
    }

    #[cfg(unix)]
    #[test]
    fn sample_file_detection_uses_lossy_non_utf8_stem() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        let path = Path::new(OsStr::from_bytes(b"/tmp/\xFFsample-clip.mkv"));
        assert!(is_sample_file(path));
    }

    // ── Parse parity: grab and import must read the same release ────────────

    /// A title with everything the canonical parse context is built from.
    fn parity_title(name: &str, facet: MediaFacet, year: Option<i32>) -> scryer_domain::Title {
        scryer_domain::Title {
            id: "parity-title".to_string(),
            name: name.to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/parity"),
            facet,
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year,
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
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    /// The score-relevant projection of a parse. Two lanes that agree on this
    /// cannot score the same release differently.
    fn score_relevant_fields(parsed: &crate::ParsedReleaseMetadata) -> Vec<(&'static str, String)> {
        vec![
            ("quality", format!("{:?}", parsed.quality)),
            ("source", format!("{:?}", parsed.source)),
            ("video_codec", format!("{:?}", parsed.video_codec)),
            ("audio", format!("{:?}", parsed.audio)),
            ("audio_codecs", format!("{:?}", parsed.audio_codecs)),
            ("audio_channels", format!("{:?}", parsed.audio_channels)),
            ("release_group", format!("{:?}", parsed.release_group)),
            ("edition", format!("{:?}", parsed.edition)),
            ("is_proper_upload", parsed.is_proper_upload.to_string()),
            ("is_repack", parsed.is_repack.to_string()),
            ("is_remux", parsed.is_remux.to_string()),
            ("is_bd_disk", parsed.is_bd_disk.to_string()),
            ("is_dolby_vision", parsed.is_dolby_vision.to_string()),
            ("detected_hdr", parsed.detected_hdr.to_string()),
            ("is_hdr10plus", parsed.is_hdr10plus.to_string()),
            ("is_hlg", parsed.is_hlg.to_string()),
            ("has_hdr_fallback", parsed.has_hdr_fallback.to_string()),
            ("is_10bit", parsed.is_10bit.to_string()),
            ("is_atmos", parsed.is_atmos.to_string()),
            ("is_dual_audio", parsed.is_dual_audio.to_string()),
            (
                "streaming_service",
                format!("{:?}", parsed.streaming_service),
            ),
            ("anime_version", format!("{:?}", parsed.anime_version)),
            ("is_ai_enhanced", parsed.is_ai_enhanced.to_string()),
            ("is_hardcoded_subs", parsed.is_hardcoded_subs.to_string()),
            ("is_uncensored", parsed.is_uncensored.to_string()),
            ("is_dubs_only", parsed.is_dubs_only.to_string()),
            ("languages_audio", format!("{:?}", parsed.languages_audio)),
        ]
    }

    /// **Parse parity.** The grab lane parses a candidate with
    /// `parse_release_metadata_for_target` against the title's canonical parse
    /// context; import parses the same name with `parse_import_release_for_title`.
    /// Every field that can move a score must come out identical, or the two
    /// sides are scoring different releases and no amount of sharing the scorer
    /// will make them agree.
    #[test]
    fn grab_and_import_read_the_same_score_relevant_facts() {
        let corpus: &[(&str, MediaFacet, Option<i32>, &[&str])] = &[
            (
                "Portmere",
                MediaFacet::Movie,
                Some(2024),
                &[
                    "Portmere.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR10Plus.H.265-GRP",
                    "Portmere.2024.1080p.BluRay.REMUX.AVC.TrueHD.7.1-FraMeSToR",
                    "Portmere.2024.1080p.AMZN.WEB-DL.DDP5.1.H.264-NTb",
                    "Portmere.2024.REPACK.1080p.WEB-DL.x265-GRP",
                    "Portmere.2024.PROPER.2160p.NF.WEB-DL.DV.HEVC-FLUX",
                    "Portmere.2024.Extended.Cut.1080p.BluRay.x264-SPARKS",
                    "Portmere.2024.COMPLETE.UHD.BLURAY-TERMiNAL",
                    "Portmere.2024.1080p.WEBRip.AV1.Opus.5.1-GRP",
                ],
            ),
            (
                "Glass Harbor",
                MediaFacet::Series,
                Some(2021),
                &[
                    "Glass.Harbor.S02E04.1080p.WEB-DL.DDP5.1.H.264-GRP",
                    "Glass.Harbor.S02E04.REPACK.2160p.DSNP.WEB-DL.DV.HDR.H.265-FLUX",
                    "Glass.Harbor.S01E01.720p.HDTV.x264-GRP",
                    "Glass.Harbor.S03.1080p.BluRay.REMUX.AVC.DTS-HD.MA.5.1-NOGRP",
                    "Glass.Harbor.S02E04.PROPER.1080p.AMZN.WEBRip.DDP2.0.x264-NTb",
                ],
            ),
            (
                "Umibe Signal",
                MediaFacet::Anime,
                Some(2019),
                &[
                    "[SubsPlease] Umibe Signal - 11 (1080p) [A1B2C3D4].mkv",
                    "Umibe.Signal.S01E11.1080p.CR.WEB-DL.AAC2.0.H.264-VARYG",
                    "[Erai-raws] Umibe Signal - 11v2 [1080p][HEVC][Multiple Subtitle]",
                    "Umibe.Signal.S01E11.UNCENSORED.1080p.BluRay.x265.10bit.FLAC-GRP",
                ],
            ),
        ];

        for (name, facet, year, releases) in corpus {
            let title = parity_title(name, facet.clone(), *year);
            let grab_context =
                crate::acquisition_release_search::canonical_title_evidence(&title).parse_context;

            for release in releases.iter() {
                let at_grab = crate::parse_release_metadata_for_target(release, &grab_context);
                let at_import = parse_import_release_for_title(release, &title);

                for ((field, grab_value), (_, import_value)) in score_relevant_fields(&at_grab)
                    .into_iter()
                    .zip(score_relevant_fields(&at_import))
                {
                    assert_eq!(
                        grab_value, import_value,
                        "`{release}` parsed differently at grab and at import: \
                         {field} is {grab_value} vs {import_value}"
                    );
                }
            }
        }
    }

    /// The one field that genuinely differed: `languages_audio`.
    ///
    /// The parse itself agrees, but the grab lane *enriches* it before scoring —
    /// inferring the title's original language when the name says nothing and
    /// the profile requires one. Import did not, so
    /// `required_audio_language_missing` fired on every release for such a
    /// profile. Both sides go through `announced_metadata_for_title` now.
    #[test]
    fn both_lanes_enrich_audio_languages_the_same_way() {
        let mut title = parity_title("Umibe Signal", MediaFacet::Anime, Some(2019));
        title.language = Some("ja".to_string());

        let profile = crate::QualityProfile::parse(
            r#"{"id":"p","name":"P","criteria":{"quality_tiers":["1080P"],"required_audio_languages":["jpn"]}}"#,
        )
        .expect("profile fixture parses");

        let release = "Umibe.Signal.S01E11.1080p.WEB-DL.H.264-GRP";
        let at_grab = crate::parse_release_metadata_for_target(
            release,
            &crate::acquisition_release_search::canonical_title_evidence(&title).parse_context,
        );
        let at_import = parse_import_release_for_title(release, &title);

        assert!(
            at_grab.languages_audio.is_empty(),
            "fixture precondition: the name says nothing about audio"
        );

        let grab_enriched = crate::quality::canonical_context::announced_metadata_for_title(
            &title,
            &at_grab,
            &profile.criteria.required_audio_languages,
            None,
        );
        let import_enriched = crate::quality::canonical_context::announced_metadata_for_title(
            &title,
            &at_import,
            &profile.criteria.required_audio_languages,
            None,
        );

        assert_eq!(
            grab_enriched.languages_audio,
            import_enriched.languages_audio
        );
        assert!(
            grab_enriched
                .languages_audio
                .iter()
                .any(|code| code == "jpn"),
            "the title's original language must be inferred: {:?}",
            grab_enriched.languages_audio
        );
    }
}
#[cfg(test)]
mod pack_blocklist_ledger_tests {
    use super::{DownloadBlocklistLedger, ImportSkipReason};
    use std::path::Path;

    fn rejection(message: &str) -> crate::post_download_gate::ImportedFileRejection {
        crate::post_download_gate::ImportedFileRejection {
            message: message.to_string(),
            recycle_reason: "truth_blocked",
            skip_reason: Some(ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: vec!["quality_contradicted:1080P->720P".to_string()],
        }
    }

    /// Twelve members of one pack that all trip the same verdict used to write
    /// twelve identical blocklist rows, each naming one episode. The release is
    /// the unit being burned, so it is one row — attributed to every episode the
    /// download covered (review m9).
    #[test]
    fn a_pack_writes_one_row_for_the_union_of_its_members() {
        let mut ledger = DownloadBlocklistLedger {
            collection_id: Some("season-1".to_string()),
            ..DownloadBlocklistLedger::default()
        };
        for (index, episode) in ["ep-1", "ep-2", "ep-3"].iter().enumerate() {
            ledger.record_rejection(
                "Show.S01.1080p.WEB-DL-GRP",
                Path::new("/downloads/pack/file.mkv"),
                &[(*episode).to_string()],
                &rejection(&format!("member {index} lied")),
            );
        }
        // A duplicate episode id from a second file covering the same member.
        ledger.record_rejection(
            "Show.S01.1080p.WEB-DL-GRP",
            Path::new("/downloads/pack/other.mkv"),
            &["ep-2".to_string()],
            &rejection("member 1 lied again"),
        );

        let write = ledger.planned_write().expect("the download earned a write");
        assert_eq!(write.release_title, "Show.S01.1080p.WEB-DL-GRP");
        assert_eq!(
            write.attribution.episode_ids,
            ["ep-1".to_string(), "ep-2".to_string(), "ep-3".to_string()]
        );
        assert_eq!(write.attribution.collection_id, Some("season-1"));
        assert_eq!(
            write
                .rejection
                .expect("a refusal is what the row records")
                .message,
            "member 0 lied",
            "the first refusal stands for the download"
        );
    }

    /// A refusal outranks an imported-but-mis-advertised member: it carries the
    /// recycle reason and it is what reopens the scopes.
    #[test]
    fn a_refusal_outranks_an_import_and_blocklist_in_the_same_download() {
        let mut ledger = DownloadBlocklistLedger::default();
        ledger.record_import_blocklist(
            "Show.S01E01.1080p-GRP",
            Path::new("/downloads/a.mkv"),
            &["ep-1".to_string()],
            "imported at its real quality".to_string(),
        );
        ledger.record_rejection(
            "Show.S01E01.1080p-GRP",
            Path::new("/downloads/b.mkv"),
            &["ep-2".to_string()],
            &rejection("this one lied"),
        );

        let write = ledger.planned_write().expect("the download earned a write");
        assert!(write.rejection.is_some());
        assert_eq!(
            write.attribution.episode_ids,
            ["ep-1".to_string(), "ep-2".to_string()]
        );
        // …but only the refused member is reopened: ep-1 imported (and was
        // marked completed), so flipping it back to `wanted` would leave a
        // wanted row with a file on disk.
        assert_eq!(write.reopen_episode_ids, ["ep-2".to_string()]);
    }

    /// A download where nothing was burned writes nothing.
    #[test]
    fn a_clean_download_earns_no_row() {
        assert!(DownloadBlocklistLedger::default().planned_write().is_none());
    }

    /// A user/system rule veto raised by the probe gate is an import failure:
    /// the release is burned so convergence can try another result.
    #[test]
    fn a_probe_gate_rule_veto_is_blocklisted() {
        let rule_veto = crate::post_download_gate::ImportedFileRejection {
            message: "operator rule refused the file".to_string(),
            recycle_reason: "post_download_rule_blocked",
            skip_reason: Some(scryer_domain::ImportSkipReason::PostDownloadRuleBlocked),
            blocking_rule_codes: vec!["no_eight_bit".to_string()],
        };
        assert_eq!(
            crate::import_decide::prepare_rejection_disposition_for_origin(
                &rule_veto,
                crate::import_decide::ImportOrigin::Automatic,
            ),
            crate::import_decide::RejectionDisposition::Blocklist
        );

        let corrupt = crate::post_download_gate::ImportedFileRejection {
            message: "container could not be read".to_string(),
            recycle_reason: "probe_failed",
            skip_reason: Some(scryer_domain::ImportSkipReason::PolicyMismatch),
            blocking_rule_codes: Vec::new(),
        };
        assert_eq!(
            crate::import_decide::prepare_rejection_disposition_for_origin(
                &corrupt,
                crate::import_decide::ImportOrigin::Automatic,
            ),
            crate::import_decide::RejectionDisposition::Blocklist
        );
    }
}

#[cfg(test)]
#[path = "../app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
