const SMG_VERSION_COMPATIBILITY_NOTICE_KEY: &str = "smg.version_compatibility_notice";
const SMG_SCRYER_UPDATE_NOTICE_KEY: &str = "smg.scryer_update_notice";
const AUTO_BACKUP_KEY_MIN_LENGTH: usize = 8;
const BYTES_PER_MIB: u64 = 1024 * 1024;

fn configured_image_cache_max_bytes(image_cache_max_size_mb: i32) -> u64 {
    u64::try_from(image_cache_max_size_mb)
        .unwrap_or_default()
        .saturating_mul(BYTES_PER_MIB)
}

fn effective_image_cache_limit(image_cache_max_size_mb: i32) -> (u64, f64, bool) {
    let env_override = std::env::var(IMAGE_CACHE_MAX_BYTES_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let effective_bytes = env_override
        .unwrap_or_else(|| configured_image_cache_max_bytes(image_cache_max_size_mb));
    (
        effective_bytes,
        effective_bytes as f64 / BYTES_PER_MIB as f64,
        env_override.is_some(),
    )
}

fn normalize_auto_backup_daily_time_local(value: &str) -> AppResult<String> {
    let value = value.trim();
    let (hour, minute) = value
        .split_once(':')
        .ok_or_else(|| AppError::Validation("daily time must use HH:MM format".to_string()))?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time hour must be numeric".to_string()))?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time minute must be numeric".to_string()))?;
    if hour > 23 || minute > 59 {
        return Err(AppError::Validation(
            "daily time must be between 00:00 and 23:59".to_string(),
        ));
    }
    Ok(format!("{hour:02}:{minute:02}"))
}

fn auto_backup_key_non_whitespace_len(value: &str) -> usize {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .count()
}

