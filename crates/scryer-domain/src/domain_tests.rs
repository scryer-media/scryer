use super::*;
use std::{collections::HashMap, path::Path};

#[test]
fn library_root_trimming_preserves_filesystem_roots() {
    assert_eq!(trim_library_root_path(" / "), "/");
    assert_eq!(trim_library_root_path("C:\\"), "C:\\");
    assert_eq!(
        trim_library_root_path("\\\\server\\share\\"),
        "\\\\server\\share\\"
    );
    assert_eq!(trim_library_root_path("/media/movies/"), "/media/movies");
}

#[test]
fn library_root_comparison_canonicalizes_unc_trailing_separators() {
    assert_eq!(
        normalize_library_root_path("\\\\server\\share\\"),
        normalize_library_root_path("\\\\server\\share")
    );
}

// ── is_video_file ─────────────────────────────────────────────────────────

#[test]
fn video_file_mkv() {
    assert!(is_video_file(Path::new("/data/movie.mkv")));
}

#[test]
fn video_file_mp4() {
    assert!(is_video_file(Path::new("/downloads/episode.mp4")));
}

#[test]
fn video_file_avi() {
    assert!(is_video_file(Path::new("old_movie.avi")));
}

#[test]
fn video_file_m2ts() {
    assert!(is_video_file(Path::new("/bluray/BDMV/STREAM/00001.m2ts")));
}

#[test]
fn video_file_webm() {
    assert!(is_video_file(Path::new("clip.webm")));
}

#[test]
fn video_file_strm() {
    assert!(is_video_file(Path::new("/mounts/nzbdav/Show.S01E01.strm")));
}

#[test]
fn video_file_case_insensitive() {
    assert!(is_video_file(Path::new("movie.MKV")));
    assert!(is_video_file(Path::new("movie.Mp4")));
}

#[test]
fn video_file_trailing_quote_extension() {
    let path = Path::new("Fixture.Payload.mkv\"");
    assert!(is_video_file(path));
    assert_eq!(canonical_video_extension(path), Some("mkv"));
}

#[test]
fn video_file_trailing_sanitized_underscore_extension() {
    let path = Path::new("Fixture.Payload.mkv_");
    assert!(is_video_file(path));
    assert_eq!(canonical_video_extension(path), Some("mkv"));
}

#[test]
fn video_file_trailing_dot_extension() {
    let path = Path::new("Fixture.Payload.mkv.");
    assert!(is_video_file(path));
    assert_eq!(canonical_video_extension(path), Some("mkv"));
}

#[test]
fn video_file_double_extension_is_not_video() {
    assert!(!is_video_file(Path::new("Fixture.Payload.mkv.exe")));
}

#[test]
fn video_file_sanitized_artifact_before_executable_extension_is_not_video() {
    assert!(!is_video_file(Path::new("Fixture.Payload.mkv_.exe")));
}

#[test]
fn not_video_file_subtitle() {
    assert!(!is_video_file(Path::new("movie.srt")));
}

#[test]
fn not_video_file_nfo() {
    assert!(!is_video_file(Path::new("movie.nfo")));
}

#[test]
fn not_video_file_image() {
    assert!(!is_video_file(Path::new("poster.jpg")));
}

#[test]
fn not_video_file_no_extension() {
    assert!(!is_video_file(Path::new("README")));
}

#[test]
fn not_video_file_directory() {
    assert!(!is_video_file(Path::new("/data/movies/")));
}

#[test]
fn not_video_file_nzb() {
    assert!(!is_video_file(Path::new("download.nzb")));
}

// ── subtitle/image classification ──────────────────────────────────────────

#[test]
fn subtitle_file_srt() {
    assert!(is_subtitle_file(Path::new("movie.eng.srt")));
    assert!(is_subtitle_file(Path::new("movie.FORCED.ASS")));
}

#[test]
fn not_subtitle_file_video() {
    assert!(!is_subtitle_file(Path::new("movie.mkv")));
}

#[test]
fn image_file_jpeg() {
    assert!(is_image_file(Path::new("poster.jpg")));
    assert!(is_image_file(Path::new("fanart.WEBP")));
}

#[test]
fn not_image_file_subtitle() {
    assert!(!is_image_file(Path::new("movie.eng.srt")));
}

