pub(crate) async fn has_enabled_download_clients(app: &AppUseCase) -> bool {
    app.services
        .integrations
        .download_client_configs
        .list(None)
        .await
        .map(|configs| configs.into_iter().any(|config| config.is_enabled))
        .unwrap_or(false)
}
impl AppUseCase {
    pub async fn get_wanted_item(&self, actor: &User, id: &str) -> AppResult<Option<AcquisitionScopeState>> {
        let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(id)
            .await?
        else {
            return Ok(None);
        };

        let library_id = match item.library_id.clone() {
            Some(library_id) => library_id,
            None => self
                .services
                .catalog
                .titles
                .get_by_id(&item.title_id)
                .await?
                .map(|title| title.library_id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?,
        };
        self.require_library_permission(actor, &library_id, scryer_domain::LibraryPermission::View)
            .await?;
        Ok(Some(item))
    }

    /// Batch variant of [`Self::get_wanted_item`]: loads wanted items by id and
    /// silently drops those the actor cannot `View`.
    pub async fn get_wanted_items_by_ids(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_by_ids(ids)
            .await?;
        self.filter_wanted_items_for_permission(
            actor,
            items,
            scryer_domain::LibraryPermission::View,
        )
        .await
    }
}
impl AppUseCase {
    pub async fn list_acquisition_scope_states(
        &self,
        actor: &User,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<(Vec<AcquisitionScopeState>, i64)> {
        let requested_library_ids = query.library_ids.clone();
        let mut library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if !requested_library_ids.is_empty() {
            let authorized = library_ids.into_iter().collect::<HashSet<_>>();
            library_ids = requested_library_ids
                .into_iter()
                .filter(|library_id| authorized.contains(library_id))
                .collect();
        }
        let (mut items, total) = self
            .list_wanted_items_for_libraries(query, library_ids)
            .await?;
        let mut rows: Vec<&mut AcquisitionScopeState> = items.iter_mut().collect();
        self.decorate_landed_bars(&mut rows).await;
        Ok((items, total))
    }

    /// Fill in each scope's landed bar from the file occupying it.
    ///
    /// The bar is a fact about the library, so it is resolved on read rather
    /// than carried on the scope row. The column that used to hold it only held
    /// a landed score in one of its five lifecycle states — after a rejected
    /// import it held the score of a release that never landed, which read
    /// *lower* than the incumbent and quietly dropped the bar.
    ///
    /// The number is the **re-derived canonical bar** — the same one admission
    /// compares against and the decision log records — not the persisted
    /// `media_files.acquisition_score`. A stored score is only valid while the
    /// profile, persona, rule packs and algorithm that produced it are all
    /// unchanged, so showing it would put a second, older-scale "current score"
    /// in the UI beside the one every gate actually uses.
    ///
    /// Batched per page: one media-file query, then one profile + scoring
    /// context per distinct title however many scopes that title contributes.
    /// Only the files that actually occupy one of the requested scopes are
    /// scored — a 200-episode series contributing one scope to the page must
    /// not cost 200 parses and term pipelines.
    /// Failures leave the bars unset rather than failing the listing: this is
    /// display, and a missing number beats a blank page.
    pub(crate) async fn landed_bars_for_scopes(
        &self,
        scopes: &[LandedBarScope],
    ) -> Vec<Option<i32>> {
        let mut bars = vec![None; scopes.len()];
        if scopes.is_empty() {
            return bars;
        }

        let mut title_ids: Vec<String> = scopes.iter().map(|scope| scope.title_id.clone()).collect();
        title_ids.sort();
        title_ids.dedup();

        let rows = match self
            .services
            .library
            .media_files
            .list_media_files_for_titles(&title_ids)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to resolve landed bars for acquisition scopes"
                );
                return bars;
            }
        };
        if rows.is_empty() {
            return bars;
        }

        // **One entry per file, carrying its whole episode span.** The listing
        // joins the file-episode table, so a two-episode file arrives as two
        // rows with a scalar `episode_id` each. Scoring those rows independently
        // measured a 48-minute file against one 24-minute episode — twice the
        // expected size, a `size_massive` band, and a displayed `currentScore`
        // that was not the number the gate compares against (D10 says it is the
        // re-derived bar, and `admission_subject_for_scope` re-derives it over
        // the file's full span).
        let files = LandedBarFile::group(rows);

