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
            relocation: None,
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

        // The same check again with every link resolved, so a bin or root
        // that reaches the other through a symlink is caught. A path that
        // cannot be resolved refuses the bin rather than skipping the check.
        if custom_path && !configured_roots.is_empty() {
            let resolved_base = match Self::resolve_recycle_config_links(base_path) {
                Ok(resolved) => resolved,
                Err(reason) => {
                    return Some(format!(
                        "custom recycle bin path {} could not be resolved: {reason}",
                        base_path.display()
                    ));
                }
            };
            for root in configured_roots.iter().filter(|root| root.is_absolute()) {
                let resolved_root = match Self::resolve_recycle_config_links(root) {
                    Ok(resolved) => resolved,
                    Err(reason) => {
                        return Some(format!(
                            "custom recycle bin path {} could not be checked against configured media root {}: {reason}",
                            normalized_base.display(),
                            root.display()
                        ));
                    }
                };
                if resolved_base == resolved_root
                    || resolved_base.starts_with(&resolved_root)
                    || resolved_root.starts_with(&resolved_base)
                {
                    return Some(format!(
                        "custom recycle bin path {} must be outside configured media root {} (they meet at {} once links are resolved)",
                        normalized_base.display(),
                        root.display(),
                        resolved_base.display()
                    ));
                }
            }
        }

        None
    }

    /// `path` with every link in the part that exists resolved. The part that
    /// does not exist yet is appended as written, since nothing there can
    /// redirect it. Fails when the path is not absolute, when a component
    /// cannot be inspected, when a component is a link that leads nowhere
    /// (creating the bin would follow it later), or when `..` follows a
    /// missing component, since where it lands depends on what gets created.
    fn resolve_recycle_config_links(path: &Path) -> Result<PathBuf, String> {
        use std::path::Component;

        if !path.is_absolute() {
            return Err("the path is not absolute".to_string());
        }
        let mut resolved = PathBuf::new();
        let mut exists = true;
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    resolved.push(component.as_os_str());
                    if matches!(component, Component::RootDir) {
                        resolved = Self::resolve_existing_component(&resolved)?
                            .unwrap_or(resolved);
                    }
                }
                Component::CurDir => {}
                Component::ParentDir if exists => {
                    let candidate = resolved.join("..");
                    match Self::resolve_existing_component(&candidate)? {
                        Some(real) => resolved = real,
                        None => {
                            return Err(format!("{} could not be followed", candidate.display()));
                        }
                    }
                }
                Component::ParentDir => {
                    return Err(format!(
                        "`..` follows {}, which does not exist yet",
                        resolved.display()
                    ));
                }
                Component::Normal(segment) => {
                    let candidate = resolved.join(segment);
                    if exists {
                        match Self::resolve_existing_component(&candidate)? {
                            Some(real) => resolved = real,
                            None => {
                                exists = false;
                                resolved = candidate;
                            }
                        }
                    } else {
                        resolved = candidate;
                    }
                }
            }
        }
        Ok(resolved)
    }

    /// The real path of `candidate`, `None` when nothing is there, or why it
    /// cannot be told apart from either.
    fn resolve_existing_component(candidate: &Path) -> Result<Option<PathBuf>, String> {
        match std::fs::symlink_metadata(candidate) {
            Ok(metadata) => match std::fs::canonicalize(candidate) {
                Ok(real) => Ok(Some(real)),
                Err(error) if metadata.file_type().is_symlink() => Err(format!(
                    "{} is a link that cannot be followed: {error}",
                    candidate.display()
                )),
                Err(error) => Err(format!("{} cannot be resolved: {error}", candidate.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{} cannot be inspected: {error}", candidate.display())),
        }
    }
}
impl AppUseCase {
    /// The first of `roots` a configured custom recycle bin conflicts with,
    /// and why: the root would hold the bin, be the bin, or sit inside it.
    ///
    /// Only roots being added or changed are checked, so a root in
    /// `existing_roots` that already conflicts never blocks an edit. Without a
    /// custom bin, or with the recycle bin turned off, there is nothing to
    /// check; a bin that is refused whatever the roots are, including one
    /// whose links cannot be resolved, is not a conflict of any root either.
    pub(crate) async fn recycle_bin_conflict_for_library_roots<'a>(
        &self,
        existing_roots: impl IntoIterator<Item = &'a str>,
        roots: impl IntoIterator<Item = &'a str>,
    ) -> Option<(String, String)> {
        let (enabled, custom_path, _) = self.recycle_bin_config_values().await;
        if !enabled {
            return None;
        }
        let bin = PathBuf::from(custom_path?);
        if Self::recycle_bin_validation_error(&bin, true, &[]).is_some()
            || Self::resolve_recycle_config_links(&bin).is_err()
        {
            return None;
        }
        let normalize = |root: &str| Self::normalize_recycle_config_path(Path::new(root.trim()));
        let existing_roots = existing_roots
            .into_iter()
            .map(normalize)
            .collect::<HashSet<_>>();
        roots.into_iter().find_map(|root| {
            let normalized = normalize(root);
            if normalized.as_os_str().is_empty() || existing_roots.contains(&normalized) {
                return None;
            }
            Self::recycle_bin_validation_error(&bin, true, std::slice::from_ref(&normalized))
                .map(|reason| (root.trim().to_string(), reason))
        })
    }
}
impl AppUseCase {
    /// Every current library root a custom recycle bin must stay outside,
    /// whichever root the file being recycled comes from.
    ///
    /// Without a custom bin there is nothing to check, so nothing is read. A
    /// failed read comes back as the reason to refuse the recycle, never as an
    /// empty list, so the check is never skipped.
    async fn recycle_bin_library_roots(
        &self,
        custom_path: Option<&str>,
    ) -> Result<Vec<PathBuf>, String> {
        if custom_path.is_none() {
            return Ok(Vec::new());
        }
        self.all_library_root_folders()
            .await
            .map(|roots| {
                roots
                    .into_iter()
                    .map(|root| Self::normalize_recycle_config_path(Path::new(root.path.trim())))
                    .filter(|root| !root.as_os_str().is_empty())
                    .collect()
            })
            .map_err(|error| {
                format!(
                    "custom recycle bin path could not be checked against the library roots: {error}"
                )
            })
    }
}
impl AppUseCase {
    /// `configured_roots` are the roots the source file may come from.
    /// `library_roots` are further roots a custom bin must also stay outside,
    /// or why they could not be read, which refuses a custom bin outright.
    fn recycle_bin_config_from_values(
        enabled: bool,
        custom_path: Option<&str>,
        retention_days: u32,
        media_root: Option<&str>,
        configured_roots: &[PathBuf],
        library_roots: Result<&[PathBuf], &str>,
    ) -> crate::recycle_bin::RecycleBinConfig {
        Self::recycle_bin_config_from_path_values(
            enabled,
            custom_path,
            retention_days,
            media_root.map(Path::new),
            configured_roots,
            library_roots,
        )
    }

    fn recycle_bin_config_from_path_values(
        enabled: bool,
        custom_path: Option<&str>,
        retention_days: u32,
        media_root: Option<&Path>,
        configured_roots: &[PathBuf],
        library_roots: Result<&[PathBuf], &str>,
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
        )
        .or_else(|| match library_roots {
            Ok(roots) => {
                Self::recycle_bin_validation_error(&base_path, custom_path_configured, roots)
            }
            Err(error) if custom_path_configured => Some(error.to_string()),
            Err(_) => None,
        });
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
    /// The bin a file removed from `media_root` goes to. The source file must
    /// live under `media_root`; a custom bin must stay outside every current
    /// library root, not only this one.
    pub async fn recycle_bin_config_for_media_root(
        &self,
        media_root: Option<&str>,
    ) -> crate::recycle_bin::RecycleBinConfig {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let library_roots = self.recycle_bin_library_roots(custom_path.as_deref()).await;
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
            library_roots.as_deref().map_err(String::as_str),
        )
    }
}
impl AppUseCase {
    /// [`Self::recycle_bin_config_for_media_root`] for a root held as a path.
    pub(crate) async fn recycle_bin_config_for_media_root_path(
        &self,
        media_root: Option<&Path>,
    ) -> crate::recycle_bin::RecycleBinConfig {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let library_roots = self.recycle_bin_library_roots(custom_path.as_deref()).await;
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
            library_roots.as_deref().map_err(String::as_str),
        )
    }
}
impl AppUseCase {
    /// Bins for a set of roots, checked only against those roots. Callers
    /// that pass every library root get the full check; a caller about to
    /// recycle from a subset uses [`Self::recycle_bin_configs_for_recycling`].
    pub async fn recycle_bin_configs_for_media_roots<I>(
        &self,
        media_roots: I,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)>
    where
        I: IntoIterator<Item = String>,
    {
        let values = self.recycle_bin_config_values().await;
        Self::recycle_bin_configs_from_values(values, media_roots, Ok(&[]))
    }

