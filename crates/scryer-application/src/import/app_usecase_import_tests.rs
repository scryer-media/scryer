use super::*;
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
        canonical_tags: vec![],
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
        release_name: None,
        dest_dir: dest_dir.to_string_lossy().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

fn observation_evidence(completed: &CompletedDownload) -> ReleaseEvidence {
    ReleaseEvidence::DownloaderObservation {
        release_name: completed.release_name.clone(),
    }
}

fn titled(facet: MediaFacet, name: &str, year: Option<i32>) -> Title {
    let mut title = test_title(facet);
    title.name = name.to_string();
    title.year = year;
    title
}

/// The grab-time parse of `release_title` for `title`: what `catalog/release_search.rs`
/// scores a candidate against.
fn grab_time_parse(release_title: &str, title: &Title) -> crate::ParsedReleaseMetadata {
    let evidence = crate::acquisition_release_search::canonical_title_evidence(title);
    crate::parse_release_metadata_for_target(release_title, &evidence.parse_context)
}

fn assert_score_bearing_facts_match(
    actual: &crate::ParsedReleaseMetadata,
    expected: &crate::ParsedReleaseMetadata,
) {
    assert_eq!(actual.quality, expected.quality, "quality");
    assert_eq!(actual.source, expected.source, "source");
    assert_eq!(
        actual.release_group, expected.release_group,
        "release group"
    );
    assert_eq!(actual.edition, expected.edition, "edition");
    assert_eq!(
        actual.languages_audio, expected.languages_audio,
        "audio languages"
    );
    assert_eq!(actual.video_codec, expected.video_codec, "video codec");
    assert_eq!(actual.audio, expected.audio, "audio codec");
    assert_eq!(actual.is_remux, expected.is_remux, "remux");
    assert_eq!(actual.year, expected.year, "year");
    assert_eq!(
        actual
            .guide_facts
            .iter()
            .map(|fact| fact.code.as_str())
            .collect::<Vec<_>>(),
        expected
            .guide_facts
            .iter()
            .map(|fact| fact.code.as_str())
            .collect::<Vec<_>>(),
        "guide facts"
    );
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
        "My Lighthouse",
        Some(2020),
        vec!["Mon Phare"],
        vec![],
    )];

    let parsed =
        crate::parse_release_metadata("Mon.Phare.A.K.A.My.Lighthouse.2020.1080p.BluRay.x264-GRP");

    let matched = find_monitored_movie_title_from_release(&titles, &parsed)
        .expect("movie should resolve through alias/title variants");

    assert_eq!(matched.id, "movie-1");
}

#[test]
fn find_monitored_movie_title_from_release_matches_tagged_alias_variant() {
    let mut title =
        test_movie_title_with_aliases_and_ids("movie-1", "Nightfall!!", Some(2022), vec![], vec![]);
    title.tagged_aliases = vec![scryer_domain::TaggedAlias {
        name: "Nightfall Heavy Chorus Dark Lantern".to_string(),
        language: "eng".to_string(),
    }];

    let parsed =
        crate::parse_release_metadata("NIGHTFALL.Heavy.Chorus.Dark.Lantern.2022.1080p.WEB-DL");

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
    let mut completed = test_completed_download("downloader display label", &dest_dir);
    completed.release_name = Some("Paper.Lantern.2012.1080p.BluRay.x264-GRP".to_string());

    let parsed = build_augmented_movie_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Movie, "Paper Lantern", Some(2012)),
    );

    assert_eq!(parsed.year, Some(2012));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(
        parsed.source.as_ref().map(|source| source.as_str()),
        Some("BluRay")
    );
}

#[test]
fn build_augmented_movie_import_metadata_does_not_use_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("job-123");
    let release_dir = dest_dir.join("Paper.Lantern.2012.1080p.BluRay.x264-GRP");
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");
    let completed = test_completed_download("job-123", &dest_dir);

    let parsed = build_augmented_movie_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Movie, "Paper Lantern", Some(2012)),
    );

    assert_ne!(parsed.normalized_title, "PAPER LANTERN");
    assert_eq!(parsed.year, None);
    assert_eq!(parsed.quality, None);
    assert_eq!(parsed.source, None);
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
    let mut completed = test_completed_download("downloader display label", &dest_dir);
    completed.release_name = Some("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Series, "Harbor Pals", Some(2024)),
        false,
    );
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

#[test]
fn ambiguous_obfuscated_episode_message_explains_season_assignment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some(
        "[Erai-raws].Yuki-sama.Kagami.no.Toki.Desu-09.[1080p][Multiple.Subtitle][AA7AC7E5]"
            .to_string(),
    );

    assert_eq!(
        ambiguous_obfuscated_episode_message(&file_path, &observation_evidence(&completed), 1)
            .as_deref(),
        Some(
            "Automatic import could not choose a season for episode 9: the release name does not include a season and the downloaded filename is obfuscated. Open Manual Import and assign the correct season and episode."
        )
    );
}

#[test]
fn ambiguous_obfuscated_episode_message_names_the_video_file_count_for_multi_file_downloads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("7b2c41d8e5f609aa.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());

    // With other video files present the release name's numbering is never
    // applied, so the season-less/season-ful distinction of the release name
    // is irrelevant: the file had to identify itself and could not.
    assert_eq!(
        ambiguous_obfuscated_episode_message(&file_path, &observation_evidence(&completed), 2)
            .as_deref(),
        Some(
            "Automatic import could not identify the episode for this file: this download contains 2 video files and this file's name is obfuscated. Open Manual Import and assign the correct season and episode."
        )
    );
}

#[test]
fn ambiguous_obfuscated_episode_message_stays_silent_for_self_identifying_multi_file_member() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir
        .path()
        .join("Bluey.S01E02.720p.WEB-DL.AV1.AAC2.0-NTb.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some("Bluey.S01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());

    // A member whose own name is usable is not obfuscated; whatever went wrong
    // with it is not explained by this message.
    assert!(
        ambiguous_obfuscated_episode_message(&file_path, &observation_evidence(&completed), 2)
            .is_none()
    );
}

#[test]
fn ambiguous_obfuscated_episode_message_ignores_release_with_explicit_season() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download(
        "Yuki-sama.Kagami.no.Toki.Desu.S02E09.1080p.WEB-DL",
        dir.path(),
    );

    assert!(
        ambiguous_obfuscated_episode_message(&file_path, &observation_evidence(&completed), 1)
            .is_none()
    );
}