        // One catalog read per title, shared by the runtime basis and by
        // collection membership. Resolving members with
        // `list_episodes_for_collection` per collection made a 50-row page
        // across 50 seasons 50 extra round-trips for a display number.
        let mut episodes_by_title: HashMap<&str, Vec<scryer_domain::Episode>> = HashMap::new();
        let needs_episodes: HashSet<&str> = scopes
            .iter()
            .filter(|scope| {
                LandedBarScope::non_empty(&scope.episode_id).is_some()
                    || LandedBarScope::non_empty(&scope.collection_id).is_some()
            })
            .map(|scope| scope.title_id.as_str())
            .collect();
        for title_id in needs_episodes {
            episodes_by_title.insert(
                title_id,
                self.services
                    .catalog
                    .shows
                    .list_episodes_for_title(title_id)
                    .await
                    .unwrap_or_default(),
            );
        }
        let collection_members = |title_id: &str, collection_id: &str| -> Vec<&str> {
            episodes_by_title
                .get(title_id)
                .map(|episodes| {
                    episodes
                        .iter()
                        .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
                        .map(|episode| episode.id.as_str())
                        .collect()
                })
                .unwrap_or_default()
        };

        // Match scopes to files *before* scoring anything: only files that
        // occupy one of the scopes on this page are worth a scoring pass.
        let matches_by_scope: Vec<Vec<usize>> = scopes
            .iter()
            .map(|scope| {
                files
                    .iter()
                    .enumerate()
                    .filter(|(_, file)| file.file.title_id == scope.title_id)
                    .filter(|(_, file)| {
                        scope.matches(file, |collection_id| {
                            collection_members(&scope.title_id, collection_id)
                        })
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();
        let scored_indices: HashSet<usize> = matches_by_scope.iter().flatten().copied().collect();
        if scored_indices.is_empty() {
            return bars;
        }

        let titles = self
            .services
            .catalog
            .titles
            .get_by_ids(&title_ids)
            .await
            .unwrap_or_default();

        // One scoring context per title, reused by every scope that title owns.
        let mut bars_by_index: HashMap<usize, i32> = HashMap::new();
        for title in &titles {
            let title_indices: Vec<usize> = scored_indices
                .iter()
                .copied()
                .filter(|index| files[*index].file.title_id == title.id)
                .collect();
            if title_indices.is_empty() {
                continue;
            }
            let profile = match self.resolve_quality_profile_for_title(title).await {
                Ok(profile) => profile,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        title_id = %title.id,
                        "failed to resolve quality profile for the landed bar; leaving it unset"
                    );
                    continue;
                }
            };
            let context = self.resolve_canonical_scoring_context(title, &profile).await;
            let no_episodes: Vec<scryer_domain::Episode> = Vec::new();
            let title_episodes = episodes_by_title
                .get(title.id.as_str())
                .unwrap_or(&no_episodes);
            for index in title_indices {
                // The gate's runtime basis (D4): the length of what *this file*
                // holds, summed over its whole span.
                let basis = crate::acquisition_coverage::episode_span_size_basis(
                    title_episodes,
                    &files[index].episode_ids,
                    title.runtime_minutes,
                );
                let bar = self.incumbent_bar(&files[index].file, &context, basis);
                bars_by_index.insert(index, bar.score);
            }
        }

        for (bar, matches) in bars.iter_mut().zip(matches_by_scope.iter()) {
            *bar = matches
                .iter()
                .filter_map(|index| bars_by_index.get(index).copied())
                .max();
        }
        bars
    }

    /// [`Self::landed_bars_for_scopes`] over persisted state rows — the
    /// `Title.wantedItems` relation. The Wanted page decorates its *views*
    /// instead, because half of them have no row.
    pub(crate) async fn decorate_landed_bars(&self, items: &mut [&mut AcquisitionScopeState]) {
        if items.is_empty() {
            return;
        }
        let scopes: Vec<LandedBarScope> = items
            .iter()
            .map(|item| LandedBarScope {
                title_id: item.title_id.clone(),
                episode_id: item.episode_id.clone(),
                collection_id: item.collection_id.clone(),
                series_movie_link_id: item.series_movie_link_id.clone(),
            })
            .collect();
        let bars = self.landed_bars_for_scopes(&scopes).await;
        for (item, bar) in items.iter_mut().zip(bars) {
            item.landed_bar = bar;
        }
    }
}

/// One scope's identity for landed-bar resolution.
///
/// Both listing paths produce it: a persisted `AcquisitionScopeState` row, and a
/// `WantedScopeView` derived from the projection that may have no row at all.
/// The precedence mirrors [`crate::AcquisitionScopeState::submission_scope`]
/// exactly, so the display number and the gate agree about what a scope covers.
#[derive(Clone, Debug)]
pub(crate) struct LandedBarScope {
    pub title_id: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
}

impl LandedBarScope {
    /// The precedence every scope test shares: a present-but-blank id is no id.
    /// The collection pre-resolution used to test `is_none()` instead, so a row
    /// carrying `Some("")` skipped it and then reported no bar at all.
    fn non_empty(value: &Option<String>) -> Option<&str> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    /// Does `file` occupy this scope? `collection_members` resolves a
    /// collection id to its episode ids.
    /// Only a **primary** occupant sets a bar, and primary is per episode: a
    /// file that is primary for E02 and additional for E03 answers `true` for
    /// the first scope and `false` for the second.
    fn matches<'a>(
        &self,
        file: &LandedBarFile,
        collection_members: impl Fn(&str) -> Vec<&'a str>,
    ) -> bool {
        if let Some(episode_id) = Self::non_empty(&self.episode_id) {
            return file.is_primary_for(episode_id);
        }
        if let Some(link_id) = Self::non_empty(&self.series_movie_link_id) {
            return file.is_primary_without_episodes()
                && file
                    .file
                    .series_movie_link_ids
                    .iter()
                    .any(|candidate| candidate == link_id);
        }
        if let Some(collection_id) = Self::non_empty(&self.collection_id) {
            return collection_members(collection_id)
                .iter()
                .any(|member| file.is_primary_for(member));
        }
        file.is_primary_without_episodes()
            && file.episode_ids.is_empty()
            && file.file.series_movie_link_ids.is_empty()
    }
}

