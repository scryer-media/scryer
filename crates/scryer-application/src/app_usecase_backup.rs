use super::*;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolves the backup directory from the configured database path.
fn backup_dir_from_db_path(db_path: &str) -> PathBuf {
    // db_path is like "sqlite:///data/scryer.db" or "/data/scryer.db"
    let raw = db_path
        .strip_prefix("sqlite://")
        .unwrap_or(db_path);
    let db_file = Path::new(raw);
    db_file
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups")
}

/// Helper: list backup files sorted newest-first.
fn list_backup_files(backup_dir: &Path) -> Vec<(String, u64, String)> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(backup_dir) else {
        return entries;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("db") {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !filename.starts_with("scryer_backup_") {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                Some(dt.to_rfc3339())
            })
            .unwrap_or_default();
        entries.push((filename.to_string(), meta.len(), mtime));
    }
    entries.sort_by(|a, b| b.2.cmp(&a.2)); // newest first
    entries
}

pub trait BackupService {
    fn backup_dir(&self) -> PathBuf;
}

impl BackupService for AppUseCase {
    fn backup_dir(&self) -> PathBuf {
        backup_dir_from_db_path(&self.services.db_path)
    }
}

impl AppUseCase {
    pub async fn create_backup(&self, actor: &User) -> AppResult<BackupInfo> {
        require(actor, &Entitlement::ManageConfig)?;

        let dir = self.backup_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Repository(format!("failed to create backup directory: {e}")))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("scryer_backup_{timestamp}.db");
        let dest = dir.join(&filename);
        let dest_str = dest
            .to_str()
            .ok_or_else(|| AppError::Repository("backup path is not valid UTF-8".into()))?
            .to_string();

        self.services.system_info.vacuum_into(&dest_str).await?;

        let meta = std::fs::metadata(&dest)
            .map_err(|e| AppError::Repository(format!("failed to stat backup file: {e}")))?;
        let size_bytes = meta.len();
        let created_at = chrono::Utc::now().to_rfc3339();

        info!(filename = %filename, size_bytes, "database backup created");

        self.services
            .record_event(
                Some(actor.id.clone()),
                None,
                EventType::ActionTriggered,
                format!("database backup created: {filename}"),
            )
            .await?;

        Ok(BackupInfo {
            filename,
            size_bytes,
            created_at,
        })
    }

    pub async fn list_backups(&self, actor: &User) -> AppResult<Vec<BackupInfo>> {
        require(actor, &Entitlement::ManageConfig)?;

        let dir = self.backup_dir();
        let entries = list_backup_files(&dir);
        Ok(entries
            .into_iter()
            .map(|(filename, size_bytes, created_at)| BackupInfo {
                filename,
                size_bytes,
                created_at,
            })
            .collect())
    }

    pub async fn delete_backup(&self, actor: &User, filename: &str) -> AppResult<bool> {
        require(actor, &Entitlement::ManageConfig)?;

        // Sanitize filename — must match expected pattern
        if !filename.starts_with("scryer_backup_") || !filename.ends_with(".db") || filename.contains('/') || filename.contains('\\') {
            return Err(AppError::Validation("invalid backup filename".into()));
        }

        let path = self.backup_dir().join(filename);
        if !path.exists() {
            return Ok(false);
        }

        std::fs::remove_file(&path)
            .map_err(|e| AppError::Repository(format!("failed to delete backup: {e}")))?;

        info!(filename, "backup deleted");
        Ok(true)
    }

    /// Enforce backup retention: delete oldest backups exceeding the retention count.
    pub async fn enforce_backup_retention(&self, retention_count: usize) -> AppResult<u32> {
        let dir = self.backup_dir();
        let entries = list_backup_files(&dir);
        let mut deleted = 0u32;
        if entries.len() > retention_count {
            for entry in &entries[retention_count..] {
                let path = dir.join(&entry.0);
                if let Err(e) = std::fs::remove_file(&path) {
                    warn!(filename = %entry.0, error = %e, "failed to remove old backup");
                } else {
                    deleted += 1;
                }
            }
        }
        if deleted > 0 {
            info!(deleted, "old backups pruned by retention policy");
        }
        Ok(deleted)
    }

    /// Auto-backup if enough time has passed since the last backup.
    pub async fn auto_backup_if_due(&self) -> AppResult<()> {
        let interval_hours: u64 = self
            .read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, "backup.interval_hours", None)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);

        if interval_hours == 0 {
            return Ok(()); // disabled
        }

        let retention_count: usize = self
            .read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, "backup.retention_count", None)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);

        let dir = self.backup_dir();
        let entries = list_backup_files(&dir);

        // Check if a recent enough backup exists
        let needs_backup = if let Some(newest) = entries.first() {
            if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(&newest.2) {
                let elapsed = chrono::Utc::now() - last_time.with_timezone(&chrono::Utc);
                elapsed > chrono::Duration::hours(interval_hours as i64)
            } else {
                true
            }
        } else {
            true // no backups exist
        };

        if needs_backup {
            // Use a system actor for auto-backup
            let actor = self.find_or_create_default_user().await?;
            self.create_backup(&actor).await?;
            self.enforce_backup_retention(retention_count).await?;
        }

        Ok(())
    }
}
