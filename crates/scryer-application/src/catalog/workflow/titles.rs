fn push_optional_mapping_id(
    ids: &mut Vec<(&'static str, String)>,
    source: &'static str,
    value: Option<i64>,
) {
    if let Some(value) = value
        && value > 0
    {
        ids.push((source, value.to_string()));
    }
}
fn non_empty_scope(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn record_download_client_feedback_categories(
    client_id: &str,
    categories: impl IntoIterator<Item = String>,
    admission_by_client: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
    feedback_by_client: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) {
    for category in categories {
        let category = category.trim().to_string();
        if category.is_empty() {
            continue;
        }
        let normalized = crate::services::normalize_download_client_category(&category);
        if normalized.is_empty() {
            continue;
        }
        admission_by_client
            .entry(client_id.to_string())
            .or_default()
            .insert(normalized);
        feedback_by_client
            .entry(client_id.to_string())
            .or_default()
            .insert(category);
    }
}

fn record_configured_download_client_category(
    client_id: &str,
    enabled: bool,
    category: Option<String>,
    admission_by_client: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
    feedback_by_client: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) {
    let Some(category) = category
        .map(|category| category.trim().to_string())
        .filter(|category| !category.is_empty())
    else {
        return;
    };
    let normalized = crate::services::normalize_download_client_category(&category);
    if normalized.is_empty() {
        return;
    }
    admission_by_client
        .entry(client_id.to_string())
        .or_default()
        .insert(normalized);
    if enabled {
        feedback_by_client
            .entry(client_id.to_string())
            .or_default()
            .insert(category);
    }
}

#[cfg(test)]
mod download_client_feedback_category_tests {
    use super::*;

    #[test]
    fn inventory_combines_default_facet_and_library_routes_without_losing_spelling() {
        let mut admission = std::collections::HashMap::new();
        let mut feedback = std::collections::HashMap::new();

        record_download_client_feedback_categories(
            "qbit",
            [" Movies ".to_string()],
            &mut admission,
            &mut feedback,
        );
        record_download_client_feedback_categories(
            "qbit",
            ["TV / Anime".to_string()],
            &mut admission,
            &mut feedback,
        );
        record_download_client_feedback_categories(
            "qbit",
            ["Series-HD".to_string(), "movies".to_string()],
            &mut admission,
            &mut feedback,
        );
        record_download_client_feedback_categories(
            "other",
            ["Other Client".to_string()],
            &mut admission,
            &mut feedback,
        );

        assert_eq!(
            admission["qbit"],
            std::collections::HashSet::from([
                "movies".to_string(),
                "tv / anime".to_string(),
                "series-hd".to_string(),
            ])
        );
        assert_eq!(
            feedback["qbit"],
            std::collections::HashSet::from([
                "Movies".to_string(),
                "movies".to_string(),
                "TV / Anime".to_string(),
                "Series-HD".to_string(),
            ])
        );
        assert_eq!(
            feedback["other"],
            std::collections::HashSet::from(["Other Client".to_string()])
        );
    }

    #[test]
    fn disabled_configured_routes_remain_admissible_but_are_not_polled() {
        let mut admission = std::collections::HashMap::new();
        let mut feedback = std::collections::HashMap::new();

        record_configured_download_client_category(
            "qbit",
            false,
            Some(" Disabled Route ".to_string()),
            &mut admission,
            &mut feedback,
        );
        record_configured_download_client_category(
            "qbit",
            true,
            Some("Enabled Route".to_string()),
            &mut admission,
            &mut feedback,
        );

        assert_eq!(
            admission["qbit"],
            std::collections::HashSet::from([
                "disabled route".to_string(),
                "enabled route".to_string(),
            ])
        );
        assert_eq!(
            feedback["qbit"],
            std::collections::HashSet::from(["Enabled Route".to_string()])
        );
    }
}

impl AppUseCase {
    async fn read_download_client_routing_value(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
    }
}
impl AppUseCase {
    async fn read_explicit_download_client_routing_value(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .read_setting_string_value_explicit(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id),
            )
            .await?
        {
            return Ok(Some(value));
        }

        self.read_setting_string_value_explicit(
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id),
        )
        .await
    }
}
impl AppUseCase {
    /// Returns `Some(entry)` when the persisted JSON has an entry for this
    /// client in this scope, else `None`. Callers are responsible for applying
    /// the canonical default — the explicit fallback site — so the read path
    /// stays a thin lookup over normalized data. Legacy installs converge via
    /// the startup `normalize_routing_settings` pass.
    async fn read_download_client_routing_entry(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Ok(None);
        }

        if let Some(library_id) = library_id.map(str::trim).filter(|value| !value.is_empty())
            && let Some(raw_json) = self
                .read_explicit_download_client_routing_value(library_id)
                .await?
            && let Some(routing_map) = parse_download_client_routing_map(&raw_json)
        {
            if let Some(config) = routing_map.get(client_id) {
                return Ok(Some(parse_download_client_routing_entry(config)));
            }

            let mut disabled_entry = default_download_client_routing_entry();
            disabled_entry.enabled = false;
            return Ok(Some(disabled_entry));
        }

        let scope_id = facet.as_str();

        let Some(raw_json) = self.read_download_client_routing_value(scope_id).await? else {
            return Ok(None);
        };

