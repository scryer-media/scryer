use super::*;
use crate::ManualImportSourceResolution;
use crate::import_title_resolution::find_monitored_movie_title_from_release;
use crate::missing_required_audio_languages;
use crate::null_repositories::NullSettingsRepository;
use crate::null_repositories::test_nulls::{
    NullDownloadClientConfigRepository, NullIndexerClient, NullQualityProfileRepository,
    NullReleaseAttemptRepository, NullShowRepository, NullUserRepository,
};
use crate::post_download_gate::facet_to_category_hint;
use crate::{
    AppError, AppResult, AppServices, AppUseCase, DownloadClient, DownloadClientAddRequest,
    DownloadGrabResult, FacetRegistry, IndexerConfigRepository, JwtAuthConfig, TitleRepository,
};
use async_trait::async_trait;
use scryer_domain::{CompletedDownload, ExternalId, IndexerConfig, MediaFacet, Title};
use std::sync::Arc;
use tokio::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_title(facet: MediaFacet) -> Title {
    Title {
        id: "t1".to_string(),
        name: "Test Movie".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
        facet,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
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

fn test_parsed() -> crate::ParsedReleaseMetadata {
    crate::parse_release_metadata("Test.Movie.2024.1080p.WEB-DL.DDP5.1.H.264-Group")
}

fn test_movie_title_with_aliases_and_ids(
    id: &str,
    name: &str,
    year: Option<i32>,
    aliases: Vec<&str>,
    external_ids: Vec<(&str, &str)>,
) -> Title {
    let mut title = test_title(MediaFacet::Movie);
    title.id = id.to_string();
    title.name = name.to_string();
    title.year = year;
    title.aliases = aliases.into_iter().map(str::to_string).collect();
    title.external_ids = external_ids
        .into_iter()
        .map(|(source, value)| ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        })
        .collect();
    title
}

fn test_completed_download(name: &str, dest_dir: &std::path::Path) -> CompletedDownload {
    CompletedDownload {
        client_type: "weaver".to_string(),
        client_id: "client-1".to_string(),
        download_client_item_id: "job-1".to_string(),
        download_id: None,
        name: name.to_string(),
        dest_dir: dest_dir.to_string_lossy().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

fn test_manual_import_payload(files: Vec<ManualImportFileMapping>) -> ManualImportRequestPayload {
    ManualImportRequestPayload {
        requested_by_user_id: Some("user-1".to_string()),
        title_id: Some("title-1".to_string()),
        download_client_item_id: "job-1".to_string(),
        client_id: Some("client-1".to_string()),
        client_type: "weaver".to_string(),
        files,
        requested_at: chrono::Utc::now().to_rfc3339(),
    }
}

// ── has_scryer_origin ─────────────────────────────────────────────────────────

#[test]
fn has_scryer_origin_with_title_id() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert!(has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_without_title_id() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert!(!has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert!(!has_scryer_origin(&params));
}

// ── extract_parameter ─────────────────────────────────────────────────────────

#[test]
fn extract_parameter_found() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert_eq!(
        extract_parameter(&params, "*scryer_title_id"),
        Some("abc-123".to_string())
    );
}

#[test]
fn extract_parameter_not_found() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert_eq!(extract_parameter(&params, "*scryer_title_id"), None);
}

#[test]
fn extract_parameter_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert_eq!(extract_parameter(&params, "anything"), None);
}

#[test]
fn extract_parameter_first_match() {
    let params = vec![
        ("key".to_string(), "first".to_string()),
        ("key".to_string(), "second".to_string()),
    ];
    assert_eq!(extract_parameter(&params, "key"), Some("first".to_string()));
}

#[test]
fn queued_mapped_manual_import_allows_missing_source() {
    let payload = test_manual_import_payload(vec![ManualImportFileMapping {
        file_path: "/downloads/episode.mkv".to_string(),
        episode_id: Some("ep-1".to_string()),
        series_movie_link_id: None,
        quality: None,
    }]);

    let result = resolve_queued_manual_import_completed_source(
        "import-1",
        &payload,
        ManualImportSourceResolution::NotEligible {
            message: "download no longer available".to_string(),
        },
    );

    assert!(matches!(result, Ok(None)));
}

#[test]
fn queued_unmapped_manual_import_fails_when_source_is_missing() {
    let payload = test_manual_import_payload(Vec::new());

    let result = resolve_queued_manual_import_completed_source(
        "import-1",
        &payload,
        ManualImportSourceResolution::SourceFailed {
            message: "download no longer available".to_string(),
        },
    );

    assert!(matches!(result, Err((ImportStatus::Failed, Some(_)))));
}

// ── normalize_imdb_id ─────────────────────────────────────────────────────────

#[test]
fn normalize_imdb_id_with_prefix() {
    assert_eq!(
        normalize_imdb_id("tt1234567"),
        Some("tt1234567".to_string())
    );
}

#[test]
fn normalize_imdb_id_digits_only() {
    assert_eq!(normalize_imdb_id("1234567"), Some("tt1234567".to_string()));
}

#[test]
fn normalize_imdb_id_with_extra_chars() {
    assert_eq!(
        normalize_imdb_id("tt0123456abc"),
        Some("tt0123456".to_string())
    );
}

#[test]
fn normalize_imdb_id_empty() {
    assert_eq!(normalize_imdb_id(""), None);
}

#[test]
fn normalize_imdb_id_no_digits() {
    assert_eq!(normalize_imdb_id("abcdef"), None);
}

// ── movie title resolution ───────────────────────────────────────────────────

#[test]
fn find_monitored_movie_title_from_release_matches_alias_variant() {
    let titles = vec![test_movie_title_with_aliases_and_ids(
        "movie-1",
        "My Cousin",
        Some(2020),
        vec!["Mon Cousin"],
        vec![],
    )];

    let parsed =
        crate::parse_release_metadata("Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.x264-GRP");

    let matched = find_monitored_movie_title_from_release(&titles, &parsed)
        .expect("movie should resolve through alias/title variants");

    assert_eq!(matched.id, "movie-1");
}

#[test]
fn find_monitored_movie_title_from_release_matches_tagged_alias_variant() {
    let mut title =
        test_movie_title_with_aliases_and_ids("movie-1", "Nightfall!!", Some(2022), vec![], vec![]);
    title.tagged_aliases = vec![scryer_domain::TaggedAlias {
        name: "Nightfall Heavy Metal Dark Fantasy".to_string(),
        language: "eng".to_string(),
    }];

    let parsed =
        crate::parse_release_metadata("NIGHTFALL.Heavy.Metal.Dark.Fantasy.2022.1080p.WEB-DL");

    let matched = find_monitored_movie_title_from_release(&[title], &parsed)
        .expect("movie should resolve through tagged alias variants");

    assert_eq!(matched.id, "movie-1");
}

#[test]
fn find_monitored_movie_title_from_release_prefers_imdb_id() {
    let titles = vec![
        test_movie_title_with_aliases_and_ids(
            "movie-1",
            "Glass Harbor",
            Some(1984),
            vec![],
            vec![("imdb", "tt0087182")],
        ),
        test_movie_title_with_aliases_and_ids(
            "movie-2",
            "Glass Harbor",
            Some(2021),
            vec![],
            vec![("imdb", "tt1160419"), ("tmdb", "438631")],
        ),
    ];

    let parsed = crate::parse_release_metadata(
        "Glass.Harbor.2021.{tmdb-438631}.[tt1160419].1080p.BluRay.x264-GRP",
    );

    let matched = find_monitored_movie_title_from_release(&titles, &parsed)
        .expect("movie should resolve by embedded IDs");

    assert_eq!(matched.id, "movie-2");
}

#[test]
fn build_augmented_movie_import_metadata_prefers_download_title_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Paper.Lantern.2012.1080p.BluRay.x264-GRP");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");
    let completed = test_completed_download("Paper.Lantern.2012.1080p.BluRay.x264-GRP", &dest_dir);

    let parsed = build_augmented_movie_import_metadata(&file_path, &completed);

    assert_eq!(parsed.year, Some(2012));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(
        parsed.source.as_ref().map(|source| source.as_str()),
        Some("BluRay")
    );
}