    /// Bins for a set of roots a file is about to be recycled from. The source
    /// file must live under one of `media_roots`; a custom bin must stay
    /// outside every current library root.
    pub(crate) async fn recycle_bin_configs_for_recycling<I>(
        &self,
        media_roots: I,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)>
    where
        I: IntoIterator<Item = String>,
    {
        let values = self.recycle_bin_config_values().await;
        let library_roots = self.recycle_bin_library_roots(values.1.as_deref()).await;
        Self::recycle_bin_configs_from_values(
            values,
            media_roots,
            library_roots.as_deref().map_err(String::as_str),
        )
    }

    fn recycle_bin_configs_from_values<I>(
        (enabled, custom_path, retention_days): (bool, Option<String>, u32),
        media_roots: I,
        library_roots: Result<&[PathBuf], &str>,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)>
    where
        I: IntoIterator<Item = String>,
    {
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
                library_roots,
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
        // Sending the stored path back is not a change. It is neither
        // revalidated, so a bin a later library root invalidated never blocks
        // saving other fields, nor rewritten, so no entries move.
        let (_, stored_path, _) = self.recycle_bin_config_values().await;
        let path = path.filter(|path| *path != stored_path);
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
        let bins_before_path_change = match path {
            Some(_) => Some(self.recycle_bin_bases_by_media_root().await?),
            None => None,
        };

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

        // Entries follow the location only on a save that changed it. The
        // setting is already written, so recycling from here on lands in the
        // new location and cannot race entries into the old one.
        let relocation = match bins_before_path_change {
            // The save itself has succeeded by now. Failing to read the new
            // locations leaves every entry where it is rather than failing
            // the save.
            Some(before) => match self.recycle_bin_bases_by_media_root().await {
                Ok(after) => {
                    let report = crate::recycle_bin::relocate_recycle_entries(
                        Self::recycle_bin_relocation_plans(&before, &after),
                    )
                    .await;
                    (!report.is_empty()).then_some(report)
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "recycle bin location saved, entries left in place: new locations could not be read"
                    );
                    None
                }
            },
            None => None,
        };