        let Some(routing_map) = parse_download_client_routing_map(&raw_json) else {
            return Ok(None);
        };

        Ok(routing_map
            .get(client_id)
            .map(parse_download_client_routing_entry))
    }
}
impl AppUseCase {
    #[expect(
        clippy::too_many_arguments,
        reason = "title catalog listing mirrors the user-visible filter, sort, pagination, and projection surface"
    )]
    pub async fn list_titles(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        query: Option<String>,
        filter: crate::TitleCatalogFilter,
        sort: crate::TitleCatalogSort,
        limit: usize,
        offset: usize,
        include_external_ids: bool,
        include_catalog_counts: bool,
    ) -> AppResult<crate::TitleCatalogResult> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.services
            .catalog
            .titles
            .list_for_libraries_catalog(
                facet,
                &library_ids,
                query,
                filter,
                sort,
                limit,
                offset,
                include_external_ids,
                include_catalog_counts,
            )
            .await
    }

    pub async fn title_catalog_filter_options(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        root_folder_ids: Vec<String>,
    ) -> AppResult<crate::TitleCatalogFilterOptions> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }

        self.services
            .catalog
            .titles
            .title_catalog_filter_options(facet, &library_ids, &root_folder_ids)
            .await
    }

    pub async fn list_titles_unpaged(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.services
            .catalog
            .titles
            .list_for_libraries(facet, &library_ids, query)
            .await
    }
}
impl AppUseCase {
    pub async fn list_titles_without_external_ids(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let mut library_ids = self
            .authorized_library_ids(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.services
            .catalog
            .titles
            .list_for_libraries_without_external_ids(facet, &library_ids, query)
            .await
    }
}
impl AppUseCase {
    pub async fn list_titles_by_external_ids(
        &self,
        actor: &User,
        source: &str,
        values: &[String],
    ) -> AppResult<Vec<Title>> {
        let normalized_source = source.trim();
        if normalized_source.is_empty() {
            return Ok(Vec::new());
        }

        let mut seen = HashSet::new();
        let mut normalized_values = Vec::new();
        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                normalized_values.push(trimmed.to_string());
            }
        }

