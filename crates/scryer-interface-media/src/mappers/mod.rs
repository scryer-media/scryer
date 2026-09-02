use crate::types::*;
use async_graphql::ID;
use chrono::{DateTime, Utc};
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    ActivityEvent, ActivityWindowCounts, AppUseCase, BackupInfo, CatalogDiscoveryGroup,
    CatalogDiscoveryGroupKind, CatalogDiscoveryQuery, CatalogDiscoveryResult,
    CatalogDiscoverySurface, DashboardActivityStats, DeletePreview, DiscoveryFacetRecord,
    DiscoveryHomeFilterOptions, DiscoveryHomeFilters, DiscoveryHomeQuery, DiscoveryHomeResult,
    DiscoveryItemDetailQuery, DiscoveryItemRecord, DiscoveryItemsQuery, DiscoveryItemsResult,
    DiscoverySectionResult, DiscoverySyncStateRecord, DiscoverySyncStatus,
    DownloadClientRoutingSettingsEntry, EpisodeMediaAvailability, EpisodeMediaAvailabilityState,
    FacetScoringPersonaSelection, IgnorePendingImportResult, ImageProxyKind,
    IndexerRoutingSettingsEntry, IndexerSearchResult, JobDefinition, JobRun, LibraryPathsSettings,
    LibraryScanSummary, LibrarySettings, ManualPluginPreview, MediaRequestCounts, MediaSettings,
    ParsedEpisodeMetadata, ParsedReleaseMetadata, PendingImportConnection, PendingImportCounts,
    PendingImportItem, PendingImportSearchAttempt, PendingRelease, PluginCatalogStatus,
    ProxyTestResult, QualityProfile, QualityProfileCriteria, QualityProfileDecision,
    QualityProfileSelection, QualityProfileSettings, RegistryPlugin, RenameApplyItemResult,
    RenameApplyResult, RenamePlan, RenamePlanItem, ResolvePendingImportResult, RssSyncReport,
    ScoringEntry, ScoringSource, ServiceSettings, SmgScryerUpdateNotice,
    SmgVersionCompatibilityNotice, StorageRootUsage, SubmissionScope, SystemHealth, TitleCredit,
    TitleHistoryPage, TitleRatingSummary, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    CalendarEpisode, Collection, ConfigFieldDef, ConfigFieldType, DomainEvent,
    DownloadClientConfig, DownloadQueueItem, Episode, IndexerConfig, Library, MediaFacet,
    MediaRequest, PluginInstallation, PluginSupportTier, PostImportTracking, ProxyConfig, RuleSet,
    SeasonPackSeedMode, SeedGoalMetAction, SeedingProfile, SubtitleProviderConfig, Title,
    TitleHistoryRecord, User,
};
use scryer_rules;
use serde_json::Value;

mod acquisition;
mod configuration;
mod discovery;
mod identity;
mod library;
mod location;
mod maintenance_rules;
mod runtime;
mod scalars;

pub use acquisition::*;
pub use configuration::*;
pub use discovery::*;
pub use identity::*;
pub use library::*;
pub use location::*;
pub use maintenance_rules::*;
pub use runtime::*;
pub use scalars::parse_iso_date;

use scalars::{parse_date, parse_datetime, parse_optional_datetime, parse_required_datetime};

#[cfg(test)]
mod tests {
    use super::discovery::{discovery_surface_value, preferred_discovery_poster_source};
    use super::{
        discovery_home_query_from_input, from_download_queue_item, from_import_record,
        from_indexer_config_with_fields, from_proxy_config, from_title_history_record,
        from_wanted_item, provider_config_values_from_json_with_fields,
        provider_config_values_to_json,
    };
    use crate::types::{
        BoolConfigValuePayload, DiscoveryHomeFiltersInput, DiscoveryHomeInput,
        DiscoverySurfaceValue, FloatConfigValuePayload, IntConfigValuePayload, MediaFacetValue,
        PluginConfigFieldTypeValue, ProviderConfigFieldValue, ProviderConfigValueInput,
        SecretConfigValuePayload,
    };
    use chrono::Utc;
    use scryer_application::{AcquisitionScopeState, AcquisitionScopeStatus};
    use scryer_domain::{
        ChallengeSolverProtocol, CompletedDownload, ConfigFieldDef, ConfigFieldType,
        ConfigFieldValueSource, ImportRecord, ImportStatus, ImportType, IndexerConfig, ProxyConfig,
        ProxyProviderType, TitleHistoryEventType, TitleHistoryRecord,
    };
    use serde_json::{Value, json};

