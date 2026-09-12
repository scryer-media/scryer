use super::*;
use std::collections::{HashMap, HashSet};
use tracing::warn;

fn health_root_label(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "Movies",
        MediaFacet::Series => "Series",
        MediaFacet::Anime => "Anime",
    }
}

fn health_library_root_label(root: &crate::catalog_workflow::LibraryRootFolder) -> String {
    format!("{} ({})", root.library_name, health_root_label(&root.facet))
}

fn path_overlaps(left: &str, right: &str) -> bool {
    crate::catalog_workflow::library_root_paths_overlap(left, right)
}

fn download_client_status_health_results(
    config_name: &str,
    status: &DownloadClientStatus,
    has_remote_path_mappings: bool,
    library_roots: &[String],
) -> Vec<HealthCheckResult> {
    let unresolved_roots = status
        .remote_output_roots
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .filter(|path| !std::path::Path::new(path).exists())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let overlapping_roots = status
        .remote_output_roots
        .iter()
        .filter_map(|download_root| {
            library_roots
                .iter()
                .find(|library_root| path_overlaps(download_root, library_root))
                .map(|library_root| {
                    format!("{} overlaps {}", download_root.trim(), library_root.trim())
                })
        })
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    if !unresolved_roots.is_empty()
        && (status.is_localhost == Some(false) || has_remote_path_mappings)
    {
        results.push(HealthCheckResult {
            source: "DownloadClient".into(),
            status: HealthCheckStatus::Warning,
            message: format!(
                "Download client '{}' reports output paths that Scryer still cannot access after remote path mapping: {}. Check remote path mappings and container volume mounts.",
                config_name,
                unresolved_roots.join(", ")
            ),
        });
    }

    if !overlapping_roots.is_empty() {
        results.push(HealthCheckResult {
            source: "DownloadClient".into(),
            status: HealthCheckStatus::Warning,
            message: format!(
                "Download client '{}' reports output roots that overlap library roots: {}. Separate download and library folders to avoid blocked completed-download imports.",
                config_name,
                overlapping_roots.join(", ")
            ),
        });
    }

    results
}