        if normalized_values.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self
            .services
            .catalog
            .titles
            .list_by_external_ids(normalized_source, &normalized_values)
            .await?;
        let library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.contains(&title.library_id))
            .collect())
    }
}
impl AppUseCase {
    pub async fn list_cutoff_unmet_titles(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<CutoffUnmetItem>> {
        let authorized_libraries = self
            .list_libraries_for_permission(
                actor,
                facet.clone(),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut library_ids = authorized_libraries
            .iter()
            .map(|library| library.id.clone())
            .collect::<Vec<_>>();
        let requested_library_ids = requested_library_ids
            .as_ref()
            .map(|requested| {
                requested
                    .iter()
                    .map(|library_id| library_id.trim())
                    .filter(|library_id| !library_id.is_empty())
                    .map(str::to_owned)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        if !requested_library_ids.is_empty() {
            library_ids.retain(|library_id| requested_library_ids.contains(library_id));
        }
        self.compute_cutoff_unmet_items(facet, Some(library_ids))
            .await
    }

    /// Actor-less core of the cutoff-unmet derivation: scopes
    /// whose primary file sits strictly below the effective profile cutoff.
    /// The convergence cursor derives upgrade targets from every library
    /// (`library_filter: None`); the API path passes the actor's authorized
    /// subset.
    /// Every monitored title in scope, paired with the quality profile that
    /// governs it.
    ///
    /// Profile resolution is a four-level fallback (title tag, category
    /// override, global setting, built-in default) plus a name-equality retry;
    /// the two cutoff sweeps both need it and must not answer differently.
    pub(crate) async fn monitored_titles_with_profiles(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
    ) -> AppResult<Vec<(scryer_domain::Title, QualityProfile)>> {
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(facet, library_ids, None)
            .await?;
        let monitored: Vec<scryer_domain::Title> =
            titles.into_iter().filter(|title| title.monitored).collect();
        if monitored.is_empty() {
            return Ok(Vec::new());
        }

        let profile_settings = self.load_quality_profile_settings().await?;
        let global_profile_id = Some(profile_settings.global_profile_id.as_str());
        let profile_map: HashMap<&str, &QualityProfile> = profile_settings
            .profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile))
            .collect();
        let default_profile = crate::builtin_default_quality_profile();

        Ok(monitored
            .into_iter()
            .map(|title| {
                let title_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:")
                    .map(str::trim)
                    .filter(|value| {
                        !value.is_empty() && *value != crate::QUALITY_PROFILE_INHERIT_VALUE
                    });
                let category_profile_id = profile_settings
                    .category_selections
                    .iter()
                    .find(|selection| selection.facet == title.facet)
                    .and_then(|selection| selection.override_profile_id.as_deref());
                let resolved_profile_id = crate::resolve_profile_id_for_title(
                    title_profile_id,
                    None,
                    category_profile_id,
                    global_profile_id,
                );
                let profile = resolved_profile_id
                    .as_deref()
                    .and_then(|profile_id| {
                        profile_map.get(profile_id).copied().or_else(|| {
                            profile_settings.profiles.iter().find(|profile| {
                                crate::settings::runtime::quality_profile_ids_equal(
                                    &profile.id,
                                    profile_id,
                                )
                            })
                        })
                    })
                    .unwrap_or(&default_profile)
                    .clone();
                (title, profile)
            })
            .collect())
    }

    pub(crate) async fn compute_cutoff_unmet_items(
        &self,
        facet: Option<MediaFacet>,
        library_filter: Option<Vec<String>>,
    ) -> AppResult<Vec<CutoffUnmetItem>> {
        let mut libraries = self.services.catalog.libraries.list(facet.clone()).await?;
        if let Some(filter) = library_filter {
            let allowed: HashSet<String> = filter.into_iter().collect();
            libraries.retain(|library| allowed.contains(&library.id));
        }
        let library_name_by_id = libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<HashMap<_, _>>();
        let library_slug_by_id = libraries
            .iter()
            .map(|library| (library.id.clone(), library.slug.clone()))
            .collect::<HashMap<_, _>>();
        let library_ids = libraries
            .iter()
            .map(|library| library.id.clone())
            .collect::<Vec<_>>();
        let monitored_titles = self
            .monitored_titles_with_profiles(facet, &library_ids)
            .await?;
        if monitored_titles.is_empty() {
            return Ok(Vec::new());
        }

        let title_ids = monitored_titles
            .iter()
            .map(|(title, _)| title.id.clone())
            .collect::<Vec<_>>();
        let quality_summaries = self
            .services
            .library
            .media_files
            .list_cutoff_unmet_quality_summaries(&title_ids)
            .await?;

        let mut title_map = HashMap::new();
        let mut cutoff_profile_map = HashMap::new();
        for (title, profile) in monitored_titles {
            if !profile.criteria.allow_upgrades {
                continue;
            }

            let Some(cutoff_tier) = profile
                .criteria
                .cutoff_tier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };

            let Some(normalized_cutoff_tier) =
                crate::quality_profile::normalize_quality_tier(Some(cutoff_tier))
            else {
                continue;
            };

            if !profile
                .criteria
                .quality_tiers
                .iter()
                .any(|tier| tier == &normalized_cutoff_tier)
            {
                continue;
            }

            cutoff_profile_map.insert(
                title.id.clone(),
                (
                    profile.criteria.quality_tiers.clone(),
                    normalized_cutoff_tier,
                ),
            );
            title_map.insert(title.id.clone(), title);
        }

        let mut items = Vec::new();
        for summary in quality_summaries {
            let Some(title) = title_map.get(summary.title_id.as_str()) else {
                continue;
            };
            let Some((quality_tiers, normalized_cutoff_tier)) =
                cutoff_profile_map.get(summary.title_id.as_str())
            else {
                continue;
            };

            if summary.episode_id.is_none() && title.facet != MediaFacet::Movie {
                continue;
            }

            let Some(normalized_current_tier) =
                crate::quality_profile::normalize_quality_tier(Some(summary.quality_tier.as_str()))
            else {
                continue;
            };

            if !quality_tiers
                .iter()
                .any(|tier| tier == &normalized_current_tier)
            {
                continue;
            }

            if crate::quality_profile::quality_meets_or_exceeds_cutoff(
                normalized_current_tier.as_str(),
                normalized_cutoff_tier.as_str(),
                quality_tiers,
            ) {
                continue;
            }

            items.push(CutoffUnmetItem {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                title_slug: title.slug.clone(),
                title_facet: title.facet.clone(),
                library_id: title.library_id.clone(),
                library_name: library_name_by_id.get(&title.library_id).cloned(),
                library_slug: library_slug_by_id.get(&title.library_id).cloned(),
                episode_id: summary.episode_id,
                season_number: summary.season_number,
                episode_number: summary.episode_number,
                current_tier: normalized_current_tier,
                target_tier: normalized_cutoff_tier.clone(),
            });
        }

        fn parse_episode_sort_number(value: Option<&str>) -> i64 {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|value| {
                    let digits = value
                        .chars()
                        .filter(|ch| ch.is_ascii_digit())
                        .collect::<String>();
                    if digits.is_empty() {
                        None
                    } else {
                        digits.parse::<i64>().ok()
                    }
                })
                .unwrap_or(i64::MAX)
        }

        items.sort_by(|left, right| {
            left.title_name
                .to_ascii_lowercase()
                .cmp(&right.title_name.to_ascii_lowercase())
                .then_with(|| {
                    parse_episode_sort_number(left.season_number.as_deref())
                        .cmp(&parse_episode_sort_number(right.season_number.as_deref()))
                })
                .then_with(|| {
                    parse_episode_sort_number(left.episode_number.as_deref())
                        .cmp(&parse_episode_sort_number(right.episode_number.as_deref()))
                })
        });

        Ok(items)
    }
}
impl AppUseCase {
    /// Bounded view: one page of cutoff-unmet targets plus the full
    /// count. Computes the unmet set then slices — this bounds what reaches the
    /// browser (the immediate large-library pain); paging the server-side
    /// compute is a follow-up. `limit == 0` returns just the total with no items.
    pub async fn list_cutoff_unmet_titles_page(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> AppResult<CutoffUnmetPage> {
        let all = self
            .list_cutoff_unmet_titles(actor, facet, requested_library_ids)
            .await?;
        let total = all.len();
        let items = all.into_iter().skip(offset).take(limit).collect();
        Ok(CutoffUnmetPage { items, total })
    }

    /// One page of cutoff-unmet targets plus per-item convergence progress. The page's convergence is derived in one batched coverage round-trip,
    /// so the Upgrades table shows the same convergence state as the derived
    /// Missing/Upgrades views.
    pub async fn list_cutoff_unmet_titles_page_with_convergence(
        &self,
        actor: &User,
        facet: Option<MediaFacet>,
        requested_library_ids: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<(CutoffUnmetItem, crate::WantedViewConvergence)>, usize)> {
        let page = self
            .list_cutoff_unmet_titles_page(actor, facet, requested_library_ids, limit, offset)
            .await?;
        let total = page.total;

        let scopes: Vec<(String, String)> = page
            .items
            .iter()
            .filter_map(|item| {
                let scope = crate::contracts::SubmissionScope::from_persisted(
                    &item.title_id,
                    item.episode_id.clone(),
                    None,
                    None,
                    None,
                );
                crate::acquisition::convergence::convergence_scope_key(&scope, &item.title_id)
                    .map(|key| (item.title_id.clone(), key))
            })
            .collect();
        let convergence = self.page_convergence_by_scope_key(&scopes).await;

        let items = page
            .items
            .into_iter()
            .map(|item| {
                let scope = crate::contracts::SubmissionScope::from_persisted(
                    &item.title_id,
                    item.episode_id.clone(),
                    None,
                    None,
                    None,
                );
                let convergence =
                    crate::acquisition::convergence::convergence_scope_key(&scope, &item.title_id)
                        .and_then(|key| convergence.get(&key).copied())
                        .unwrap_or(crate::WantedViewConvergence {
                            state: crate::WantedConvergenceState::Converged,
                            indexers_covered: 0,
                            indexers_routed: 0,
                        });
                (item, convergence)
            })
            .collect();
        Ok((items, total))
    }
}
impl AppUseCase {
    pub(crate) async fn default_media_root_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<String> {
        let handler = self.facet_registry.get(&title.facet);
        let default_path = handler.map(|h| h.default_library_path()).unwrap_or("/data");
        let root_folders = self
            .root_folders_for_library(&title.library_id, &title.facet)
            .await?;

        Ok(root_folders
            .iter()
            .find(|entry| entry.is_default)
            .or_else(|| root_folders.first())
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| default_path.to_string()))
    }
}
impl AppUseCase {
    pub async fn add_title_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
    ) -> AppResult<AddTitleOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.add_title_with_outcome_in_library(actor, request, library_id)
            .await
    }
}
impl AppUseCase {
    pub async fn add_title_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
    ) -> AppResult<AddTitleOutcome> {
        self.add_title_with_options_patch_outcome_in_library(
            actor,
            request,
            library_id,
            TitleOptionsPatch::default(),
        )
        .await
    }

    pub async fn add_title_with_options_patch_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<AddTitleOutcome> {
        let created = self
            .create_title_without_hydration_with_options_patch_in_library(
                actor,
                request,
                library_id,
                options_patch,
            )
            .await?;
        self.finish_add_title_with_outcome(created).await
    }

    pub(crate) async fn add_title_and_bind_pending_import_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        pending_import_id: &str,
    ) -> AppResult<AddTitleOutcome> {
        let created = self
            .create_title_without_hydration_and_bind_pending_import_in_library(
                actor,
                request,
                library_id,
                pending_import_id,
            )
            .await?;
        if created.reused_existing {
            return Ok(AddTitleOutcome {
                title: created.title,
                metadata_hydration_state: AddTitleHydrationState::NotRequired,
                reused_existing_title: true,
            });
        }
        self.finish_add_title_with_outcome(created).await
    }

    pub(crate) async fn add_title_with_options_patch_outcome_after_library_authorization(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<AddTitleOutcome> {
        let created = self
            .create_title_without_hydration_with_options_patch_after_library_authorization(
                actor,
                request,
                library_id,
                options_patch,
            )
            .await?;
        self.finish_add_title_with_outcome(created).await
    }

    pub(crate) async fn add_title_with_options_patch_outcome_after_library_authorization_profile_lock_held(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        options_patch: TitleOptionsPatch,
    ) -> AppResult<AddTitleOutcome> {
        let created = self
            .create_title_without_hydration_with_options_patch_after_library_authorization_lock_held(
                actor,
                request,
                library_id,
                options_patch,
            )
            .await?;
        self.finish_add_title_with_outcome(created).await
    }

    async fn finish_add_title_with_outcome(
        &self,
        created: CreateTitleOutcome,
    ) -> AppResult<AddTitleOutcome> {
        self.notify_title_image_wakes(&created.title);

        let metadata_hydration_state = if created.title.metadata_fetched_at.is_some() {
            AddTitleHydrationState::Complete
        } else if matches!(created.title.facet, MediaFacet::Movie)
            .then(|| movie_title_ref(&created.title).is_some())
            .unwrap_or_else(|| extract_tvdb_id(&created.title).is_some())
        {
            if created.reused_existing {
                self.services
                    .catalog
                    .titles
                    .mark_title_metadata_hydration_due_now(&created.title.id)
                    .await?;
            }
            self.runtime.catalog.title_hydration_wake.notify_one();
            AddTitleHydrationState::Pending
        } else {
            self.services
                .catalog
                .titles
                .clear_title_metadata_hydration_retry_state(&created.title.id)
                .await?;
            AddTitleHydrationState::NotRequired
        };

        Ok(AddTitleOutcome {
            title: created.title,
            metadata_hydration_state,
            reused_existing_title: created.reused_existing,
        })
    }
}
impl AppUseCase {
    pub async fn add_title(&self, actor: &User, request: NewTitle) -> AppResult<Title> {
        Ok(self.add_title_with_outcome(actor, request).await?.title)
    }
}
impl AppUseCase {
    fn notify_title_image_wakes(&self, title: &Title) {
        if title
            .poster_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.poster_wake.notify_one();
        }
        if title
            .background_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.runtime.catalog.fanart_wake.notify_one();
        }
    }
}
impl AppUseCase {
    pub(crate) async fn find_blocking_download_submissions(
        &self,
        title: &Title,
        scope: &SubmissionScope,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if submissions.is_empty() {
            return Ok(Vec::new());
        }

        let snapshot = self
            .services
            .integrations
            .download_client
            .list_snapshot_outcome_excluding_client_types(100, &[])
            .await?;

        self.find_blocking_download_submissions_from_snapshot(title, scope, &submissions, &snapshot)
            .await
    }