#[test]
fn build_augmented_movie_import_metadata_uses_immediate_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("job-123");
    let release_dir = dest_dir.join("Paper.Lantern.2012.1080p.BluRay.x264-GRP");
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");
    let completed = test_completed_download("job-123", &dest_dir);

    let parsed = build_augmented_movie_import_metadata(&file_path, &completed);

    assert_eq!(parsed.normalized_title, "PAPER LANTERN");
    assert_eq!(parsed.year, Some(2012));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(
        parsed.source.as_ref().map(|source| source.as_str()),
        Some("BluRay")
    );
}

#[test]
fn build_augmented_episode_import_metadata_prefers_download_title_for_single_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir
        .path()
        .join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed =
        test_completed_download("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, false);
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

#[test]
fn build_augmented_episode_import_metadata_uses_immediate_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("job-123");
    let release_dir = dest_dir.join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("job-123", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, false);
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(parsed.normalized_title, "HARBOR PALS");
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

#[test]
fn build_augmented_episode_import_metadata_keeps_file_episode_when_other_files_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Harbor.Pals.S01.Complete.720p.WEB-DL.AV1");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("Harbor.Pals.S01E03.720p.WEB-DL.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("Harbor.Pals.S01.Complete.720p.WEB-DL.AV1", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, true);
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![3]);
}

