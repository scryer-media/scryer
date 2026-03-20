use std::collections::HashMap;

pub use scryer_domain::IndexerProviderCapabilities as IndexerCapabilities;
pub use scryer_domain::{ConfigFieldDef, ConfigFieldOption, TaggedAlias};
use serde::{Deserialize, Serialize};

/// Returned by a plugin's `describe()` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub name: String,
    pub version: String,
    pub sdk_version: String,
    pub plugin_type: String,
    pub provider_type: String,
    /// Additional provider type strings this plugin handles. The loader
    /// registers the plugin under each alias so existing configs with e.g.
    /// `provider_type: "nzbgeek"` route to the "newznab" plugin automatically.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    #[serde(default)]
    pub capabilities: IndexerCapabilities,
    /// Optional Rego scoring policies bundled with this plugin.
    /// Each entry is raw Rego source. Policies can reference plugin-specific
    /// data via `input.release.extra.*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scoring_policies: Vec<PluginScoringPolicy>,
    /// Plugin-declared config fields. Each entry describes a configuration key
    /// the plugin expects, with type, label, and validation hints. The frontend
    /// renders dynamic form fields based on this schema, and values are stored
    /// in `IndexerConfig.config_json` and injected via Extism `config::get()`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    /// If set, the plugin has a fixed public endpoint and doesn't need a
    /// user-supplied base_url. The frontend can hide the Base URL field and
    /// use this URL when creating or editing an IndexerConfig. Some providers
    /// may still use the standard api_key field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    /// Additional hostnames the plugin is allowed to reach beyond the
    /// configured base_url (indexer) or config_json URLs (notification).
    /// The loader always grants access to the base_url hostname and any
    /// config_json values that parse as URLs. Use this for extra static
    /// hosts (CDNs, secondary APIs). Use `["*"]` for unrestricted access.
    /// Empty (the default) means the plugin can only reach its configured URLs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    /// Recommended minimum seconds between API requests. Used as the default
    /// `rate_limit_seconds` when auto-creating an IndexerConfig for this plugin.
    /// If unset, the global default (1 second) applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_seconds: Option<i64>,
    /// Notification-specific capabilities. Only present for `plugin_type: "notification"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_capabilities: Option<NotificationCapabilities>,
    /// Download-client-specific accepted input kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_inputs: Vec<String>,
    /// Download-client-specific isolation modes such as category, tag, or directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isolation_modes: Vec<String>,
    /// Download-client-specific capabilities. Only present for
    /// `plugin_type: "download_client"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_client_capabilities: Option<DownloadClientCapabilities>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginScoringPolicy {
    pub name: String,
    pub rego_source: String,
    /// Facets this policy applies to (e.g. "movie", "tv").
    /// Empty means it applies to all facets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_facets: Vec<String>,
}

