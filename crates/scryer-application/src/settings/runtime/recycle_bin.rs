/// Longest retention the settings surface accepts. The purge cutoff is
/// `now - retention_days`, which must stay inside the representable date range.
const RECYCLE_BIN_MAX_RETENTION_DAYS: u32 = 3650;

impl AppUseCase {
    /// Report the recycle-bin settings exactly as the bin applies them.
    ///
    /// `visible_library_ids` limits which library roots the effective paths
    /// are listed for and replaces the validation error, which can name any
    /// root, with a generic one; `None` reports everything.
    async fn load_recycle_bin_settings(
        &self,
        visible_library_ids: Option<&HashSet<String>>,
    ) -> AppResult<RecycleBinSettings> {
        let (enabled, path, retention_days) = self.recycle_bin_config_values().await;
        let roots = self.all_library_root_folders().await?;
        let visible_roots = roots
            .iter()
            .filter(|root| {
                visible_library_ids.is_none_or(|visible| visible.contains(&root.library_id))
            })
            .map(|root| root.path.trim().to_string())
            .collect::<HashSet<_>>();
        let mut configs = self
            .recycle_bin_configs_for_media_roots(roots.into_iter().map(|root| root.path))
            .await;
        if configs.is_empty() && path.is_some() {
            configs.push((String::new(), self.recycle_bin_config_for_media_root(None).await));
        }

        let validation_error = configs
            .iter()
            .find_map(|(_, config)| config.validation_error.clone())
            .map(|error| {
                if visible_library_ids.is_some() {
                    "recycle bin path conflicts with a library root".to_string()
                } else {
                    error
                }
            });
        let effective_paths = configs
            .into_iter()
            .filter(|(media_root, _)| media_root.is_empty() || visible_roots.contains(media_root))
            .map(|(_, config)| config.base_path.to_string_lossy().into_owned())
            .collect();

        Ok(RecycleBinSettings {
            enabled,
            path,
            retention_days,
            effective_paths,
            validation_error,
        })
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
        if self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
        {
            return self.load_recycle_bin_settings(None).await;
        }

        let manageable_library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::ManageTitles)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if manageable_library_ids.is_empty() {
            return Err(AppError::Unauthorized(
                "You do not have permission to view recycle bin settings".to_string(),
            ));
        }

        self.load_recycle_bin_settings(Some(&manageable_library_ids))
            .await
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

        // Only fields present in the update are validated and written, so a
        // stale or partial client never resets values it did not send.
        let retention_days = match input.retention_days {
            Some(days) => Some(
                u32::try_from(days)
                    .ok()
                    .filter(|days| (1..=RECYCLE_BIN_MAX_RETENTION_DAYS).contains(days))
                    .ok_or_else(|| {
                        AppError::Validation(format!(
                            "recycle bin retention must be between 1 and {RECYCLE_BIN_MAX_RETENTION_DAYS} days"
                        ))
                    })?,
            ),
            None => None,
        };

        let path = input.path.map(|path| {
            path.as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
        });
        if let Some(Some(path)) = path.as_ref() {
            let configured_roots = self
                .all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| Self::normalize_recycle_config_path(Path::new(root.path.trim())))
                .filter(|root| !root.as_os_str().is_empty())
                .collect::<Vec<_>>();
            if let Some(error) =
                Self::recycle_bin_validation_error(Path::new(path), true, &configured_roots)
            {
                return Err(AppError::Validation(error));
            }
        }

        let updated_by = Some(actor.id.clone());
        let mut changed_keys = Vec::new();
        if let Some(enabled) = input.enabled {
            self.upsert_media_setting_json(RECYCLE_BIN_ENABLED_KEY, &enabled, updated_by.clone())
                .await?;
            changed_keys.push(RECYCLE_BIN_ENABLED_KEY.to_string());
        }
        // Retention is written before the path so a sweep between the two
        // writes never pairs a new bin with the old retention.
        if let Some(retention_days) = retention_days {
            self.upsert_media_setting_json(
                RECYCLE_BIN_RETENTION_DAYS_KEY,
                &retention_days,
                updated_by.clone(),
            )
            .await?;
            changed_keys.push(RECYCLE_BIN_RETENTION_DAYS_KEY.to_string());
        }
        if let Some(path) = path {
            // A JSON null reads back as unset, restoring the per-root default.
            self.upsert_media_setting_json(RECYCLE_BIN_PATH_KEY, &path, updated_by)
                .await?;
            changed_keys.push(RECYCLE_BIN_PATH_KEY.to_string());
        }

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
            .send(changed_keys);

        self.load_recycle_bin_settings(None).await
    }
}
