use super::*;

async fn assert_extended_movie_import(extension: &str, contents: &[u8], allowed: bool) {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    if extension == "m2v" || extension == "avc" {
        let mut profile = scryer_application::builtin_default_quality_profile();
        profile.criteria.quality_tiers.push("480P".into());
        app.save_quality_profile_settings(
            &user,
            SaveQualityProfileSettings {
                profiles: vec![profile],
                replace_existing: true,
                global_profile_id: Some(
                    scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.into(),
                ),
                category_selections: Vec::new(),
                global_scoring_persona: None,
                category_persona_selections: Vec::new(),
            },
        )
        .await
        .unwrap();
    }
    let source_root = tempfile::tempdir().unwrap();
    let source = source_root
        .path()
        .join(format!("Extended.Movie.2024.{extension}"));
    std::fs::write(&source, contents).unwrap();
    let unrelated = source_root.path().join("unrelated.txt");
    std::fs::write(&unrelated, b"preserve unrelated file").unwrap();
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_movie_title(
        &ctx,
        "extended-movie",
        "Extended Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let completed = scryer_completed(
        "extended-movie",
        source.to_str().unwrap(),
        &title.id,
        "movie",
    );
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        import_completed_download(&app, &user, &completed),
    )
    .await
    .unwrap()
    .unwrap();
    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), contents);
    assert_eq!(
        std::fs::read(&unrelated).unwrap(),
        b"preserve unrelated file"
    );
    if allowed {
        assert_eq!(result.decision, ImportDecision::Imported, "{result:?}");
        assert_eq!(files.len(), 1);
        assert_eq!(std::fs::read(&files[0].file_path).unwrap(), contents);
        assert_eq!(files[0].duration_seconds, None);
        if extension == "m2v" || extension == "avc" {
            if extension == "m2v" {
                assert_eq!(files[0].video_codec, None);
                assert!(
                    files[0]
                        .analysis_details
                        .streams
                        .iter()
                        .any(|stream| { stream.codec.as_deref() == Some("mpeg1video") })
                );
            } else {
                assert_eq!(
                    files[0].video_codec,
                    Some(scryer_application::VideoCodec::H264)
                );
            }
            assert!(files[0].video_width.is_some());
            assert!(files[0].video_height.is_some());
        } else {
            assert_eq!(files[0].video_codec, None);
        }
        if extension == "wtv" {
            assert_eq!(
                serde_json::to_value(files[0].analysis_details.report.status).unwrap(),
                serde_json::json!("unsupported")
            );
        }
    } else {
        assert_ne!(result.decision, ImportDecision::Imported, "{result:?}");
        assert!(files.is_empty(), "{files:?}");
        assert!(
            std::fs::read_dir(dest_root.path())
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[tokio::test]
async fn unavailable_probe_import_is_an_operator_decision_even_with_blocking_rules() {
    // Automatic imports stop at the runtime check before any rule runs, and an
    // operator import bypasses user rules, so the operator decides.
    assert_extended_movie_import("wtv", b"unsupported recorder container", false).await;
    assert_manual_unavailable_episode_import(true).await;
}

#[tokio::test]
async fn operator_override_imports_an_episode_whose_runtime_cannot_be_read() {
    assert_manual_unavailable_episode_import(false).await;
}

async fn assert_manual_unavailable_episode_import(with_blocking_rule: bool) {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    if with_blocking_rule {
        install_rule(
            &app,
            &user,
            r#"
import rego.v1

score_entry["unknown_duration"] := scryer.block_score() if {
    input.file != null
    input.file.duration_seconds == null
}
"#,
            vec![MediaFacet::Series],
        )
        .await;
    }
    let source_root = tempfile::tempdir().unwrap();
    let source = source_root.path().join("Extended.Series.S01E01.wtv");
    std::fs::write(&source, b"unsupported recorder container").unwrap();
    pad_file_past_series_sample_threshold(&source);
    let source_bytes = std::fs::read(&source).unwrap();
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_short_form_series_title(
        &ctx,
        "extended-series-manual",
        "Extended Series",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_short_form_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "extended-series-manual",
        source_root.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let import_id = queue_import_record(&ctx, &completed).await;
    let results = tokio::time::timeout(
        Duration::from_secs(60),
        scryer_application::execute_manual_import(
            &app,
            &user,
            &import_id,
            &title.id,
            Some(&completed),
            vec![scryer_application::ManualImportFileMapping {
                disc_selection: None,
                file_path: source.to_string_lossy().to_string(),
                episode_id: Some(episode.id.clone()),
                episode_ids: Vec::new(),
                series_movie_link_id: None,
            }],
            Some(source_root.path().to_path_buf()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(results.len(), 1);
    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert!(results[0].success, "{results:#?}");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(files[0].duration_seconds, None);
    assert_eq!(files[0].video_codec, None);
}

#[tokio::test]
async fn malformed_extended_native_media_never_uses_unavailable_fallback() {
    assert_extended_movie_import("flv", b"FLV\x01truncated header", false).await;
    assert_extended_movie_import("m2v", &[0, 0, 1, 0xb3], false).await;
}

#[tokio::test]
async fn known_audio_only_containers_never_use_unavailable_import_fallback() {
    // Empty FLAC with a complete STREAMINFO block, and a PCM WAVE with one sample.
    let mut flac = b"fLaC\x80\0\0\x22".to_vec();
    flac.extend([0x10, 0, 0x10, 0, 0, 0, 0, 0, 0, 0]);
    flac.extend(((44100u64 << 44) | (15u64 << 36)).to_be_bytes());
    flac.extend([0; 16]);
    let mut wave = b"RIFF".to_vec();
    wave.extend(38u32.to_le_bytes());
    wave.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    wave.extend(44100u32.to_le_bytes());
    wave.extend(88200u32.to_le_bytes());
    wave.extend(b"\x02\0\x10\0data\x02\0\0\0\0\0");
    for bytes in [&flac, &wave] {
        for extension in ["dat", "xvid", "wtv"] {
            assert_extended_movie_import(extension, bytes, false).await;
        }
    }
}

// Sonarr NotSampleSpecification.cs:36-38 rejects an automatic import whose
// runtime cannot be read; the source is preserved and nothing is imported.
#[tokio::test]
async fn unsupported_self_contained_movie_without_runtime_is_not_imported_automatically() {
    assert_extended_movie_import("wtv", b"unsupported recorder container", false).await;
}

#[tokio::test]
async fn partial_elementary_streams_without_runtime_are_not_imported_automatically() {
    assert_extended_movie_import(
        "m2v",
        &[
            0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x13, 0, 0, 0x60, 0, 0, 0, 1, 0, 0, 8,
        ],
        false,
    )
    .await;
    let mut avc = vec![0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128];
    // A separate filler NAL keeps the SPS bounded while meeting the normal
    // 480p size plausibility floor for this one-minute title fixture.
    avc.extend([0, 0, 1, 0x0c]);
    avc.resize(512 * 1024, 0xff);
    avc.push(0x80);
    assert_extended_movie_import("avc", &avc, false).await;
}

#[tokio::test]
async fn remote_reference_playlist_is_not_imported_automatically_without_fetching_its_target() {
    assert_extended_movie_import(
        "m3u",
        b"#EXTM3U\nhttps://unreachable.invalid/video.mkv\n",
        false,
    )
    .await;
}

#[tokio::test]
async fn local_reference_playlists_and_disc_navigation_preserve_all_files() {
    for (extension, contents) in [
        ("m3u", b"#EXTM3U\nvideo.mkv\n".as_slice()),
        ("m3u", b"#EXTM3U\n/nonexistent/video.mkv\n".as_slice()),
        ("ifo", b"DVDVIDEO-VTS\0\0\0\0".as_slice()),
    ] {
        assert_extended_movie_import(extension, contents, false).await;
    }
}

#[tokio::test]
async fn unsupported_self_contained_episode_without_runtime_is_not_imported_automatically() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_root = tempfile::tempdir().unwrap();
    let source = source_root.path().join("Extended.Series.S01E01.wtv");
    std::fs::write(&source, b"unsupported recorder container").unwrap();
    pad_file_past_series_sample_threshold(&source);
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_short_form_series_title(
        &ctx,
        "extended-series",
        "Extended Series",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    seed_short_form_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "extended-series",
        source_root.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        import_completed_download(&app, &user, &completed),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(result.decision, ImportDecision::Imported, "{result:?}");
    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert!(files.is_empty(), "{files:?}");
    assert!(source.exists());
}

#[tokio::test]
async fn unsupported_replacement_preserves_incumbent_and_candidate() {
    assert_unverified_replacement_preserved("wtv", b"unsupported recorder container").await;
}

#[tokio::test]
async fn partial_elementary_replacement_preserves_incumbent_and_candidate() {
    assert_unverified_replacement_preserved(
        "m2v",
        &[
            0, 0, 1, 0xb3, 0x2d, 0x01, 0xe0, 0x13, 0, 0, 0x60, 0, 0, 0, 1, 0, 0, 8,
        ],
    )
    .await;
    assert_unverified_replacement_preserved(
        "avc",
        &[0, 0, 1, 0x67, 66, 0, 30, 244, 5, 1, 236, 128],
    )
    .await;
}

async fn assert_unverified_replacement_preserved(extension: &str, contents: &[u8]) {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_root = tempfile::tempdir().unwrap();
    let source = source_root
        .path()
        .join(format!("Extended.Movie.2024.2160p.{extension}"));
    std::fs::write(&source, contents).unwrap();
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_movie_title(
        &ctx,
        "extended-replacement",
        "Extended Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let incumbent = dest_root.path().join("existing.mkv");
    std::fs::write(&incumbent, b"preserve incumbent media").unwrap();
    let incumbent_id = ctx
        .media_files
        .insert_media_file(&scryer_application::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: incumbent.to_string_lossy().into_owned(),
            size_bytes: 24,
            quality_label: Some("720P".into()),
            acquisition_score: Some(0),
            ..Default::default()
        })
        .await
        .unwrap();
    let completed = scryer_completed(
        "extended-replacement",
        source.to_str().unwrap(),
        &title.id,
        "movie",
    );
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        import_completed_download(&app, &user, &completed),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(result.decision, ImportDecision::Imported, "{result:?}");
    assert_eq!(std::fs::read(&source).unwrap(), contents);
    assert_eq!(
        std::fs::read(&incumbent).unwrap(),
        b"preserve incumbent media"
    );
    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, incumbent_id);
}

#[tokio::test]
async fn known_short_duration_still_rejects_an_extended_container_episode() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_root = tempfile::tempdir().unwrap();
    let source = copy_fixture(
        source_root.path(),
        "ogm_video_audio_text.ogm",
        "Extended.Series.S01E01.720p.ogm",
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        scryer_application::MediaAnalyzer::analyze_file(
            &scryer_application::NativeMediaAnalyzer,
            source.clone(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let scryer_application::MediaAnalysisOutcome::Valid(analysis) = outcome else {
        panic!("OGM fixture must have known video metadata");
    };
    assert_eq!(analysis.duration_seconds, Some(3));
    let source_bytes = std::fs::read(&source).unwrap();
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_series_title(
        &ctx,
        "extended-short-duration",
        "Extended Series",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    seed_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "extended-short-duration",
        source_root.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        import_completed_download(&app, &user, &completed),
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(result.decision, ImportDecision::Imported, "{result:?}");
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()
            .is_empty()
    );
}