        let mut settings = self.load_recycle_bin_settings(None).await?;
        settings.relocation = relocation;
        Ok(settings)
    }
}
impl AppUseCase {
    /// Each library root with the bin its deleted media goes to under the
    /// current settings. With no roots, only a custom bin is listed: the
    /// rootless fallback is never a place to move entries to.
    async fn recycle_bin_bases_by_media_root(
        &self,
    ) -> AppResult<Vec<(Option<PathBuf>, crate::recycle_bin::RecycleBinConfig)>> {
        let (enabled, custom_path, retention_days) = self.recycle_bin_config_values().await;
        let roots = self
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| root.path.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        let configured_roots = roots
            .iter()
            .map(|root| Self::normalize_recycle_config_path(Path::new(root)))
            .collect::<Vec<_>>();
        if roots.is_empty() {
            return Ok(custom_path
                .as_deref()
                .map(|path| {
                    (
                        None,
                        Self::recycle_bin_config_from_values(
                            enabled,
                            Some(path),
                            retention_days,
                            None,
                            &configured_roots,
                            Ok(&[]),
                        ),
                    )
                })
                .into_iter()
                .collect());
        }
        Ok(roots
            .iter()
            .zip(configured_roots.iter())
            .map(|(root, normalized_root)| {
                (
                    Some(normalized_root.clone()),
                    Self::recycle_bin_config_from_values(
                        enabled,
                        custom_path.as_deref(),
                        retention_days,
                        Some(root.as_str()),
                        // Already every library root, so nothing further to check.
                        &configured_roots,
                        Ok(&[]),
                    ),
                )
            })
            .collect())
    }

    /// Pair each previous bin with the bins its entries now belong in. A root
    /// whose bin did not change contributes nothing, and a new bin that fails
    /// validation is never a destination.
    fn recycle_bin_relocation_plans(
        before: &[(Option<PathBuf>, crate::recycle_bin::RecycleBinConfig)],
        after: &[(Option<PathBuf>, crate::recycle_bin::RecycleBinConfig)],
    ) -> Vec<crate::recycle_bin::RecycleRelocationPlan> {
        let mut plans: Vec<crate::recycle_bin::RecycleRelocationPlan> = Vec::new();
        for (root, old_config) in before {
            let old_base = Self::normalize_recycle_config_path(&old_config.base_path);
            let new_bins = after
                .iter()
                .filter(|(after_root, _)| root.is_none() || after_root == root)
                .filter(|(_, config)| config.validation_error.is_none())
                .map(|(after_root, config)| {
                    (
                        after_root.clone(),
                        Self::normalize_recycle_config_path(&config.base_path),
                    )
                })
                .filter(|(_, new_base)| *new_base != old_base)
                .collect::<Vec<_>>();
            if new_bins.is_empty() {
                continue;
            }
            let plan = match plans.iter_mut().find(|plan| plan.from == old_base) {
                Some(plan) => plan,
                None => {
                    plans.push(crate::recycle_bin::RecycleRelocationPlan {
                        from: old_base.clone(),
                        targets: Vec::new(),
                    });
                    plans.last_mut().expect("plan was just pushed")
                }
            };
            for (after_root, new_base) in new_bins {
                let target = match plan
                    .targets
                    .iter_mut()
                    .find(|target| target.base_path == new_base)
                {
                    Some(target) => target,
                    None => {
                        plan.targets
                            .push(crate::recycle_bin::RecycleRelocationTarget {
                                base_path: new_base,
                                media_roots: Vec::new(),
                            });
                        plan.targets.last_mut().expect("target was just pushed")
                    }
                };
                if let Some(after_root) = after_root
                    && !target.media_roots.contains(&after_root)
                {
                    target.media_roots.push(after_root);
                }
            }
        }
        plans
    }
}
