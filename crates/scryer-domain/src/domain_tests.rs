use super::*;
use std::{collections::HashMap, path::Path};

#[test]
fn media_server_connection_redacts_api_key_from_debug_and_serialization() {
    let now = chrono::Utc::now();
    let connection = MediaServerConnection {
        id: "emby-main".into(),
        provider: MediaServerProvider::Emby,
        display_name: "Emby".into(),
        base_url: "http://emby.test/emby".into(),
        external_url: None,
        enabled: true,
        login_enabled: true,
        linking_enabled: true,
        auto_add_enabled: false,
        default_app_permissions: AppPermissionMask::NONE,
        default_library_grants: vec![],
        machine_id: None,
        api_key: Some("super-secret-api-key".into()),
        emby_server_id: Some("server-id".into()),
        emby_connect_enabled: false,
        path_mappings: vec![],
        created_at: now,
        updated_at: now,
    };

    let debug = format!("{connection:?}");
    assert!(!debug.contains("super-secret-api-key"));
    assert!(debug.contains("[REDACTED]"));

    let mut serialized = serde_json::to_value(&connection).expect("serialize connection");
    assert!(serialized.get("api_key").is_none());
    serialized["api_key"] = serde_json::json!("restored-secret");
    let restored: MediaServerConnection =
        serde_json::from_value(serialized).expect("deserialize legacy/backup connection");
    assert_eq!(restored.api_key.as_deref(), Some("restored-secret"));
}

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
        password_change_required: false,
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
        IndexerProxyProviderType::Http,
        IndexerProxyProviderType::Socks4,
        IndexerProxyProviderType::Socks5,
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
    assert_eq!(
        IndexerProxyProviderType::parse(" SOCKS5 "),
        Some(IndexerProxyProviderType::Socks5)
    );
    assert_eq!(
        IndexerProxyProviderType::parse(" Socks4 "),
        Some(IndexerProxyProviderType::Socks4)
    );
    assert_eq!(IndexerProxyProviderType::parse("unknown"), None);
    // `socks5h` is not its own provider: it is Socks5 plus remote DNS.
    assert_eq!(IndexerProxyProviderType::parse("socks5h"), None);
    // `socks4a` is likewise Socks4 plus remote DNS, not a fifth provider.
    assert_eq!(IndexerProxyProviderType::parse("socks4a"), None);
}

