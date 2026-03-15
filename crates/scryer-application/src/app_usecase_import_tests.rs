use super::*;
use crate::post_download_gate::{facet_to_category_hint, missing_audio_languages};
use scryer_domain::{MediaFacet, Title};

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_title(facet: MediaFacet) -> Title {
    Title {
        id: "t1".to_string(),
        name: "Test Movie".to_string(),
        facet,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        banner_url: None,
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
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
    }
}

fn test_parsed() -> crate::ParsedReleaseMetadata {
    crate::parse_release_metadata("Test.Movie.2024.1080p.WEB-DL.DDP5.1.H.264-Group")
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

// ── is_sample_file ────────────────────────────────────────────────────────────

#[test]
fn is_sample_file_detects_sample_in_stem() {
    assert!(is_sample_file(std::path::Path::new(
        "/media/episode.sample.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new(
        "/media/sample-show.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new("/media/SAMPLE.mkv")));
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
    let title = test_title(MediaFacet::Tv);
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_true_when_tag_enabled() {
    let mut title = test_title(MediaFacet::Tv);
    title.tags = vec!["scryer:season-folder:enabled".to_string()];
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_false_when_tag_disabled() {
    let mut title = test_title(MediaFacet::Tv);
    title.tags = vec!["scryer:season-folder:disabled".to_string()];
    assert!(!use_season_folders(&title));
}

#[test]
fn use_season_folders_false_case_insensitive() {
    let mut title = test_title(MediaFacet::Tv);
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
fn build_rename_tokens_includes_quality() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("quality").map(String::as_str), Some("1080p"));
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
    let title = test_title(MediaFacet::Tv);
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

// ── missing_audio_languages ───────────────────────────────────────────────────

#[test]
fn missing_audio_languages_all_present() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["jpn".to_string(), "eng".to_string()];
    assert!(missing_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_case_normalization() {
    // media analysis emits lowercase codes; profile stores uppercase
    let required = vec!["JPN".to_string()];
    let actual = vec!["jpn".to_string()];
    assert!(missing_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_one_missing() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["eng".to_string()];
    let missing = missing_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["JPN"]);
}

#[test]
fn missing_audio_languages_all_missing() {
    let required = vec!["JPN".to_string()];
    let actual = vec!["eng".to_string(), "spa".to_string()];
    let missing = missing_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["JPN"]);
}

#[test]
fn missing_audio_languages_empty_required_always_passes() {
    let required: Vec<String> = vec![];
    let actual = vec!["eng".to_string()];
    assert!(missing_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_empty_actual_returns_all_required() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual: Vec<String> = vec![];
    let missing = missing_audio_languages(&required, &actual);
    assert_eq!(missing.len(), 2);
}

// ── facet_to_category_hint ────────────────────────────────────────────────────

#[test]
fn facet_to_category_hint_values() {
    assert_eq!(facet_to_category_hint(&MediaFacet::Movie), "movie");
    assert_eq!(facet_to_category_hint(&MediaFacet::Tv), "tv");
    assert_eq!(facet_to_category_hint(&MediaFacet::Anime), "anime");
    assert_eq!(facet_to_category_hint(&MediaFacet::Other), "other");
}