fn validate_auto_backup_key_update(
    enabled: bool,
    existing_auto_backup_key_present: bool,
    set_auto_backup_key: Option<&str>,
    clear_auto_backup_key: bool,
) -> AppResult<()> {
    let replacement_key = set_auto_backup_key
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let replacement_key_present = replacement_key.is_some();
    if let Some(replacement_key) = replacement_key
        && auto_backup_key_non_whitespace_len(replacement_key) < AUTO_BACKUP_KEY_MIN_LENGTH
    {
        return Err(AppError::Validation(format!(
            "automatic backup key must be at least {AUTO_BACKUP_KEY_MIN_LENGTH} non-whitespace characters"
        )));
    }
    if clear_auto_backup_key && replacement_key_present {
        return Err(AppError::Validation(
            "automatic backup key cannot be replaced and cleared in the same request".to_string(),
        ));
    }
    if enabled && clear_auto_backup_key {
        return Err(AppError::Validation(
            "automatic backup key cannot be cleared while automatic backups are enabled"
                .to_string(),
        ));
    }
    if enabled && !existing_auto_backup_key_present && !replacement_key_present {
        return Err(AppError::Validation(
            "automatic backups require a backup key".to_string(),
        ));
    }

    Ok(())
}
fn split_pem_certificate_blocks(bundle_pem: &str) -> AppResult<Vec<String>> {
    let trimmed = bundle_pem.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    let blocks = PEM_CERT_BLOCK_RE
        .find_iter(trimmed)
        .map(|matched| {
            let block = matched.as_str().trim();
            if block.ends_with('\n') {
                block.to_string()
            } else {
                format!("{block}\n")
            }
        })
        .collect::<Vec<_>>();

    if blocks.is_empty() {
        return Err(AppError::Validation(
            "trusted certificate bundle must contain PEM-encoded X.509 certificates".to_string(),
        ));
    }

    let remaining = PEM_CERT_BLOCK_RE.replace_all(trimmed, "");
    if !remaining.trim().is_empty() {
        return Err(AppError::Validation(
            "trusted certificate bundle may only contain X.509 certificate PEM blocks".to_string(),
        ));
    }

    Ok(blocks)
}
fn parse_pem_certificate_der(block_pem: &str) -> AppResult<Vec<u8>> {
    let mut certificates = CertificateDer::pem_slice_iter(block_pem.as_bytes());
    let certificate = certificates
        .next()
        .ok_or_else(|| {
            AppError::Validation(
            "trusted certificate bundle did not contain a readable X.509 certificate".to_string(),
            )
        })?
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to parse trusted certificate PEM block: {error}"
            ))
        })?;

    if certificates
        .next()
        .transpose()
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to parse trailing PEM content for trusted certificate: {error}"
            ))
        })?
        .is_some()
    {
        return Err(AppError::Validation(
            "each trusted certificate entry must contain exactly one X.509 certificate".to_string(),
        ));
    }

    Ok(certificate.as_ref().to_vec())
}
fn normalize_plugin_http_ca_bundle_pem(bundle_pem: &str) -> AppResult<String> {
    let blocks = split_pem_certificate_blocks(bundle_pem)?;
    if blocks.is_empty() {
        return Ok(String::new());
    }

    let mut normalized = Vec::with_capacity(blocks.len());
    for block in blocks {
        let _ = parse_pem_certificate_der(&block)?;
        normalized.push(block);
    }
    Ok(normalized.join("\n"))
}
fn summarize_plugin_http_trusted_certificates(
    bundle_pem: &str,
) -> AppResult<Vec<GeneralSettingsTrustedCertificate>> {
    let blocks = split_pem_certificate_blocks(bundle_pem)?;
    let mut certificates = Vec::with_capacity(blocks.len());
    for block in blocks {
        let der = parse_pem_certificate_der(&block)?;
        // Compatibility only: this fingerprint exists to be compared by a
        // human against what a browser, `openssl x509 -fingerprint`, or a CA
        // portal prints, and every one of those emits SHA-256. A BLAKE3 digest
        // here would be correct and useless. First-party hashing uses
        // `crate::helpers::blake3_identity_hex`.
        let digest = aws_lc_digest::digest(&aws_lc_digest::SHA256, &der);
        certificates.push(GeneralSettingsTrustedCertificate {
            fingerprint_sha256: crate::helpers::to_hex(digest.as_ref()),
            pem: block,
        });
    }
    Ok(certificates)
}
#[derive(Debug, Clone, PartialEq)]
pub struct GeneralSettings {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
    pub image_cache_max_size_mb: i32,
    pub effective_image_cache_max_size_bytes: u64,
    pub effective_image_cache_max_size_mb: f64,
    pub image_cache_max_size_env_override_active: bool,
    pub plugin_http_ca_bundle_pem: String,
    pub plugin_http_trusted_certificates: Vec<GeneralSettingsTrustedCertificate>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralSettingsTrustedCertificate {
    pub fingerprint_sha256: String,
    pub pem: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub auto_backup_key_present: bool,
    pub auto_backup_disabled_missing_key_notice: bool,
    pub next_run_at: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSettings {
    pub custom_backup_path: Option<String>,
    pub default_backup_path: String,
    pub effective_backup_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateGeneralSettings {
    pub keep_history_forever: Option<bool>,
    pub history_retention_days: Option<i32>,
    pub image_cache_max_size_mb: Option<i32>,
    pub plugin_http_ca_bundle_pem: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub set_auto_backup_key: Option<String>,
    pub clear_auto_backup_key: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateBackupSettings {
    pub custom_backup_path: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginAutoUpdateSettings {
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePluginAutoUpdateSettings {
    pub enabled: bool,
}

fn normalize_backup_path_setting(value: Option<String>) -> AppResult<Option<PathBuf>> {
    let Some(value) = normalize_optional_string(value) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "backup path must be an absolute server path".to_string(),
        ));
    }
    Ok(Some(path))
}

fn backup_path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn validate_backup_storage_dir(path: &Path) -> AppResult<()> {
    std::fs::create_dir_all(path).map_err(|error| {
        AppError::Validation(format!("failed to create backup directory: {error}"))
    })?;
    if !path.is_dir() {
        return Err(AppError::Validation(
            "backup path must be a directory".to_string(),
        ));
    }

    let timestamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let probe_path = path.join(format!(
        ".scryer-backup-write-test-{}-{timestamp}.tmp",
        std::process::id()
    ));
    std::fs::write(&probe_path, b"scryer backup path probe")
        .map_err(|error| AppError::Validation(format!("backup path is not writable: {error}")))?;
    std::fs::remove_file(&probe_path).map_err(|error| {
        AppError::Validation(format!("failed to remove backup path probe: {error}"))
    })?;
    Ok(())
}

impl AppUseCase {
    async fn load_general_settings(&self) -> AppResult<GeneralSettings> {
        let keep_history_forever = self
            .read_setting_bool_value(HISTORY_KEEP_FOREVER_KEY, None)
            .await?
            .unwrap_or(false);
        let history_retention_days = self
            .read_setting_i64_value(HISTORY_RETENTION_DAYS_KEY, None)
            .await?
            .map(|value| value.max(1) as i32)
            .unwrap_or(180);
        let image_cache_max_size_mb = self
            .read_setting_i64_value(IMAGE_CACHE_MAX_SIZE_MB_KEY, None)
            .await?
            .map(|value| value.clamp(1, i64::from(i32::MAX)) as i32)
            .unwrap_or(DEFAULT_IMAGE_CACHE_MAX_SIZE_MB);
        let (
            effective_image_cache_max_size_bytes,
            effective_image_cache_max_size_mb,
            image_cache_max_size_env_override_active,
        ) = effective_image_cache_limit(image_cache_max_size_mb);
        let stored_bundle = self
            .read_setting_string_value(PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, None)
            .await?
            .unwrap_or_default();
        let (plugin_http_ca_bundle_pem, plugin_http_trusted_certificates) =
            match normalize_plugin_http_ca_bundle_pem(&stored_bundle).and_then(|bundle| {
                let certificates = summarize_plugin_http_trusted_certificates(&bundle)?;
                Ok((bundle, certificates))
            }) {
                Ok(result) => result,
                Err(error) => {
                    if !stored_bundle.trim().is_empty() {
                        warn!(
                            error = %error,
                            "stored plugin HTTP trusted certificate bundle could not be normalized"
                        );
                    }
                    (stored_bundle, Vec::new())
                }
            };

        Ok(GeneralSettings {
            keep_history_forever,
            history_retention_days,
            image_cache_max_size_mb,
            effective_image_cache_max_size_bytes,
            effective_image_cache_max_size_mb,
            image_cache_max_size_env_override_active,
            plugin_http_ca_bundle_pem,
            plugin_http_trusted_certificates,
        })
    }
}
impl AppUseCase {
    pub fn default_backup_dir(&self) -> PathBuf {
        self.services.config.backup_dir.clone()
    }

    pub async fn effective_backup_dir(&self) -> AppResult<PathBuf> {
        Ok(normalize_backup_path_setting(
            self.read_setting_string_value(BACKUP_PATH_KEY, None)
                .await?,
        )?
        .unwrap_or_else(|| self.default_backup_dir()))
    }

    async fn load_backup_settings(&self) -> AppResult<BackupSettings> {
        let custom_backup_path = normalize_backup_path_setting(
            self.read_setting_string_value(BACKUP_PATH_KEY, None)
                .await?,
        )?
        .map(|path| backup_path_to_string(&path));
        let default_backup_path = backup_path_to_string(&self.default_backup_dir());
        let effective_backup_path = custom_backup_path
            .clone()
            .unwrap_or_else(|| default_backup_path.clone());

        Ok(BackupSettings {
            custom_backup_path,
            default_backup_path,
            effective_backup_path,
        })
    }

    async fn auto_backup_key_present(&self) -> AppResult<bool> {
        Ok(self
            .read_setting_string_value(AUTO_BACKUP_KEY_KEY, None)
            .await?
            .is_some_and(|value| !value.trim().is_empty()))
    }

    pub(crate) async fn load_auto_backup_settings(&self) -> AppResult<AutoBackupSettings> {
        let enabled = self
            .read_setting_bool_value(AUTO_BACKUP_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let daily_time_local = normalize_auto_backup_daily_time_local(
            &self
                .read_setting_string_value(AUTO_BACKUP_DAILY_TIME_LOCAL_KEY, None)
                .await?
                .unwrap_or_else(|| DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL.to_string()),
        )?;
        let auto_backup_key_present = self.auto_backup_key_present().await?;
        let auto_backup_disabled_missing_key_notice = self
            .read_setting_bool_value(AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, None)
            .await?
            .unwrap_or(false);
        let next_run_at = if enabled {
            Some(
                crate::security::backup::compute_next_auto_backup_run_at(
                    &daily_time_local,
                    chrono::Utc::now(),
                )?
                .to_rfc3339(),
            )
        } else {
            None
        };

        Ok(AutoBackupSettings {
            enabled,
            daily_time_local,
            auto_backup_key_present,
            auto_backup_disabled_missing_key_notice,
            next_run_at,
        })
    }
}
impl AppUseCase {
    pub(crate) async fn general_settings(&self) -> AppResult<GeneralSettings> {
        self.load_general_settings().await
    }

    pub async fn sync_image_cache_runtime_limit(&self) -> AppResult<()> {
        let settings = self.load_general_settings().await?;
        self.services
            .library
            .image_proxy_cache_control
            .set_configured_max_bytes(configured_image_cache_max_bytes(
                settings.image_cache_max_size_mb,
            ))
            .await
    }
}
impl AppUseCase {
    pub async fn get_general_settings(&self, actor: &User) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_general_settings().await
    }
}
impl AppUseCase {
    pub async fn get_auto_backup_settings(&self, actor: &User) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_auto_backup_settings().await
    }

    pub async fn get_backup_settings(&self, actor: &User) -> AppResult<BackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_backup_settings().await
    }
}
impl AppUseCase {
    pub async fn update_general_settings(
        &self,
        actor: &User,
        input: UpdateGeneralSettings,
    ) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let current = self.load_general_settings().await?;
        let keep_history_forever = input
            .keep_history_forever
            .unwrap_or(current.keep_history_forever);
        let requested_history_retention_days = input
            .history_retention_days
            .unwrap_or(current.history_retention_days);
        let history_retention_days = if keep_history_forever && requested_history_retention_days < 1
        {
            current.history_retention_days
        } else {
            requested_history_retention_days
        };
        let image_cache_max_size_mb = input
            .image_cache_max_size_mb
            .unwrap_or(current.image_cache_max_size_mb);