/// One media file with its whole episode span, rebuilt from the joined listing.
///
/// `list_media_files_for_titles` emits one row per file-episode link, so the
/// scalar `episode_id` on a row is *a* member of the file's span, never the
/// span. Everything that asks "how long is this file" or "does it cover that
/// episode" needs the span, and answering either from a single row is what made
/// the Wanted page's `currentScore` disagree with the gate's bar for every
/// multi-episode file.
///
/// The role is **per episode**, not per file: the listing selects
/// `COALESCE(fem.role, mf.role)`, so a file can be primary for one episode it
/// covers and additional for another. Reading the role off whichever row
/// happened to come back first made a scope's bar depend on row order.
#[derive(Clone, Debug)]
pub(crate) struct LandedBarFile {
    pub file: crate::TitleMediaFile,
    /// Every episode the file covers, whatever role it holds there.
    pub episode_ids: Vec<String>,
    /// The subset it is the **primary** occupant of. Only these decide a bar.
    primary_episode_ids: Vec<String>,
    /// For a file with no episode rows at all (a movie, a series-movie link),
    /// where the row's role is the file's own.
    role_is_primary: bool,
}

impl LandedBarFile {
    fn group(rows: Vec<crate::TitleMediaFile>) -> Vec<Self> {
        let mut by_file: HashMap<String, usize> = HashMap::new();
        let mut files: Vec<Self> = Vec::new();
        for row in rows {
            let episode_id = row.episode_id.clone();
            let is_primary = row.role.is_primary();
            let index = *by_file.entry(row.id.clone()).or_insert_with(|| {
                files.push(Self {
                    file: row,
                    episode_ids: Vec::new(),
                    primary_episode_ids: Vec::new(),
                    role_is_primary: false,
                });
                files.len() - 1
            });
            files[index].role_is_primary |= is_primary;
            if let Some(episode_id) = episode_id {
                if !files[index].episode_ids.contains(&episode_id) {
                    files[index].episode_ids.push(episode_id.clone());
                }
                if is_primary && !files[index].primary_episode_ids.contains(&episode_id) {
                    files[index].primary_episode_ids.push(episode_id);
                }
            }
        }
        files
    }

    /// Is this file the primary occupant of `episode_id`?
    fn is_primary_for(&self, episode_id: &str) -> bool {
        self.primary_episode_ids
            .iter()
            .any(|candidate| candidate == episode_id)
    }

    /// Is this file the primary occupant of a scope that has no episodes?
    fn is_primary_without_episodes(&self) -> bool {
        self.role_is_primary
    }
}

