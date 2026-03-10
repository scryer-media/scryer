use crate::{AppError, AppResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Configuration for the recycle bin, resolved from application settings.
pub struct RecycleBinConfig {
    pub enabled: bool,
    pub base_path: PathBuf,
    pub retention_days: u32,
}

/// Metadata written alongside each recycled file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecycleManifest {
    pub recycled_at: String,
    pub original_path: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    pub reason: String,
}

/// Result of a successful recycle operation.
#[derive(Debug, Clone)]
pub struct RecycleResult {
    pub recycled_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// Move a file to the recycle bin instead of deleting it.
///
/// If the recycle bin is disabled, deletes the file directly (preserving current behaviour)
/// and returns `Ok(None)`.
///
/// If the file does not exist, returns `Ok(None)` without error (matches the current
/// `ErrorKind::NotFound` handling in callers).
pub async fn recycle_file(
    config: &RecycleBinConfig,
    source_path: &Path,
    manifest: RecycleManifest,
) -> AppResult<Option<RecycleResult>> {
    // If the source doesn't exist, nothing to recycle.
    if !source_path.exists() {
        return Ok(None);
    }

    if !config.enabled {
        // Disabled: delete directly (current behaviour).
        if let Err(err) = tokio::fs::remove_file(source_path).await {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(AppError::Repository(format!(
                    "failed to delete file {}: {}",
                    source_path.display(),
                    err
                )));
            }
        }
        return Ok(None);
    }

    // Build timestamped directory name: YYYYMMDD_HHMMSSmmm_<6-char-id>
    let now = Utc::now();
    let full_id = scryer_domain::Id::new().0;
    let short_id = &full_id[..6];
    let dir_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S%3f"), short_id);
    let recycle_dir = config.base_path.join(&dir_name);

    tokio::fs::create_dir_all(&recycle_dir).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to create recycle directory {}: {}",
            recycle_dir.display(),
            e
        ))
    })?;

    // Write manifest
    let manifest_path = recycle_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| {
        AppError::Repository(format!("failed to serialize recycle manifest: {}", e))
    })?;
    tokio::fs::write(&manifest_path, manifest_json.as_bytes())
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to write recycle manifest {}: {}",
                manifest_path.display(),
                e
            ))
        })?;

    // Move the file into the recycle directory
    let file_name = source_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
    let recycled_path = recycle_dir.join(file_name);

    // Try rename first (instant if same filesystem)
    match tokio::fs::rename(source_path, &recycled_path).await {
        Ok(()) => {}
        Err(rename_err) => {
            // Cross-device: fall back to copy + delete
            warn!(
                error = %rename_err,
                "rename failed (likely cross-device), falling back to copy"
            );
            tokio::fs::copy(source_path, &recycled_path)
                .await
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to copy {} to recycle bin {}: {}",
                        source_path.display(),
                        recycled_path.display(),
                        e
                    ))
                })?;
            tokio::fs::remove_file(source_path).await.map_err(|e| {
                AppError::Repository(format!(
                    "failed to remove source file {} after copy to recycle bin: {}",
                    source_path.display(),
                    e
                ))
            })?;
        }
    }

    info!(
        original = %source_path.display(),
        recycled = %recycled_path.display(),
        reason = %manifest.reason,
        "file moved to recycle bin"
    );

    Ok(Some(RecycleResult {
        recycled_path,
        manifest_path,
    }))
}

/// Restore a file from the recycle bin to its original location.
///
/// Used by the upgrade workflow to roll back on import failure.
pub async fn restore_from_recycle(recycled_path: &Path, original_path: &Path) -> AppResult<()> {
    if !recycled_path.exists() {
        return Err(AppError::Repository(format!(
            "recycled file not found: {}",
            recycled_path.display()
        )));
    }

    // Ensure parent directory exists
    if let Some(parent) = original_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Repository(format!(
                "failed to create parent directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    match tokio::fs::rename(recycled_path, original_path).await {
        Ok(()) => {}
        Err(_) => {
            // Cross-device fallback
            tokio::fs::copy(recycled_path, original_path)
                .await
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to restore {} to {}: {}",
                        recycled_path.display(),
                        original_path.display(),
                        e
                    ))
                })?;
            let _ = tokio::fs::remove_file(recycled_path).await;
        }
    }

    info!(
        restored = %original_path.display(),
        "file restored from recycle bin"
    );

    Ok(())
}

/// Purge recycled entries older than `config.retention_days`.
///
/// Returns the count of purged entries.
pub async fn purge_expired(config: &RecycleBinConfig) -> AppResult<u32> {
    if !config.enabled || !config.base_path.exists() {
        return Ok(0);
    }

    let cutoff = Utc::now() - chrono::Duration::days(config.retention_days as i64);
    let mut purged = 0u32;

    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            // Not a recycle bin entry — skip
            continue;
        }

        let manifest_bytes = match tokio::fs::read(&manifest_path).await {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "failed to read recycle manifest, skipping"
                );
                continue;
            }
        };

        let manifest: RecycleManifest = match serde_json::from_slice(&manifest_bytes) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "failed to parse recycle manifest, skipping"
                );
                continue;
            }
        };

        let recycled_at = match chrono::DateTime::parse_from_rfc3339(&manifest.recycled_at) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => continue,
        };

        if recycled_at < cutoff {
            if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to purge expired recycle entry"
                );
            } else {
                purged += 1;
            }
        }
    }

    if purged > 0 {
        info!(purged, "purged expired recycle bin entries");
    }

    Ok(purged)
}