#[test]
fn exact_submission_episode_fallback_requires_one_episode_and_one_video() {
    let evidence = |scope| ReleaseEvidence::ScryerSubmission {
        title_id: "title-1".to_string(),
        facet: "series".to_string(),
        source_title: Some("Test.Series.S01E01.1080p.WEB-DL.x264".to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope,
    };
    let episode = evidence(SubmissionScope::Episode {
        episode_id: "ep-1".to_string(),
    });
    assert_eq!(sole_submission_episode_id(&episode, false), Some("ep-1"));
    assert_eq!(sole_submission_episode_id(&episode, true), None);

    let singleton_set = evidence(SubmissionScope::EpisodeSet {
        episode_ids: vec!["ep-1".to_string()],
    });
    assert_eq!(
        sole_submission_episode_id(&singleton_set, false),
        Some("ep-1")
    );

    let ambiguous_set = evidence(SubmissionScope::EpisodeSet {
        episode_ids: vec!["ep-1".to_string(), "ep-2".to_string()],
    });
    assert_eq!(sole_submission_episode_id(&ambiguous_set, false), None);
    let collection = evidence(SubmissionScope::Collection {
        collection_id: "season-1".to_string(),
    });
    assert_eq!(sole_submission_episode_id(&collection, false), None);
    let title = evidence(SubmissionScope::Title);
    assert_eq!(sole_submission_episode_id(&title, false), None);
    assert_eq!(
        sole_submission_episode_id(
            &ReleaseEvidence::DownloaderObservation {
                release_name: Some("Test.Series.S01E01.1080p.WEB-DL.x264".to_string()),
            },
            false,
        ),
        None
    );
}

#[test]
fn unresolved_absolute_episode_message_names_the_detected_number() {
    let dir = tempfile::tempdir().expect("tempdir");
    let release_title = "Test Series - 19 (WEB 1080p x264 10-bit AAC) [A1B2C3D4]";
    let file_path = dir.path().join(format!("{release_title}.mkv"));
    std::fs::write(&file_path, b"episode").expect("write file");
    let evidence = ReleaseEvidence::DownloaderObservation {
        release_name: Some(release_title.to_string()),
    };
    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &evidence,
        &titled(MediaFacet::Series, "Test Series", Some(2020)),
        false,
    );

    assert_eq!(
        unresolved_episode_import_message(&parsed, &file_path, &evidence, 1),
        "Automatic import found absolute episode 19, but could not map it to a season and episode for this title. Open Manual Import and assign the correct episode."
    );
}

#[test]
fn build_augmented_episode_import_metadata_does_not_use_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("job-123");
    let release_dir = dest_dir.join("Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path = release_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("job-123", &dest_dir);

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Series, "Harbor Pals", Some(2024)),
        false,
    );
    assert!(parsed.episode.is_none());
    assert_ne!(parsed.normalized_title, "HARBOR PALS");
    assert_eq!(parsed.quality, None);
}

#[test]
fn build_augmented_episode_import_metadata_keeps_file_episode_when_other_files_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Harbor.Pals.S01.Complete.720p.WEB-DL.AV1");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("Harbor.Pals.S01E03.720p.WEB-DL.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("Harbor.Pals.S01.Complete.720p.WEB-DL.AV1", &dest_dir);

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Series, "Harbor Pals", Some(2024)),
        true,
    );
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![3]);
}

#[test]
fn build_augmented_episode_import_metadata_treats_dotted_hyphen_split_episode_as_single_episode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("[SubsPlease] Harbor Pals S3.-.01 (1080p)");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("[SubsPlease] Harbor Pals S3.-.01 (1080p) [F00DBABE].mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("[SubsPlease] Harbor Pals S3.-.01 (1080p)", &dest_dir);

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Series, "Harbor Pals", Some(2024)),
        false,
    );
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(3));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert!(!episode.full_season);
    assert_eq!(
        episode.release_type,
        scryer_release_parser::ParsedEpisodeReleaseType::SingleEpisode
    );
}

#[test]
fn build_augmented_episode_import_metadata_does_not_score_downloader_display_title_when_other_files_exist()
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

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &titled(MediaFacet::Series, "Harbor Pals", Some(2024)),
        true,
    );

    assert!(parsed.episode.is_none());
    assert_eq!(parsed.quality, None);
}

#[test]
fn canonical_movie_import_parse_matches_grab_time_parse_for_aliased_title() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");
    let release_title = "Paper.Lantern.2012.Directors.Cut.1080p.BluRay.DTS.x264-GRP";
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some(release_title.to_string());
    let mut title = titled(MediaFacet::Movie, "The Lantern Keeper", Some(2012));
    title.aliases = vec!["Paper Lantern".to_string()];

    let parsed = build_augmented_movie_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &title,
    );

    let expected = grab_time_parse(release_title, &title);
    assert_score_bearing_facts_match(&parsed, &expected);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.edition.as_deref(), Some("Director's Cut"));
    assert_eq!(parsed.release_group.as_deref(), Some("GRP"));
    // Title-anchored like the grab parse: the alias resolves to the title's
    // canonical name instead of the context-free "PAPER LANTERN".
    assert_eq!(parsed.normalized_title, expected.normalized_title);
    assert_ne!(
        parsed.normalized_title,
        crate::parse_release_metadata(release_title).normalized_title
    );
}

#[test]
fn canonical_episode_import_parse_matches_grab_time_parse_for_aliased_title() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let release_title = "Tokan.2024.S01E03.1080p.WEB-DL.DDP5.1.H.264-NTb";
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some(release_title.to_string());
    let mut title = titled(MediaFacet::Series, "Sh\u{14d}gun", Some(2024));
    title.aliases = vec!["Tokan".to_string()];

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &title,
        false,
    );

    let expected = grab_time_parse(release_title, &title);
    assert_score_bearing_facts_match(&parsed, &expected);
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    let episode = parsed.episode.expect("episode from the release title");
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![3]);
}

#[test]
fn canonical_import_parse_derives_title_facet_guide_facts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let release_title = "Test.Title.2160p.WEB-DL.DDP5.1.H.264-BiTOR";
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some(release_title.to_string());
    let title = titled(MediaFacet::Series, "Test Title", Some(2024));

    // The context-free parse carries no facet-specific guide facts; the
    // canonical (grab-equivalent) parse does, in a single pass.
    assert!(
        !crate::parse_release_metadata(release_title)
            .guide_facts
            .iter()
            .any(|fact| fact.code == "trash.blocked.lq_release_title")
    );
    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &title,
        false,
    );
    assert!(
        parsed
            .guide_facts
            .iter()
            .any(|fact| fact.code == "trash.blocked.lq_release_title")
    );
}

#[test]
fn episode_import_parse_keeps_release_name_episode_when_title_context_does_not_match() {
    // A user-assigned or parameter-matched download can carry a release name
    // that does not match the title's canonical identity. The title-anchored
    // parse still yields the score-bearing facts but drops the numbering; the
    // release name's own numbering must still win over an obfuscated file stem.
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let release_title = "Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
    let mut completed = test_completed_download("downloader display label", dir.path());
    completed.release_name = Some(release_title.to_string());
    let title = titled(MediaFacet::Series, "Completely Different Name", Some(1999));
    assert!(
        grab_time_parse(release_title, &title).episode.is_none(),
        "fixture must exercise the mismatching-context fallback"
    );

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &observation_evidence(&completed),
        &title,
        false,
    );

    assert_eq!(parsed.quality.as_deref(), Some("720p"));
    assert_eq!(parsed.release_group.as_deref(), Some("NTb"));
    let episode = parsed.episode.expect("episode from the release name");
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
}

// ── episode identity: Sonarr's OtherVideoFiles rule ─────────────────────────

const BLUEY_EPISODE_RELEASE: &str = "Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
const BLUEY_PACK_RELEASE: &str = "Bluey.S01.720p.WEB-DL.AV1.AAC2.0-NTb";

fn bluey_title() -> Title {
    titled(MediaFacet::Series, "Bluey", Some(2018))
}