    fn config_field(key: &str, label: &str, field_type: ConfigFieldType) -> ConfigFieldDef {
        ConfigFieldDef {
            key: key.to_string(),
            label: label.to_string(),
            field_type,
            required: field_type == ConfigFieldType::Password,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: Vec::new(),
            help_text: None,
        }
    }

    fn config_input(key: &str) -> ProviderConfigValueInput {
        ProviderConfigValueInput {
            key: key.to_string(),
            string_value: None,
            bool_value: None,
            int_value: None,
            float_value: None,
            secret_value: None,
            clear_secret: None,
        }
    }

    #[test]
    fn discovery_posters_prefer_tmdb_paths_over_tvdb_urls() {
        assert_eq!(
            preferred_discovery_poster_source(
                Some("/poster.jpg"),
                Some("https://artworks.thetvdb.com/banners/poster.jpg"),
            )
            .as_deref(),
            Some("https://image.tmdb.org/t/p/original/poster.jpg")
        );
    }

    #[test]
    fn discovery_posters_retain_the_supplied_url_without_a_tmdb_path() {
        assert_eq!(
            preferred_discovery_poster_source(
                Some("not-a-tmdb-path"),
                Some("https://artworks.thetvdb.com/banners/poster.jpg"),
            )
            .as_deref(),
            Some("https://artworks.thetvdb.com/banners/poster.jpg")
        );
    }