#[test]
fn indexer_proxy_provider_types_split_solver_from_transport() {
    assert_eq!(
        IndexerProxyProviderType::Byparr.kind(),
        IndexerProxyKind::ChallengeSolver
    );
    assert_eq!(
        IndexerProxyProviderType::Trawl.kind(),
        IndexerProxyKind::ChallengeSolver
    );
    assert_eq!(
        IndexerProxyProviderType::Http.kind(),
        IndexerProxyKind::Transport
    );
    assert_eq!(
        IndexerProxyProviderType::Socks4.kind(),
        IndexerProxyKind::Transport
    );
    assert_eq!(
        IndexerProxyProviderType::Socks5.kind(),
        IndexerProxyKind::Transport
    );
    assert!(IndexerProxyProviderType::Trawl.is_challenge_solver());
    assert!(!IndexerProxyProviderType::Trawl.is_transport());
    assert!(IndexerProxyProviderType::Socks5.is_transport());
    assert!(!IndexerProxyProviderType::Socks5.is_challenge_solver());
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
        password_change_required: false,
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
        password_change_required: false,
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
fn seeding_profile_enums_round_trip() {
    assert_eq!(SeasonPackSeedMode::Inherit.as_str(), "inherit");
    assert_eq!(SeasonPackSeedMode::Override.as_str(), "override");
    assert_eq!(
        SeasonPackSeedMode::parse("override"),
        Some(SeasonPackSeedMode::Override)
    );
    assert_eq!(SeasonPackSeedMode::parse("season"), None);
    assert_eq!(SeasonPackSeedMode::default(), SeasonPackSeedMode::Inherit);

    assert_eq!(SeedGoalMetAction::RemoveEntry.as_str(), "remove_entry");
    assert_eq!(SeedGoalMetAction::StopSeeding.as_str(), "stop_seeding");
    assert_eq!(SeedGoalMetAction::Keep.as_str(), "keep");
    assert_eq!(
        SeedGoalMetAction::parse("stop_seeding"),
        Some(SeedGoalMetAction::StopSeeding)
    );
    assert_eq!(SeedGoalMetAction::parse("pause"), None);
    assert_eq!(SeedGoalMetAction::default(), SeedGoalMetAction::RemoveEntry);

    assert_eq!(PostImportTracking::Park.as_str(), "park");
    assert_eq!(PostImportTracking::HandOff.as_str(), "hand_off");
    for mode in [PostImportTracking::Park, PostImportTracking::HandOff] {
        assert_eq!(PostImportTracking::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(PostImportTracking::parse("handoff"), None);
    // Park is the fail-closed default: Scryer keeps managing the torrent.
    assert_eq!(PostImportTracking::default(), PostImportTracking::Park);
    assert!(PostImportTracking::HandOff.is_hand_off());
    assert!(!PostImportTracking::Park.is_hand_off());
}

#[test]
fn seeding_profile_normalizes_and_validates_goals() {
    let now = chrono::Utc::now();
    let base = SeedingProfile {
        id: "profile-1".into(),
        name: "  Private tracker  ".into(),
        ratio: Some(1.5),
        seed_time_minutes: Some(4320),
        season_pack_mode: SeasonPackSeedMode::Inherit,
        season_pack_ratio: Some(3.0),
        season_pack_seed_time_minutes: Some(120),
        honor_tracker_minimums: true,
        goal_met_action: SeedGoalMetAction::RemoveEntry,
        never_remove: false,
        minimum_seeders: None,
        post_import_tracking: PostImportTracking::Park,
        created_at: now,
        updated_at: now,
    };

    let inherited = base.clone().normalized();
    assert_eq!(inherited.name, "Private tracker");
    assert_eq!(inherited.season_pack_ratio, None);
    assert_eq!(inherited.season_pack_seed_time_minutes, None);
    assert_eq!(inherited.effective_ratio(true), Some(1.5));
    assert_eq!(inherited.effective_seed_time_minutes(true), Some(4320));

    let mut overridden = base.clone();
    overridden.season_pack_mode = SeasonPackSeedMode::Override;
    let overridden = overridden.normalized();
    assert_eq!(overridden.effective_ratio(true), Some(3.0));
    assert_eq!(overridden.effective_ratio(false), Some(1.5));
    assert_eq!(overridden.effective_seed_time_minutes(true), Some(120));

    assert!(inherited.validate().is_ok());

    let mut unnamed = base.clone();
    unnamed.name = "   ".into();
    assert!(unnamed.validate().is_err());

    let mut zero_ratio = base.clone();
    zero_ratio.ratio = Some(0.0);
    assert!(zero_ratio.validate().is_err());

    let mut negative_time = base;
    negative_time.seed_time_minutes = Some(-1);
    assert!(negative_time.validate().is_err());
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

// ── TrackedDownloadState::ImportedSeeding ─────────────────────────────────

#[test]
fn tracked_download_states_round_trip_through_their_wire_names() {
    for state in [
        TrackedDownloadState::Downloading,
        TrackedDownloadState::ImportPending,
        TrackedDownloadState::Importing,
        TrackedDownloadState::Imported,
        TrackedDownloadState::ImportedSeeding,
        TrackedDownloadState::ImportBlocked,
        TrackedDownloadState::FailedPending,
        TrackedDownloadState::Failed,
        TrackedDownloadState::Ignored,
    ] {
        assert_eq!(
            TrackedDownloadState::from_str_opt(state.as_str()),
            Some(state),
            "{} did not round-trip",
            state.as_str()
        );
    }
    assert_eq!(
        TrackedDownloadState::ImportedSeeding.as_str(),
        "imported_seeding"
    );
}

#[test]
fn imported_seeding_is_settled_but_not_terminal() {
    // Not terminal: the torrent is still live in the client and has to be
    // re-evaluated against its seeding goal on every poll.
    assert!(!TrackedDownloadState::ImportedSeeding.is_terminal());
    // Settled: the payload is already in the library, so no further import
    // work may be dispatched for it.
    assert!(TrackedDownloadState::ImportedSeeding.is_import_settled());
    assert!(TrackedDownloadState::ImportedSeeding.counts_as_imported());
    assert!(TrackedDownloadState::Imported.counts_as_imported());
    assert!(!TrackedDownloadState::Failed.counts_as_imported());
    assert!(!TrackedDownloadState::Downloading.is_import_settled());
}

// ── seeding history events ────────────────────────────────────────────────

#[test]
fn seeding_history_event_types_round_trip_through_their_wire_names() {
    for event_type in TitleHistoryEventType::ALL {
        assert_eq!(
            TitleHistoryEventType::parse(event_type.as_str()),
            Some(*event_type),
            "{} did not round-trip",
            event_type.as_str()
        );
    }
    assert_eq!(
        TitleHistoryEventType::SeedingStarted.as_str(),
        "seeding_started"
    );
    assert_eq!(
        TitleHistoryEventType::SeedingCompleted.as_str(),
        "seeding_completed"
    );
}

#[test]
fn seeding_domain_event_types_round_trip_and_match_their_payloads() {
    for event_type in DomainEventType::variants() {
        assert_eq!(
            DomainEventType::parse(event_type.as_str()),
            Some(event_type),
            "{} did not round-trip",
            event_type.as_str()
        );
    }

    let started = DomainEventPayload::SeedingStarted(SeedingStartedEventData {
        title: None,
        download_client_item_id: "hash-1".to_string(),
        client_id: None,
        client_type: Some("qbittorrent".to_string()),
        source_provider: None,
        source_title: None,
        reason: "profile_goal_unmet".to_string(),
        seed_ratio: Some(0.4),
        seed_time_seconds: Some(120),
    });
    assert_eq!(started.event_type(), DomainEventType::SeedingStarted);

    let completed = DomainEventPayload::SeedingCompleted(SeedingCompletedEventData {
        title: None,
        download_client_item_id: "hash-1".to_string(),
        client_id: None,
        client_type: Some("qbittorrent".to_string()),
        source_provider: None,
        source_title: None,
        action: "removed".to_string(),
        reason: "profile_goal_met".to_string(),
        seed_ratio: Some(2.1),
        seed_time_seconds: Some(90_000),
    });
    assert_eq!(completed.event_type(), DomainEventType::SeedingCompleted);

    // Payloads are persisted as JSON and read back by the history projection.
    let encoded = serde_json::to_string(&completed).expect("serialize seeding payload");
    assert_eq!(
        serde_json::from_str::<DomainEventPayload>(&encoded).expect("deserialize"),
        completed
    );
}