fn bluey_submission_evidence(release_title: &str, scope: SubmissionScope) -> ReleaseEvidence {
    ReleaseEvidence::ScryerSubmission {
        title_id: "t1".to_string(),
        facet: "series".to_string(),
        source_title: Some(release_title.to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope,
    }
}

fn write_video(dir: &std::path::Path, file_name: &str) -> std::path::PathBuf {
    let path = dir.join(file_name);
    std::fs::write(&path, b"episode").expect("write file");
    path
}

/// Score-bearing facts must come from the release title parsed with the
/// canonical title context — never from the file name — whichever way the
/// episode identity was decided.
fn assert_release_scored_facts(parsed: &crate::ParsedReleaseMetadata, release_title: &str) {
    assert_score_bearing_facts_match(parsed, &grab_time_parse(release_title, &bluey_title()));
    assert_eq!(parsed.quality.as_deref(), Some("720p"), "quality");
    assert_eq!(
        parsed.source.as_ref().map(|source| source.as_str()),
        Some("WEB-DL"),
        "source"
    );
    assert_eq!(
        parsed.release_group.as_deref(),
        Some("NTb"),
        "release group"
    );
}

#[test]
fn episode_identity_sole_obfuscated_video_takes_single_episode_release_numbering() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_video(dir.path(), "4f8e2c7a91b6d3e0.mkv");
    let evidence = bluey_submission_evidence(
        BLUEY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        },
    );

    let parsed = build_augmented_episode_import_metadata_for_title(
        &file_path,
        &evidence,
        &bluey_title(),
        false,
    );

    let episode = parsed
        .episode
        .as_ref()
        .expect("episode from the release title");
    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_release_scored_facts(&parsed, BLUEY_EPISODE_RELEASE);
}

#[test]
fn episode_identity_obfuscated_video_among_others_gets_no_episode_from_release_numbering() {
    // Release-gate regression: `Bluey.S01E01…` extracted to two identical
    // obfuscated videos and S01E01 was applied to both, importing whichever
    // came first in directory order. With other video files present the file
    // must identify itself, and an obfuscated name cannot.
    let dir = tempfile::tempdir().expect("tempdir");
    let first = write_video(dir.path(), "4f8e2c7a91b6d3e0.mkv");
    let second = write_video(dir.path(), "7b2c41d8e5f609aa.mkv");
    let evidence = bluey_submission_evidence(
        BLUEY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        },
    );

    for file_path in [&first, &second] {
        let parsed = build_augmented_episode_import_metadata_for_title(
            file_path,
            &evidence,
            &bluey_title(),
            true,
        );

        assert!(
            parsed.episode.is_none(),
            "{} must not inherit the release numbering: {:?}",
            file_path.display(),
            parsed.episode
        );
        assert_release_scored_facts(&parsed, BLUEY_EPISODE_RELEASE);
    }
}

#[test]
fn episode_identity_season_pack_member_uses_its_own_numbering_regardless_of_sibling_count() {
    // Release-gate regression: the pack title parsed to a whole-season episode
    // and every member resolved to all 52 episodes. A season pack has no
    // episode numbers to hand out, so a member's own name governs even when it
    // is the only video (the rest of the pack may still be extracting).
    let dir = tempfile::tempdir().expect("tempdir");
    let season_dir = dir.path().join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = write_video(&season_dir, "Bluey.S01E02.720p.WEB-DL.AV1.AAC2.0-NTb.mkv");
    let evidence = bluey_submission_evidence(
        BLUEY_PACK_RELEASE,
        SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        },
    );
    let pack_parse = grab_time_parse(BLUEY_PACK_RELEASE, &bluey_title());
    let pack_episode = pack_parse
        .episode
        .as_ref()
        .expect("pack parse names a season");
    assert!(
        pack_episode.full_season
            && pack_episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
            && pack_episode.episode_numbers.is_empty(),
        "fixture must exercise the season-pack rule: {pack_episode:?}"
    );

    for other_video_files in [false, true] {
        let parsed = build_augmented_episode_import_metadata_for_title(
            &file_path,
            &evidence,
            &bluey_title(),
            other_video_files,
        );

        let episode = parsed
            .episode
            .as_ref()
            .unwrap_or_else(|| panic!("member episode (other_video_files={other_video_files})"));
        assert_eq!(episode.season, Some(1));
        assert_eq!(episode.episode_numbers, vec![2]);
        assert_eq!(
            episode.release_type,
            crate::ParsedEpisodeReleaseType::SingleEpisode
        );
        assert!(!episode.full_season);
        assert_release_scored_facts(&parsed, BLUEY_PACK_RELEASE);
    }
}

#[test]
fn episode_identity_season_pack_member_with_obfuscated_name_gets_no_episode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_video(dir.path(), "4f8e2c7a91b6d3e0.mkv");
    let evidence = bluey_submission_evidence(
        BLUEY_PACK_RELEASE,
        SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        },
    );

    for other_video_files in [false, true] {
        let parsed = build_augmented_episode_import_metadata_for_title(
            &file_path,
            &evidence,
            &bluey_title(),
            other_video_files,
        );

        assert!(
            parsed.episode.is_none(),
            "a pack must never be applied to a member (other_video_files={other_video_files}): {:?}",
            parsed.episode
        );
        assert_release_scored_facts(&parsed, BLUEY_PACK_RELEASE);
    }
}

#[test]
fn episode_identity_sole_scene_titled_video_identifies_itself() {
    // Sonarr `SceneChecker.IsSceneTitle`: a sole video that is itself a proper
    // scene release name (dotted, grouped, quality-tagged, numbered) keeps its
    // own numbering; the release name is not applied over it. The grabbed
    // release gate then decides whether E02 belongs to the E01 grab.
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_video(dir.path(), "Bluey.S01E02.720p.WEB-DL.AV1.AAC2.0-NTb.mkv");
    let evidence = bluey_submission_evidence(
        BLUEY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        },
    );

    for other_video_files in [false, true] {
        let parsed = build_augmented_episode_import_metadata_for_title(
            &file_path,
            &evidence,
            &bluey_title(),
            other_video_files,
        );
        let episode = parsed.episode.as_ref().expect("episode from the file name");
        assert_eq!(
            episode.episode_numbers,
            vec![2],
            "other_video_files={other_video_files}"
        );
        assert_release_scored_facts(&parsed, BLUEY_EPISODE_RELEASE);
    }
}

#[test]
fn episode_identity_sole_non_scene_video_takes_release_numbering() {
    // Not scene-titled (spaces, no group/quality): the release name remains
    // the better evidence for a sole video, even when the file names another
    // episode.
    let evidence = bluey_submission_evidence(
        BLUEY_EPISODE_RELEASE,
        SubmissionScope::Episode {
            episode_id: "ep-1".to_string(),
        },
    );
    let dir = tempfile::tempdir().expect("tempdir");
    for file_name in ["bluey - s01e02.mkv", "Bluey.S01E02.mkv", "episode 2.mkv"] {
        let file_path = write_video(dir.path(), file_name);
        let parsed = build_augmented_episode_import_metadata_for_title(
            &file_path,
            &evidence,
            &bluey_title(),
            false,
        );
        let episode = parsed
            .episode
            .as_ref()
            .unwrap_or_else(|| panic!("{file_name}: episode from the release title"));
        assert_eq!(episode.episode_numbers, vec![1], "{file_name}");
    }
}

#[test]
fn episode_identity_pack_member_resolves_anime_absolute_numbering_with_title_context() {
    // A member's own name is parsed with the title's canonical context so an
    // absolute-numbered anime member resolves the way the grab path parses it.
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = write_video(
        dir.path(),
        "[SubsPlease] Harbor Pals - 03 (1080p) [F00DBABE].mkv",
    );
    let release_title = "[SubsPlease] Harbor Pals (01-12) (1080p) [Batch]";
    let title = titled(MediaFacet::Anime, "Harbor Pals", Some(2024));
    let evidence = ReleaseEvidence::ScryerSubmission {
        title_id: "t1".to_string(),
        facet: "anime".to_string(),
        source_title: Some(release_title.to_string()),
        observed_release_name: None,
        release_size_bytes: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope: SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        },
    };

    let parsed =
        build_augmented_episode_import_metadata_for_title(&file_path, &evidence, &title, true);

    let episode = parsed.episode.as_ref().expect("member episode");
    assert_eq!(episode.absolute_episode, Some(3));
    assert_eq!(episode.absolute_episode_numbers, vec![3]);
    assert_eq!(
        episode.release_type,
        crate::ParsedEpisodeReleaseType::SingleEpisode
    );
    assert_score_bearing_facts_match(&parsed, &grab_time_parse(release_title, &title));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
}