#[test]
fn domain_event_type_all_includes_title_rematched() {
    assert!(DomainEventType::variants().any(|value| value == DomainEventType::TitleRematched));
}

#[test]
fn catalog_settings_permission_does_not_include_system_settings() {
    let mut user = User {
        id: Id::new().0,
        username: "catalog-settings".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization::default(),
    };
    user.authorization.loaded = true;
    user.authorization
        .app
        .insert(AppPermissionMask::MANAGE_CATALOG_SETTINGS);

    assert!(
        !user
            .authorization
            .app
            .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
    );
}

#[test]
fn explicit_query_facets_gate_text_search() {
    let caps = IndexerProviderCapabilities {
        query_param: Some("q".to_string()),
        supported_query_facets: vec!["movie".to_string(), "anime".to_string()],
        ..IndexerProviderCapabilities::default()
    };

    assert!(caps.supports_query_for_facet("movie"));
    assert!(caps.supports_query_for_facet("ANIME"));
    assert!(!caps.supports_query_for_facet("series"));
}

#[test]
fn legacy_supported_id_facets_imply_text_search() {
    let caps = IndexerProviderCapabilities {
        query_param: Some("q".to_string()),
        supported_ids: HashMap::from([("anime".to_string(), vec!["anidb_id".to_string()])]),
        ..IndexerProviderCapabilities::default()
    };

    assert!(caps.supports_query_for_facet("anime"));
    assert!(!caps.supports_query_for_facet("movie"));
}

#[test]
fn legacy_query_only_caps_accept_current_facets() {
    let caps = IndexerProviderCapabilities {
        query_param: Some("q".to_string()),
        ..IndexerProviderCapabilities::default()
    };

    assert!(caps.supports_query_for_facet("movie"));
    assert!(caps.supports_query_for_facet("series"));
    assert!(caps.supports_query_for_facet("anime"));
    assert!(!caps.supports_query_for_facet("music"));
}

#[test]
fn missing_query_param_disables_query_facets() {
    let caps = IndexerProviderCapabilities {
        supported_query_facets: vec!["movie".to_string()],
        ..IndexerProviderCapabilities::default()
    };

    assert!(!caps.supports_query_for_facet("movie"));
}

// ── match_fuzzy ───────────────────────────────────────────────────────────

#[test]
fn fuzzy_exact_match() {
    assert!(match_fuzzy("Velvet Comet", "velvet comet"));
}

#[test]
fn fuzzy_partial_match_beginning() {
    assert!(match_fuzzy("Velvet Comet", "vel"));
}

#[test]
fn fuzzy_partial_match_middle() {
    assert!(match_fuzzy("Velvet Comet", "vet co"));
}

#[test]
fn fuzzy_partial_match_end() {
    assert!(match_fuzzy("Velvet Comet", "comet"));
}

#[test]
fn fuzzy_case_insensitive() {
    assert!(match_fuzzy("Velvet Comet", "VELVET"));
    assert!(match_fuzzy("velvet comet", "COMET"));
}

#[test]
fn fuzzy_no_match() {
    assert!(!match_fuzzy("Velvet Comet", "solara"));
}

#[test]
fn fuzzy_empty_query_matches_everything() {
    assert!(match_fuzzy("Velvet Comet", ""));
    assert!(match_fuzzy("", ""));
}

#[test]
fn fuzzy_empty_candidate_no_match() {
    assert!(!match_fuzzy("", "cowboy"));
}

#[test]
fn fuzzy_whitespace_query() {
    assert!(match_fuzzy("Velvet Comet", "  "));
}

// ── normalize_tags ────────────────────────────────────────────────────────

#[test]
fn tags_lowercased() {
    let result = normalize_tags(&["Anime".into(), "ACTION".into()]);
    assert_eq!(result, vec!["action", "anime"]);
}

#[test]
fn tags_deduplication() {
    let result = normalize_tags(&["anime".into(), "Anime".into(), "ANIME".into()]);
    assert_eq!(result, vec!["anime"]);
}

#[test]
fn tags_sorted() {
    let result = normalize_tags(&["zebra".into(), "alpha".into(), "middle".into()]);
    assert_eq!(result, vec!["alpha", "middle", "zebra"]);
}