    pub(crate) async fn find_blocking_download_submissions_from_snapshot(
        &self,
        title: &Title,
        scope: &SubmissionScope,
        submissions: &[DownloadSubmission],
        snapshot: &DownloadClientSnapshotOutcome,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        if submissions.is_empty() {
            return Ok(Vec::new());
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;

        Self::find_blocking_download_submissions_in_state(
            title,
            scope,
            submissions,
            snapshot,
            &episodes,
            &HashSet::new(),
        )
    }

    pub(crate) fn find_blocking_download_submissions_in_state(
        title: &Title,
        scope: &SubmissionScope,
        submissions: &[DownloadSubmission],
        snapshot: &DownloadClientSnapshotOutcome,
        episodes: &[scryer_domain::Episode],
        accepted_download_ids: &HashSet<scryer_domain::download_identity::DownloadId>,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        if submissions.is_empty() {
            return Ok(Vec::new());
        }

        let mut conflicts = Vec::new();
        for submission in submissions {
            if !submission_scopes_overlap(&title.id, &submission.scope, scope, episodes) {
                continue;
            }

            if accepted_download_ids.contains(&submission.download_id) {
                conflicts.push(SubmissionScopeConflict {
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    download_client_id: submission.download_client_id.clone(),
                    download_client_type: submission.download_client_type.clone(),
                    download_client_item_id: submission.download_client_item_id.clone(),
                    source_title: submission.source_title.clone(),
                    source_kind: submission.source_kind,
                    scope: submission.scope.clone(),
                    state: Some(DownloadQueueState::Queued),
                    replaceable: false,
                });
                continue;
            }

            let queue_item = snapshot
                .items
                .iter()
                .find(|item| queue_item_matches_submission(item, submission));
            let authoritative = submission
                .download_client_id
                .as_deref()
                .map(str::trim)
                .filter(|client_id| !client_id.is_empty())
                .is_some_and(|client_id| snapshot.authoritative_client_ids.contains(client_id));
            let Some(queue_item) = queue_item else {
                if !authoritative {
                    return Err(AppError::DownloadSubmitUnavailable(format!(
                        "download client state is unavailable for submission {} on title {}",
                        submission.download_id, title.id
                    )));
                }
                continue;
            };
            if !queue_state_blocks_submission(queue_item.state) {
                if !authoritative {
                    return Err(AppError::DownloadSubmitUnavailable(format!(
                        "download client state is unavailable for terminal submission {} on title {}",
                        submission.download_id, title.id
                    )));
                }
                continue;
            }

            conflicts.push(SubmissionScopeConflict {
                title_id: title.id.clone(),
                title_name: title.name.clone(),
                download_client_id: submission.download_client_id.clone(),
                download_client_type: submission.download_client_type.clone(),
                download_client_item_id: submission.download_client_item_id.clone(),
                source_title: submission.source_title.clone(),
                source_kind: submission.source_kind,
                scope: submission.scope.clone(),
                state: Some(queue_item.state),
                replaceable: queue_state_is_replaceable(queue_item.state),
            });
        }

        Ok(conflicts)
    }
}
impl AppUseCase {
    pub(crate) async fn replace_blocking_download_submission(
        &self,
        conflict: &SubmissionScopeConflict,
    ) -> AppResult<()> {
        if !conflict.replaceable {
            return Err(AppError::Validation(
                "the existing download is no longer safe to replace".into(),
            ));
        }

        if let Some(client_id) = conflict.download_client_id.as_deref() {
            self.services
                .integrations
                .download_client
                .delete_queue_item_for_client_id(
                    client_id,
                    &conflict.download_client_item_id,
                    false,
                    false,
                )
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .delete_queue_item_for_client(
                    &conflict.download_client_type,
                    &conflict.download_client_item_id,
                    false,
                    false,
                )
                .await?;
        }

        self.services
            .workflow
            .download_submissions
            .delete_by_client_item_id(&ClientJobLocator::new(
                conflict.download_client_id.as_deref(),
                &conflict.download_client_type,
                &conflict.download_client_item_id,
            ))
            .await?;
        self.reset_wanted_items_for_submission_scope(&conflict.title_id, &conflict.scope)
            .await?;

        Ok(())
    }
}
impl AppUseCase {
    pub(crate) async fn replace_blocking_download_submissions(
        &self,
        conflicts: &[SubmissionScopeConflict],
    ) -> AppResult<()> {
        for conflict in conflicts {
            self.replace_blocking_download_submission(conflict).await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    /// Resolve the category Scryer would submit with for this title/client.
    pub(crate) async fn effective_download_client_category_for_title(
        &self,
        title: &Title,
        client_id: &str,
    ) -> AppResult<Option<String>> {
        self.effective_download_client_category_for_scope(
            Some(&title.library_id),
            &title.facet,
            client_id,
        )
        .await
    }

    async fn effective_download_client_category_for_scope(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(entry) = self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await?
        {
            if !entry.enabled {
                return Ok(None);
            }
            if let Some(category) = entry
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Ok(Some(category.to_string()));
            }
        }
        Ok(Some(self.derive_download_category(facet).await))
    }

    async fn effective_download_client_category_for_admission_scope(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(entry) = self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await?
        {
            if !entry.enabled {
                return Ok(None);
            }
            if let Some(category) = entry
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Ok(Some(category.to_string()));
            }
        }
        Ok(Some(
            self.derive_download_category_for_admission(facet).await?,
        ))
    }