#[test]
fn title_evidence_candidates_from_video_files_uses_immediate_parent_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let release_dir = dir.path().join(
        "Harbor.Pilot.And.The.Silent.Harbors.Part1.2010.720p.BluRay.DTS.x264-LEGION-Obfuscated",
    );
    std::fs::create_dir_all(&release_dir).expect("create release dir");
    let file_path =
        release_dir.join("aUUKqrO833LbSr7VlByumnR24y7ULADpVJ7K0FTnPhPMqpp0KIIaLSLYXJmyjm.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");

    let candidates = title_evidence_candidates_from_video_files(&[file_path]);

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].normalized_title,
        "HARBOR PILOT AND THE SILENT HARBORS PART 1"
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

#[test]
fn is_sample_file_allows_small_double_extension_strm_pointer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pointer = dir.path().join("Show.S01E01.1080p.mkv.strm");
    std::fs::write(&pointer, b"https://nzbdav.example/stream/episode").expect("write strm");

    assert!(!is_sample_file(&pointer));
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
        dovi_profile: None,
        dovi_bl_compat_id: None,
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
        audio_language_warning: None,
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
        audio_language_warning: None,
    };

    let result = {
        let context = app
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        crate::post_download_gate::compute_post_download_acquisition_decision(
            &context,
            &title,
            &parsed,
            &acceptance,
            crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes),
            5 * 1024 * 1024,
            &[],
            false,
        )
    };

    assert_eq!(result.parsed.quality.as_deref(), Some("720p"));
    assert!(result.score < 0);
    // The resolution the file actually has contradicts the one it advertised.
    // The score reflects the truth; the verdict names the disagreement.
    assert!(
        !result.truth_verdict.is_consistent(),
        "a 720p file sold as 1080p must be recorded as contradicted, got {:?}",
        result.truth_verdict
    );
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
            .any(|change| change
                .as_str()
                .is_some_and(|value| value.contains("resolution")))
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
        audio_language_warning: None,
    };
    let (prepared_parsed, first_pass_changes) =
        crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);
    assert!(
        first_pass_changes
            .iter()
            .any(|change| change.contains("resolution"))
    );

    let result = {
        let context = app
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        crate::post_download_gate::compute_post_download_acquisition_decision(
            &context,
            &title,
            &prepared_parsed,
            &acceptance,
            crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes),
            5 * 1024 * 1024,
            &first_pass_changes,
            false,
        )
    };

    let scoring_log = result.scoring_log.expect("scoring log should serialize");
    let scoring_log: serde_json::Value =
        serde_json::from_str(&scoring_log).expect("scoring log should be JSON");
    assert!(
        scoring_log["rescore_changes"]
            .as_array()
            .expect("rescore changes should be an array")
            .iter()
            .any(|change| change
                .as_str()
                .is_some_and(|value| value.contains("resolution")))
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
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec!["movie".to_string()],
    };
    let engine =
        scryer_rules::UserRulesEngine::build(&[policy]).expect("user rule engine should compile");
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
        audio_language_warning: None,
    };

    let result = {
        let context = app
            .resolve_canonical_scoring_context(&title, &profile)
            .await;
        crate::post_download_gate::compute_post_download_acquisition_decision(
            &context,
            &title,
            &parsed,
            &acceptance,
            crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes),
            5 * 1024 * 1024,
            &[],
            false,
        )
    };

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
        audio_language_warning: None,
    };
    let (rescored, _) = crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);

    let dest_path = episode_import_dest_path(
        &title,
        true,
        &rescored,
        "mkv",
        std::path::Path::new("/downloads/obfuscated.release.name.mkv"),
        std::path::Path::new("/library/Test Show"),
        true,
        "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        "Season {season:2}",
        "Specials",
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
        true,
        &parsed,
        "mkv",
        std::path::Path::new("/downloads/Obfuscated.Source.Name.mkv"),
        std::path::Path::new("/library/Test Show"),
        false,
        "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        "Season {season:2}",
        "Specials",
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
fn episode_import_dest_path_uses_configured_regular_and_specials_folders() {
    let mut title = test_title(MediaFacet::Series);
    title.name = "Test Show".to_string();
    let parsed = crate::parse_release_metadata("Test.Show.S03E07.mkv");
    let source = std::path::Path::new("/downloads/Test.Show.S03E07.mkv");
    let title_folder = std::path::Path::new("/library/Test Show");

    let regular = episode_import_dest_path(
        &title,
        true,
        &parsed,
        "mkv",
        source,
        title_folder,
        false,
        "unused",
        "{title|space:.}.S{season:2}",
        "Extras",
        3,
        "7",
        None,
        None,
        None,
    );
    assert_eq!(
        regular,
        std::path::PathBuf::from("/library/Test Show/Test.Show.S03/Test.Show.S03E07.mkv")
    );

    let specials = episode_import_dest_path(
        &title,
        true,
        &parsed,
        "mkv",
        source,
        title_folder,
        false,
        "unused",
        "Season {season}",
        "Extras",
        0,
        "7",
        None,
        None,
        None,
    );
    assert_eq!(
        specials,
        std::path::PathBuf::from("/library/Test Show/Extras/Test.Show.S03E07.mkv")
    );

    title.tags = vec!["scryer:season-folder:disabled".to_string()];
    let flat = episode_import_dest_path(
        &title,
        false,
        &parsed,
        "mkv",
        source,
        title_folder,
        false,
        "unused",
        "Season {season}",
        "Extras",
        3,
        "7",
        None,
        None,
        None,
    );
    assert_eq!(
        flat,
        std::path::PathBuf::from("/library/Test Show/Test.Show.S03E07.mkv")
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
fn find_video_files_accepts_direct_video_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let video = dir.path().join("movie.mkv");
    std::fs::write(&video, b"data").expect("write");

    assert_eq!(find_video_files(&video, false).expect("find"), vec![video]);
}

#[test]
fn find_video_files_rejects_direct_non_video_without_reading_it_as_a_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"data").expect("write");

    assert!(
        find_video_files(&payload, false)
            .expect("classify")
            .is_empty()
    );
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
fn find_video_files_keeps_small_strm_when_filtering_samples() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pointer = dir.path().join("Show.S01E01.1080p.mkv.strm");
    std::fs::write(&pointer, b"https://nzbdav.example/stream/episode").expect("write strm");

    let files = find_video_files(dir.path(), true).expect("find");

    assert_eq!(files, vec![pointer]);
}

#[test]
fn find_video_files_returns_error_for_missing_dir() {
    let result = find_video_files(std::path::Path::new("/nonexistent/dir/abc"), false);
    assert!(matches!(
        result,
        Err(AppError::ImportSourceInspection { .. })
    ));
}