        if history_retention_days < 1 {
            return Err(AppError::Validation(
                "history retention days must be at least 1".to_string(),
            ));
        }
        if image_cache_max_size_mb < 1 {
            return Err(AppError::Validation(
                "image cache maximum size must be at least 1 MiB".to_string(),
            ));
        }
        let plugin_http_ca_bundle_pem_update = input
            .plugin_http_ca_bundle_pem
            .as_deref()
            .map(normalize_plugin_http_ca_bundle_pem)
            .transpose()?;
        let plugin_http_ca_bundle_pem = plugin_http_ca_bundle_pem_update
            .clone()
            .unwrap_or(current.plugin_http_ca_bundle_pem);
        let plugin_http_trusted_certificates =
            summarize_plugin_http_trusted_certificates(&plugin_http_ca_bundle_pem)?;

        let mut changed_keys = Vec::new();
        if input.keep_history_forever.is_some() {
            self.upsert_system_setting_json(
                HISTORY_KEEP_FOREVER_KEY,
                &keep_history_forever,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(HISTORY_KEEP_FOREVER_KEY.to_string());
        }
        if input.history_retention_days.is_some() {
            self.upsert_system_setting_json(
                HISTORY_RETENTION_DAYS_KEY,
                &history_retention_days,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(HISTORY_RETENTION_DAYS_KEY.to_string());
        }
        if input.image_cache_max_size_mb.is_some() {
            self.upsert_system_setting_json(
                IMAGE_CACHE_MAX_SIZE_MB_KEY,
                &image_cache_max_size_mb,
                Some(actor.id.clone()),
            )
            .await?;
            self.services
                .library
                .image_proxy_cache_control
                .set_configured_max_bytes(configured_image_cache_max_bytes(image_cache_max_size_mb))
                .await?;
            changed_keys.push(IMAGE_CACHE_MAX_SIZE_MB_KEY.to_string());
        }
        if let Some(plugin_http_ca_bundle_pem) = plugin_http_ca_bundle_pem_update {
            self.upsert_system_setting_json(
                PLUGIN_HTTP_CA_BUNDLE_PEM_KEY,
                &plugin_http_ca_bundle_pem,
                Some(actor.id.clone()),
            )
            .await?;
            if let Some(runtime) = self.services.config.plugin_http_trust_runtime.available() {
                runtime.set_plugin_http_ca_bundle_pem(plugin_http_ca_bundle_pem)?;
            }
            changed_keys.push(PLUGIN_HTTP_CA_BUNDLE_PEM_KEY.to_string());
        }

        if !changed_keys.is_empty() {
            self.emit_settings_saved(actor, "general_settings", None, changed_keys)
                .await;
        }

        let (
            effective_image_cache_max_size_bytes,
            effective_image_cache_max_size_mb,
            image_cache_max_size_env_override_active,
        ) = effective_image_cache_limit(image_cache_max_size_mb);
        Ok(GeneralSettings {
            keep_history_forever,
            history_retention_days,
            image_cache_max_size_mb,
            effective_image_cache_max_size_bytes,
            effective_image_cache_max_size_mb,
            image_cache_max_size_env_override_active,
            plugin_http_ca_bundle_pem,
            plugin_http_trusted_certificates,
        })
    }
}
impl AppUseCase {
    pub async fn update_auto_backup_settings(
        &self,
        actor: &User,
        input: UpdateAutoBackupSettings,
    ) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let current_auto_backup_key_present = self.auto_backup_key_present().await?;
        validate_auto_backup_key_update(
            input.enabled,
            current_auto_backup_key_present,
            input.set_auto_backup_key.as_deref(),
            input.clear_auto_backup_key,
        )?;
        let daily_time_local = normalize_auto_backup_daily_time_local(&input.daily_time_local)?;