#[test]
fn build_augmented_episode_import_metadata_does_not_infer_episode_from_download_title_when_other_files_exist()
 {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir
        .path()
        .join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed =
        test_completed_download("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, true);

    assert!(parsed.episode.is_none());
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

#[test]
fn title_evidence_candidates_from_video_files_uses_immediate_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let release_dir = dir.path().join(
        "Harry.Potter.And.The.Deathly.Hallows.Part1.2010.720p.BluRay.DTS.x264-LEGION-Obfuscated",
    );
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path =
        release_dir.join("aUUKqrO833LbSr7VlByumnR24y7ULADpVJ7K0FTnPhPMqpp0KIIaLSLYXJmyjm.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");

    let candidates = title_evidence_candidates_from_video_files(&[file_path]);

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].normalized_title,
        "HARRY POTTER AND THE DEATHLY HALLOWS PART 1"
    );
    assert_eq!(candidates[0].year, Some(2010));
}

#[test]
fn title_evidence_candidates_from_video_files_prefers_usable_file_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let release_dir = dir.path().join("Generic");
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path = release_dir.join("Paper.Lantern.2012.1080p.BluRay.x264-GRP.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");

    let candidates = title_evidence_candidates_from_video_files(&[file_path]);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].normalized_title, "PAPER LANTERN");
    assert_eq!(candidates[0].year, Some(2012));
}

// ── is_sample_file ────────────────────────────────────────────────────────────

#[test]
fn is_sample_file_detects_sample_in_stem() {
    assert!(is_sample_file(std::path::Path::new(
        "/data/episode.sample.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new(
        "/data/sample-show.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new("/data/SAMPLE.mkv")));
}

#[test]
fn is_sample_file_allows_normal_video_file() {
    // Non-existent path → metadata fails → size defaults to 0, but file doesn't
    // contain "sample" so the filename check returns false; the size check on a
    // nonexistent file returns Ok(0) via unwrap_or(false)... actually
    // std::fs::metadata on a non-existent path returns Err, so unwrap_or(false)
    // → false. So this test should pass.
    assert!(!is_sample_file(std::path::Path::new(
        "/nonexistent/Show.S01E01.1080p.mkv"
    )));
    assert!(!is_sample_file(std::path::Path::new(
        "/nonexistent/Movie.2024.mkv"
    )));
}

// ── pick_largest_file ─────────────────────────────────────────────────────────

#[test]
fn pick_largest_file_empty_list_returns_error() {
    let result = pick_largest_file(&[]);
    assert!(result.is_err());
}

#[test]
fn pick_largest_file_single_file_returns_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("only.mkv");
    std::fs::write(&path, b"content").expect("write");
    let result = pick_largest_file(std::slice::from_ref(&path));
    assert_eq!(result.expect("pick"), path);
}

#[test]
fn pick_largest_file_returns_biggest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let small = dir.path().join("small.mkv");
    let large = dir.path().join("large.mkv");
    let tiny = dir.path().join("tiny.mkv");
    std::fs::write(&small, vec![0u8; 100]).expect("write small");
    std::fs::write(&large, vec![0u8; 1000]).expect("write large");
    std::fs::write(&tiny, vec![0u8; 10]).expect("write tiny");
    let result = pick_largest_file(&[small, large.clone(), tiny]);
    assert_eq!(result.expect("pick"), large);
}

// ── use_season_folders ────────────────────────────────────────────────────────

#[test]
fn use_season_folders_true_when_tag_absent() {
    let title = test_title(MediaFacet::Series);
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_true_when_tag_enabled() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:enabled".to_string()];
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_false_when_tag_disabled() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:disabled".to_string()];
    assert!(!use_season_folders(&title));
}

#[test]
fn use_season_folders_false_case_insensitive() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:DISABLED".to_string()];
    assert!(!use_season_folders(&title));
}

// ── build_rename_tokens ───────────────────────────────────────────────────────

#[test]
fn build_rename_tokens_includes_title_and_year() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("title").map(String::as_str), Some("Test Movie"));
    assert_eq!(tokens.get("ext").map(String::as_str), Some("mkv"));
    assert_eq!(tokens.get("year").map(String::as_str), Some("2024"));
}

#[test]
fn build_rename_tokens_falls_back_to_title_year_when_release_year_is_missing() {
    let title = test_title(MediaFacet::Movie);
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("year").map(String::as_str), Some("2024"));
}

#[test]
fn build_rename_tokens_includes_quality() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("quality").map(String::as_str), Some("1080p"));
}

fn test_media_analysis(video_height: Option<i32>) -> crate::MediaFileAnalysis {
    crate::MediaFileAnalysis {
        video_codec: Some(crate::release_parser::VideoCodec::H264),
        video_width: Some(1920),
        video_height,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: Some("aac".to_string()),
        audio_profile: None,
        audio_channels: Some(2),
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: None,
    }
}