fn release_age_unknown_health_results(
    pending_releases: impl IntoIterator<Item = PendingRelease>,
) -> Vec<HealthCheckResult> {
    let mut by_indexer = HashMap::<String, (usize, String)>::new();
    for pending in pending_releases {
        let indexer = pending
            .indexer_id
            .or(pending.indexer_source)
            .unwrap_or_else(|| "unknown indexer".to_string());
        let entry = by_indexer
            .entry(indexer)
            .or_insert((0, pending.last_observed_at.clone()));
        entry.0 += 1;
        if pending.last_observed_at > entry.1 {
            entry.1 = pending.last_observed_at;
        }
    }

    let mut results = by_indexer
        .into_iter()
        .map(|(indexer, (count, last_observed_at))| HealthCheckResult {
            source: "ReleaseAgeUnknown".into(),
            status: HealthCheckStatus::Warning,
            message: format!(
                "Indexer '{indexer}' has {count} pending release(s) with an unknown publication time (last observed {last_observed_at}). They will remain pending until a later feed observation supplies a valid publication time or they require review."
            ),
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.message.cmp(&right.message));
    results
}

/// Every health-check source this module can emit a result for.
///
/// The list is explicit rather than derived from the results because a healthy
/// source produces no result at all: without a fixed roster, "everything is
/// fine" and "the check never ran" would look identical on the scrape. A source
/// added to the checks above without being added here is simply not exported;
/// the unit tests below pin the roster against the checks.
const HEALTH_CHECK_SOURCES: [&str; 6] = [
    "DiskSpace",
    "DownloadClient",
    "Indexer",
    "RecycleBin",
    "ReleaseAgeUnknown",
    "RootFolder",
];

/// Statuses in increasing severity, so "worst wins" is a max over this order.
const HEALTH_CHECK_STATUSES: [HealthCheckStatus; 3] = [
    HealthCheckStatus::Ok,
    HealthCheckStatus::Warning,
    HealthCheckStatus::Error,
];

fn health_status_severity(status: &HealthCheckStatus) -> u8 {
    match status {
        HealthCheckStatus::Ok => 0,
        HealthCheckStatus::Warning => 1,
        HealthCheckStatus::Error => 2,
    }
}

/// The `(source, status, value)` triples describing the current health state.
///
/// Each source contributes one row per status: `1` for the worst status any of
/// its results reported (`Ok` when it reported nothing, which is how a passing
/// check looks) and `0` for the others. Emitting the zeros keeps every series
/// present, so an alert can be written as `== 1` without absent-handling, and
/// a source that recovers visibly drops back to `Ok` instead of leaving a stale
/// `Error` series behind.
///
/// Results are aggregated per source on purpose: the per-client and per-indexer
/// detail lives in the message text, and turning client or indexer names into
/// label values would make the series count grow with the operator's config.
fn health_status_gauge_values(
    results: &[HealthCheckResult],
) -> Vec<(&'static str, &'static str, f64)> {
    HEALTH_CHECK_SOURCES
        .iter()
        .flat_map(|source| {
            // A source that reported nothing is healthy, so start at `Ok` and
            // let the worst result it did report win.
            let mut worst = health_status_severity(&HealthCheckStatus::Ok);
            for result in results.iter().filter(|result| result.source == *source) {
                worst = worst.max(health_status_severity(&result.status));
            }
            HEALTH_CHECK_STATUSES.into_iter().map(move |status| {
                let value = if health_status_severity(&status) == worst {
                    1.0
                } else {
                    0.0
                };
                (*source, status.as_str(), value)
            })
        })
        .collect()
}

fn record_health_check_gauges(results: &[HealthCheckResult]) {
    for (source, status, value) in health_status_gauge_values(results) {
        metrics::gauge!(
            crate::metrics_support::HEALTH_CHECK_STATUS,
            "source" => source,
            "status" => status
        )
        .set(value);
    }
}

impl AppUseCase {
    /// Run all health checks and return results.
    pub async fn run_health_checks(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        results.extend(self.check_download_clients().await);
        results.extend(self.check_indexers().await);
        results.extend(self.check_root_folders().await);
        results.extend(self.check_recycle_bin_config().await);
        results.extend(self.check_disk_space_health().await);
        results.extend(self.check_release_age_unknown_pending().await);
        record_health_check_gauges(&results);
        results
    }

    async fn check_release_age_unknown_pending(&self) -> Vec<HealthCheckResult> {
        match self
            .services
            .workflow
            .pending_releases
            .list_active_release_age_unknown_pending_releases()
            .await
        {
            Ok(pending) => release_age_unknown_health_results(pending),
            Err(error) => {
                warn!(error = %error, "health check: failed to list unknown-age pending releases");
                Vec::new()
            }
        }
    }

    async fn health_library_roots(
        &self,
    ) -> AppResult<Vec<crate::catalog_workflow::LibraryRootFolder>> {
        let mut roots = Vec::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            roots.extend(self.all_library_root_folders_for_facet(&facet).await?);
        }
        Ok(roots)
    }

    async fn check_download_clients(&self) -> Vec<HealthCheckResult> {
        let configs = match self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "health check: failed to list download clients");
                return vec![HealthCheckResult {
                    source: "DownloadClient".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to query download clients: {e}"),
                }];
            }
        };

        if configs.is_empty() {
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Error,
                message: "No download client is configured".into(),
            }];
        }

        let enabled: Vec<_> = configs.iter().filter(|c| c.is_enabled).collect();
        if enabled.is_empty() {
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: "All download clients are disabled".into(),
            }];
        }

        let errored: Vec<_> = enabled
            .iter()
            .filter(|c| {
                c.status == scryer_domain::DownloadClientStatus::Error
                    || c.status == scryer_domain::DownloadClientStatus::Failed
            })
            .collect();
        if !errored.is_empty() {
            let names: Vec<&str> = errored.iter().map(|c| c.name.as_str()).collect();
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: format!("Download client(s) reporting errors: {}", names.join(", ")),
            }];
        }

        let mut results = Vec::new();
        let library_roots = match self.health_library_roots().await {
            Ok(roots) => roots.into_iter().map(|root| root.path).collect::<Vec<_>>(),
            Err(error) => {
                warn!(
                    error = %error,
                    "health check: failed to resolve library roots while checking download clients"
                );
                results.push(HealthCheckResult {
                    source: "DownloadClient".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to resolve library roots: {error}"),
                });
                Vec::new()
            }
        };
        for config in enabled {
            let has_remote_path_mappings =
                match crate::has_download_client_remote_path_mappings(&config.config_json) {
                    Ok(value) => value,
                    Err(error) => {
                        results.push(HealthCheckResult {
                            source: "DownloadClient".into(),
                            status: HealthCheckStatus::Warning,
                            message: format!(
                                "Download client '{}' has invalid remote path mappings: {error}",
                                config.name
                            ),
                        });
                        continue;
                    }
                };

            let status = match self
                .services
                .integrations
                .download_client
                .get_client_status_for_client_id(&config.id)
                .await
            {
                Ok(status) => status,
                Err(_) => continue,
            };

            results.extend(download_client_status_health_results(
                &config.name,
                &status,
                has_remote_path_mappings,
                &library_roots,
            ));
        }

        results
    }

    async fn check_indexers(&self) -> Vec<HealthCheckResult> {
        let configs = match self.services.integrations.indexer_configs.list(None).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "health check: failed to list indexers");
                return vec![HealthCheckResult {
                    source: "Indexer".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to query indexers: {e}"),
                }];
            }
        };

        if configs.is_empty() {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Warning,
                message: "No indexer is configured".into(),
            }];
        }

        let enabled: Vec<_> = configs.iter().filter(|c| c.is_enabled).collect();
        if enabled.is_empty() {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Warning,
                message: "All indexers are disabled".into(),
            }];
        }

        let stats = self.services.integrations.indexer_stats.all_stats();
        let all_failing = !stats.is_empty()
            && stats
                .iter()
                .all(|s| s.failed_last_24h > 0 && s.successful_last_24h == 0);
        if all_failing {
            return vec![HealthCheckResult {
                source: "Indexer".into(),
                status: HealthCheckStatus::Error,
                message: "All indexers are failing".into(),
            }];
        }

        vec![]
    }

    async fn check_root_folders(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        let root_folders = match self.health_library_roots().await {
            Ok(root_folders) => root_folders,
            Err(error) => {
                return vec![HealthCheckResult {
                    source: "RootFolder".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to resolve library roots: {error}"),
                }];
            }
        };
        let mut seen = HashSet::new();

        for root in root_folders {
            if !seen.insert(root.normalized_path.clone()) {
                continue;
            }
            let label = health_library_root_label(&root);
            let p = std::path::Path::new(&root.path);
            if !p.exists() {
                results.push(HealthCheckResult {
                    source: "RootFolder".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("{label} root folder does not exist: {}", root.path),
                });
            } else if p
                .metadata()
                .map(|m| m.permissions().readonly())
                .unwrap_or(true)
            {
                results.push(HealthCheckResult {
                    source: "RootFolder".into(),
                    status: HealthCheckStatus::Warning,
                    message: format!("{label} root folder is read-only: {}", root.path),
                });
            }
        }

        results
    }

    async fn check_recycle_bin_config(&self) -> Vec<HealthCheckResult> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();
        let root_folders = match self.health_library_roots().await {
            Ok(root_folders) => root_folders,
            Err(error) => {
                return vec![HealthCheckResult {
                    source: "RecycleBin".into(),
                    status: HealthCheckStatus::Error,
                    message: format!(
                        "Failed to resolve library roots while validating recycle bin config: {error}"
                    ),
                }];
            }
        };

        for root in root_folders {
            if !seen.insert(root.normalized_path.clone()) {
                continue;
            }
            let label = health_library_root_label(&root);

            let config = self
                .recycle_bin_config_for_media_root(Some(&root.path))
                .await;
            if config.enabled && !config.cleanup_enabled {
                results.push(HealthCheckResult {
                    source: "RecycleBin".into(),
                    status: HealthCheckStatus::Error,
                    message: format!(
                        "Recycle bin cleanup is disabled for {} root {}: {}",
                        label,
                        root.path,
                        config
                            .validation_error
                            .as_deref()
                            .unwrap_or("invalid recycle bin configuration")
                    ),
                });
            }
        }

        results
    }

    async fn check_disk_space_health(&self) -> Vec<HealthCheckResult> {
        let root_folders = match self.health_library_roots().await {
            Ok(root_folders) => root_folders,
            Err(error) => {
                return vec![HealthCheckResult {
                    source: "DiskSpace".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("Failed to resolve library roots: {error}"),
                }];
            }
        };

        {
            let mut seen = HashSet::new();
            let mut results = Vec::new();

            for root in root_folders {
                if !seen.insert(root.normalized_path.clone()) {
                    continue;
                }
                let label = health_library_root_label(&root);

                if let Some(space) = filesystem_space(&root.path) {
                    // `root` is the configured root-folder path: a handful of
                    // operator-chosen values, and the only identity under which
                    // a capacity figure means anything. Media paths, titles and
                    // ids never become labels.
                    metrics::gauge!(
                        crate::metrics_support::ROOT_FOLDER_FREE_BYTES,
                        "root" => root.path.clone()
                    )
                    .set(space.available_bytes as f64);
                    metrics::gauge!(
                        crate::metrics_support::ROOT_FOLDER_TOTAL_BYTES,
                        "root" => root.path.clone()
                    )
                    .set(space.total_bytes as f64);

                    let free = space.available_bytes;
                    let mb_100 = 100 * 1024 * 1024;
                    let mb_500 = 500 * 1024 * 1024;

                    if free < mb_100 {
                        results.push(HealthCheckResult {
                            source: "DiskSpace".into(),
                            status: HealthCheckStatus::Error,
                            message: format!(
                                "{} disk space critically low: {} MB free at {}",
                                label,
                                free / (1024 * 1024),
                                root.path
                            ),
                        });
                    } else if free < mb_500 {
                        results.push(HealthCheckResult {
                            source: "DiskSpace".into(),
                            status: HealthCheckStatus::Warning,
                            message: format!(
                                "{} disk space low: {} MB free at {}",
                                label,
                                free / (1024 * 1024),
                                root.path
                            ),
                        });
                    }
                }
            }

            results
        }
    }
}