    pub(crate) async fn download_client_category_admission_snapshot(
        &self,
    ) -> Option<std::sync::Arc<crate::services::DownloadClientCategoryAdmissionSnapshot>> {
        if let Some(snapshot) = self
            .runtime
            .acquisition
            .download_client_category_admission
            .snapshot()
            .await
        {
            return Some(snapshot);
        }
        if let Err(error) = self.refresh_download_client_category_admission().await {
            tracing::warn!(
                error = %error,
                "download-client category admission is not ready; deferring untracked observations"
            );
        }
        self.runtime
            .acquisition
            .download_client_category_admission
            .snapshot()
            .await
    }

    pub async fn refresh_download_client_category_admission(&self) -> AppResult<()> {
        let snapshot = self
            .load_download_client_category_admission_snapshot()
            .await?;
        self.runtime
            .acquisition
            .download_client_category_admission
            .replace(snapshot)
            .await;
        Ok(())
    }

    pub(crate) async fn refresh_download_client_category_admission_best_effort(&self) {
        if let Err(error) = self.refresh_download_client_category_admission().await {
            tracing::warn!(
                error = %error,
                "failed to refresh download-client category admission; retaining last known-good snapshot"
            );
        }
    }

    async fn load_download_client_category_admission_snapshot(
        &self,
    ) -> AppResult<crate::services::DownloadClientCategoryAdmissionSnapshot> {
        let mut default_categories = std::collections::HashSet::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let category = self.derive_download_category_for_admission(&facet).await?;
            let category = crate::services::normalize_download_client_category(&category);
            if !category.is_empty() {
                default_categories.insert(category);
            }
        }