#[test]
fn tags_whitespace_trimmed() {
    let result = normalize_tags(&[" anime ".into(), "  series  ".into()]);
    assert_eq!(result, vec!["anime", "series"]);
}

#[test]
fn tags_empty_strings_ignored() {
    let result = normalize_tags(&["anime".into(), "".into(), "  ".into()]);
    assert_eq!(result, vec!["anime"]);
}

#[test]
fn tags_scryer_prefix_preserves_case() {
    let result = normalize_tags(&["scryer:season-folder:disabled".into(), "anime".into()]);
    assert!(result.contains(&"scryer:season-folder:disabled".to_string()));
    assert!(result.contains(&"anime".to_string()));
}

#[test]
fn tags_empty_input() {
    let result = normalize_tags(&[]);
    assert!(result.is_empty());
}

// ── config / notification helpers ────────────────────────────────────────

#[test]
fn config_field_type_supports_multiline() {
    assert_eq!(
        ConfigFieldType::parse("multiline"),
        Some(ConfigFieldType::Multiline)
    );
    assert_eq!(ConfigFieldType::Multiline.as_str(), "multiline");
    assert_eq!(ConfigFieldType::parse("path"), Some(ConfigFieldType::Path));
    assert_eq!(ConfigFieldType::Path.as_str(), "path");
    assert_eq!(ConfigFieldType::parse("tag"), Some(ConfigFieldType::Tag));
    assert_eq!(ConfigFieldType::Tag.as_str(), "tag");
}

#[test]
fn indexer_proxy_provider_types_round_trip() {
    for provider in [
        IndexerProxyProviderType::Byparr,
        IndexerProxyProviderType::Trawl,
    ] {
        assert_eq!(
            IndexerProxyProviderType::parse(provider.as_str()),
            Some(provider)
        );
    }
    assert_eq!(
        IndexerProxyProviderType::parse("TRAWL"),
        Some(IndexerProxyProviderType::Trawl)
    );
    assert_eq!(IndexerProxyProviderType::parse("unknown"), None);
}

#[test]
fn notification_channel_type_normalizes_provider_string() {
    let provider = ChannelType::parse("  Jellyfin  ").expect("provider");
    assert_eq!(provider.as_str(), "jellyfin");
}

// ── User permission masks ─────────────────────────────────────────────────

#[test]
fn admin_has_full_permission_masks() {
    let admin = User::new_admin("root");
    assert!(admin.authorization.loaded);
    assert!(
        admin
            .authorization
            .default_library
            .contains(LibraryPermissionMask::VIEW)
    );
    assert!(
        admin
            .authorization
            .default_library
            .contains(LibraryPermissionMask::MANAGE_TITLES)
    );
    assert!(
        admin
            .authorization
            .app
            .contains(AppPermissionMask::MANAGE_USERS)
    );
    assert!(
        admin
            .authorization
            .app
            .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
    );
}

#[test]
fn user_with_limited_permission_masks() {
    let user = User {
        id: Id::new().0,
        username: "viewer".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            default_library: LibraryPermissionMask::from_permissions([
                LibraryPermission::View,
                LibraryPermission::ManageTitles,
            ]),
            actor_capabilities: ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            ..Default::default()
        },
    };
    assert!(
        user.authorization
            .default_library
            .contains(LibraryPermissionMask::VIEW)
    );
    assert!(
        user.authorization
            .default_library
            .contains(LibraryPermissionMask::MANAGE_TITLES)
    );
    assert!(
        !user
            .authorization
            .app
            .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
    );
    assert!(
        !user
            .authorization
            .app
            .contains(AppPermissionMask::MANAGE_USERS)
    );
}

