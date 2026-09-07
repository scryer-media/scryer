use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::{AppError, AppResult};
use scryer_outbound_http::{
    HostRpsProfile, OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    external_arr_reqwest_client, validate_operator_http_url,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;

pub const EXTERNAL_IMPORT_HOST_RPS_LANE: &str = "external_import";
pub const EXTERNAL_IMPORT_HOST_RPS_PROFILE: HostRpsProfile = HostRpsProfile::limited(200.0, 200);

/// Root folder discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrRootFolder {
    pub id: i64,
    pub path: String,
}

/// Download client discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrDownloadClient {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

/// Indexer discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrIndexer {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArrNamingConfig {
    pub rename_enabled: Option<bool>,
    pub replace_illegal_characters: Option<bool>,
    pub colon_replacement_format: Option<String>,
    pub standard_format: Option<String>,
    pub folder_format: Option<String>,
    pub season_folder_format: Option<String>,
    pub specials_folder_format: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArrMediaManagementConfig {
    pub set_permissions_linux: Option<bool>,
    pub chmod_folder: Option<String>,
    pub chown_group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrMetadataProvider {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub enable: bool,
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrQualityProfile {
    pub id: i64,
    pub name: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProwlarrIndexer {
    pub base_url: String,
    pub api_key: Option<String>,
    pub child_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrMovie {
    pub id: i64,
    pub root_folder_path: String,
    pub path: Option<String>,
    pub file_path: Option<String>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub monitored: bool,
    #[serde(default)]
    pub quality_profile_id: Option<i64>,
    #[serde(default)]
    pub minimum_availability: Option<String>,
    #[serde(default)]
    pub original_language: Option<String>,
    #[serde(default)]
    pub tags: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrSeriesSeason {
    pub season_number: i32,
    pub monitored: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArrSeriesStatistics {
    pub total_episode_count: Option<i32>,
    pub monitored_episode_count: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrSeries {
    pub id: i64,
    pub root_folder_path: String,
    pub path: Option<String>,
    pub tvdb_id: Option<String>,
    pub monitored: bool,
    #[serde(default)]
    pub quality_profile_id: Option<i64>,
    #[serde(default)]
    pub series_type: Option<String>,
    #[serde(default)]
    pub season_folder: Option<bool>,
    #[serde(default)]
    pub monitor_new_items: Option<String>,
    #[serde(default)]
    pub original_language: Option<String>,
    #[serde(default)]
    pub tags: Vec<i64>,
    pub seasons: Vec<ArrSeriesSeason>,
    pub statistics: ArrSeriesStatistics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrEpisode {
    pub id: i64,
    pub series_id: i64,
    pub tvdb_id: Option<String>,
    pub season_number: i32,
    pub episode_number: i32,
    pub file_path: Option<String>,
    pub monitored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalArrApiBucket {
    SonarrV4,
    RadarrV6,
}

impl ExternalArrApiBucket {
    const fn expected_app_name(self) -> &'static str {
        match self {
            Self::SonarrV4 => "Sonarr",
            Self::RadarrV6 => "Radarr",
        }
    }

    const fn minimum_supported_major(self) -> u64 {
        match self {
            Self::SonarrV4 => 4,
            Self::RadarrV6 => 6,
        }
    }

    const fn supported_api_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::SonarrV4 => &["v3", "v4", "v5"],
            Self::RadarrV6 => &["v3"],
        }
    }

    const fn request_namespace(self) -> &'static str {
        match self {
            Self::SonarrV4 => "sonarr_v4",
            Self::RadarrV6 => "radarr_v6",
        }
    }

    fn naming_config_path(self, api_prefix: &str) -> &'static str {
        match self {
            Self::SonarrV4 if api_prefix.eq_ignore_ascii_case("v5") => "settings/naming",
            Self::SonarrV4 | Self::RadarrV6 => "config/naming",
        }
    }

    fn media_management_config_path(self, api_prefix: &str) -> &'static str {
        match self {
            Self::SonarrV4 if api_prefix.eq_ignore_ascii_case("v5") => "settings/mediamanagement",
            Self::SonarrV4 | Self::RadarrV6 => "config/mediamanagement",
        }
    }

    fn validate_status(self, app_name: &str, version: &str) -> AppResult<()> {
        let app_name = app_name.trim();
        if !app_name.eq_ignore_ascii_case(self.expected_app_name()) {
            let message = if app_name.is_empty() {
                format!(
                    "base_url responded but did not identify itself as {}",
                    self.expected_app_name()
                )
            } else {
                format!(
                    "base_url responded as '{}', not {}",
                    app_name,
                    self.expected_app_name()
                )
            };
            return Err(AppError::Validation(message));
        }

        let version = version.trim();
        let Some(version_major) = external_arr_version_major(version) else {
            return Err(AppError::Validation(format!(
                "could not determine {} version from '{}'",
                self.expected_app_name(),
                version
            )));
        };

        if version_major < self.minimum_supported_major() {
            return Err(AppError::Validation(format!(
                "unsupported {} version '{}'; expected major {} or newer",
                self.expected_app_name(),
                version,
                self.minimum_supported_major()
            )));
        }

        Ok(())
    }

    fn validate_api_prefix(self, api_prefix: &str) -> AppResult<()> {
        let api_prefix = api_prefix.trim().trim_matches('/').to_ascii_lowercase();
        if api_prefix.is_empty() {
            return Err(AppError::Validation(format!(
                "{} did not report an API version",
                self.expected_app_name()
            )));
        }

        if self
            .supported_api_prefixes()
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(&api_prefix))
        {
            return Ok(());
        }

        Err(AppError::Validation(format!(
            "unsupported {} API version '{}'; expected one of {}",
            self.expected_app_name(),
            api_prefix,
            self.supported_api_prefixes().join(", ")
        )))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ExternalArrApiInfo {
    current: String,
}

#[derive(Debug, Clone)]
struct ExternalArrSystemStatus {
    app_name: String,
    version: String,
}

fn external_arr_version_major(version: &str) -> Option<u64> {
    version.trim().split('.').next()?.parse().ok()
}

#[derive(Debug, Clone)]
struct ExternalArrBaseUrl {
    configured: String,
    effective: String,
    localhost_ipv4_hint: Option<String>,
}

fn normalize_external_arr_base_url(
    raw: &str,
    label: &'static str,
) -> AppResult<ExternalArrBaseUrl> {
    let trimmed = raw.trim();
    let url = validate_operator_http_url(trimmed, label)
        .map_err(|error| AppError::Validation(error.to_string()))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::Validation(format!(
            "{label} must not include a query string or fragment"
        )));
    }

    let configured = trimmed.trim_end_matches('/').to_string();
    let mut effective_url = url.clone();
    let is_http_localhost = url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("localhost"));

    if is_http_localhost {
        effective_url
            .set_host(Some("127.0.0.1"))
            .map_err(|_| AppError::Validation(format!("{label} has an invalid host")))?;
    }

    let effective = effective_url.as_str().trim_end_matches('/').to_string();
    let localhost_ipv4_hint = is_http_localhost.then_some(effective.clone());

    Ok(ExternalArrBaseUrl {
        configured,
        effective,
        localhost_ipv4_hint,
    })
}

/// HTTP client for Sonarr/Radarr APIs pinned to supported product-major buckets.
#[derive(Clone)]
pub struct ExternalArrClient {
    configured_base_url: String,
    effective_base_url: String,
    localhost_ipv4_hint: Option<String>,
    api_key: String,
    outbound_http: OutboundHttpClient,
    api_bucket: ExternalArrApiBucket,
    api_prefix: Arc<RwLock<Option<String>>>,
    system_status: Arc<RwLock<Option<ExternalArrSystemStatus>>>,
}

impl ExternalArrClient {
    pub fn for_sonarr_v4(base_url: String, api_key: String) -> AppResult<Self> {
        Self::new(
            base_url,
            api_key,
            ExternalArrApiBucket::SonarrV4,
            "Sonarr base URL",
        )
    }

    pub fn for_radarr_v6(base_url: String, api_key: String) -> AppResult<Self> {
        Self::new(
            base_url,
            api_key,
            ExternalArrApiBucket::RadarrV6,
            "Radarr base URL",
        )
    }

    fn new(
        base_url: String,
        api_key: String,
        api_bucket: ExternalArrApiBucket,
        label: &'static str,
    ) -> AppResult<Self> {
        let base_url = normalize_external_arr_base_url(&base_url, label)?;
        let http_client = external_arr_reqwest_client();
        Ok(Self {
            configured_base_url: base_url.configured,
            effective_base_url: base_url.effective,
            localhost_ipv4_hint: base_url.localhost_ipv4_hint,
            api_key,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            api_bucket,
            api_prefix: Arc::new(RwLock::new(None)),
            system_status: Arc::new(RwLock::new(None)),
        })
    }

    /// Test connectivity and return (app_name, version).
    pub async fn test_connection(&self) -> AppResult<(String, String)> {
        let status = self.ensure_supported_system_status().await?;
        Ok((status.app_name, status.version))
    }

    /// Fetch root folders (media library paths).
    pub async fn list_root_folders(&self) -> AppResult<Vec<ArrRootFolder>> {
        let json = self.api_get("rootfolder").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("rootfolder response was not an array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_i64()?;
                let path = item.get("path")?.as_str()?.to_string();
                if path.is_empty() {
                    return None;
                }
                Some(ArrRootFolder { id, path })
            })
            .collect())
    }

    /// Fetch download client configurations.
    ///
    /// Fetches the list first, then re-fetches each client individually because
    /// Sonarr v4+ / Radarr v5+ mask sensitive field values (e.g. `apiKey`) in the
    /// list endpoint response.
    pub async fn list_download_clients(&self) -> AppResult<Vec<ArrDownloadClient>> {
        let json = self.api_get("downloadclient").await?;
        let arr = json.as_array().ok_or_else(|| {
            AppError::Repository("downloadclient response was not an array".into())
        })?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            // Re-fetch individually to get unmasked sensitive fields.
            let fields = match self.api_get(&format!("downloadclient/{id}")).await {
                Ok(detail) => detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
                Err(_) => item
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
            };

            results.push(ArrDownloadClient {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    /// Fetch indexer configurations.
    ///
    /// Like `list_download_clients`, re-fetches each indexer individually to get
    /// unmasked sensitive fields (e.g. `apiKey`).
    pub async fn list_indexers(&self) -> AppResult<Vec<ArrIndexer>> {
        let json = self.api_get("indexer").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("indexer response was not an array".into()))?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            let fields = item
                .get("fields")
                .and_then(Value::as_array)
                .map(|fields| flatten_arr_fields(fields))
                .unwrap_or_default();

            results.push(ArrIndexer {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    pub async fn get_naming_config(&self) -> AppResult<ArrNamingConfig> {
        self.ensure_supported_system_status().await?;
        let api_prefix = self.ensure_supported_api_prefix().await?;
        let json = self
            .api_get_with_prefix(&api_prefix, self.api_bucket.naming_config_path(&api_prefix))
            .await?;
        Ok(ArrNamingConfig {
            rename_enabled: value_bool(json.get(match self.api_bucket {
                ExternalArrApiBucket::SonarrV4 => "renameEpisodes",
                ExternalArrApiBucket::RadarrV6 => "renameMovies",
            })),
            replace_illegal_characters: value_bool(json.get("replaceIllegalCharacters")),
            colon_replacement_format: value_trimmed_string(json.get("colonReplacementFormat")),
            standard_format: value_trimmed_string(json.get(match self.api_bucket {
                ExternalArrApiBucket::SonarrV4 => "standardEpisodeFormat",
                ExternalArrApiBucket::RadarrV6 => "standardMovieFormat",
            })),
            folder_format: value_trimmed_string(json.get(match self.api_bucket {
                ExternalArrApiBucket::SonarrV4 => "seriesFolderFormat",
                ExternalArrApiBucket::RadarrV6 => "movieFolderFormat",
            })),
            season_folder_format: value_trimmed_string(json.get("seasonFolderFormat")),
            specials_folder_format: value_trimmed_string(json.get("specialsFolderFormat")),
        })
    }

    pub async fn get_media_management_config(&self) -> AppResult<ArrMediaManagementConfig> {
        self.ensure_supported_system_status().await?;
        let api_prefix = self.ensure_supported_api_prefix().await?;
        let json = self
            .api_get_with_prefix(
                &api_prefix,
                self.api_bucket.media_management_config_path(&api_prefix),
            )
            .await?;
        Ok(ArrMediaManagementConfig {
            set_permissions_linux: value_bool(json.get("setPermissionsLinux")),
            chmod_folder: value_trimmed_string(json.get("chmodFolder")),
            chown_group: value_trimmed_string(json.get("chownGroup")),
        })
    }

    pub async fn list_metadata_providers(&self) -> AppResult<Vec<ArrMetadataProvider>> {
        let json = self.api_get("metadata").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("metadata response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                Some(ArrMetadataProvider {
                    id: item.get("id").and_then(Value::as_i64)?,
                    name: value_trimmed_string(item.get("name"))?,
                    implementation: value_trimmed_string(item.get("implementation"))?,
                    enable: value_bool(item.get("enable")).unwrap_or(false),
                    fields: item
                        .get("fields")
                        .and_then(Value::as_array)
                        .map(|fields| flatten_arr_fields(fields))
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn list_quality_profiles(&self) -> AppResult<Vec<ArrQualityProfile>> {
        let json = self.api_get("qualityprofile").await?;
        let arr = json.as_array().ok_or_else(|| {
            AppError::Repository("qualityprofile response was not an array".into())
        })?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                Some(ArrQualityProfile {
                    id: item.get("id").and_then(Value::as_i64)?,
                    name: value_trimmed_string(item.get("name"))?,
                    language: value_language_name(item.get("language")),
                })
            })
            .collect())
    }

    pub async fn list_movies(&self) -> AppResult<Vec<ArrMovie>> {
        let json = self.api_get("movie").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("movie response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let root_folder_path = item
                    .get("rootFolderPath")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                if root_folder_path.is_empty() {
                    return None;
                }

                let path = value_trimmed_string(item.get("path"));
                Some(ArrMovie {
                    id,
                    root_folder_path,
                    file_path: arr_nested_file_path(item.get("movieFile"), path.as_deref()),
                    path,
                    tmdb_id: value_str_or_number(item.get("tmdbId")),
                    imdb_id: item
                        .get("imdbId")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    quality_profile_id: item.get("qualityProfileId").and_then(Value::as_i64),
                    minimum_availability: value_trimmed_string(item.get("minimumAvailability")),
                    original_language: value_language_name(item.get("originalLanguage")),
                    tags: value_i64_vec(item.get("tags")),
                })
            })
            .collect())
    }

    pub async fn list_series(&self) -> AppResult<Vec<ArrSeries>> {
        let json = self.api_get("series").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("series response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let root_folder_path = item
                    .get("rootFolderPath")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                if root_folder_path.is_empty() {
                    return None;
                }

                let seasons = item
                    .get("seasons")
                    .and_then(Value::as_array)
                    .map(|seasons| {
                        seasons
                            .iter()
                            .filter_map(|season| {
                                let season_number = season
                                    .get("seasonNumber")
                                    .and_then(Value::as_i64)?
                                    .try_into()
                                    .ok()?;
                                Some(ArrSeriesSeason {
                                    season_number,
                                    monitored: season
                                        .get("monitored")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                Some(ArrSeries {
                    id,
                    root_folder_path,
                    path: item
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    tvdb_id: value_str_or_number(item.get("tvdbId")),
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    quality_profile_id: item.get("qualityProfileId").and_then(Value::as_i64),
                    series_type: value_trimmed_string(item.get("seriesType")),
                    season_folder: value_bool(item.get("seasonFolder")),
                    monitor_new_items: value_trimmed_string(item.get("monitorNewItems")),
                    original_language: value_language_name(item.get("originalLanguage")),
                    tags: value_i64_vec(item.get("tags")),
                    seasons,
                    statistics: item
                        .get("statistics")
                        .and_then(Value::as_object)
                        .map(|statistics| ArrSeriesStatistics {
                            total_episode_count: value_i32(statistics.get("totalEpisodeCount"))
                                .or_else(|| value_i32(statistics.get("episodeCount"))),
                            monitored_episode_count: value_i32(
                                statistics.get("monitoredEpisodeCount"),
                            ),
                        })
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn list_episodes_for_series(
        &self,
        series_id: i64,
        series_path: Option<&str>,
    ) -> AppResult<Vec<ArrEpisode>> {
        self.ensure_supported_system_status().await?;
        let api_prefix = self.ensure_supported_api_prefix().await?;
        let path = sonarr_episode_list_path(series_id, &api_prefix);
        let json = self.api_get_with_prefix(&api_prefix, &path).await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("episode response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let season_number = item
                    .get("seasonNumber")
                    .and_then(Value::as_i64)?
                    .try_into()
                    .ok()?;
                let episode_number = item
                    .get("episodeNumber")
                    .and_then(Value::as_i64)?
                    .try_into()
                    .ok()?;

                Some(ArrEpisode {
                    id,
                    series_id,
                    tvdb_id: value_str_or_number(item.get("tvdbId")),
                    season_number,
                    episode_number,
                    file_path: arr_nested_file_path(item.get("episodeFile"), series_path),
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect())
    }

    async fn api_get(&self, path: &str) -> AppResult<Value> {
        self.ensure_supported_system_status().await?;
        let api_prefix = self.ensure_supported_api_prefix().await?;
        self.api_get_with_prefix(&api_prefix, path).await
    }

    async fn ensure_supported_system_status(&self) -> AppResult<ExternalArrSystemStatus> {
        if let Some(status) = self.system_status.read().await.clone() {
            return Ok(status);
        }

        let mut guard = self.system_status.write().await;
        if let Some(status) = guard.clone() {
            return Ok(status);
        }

        let api_prefix = self.ensure_supported_api_prefix().await?;
        let json = self
            .api_get_with_prefix(&api_prefix, "system/status")
            .await?;
        let status = ExternalArrSystemStatus {
            app_name: json
                .get("appName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            version: json
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        self.api_bucket
            .validate_status(&status.app_name, &status.version)?;
        *guard = Some(status.clone());
        Ok(status)
    }

    async fn ensure_supported_api_prefix(&self) -> AppResult<String> {
        if let Some(api_prefix) = self.api_prefix.read().await.clone() {
            return Ok(api_prefix);
        }

        let mut guard = self.api_prefix.write().await;
        if let Some(api_prefix) = guard.clone() {
            return Ok(api_prefix);
        }

        let api_info: ExternalArrApiInfo = self.api_get_unversioned("api").await?;
        let api_prefix = api_info
            .current
            .trim()
            .trim_matches('/')
            .split('/')
            .next_back()
            .unwrap_or_default()
            .to_ascii_lowercase();
        self.api_bucket.validate_api_prefix(&api_prefix)?;
        *guard = Some(api_prefix.clone());
        Ok(api_prefix)
    }

    async fn api_get_with_prefix(&self, api_prefix: &str, path: &str) -> AppResult<Value> {
        let url = format!("{}/api/{}/{}", self.effective_base_url, api_prefix, path);
        let display_url = format!("{}/api/{}/{}", self.configured_base_url, api_prefix, path);
        self.request_json(&url, &display_url, path).await
    }

    async fn api_get_unversioned<T>(&self, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let path = path.trim_start_matches('/');
        let url = format!("{}/{}", self.effective_base_url, path);
        let display_url = format!("{}/{}", self.configured_base_url, path);
        self.request_json(&url, &display_url, path).await
    }

    async fn request_json<T>(&self, url: &str, display_url: &str, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .outbound_http
            .send(self.request_policy(path), || {
                self.outbound_http
                    .client()
                    .get(url)
                    .header("X-Api-Key", &self.api_key)
            })
            .await
            .map_err(|error| match error {
                OutboundHttpError::DispatchRejected => {
                    AppError::canceled("external API dispatch rejected")
                }
                OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => format!(
                            "external api call to {display_url} was rate limited; retry after {}s",
                            delay.as_secs()
                        ),
                        None => format!("external api call to {display_url} was rate limited"),
                    },
                ),
                OutboundHttpError::Transport { source, .. } => {
                    AppError::Repository(self.transport_error_message(display_url, url, &source))
                }
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!(
                "external api response from {display_url} read failed: {err}"
            ))
        })?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Repository("invalid API key".into()));
        }
        if !status.is_success() {
            let preview = body.chars().take(400).collect::<String>();
            return Err(AppError::Repository(format!(
                "external api call to {display_url} returned status {status}: {preview}"
            )));
        }

        serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!(
                "external api call to {display_url} returned non-json: {err}"
            ))
        })
    }

    fn transport_error_message(
        &self,
        display_url: &str,
        effective_url: &str,
        source: &reqwest::Error,
    ) -> String {
        let mut message = format!("external api call to {display_url}");
        if display_url != effective_url {
            message.push_str(&format!(" via {effective_url}"));
        }
        message.push_str(&format!(" failed: {source}"));
        if let Some(hint) = &self.localhost_ipv4_hint {
            message.push_str(&format!(
                "; try {hint} if Sonarr/Radarr only listen on IPv4"
            ));
        }
        message
    }

    fn request_policy(&self, path: &str) -> RequestPolicy {
        RequestPolicy::safe_read(
            format!(
                "external_arr:{}:{}",
                self.api_bucket.request_namespace(),
                self.configured_base_url
            ),
            format!(
                "external_arr:{}:{path}",
                self.api_bucket.request_namespace()
            ),
        )
        .with_max_retries(2)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
        .with_host_rps_limit(
            EXTERNAL_IMPORT_HOST_RPS_LANE,
            EXTERNAL_IMPORT_HOST_RPS_PROFILE,
        )
        .without_redirects()
    }
}

/// Map the Sonarr/Radarr implementation name to a Scryer download client type.
pub fn map_download_client_type(implementation: &str) -> Option<&'static str> {
    match implementation.trim().to_ascii_lowercase().as_str() {
        "nzbget" => Some("nzbget"),
        "sabnzbd" => Some("sabnzbd"),
        "qbittorrent" => Some("qbittorrent"),
        _ => None,
    }
}

/// Map the Sonarr/Radarr indexer implementation to a Scryer provider type.
///
/// This intentionally maps Arr's *nab families to Scryer's generic *nab
/// plugins only. Provider-specific plugin matching is deferred until catalog
/// metadata can drive it.
pub fn map_indexer_provider_type(
    implementation: &str,
    _fields: &HashMap<String, Value>,
) -> Option<&'static str> {
    match implementation.trim().to_ascii_lowercase().as_str() {
        "newznab" => Some("newznab"),
        "torznab" => Some("torznab"),
        _ => None,
    }
}

pub fn detect_prowlarr_proxy_indexer(indexer: &ArrIndexer) -> Option<DetectedProwlarrIndexer> {
    let implementation = indexer.implementation.trim().to_ascii_lowercase();
    if implementation != "newznab" && implementation != "torznab" {
        return None;
    }

    let api_path = field_str(&indexer.fields, "apiPath")?;
    if api_path.trim().trim_end_matches('/') != "/api" {
        return None;
    }

    let base_url = prowlarr_parent_base_url(&field_str(&indexer.fields, "baseUrl")?)?;
    Some(DetectedProwlarrIndexer {
        base_url,
        api_key: field_str_sensitive(&indexer.fields, "apiKey"),
        child_name: indexer.name.clone(),
    })
}

pub fn detect_linked_prowlarr_proxy_indexer(
    indexer: &ArrIndexer,
    linked_prowlarr_base_url: &str,
) -> Option<DetectedProwlarrIndexer> {
    let implementation = indexer.implementation.trim().to_ascii_lowercase();
    if implementation != "newznab" && implementation != "torznab" {
        return None;
    }

    let base_url = prowlarr_parent_base_url(&field_str(&indexer.fields, "baseUrl")?)?;
    if !same_base_url(&base_url, linked_prowlarr_base_url) {
        return None;
    }

    Some(DetectedProwlarrIndexer {
        base_url,
        api_key: field_str_sensitive(&indexer.fields, "apiKey"),
        child_name: indexer.name.clone(),
    })
}

pub fn should_skip_imported_indexer(indexer: &ArrIndexer) -> bool {
    let implementation = indexer.implementation.trim().to_ascii_lowercase();
    if implementation != "newznab" && implementation != "torznab" {
        return false;
    }

    let Some(base_url) = field_str(&indexer.fields, "baseUrl") else {
        return false;
    };

    let normalized = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    // Dead-service tombstone: legacy Arr installs may still contain these
    // indexers, but Scryer should not import or auto-install anything for them.
    normalized.contains("animetosho.org") || normalized.contains("feed.animetosho.org")
}

fn prowlarr_parent_base_url(base_url: &str) -> Option<String> {
    let mut url = url::Url::parse(base_url.trim()).ok()?;
    url.set_query(None);
    url.set_fragment(None);

    let path = url.path().trim_end_matches('/').to_string();
    let indexer_id = path.rsplit('/').next()?;
    if indexer_id.is_empty() || !indexer_id.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let parent_len = path.len().saturating_sub(indexer_id.len());
    let parent_path = path[..parent_len].trim_end_matches('/');
    url.set_path(parent_path);

    Some(url.as_str().trim_end_matches('/').to_string())
}

fn same_base_url(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

/// Supported Sonarr v4+ / Radarr v6+ builds replace sensitive field values with this placeholder
/// in all API responses (both list and individual GET endpoints).
const ARR_MASKED_VALUE: &str = "********";

/// Extract a string value from the flattened fields map.
pub fn field_str(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })
}

/// Like `field_str`, but returns `None` for values masked by Sonarr/Radarr
/// (`"********"`). Use this for sensitive fields such as `apiKey` and
/// `password` so that callers can detect when a real value was not returned.
pub fn field_str_sensitive(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() && s != ARR_MASKED_VALUE => Some(s.clone()),
        _ => None,
    })
}

/// Extract a string from the fields map, falling back to empty string for numeric values.
pub fn field_str_or_number(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Extract a boolean from the fields map.
pub fn field_bool(fields: &HashMap<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn value_str_or_number(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn value_bool(value: Option<&Value>) -> Option<bool> {
    value.and_then(|value| match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        Value::Number(number) => number.as_i64().and_then(|value| match value {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }),
        _ => None,
    })
}

fn value_i64_vec(value: Option<&Value>) -> Vec<i64> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| match value {
                    Value::Number(number) => number.as_i64(),
                    Value::String(value) => value.trim().parse::<i64>().ok(),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn value_trimmed_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_language_name(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Object(object) => value_trimmed_string(object.get("name"))
            .or_else(|| value_trimmed_string(object.get("nameLower")))
            .or_else(|| value_trimmed_string(object.get("id")))
            .or_else(|| value_str_or_number(object.get("id"))),
        _ => None,
    }
}

fn arr_nested_file_path(value: Option<&Value>, parent_path: Option<&str>) -> Option<String> {
    let object = value.and_then(Value::as_object)?;
    value_trimmed_string(object.get("path")).or_else(|| {
        let relative = value_trimmed_string(object.get("relativePath"))?;
        Some(join_arr_relative_path(parent_path?, &relative))
    })
}

fn join_arr_relative_path(parent_path: &str, relative_path: &str) -> String {
    format!(
        "{}/{}",
        parent_path.trim().trim_end_matches(['/', '\\']),
        relative_path.trim().trim_start_matches(['/', '\\'])
    )
}

fn sonarr_episode_list_path(series_id: i64, api_prefix: &str) -> String {
    if api_prefix.trim().eq_ignore_ascii_case("v5") {
        format!("episode?seriesId={series_id}&includeSubresources=EpisodeFile")
    } else {
        format!("episode?seriesId={series_id}&includeEpisodeFile=true")
    }
}

fn value_i32(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(Value::as_i64)
        .and_then(|value| value.try_into().ok())
}

fn flatten_arr_fields(fields: &[Value]) -> HashMap<String, Value> {
    fields
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?.to_string();
            let value = f.get("value")?.clone();
            Some((name, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::TcpListener as StdTcpListener;
    use std::sync::Arc;

    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    use super::{
        ArrIndexer, EXTERNAL_IMPORT_HOST_RPS_LANE, EXTERNAL_IMPORT_HOST_RPS_PROFILE,
        ExternalArrApiBucket, ExternalArrClient, detect_linked_prowlarr_proxy_indexer,
        detect_prowlarr_proxy_indexer, field_str_sensitive, map_download_client_type,
        map_indexer_provider_type, should_skip_imported_indexer, sonarr_episode_list_path,
    };

    #[test]
    fn external_arr_requests_use_importer_host_quota() {
        let client = ExternalArrClient::for_sonarr_v4(
            "http://127.0.0.1:8989".to_string(),
            "api-key".to_string(),
        )
        .unwrap();
        let request_override = client
            .request_policy("series")
            .host_rps_override
            .expect("external Arr requests should select an importer quota");

        assert_eq!(
            request_override.lane.as_ref(),
            EXTERNAL_IMPORT_HOST_RPS_LANE
        );
        assert_eq!(request_override.profile, EXTERNAL_IMPORT_HOST_RPS_PROFILE);
    }

    fn json_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn redirect_response(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        )
    }

    async fn spawn_ipv4_arr_mock(responses: Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ipv4 arr mock");
        let port = listener.local_addr().expect("mock address").port();
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    return;
                };
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer).await;
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        format!("http://localhost:{port}")
    }

    async fn spawn_indexer_list_mock(indexers: Value) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind indexer list mock");
        let port = listener.local_addr().expect("mock address").port();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let paths_for_task = paths.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    return;
                };
                let mut buffer = [0_u8; 4096];
                let Ok(read) = stream.read(&mut buffer).await else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                paths_for_task.lock().await.push(path.clone());
                let response = if path.starts_with("/api/v3/system/status") {
                    json_response(r#"{"appName":"Sonarr","version":"4.0.0"}"#)
                } else if path == "/api" || path.starts_with("/api?") {
                    json_response(r#"{"current":"v3"}"#)
                } else if path.starts_with("/api/v3/indexer") {
                    json_response(&indexers.to_string())
                } else {
                    "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (format!("http://localhost:{port}"), paths)
    }

    fn unused_ipv4_port() -> u16 {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind unused ipv4 port");
        listener.local_addr().expect("unused ipv4 address").port()
    }

    #[test]
    fn external_arr_operator_urls_accept_local_and_lan_targets() {
        for raw in [
            "http://localhost:8989",
            "http://127.0.0.1:8989",
            "http://192.168.1.20:8989",
        ] {
            ExternalArrClient::for_sonarr_v4(raw.into(), "fixture-key".into())
                .unwrap_or_else(|error| panic!("{raw} should be accepted: {error}"));
        }
    }

    #[test]
    fn external_arr_operator_urls_reject_unsupported_schemes_missing_hosts_and_credentials() {
        for raw in [
            "file:///fixture/sonarr",
            "http://",
            "http://fixture-user:fixture-pass@localhost:8989",
        ] {
            assert!(
                ExternalArrClient::for_sonarr_v4(raw.into(), "fixture-key".into()).is_err(),
                "{raw} should be rejected"
            );
        }
    }

    #[test]
    fn external_arr_http_localhost_uses_ipv4_effective_base_url() {
        let client =
            ExternalArrClient::for_sonarr_v4("http://localhost:8989".into(), "fixture-key".into())
                .expect("valid localhost URL");

        assert_eq!(client.configured_base_url, "http://localhost:8989");
        assert_eq!(client.effective_base_url, "http://127.0.0.1:8989");
    }

    #[test]
    fn external_arr_http_localhost_preserves_path_prefix() {
        let client = ExternalArrClient::for_sonarr_v4(
            "http://localhost:8989/sonarr".into(),
            "fixture-key".into(),
        )
        .expect("valid localhost URL");

        assert_eq!(client.configured_base_url, "http://localhost:8989/sonarr");
        assert_eq!(client.effective_base_url, "http://127.0.0.1:8989/sonarr");
    }

    #[test]
    fn external_arr_https_localhost_is_not_rewritten() {
        let client =
            ExternalArrClient::for_sonarr_v4("https://localhost:8989".into(), "fixture-key".into())
                .expect("valid https localhost URL");

        assert_eq!(client.effective_base_url, "https://localhost:8989");
    }

    #[test]
    fn external_arr_ipv6_loopback_is_not_rewritten() {
        let client =
            ExternalArrClient::for_sonarr_v4("http://[::1]:8989".into(), "fixture-key".into())
                .expect("valid ipv6 loopback URL");

        assert_eq!(client.effective_base_url, "http://[::1]:8989");
    }

    #[tokio::test]
    async fn external_arr_http_localhost_reaches_ipv4_bound_mock() {
        let base_url = spawn_ipv4_arr_mock(vec![
            json_response(r#"{"current":"v3"}"#),
            json_response(r#"{"appName":"Sonarr","version":"4.0.17.2952"}"#),
        ])
        .await;
        let client = ExternalArrClient::for_sonarr_v4(base_url, "fixture-key".into())
            .expect("valid localhost URL");

        let (app_name, version) = client.test_connection().await.expect("mock responds");

        assert_eq!(app_name, "Sonarr");
        assert_eq!(version, "4.0.17.2952");
    }

    #[tokio::test]
    async fn list_indexers_uses_one_list_request_and_no_detail_requests() {
        let mut fixtures = vec![serde_json::json!({
            "id": 1,
            "name": "Prowlarr Proxy",
            "implementation": "Torznab",
            "fields": [
                { "name": "baseUrl", "value": "https://media.example.test/prowlarr/12345/" },
                { "name": "apiPath", "value": "/api" },
                { "name": "apiKey", "value": "********" }
            ]
        })];
        fixtures.extend((2..=41).map(|id| {
            serde_json::json!({
                "id": id,
                "name": format!("Indexer {id}"),
                "implementation": "Newznab",
                "fields": [
                    { "name": "baseUrl", "value": format!("https://indexer-{id}.example.test") },
                    { "name": "apiKey", "value": "********" }
                ]
            })
        }));
        let (base_url, request_paths) = spawn_indexer_list_mock(Value::Array(fixtures)).await;
        let client = ExternalArrClient::for_sonarr_v4(base_url, "fixture-key".into())
            .expect("valid localhost URL");

        let indexers = client.list_indexers().await.expect("indexer list");

        assert_eq!(indexers.len(), 41);
        assert!(field_str_sensitive(&indexers[0].fields, "apiKey").is_none());
        let detected = detect_prowlarr_proxy_indexer(&indexers[0])
            .expect("masked proxy indexer should still be detected");
        assert_eq!(detected.base_url, "https://media.example.test/prowlarr");
        assert!(detected.api_key.is_none());
        let paths = request_paths.lock().await;
        let list_requests = paths
            .iter()
            .filter(|path| {
                path.as_str() == "/api/v3/indexer" || path.starts_with("/api/v3/indexer?")
            })
            .count();
        assert_eq!(list_requests, 1);
        assert!(paths.iter().all(|path| !path.contains("/indexer/")));
    }

    #[tokio::test]
    async fn external_arr_cross_host_redirect_is_rejected() {
        let base_url =
            spawn_ipv4_arr_mock(vec![redirect_response("http://example.invalid/api")]).await;
        let client = ExternalArrClient::for_sonarr_v4(base_url.clone(), "fixture-key".into())
            .expect("valid localhost URL");

        let error = client
            .test_connection()
            .await
            .expect_err("redirect should not be followed")
            .to_string();

        assert!(
            error.contains(&format!(
                "external api call to {base_url}/api returned status 302"
            )),
            "{error}"
        );
    }

    #[tokio::test]
    async fn external_arr_localhost_transport_error_includes_effective_url_and_hint() {
        let port = unused_ipv4_port();
        let base_url = format!("http://localhost:{port}");
        let client = ExternalArrClient::for_sonarr_v4(base_url.clone(), "fixture-key".into())
            .expect("valid localhost URL");

        let error = client
            .test_connection()
            .await
            .expect_err("closed port should fail")
            .to_string();

        assert!(
            error.contains(&format!(
                "external api call to {base_url}/api via http://127.0.0.1:{port}/api failed"
            )),
            "{error}"
        );
        assert!(
            error.contains(&format!(
                "try http://127.0.0.1:{port} if Sonarr/Radarr only listen on IPv4"
            )),
            "{error}"
        );
    }

    #[test]
    fn sonarr_v4_bucket_accepts_sonarr_v4_status() {
        ExternalArrApiBucket::SonarrV4
            .validate_status("Sonarr", "4.0.17.2952")
            .expect("supported version");
    }

    #[test]
    fn sonarr_v4_bucket_accepts_supported_api_prefixes() {
        ExternalArrApiBucket::SonarrV4
            .validate_api_prefix("v3")
            .expect("legacy Sonarr v4 api prefix should be supported");
        ExternalArrApiBucket::SonarrV4
            .validate_api_prefix("v5")
            .expect("newer Sonarr api prefix should be supported");
    }

    #[test]
    fn sonarr_v4_bucket_uses_legacy_settings_routes_before_v5() {
        assert_eq!(
            ExternalArrApiBucket::SonarrV4.naming_config_path("v4"),
            "config/naming"
        );
        assert_eq!(
            ExternalArrApiBucket::SonarrV4.media_management_config_path("v4"),
            "config/mediamanagement"
        );
    }

    #[test]
    fn sonarr_v4_bucket_uses_v5_settings_routes_for_v5_prefix() {
        assert_eq!(
            ExternalArrApiBucket::SonarrV4.naming_config_path("v5"),
            "settings/naming"
        );
        assert_eq!(
            ExternalArrApiBucket::SonarrV4.media_management_config_path("v5"),
            "settings/mediamanagement"
        );
    }

    #[test]
    fn radarr_v6_bucket_uses_v3_config_settings_routes() {
        assert_eq!(
            ExternalArrApiBucket::RadarrV6.naming_config_path("v3"),
            "config/naming"
        );
        assert_eq!(
            ExternalArrApiBucket::RadarrV6.media_management_config_path("v3"),
            "config/mediamanagement"
        );
    }

    #[test]
    fn sonarr_episode_list_path_uses_legacy_include_flag_before_v5() {
        assert_eq!(
            sonarr_episode_list_path(42, "v4"),
            "episode?seriesId=42&includeEpisodeFile=true"
        );
    }

    #[test]
    fn sonarr_episode_list_path_uses_v5_subresource_parameter() {
        assert_eq!(
            sonarr_episode_list_path(42, "v5"),
            "episode?seriesId=42&includeSubresources=EpisodeFile"
        );
    }

    #[test]
    fn radarr_v6_bucket_accepts_matching_major_version() {
        ExternalArrApiBucket::RadarrV6
            .validate_status("Radarr", "6.0.1.0")
            .expect("supported version");
    }

    #[test]
    fn map_download_client_type_recognizes_qbittorrent_variants() {
        assert_eq!(map_download_client_type("qBittorrent"), Some("qbittorrent"));
        assert_eq!(map_download_client_type("QBittorrent"), Some("qbittorrent"));
        assert_eq!(map_download_client_type("qbittorrent"), Some("qbittorrent"));
    }

    #[test]
    fn map_indexer_provider_type_maps_newznab_presets_to_generic_provider() {
        assert_eq!(
            map_indexer_provider_type(
                "Newznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://api.nzbgeek.info".into()),
                )]),
            ),
            Some("newznab")
        );
        assert_eq!(
            map_indexer_provider_type(
                "Newznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://api.dognzb.cr".into()),
                )]),
            ),
            Some("newznab")
        );
    }

    #[test]
    fn map_indexer_provider_type_maps_torznab_to_generic_provider() {
        assert_eq!(
            map_indexer_provider_type(
                "Torznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://torznab.example.com".into()),
                )]),
            ),
            Some("torznab")
        );
    }

    #[test]
    fn map_indexer_provider_type_keeps_unknown_implementation_unsupported() {
        assert_eq!(
            map_indexer_provider_type("TorrentRss", &HashMap::new()),
            None
        );
    }

    #[test]
    fn should_skip_imported_indexer_skips_animetosho() {
        assert!(should_skip_imported_indexer(&ArrIndexer {
            id: 1,
            name: "AnimeTosho".into(),
            implementation: "Torznab".into(),
            fields: HashMap::from([(
                "baseUrl".into(),
                Value::String("https://feed.animetosho.org".into()),
            )]),
        }));
        assert!(should_skip_imported_indexer(&ArrIndexer {
            id: 2,
            name: "AnimeTosho".into(),
            implementation: "Newznab".into(),
            fields: HashMap::from([(
                "baseUrl".into(),
                Value::String("https://animetosho.org".into()),
            )]),
        }));
    }

    #[test]
    fn should_skip_imported_indexer_keeps_other_indexers() {
        assert!(!should_skip_imported_indexer(&ArrIndexer {
            id: 1,
            name: "NZBGeek".into(),
            implementation: "Newznab".into(),
            fields: HashMap::from([(
                "baseUrl".into(),
                Value::String("https://api.nzbgeek.info".into()),
            )]),
        }));
    }

    #[test]
    fn detects_prowlarr_proxy_indexer_and_preserves_reverse_proxy_path() {
        let detected = detect_prowlarr_proxy_indexer(&ArrIndexer {
            id: 1,
            name: "NZBGeek".into(),
            implementation: "Newznab".into(),
            fields: HashMap::from([
                (
                    "baseUrl".into(),
                    Value::String("https://media.example.test/prowlarr/12345/".into()),
                ),
                ("apiPath".into(), Value::String("/api".into())),
                ("apiKey".into(), Value::String("secret".into())),
            ]),
        })
        .expect("prowlarr proxy indexer");

        assert_eq!(detected.base_url, "https://media.example.test/prowlarr");
        assert_eq!(detected.api_key.as_deref(), Some("secret"));
        assert_eq!(detected.child_name, "NZBGeek");
    }

    #[test]
    fn detects_prowlarr_proxy_indexer_with_unbounded_numeric_id() {
        let detected = detect_prowlarr_proxy_indexer(&ArrIndexer {
            id: 1,
            name: "Large ID".into(),
            implementation: "Torznab".into(),
            fields: HashMap::from([
                (
                    "baseUrl".into(),
                    Value::String("http://prowlarr.local/123456".into()),
                ),
                ("apiPath".into(), Value::String("/api/".into())),
            ]),
        })
        .expect("prowlarr proxy indexer");

        assert_eq!(detected.base_url, "http://prowlarr.local");
        assert_eq!(detected.api_key, None);
    }

    #[test]
    fn detects_linked_prowlarr_proxy_indexer_without_api_path() {
        let detected = detect_linked_prowlarr_proxy_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Child".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([
                    (
                        "baseUrl".into(),
                        Value::String("https://media.example.test/prowlarr/98765".into()),
                    ),
                    ("apiKey".into(), Value::String("secret".into())),
                ]),
            },
            "https://media.example.test/prowlarr",
        )
        .expect("linked prowlarr proxy indexer");

        assert_eq!(detected.base_url, "https://media.example.test/prowlarr");
        assert_eq!(detected.api_key.as_deref(), Some("secret"));
        assert_eq!(detected.child_name, "Torrent Child");
    }

    #[test]
    fn ignores_non_prowlarr_shaped_arr_indexers() {
        assert!(
            detect_prowlarr_proxy_indexer(&ArrIndexer {
                id: 1,
                name: "Direct Newznab".into(),
                implementation: "Newznab".into(),
                fields: HashMap::from([
                    (
                        "baseUrl".into(),
                        Value::String("https://indexer.example/api".into()),
                    ),
                    ("apiPath".into(), Value::String("/api".into())),
                ]),
            })
            .is_none()
        );
    }
}