        self.upsert_system_setting_json(
            AUTO_BACKUP_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
            &daily_time_local,
            Some(actor.id.clone()),
        )
        .await?;

        let mut changed_keys = vec![
            AUTO_BACKUP_ENABLED_KEY.to_string(),
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY.to_string(),
        ];

        if input.clear_auto_backup_key {
            self.delete_system_setting(AUTO_BACKUP_KEY_KEY).await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        } else if let Some(set_auto_backup_key) = input.set_auto_backup_key
            && !set_auto_backup_key.trim().is_empty()
        {
            self.upsert_system_setting_json(
                AUTO_BACKUP_KEY_KEY,
                &set_auto_backup_key,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        }

        self.emit_settings_saved(actor, "auto_backup_settings", None, changed_keys)
            .await;

        self.load_auto_backup_settings().await
    }

    pub async fn update_backup_settings(
        &self,
        actor: &User,
        input: UpdateBackupSettings,
    ) -> AppResult<BackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let custom_path = normalize_backup_path_setting(input.custom_backup_path)?;
        if let Some(path) = &custom_path {
            validate_backup_storage_dir(path)?;
            self.upsert_system_setting_json(
                BACKUP_PATH_KEY,
                &backup_path_to_string(path),
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_system_setting(BACKUP_PATH_KEY).await?;
        }

        self.emit_settings_saved(
            actor,
            "backup_settings",
            None,
            vec![BACKUP_PATH_KEY.to_string()],
        )
        .await;
        self.load_backup_settings().await
    }