#[cfg(unix)]
#[test]
fn find_video_files_reports_an_unreadable_root_as_source_inspection() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let original_permissions = std::fs::metadata(dir.path())
        .expect("metadata")
        .permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o000))
        .expect("remove permissions");
    let result = find_video_files(dir.path(), false);
    std::fs::set_permissions(dir.path(), original_permissions).expect("restore permissions");

    assert!(matches!(
        result,
        Err(AppError::ImportSourceInspection { .. })
    ));
}

#[cfg(unix)]
#[test]
fn find_video_files_rejects_special_filesystem_objects() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("download.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).expect("bind socket");

    assert!(matches!(
        find_video_files(&socket_path, false),
        Err(AppError::UnsupportedImportSource { .. })
    ));
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
            announced_size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
            content_hashes: None,
            quality_label: Some("1080p".to_string()),
            scan_status: "scanned".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            dovi_profile: None,
            dovi_bl_compat_id: None,
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
        title_role: crate::MediaFileRole::Primary,
        episode_ids: episode_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        primary_episode_ids: episode_ids
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
    }
}

/// The comparison half of [`crate::import_decide::decide_import`], driven with
/// episode-scoped rows. Pure: no app, no probe, no filesystem.
fn episode_import_admission(
    incumbents: &[crate::EpisodeScopedMediaFile],
    target_episode_ids: &[&str],
    candidate: crate::admission::CandidateFacts,
    operator_intent: bool,
) -> Result<
    (Vec<crate::EpisodeScopedMediaFile>, i32),
    (
        crate::post_download_gate::ImportedFileRejection,
        crate::import_decide::RejectionDisposition,
    ),
> {
    let admitted = crate::import_decide::evaluate_import_admission(
        &test_admission_subject(incumbents, target_episode_ids),
        candidate,
        operator_intent,
        &crate::import_decide::IncumbentRows::Episodes(incumbents),
        "this episode",
    )?;
    let crate::import_decide::SupersededIncumbents::Episodes(rows) = admitted.superseded else {
        panic!("episode rows in, episode rows out");
    };
    Ok((rows, admitted.previous_best_score))
}

#[test]
fn decide_import_replaces_a_different_filename_when_the_new_score_is_higher() {
    let incumbents = vec![scoped_media_file(
        "file-1",
        "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01 - 720p.mkv",
        510,
        &["ep-1"],
    )];

    let (superseded, previous_best_score) = episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
        false,
    )
    .expect("import admission should accept a higher-scored replacement");

    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0].media_file.id, "file-1");
    assert_eq!(previous_best_score, 510);
}

#[test]
fn decide_import_skips_when_the_existing_episode_file_scores_higher() {
    let incumbents = vec![scoped_media_file(
        "file-1",
        "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01 - 1080p.mkv",
        820,
        &["ep-1"],
    )];

    let (rejection, disposition) = episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 700),
        false,
    )
    .unwrap_err();

    assert_eq!(
        rejection.skip_reason,
        Some(ImportSkipReason::AlreadyImported)
    );
    assert!(rejection.message.contains("equal or better"));
    // A release that merely lost the comparison is not burned (D17).
    assert_eq!(
        disposition,
        crate::import_decide::RejectionDisposition::Skip
    );
}

#[test]
fn manual_replacement_bypasses_equal_or_lower_score_comparison() {
    let incumbents = vec![scoped_media_file(
        "file-1",
        "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01 - 1080p.mkv",
        820,
        &["ep-1"],
    )];

    // Without operator intent, a *downgrade* is still rejected.
    assert!(
        episode_import_admission(
            &incumbents,
            &["ep-1"],
            crate::admission::CandidateFacts::new(Some(0), 0, 600),
            false
        )
        .is_err()
    );

    // An equal score no longer needs force. The download already happened;
    // refusing a tie is what produced "existing file is equal or better" on a
    // release Scryer itself queued.
    episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 820),
        false,
    )
    .expect("an equally scored import is not a downgrade");

    // Operator intent is what lets something genuinely lower land.
    let (superseded, _) = episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 600),
        true,
    )
    .expect("manual replacement should replace a higher-scored incumbent");
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0].media_file.id, "file-1");
}

#[test]
fn decide_import_holds_when_the_existing_file_covers_a_broader_episode_set() {
    let incumbents = vec![scoped_media_file(
        "file-pack",
        "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01-E02.mkv",
        400,
        &["ep-1", "ep-2"],
    )];

    let (rejection, disposition) = episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
        false,
    )
    .unwrap_err();

    assert_eq!(
        rejection.skip_reason,
        Some(ImportSkipReason::PolicyMismatch)
    );
    assert!(rejection.message.contains("broader episode set"));
    // The release is fine; it just cannot be placed here. That is an operator
    // decision, not a comparison the release lost (D8's bounded I4 exception).
    assert_eq!(
        disposition,
        crate::import_decide::RejectionDisposition::Hold
    );
}

#[test]
fn manual_import_error_from_skip_reason_maps_policy_mismatch() {
    assert_eq!(
        manual_import_error_from_skip_reason(Some(ImportSkipReason::PolicyMismatch)),
        scryer_domain::ImportErrorCode::PolicyMismatch
    );
}

#[test]
fn parsed_with_quality_override_replaces_parsed_quality() {
    let parsed = test_parsed();

    let effective = parsed_with_quality_override(&parsed, Some("2160P"));

    assert_eq!(effective.quality.as_deref(), Some("2160P"));
}

#[test]
fn decide_import_supersedes_all_duplicate_incumbents_for_the_same_target_set() {
    let incumbents = vec![
        scoped_media_file(
            "file-1",
            "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01 - 720p.mkv",
            300,
            &["ep-1"],
        ),
        scoped_media_file(
            "file-2",
            "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01 - 1080p.mkv",
            500,
            &["ep-1"],
        ),
    ];

    let (superseded, _) = episode_import_admission(
        &incumbents,
        &["ep-1"],
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
        false,
    )
    .expect("higher score should supersede all incumbents");

    assert_eq!(superseded.len(), 2);
    assert_eq!(superseded[0].media_file.id, "file-2");
    assert_eq!(superseded[1].media_file.id, "file-1");
}

#[test]
fn decide_import_allows_a_pack_to_replace_singles_when_it_beats_all_of_them() {
    let incumbents = vec![
        scoped_media_file(
            "file-1",
            "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E01.mkv",
            300,
            &["ep-1"],
        ),
        scoped_media_file(
            "file-2",
            "/data/TV/Quiet Orbit/Season 01/Quiet Orbit - S01E02.mkv",
            450,
            &["ep-2"],
        ),
    ];

    let (superseded, previous_best_score) = episode_import_admission(
        &incumbents,
        &["ep-1", "ep-2"],
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
        false,
    )
    .expect("season pack should replace lower-scored singles");

    assert_eq!(previous_best_score, 450);
    assert_eq!(superseded.len(), 2);
}

// ── series-movie link incumbents (D14) ────────────────────────────────────────

fn linked_media_file(
    id: &str,
    file_path: &str,
    link_id: &str,
    score: i32,
) -> crate::TitleMediaFile {
    let mut file = scoped_media_file(id, file_path, score, &[]).media_file;
    file.episode_id = None;
    file.series_movie_link_ids = vec![link_id.to_string()];
    file
}

fn series_movie_subject(
    link_id: &str,
    files: &[crate::TitleMediaFile],
) -> crate::admission::AdmissionSubject {
    crate::admission::AdmissionSubject::new(
        crate::admission::AdmissionScope::SeriesMovieLink(link_id.to_string()),
        files
            .iter()
            .filter(|file| {
                file.series_movie_link_ids
                    .iter()
                    .any(|candidate| candidate == link_id)
            })
            .map(|file| {
                (
                    crate::admission::Incumbent {
                        tier_index: Some(0),
                        revision: 0,
                        file_id: file.id.clone(),
                        file_path: file.file_path.clone(),
                        release_group: file.release_group.clone(),
                        score: file.acquisition_score.unwrap_or(0),
                        covers: Vec::new(),
                        created_at: file.created_at.clone(),
                    },
                    file.role.is_primary(),
                )
            }),
    )
}