    fn wanted_item_fixture() -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: Some("Example".to_string()),
            title_slug: Some("example".to_string()),
            title_facet: Some("movie".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2026-06-19T00:00:00Z".to_string(),
            updated_at: "2026-06-19T00:00:00Z".to_string(),
        }
    }

    fn title_history_record_fixture() -> TitleHistoryRecord {
        TitleHistoryRecord {
            id: "history-1".to_string(),
            title_id: "title-1".to_string(),
            title_name: Some("Example".to_string()),
            poster_url: None,
            library_id: None,
            facet: None,
            size_bytes: None,
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: None,
            event_type: TitleHistoryEventType::Grabbed,
            actor_kind: None,
            actor_user_id: None,
            actor_display_name: None,
            source_title: None,
            display_title: None,
            source_system: None,
            source_ref: None,
            source_hint: None,
            quality: None,
            download_id: None,
            client_id: None,
            client_name: None,
            import_id: None,
            skip_reason: None,
            retry_requires_password: false,
            failure_reason: None,
            blocklist_reason: None,
            source_path: None,
            dest_path: None,
            data_json: None,
            occurred_at: "2026-06-19T00:00:00Z".to_string(),
            created_at: "2026-06-19T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn provider_config_projection_redacts_secret_values() {
        let fields = vec![
            config_field("api_key", "API key", ConfigFieldType::Password),
            config_field("base_url", "Base URL", ConfigFieldType::String),
        ];
        let values = provider_config_values_from_json_with_fields(
            Some(
                r#"{
                  "api_key": "super-secret",
                  "username": "operator",
                  "base_url": "https://example.test",
                  "enabled": true,
                  "retries": 3,
                  "ratio": 1.5,
                  "metadata": {"nested": true}
                }"#,
            ),
            &fields,
        );

        let field = |key: &str| {
            values
                .iter()
                .find(|value| value.key == key)
                .unwrap_or_else(|| panic!("{key} should be present"))
        };

        let api_key = field("api_key");
        assert_eq!(api_key.label.as_deref(), Some("API key"));
        assert!(matches!(
            api_key.field_type,
            Some(PluginConfigFieldTypeValue::Password)
        ));
        assert!(api_key.required);
        assert!(matches!(
            api_key.value,
            Some(ProviderConfigFieldValue::Secret(SecretConfigValuePayload {
                stored: true
            }))
        ));
        let username = field("username");
        assert!(matches!(
            username.value,
            Some(ProviderConfigFieldValue::Secret(SecretConfigValuePayload {
                stored: true
            }))
        ));

        let base_url = field("base_url");
        match &base_url.value {
            Some(ProviderConfigFieldValue::String(payload)) => {
                assert_eq!(payload.value, "https://example.test");
            }
            _ => panic!("base_url should be a string value"),
        }
        assert!(matches!(
            field("enabled").value,
            Some(ProviderConfigFieldValue::Bool(BoolConfigValuePayload {
                value: true
            }))
        ));
        assert!(matches!(
            field("retries").value,
            Some(ProviderConfigFieldValue::Int(IntConfigValuePayload {
                value: 3
            }))
        ));
        assert!(matches!(
            field("ratio").value,
            Some(ProviderConfigFieldValue::Float(FloatConfigValuePayload { value })) if value == 1.5
        ));
        let metadata = field("metadata");
        assert!(metadata.value.is_none());
    }

    #[test]
    fn provider_config_inputs_use_secret_value_and_clear_secret() {
        let mut secret = config_input("api_key");
        secret.secret_value = Some("replacement-secret".to_string());
        let mut cleared = config_input("optional_password");
        cleared.clear_secret = Some(true);
        let mut base_url = config_input("base_url");
        base_url.string_value = Some("https://example.test".to_string());

        let raw = provider_config_values_to_json(vec![secret, cleared, base_url])
            .expect("typed config values should serialize");
        let value: Value = serde_json::from_str(&raw).expect("config should be valid json");

        assert_eq!(value["api_key"], json!("replacement-secret"));
        assert_eq!(value["optional_password"], Value::Null);
        assert_eq!(value["base_url"], json!("https://example.test"));
    }

    #[test]
    fn provider_config_inputs_reject_ambiguous_secret_values() {
        let mut value = config_input("api_key");
        value.secret_value = Some("replacement-secret".to_string());
        value.string_value = Some("projected-secret".to_string());

        let err = provider_config_values_to_json(vec![value])
            .expect_err("ambiguous config values should be rejected");
        assert!(
            err.to_string()
                .contains("config value 'api_key' must provide exactly one value"),
            "{err}"
        );
    }

    #[test]
    fn wanted_item_mapper_rejects_invalid_persisted_media_type() {
        let mut item = wanted_item_fixture();
        item.media_type = "mystery".to_string();

        let err = match from_wanted_item(item) {
            Ok(_) => panic!("invalid media type should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("invalid wanted item media_type 'mystery'"),
            "{err}"
        );
    }

    #[test]
    fn title_history_mapper_rejects_invalid_persisted_timestamps() {
        let mut record = title_history_record_fixture();
        record.occurred_at = "not-a-date".to_string();

        let err = match from_title_history_record(record) {
            Ok(_) => panic!("invalid timestamp should be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("invalid title history occurred_at timestamp"),
            "{err}"
        );
    }

    #[test]
    fn from_import_record_uses_release_folder_for_numeric_weaver_job_name() {
        let payload = CompletedDownload {
            client_type: "weaver".to_string(),
            client_id: String::new(),
            download_client_item_id: "10495".to_string(),
            download_id: None,
            name: "10495".to_string(),
            release_name: None,
            dest_dir: "/downloads/Example.Show.S01E01.1080p.WEB-DL".to_string(),
            category: Some("anime".to_string()),
            size_bytes: None,
            completed_at: None,
            parameters: vec![("*scryer_facet".to_string(), "anime".to_string())],
        };
        let record = ImportRecord {
            id: "import-1".to_string(),
            source_client_id: None,
            source_system: "weaver".to_string(),
            source_ref: "10495".to_string(),
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            import_type: ImportType::SeriesDownload,
            status: ImportStatus::Completed,
            payload_json: serde_json::to_string(&payload).expect("serialize completed download"),
            result_json: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-04-27T20:17:00Z".to_string(),
            updated_at: "2026-04-27T20:17:00Z".to_string(),
        };

        let mapped = from_import_record(record);
        assert_eq!(
            mapped.source_title.as_deref(),
            Some("Example.Show.S01E01.1080p.WEB-DL")
        );
        assert!(matches!(mapped.facet, Some(MediaFacetValue::Anime)));
    }

    #[test]
    fn discovery_home_filters_require_valid_ranges() {
        let invalid_rating = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                minimum_rating: Some(10.1),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("ratings above ten must be rejected");
        assert!(invalid_rating.to_string().contains("minimumRating"));

        let invalid_years = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                minimum_year: Some(2025),
                maximum_year: Some(2024),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("inverted years must be rejected");
        assert!(invalid_years.to_string().contains("minimumYear"));

        let invalid_limit = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            limit_per_section: Some(0),
            ..Default::default()
        }))
        .expect_err("zero card limits must be rejected");
        assert!(invalid_limit.to_string().contains("limitPerSection"));

        let blank_tag_key = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                genre_tag_keys: Some(vec!["  ".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect_err("blank canonical tag keys must be rejected");
        assert!(blank_tag_key.to_string().contains("genreTagKeys"));
    }

    #[test]
    fn discovery_home_filters_map_media_enums_and_canonical_tag_keys() {
        let query = discovery_home_query_from_input(Some(DiscoveryHomeInput {
            filters: Some(DiscoveryHomeFiltersInput {
                content_types: Some(vec![MediaFacetValue::Anime, MediaFacetValue::Movie]),
                genre_tag_keys: Some(vec!["Canonical:Genre:Drama".to_string()]),
                theme_tag_keys: Some(vec!["canonical:theme:found-family".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .expect("semantic filters should map");

        assert_eq!(query.filters.content_types, ["anime", "movie"]);
        assert_eq!(query.filters.genre_tag_keys, ["Canonical:Genre:Drama"]);
        assert_eq!(
            query.filters.theme_tag_keys,
            ["canonical:theme:found-family"]
        );
    }

    #[test]
    fn discovery_home_section_maps_mixed_surface() {
        let surface =
            discovery_surface_value("mixed").expect("mixed is a supported discovery home surface");

        assert!(matches!(surface, DiscoverySurfaceValue::Mixed));
    }

    #[test]
    fn discovery_home_section_rejects_unknown_surface_without_panicking() {
        let error = discovery_surface_value("unknown")
            .err()
            .expect("unknown surfaces must be returned as validation errors");

        assert!(error.to_string().contains("unsupported surface"));
    }

    #[test]
    fn discovery_home_rejects_unknown_media_facets_without_panicking() {
        let error = super::discovery::discovery_home_media_facet("documentary", "card targetKind")
            .err()
            .expect("unsupported media facets must be returned as validation errors");

        assert!(error.to_string().contains("card targetKind"));
    }

    fn torrent_queue_item() -> scryer_domain::DownloadQueueItem {
        scryer_domain::DownloadQueueItem {
            id: "qbittorrent:abc".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Example".to_string(),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "qBittorrent".to_string(),
            client_type: "qbittorrent".to_string(),
            state: scryer_domain::DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: Some(1024),
            remaining_seconds: Some(0),
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "abc".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        }
    }

    #[test]
    fn the_queue_payload_carries_seeding_progress_and_its_goals() {
        let mut item = torrent_queue_item();
        item.seeding = Some(scryer_domain::DownloadSeedingSnapshot {
            can_remove: Some(false),
            can_move_files: Some(true),
            seed_ratio: Some(0.75),
            seed_time_seconds: Some(5_400),
            is_private: Some(true),
            uploaded_bytes: Some(768),
            completed_at: None,
            seed_goal_ratio: Some(2.0),
            seed_goal_seconds: Some(86_400),
            never_remove: false,
        });

        let payload = from_download_queue_item(item);

        assert_eq!(payload.seed_ratio, Some(0.75));
        assert_eq!(payload.seed_ratio_goal, Some(2.0));
        assert_eq!(payload.seed_time_seconds.map(|value| value.0), Some(5_400));
        assert_eq!(
            payload.seed_time_goal_seconds.map(|value| value.0),
            Some(86_400)
        );
        assert_eq!(payload.is_private, Some(true));
        assert!(matches!(
            payload.seeding_state,
            Some(crate::types::DownloadSeedingStateValue::Seeding)
        ));
    }

    #[test]
    fn a_warned_row_reaches_the_api_as_warning_and_not_as_a_failure() {
        let mut item = torrent_queue_item();
        item.state = scryer_domain::DownloadQueueState::Warning;
        item.progress_percent = 42;
        item.attention_required = true;
        item.attention_reason = Some("files are missing from the save path".to_string());

        let payload = from_download_queue_item(item);

        assert!(matches!(
            payload.state,
            crate::types::DownloadQueueStateValue::Warning
        ));
        assert!(matches!(
            payload.display_state,
            crate::types::DownloadDisplayStateValue::Warning
        ));
        assert_eq!(
            payload.attention_reason.as_deref(),
            Some("files are missing from the save path")
        );
    }

    #[test]
    fn a_row_without_seeding_information_leaves_every_new_field_null() {
        let payload = from_download_queue_item(torrent_queue_item());

        assert_eq!(payload.seed_ratio, None);
        assert_eq!(payload.seed_ratio_goal, None);
        assert_eq!(payload.seed_time_seconds.map(|value| value.0), None);
        assert_eq!(payload.seed_time_goal_seconds.map(|value| value.0), None);
        assert_eq!(payload.is_private, None);
        assert!(payload.seeding_state.is_none());
    }

    fn prowlarr_child_indexer(managed_metadata: serde_json::Value) -> IndexerConfig {
        let now = Utc::now();
        IndexerConfig {
            id: "idx-managed".to_string(),
            name: "Managed child".to_string(),
            provider_type: "torznab".to_string(),
            base_url: "https://example.invalid".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some("prowlarr-parent".to_string()),
            managed_child_key: Some("7".to_string()),
            managed_metadata_json: Some(managed_metadata.to_string()),
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn managed_by_prowlarr_is_claimed_for_goals_only() {
        // The indexer dropdown reads this flag to say "Managed by Prowlarr" and
        // lock the seeding-profile picker. An imported `appMinimumSeeders` is an
        // admission threshold, not a seed goal, so on its own it must leave the
        // picker free.
        let goals = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7, "seed_ratio": 1.5 })),
            &[],
        );
        assert!(goals.has_prowlarr_seed_criteria);

        let minimum_only = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7, "minimum_seeders": 4 })),
            &[],
        );
        assert!(!minimum_only.has_prowlarr_seed_criteria);

        let nothing = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7 })),
            &[],
        );
        assert!(!nothing.has_prowlarr_seed_criteria);
    }

    #[test]
    fn the_imported_minimum_seeders_reads_back_on_the_indexer_payload() {
        // The flag above deliberately says nothing about admission, so the
        // imported threshold needs its own field or the operator cannot see
        // what governs the row.
        let imported = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7, "minimum_seeders": 4 })),
            &[],
        );
        assert_eq!(imported.prowlarr_minimum_seeders, Some(4));

        let absent = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7, "seed_ratio": 1.5 })),
            &[],
        );
        assert_eq!(absent.prowlarr_minimum_seeders, None);

        // Zero is Prowlarr's "do not enforce", not "inherit", so it must survive
        // the trip as `Some(0)` rather than collapsing into null.
        let disabled = from_indexer_config_with_fields(
            prowlarr_child_indexer(json!({ "indexer_id": 7, "minimum_seeders": 0 })),
            &[],
        );
        assert_eq!(disabled.prowlarr_minimum_seeders, Some(0));
    }

    fn proxy_config(
        provider_type: ProxyProviderType,
        username: Option<&str>,
        password: Option<&str>,
    ) -> ProxyConfig {
        ProxyConfig {
            id: "proxy-1".to_string(),
            name: "Gateway".to_string(),
            provider_type,
            protocol: provider_type
                .is_challenge_solver()
                .then_some(ChallengeSolverProtocol::RequestSolutionV1),
            base_url: "socks5://gateway:1080".to_string(),
            request_timeout_seconds: 60,
            is_enabled: true,
            username_encrypted: username.map(str::to_string),
            password_encrypted: password.map(str::to_string),
            remote_dns: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
        }
    }

    #[test]
    fn proxy_payload_reports_stored_credentials_without_exposing_them() {
        let payload = from_proxy_config(proxy_config(
            ProxyProviderType::Socks5,
            Some("operator"),
            Some("hunter2"),
        ));

        assert!(payload.has_credentials);
        assert!(payload.remote_dns);
        assert_eq!(payload.protocol, None);
        // The point of the flag: no field on the payload carries the secret, so
        // rendering every string-bearing field must not surface either value.
        let rendered = format!(
            "{}|{}|{}|{:?}|{:?}|{:?}",
            payload.name,
            payload.provider_type,
            payload.base_url,
            payload.protocol,
            payload.last_health_status,
            payload.last_error_message
        );
        assert!(!rendered.contains("operator"));
        assert!(!rendered.contains("hunter2"));

        // A username alone still counts as "credentials set".
        let username_only = from_proxy_config(proxy_config(
            ProxyProviderType::Http,
            Some("operator"),
            None,
        ));
        assert!(username_only.has_credentials);

        // An empty stored value is not a credential.
        let blank = from_proxy_config(proxy_config(ProxyProviderType::Http, Some(""), Some("")));
        assert!(!blank.has_credentials);
    }

    #[test]
    fn proxy_payload_keeps_solver_rows_unchanged() {
        let payload = from_proxy_config(proxy_config(ProxyProviderType::Trawl, None, None));

        assert_eq!(payload.provider_type, "trawl");
        assert_eq!(payload.protocol.as_deref(), Some("request_solution_v1"));
        assert!(!payload.has_credentials);
    }
}