impl AppUseCase {
    async fn list_wanted_items_for_libraries(
        &self,
        query: AcquisitionScopeStatesQuery,
        library_ids: Vec<String>,
    ) -> AppResult<(Vec<AcquisitionScopeState>, i64)> {
        let AcquisitionScopeStatesQuery {
            statuses,
            media_types,
            title_id,
            library_ids: _,
            title_search,
            latest_decision_codes,
            limit,
            offset,
        } = query;
        let title_search = title_search.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        if library_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: statuses.clone(),
                media_types: media_types.clone(),
                title_id: title_id.clone(),
                library_ids: library_ids.clone(),
                title_search: title_search.clone(),
                latest_decision_codes: latest_decision_codes.clone(),
                limit,
                offset,
            })
            .await?;
        let total = self
            .services
            .workflow
            .acquisition_scope_states
            .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses,
                media_types,
                title_id,
                library_ids,
                title_search,
                latest_decision_codes,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        Ok((items, total))
    }
}
impl AppUseCase {
    /// Pause acquisition for a scope: user intent persisted on the state row.
    /// A paused scope is excluded from the derived target set until resumed. The
    /// identifier is a state-row id or a convergence scope key; a
    /// scope key with no row yet materializes one so the pause has somewhere to live.
    pub async fn pause_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .resolve_or_create_wanted_state_row(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .workflow
            .acquisition_scope_states
            .transition_acquisition_scope_to_paused(&AcquisitionScopePauseTransition {
                id: item.id.clone(),
                last_search_at: item.last_search_at.clone(),
                grabbed_release: item.grabbed_release.clone(),
            })
            .await
    }
}
impl AppUseCase {
    /// Resume acquisition for a paused scope: it re-enters the derived target
    /// set. Existing coverage stays valid — the cursor only searches indexers
    /// it has not already searched under the current fingerprint. Accepts a
    /// state-row id or a convergence scope key.
    pub async fn resume_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .resolve_or_create_wanted_state_row(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .workflow
            .acquisition_scope_states
            .update_acquisition_scope_status(
                &item.id,
                AcquisitionScopeStatus::Wanted.as_str(),
                item.last_search_at.as_deref(),
                item.grabbed_release.as_deref(),
            )
            .await?;
        self.runtime.acquisition.acquisition_wake.notify_one();
        Ok(())
    }
}
impl AppUseCase {
    async fn wanted_item_submission_scope(&self, item: &AcquisitionScopeState) -> AppResult<SubmissionScope> {
        let episode = if let Some(episode_id) = item.episode_id.as_deref() {
            self.services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
        } else {
            None
        };
        Ok(direct_download_submission_scope_for_wanted_item(
            item,
            episode.as_ref(),
        ))
    }
}
impl AppUseCase {
    pub(crate) async fn covered_wanted_item_ids_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
        fallback_wanted_item_id: &str,
    ) -> AppResult<Vec<String>> {
        let title_ids = [title_id.to_string()];
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_for_title_ids(&title_ids)
            .await?;
        if items.is_empty() {
            return Ok(if fallback_wanted_item_id.is_empty() {
                Vec::new()
            } else {
                vec![fallback_wanted_item_id.to_string()]
            });
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(title_id)
            .await?;
        let fake_submission = DownloadSubmission {
    download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title_id.to_string(),
            // Scope matching only; this submission is never persisted.
            release_size_bytes: None,
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: String::new(),
            download_client_id: None,
            download_client_type: String::new(),
            download_client_item_id: String::new(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: None,
            info_hash: None,
            request_signature: None,
            scope: scope.clone(),
        };

        let mut covered = items
            .iter()
            .filter(|item| {
                let episode_collection_id = item.episode_id.as_ref().and_then(|episode_id| {
                    episodes
                        .iter()
                        .find(|episode| &episode.id == episode_id)
                        .and_then(|episode| episode.collection_id.as_deref())
                });
                item.id == fallback_wanted_item_id
                    || submission_blocks_wanted_item(&fake_submission, item, episode_collection_id)
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        covered.sort();
        covered.dedup();
        if covered.is_empty() && !fallback_wanted_item_id.is_empty() {
            covered.push(fallback_wanted_item_id.to_string());
        }
        Ok(covered)
    }
}
impl AppUseCase {
    /// Operator queue replacement: the blocking download is gone, so every
    /// scope it covered re-opens from scratch — score baseline cleared (the
    /// replacement decides on its own merits), all coverage pruned, loop woken.
    pub(crate) async fn reset_wanted_items_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
    ) -> AppResult<()> {
        let wanted_item_ids = self
            .covered_wanted_item_ids_for_submission_scope(title_id, scope, "")
            .await?;
        for wanted_item_id in wanted_item_ids {
            if let Some(item) = self
                .services
                .workflow
                .acquisition_scope_states
                .get_acquisition_scope_state_by_id(&wanted_item_id)
                .await?
            {
                self.services
                    .workflow
                    .acquisition_scope_states
                    .update_acquisition_scope_status(
                        &item.id,
                        AcquisitionScopeStatus::Wanted.as_str(),
                        None,
                        None,
                    )
                    .await?;
                self.reopen_wanted_scope_for_acquisition(&item, CoverageReopen::All)
                    .await;
            }
        }
        Ok(())
    }
}