#[test]
fn user_with_no_permission_masks() {
    let user = User {
        id: Id::new().0,
        username: "empty".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    assert!(
        !user
            .authorization
            .default_library
            .contains(LibraryPermissionMask::VIEW)
    );
    assert!(user.authorization.app.is_empty());
}

#[test]
fn user_with_password_hash_has_full_permission_masks() {
    let user = User::with_password_hash("admin", "hashed_pw");
    assert!(user.authorization.loaded);
    assert!(
        user.authorization
            .app
            .contains(AppPermissionMask::MANAGE_USERS)
    );
    assert_eq!(user.password_hash.as_deref(), Some("hashed_pw"));
}

// ── ImportStatus / ImportDecision as_str ───────────────────────────────────

#[test]
fn import_status_as_str() {
    assert_eq!(ImportStatus::Pending.as_str(), "pending");
    assert_eq!(ImportStatus::Running.as_str(), "running");
    assert_eq!(ImportStatus::Processing.as_str(), "processing");
    assert_eq!(ImportStatus::Completed.as_str(), "completed");
    assert_eq!(ImportStatus::Failed.as_str(), "failed");
    assert_eq!(ImportStatus::Skipped.as_str(), "skipped");
    assert!(ImportStatus::Pending.is_active());
    assert!(ImportStatus::Running.is_active());
    assert!(ImportStatus::Processing.is_active());
    assert!(!ImportStatus::Completed.is_active());
}

#[test]
fn import_decision_as_str() {
    assert_eq!(ImportDecision::Imported.as_str(), "imported");
    assert_eq!(ImportDecision::Rejected.as_str(), "rejected");
    assert_eq!(ImportDecision::Skipped.as_str(), "skipped");
    assert_eq!(ImportDecision::Conflict.as_str(), "conflict");
    assert_eq!(ImportDecision::Unmatched.as_str(), "unmatched");
    assert_eq!(ImportDecision::Failed.as_str(), "failed");
}

#[test]
fn import_type_parse_includes_manual_import() {
    assert_eq!(ImportType::ManualImport.as_str(), "manual_import");
    assert_eq!(
        ImportType::parse("manual_import"),
        Some(ImportType::ManualImport)
    );
}

#[test]
fn import_error_code_round_trips() {
    assert_eq!(ImportErrorCode::FileNotFound.as_str(), "file_not_found");
    assert_eq!(
        ImportErrorCode::parse("episode_lookup_failed"),
        Some(ImportErrorCode::EpisodeLookupFailed)
    );
    assert_eq!(
        ImportErrorCode::SourceJobFailed.as_str(),
        "source_job_failed"
    );
    assert_eq!(
        ImportErrorCode::parse("source_job_failed"),
        Some(ImportErrorCode::SourceJobFailed)
    );
    assert_eq!(
        ImportErrorCode::parse("policy_mismatch"),
        Some(ImportErrorCode::PolicyMismatch)
    );
    assert_eq!(
        ImportErrorCode::parse("unknown"),
        Some(ImportErrorCode::Unknown)
    );
}

#[test]
fn import_mode_as_str_and_setting_parse() {
    assert_eq!(ImportMode::HardlinkOrCopy.as_str(), "hardlink_or_copy");
    assert_eq!(ImportMode::Move.as_str(), "move");
    assert_eq!(
        ImportMode::from_setting("hardlink_or_copy"),
        Ok(ImportMode::HardlinkOrCopy)
    );
    assert_eq!(ImportMode::from_setting("move"), Ok(ImportMode::Move));
    assert!(ImportMode::from_setting("auto").is_err());
}

#[test]
fn import_strategy_as_str() {
    assert_eq!(ImportStrategy::HardLink.as_str(), "hardlink");
    assert_eq!(ImportStrategy::Copy.as_str(), "copy");
    assert_eq!(ImportStrategy::Symlink.as_str(), "symlink");
    assert_eq!(ImportStrategy::Move.as_str(), "move");
}

// ── NewTitle ──────────────────────────────────────────────────────────────

#[test]
fn new_title_with_defaults() {
    let title = NewTitle::with_defaults("Test Movie", MediaFacet::Movie);
    assert_eq!(title.name, "Test Movie");
    assert_eq!(title.facet, MediaFacet::Movie);
    assert!(title.monitored);
    assert!(title.tags.is_empty());
    assert!(title.external_ids.is_empty());
}

// ── parse_query ───────────────────────────────────────────────────────────

#[test]
fn parse_query_trims_and_lowercases() {
    assert_eq!(parse_query("  Velvet Comet  "), "velvet comet");
    assert_eq!(parse_query("UPPERCASE"), "uppercase");
}

#[test]
fn config_field_type_accepts_secret_alias() {
    assert_eq!(
        ConfigFieldType::parse("secret"),
        Some(ConfigFieldType::Password)
    );
}