/// The comparison half of the decision for a series-movie link, over the
/// title's whole primary-file list.
fn link_import_admission(
    link_id: &str,
    existing_files: &[crate::TitleMediaFile],
    subject_files: &[crate::TitleMediaFile],
    candidate: crate::admission::CandidateFacts,
) -> Result<
    (Vec<crate::TitleMediaFile>, i32),
    (
        crate::post_download_gate::ImportedFileRejection,
        crate::import_decide::RejectionDisposition,
    ),
> {
    let admitted = crate::import_decide::evaluate_import_admission(
        &series_movie_subject(link_id, subject_files),
        candidate,
        false,
        &crate::import_decide::IncumbentRows::Title(existing_files),
        "this series-movie link",
    )?;
    let crate::import_decide::SupersededIncumbents::Title(rows) = admitted.superseded else {
        panic!("title rows in, title rows out");
    };
    Ok((rows, admitted.previous_best_score))
}

/// The crash regression. A linked incumbent lives at whatever path it was
/// imported under — rename disabled, a changed template, `.mp4` → `.mkv` — so an
/// upgrade whose destination differs must still find the row it displaces.
/// Resolving by path found nothing while admission said the scope was occupied,
/// and the `.expect` panicked the import task.
#[test]
fn a_linked_incumbent_at_another_path_still_resolves_for_the_upgrade_branch() {
    let dest_path = "/data/TV/Quiet Orbit/Season 00/Quiet Orbit - S00E00 - The Film.mkv";
    let existing_files = vec![linked_media_file(
        "file-1",
        "/data/TV/Quiet Orbit/Season 00/preserved.original.name.mp4",
        "link-1",
        400,
    )];

    // Precondition: the old path filter really does come up empty here.
    assert!(
        !existing_files
            .iter()
            .any(|file| file.file_path == dest_path),
        "fixture must model an incumbent that is NOT at the import destination"
    );
    assert!(!series_movie_subject("link-1", &existing_files).is_unoccupied());

    let (superseded, previous_best_score) = link_import_admission(
        "link-1",
        &existing_files,
        &existing_files,
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
    )
    .expect("a better linked file is an upgrade, and its row must resolve");

    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0].id, "file-1");
    assert_eq!(previous_best_score, 400);
}

/// A refused linked upgrade names the incumbent that blocked it, and does not
/// burn the release for losing a fair comparison.
#[test]
fn a_refused_linked_upgrade_still_names_the_blocking_incumbent() {
    let existing_files = vec![linked_media_file(
        "file-1",
        "/data/TV/Quiet Orbit/Season 00/preserved.original.name.mp4",
        "link-1",
        900,
    )];

    let (rejection, disposition) = link_import_admission(
        "link-1",
        &existing_files,
        &existing_files,
        crate::admission::CandidateFacts::new(Some(0), 0, 100),
    )
    .unwrap_err();

    assert!(
        rejection.message.contains("preserved.original.name.mp4"),
        "the refusal must name the row that blocked it, got {}",
        rejection.message
    );
    assert_eq!(
        disposition,
        crate::import_decide::RejectionDisposition::Skip
    );
}