        let clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?;
        let mut categories_by_client = std::collections::HashMap::new();
        let mut feedback_category_sets_by_client = std::collections::HashMap::new();
        for client in clients {
            let feedback_categories = self
                .load_download_client_feedback_categories(&client.id)
                .await?;
            record_download_client_feedback_categories(
                &client.id,
                feedback_categories,
                &mut categories_by_client,
                &mut feedback_category_sets_by_client,
            );
        }

        let mut routing_scopes = std::collections::HashSet::from([
            MediaFacet::Movie.as_str().to_string(),
            MediaFacet::Series.as_str().to_string(),
            MediaFacet::Anime.as_str().to_string(),
        ]);
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            for library in self.services.catalog.libraries.list(Some(facet)).await? {
                routing_scopes.insert(library.id);
            }
        }
        for scope_id in routing_scopes {
            self.collect_configured_download_client_categories(
                &scope_id,
                &mut categories_by_client,
                &mut feedback_category_sets_by_client,
            )
            .await?;
        }

        let feedback_categories_by_client = feedback_category_sets_by_client
            .into_iter()
            .map(|(client_id, categories)| {
                let mut categories = categories.into_iter().collect::<Vec<_>>();
                categories.sort();
                (client_id, categories)
            })
            .collect();

        Ok(crate::services::DownloadClientCategoryAdmissionSnapshot {
            default_categories,
            categories_by_client,
            feedback_categories_by_client,
        })
    }

    async fn collect_configured_download_client_categories(
        &self,
        scope_id: &str,
        categories_by_client: &mut std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        >,
        feedback_categories_by_client: &mut std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        >,
    ) -> AppResult<()> {
        for key in [
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
        ] {
            let Some(raw_json) = self
                .read_setting_string_value_explicit(key, Some(scope_id))
                .await?
            else {
                continue;
            };
            let Some(routing_map) = parse_download_client_routing_map(&raw_json) else {
                continue;
            };
            for (client_id, config) in routing_map {
                let entry = parse_download_client_routing_entry(&config);
                record_configured_download_client_category(
                    &client_id,
                    entry.enabled,
                    entry.category,
                    categories_by_client,
                    feedback_categories_by_client,
                );
            }
        }
        Ok(())
    }

    async fn load_download_client_feedback_categories(
        &self,
        client_id: &str,
    ) -> AppResult<std::collections::HashSet<String>> {
        let mut categories = std::collections::HashSet::new();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let libraries = self
                .services
                .catalog
                .libraries
                .list(Some(facet.clone()))
                .await?;
            if let Some(category) = self
                .effective_download_client_category_for_admission_scope(None, &facet, client_id)
                .await?
            {
                categories.insert(category.trim().to_string());
            }
            for library in libraries {
                if let Some(category) = self
                    .effective_download_client_category_for_admission_scope(
                        Some(&library.id),
                        &facet,
                        client_id,
                    )
                    .await?
                {
                    categories.insert(category.trim().to_string());
                }
            }
        }
        categories.retain(|category| !category.trim().is_empty());
        Ok(categories)
    }

    /// Resolve the per-facet fallback category used when the selected client
    /// does not declare an explicit routing category.
    pub(crate) async fn derive_download_category(&self, facet: &MediaFacet) -> String {
        let scope_id = facet.as_str();

        if let Ok(Some(configured)) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        if let Ok(Some(configured)) = self
            .read_setting_string_value(LEGACY_NZBGET_CATEGORY_SETTING_KEY, Some(scope_id))
            .await
        {
            let trimmed = configured.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }

        self.facet_registry
            .get(facet)
            .map(|h| h.download_category().to_string())
            .unwrap_or_else(|| "other".to_string())
    }

    async fn derive_download_category_for_admission(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<String> {
        let scope_id = facet.as_str();
        if let Some(configured) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, Some(scope_id))
            .await?
        {
            let trimmed = configured.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        if let Some(configured) = self
            .read_setting_string_value(LEGACY_NZBGET_CATEGORY_SETTING_KEY, Some(scope_id))
            .await?
        {
            let trimmed = configured.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        Ok(self
            .facet_registry
            .get(facet)
            .map(|handler| handler.download_category().to_string())
            .unwrap_or_else(|| "other".to_string()))
    }
}
impl AppUseCase {
    /// Canonical owner for the "this title should be actionable right now"
    /// orchestration. The title's missing scopes are already in the derived
    /// target set (and hot — it was just added), so acting immediately only
    /// requires waking the convergence cycle.
    async fn sync_title_for_immediate_acquisition(&self, title: &Title) {
        if !title.monitored {
            return;
        }
        self.runtime.acquisition.acquisition_wake.notify_one();
    }
}
impl AppUseCase {
    pub(crate) async fn purge_title_logical_dependents(
        &self,
        title: &scryer_domain::Title,
        purge_recycle_bin_entries: bool,
        actor: impl Into<DomainEventActor>,
    ) -> AppResult<()> {
        let actor = actor.into();
        let title_id = title.id.as_str();

        if purge_recycle_bin_entries {
            let media_roots = match self.all_library_root_folders_for_facet(&title.facet).await {
                Ok(roots) => roots
                    .into_iter()
                    .filter(|root| root.library_id == title.library_id)
                    .map(|root| root.path)
                    .collect::<Vec<_>>(),
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title_id,
                        "failed to resolve recycle roots for deleted title"
                    );
                    Vec::new()
                }
            };
            let configs = self.recycle_bin_configs_for_media_roots(media_roots).await;
            let mut purged = 0u32;
            for (media_root, config) in configs {
                match crate::recycle_bin::list_committed_entries(&config).await {
                    Ok(entries) => {
                        for entry in entries {
                            if entry.manifest.title_id.as_deref() != Some(title_id) {
                                continue;
                            }
                            match self
                                .purge_recycle_entry_after_validation(
                                    &media_root,
                                    &config,
                                    &entry,
                                    actor.clone(),
                                )
                                .await
                            {
                                Ok(true) => {
                                    purged += 1;
                                }
                                Ok(false) => {}
                                Err(error) => warn!(
                                    error = %error,
                                    title_id = %title_id,
                                    "failed to purge recycle entry for deleted title"
                                ),
                            }
                        }
                    }
                    Err(e) => warn!(
                        error = %e,
                        title_id = %title_id,
                        "failed to list recycle entries for deleted title"
                    ),
                }
            }
            if purged > 0 {
                info!(
                    purged,
                    title_id = %title_id,
                    "purged recycle bin entries for deleted title"
                );
            }
        }

        self.purge_title_dependent_records(title_id, actor).await
    }

    /// The half of [`Self::purge_title_logical_dependents`] that needs nothing
    /// but the title's id: the rows no foreign key reaches, and the downloads
    /// that would otherwise keep running for a title that is gone.
    ///
    /// Split out so a merged source title — whose `titles` row the merge
    /// transaction already removed, and whose files were repointed rather than
    /// deleted — retires through exactly the same cleanup.
    pub(crate) async fn purge_title_dependent_records(
        &self,
        title_id: &str,
        actor: DomainEventActor,
    ) -> AppResult<()> {
        let download_submissions = match self
            .services
            .workflow
            .download_submissions
            .list_for_title(title_id)
            .await
        {
            Ok(submissions) => submissions,
            Err(err) => {
                warn!(
                    title_id = %title_id,
                    error = %err,
                    "failed to list download submissions while deleting title; skipping download cancellation"
                );
                Vec::new()
            }
        };

        let mut seen_downloads = HashSet::new();
        for submission in download_submissions {
            let identity = ClientJobLocator::from_submission(&submission);
            if !seen_downloads.insert(identity.clone()) {
                continue;
            }

            let tracked_state = self
                .services
                .workflow
                .download_submissions
                .get_tracked_state(&identity)
                .await
                .ok()
                .flatten();
            if !tracked_download_state_is_active(tracked_state.as_deref()) {
                debug!(
                    title_id = %title_id,
                    client_type = %identity.client_type,
                    download_item_id = %identity.item_id,
                    tracked_state = tracked_state.as_deref().unwrap_or("none"),
                    "skipping recorded download during title deletion because it is not active"
                );
                continue;
            }

            match self
                .services
                .workflow
                .download_queue_commands
                .queue_delete_command(
                    identity.client_id.as_deref(),
                    &identity.client_type,
                    &identity.item_id,
                    false,
                    actor.user_id.as_deref(),
                )
                .await
            {
                Ok(_) => debug!(
                    title_id = %title_id,
                    client_type = %identity.client_type,
                    download_item_id = %identity.item_id,
                    "queued targeted download cancellation for deleted title"
                ),
                Err(err) => warn!(
                    title_id = %title_id,
                    client_type = %identity.client_type,
                    download_item_id = %identity.item_id,
                    error = %err,
                    "failed to queue targeted download cancellation while deleting title"
                ),
            }
        }

        self.services
            .workflow
            .pending_releases
            .delete_pending_releases_for_title(title_id)
            .await?;
        self.services
            .workflow
            .acquisition_scope_states
            .delete_acquisition_scope_states_for_title(title_id)
            .await?;
        self.services
            .workflow
            .download_submissions
            .delete_for_title(title_id)
            .await?;
        self.services
            .workflow
            .blocklist_repo
            .delete_for_title(title_id)
            .await?;
        self.services
            .library
            .library_probe_signatures
            .delete_probe_signatures_for_title_ids(&[title_id.to_string()])
            .await?;

        Ok(())
    }
}
impl AppUseCase {
    pub async fn get_title(&self, actor: &User, id: &str) -> AppResult<Option<Title>> {
        let title = self.services.catalog.titles.get_by_id(id).await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }
}