    pub async fn acknowledge_auto_backup_disabled_missing_key_notice(
        &self,
        actor: &User,
    ) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.upsert_system_setting_json(
            AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY,
            &false,
            Some(actor.id.clone()),
        )
        .await?;
        self.emit_settings_saved(
            actor,
            "auto_backup_settings",
            None,
            vec![AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY.to_string()],
        )
        .await;
        self.load_auto_backup_settings().await
    }
}
impl AppUseCase {
    pub(crate) async fn load_plugin_auto_update_settings(
        &self,
    ) -> AppResult<PluginAutoUpdateSettings> {
        let enabled = self
            .read_setting_bool_value(PLUGIN_AUTO_UPDATE_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);

        Ok(PluginAutoUpdateSettings { enabled })
    }

    pub async fn get_plugin_auto_update_settings(
        &self,
        actor: &User,
    ) -> AppResult<PluginAutoUpdateSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_plugin_auto_update_settings().await
    }

    pub async fn update_plugin_auto_update_settings(
        &self,
        actor: &User,
        input: UpdatePluginAutoUpdateSettings,
    ) -> AppResult<PluginAutoUpdateSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.upsert_system_setting_json(
            PLUGIN_AUTO_UPDATE_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_settings_saved(
            actor,
            "plugin_auto_update_settings",
            None,
            vec![PLUGIN_AUTO_UPDATE_ENABLED_KEY.to_string()],
        )
        .await;

        self.load_plugin_auto_update_settings().await
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );

    #[test]
    fn normalize_auto_backup_daily_time_local_trims_and_zero_pads_values() {
        let normalized = normalize_auto_backup_daily_time_local(" 3:5 ").expect("normalized time");

        assert_eq!(normalized, "03:05");
    }

    #[test]
    fn normalize_auto_backup_daily_time_local_rejects_invalid_values() {
        assert!(normalize_auto_backup_daily_time_local("24:00").is_err());
        assert!(normalize_auto_backup_daily_time_local("10:60").is_err());
        assert!(normalize_auto_backup_daily_time_local("nope").is_err());
    }

    #[test]
    fn normalize_backup_path_setting_resets_empty_and_rejects_relative_paths() {
        assert_eq!(
            normalize_backup_path_setting(Some("   ".to_string())).expect("empty resets"),
            None
        );
        assert!(normalize_backup_path_setting(Some("relative/backups".to_string())).is_err());
    }