/// Notification capabilities declared by a notification plugin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationCapabilities {
    #[serde(default)]
    pub supports_rich_text: bool,
    #[serde(default)]
    pub supports_images: bool,
    /// Which event types this plugin can meaningfully handle.
    /// Empty means all events are supported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_events: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadClientCapabilities {
    #[serde(default)]
    pub pause: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub remove_with_data: bool,
    #[serde(default)]
    pub mark_imported: bool,
    #[serde(default)]
    pub prepare_for_import: bool,
    #[serde(default)]
    pub client_status: bool,
    #[serde(default)]
    pub queue_priority: bool,
    #[serde(default)]
    pub seed_limits: bool,
    #[serde(default)]
    pub start_paused: bool,
    #[serde(default)]
    pub force_start: bool,
    #[serde(default)]
    pub per_download_directory: bool,
    #[serde(default)]
    pub host_fs_required: bool,
    #[serde(default)]
    pub test_connection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadClientAddRequest {
    pub source: PluginDownloadSource,
    pub release: PluginDownloadRelease,
    pub title: PluginDownloadTitle,
    pub routing: PluginDownloadRouting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadSource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnet_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_bytes_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_password: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginDownloadRelease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_recent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_pack: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadTitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    pub title_name: String,
    pub media_facet: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginDownloadRouting {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadClientAddResponse {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadItem {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub title: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_move_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_remove: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCompletedDownload {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    pub name: String,
    pub dest_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadClientControlRequest {
    pub action: String,
    pub client_item_id: String,
    #[serde(default)]
    pub remove_data: bool,
    #[serde(default)]
    pub is_history: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDownloadClientMarkImportedRequest {
    pub client_item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginDownloadClientStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_localhost: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_output_roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removes_completed_downloads: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sorting_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Sent to a notification plugin's `send_notification()` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginNotificationRequest {
    pub event_type: String,
    pub title: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_facet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_version: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Returned by a notification plugin's `send_notification()` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginNotificationResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Sent to a plugin's `search()` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imdb_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tvdb_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anidb_id: Option<String>,
    /// Semantic category hint from the caller (e.g. "movie", "tv", "anime").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Maximum results the plugin should return. The plugin owns pagination
    /// internally — this is just an upper bound hint so the plugin can stop
    /// early. The host always sends 1000.
    #[serde(default)]
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tagged_aliases: Vec<TaggedAlias>,
}

/// Returned by a plugin's `search()` export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSearchResponse {
    #[serde(default)]
    pub results: Vec<PluginSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_current: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grab_max: Option<u32>,
}

/// A single search result from a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSearchResult {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grabs: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Arbitrary indexer-specific metadata. The adapter maps well-known keys
    /// (e.g. "thumbs_up", "thumbs_down", "subtitles", "password_protected")
    /// to the corresponding IndexerSearchResult fields.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_round_trip() {
        let json = r#"{
            "name": "Test Plugin",
            "version": "1.0.0",
            "sdk_version": "0.1",
            "plugin_type": "indexer",
            "provider_type": "test",
            "capabilities": { "search": true, "imdb_search": false, "tvdb_search": false }
        }"#;
        let desc: PluginDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(desc.name, "Test Plugin");
        assert_eq!(desc.plugin_type, "indexer");
        assert!(desc.capabilities.search);
        assert!(!desc.capabilities.imdb_search);
        assert!(desc.scoring_policies.is_empty());
        assert!(desc.config_fields.is_empty());
    }

    #[test]
    fn descriptor_with_config_fields() {
        let json = r#"{
            "name": "Custom Plugin",
            "version": "1.0.0",
            "sdk_version": "0.1",
            "plugin_type": "indexer",
            "provider_type": "custom",
            "capabilities": { "search": true },
            "config_fields": [
                {
                    "key": "endpoint_path",
                    "label": "Endpoint Path",
                    "field_type": "string",
                    "required": true,
                    "default_value": "/api",
                    "help_text": "Custom API endpoint path"
                },
                {
                    "key": "auth_mode",
                    "label": "Auth Mode",
                    "field_type": "select",
                    "options": [
                        { "value": "basic", "label": "Basic Auth" },
                        { "value": "token", "label": "Bearer Token" }
                    ]
                }
            ]
        }"#;
        let desc: PluginDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(desc.config_fields.len(), 2);
        assert_eq!(desc.config_fields[0].key, "endpoint_path");
        assert_eq!(desc.config_fields[0].field_type, "string");
        assert!(desc.config_fields[0].required);
        assert_eq!(desc.config_fields[0].default_value.as_deref(), Some("/api"));
        assert_eq!(
            desc.config_fields[0].help_text.as_deref(),
            Some("Custom API endpoint path")
        );
        assert_eq!(desc.config_fields[1].key, "auth_mode");
        assert_eq!(desc.config_fields[1].field_type, "select");
        assert_eq!(desc.config_fields[1].options.len(), 2);
        assert_eq!(desc.config_fields[1].options[0].value, "basic");
        assert_eq!(desc.config_fields[1].options[1].label, "Bearer Token");
        assert!(!desc.config_fields[1].required);
    }

    #[test]
    fn descriptor_with_scoring_policies() {
        let json = r#"{
            "name": "Scored Plugin",
            "version": "1.0.0",
            "sdk_version": "0.1",
            "plugin_type": "indexer",
            "provider_type": "scored",
            "capabilities": { "search": true },
            "scoring_policies": [
                { "name": "vote_boost", "rego_source": "package test\nscore_entry[\"boost\"] := 100" }
            ]
        }"#;
        let desc: PluginDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(desc.scoring_policies.len(), 1);
        assert_eq!(desc.scoring_policies[0].name, "vote_boost");
        assert!(desc.scoring_policies[0].applied_facets.is_empty());
    }

    #[test]
    fn search_request_round_trip() {
        let req = PluginSearchRequest {
            query: "Dune".to_string(),
            imdb_id: Some("tt15239678".to_string()),
            tvdb_id: None,
            anidb_id: None,
            category: Some("movie".to_string()),
            categories: vec!["2000".to_string()],
            limit: 1000,
            season: None,
            episode: None,
            tagged_aliases: vec![TaggedAlias {
                name: "Suna no Wakusei".to_string(),
                language: "jpn".to_string(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: PluginSearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.query, "Dune");
        assert_eq!(parsed.imdb_id, Some("tt15239678".to_string()));
        assert!(parsed.tvdb_id.is_none());
        assert_eq!(parsed.tagged_aliases.len(), 1);
    }

    #[test]
    fn search_response_round_trip() {
        let json = r#"{
            "results": [{
                "title": "Dune 2024 2160p",
                "link": "https://example.com/1",
                "download_url": "https://example.com/1.nzb",
                "size_bytes": 15000000000,
                "published_at": "2024-06-15T00:00:00Z",
                "grabs": 1500,
                "languages": ["English"],
                "extra": { "thumbs_up": 42, "thumbs_down": 3 }
            }]
        }"#;
        let resp: PluginSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let r = &resp.results[0];
        assert_eq!(r.title, "Dune 2024 2160p");
        assert_eq!(r.size_bytes, Some(15000000000));
        assert_eq!(r.languages, vec!["English"]);
        assert_eq!(r.extra.get("thumbs_up").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(r.extra.get("thumbs_down").and_then(|v| v.as_i64()), Some(3));
    }

    #[test]
    fn search_response_defaults_missing_fields() {
        let json = r#"{ "results": [{ "title": "Minimal Result" }] }"#;
        let resp: PluginSearchResponse = serde_json::from_str(json).unwrap();
        let r = &resp.results[0];
        assert_eq!(r.title, "Minimal Result");
        assert!(r.link.is_none());
        assert!(r.download_url.is_none());
        assert!(r.size_bytes.is_none());
        assert!(r.languages.is_empty());
        assert!(r.extra.is_empty());
    }

    #[test]
    fn notification_descriptor_round_trip() {
        let json = r#"{
            "name": "Webhook",
            "version": "1.0.0",
            "sdk_version": "0.1",
            "plugin_type": "notification",
            "provider_type": "webhook",
            "capabilities": { "search": false },
            "notification_capabilities": {
                "supports_rich_text": false,
                "supports_images": false,
                "supported_events": ["grab", "import_complete"]
            },
            "config_fields": [
                { "key": "webhook_url", "label": "Webhook URL", "field_type": "string", "required": true }
            ]
        }"#;
        let desc: PluginDescriptor = serde_json::from_str(json).unwrap();
        assert_eq!(desc.plugin_type, "notification");
        assert_eq!(desc.provider_type, "webhook");
        let caps = desc.notification_capabilities.unwrap();
        assert!(!caps.supports_rich_text);
        assert_eq!(caps.supported_events, vec!["grab", "import_complete"]);
        assert_eq!(desc.config_fields.len(), 1);
    }

    #[test]
    fn notification_request_round_trip() {
        let req = PluginNotificationRequest {
            event_type: "grab".to_string(),
            title: "Download started".to_string(),
            message: "Dune was grabbed".to_string(),
            title_name: Some("Dune".to_string()),
            title_year: Some(2024),
            title_facet: Some("movie".to_string()),
            poster_url: None,
            episode_info: None,
            quality: Some("Bluray-2160p".to_string()),
            release_title: Some("Dune.2024.2160p.BluRay".to_string()),
            download_client: Some("sabnzbd".to_string()),
            file_path: None,
            health_message: None,
            application_version: None,
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: PluginNotificationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "grab");
        assert_eq!(parsed.title_name, Some("Dune".to_string()));
        assert_eq!(parsed.title_year, Some(2024));
    }

    #[test]
    fn notification_response_round_trip() {
        let json = r#"{ "success": true }"#;
        let resp: PluginNotificationResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.error.is_none());

        let json = r#"{ "success": false, "error": "connection refused" }"#;
        let resp: PluginNotificationResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert_eq!(resp.error.as_deref(), Some("connection refused"));
    }
}
