use std::collections::HashMap;

pub use scryer_domain::{ConfigFieldDef, ConfigFieldOption};
pub use scryer_domain::IndexerProviderCapabilities as IndexerCapabilities;
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
    /// If set, the plugin has a fixed public endpoint and doesn't need
    /// user-supplied base_url or api_key. The frontend hides those fields
    /// and uses this URL when auto-creating an IndexerConfig.
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
    /// Semantic category hint from the caller (e.g. "movie", "tv", "anime").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
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
        assert_eq!(desc.config_fields[0].help_text.as_deref(), Some("Custom API endpoint path"));
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
            category: Some("movie".to_string()),
            categories: vec!["2000".to_string()],
            limit: 50,
            season: None,
            episode: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: PluginSearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.query, "Dune");
        assert_eq!(parsed.imdb_id, Some("tt15239678".to_string()));
        assert!(parsed.tvdb_id.is_none());
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
