#[cfg(test)]
mod tests {
    use super::{
        COMPLETED_ORIGIN_SCOPE_CONFLICT, CompletedDownloadOriginResolution,
        CompletedDownloadSubmissionMatch, CompletedDownloadSubmissionResolution,
        IMPORT_TRANSFER_HEARTBEAT_INTERVAL, ManualImportCandidateMapping,
        completed_import_status_for_result, resolve_completed_download_origin,
        resolved_episode_ids_are_within_expected, sanitized_title_folder_component,
        should_persist_import_transfer_heartbeat, skip_reason_for_import_check_code,
        validate_manual_import_candidate_mapping_targets,
        validate_manual_import_source_under_trusted_root,
    };
    #[cfg(unix)]
    use super::is_sample_file;
    use crate::{DownloadSubmission, DownloadSubmissionPurpose, SubmissionScope};
    use scryer_domain::MediaFacet;
    use chrono::Utc;
    use scryer_domain::{
        CompletedDownload, ImportDecision, ImportResult, ImportSkipReason, ImportStatus,
    };
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

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

    fn completed_download_with_parameters(parameters: Vec<(&str, &str)>) -> CompletedDownload {
        CompletedDownload {
            client_type: "sabnzbd".to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: "item-1".to_string(),
            download_id: Some("download-1".to_string()),
            name: "Release".to_string(),
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

    fn matched_submission(
        title_id: &str,
        facet: &str,
        scope: SubmissionScope,
    ) -> CompletedDownloadSubmissionResolution {
        CompletedDownloadSubmissionResolution::Matched(Box::new(
            CompletedDownloadSubmissionMatch {
                submission: DownloadSubmission {
                    title_id: title_id.to_string(),
                    facet: facet.to_string(),
                    download_client_id: Some("client-1".to_string()),
                    download_client_type: "sabnzbd".to_string(),
                    download_client_item_id: "item-1".to_string(),
                    source_hint: None,
                    source_provider_id: None,
                    source_provider_name: None,
                    source_kind: None,
                    source_title: None,
                    request_signature: None,
                    purpose: DownloadSubmissionPurpose::Standard,
                    scope,
                },
                identity: None,
            },
        ))
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
    fn completed_origin_resolution_preserves_legacy_collection_for_series_movie() {
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
            Some("legacy-collection-1")
        );
        assert_eq!(
            parameter_value(&resolved.parameters, "*scryer_series_movie_link_id"),
            Some("series-movie-link-1")
        );
    }

    #[test]
    fn completed_origin_resolution_conflicts_on_title_facet_or_scope_mismatch() {
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
            let CompletedDownloadOriginResolution::Conflict { reason, detail } =
                resolve_completed_download_origin(&completed, &resolution)
            else {
                panic!("expected origin conflict");
            };
            assert_eq!(reason, COMPLETED_ORIGIN_SCOPE_CONFLICT);
            assert!(!detail.is_empty());
        }
    }

    #[test]
    fn completed_origin_resolution_preserves_existing_params_for_stub_submission() {
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

        let CompletedDownloadOriginResolution::Ready(resolved) =
            resolve_completed_download_origin(&completed, &resolution)
        else {
            panic!("expected ready completed download");
        };

        assert_eq!(resolved.parameters, completed.parameters);
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
    fn invalid_and_sample_check_codes_are_permanent_policy_mismatches() {
        assert_eq!(
            skip_reason_for_import_check_code("invalid_extension"),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code("sample_file"),
            ImportSkipReason::PolicyMismatch
        );
        assert_eq!(
            skip_reason_for_import_check_code("sample_directory"),
            ImportSkipReason::PolicyMismatch
        );
    }

    #[test]
    fn import_transfer_heartbeat_refreshes_when_progress_stalls() {
        assert!(should_persist_import_transfer_heartbeat(None));
        assert!(!should_persist_import_transfer_heartbeat(Some(
            Instant::now()
        )));

        let stale_emit = Instant::now() - IMPORT_TRANSFER_HEARTBEAT_INTERVAL - Duration::from_secs(1);
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

    #[test]
    fn manual_import_candidate_mapping_validation_requires_unique_candidate_and_exactly_one_target() {
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
        let err =
            validate_manual_import_candidate_mapping_targets(&[movie_with_both], &MediaFacet::Movie)
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
}
#[cfg(test)]
#[path = "../app_usecase_import_tests.rs"]
mod app_usecase_import_tests;