fn test_rule_file_doc(
    dovi_profile: Option<u8>,
    dovi_bl_compat_id: Option<u8>,
) -> scryer_rules::FileDoc {
    scryer_rules::FileDoc {
        video_codec: Some("hevc".to_string()),
        video_width: Some(3840),
        video_height: Some(2160),
        video_bitrate_kbps: None,
        video_bit_depth: Some(10),
        video_hdr_format: Some("Dolby Vision".to_string()),
        dovi_profile,
        dovi_bl_compat_id,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: Some("eac3".to_string()),
        audio_profile: None,
        audio_channels: Some(6),
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: Some("Matroska".to_string()),
    }
}

fn post_download_test_profile() -> crate::QualityProfile {
    crate::QualityProfile::parse(
        r#"{
            "id": "test",
            "name": "Test",
            "criteria": {
                "quality_tiers": ["2160p", "1080p", "720p"],
                "allow_unknown_quality": false,
                "allow_upgrades": true
            }
        }"#,
    )
    .expect("quality profile should parse")
}

#[test]
fn rescore_from_mediainfo_updates_quality_when_parsed_quality_is_missing() {
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(1080))),
        scan_error: None,
        rule_file_doc: None,
    };

    let (rescored, changes) =
        crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);

    assert_eq!(rescored.quality.as_deref(), Some("1080p"));
    assert!(changes.iter().any(|change| change.contains("resolution")));
}

#[tokio::test]
async fn post_download_score_uses_rescored_quality_and_records_negative_audit() {
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    );
    let title = test_title(MediaFacet::Movie);
    let profile = crate::QualityProfile::parse(
        r#"{
            "id": "test",
            "name": "Test",
            "criteria": {
                "quality_tiers": ["1080p", "720p"],
                "allow_unknown_quality": false,
                "allow_upgrades": true
            }
        }"#,
    )
    .expect("quality profile should parse");
    let parsed = crate::parse_release_metadata("Test.Movie.2024.1080p.WEB-DL.H264.AAC2.0-GRP");
    let mut analysis = test_media_analysis(Some(720));
    analysis.video_width = Some(1280);
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(analysis),
        scan_error: None,
        rule_file_doc: None,
    };

    let result = crate::post_download_gate::compute_post_download_acquisition_decision(
        &app,
        &parsed,
        &acceptance,
        &profile,
        &title,
        title.runtime_minutes,
        5 * 1024 * 1024,
        false,
        None,
        &[],
        false,
    )
    .await;

    assert_eq!(result.parsed.quality.as_deref(), Some("720p"));
    assert!(result.score < 0);
    let scoring_log = result.scoring_log.expect("scoring log should serialize");
    let scoring_log: serde_json::Value =
        serde_json::from_str(&scoring_log).expect("scoring log should be JSON");
    assert_eq!(
        scoring_log["kind"],
        serde_json::Value::String("post_download_acquisition_score".to_string())
    );
    assert_eq!(scoring_log["preference_score"], result.score);
    assert!(
        scoring_log["rescore_changes"]
            .as_array()
            .expect("rescore changes should be an array")
            .iter()
            .any(|change| change.as_str().is_some_and(|value| value.contains("resolution")))
    );
}

#[tokio::test]
async fn post_download_score_preserves_prepared_rescore_changes_when_parsed_already_rescored() {
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    );
    let title = test_title(MediaFacet::Movie);
    let profile = post_download_test_profile();
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(1080))),
        scan_error: None,
        rule_file_doc: None,
    };
    let (prepared_parsed, first_pass_changes) =
        crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);
    assert!(
        first_pass_changes
            .iter()
            .any(|change| change.contains("resolution"))
    );

    let result = crate::post_download_gate::compute_post_download_acquisition_decision(
        &app,
        &prepared_parsed,
        &acceptance,
        &profile,
        &title,
        title.runtime_minutes,
        5 * 1024 * 1024,
        false,
        None,
        &first_pass_changes,
        false,
    )
    .await;

    let scoring_log = result.scoring_log.expect("scoring log should serialize");
    let scoring_log: serde_json::Value =
        serde_json::from_str(&scoring_log).expect("scoring log should be JSON");
    assert!(
        scoring_log["rescore_changes"]
            .as_array()
            .expect("rescore changes should be an array")
            .iter()
            .any(|change| change.as_str().is_some_and(|value| value.contains("resolution")))
    );
}

