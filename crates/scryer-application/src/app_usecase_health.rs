use super::*;
use std::collections::HashSet;
use tracing::warn;

fn to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

impl AppUseCase {
    /// Run all health checks and return results.
    pub async fn run_health_checks(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();
        results.extend(self.check_download_clients().await);
        results.extend(self.check_indexers().await);
        results.extend(self.check_root_folders().await);
        results.extend(self.check_disk_space_health().await);
        results.extend(self.check_smg_certificate().await);
        results
    }

    async fn check_download_clients(&self) -> Vec<HealthCheckResult> {
        let configs = match self.services.download_client_configs.list(None).await {
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
            .filter(|c| c.status == "error" || c.status == "failed")
            .collect();
        if !errored.is_empty() {
            let names: Vec<&str> = errored.iter().map(|c| c.name.as_str()).collect();
            return vec![HealthCheckResult {
                source: "DownloadClient".into(),
                status: HealthCheckStatus::Warning,
                message: format!("Download client(s) reporting errors: {}", names.join(", ")),
            }];
        }

        vec![]
    }

    async fn check_indexers(&self) -> Vec<HealthCheckResult> {
        let configs = match self.services.indexer_configs.list(None).await {
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

        let stats = self.services.indexer_stats.all_stats();
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
        let path_keys = [
            ("series.path", "/media/series", "Series"),
            ("anime.path", "/media/anime", "Anime"),
            ("movies.path", "/media/movies", "Movies"),
        ];

        let mut results = Vec::new();
        for (key, default, label) in &path_keys {
            let path = self
                .read_setting_string_value_for_scope(SETTINGS_SCOPE_MEDIA, key, None)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| default.to_string());

            let p = std::path::Path::new(&path);
            if !p.exists() {
                results.push(HealthCheckResult {
                    source: "RootFolder".into(),
                    status: HealthCheckStatus::Error,
                    message: format!("{label} root folder does not exist: {path}"),
                });
            } else if p
                .metadata()
                .map(|m| m.permissions().readonly())
                .unwrap_or(true)
            {
                results.push(HealthCheckResult {
                    source: "RootFolder".into(),
                    status: HealthCheckStatus::Warning,
                    message: format!("{label} root folder is read-only: {path}"),
                });
            }
        }

        results
    }

    async fn check_disk_space_health(&self) -> Vec<HealthCheckResult> {
        let path_keys = [
            ("series.path", "/media/series", "Series"),
            ("anime.path", "/media/anime", "Anime"),
            ("movies.path", "/media/movies", "Movies"),
        ];

        let mut seen = HashSet::new();
        let mut results = Vec::new();

        for (key, default, label) in &path_keys {
            let path = self
                .read_setting_string_value_for_scope(SETTINGS_SCOPE_MEDIA, key, None)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| default.to_string());

            if !seen.insert(path.clone()) {
                continue;
            }

            if let Some(stat) = statvfs_path(&path) {
                let free = to_u64(stat.f_bavail) * to_u64(stat.f_frsize);
                let mb_100 = 100 * 1024 * 1024;
                let mb_500 = 500 * 1024 * 1024;

                if free < mb_100 {
                    results.push(HealthCheckResult {
                        source: "DiskSpace".into(),
                        status: HealthCheckStatus::Error,
                        message: format!(
                            "{label} disk space critically low: {} MB free at {path}",
                            free / (1024 * 1024)
                        ),
                    });
                } else if free < mb_500 {
                    results.push(HealthCheckResult {
                        source: "DiskSpace".into(),
                        status: HealthCheckStatus::Warning,
                        message: format!(
                            "{label} disk space low: {} MB free at {path}",
                            free / (1024 * 1024)
                        ),
                    });
                }
            }
        }

        results
    }

    async fn check_smg_certificate(&self) -> Vec<HealthCheckResult> {
        let expires_at = match self.services.system_info.smg_cert_expires_at().await {
            Ok(Some(v)) => v,
            _ => return vec![],
        };

        let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&expires_at) else {
            return vec![];
        };

        let days_remaining = (expires.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days();

        if days_remaining < 0 {
            return vec![HealthCheckResult {
                source: "SmgCertificate".into(),
                status: HealthCheckStatus::Error,
                message: "SMG certificate has expired".into(),
            }];
        }

        if days_remaining < 7 {
            return vec![HealthCheckResult {
                source: "SmgCertificate".into(),
                status: HealthCheckStatus::Error,
                message: format!("SMG certificate expires in {days_remaining} days"),
            }];
        }

        if days_remaining < 30 {
            return vec![HealthCheckResult {
                source: "SmgCertificate".into(),
                status: HealthCheckStatus::Warning,
                message: format!("SMG certificate expires in {days_remaining} days"),
            }];
        }

        vec![]
    }
}
