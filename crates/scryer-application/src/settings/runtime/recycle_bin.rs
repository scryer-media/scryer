impl AppUseCase {
    async fn load_recycle_bin_settings(&self) -> AppResult<RecycleBinSettings> {
        let enabled = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_MEDIA,
                RECYCLE_BIN_ENABLED_KEY,
                None,
            )
            .await?
            .map(|value| value != "false")
            .unwrap_or(true);

        Ok(RecycleBinSettings { enabled })
    }
}
impl AppUseCase {
    async fn recycle_bin_config_values(&self) -> (bool, Option<String>, u32) {
        let enabled = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_MEDIA,
                RECYCLE_BIN_ENABLED_KEY,
                None,
            )
            .await
            .ok()
            .flatten()
            .map(|value| value != "false")
            .unwrap_or(true);

        let custom_path = self
            .read_setting_string_value_for_scope(SETTINGS_SCOPE_MEDIA, RECYCLE_BIN_PATH_KEY, None)
            .await
            .ok()
            .flatten()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let retention_days = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_MEDIA,
                RECYCLE_BIN_RETENTION_DAYS_KEY,
                None,
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u32>().ok())
            // Clamp to a minimum of 1 day: a retention of 0 makes the purge cutoff
            // `now`, which would purge the entire recycle bin on the next sweep.
            .map(|value| value.max(1))
            .unwrap_or(7);

        (enabled, custom_path, retention_days)
    }
}
impl AppUseCase {
    fn recycle_bin_validation_error(
        base_path: &Path,
        custom_path: bool,
        configured_roots: &[PathBuf],
    ) -> Option<String> {
        if custom_path && !base_path.is_absolute() {
            return Some(format!(
                "custom recycle bin path must be absolute: {}",
                base_path.display()
            ));
        }

        let normalized_base = Self::normalize_recycle_config_path(base_path);
        for root in configured_roots {
            if custom_path
                && (normalized_base == *root
                    || normalized_base.starts_with(root)
                    || root.starts_with(&normalized_base))
            {
                return Some(format!(
                    "custom recycle bin path {} must be outside configured media root {}",
                    normalized_base.display(),
                    root.display()
                ));
            }
        }

        None
    }
}
impl AppUseCase {
    fn recycle_bin_config_from_values(
        enabled: bool,
        custom_path: Option<&str>,
        retention_days: u32,
        media_root: Option<&str>,
        configured_roots: &[PathBuf],
    ) -> crate::recycle_bin::RecycleBinConfig {
        Self::recycle_bin_config_from_path_values(
            enabled,
            custom_path,
            retention_days,
            media_root.map(Path::new),
            configured_roots,
        )
    }

    fn recycle_bin_config_from_path_values(
        enabled: bool,
        custom_path: Option<&str>,
        retention_days: u32,
        media_root: Option<&Path>,
        configured_roots: &[PathBuf],
    ) -> crate::recycle_bin::RecycleBinConfig {
        let custom_path_configured = custom_path.is_some();
        let base_path = if let Some(path) = custom_path {
            PathBuf::from(path)
        } else if let Some(root) = media_root {
            root.join(".scryer-recycle")
        } else {
            PathBuf::from("/tmp/.scryer-recycle")
        };
        let validation_error = Self::recycle_bin_validation_error(
            &base_path,
            custom_path_configured,
            configured_roots,
        );
        let cleanup_enabled = validation_error.is_none();

        crate::recycle_bin::RecycleBinConfig {
            enabled,
            base_path,
            retention_days,
            cleanup_enabled,
            validation_error,
            source_roots: configured_roots.to_vec(),
        }
    }
}
impl AppUseCase {
    pub async fn recycle_bin_config_for_media_root(
        &self,
        media_root: Option<&str>,
    ) -> crate::recycle_bin::RecycleBinConfig {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let configured_roots = media_root
            .into_iter()
            .map(|root| Self::normalize_recycle_config_path(Path::new(root.trim())))
            .filter(|root| !root.as_os_str().is_empty())
            .collect::<Vec<_>>();
        Self::recycle_bin_config_from_values(
            enabled,
            custom_path.as_deref(),
            retention_days,
            media_root,
            &configured_roots,
        )
    }
}
impl AppUseCase {
    pub(crate) async fn recycle_bin_config_for_media_root_path(
        &self,
        media_root: Option<&Path>,
    ) -> crate::recycle_bin::RecycleBinConfig {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let configured_roots = media_root
            .into_iter()
            .map(Self::normalize_recycle_config_path)
            .filter(|root| !root.as_os_str().is_empty())
            .collect::<Vec<_>>();
        Self::recycle_bin_config_from_path_values(
            enabled,
            custom_path.as_deref(),
            retention_days,
            media_root,
            &configured_roots,
        )
    }
}
impl AppUseCase {
    pub async fn recycle_bin_configs_for_media_roots<I>(
        &self,
        media_roots: I,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)>
    where
        I: IntoIterator<Item = String>,
    {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let media_roots = media_roots
            .into_iter()
            .map(|media_root| media_root.trim().to_string())
            .filter(|media_root| !media_root.is_empty())
            .collect::<Vec<_>>();
        let configured_roots = media_roots
            .iter()
            .map(|media_root| Self::normalize_recycle_config_path(Path::new(media_root)))
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<Vec<_>>();
        let mut configs = Vec::new();
        let mut seen_paths = HashSet::new();

        for media_root in media_roots {
            let config = Self::recycle_bin_config_from_values(
                enabled,
                custom_path.as_deref(),
                retention_days,
                Some(media_root.as_str()),
                &configured_roots,
            );
            if !seen_paths.insert(Self::normalize_recycle_config_path(&config.base_path)) {
                continue;
            }

            let entry_media_root = if custom_path.is_some() {
                String::new()
            } else {
                media_root
            };
            configs.push((entry_media_root, config));
        }

        configs
    }
}
impl AppUseCase {
    pub async fn get_recycle_bin_settings(&self, actor: &User) -> AppResult<RecycleBinSettings> {
        if !self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
            && !self
                .has_any_library_permission(
                    actor,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to view recycle bin settings".to_string(),
            ));
        }

        self.load_recycle_bin_settings().await
    }
}
impl AppUseCase {
    pub async fn update_recycle_bin_settings(
        &self,
        actor: &User,
        input: UpdateRecycleBinSettings,
    ) -> AppResult<RecycleBinSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.upsert_media_setting_json(
            RECYCLE_BIN_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "recycle_bin_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(vec![RECYCLE_BIN_ENABLED_KEY.to_string()]);

        self.load_recycle_bin_settings().await
    }
}