#[tokio::test]
async fn post_download_user_rule_scoring_uses_probe_file_doc_dovi_facts() {
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    );
    let policy = scryer_rules::UserPolicy {
        id: "dv_profile".to_string(),
        name: "DV Profile".to_string(),
        rego_source: scryer_rules::rewrite_package_declaration(
            r#"
score_entry["dv_profile_bonus"] := 123 if {
    input.file != null
    input.file.dovi_profile == 8
    input.file.dovi_bl_compat_id == 1
}
"#,
            "dv_profile",
        ),
        applied_facets: vec!["movie".to_string()],
    };
    let engine = scryer_rules::UserRulesEngine::build(&[policy])
        .expect("user rule engine should compile");
    *app.services
        .customization
        .user_rules
        .write()
        .expect("user rules lock should be writable") = engine;

    let title = test_title(MediaFacet::Movie);
    let profile = post_download_test_profile();
    let parsed = crate::parse_release_metadata("Test.Movie.2024.2160p.WEB-DL.HEVC-GRP");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(2160))),
        scan_error: None,
        rule_file_doc: Some(test_rule_file_doc(Some(8), Some(1))),
    };

    let result = crate::post_download_gate::compute_post_download_acquisition_decision(
        &app,
        &parsed,
        &acceptance,
        &profile,
        &title,
        title.runtime_minutes,
        5 * 1024 * 1024,
        false,
        None,
        &[],
        false,
    )
    .await;

    let scoring_log = result.scoring_log.expect("scoring log should serialize");
    let scoring_log: serde_json::Value =
        serde_json::from_str(&scoring_log).expect("scoring log should be JSON");
    assert!(
        scoring_log["scoring_log"]
            .as_array()
            .expect("scoring log should be an array")
            .iter()
            .any(|entry| {
                entry["code"] == "dv_profile_bonus"
                    && entry["delta"] == 123
                    && entry["source"]["kind"] == "user_rule"
                    && entry["source"]["id"] == "dv_profile"
            })
    );
}

#[test]
fn episode_import_dest_path_uses_rescored_parsed_quality_without_override() {
    let mut title = test_title(MediaFacet::Series);
    title.name = "Test Show".to_string();
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(1080))),
        scan_error: None,
        rule_file_doc: None,
    };
    let (rescored, _) = crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);

    let dest_path = episode_import_dest_path(
        &title,
        &rescored,
        "mkv",
        std::path::Path::new("/downloads/obfuscated.release.name.mkv"),
        std::path::Path::new("/library/Test Show"),
        true,
        "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        "Season {season:2}",
        8,
        "7",
        None,
        None,
        None,
    );

    assert_eq!(
        dest_path,
        std::path::PathBuf::from("/library/Test Show/Season 08/Test Show - S08E07 - 1080p.mkv")
    );
}

#[test]
fn episode_import_dest_path_preserves_source_filename_when_renamer_disabled() {
    let mut title = test_title(MediaFacet::Series);
    title.name = "Test Show".to_string();
    let parsed = crate::parse_release_metadata("obfuscated.release.name");

    let dest_path = episode_import_dest_path(
        &title,
        &parsed,
        "mkv",
        std::path::Path::new("/downloads/Obfuscated.Source.Name.mkv"),
        std::path::Path::new("/library/Test Show"),
        false,
        "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        "Season {season:2}",
        8,
        "7",
        None,
        None,
        None,
    );

    assert_eq!(
        dest_path,
        std::path::PathBuf::from("/library/Test Show/Season 08/Obfuscated.Source.Name.mkv")
    );
}

#[test]
fn build_rename_tokens_episode_is_empty_for_movie() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("season").map(String::as_str), Some(""));
    assert_eq!(tokens.get("episode").map(String::as_str), Some(""));
}

#[test]
fn build_rename_tokens_episode_metadata_for_series() {
    let title = test_title(MediaFacet::Series);
    let parsed = crate::parse_release_metadata("Show.S02E05.720p.HDTV.mkv");
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("season").map(String::as_str), Some("2"));
    assert_eq!(tokens.get("episode").map(String::as_str), Some("5"));
}

// ── find_video_files ──────────────────────────────────────────────────────────

#[test]
fn find_video_files_finds_mkv_in_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("movie.mkv"), b"data").expect("write");
    std::fs::write(dir.path().join("notes.txt"), b"text").expect("write");
    let files = find_video_files(dir.path(), false).expect("find");
    assert_eq!(files.len(), 1);
    assert!(files[0].to_str().unwrap().ends_with("movie.mkv"));
}

#[test]
fn find_video_files_includes_trailing_sanitized_video_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sanitized_path = dir.path().join("Fixture.Payload.mkv_");
    let quoted_path = dir.path().join("Fixture.Payload.mkv\"");
    let executable_path = dir.path().join("Fixture.Payload.mkv.exe");
    std::fs::write(&sanitized_path, b"data").expect("write sanitized");
    std::fs::write(&quoted_path, b"data").expect("write quoted");
    std::fs::write(&executable_path, b"data").expect("write executable");

    let mut files = find_video_files(dir.path(), false).expect("find");
    files.sort();

    assert_eq!(files, vec![quoted_path, sanitized_path]);
}

