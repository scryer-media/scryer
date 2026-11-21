//! Metadata-provider plugin family.
//!
//! A metadata provider answers for titles the metadata gateway cannot: a film
//! with an IMDb id SMG has never heard of, such as a fan edit. The host calls
//! it only after SMG came back empty, so a provider whose upstream counts
//! requests is spent on genuine gaps.
//! The host owns everything around the answer: matching, creating the title,
//! storing it and deciding when to ask again.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ConfigFieldDef;

/// Media kinds a metadata provider may answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataMediaKind {
    Movie,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataProviderDescriptor {
    pub provider_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_aliases: Vec<String>,
    /// Shown beside results this provider supplied, such as `OMDb`.
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage: Vec<MetadataMediaKind>,
    #[serde(default)]
    pub capabilities: MetadataProviderCapabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_fields: Vec<ConfigFieldDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MetadataProviderCapabilities {
    /// The plugin answers `search` (title and year). Without it the host only
    /// asks by IMDb id.
    #[serde(default)]
    pub search: bool,
    /// The plugin answers `health` for its installation-wide settings.
    #[serde(default)]
    pub health: bool,
}

/// One title as the provider knows it. Dates are `YYYY-MM-DD`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginTitle {
    /// `tt` followed by digits.
    pub imdb_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_minutes: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    /// The certification, such as `PG-13`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb_votes: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cast: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

/// One title by its IMDb id.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginLookupRequest {
    pub kind: MetadataMediaKind,
    pub imdb_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginLookupResponse {
    /// `None` when the provider has no title under that id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<MetadataPluginTitle>,
}

/// Titles by name, for a provider whose `search` capability is on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginSearchRequest {
    pub kind: MetadataMediaKind,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub limit: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginSearchResponse {
    #[serde(default)]
    pub titles: Vec<MetadataPluginTitle>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginHealthRequest {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MetadataPluginHealthResponse {
    pub healthy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// An IMDb title id as hosts and providers compare it: trimmed, lowercase
/// `tt` and digits, or `None` when `value` is not one.
pub fn normalize_imdb_id(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let digits = value.strip_prefix("tt")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imdb_ids_are_normalized_or_refused() {
        assert_eq!(
            normalize_imdb_id(" TT90000001 ").as_deref(),
            Some("tt90000001")
        );
        assert_eq!(normalize_imdb_id("tt0000042").as_deref(), Some("tt0000042"));
        for bad in ["", "tt", "nm0000001", "tt12a", "18183916"] {
            assert_eq!(normalize_imdb_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn titles_round_trip_without_empty_fields() {
        let title = MetadataPluginTitle {
            imdb_id: "tt90000001".into(),
            title: "Fixture Feature: The Long Edit".into(),
            year: Some(2005),
            ..MetadataPluginTitle::default()
        };
        let json = serde_json::to_string(&title).unwrap();
        assert_eq!(
            json,
            r#"{"imdb_id":"tt90000001","title":"Fixture Feature: The Long Edit","year":2005}"#
        );
        assert_eq!(
            serde_json::from_str::<MetadataPluginTitle>(&json).unwrap(),
            title
        );
    }
}