/// Resolve the media root path for a title's facet.
///
/// Uses the facet registry to look up the configured path setting.
pub async fn media_root_for_title(
    app: &crate::AppUseCase,
    title: &scryer_domain::Title,
) -> Option<String> {
    let handler = app.facet_registry.get(&title.facet);
    let path_key = handler
        .map(|h| h.library_path_key())
        .unwrap_or("series.path");
    let default_path = handler
        .map(|h| h.default_library_path())
        .unwrap_or("/media/series");

    app.read_setting_string_value_for_scope(crate::SETTINGS_SCOPE_MEDIA, path_key, None)
        .await
        .ok()
        .flatten()
        .or_else(|| Some(default_path.to_string()))
}

/// Resolve recycle bin configuration from application settings.
///
/// Reads settings with hardcoded defaults (same pattern as `nfo.write_on_import.*`).
/// When `media_root` is provided and no custom path is configured, defaults to
/// `{media_root}/.scryer-recycle/`.
pub async fn resolve_recycle_config(
    app: &crate::AppUseCase,
    media_root: Option<&str>,
) -> RecycleBinConfig {
    let enabled = app
        .read_setting_string_value_for_scope(
            crate::SETTINGS_SCOPE_MEDIA,
            "recycle_bin.enabled",
            None,
        )
        .await
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(true);

    let custom_path = app
        .read_setting_string_value_for_scope(crate::SETTINGS_SCOPE_MEDIA, "recycle_bin.path", None)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());

    let base_path = if let Some(p) = custom_path {
        PathBuf::from(p)
    } else if let Some(root) = media_root {
        PathBuf::from(root).join(".scryer-recycle")
    } else {
        // Fallback: use a temp-ish location; shouldn't normally happen
        PathBuf::from("/tmp/.scryer-recycle")
    };

    let retention_days = app
        .read_setting_string_value_for_scope(
            crate::SETTINGS_SCOPE_MEDIA,
            "recycle_bin.retention_days",
            None,
        )
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(7);

    RecycleBinConfig {
        enabled,
        base_path,
        retention_days,
    }
}

/// Build a recycle bin config from a file path by walking up to find the media root.
///
/// For use in contexts where `AppUseCase` is not available (e.g., standalone async functions).
/// Defaults: enabled=true, retention_days=7, base_path derived from file's grandparent.
pub fn config_from_file_path(file_path: &Path) -> RecycleBinConfig {
    // Walk up to the grandparent as a rough media root estimate.
    // e.g. /media/movies/Movie (2024)/Movie.mkv → /media/movies/
    let base = file_path
        .parent() // Movie (2024)/
        .and_then(|p| p.parent()) // /media/movies/
        .unwrap_or_else(|| Path::new("/tmp"));

    RecycleBinConfig {
        enabled: true,
        base_path: base.join(".scryer-recycle"),
        retention_days: 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &Path) -> RecycleBinConfig {
        RecycleBinConfig {
            enabled: true,
            base_path: dir.to_path_buf(),
            retention_days: 7,
        }
    }

    fn test_manifest() -> RecycleManifest {
        RecycleManifest {
            recycled_at: Utc::now().to_rfc3339(),
            original_path: "/media/movies/test.mkv".to_string(),
            size_bytes: 1024,
            title_id: Some("title-123".to_string()),
            reason: "title_deleted".to_string(),
        }
    }

    #[tokio::test]
    async fn test_recycle_creates_dir_and_manifest() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.recycled_path.exists());
        assert!(r.manifest_path.exists());
        assert!(!source.exists());

        // Verify manifest is valid JSON
        let bytes = tokio::fs::read(&r.manifest_path).await.unwrap();
        let m: RecycleManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(m.reason, "title_deleted");
    }

    #[tokio::test]
    async fn test_recycle_disabled_deletes_directly() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
        };

        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn test_recycle_nonexistent_file_returns_none() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp.path().join("recycle"));

        let result = recycle_file(&config, &tmp.path().join("nope.mkv"), test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_restore_returns_file() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        let content = b"video data for restore test";
        tokio::fs::write(&source, content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        assert!(!source.exists());

        restore_from_recycle(&result.recycled_path, &source)
            .await
            .unwrap();

        assert!(source.exists());
        let restored = tokio::fs::read(&source).await.unwrap();
        assert_eq!(restored, content);
    }

    #[tokio::test]
    async fn test_purge_removes_expired_only() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();

        // Create an "expired" entry (recycled 30 days ago)
        let old_dir = recycle_dir.join("20260205_120000000_abc123");
        tokio::fs::create_dir_all(&old_dir).await.unwrap();
        let old_manifest = RecycleManifest {
            recycled_at: (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            original_path: "/old.mkv".to_string(),
            size_bytes: 100,
            title_id: None,
            reason: "test".to_string(),
        };
        tokio::fs::write(
            old_dir.join("manifest.json"),
            serde_json::to_string(&old_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(old_dir.join("old.mkv"), b"old")
            .await
            .unwrap();

        // Create a "fresh" entry (recycled just now)
        let new_dir = recycle_dir.join("20260307_120000000_def456");
        tokio::fs::create_dir_all(&new_dir).await.unwrap();
        let new_manifest = RecycleManifest {
            recycled_at: Utc::now().to_rfc3339(),
            original_path: "/new.mkv".to_string(),
            size_bytes: 100,
            title_id: None,
            reason: "test".to_string(),
        };
        tokio::fs::write(
            new_dir.join("manifest.json"),
            serde_json::to_string(&new_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(new_dir.join("new.mkv"), b"new")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 1);
        assert!(!old_dir.exists(), "expired entry should be purged");
        assert!(new_dir.exists(), "fresh entry should survive");
    }
}