#[test]
fn preserved_import_filename_sanitizes_trailing_bad_chars() {
    assert_eq!(
        preserved_import_filename(std::path::Path::new("Fixture.Payload.mkv\"")),
        "Fixture.Payload.mkv"
    );
}

#[test]
fn find_video_files_filters_samples_when_flag_set() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir().expect("tempdir");

    // movie.mkv must be >= 50 MB so the size check doesn't also flag it as a sample.
    // Use a sparse file (seek past threshold, write one byte) to avoid allocating 50 MB.
    let main_path = dir.path().join("movie.mkv");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&main_path)
        .expect("open main");
    f.seek(SeekFrom::Start(52 * 1024 * 1024)).expect("seek");
    f.write_all(b"\0").expect("write");
    drop(f);

    // sample file — name alone triggers filtering regardless of size
    std::fs::write(dir.path().join("movie.sample.mkv"), b"data").expect("write sample");

    let files = find_video_files(dir.path(), true).expect("find");
    // sample file is filtered; only movie.mkv remains
    assert_eq!(files.len(), 1);
    assert!(!files[0].to_str().unwrap().contains("sample"));
}

#[test]
fn find_video_files_returns_error_for_missing_dir() {
    let result = find_video_files(std::path::Path::new("/nonexistent/dir/abc"), false);
    assert!(result.is_err());
}

#[test]
fn find_video_files_recurses_into_subdirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let subdir = dir.path().join("season1");
    std::fs::create_dir(&subdir).expect("mkdir");
    std::fs::write(subdir.join("ep1.mkv"), b"data").expect("write");
    std::fs::write(dir.path().join("ep2.mp4"), b"data").expect("write");
    let files = find_video_files(dir.path(), false).expect("find");
    assert_eq!(files.len(), 2);
}

#[cfg(unix)]
#[test]
fn find_video_files_follows_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("season1");
    std::fs::create_dir(&target).expect("mkdir");
    std::fs::write(target.join("ep1.mkv"), b"data").expect("write");
    symlink(&target, dir.path().join("linked-season1")).expect("symlink");

    let files = find_video_files(dir.path(), false).expect("find");

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("linked-season1/ep1.mkv"));
}

// ── missing_audio_languages ───────────────────────────────────────────────────