    #[test]
    fn normalize_backup_path_setting_accepts_absolute_paths() {
        let path = normalize_backup_path_setting(Some("/tmp/scryer-backups".to_string()))
            .expect("absolute path")
            .expect("custom path");

        assert_eq!(path, PathBuf::from("/tmp/scryer-backups"));
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_replace_and_clear_together() {
        let error = validate_auto_backup_key_update(false, true, Some("secret12"), true)
            .expect_err("set and clear should be rejected");

        assert!(
            error
                .to_string()
                .contains("automatic backup key cannot be replaced and cleared"),
        );
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_enabled_without_effective_key() {
        let error = validate_auto_backup_key_update(true, false, None, false)
            .expect_err("enabled without key should be rejected");

        assert!(
            error
                .to_string()
                .contains("automatic backups require a backup key"),
        );
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_short_replacement_key() {
        let error = validate_auto_backup_key_update(true, false, Some("  1234567  "), false)
            .expect_err("short replacement key should be rejected");

        assert!(
            error
                .to_string()
                .contains("at least 8 non-whitespace characters"),
        );
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_clearing_key_while_enabled() {
        let error = validate_auto_backup_key_update(true, true, None, true)
            .expect_err("enabled clear should be rejected");

        assert!(
            error
                .to_string()
                .contains("cannot be cleared while automatic backups are enabled"),
        );
    }

    #[test]
    fn validate_auto_backup_key_update_accepts_enabled_with_replacement_key() {
        validate_auto_backup_key_update(true, false, Some("  secret12  "), false)
            .expect("replacement key should allow enabling");
    }

    #[test]
    fn validate_auto_backup_key_update_accepts_existing_key_without_replacement() {
        validate_auto_backup_key_update(true, true, None, false)
            .expect("existing saved key should allow enabling");
    }

    #[test]
    fn parse_pem_certificate_der_accepts_one_certificate() {
        let certificate = parse_pem_certificate_der(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("valid certificate PEM should parse");

        assert!(!certificate.is_empty());
    }

    #[test]
    fn parse_pem_certificate_der_rejects_malformed_base64() {
        let error = parse_pem_certificate_der(
            "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n",
        )
        .expect_err("malformed certificate PEM should be rejected");

        assert!(
            error
                .to_string()
                .contains("failed to parse trusted certificate PEM block"),
        );
    }

    #[test]
    fn parse_pem_certificate_der_rejects_multiple_certificates() {
        let error = parse_pem_certificate_der(&format!(
            "{TEST_PLUGIN_HTTP_CA_CERT_PEM}\n{TEST_PLUGIN_HTTP_CA_CERT_PEM}"
        ))
        .expect_err("multiple certificate PEM blocks should be rejected");

        assert!(
            error
                .to_string()
                .contains("must contain exactly one X.509 certificate"),
        );
    }

    #[test]
    fn normalize_plugin_http_ca_bundle_pem_rejects_trailing_non_certificate_text() {
        let error = normalize_plugin_http_ca_bundle_pem(&format!(
            "{TEST_PLUGIN_HTTP_CA_CERT_PEM}\nnot-a-certificate"
        ))
        .expect_err("trailing text should be rejected");

        assert!(
            error
                .to_string()
                .contains("may only contain X.509 certificate PEM blocks"),
        );
    }

    #[test]
    fn summarize_plugin_http_trusted_certificates_preserves_normalized_blocks() {
        let normalized = normalize_plugin_http_ca_bundle_pem(&format!(
            "{TEST_PLUGIN_HTTP_CA_CERT_PEM}\n\n{TEST_PLUGIN_HTTP_CA_CERT_PEM}"
        ))
        .expect("normalized certificate bundle");
        let certificates = summarize_plugin_http_trusted_certificates(&normalized)
            .expect("summarized certificate bundle");

        assert_eq!(certificates.len(), 2);
        assert_eq!(
            certificates[0].fingerprint_sha256,
            certificates[1].fingerprint_sha256
        );
        assert!(!certificates[0].fingerprint_sha256.is_empty());
        assert_eq!(certificates[0].pem, TEST_PLUGIN_HTTP_CA_CERT_PEM);
        assert_eq!(certificates[1].pem, TEST_PLUGIN_HTTP_CA_CERT_PEM);
    }
}