/// And when the subject and the file list genuinely disagree, that is a
/// rejection rather than an assertion (D14) — and a *hold*, because nothing was
/// judged about the release.
#[test]
fn an_unresolvable_linked_incumbent_is_a_rejection_not_a_panic() {
    let subject_files = vec![linked_media_file(
        "file-gone",
        "/data/gone.mkv",
        "link-1",
        400,
    )];

    let (rejection, disposition) = link_import_admission(
        "link-1",
        &[],
        &subject_files,
        crate::admission::CandidateFacts::new(Some(0), 0, 900),
    )
    .unwrap_err();

    assert_eq!(
        rejection.skip_reason,
        Some(ImportSkipReason::PolicyMismatch)
    );
    assert_eq!(rejection.recycle_reason, "policy_mismatch");
    assert!(rejection.message.contains("this series-movie link"));
    assert_eq!(
        disposition,
        crate::import_decide::RejectionDisposition::Hold
    );
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

    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        _remove_data: bool,
    ) -> AppResult<()> {
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

// ── completed manual-import recovery loop ────────────────────────────────────

struct RecoveryImportRepo {
    records: Vec<scryer_domain::ImportRecord>,
    canonical_download_ids:
        std::collections::HashMap<String, scryer_domain::download_identity::DownloadId>,
    windows: Mutex<Vec<chrono::DateTime<chrono::Utc>>>,
    deleted_sources: Mutex<Vec<crate::ClientJobLocator>>,
}

#[async_trait]
impl crate::ImportRepository for RecoveryImportRepo {
    async fn queue_import_request(
        &self,
        _: crate::ClientJobLocator,
        _: String,
        _: String,
    ) -> AppResult<String> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_import_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::ImportRecord>> {
        Ok(None)
    }

    async fn canonical_download_id_for_import(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::download_identity::DownloadId>> {
        Ok(self.canonical_download_ids.get(id).copied())
    }

    async fn update_import_status(
        &self,
        _: &str,
        _: scryer_domain::ImportStatus,
        _: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn update_import_transfer_progress(
        &self,
        _: &str,
        _: scryer_domain::ImportTransferPhase,
        _: i64,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn recover_stale_processing_imports(&self, _: i64) -> AppResult<u64> {
        Ok(0)
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        _: scryer_domain::ImportType,
        _: i64,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<scryer_domain::ImportRecord>> {
        Ok(Vec::new())
    }

    async fn list_pending_imports_for_type(
        &self,
        _: scryer_domain::ImportType,
    ) -> AppResult<Vec<scryer_domain::ImportRecord>> {
        Ok(Vec::new())
    }

    async fn list_imports_for_identities(
        &self,
        _: &[crate::ClientJobLocator],
    ) -> AppResult<Vec<scryer_domain::ImportRecord>> {
        Ok(Vec::new())
    }

    async fn list_completed_manual_imports(
        &self,
        updated_after: chrono::DateTime<chrono::Utc>,
        _: usize,
    ) -> AppResult<Vec<scryer_domain::ImportRecord>> {
        self.windows.lock().await.push(updated_after);
        Ok(self.records.clone())
    }

    async fn delete_manual_import_selections_for_source(
        &self,
        source_identity: &crate::ClientJobLocator,
    ) -> AppResult<()> {
        self.deleted_sources
            .lock()
            .await
            .push(source_identity.clone());
        Ok(())
    }

    async fn list_imports(&self, _: usize) -> AppResult<Vec<scryer_domain::ImportRecord>> {
        Ok(Vec::new())
    }
}

fn completed_manual_import_record_for(
    import_id: &str,
    item_id: &str,
) -> scryer_domain::ImportRecord {
    let result = ManualImportExecutionResult {
        import_id: import_id.to_string(),
        client_type: "qbittorrent".to_string(),
        download_client_item_id: item_id.to_string(),
        title_id: Some("title-1".to_string()),
        status: scryer_domain::ImportStatus::Completed,
        error_code: None,
        error_message: None,
        requires_reconciliation: false,
        retry_attempts: 0,
        next_retry_at: None,
        file_results: vec![ManualImportFileResult {
            file_path: format!("/downloads/{item_id}/movie.mkv"),
            episode_id: None,
            series_movie_link_id: None,
            success: true,
            skipped: false,
            dest_path: Some("/library/movie.mkv".to_string()),
            error_code: None,
            error_message: None,
        }],
        completed_at: chrono::Utc::now(),
    };
    scryer_domain::ImportRecord {
        id: import_id.to_string(),
        source_client_id: Some("client-1".to_string()),
        source_system: "qbittorrent".to_string(),
        source_ref: item_id.to_string(),
        import_type: scryer_domain::ImportType::ManualImport,
        status: scryer_domain::ImportStatus::Completed,
        payload_json: "{}".to_string(),
        result_json: Some(serde_json::to_string(&result).expect("result JSON")),
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: None,
        finished_at: None,
        created_at: "2026-08-17T00:00:00Z".to_string(),
        updated_at: "2026-08-17T00:00:00Z".to_string(),
    }
}

/// A scripted tracked-download runtime: answers `MarkImportedIfAwaitingImport`
/// per item id from `script` (falling back to `Unchanged`) and records every
/// (item id, record completion time) it was asked about.
type TrackedDownloadImportRequests = Arc<
    Mutex<
        Vec<(
            String,
            chrono::DateTime<chrono::Utc>,
            Option<scryer_domain::download_identity::DownloadId>,
        )>,
    >,
>;

fn scripted_tracked_download_runtime(
    script: Vec<(
        &'static str,
        Vec<crate::tracked_downloads::ManualImportRecoveryOutcome>,
    )>,
) -> (
    crate::tracked_downloads::TrackedDownloadHandle,
    TrackedDownloadImportRequests,
) {
    use crate::tracked_downloads::{ManualImportRecoveryOutcome, TrackedDownloadCommand};

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let asked_task = asked.clone();
    tokio::spawn(async move {
        let mut script = script
            .into_iter()
            .map(|(item_id, outcomes)| (item_id, outcomes.into_iter()))
            .collect::<Vec<_>>();
        while let Some(command) = rx.recv().await {
            let TrackedDownloadCommand::MarkImportedIfAwaitingImport {
                source_identity,
                canonical_download_id,
                record_completed_at,
                reply,
            } = command
            else {
                continue;
            };
            asked_task.lock().await.push((
                source_identity.item_id.clone(),
                record_completed_at,
                canonical_download_id,
            ));
            let outcome = script
                .iter_mut()
                .find(|(item_id, _)| *item_id == source_identity.item_id)
                .and_then(|(_, outcomes)| outcomes.next())
                .unwrap_or(ManualImportRecoveryOutcome::Unchanged);
            let _ = reply.send(Ok(outcome));
        }
    });
    (
        crate::tracked_downloads::TrackedDownloadHandle::new(tx),
        asked,
    )
}

#[tokio::test]
async fn completed_manual_import_recovery_decides_each_record_once_and_only_marks_eligible_sources()
{
    use crate::tracked_downloads::ManualImportRecoveryOutcome;

    let repo = Arc::new(RecoveryImportRepo {
        records: vec![
            completed_manual_import_record_for("import-marked", "hash-marked"),
            completed_manual_import_record_for("import-already-imported", "hash-already"),
            completed_manual_import_record_for("import-fresh-download", "hash-fresh"),
        ],
        canonical_download_ids: std::collections::HashMap::from([(
            "import-marked".to_string(),
            scryer_domain::download_identity::DownloadId::new(),
        )]),
        windows: Mutex::new(Vec::new()),
        deleted_sources: Mutex::new(Vec::new()),
    });
    let (handle, asked) = scripted_tracked_download_runtime(vec![
        ("hash-marked", vec![ManualImportRecoveryOutcome::Marked]),
        // Already `Imported` and a fresh `Downloading` re-grab of the same
        // info-hash both come back unchanged; neither may be acted on again.
        ("hash-already", vec![ManualImportRecoveryOutcome::Unchanged]),
        ("hash-fresh", vec![ManualImportRecoveryOutcome::Unchanged]),
    ]);
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    )
    .with_test_overrides(|builder| {
        builder
            .with_imports(repo.clone())
            .with_tracked_download_handle(handle)
    });
    let worker = PollingWorker::new(
        "manual_import_recovery_test",
        tokio_util::sync::CancellationToken::new(),
    );
    let mut memo = std::collections::HashMap::new();

    let before = chrono::Utc::now();
    recover_completed_manual_imports(&app, &worker, &mut memo).await;
    recover_completed_manual_imports(&app, &worker, &mut memo).await;
    recover_completed_manual_imports(&app, &worker, &mut memo).await;

    let asked = asked.lock().await.clone();
    let mut asked_items = asked
        .iter()
        .map(|(item_id, _, _)| item_id.clone())
        .collect::<Vec<_>>();
    asked_items.sort();
    assert_eq!(
        asked_items,
        vec![
            "hash-already".to_string(),
            "hash-fresh".to_string(),
            "hash-marked".to_string()
        ],
        "every record is decided exactly once per process, not once per tick"
    );
    let record_completed_at = "2026-08-17T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .expect("fixture updated_at");
    assert!(
        asked
            .iter()
            .all(|(_, completed_at, _)| *completed_at == record_completed_at),
        "the runtime is told when each record completed (finished_at, else updated_at): {asked:?}"
    );
    assert!(
        asked.iter().any(|(item_id, _, canonical_download_id)| {
            item_id == "hash-marked" && canonical_download_id.is_some()
        }) && asked.iter().any(|(item_id, _, canonical_download_id)| {
            item_id == "hash-already" && canonical_download_id.is_none()
        }),
        "manual-import recovery forwards canonical identity when present and preserves legacy rows without it: {asked:?}"
    );
    assert_eq!(
        *repo.deleted_sources.lock().await,
        vec![crate::ClientJobLocator::new(
            Some("client-1"),
            "qbittorrent",
            "hash-marked"
        )],
        "selections are cleaned up only for the source that was actually marked"
    );
    assert_eq!(
        memo,
        std::collections::HashMap::from([
            (
                "import-marked".to_string(),
                ManualImportRecoveryMemo::Settled
            ),
            (
                "import-already-imported".to_string(),
                ManualImportRecoveryMemo::Settled
            ),
            (
                "import-fresh-download".to_string(),
                ManualImportRecoveryMemo::Settled
            ),
        ])
    );
    let windows = repo.windows.lock().await.clone();
    assert_eq!(windows.len(), 3);
    for window in windows {
        let lookback = before - window;
        assert!(
            lookback >= chrono::Duration::hours(24) - chrono::Duration::minutes(1)
                && lookback <= chrono::Duration::hours(24) + chrono::Duration::minutes(1),
            "recovery scans a 24h window, got {lookback}"
        );
    }
}

#[tokio::test]
async fn completed_manual_import_recovery_retries_busy_and_untracked_sources_on_later_ticks() {
    use crate::tracked_downloads::ManualImportRecoveryOutcome;

    let repo = Arc::new(RecoveryImportRepo {
        records: vec![
            completed_manual_import_record_for("import-busy", "hash-busy"),
            completed_manual_import_record_for("import-untracked", "hash-untracked"),
        ],
        canonical_download_ids: std::collections::HashMap::new(),
        windows: Mutex::new(Vec::new()),
        deleted_sources: Mutex::new(Vec::new()),
    });
    let (handle, asked) = scripted_tracked_download_runtime(vec![
        (
            "hash-busy",
            vec![
                ManualImportRecoveryOutcome::Busy,
                ManualImportRecoveryOutcome::Marked,
            ],
        ),
        (
            "hash-untracked",
            vec![
                ManualImportRecoveryOutcome::Untracked,
                ManualImportRecoveryOutcome::Untracked,
                ManualImportRecoveryOutcome::Marked,
            ],
        ),
    ]);
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    )
    .with_test_overrides(|builder| {
        builder
            .with_imports(repo.clone())
            .with_tracked_download_handle(handle)
    });
    let worker = PollingWorker::new(
        "manual_import_recovery_retry_test",
        tokio_util::sync::CancellationToken::new(),
    );
    let mut memo = std::collections::HashMap::new();
    let rewind_untracked =
        |memo: &mut std::collections::HashMap<String, ManualImportRecoveryMemo>,
         expected_attempts: u32| {
            // The loop backs off untracked records (30 s, 2 m, …); the tests do not
            // wait, they move the clock: the deferral must exist with the expected
            // attempt count, then it is made due.
            let Some(ManualImportRecoveryMemo::RetryAfter {
                next_check_at,
                attempts,
            }) = memo.get_mut("import-untracked")
            else {
                panic!("untracked record must be deferred, got {memo:?}");
            };
            assert_eq!(*attempts, expected_attempts);
            assert!(*next_check_at > chrono::Utc::now());
            *next_check_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        };

    recover_completed_manual_imports(&app, &worker, &mut memo).await;
    assert!(
        !memo.contains_key("import-busy"),
        "a busy source is asked again next tick, not remembered: {memo:?}"
    );
    rewind_untracked(&mut memo, 1);
    recover_completed_manual_imports(&app, &worker, &mut memo).await;
    assert_eq!(
        memo.get("import-busy"),
        Some(&ManualImportRecoveryMemo::Settled)
    );
    // Still untracked on its second (due) check: deferred again, longer.
    rewind_untracked(&mut memo, 2);
    recover_completed_manual_imports(&app, &worker, &mut memo).await;
    assert_eq!(
        memo.get("import-untracked"),
        Some(&ManualImportRecoveryMemo::Settled),
        "{memo:?}"
    );
    // A further tick asks nothing: both records are settled.
    recover_completed_manual_imports(&app, &worker, &mut memo).await;

    let asked = asked.lock().await.clone();
    assert_eq!(
        asked
            .iter()
            .filter(|(item, _, _)| item.as_str() == "hash-busy")
            .count(),
        2,
        "a busy source is retried once, then remembered"
    );
    assert_eq!(
        asked
            .iter()
            .filter(|(item, _, _)| item.as_str() == "hash-untracked")
            .count(),
        3,
        "an untracked source is retried on its backoff until the runtime knows it"
    );
    assert_eq!(
        memo,
        std::collections::HashMap::from([
            ("import-busy".to_string(), ManualImportRecoveryMemo::Settled),
            (
                "import-untracked".to_string(),
                ManualImportRecoveryMemo::Settled
            ),
        ])
    );
    let mut deleted = repo.deleted_sources.lock().await.clone();
    deleted.sort_by(|left, right| left.item_id.cmp(&right.item_id));
    assert_eq!(
        deleted,
        vec![
            crate::ClientJobLocator::new(Some("client-1"), "qbittorrent", "hash-busy"),
            crate::ClientJobLocator::new(Some("client-1"), "qbittorrent", "hash-untracked"),
        ]
    );
}

// ── manual import preview candidates ─────────────────────────────────────────

#[tokio::test]
async fn manual_import_preview_excludes_samples_for_movies_but_keeps_them_for_series() {
    let app = build_manual_import_cleanup_app(
        Vec::new(),
        Arc::new(ManualImportCleanupDownloadClient::default()),
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let video_fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scryer-mediainfo/tests/media/hevc_hdr10plus.mkv");
    let primary = dir.path().join("Manual.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::copy(&video_fixture, &primary).expect("copy primary video fixture");
    let named_sample = dir.path().join("Manual.Movie.2024.1080p.WEB-DL-sample.mkv");
    std::fs::copy(&video_fixture, &named_sample).expect("copy sample video fixture");
    let tiny_extra = dir.path().join("Manual.Movie.2024.Making.Of.mkv");
    std::fs::copy(&video_fixture, &tiny_extra).expect("copy extra video fixture");
    let mut completed = test_completed_download("Manual.Movie.2024.1080p.WEB-DL", dir.path());
    completed.release_name = Some("Manual.Movie.2024.1080p.WEB-DL".to_string());
    let evidence = observation_evidence(&completed);
    let mut movie_title = titled(MediaFacet::Movie, "Manual Movie", Some(2024));
    movie_title.id = "title-1".to_string();
    let mut series_title = titled(MediaFacet::Series, "Manual Movie", Some(2024));
    series_title.id = "title-1".to_string();

    let movie_preview = preview_manual_import(&app, dir.path(), &movie_title, &evidence, &[])
        .await
        .expect("movie preview");
    // The preview shows the quality the import will score — the release
    // evidence parsed with the title's context — for every file, including a
    // sibling whose own name carries no quality token.
    for file in &movie_preview.files {
        assert_eq!(
            file.quality.as_deref(),
            Some("1080p"),
            "{}: preview quality comes from the release evidence, not the file name",
            file.file_name
        );
    }
    let mut movie_files = movie_preview
        .files
        .iter()
        .map(|file| file.file_name.as_str())
        .collect::<Vec<_>>();
    movie_files.sort_unstable();
    assert_eq!(
        movie_files,
        vec![
            "Manual.Movie.2024.1080p.WEB-DL.mkv",
            "Manual.Movie.2024.Making.Of.mkv",
        ],
        "movie previews drop sample-named files but never size-filter (a small movie stays importable)"
    );

    let series_preview = preview_manual_import(&app, dir.path(), &series_title, &evidence, &[])
        .await
        .expect("series preview");
    let mut series_files = series_preview
        .files
        .iter()
        .map(|file| file.file_name.as_str())
        .collect::<Vec<_>>();
    series_files.sort_unstable();
    assert_eq!(
        series_files,
        vec![
            "Manual.Movie.2024.1080p.WEB-DL-sample.mkv",
            "Manual.Movie.2024.1080p.WEB-DL.mkv",
            "Manual.Movie.2024.Making.Of.mkv",
        ],
        "series previews keep every video for explicit mapping"
    );
}

/// Build the admission subject a plan-builder test needs.
///
/// Production resolves the incumbent bar canonically; these tests are about
/// ranking and span guards, so they take the stored score as the bar and keep
/// the fixtures readable.
fn test_admission_subject(
    incumbents: &[crate::EpisodeScopedMediaFile],
    target_episode_ids: &[&str],
) -> crate::admission::AdmissionSubject {
    crate::admission::AdmissionSubject::new(
        crate::admission::AdmissionScope::Episodes(
            target_episode_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
        ),
        incumbents.iter().map(|incumbent| {
            (
                crate::admission::Incumbent {
                    tier_index: Some(0),
                    revision: 0,
                    file_id: incumbent.media_file.id.clone(),
                    file_path: incumbent.media_file.file_path.clone(),
                    release_group: incumbent.media_file.release_group.clone(),
                    score: incumbent.media_file.acquisition_score.unwrap_or(0),
                    covers: incumbent.episode_ids.clone(),
                    created_at: incumbent.media_file.created_at.clone(),
                },
                incumbent.media_file.role.is_primary(),
            )
        }),
    )
}