#[cfg(test)]
mod tests {
    use metrics::with_local_recorder;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;

    fn test_library(
        id: &str,
        name: &str,
        facet: MediaFacet,
        paths: &[&str],
    ) -> scryer_domain::Library {
        let now = chrono::Utc::now();
        scryer_domain::Library {
            id: id.to_string(),
            facet,
            name: name.to_string(),
            slug: id.to_string(),
            is_default: false,
            roots: paths
                .iter()
                .enumerate()
                .map(|(index, path)| scryer_domain::LibraryRoot {
                    id: format!("{id}-root-{index}"),
                    library_id: id.to_string(),
                    path: (*path).to_string(),
                    is_default: index == 0,
                    created_at: now,
                    updated_at: now,
                })
                .collect(),
            created_at: now,
            updated_at: now,
        }
    }

    fn unknown_age_pending(
        indexer_id: Option<&str>,
        source: Option<&str>,
        added_at: &str,
        last_observed_at: &str,
    ) -> PendingRelease {
        PendingRelease {
            id: format!("pending-{added_at}"),
            wanted_item_id: "wanted-1".to_string(),
            title_id: "title-1".to_string(),
            release_title: "Example.Release.1080p".to_string(),
            release_url: None,
            source_kind: None,
            release_size_bytes: None,
            release_score: 0,
            scoring_log_json: None,
            indexer_source: source.map(str::to_string),
            indexer_id: indexer_id.map(str::to_string),
            release_guid: None,
            added_at: added_at.to_string(),
            last_observed_at: last_observed_at.to_string(),
            delay_until: added_at.to_string(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
            seed_minimums: crate::ReleaseSeedMinimums::default(),
            seeders: None,
            release_identity: format!("release-{added_at}"),
            coverage_identity: "scope:wanted-1".to_string(),
            role: PendingReleaseRole::Primary,
            last_decision_code: Some("release_age_unknown".to_string()),
            release_age_unknown: true,
        }
    }

    #[test]
    fn unknown_release_age_health_groups_by_indexer_and_keeps_last_observation() {
        let results = release_age_unknown_health_results(vec![
            unknown_age_pending(
                Some("indexer-a"),
                Some("A"),
                "2026-08-01T00:00:00Z",
                "2026-08-04T00:00:00Z",
            ),
            unknown_age_pending(
                Some("indexer-a"),
                Some("A"),
                "2026-08-02T00:00:00Z",
                "2026-08-05T00:00:00Z",
            ),
            unknown_age_pending(
                None,
                Some("Indexer B"),
                "2026-08-03T00:00:00Z",
                "2026-08-03T12:00:00Z",
            ),
        ]);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|result| {
            result.source == "ReleaseAgeUnknown"
                && result.message.contains("indexer-a")
                && result.message.contains("2 pending release(s)")
                && result.message.contains("2026-08-05T00:00:00Z")
        }));
        assert!(
            results
                .iter()
                .any(|result| result.message.contains("Indexer B"))
        );
        assert!(release_age_unknown_health_results(Vec::new()).is_empty());
    }

    #[test]
    fn health_library_roots_include_multiple_libraries_per_facet() {
        let libraries = [
            test_library("movies", "Movies", MediaFacet::Movie, &["/media/movies"]),
            test_library(
                "movies-4k",
                "Movies 4K",
                MediaFacet::Movie,
                &["/media/movies-4k"],
            ),
            test_library("series", "Series", MediaFacet::Series, &["/media/series"]),
        ];
        let roots = crate::catalog_workflow::library_root_folders_from_libraries(&libraries, None);

        let paths = roots
            .iter()
            .map(|root| root.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec!["/media/movies", "/media/movies-4k", "/media/series"]
        );
        assert!(
            roots
                .iter()
                .any(|root| health_library_root_label(root) == "Movies 4K (Movies)")
        );

        let movie_roots = crate::catalog_workflow::library_root_folders_from_libraries(
            &libraries,
            Some(&MediaFacet::Movie),
        );
        let movie_paths = movie_roots
            .iter()
            .map(|root| root.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(movie_paths, vec!["/media/movies", "/media/movies-4k"]);
    }

    #[test]
    fn download_client_health_warns_for_inaccessible_mapped_roots() {
        let missing_root = std::env::temp_dir().join(format!(
            "scryer-health-missing-{}",
            scryer_domain::Id::new().0
        ));
        let status = DownloadClientStatus {
            is_localhost: Some(true),
            remote_output_roots: vec![missing_root.display().to_string()],
            ..DownloadClientStatus::default()
        };

        let results = download_client_status_health_results("Decypharr SAB", &status, true, &[]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "DownloadClient");
        assert_eq!(results[0].status, HealthCheckStatus::Warning);
        assert!(results[0].message.contains("Decypharr SAB"));
        assert!(
            results[0]
                .message
                .contains("still cannot access after remote path mapping")
        );
        assert!(
            results[0]
                .message
                .contains(missing_root.display().to_string().as_str())
        );
    }

    #[test]
    fn download_client_health_warns_for_overlapping_library_roots() {
        let status = DownloadClientStatus {
            is_localhost: Some(true),
            remote_output_roots: vec!["/srv/downloads/complete/series".to_string()],
            ..DownloadClientStatus::default()
        };

        let results = download_client_status_health_results(
            "Decypharr qBittorrent",
            &status,
            false,
            &["/srv/downloads/complete/series".to_string()],
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, "DownloadClient");
        assert_eq!(results[0].status, HealthCheckStatus::Warning);
        assert!(results[0].message.contains("Decypharr qBittorrent"));
        assert!(results[0].message.contains("overlap library roots"));
        assert!(
            results[0]
                .message
                .contains("/srv/downloads/complete/series overlaps /srv/downloads/complete/series")
        );
    }

    fn health_result(source: &str, status: HealthCheckStatus) -> HealthCheckResult {
        HealthCheckResult {
            source: source.to_string(),
            status,
            message: String::new(),
        }
    }

    fn gauge_value(
        values: &[(&'static str, &'static str, f64)],
        source: &str,
        status: &str,
    ) -> Option<f64> {
        values
            .iter()
            .find(|(series_source, series_status, _)| {
                *series_source == source && *series_status == status
            })
            .map(|(_, _, value)| *value)
    }

    #[test]
    fn health_gauges_report_every_source_and_status_with_exactly_one_hot() {
        let values = health_status_gauge_values(&[]);

        assert_eq!(
            values.len(),
            HEALTH_CHECK_SOURCES.len() * HEALTH_CHECK_STATUSES.len()
        );
        for source in HEALTH_CHECK_SOURCES {
            let hot = HEALTH_CHECK_STATUSES
                .iter()
                .filter(|status| gauge_value(&values, source, status.as_str()) == Some(1.0))
                .count();
            assert_eq!(hot, 1, "{source} should have exactly one status set to 1");
            assert_eq!(
                gauge_value(&values, source, "ok"),
                Some(1.0),
                "{source} with no results is healthy"
            );
        }
    }

    #[test]
    fn health_gauges_take_the_worst_status_per_source() {
        let values = health_status_gauge_values(&[
            health_result("Indexer", HealthCheckStatus::Warning),
            health_result("Indexer", HealthCheckStatus::Error),
            health_result("Indexer", HealthCheckStatus::Warning),
            health_result("RootFolder", HealthCheckStatus::Warning),
        ]);

        assert_eq!(gauge_value(&values, "Indexer", "error"), Some(1.0));
        assert_eq!(gauge_value(&values, "Indexer", "warning"), Some(0.0));
        assert_eq!(gauge_value(&values, "Indexer", "ok"), Some(0.0));

        assert_eq!(gauge_value(&values, "RootFolder", "warning"), Some(1.0));
        assert_eq!(gauge_value(&values, "RootFolder", "error"), Some(0.0));

        // Untouched sources stay reported, and stay healthy.
        assert_eq!(gauge_value(&values, "DiskSpace", "ok"), Some(1.0));
    }

    #[test]
    fn health_gauges_emit_the_recorded_series() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_health_check_gauges(&[health_result("DiskSpace", HealthCheckStatus::Error)]);
        });

        let recorded = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _unit, _description, value)| match value {
                DebugValue::Gauge(gauge) => {
                    let key = key.key();
                    let labels = key
                        .labels()
                        .map(|label| (label.key().to_string(), label.value().to_string()))
                        .collect::<Vec<_>>();
                    Some((key.name().to_string(), labels, gauge.into_inner()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            recorded.len(),
            HEALTH_CHECK_SOURCES.len() * HEALTH_CHECK_STATUSES.len()
        );
        assert!(
            recorded
                .iter()
                .all(|(name, _, _)| name == crate::metrics_support::HEALTH_CHECK_STATUS)
        );
        let disk_space_error = recorded
            .iter()
            .find(|(_, labels, _)| {
                labels.contains(&("source".to_string(), "DiskSpace".to_string()))
                    && labels.contains(&("status".to_string(), "error".to_string()))
            })
            .map(|(_, _, value)| *value);
        assert_eq!(disk_space_error, Some(1.0));
    }
}