#[test]
fn missing_audio_languages_all_present() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["jpn".to_string(), "eng".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_case_normalization() {
    // media analysis emits lowercase codes; profile stores uppercase
    let required = vec!["JPN".to_string()];
    let actual = vec!["jpn".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_accepts_full_iso_language_names() {
    let required = vec!["Filipino".to_string()];
    let actual = vec!["fil-PH".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_one_missing() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["eng".to_string()];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["jpn"]);
}

#[test]
fn missing_audio_languages_all_missing() {
    let required = vec!["JPN".to_string()];
    let actual = vec!["eng".to_string(), "spa".to_string()];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["jpn"]);
}

#[test]
fn missing_audio_languages_empty_required_always_passes() {
    let required: Vec<String> = vec![];
    let actual = vec!["eng".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_empty_actual_returns_all_required() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual: Vec<String> = vec![];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing.len(), 2);
}

// ── facet_to_category_hint ────────────────────────────────────────────────────

#[test]
fn facet_to_category_hint_values() {
    assert_eq!(facet_to_category_hint(&MediaFacet::Movie), "movie");
    assert_eq!(facet_to_category_hint(&MediaFacet::Series), "series");
    assert_eq!(facet_to_category_hint(&MediaFacet::Anime), "anime");
}

fn scoped_media_file(
    id: &str,
    file_path: &str,
    acquisition_score: i32,
    episode_ids: &[&str],
) -> crate::EpisodeScopedMediaFile {
    crate::EpisodeScopedMediaFile {
        media_file: crate::TitleMediaFile {
            id: id.to_string(),
            title_id: "title-1".to_string(),
            episode_id: episode_ids.first().map(|value| (*value).to_string()),
            series_movie_link_ids: Vec::new(),
            role: crate::MediaFileRole::Primary,
            file_path: file_path.to_string(),
            size_bytes: 1_000,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("1080p".to_string()),
            scan_status: "scanned".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_languages: Vec::new(),
            subtitle_codecs: Vec::new(),
            subtitle_streams: Vec::new(),
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: None,
            release_group: None,
            source_type: None,
            resolution: None,
            video_codec_parsed: None,
            audio_codec_parsed: None,
            audio_channels_parsed: None,
            acquisition_score: Some(acquisition_score),
            scoring_log: None,
            indexer_source: None,
            grabbed_release_title: None,
            grabbed_at: None,
            edition: None,
            original_file_path: None,
            release_hash: None,
        },
        episode_ids: episode_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

#[test]
fn build_episode_upgrade_plan_replaces_different_filename_when_new_score_is_higher() {
    let incumbents = vec![scoped_media_file(
        "file-1",
        "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01 - 720p.mkv",
        510,
        &["ep-1"],
    )];

    let plan = build_episode_upgrade_plan(&incumbents, &["ep-1".to_string()], 900)
        .expect("upgrade plan should accept higher-scored replacement");

    assert_eq!(plan.primary_incumbent.media_file.id, "file-1");
    assert_eq!(plan.previous_best_score, 510);
    assert!(plan.additional_superseded.is_empty());
}

#[test]
fn build_episode_upgrade_plan_rejects_when_existing_episode_file_scores_higher() {
    let incumbents = vec![scoped_media_file(
        "file-1",
        "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01 - 1080p.mkv",
        820,
        &["ep-1"],
    )];

    let rejection =
        build_episode_upgrade_plan(&incumbents, &["ep-1".to_string()], 700).unwrap_err();

    assert_eq!(
        rejection.skip_reason,
        Some(ImportSkipReason::AlreadyImported)
    );
    assert!(rejection.message.contains("equal or better"));
}

#[test]
fn build_episode_upgrade_plan_rejects_when_existing_file_covers_broader_episode_set() {
    let incumbents = vec![scoped_media_file(
        "file-pack",
        "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01-E02.mkv",
        400,
        &["ep-1", "ep-2"],
    )];

    let rejection =
        build_episode_upgrade_plan(&incumbents, &["ep-1".to_string()], 900).unwrap_err();

    assert_eq!(
        rejection.skip_reason,
        Some(ImportSkipReason::PolicyMismatch)
    );
    assert!(rejection.message.contains("broader episode set"));
}

#[test]
fn manual_import_error_from_skip_reason_maps_policy_mismatch() {
    assert_eq!(
        manual_import_error_from_skip_reason(Some(ImportSkipReason::PolicyMismatch)),
        scryer_domain::ImportErrorCode::PolicyMismatch
    );
}

#[test]
fn prefer_broader_coverage_episodes_returns_claimed_pack() {
    let target = vec![scryer_domain::Episode {
        id: "ep-1".to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some("season-1".to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Episode 1".to_string()),
        air_date: None,
        duration_seconds: Some(24 * 60),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    }];
    let mut claimed = target.clone();
    claimed.push(scryer_domain::Episode {
        id: "ep-2".to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some("season-1".to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("2".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E02".to_string()),
        title: Some("Episode 2".to_string()),
        air_date: None,
        duration_seconds: Some(24 * 60),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    });

    let coverage = prefer_broader_coverage_episodes(&target, claimed);

    assert_eq!(coverage.len(), 2);
    assert_eq!(coverage[0].id, "ep-1");
    assert_eq!(coverage[1].id, "ep-2");
}

#[test]
fn parsed_with_quality_override_replaces_parsed_quality() {
    let parsed = test_parsed();

    let effective = parsed_with_quality_override(&parsed, Some("2160P"));

    assert_eq!(effective.quality.as_deref(), Some("2160P"));
}

#[test]
fn build_episode_upgrade_plan_supersedes_all_duplicate_incumbents_for_same_target_set() {
    let incumbents = vec![
        scoped_media_file(
            "file-1",
            "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01 - 720p.mkv",
            300,
            &["ep-1"],
        ),
        scoped_media_file(
            "file-2",
            "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01 - 1080p.mkv",
            500,
            &["ep-1"],
        ),
    ];

    let plan = build_episode_upgrade_plan(&incumbents, &["ep-1".to_string()], 900)
        .expect("higher score should supersede all incumbents");

    assert_eq!(plan.primary_incumbent.media_file.id, "file-2");
    assert_eq!(plan.additional_superseded.len(), 1);
    assert_eq!(plan.additional_superseded[0].media_file.id, "file-1");
}

#[test]
fn build_episode_upgrade_plan_allows_pack_to_replace_singles_when_it_beats_all_of_them() {
    let incumbents = vec![
        scoped_media_file(
            "file-1",
            "/data/TV/Resident Alien/Season 01/Resident Alien - S01E01.mkv",
            300,
            &["ep-1"],
        ),
        scoped_media_file(
            "file-2",
            "/data/TV/Resident Alien/Season 01/Resident Alien - S01E02.mkv",
            450,
            &["ep-2"],
        ),
    ];

    let plan =
        build_episode_upgrade_plan(&incumbents, &["ep-1".to_string(), "ep-2".to_string()], 900)
            .expect("season pack should replace lower-scored singles");

    assert_eq!(plan.previous_best_score, 450);
    assert_eq!(plan.additional_superseded.len(), 1);
}

#[derive(Default)]
struct ManualImportCleanupTitleRepo {
    titles: Mutex<Vec<Title>>,
}

#[async_trait]
impl TitleRepository for ManualImportCleanupTitleRepo {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let titles = self.titles.lock().await.clone();
        Ok(titles
            .into_iter()
            .filter(|title| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &title.facet == expected)
            })
            .filter(|title| {
                query.as_ref().is_none_or(|needle| {
                    title
                        .name
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase())
                })
            })
            .collect())
    }

    async fn list_by_external_ids(&self, _: &str, _: &[String]) -> AppResult<Vec<Title>> {
        Ok(vec![])
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        Ok(self
            .titles
            .lock()
            .await
            .iter()
            .find(|title| title.id == id)
            .cloned())
    }

    async fn get_by_facet_and_slug(&self, _: MediaFacet, _: &str) -> AppResult<Option<Title>> {
        Ok(None)
    }

    async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
        Ok(None)
    }

    async fn find_by_external_id_in_facet(
        &self,
        _: MediaFacet,
        _: &str,
        _: &str,
    ) -> AppResult<Option<Title>> {
        Ok(None)
    }

    async fn create_or_get_existing(&self, _: Title) -> AppResult<crate::CreateTitleOutcome> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn create(&self, _: Title) -> AppResult<Title> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn list_titles_due_for_hydration(
        &self,
        _: usize,
        _: &[MediaFacet],
    ) -> AppResult<Vec<crate::PendingTitleHydration>> {
        Ok(vec![])
    }

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        _: usize,
    ) -> AppResult<Vec<String>> {
        Ok(vec![])
    }

    async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn update_metadata(
        &self,
        _: &str,
        _: Option<String>,
        _: Option<MediaFacet>,
        _: Option<Vec<String>>,
        _: Option<String>,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn update_title_hydrated_metadata(
        &self,
        _: &str,
        _: crate::TitleMetadataUpdate,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn replace_match_state(
        &self,
        _: &str,
        _: Vec<scryer_domain::ExternalId>,
        _: Vec<String>,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
struct ManualImportCleanupDownloadClient {
    deleted_items: Mutex<Vec<(String, bool)>>,
}

#[async_trait]
impl DownloadClient for ManualImportCleanupDownloadClient {
    async fn submit_download(&self, _: &DownloadClientAddRequest) -> AppResult<DownloadGrabResult> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        self.deleted_items
            .lock()
            .await
            .push((id.to_string(), is_history));
        Ok(())
    }
}

struct ManualImportCleanupIndexerConfigRepo;

#[async_trait]
impl IndexerConfigRepository for ManualImportCleanupIndexerConfigRepo {
    async fn list(&self, _: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<IndexerConfig>> {
        Ok(None)
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        Ok(config)
    }

    async fn touch_last_error(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update(&self, _: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

fn build_manual_import_cleanup_app(
    titles: Vec<Title>,
    download_client: Arc<dyn DownloadClient>,
) -> AppUseCase {
    let services = AppServices::builder(
        Arc::new(ManualImportCleanupTitleRepo {
            titles: Mutex::new(titles),
        }),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        Arc::new(ManualImportCleanupIndexerConfigRepo),
        Arc::new(NullIndexerClient),
        download_client,
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    )
    .build_partial_for_tests();

    AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(FacetRegistry::new()),
    )
}

#[tokio::test]
async fn maybe_remove_completed_manual_import_download_deletes_history_for_collection_success() {
    let mut title = test_title(MediaFacet::Series);
    title.id = "series-1".to_string();
    title.name = "Harbor Pals".to_string();

    let download_client = Arc::new(ManualImportCleanupDownloadClient::default());
    let app = build_manual_import_cleanup_app(vec![title], download_client.clone());
    let dir = tempfile::tempdir().expect("tempdir");
    let completed = test_completed_download("Harbor.Pals.S01.Complete.1080p.WEB-DL", dir.path());

    maybe_remove_completed_manual_import_download(&app, Some(&completed), Some("series-1"), true)
        .await;

    assert_eq!(
        *download_client.deleted_items.lock().await,
        vec![("job-1".to_string(), true)]
    );
}

#[tokio::test]
async fn maybe_remove_completed_manual_import_download_deletes_history_for_episode_set_success() {
    let mut title = test_title(MediaFacet::Anime);
    title.id = "anime-1".to_string();
    title.name = "Silver Horizon".to_string();

    let download_client = Arc::new(ManualImportCleanupDownloadClient::default());
    let app = build_manual_import_cleanup_app(vec![title], download_client.clone());
    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed =
        test_completed_download("Silver Horizon.S01E03-E04.1080p.WEB-DL", dir.path());
    completed.download_client_item_id = "job-episode-set".to_string();

    maybe_remove_completed_manual_import_download(&app, Some(&completed), Some("anime-1"), true)
        .await;

    assert_eq!(
        *download_client.deleted_items.lock().await,
        vec![("job-episode-set".to_string(), true)]
    );
}