impl AppUseCase {
    /// Batch variant of [`Self::get_title`]. Loads titles by id in one query and
    /// silently drops ids the actor cannot `View` (missing/forbidden ids are
    /// simply absent from the result), matching dataloader lookup semantics.
    pub async fn get_titles_by_ids(&self, actor: &User, ids: &[String]) -> AppResult<Vec<Title>> {
        self.get_titles_by_ids_with_permission(actor, ids, scryer_domain::LibraryPermission::View)
            .await
    }

    /// Batch variant of [`Self::get_title_for_management`]. Silently drops ids the
    /// actor cannot manage rather than erroring.
    pub async fn get_titles_by_ids_for_management(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<Title>> {
        self.get_titles_by_ids_with_permission(
            actor,
            ids,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await
    }

    async fn get_titles_by_ids_with_permission(
        &self,
        actor: &User,
        ids: &[String],
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<Title>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .services
            .catalog
            .titles
            .get_by_ids(ids)
            .await?
            .into_iter()
            .filter(|title| allowed_library_ids.contains(&title.library_id))
            .collect())
    }
}

fn tracked_download_state_is_active(state: Option<&str>) -> bool {
    matches!(
        state.map(str::trim),
        Some("downloading" | "import_pending" | "importing")
    )
}

impl AppUseCase {
    pub async fn get_title_without_external_ids(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<Title>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id_without_external_ids(id)
            .await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }
}
impl AppUseCase {
    pub async fn get_title_by_slug(
        &self,
        actor: &User,
        facet: MediaFacet,
        library_id: Option<String>,
        library_slug: Option<String>,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let mut authorized_libraries = self
            .list_libraries_for_permission(
                actor,
                Some(facet.clone()),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        if let Some(requested_library_id) = library_id {
            authorized_libraries.retain(|library| library.id == requested_library_id);
        }
        if let Some(requested_library_slug) = library_slug {
            let normalized_slug = requested_library_slug.trim();
            if normalized_slug
                .eq_ignore_ascii_case(scryer_domain::default_library_slug_for_facet(&facet))
            {
                authorized_libraries.retain(|library| library.is_default);
            } else {
                authorized_libraries
                    .retain(|library| library.slug.eq_ignore_ascii_case(normalized_slug));
            }
        }
        let library_ids = authorized_libraries
            .into_iter()
            .map(|library| library.id)
            .collect::<Vec<_>>();
        let title = self
            .services
            .catalog
            .titles
            .get_by_facet_libraries_and_slug(facet, &library_ids, slug)
            .await?;
        if let Some(title) = title.as_ref() {
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(title)
    }
}
