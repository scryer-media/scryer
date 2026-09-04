//! Derived wanted/upgrades views. The Missing and Upgrades tabs
//! read the SAME derived target set the convergence cursor rotates over
//! (`derive_missing_targets` / `compute_cutoff_unmet_items`), joined to the
//! activity-driven state row when one exists and enriched with the per-scope
//! convergence progress the UI shows instead of a search cadence. Convergence is
//! derived for a whole page in ONE coverage round-trip (#12): resolve each scope's
//! `(fingerprint, routed indexers)` once per title, fetch all coverage rows for the
//! page's scope keys together, then compute covered/routed counts in memory and
//! fold in the scheduler availability snapshot for the `Deferred` state.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use chrono::Utc;
use scryer_domain::{
    DomainEventPayload, Id, JobRunCompletedEventData, JobRunFailedEventData,
    JobRunStartedEventData, MediaFacet,
};

use super::*;
use crate::acquisition::convergence::convergence_scope_key;
use crate::acquisition::targets::AcquisitionTarget;
use crate::contracts::{QueueDownloadOutcome, SubmissionConflictPolicy, SubmissionScope};

/// Convergence state of a scope for the UI. Mirrors the GraphQL
/// `ConvergenceStateValue`; the interface maps this 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WantedConvergenceState {
    /// No routed indexer searched yet under the current fingerprint.
    Queued,
    /// Some but not all routed indexers covered — sweep in progress.
    Searching,
    /// Every routed indexer covered — watching RSS.
    Converged,
    /// Not converged, and every still-uncovered indexer is currently unavailable.
    Deferred,
}

/// Per-scope convergence progress carried on a wanted view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantedViewConvergence {
    pub state: WantedConvergenceState,
    pub indexers_covered: i32,
    pub indexers_routed: i32,
}

/// One derived wanted/upgrades row: the target coordinates, the
/// joined activity-state row (when one exists), title/library enrichment, the
/// recency lane, and the batched convergence progress. `id`-identity is decided by
/// the interface mapper (state-row id, else scope key).
#[derive(Clone, Debug)]
pub struct WantedScopeView {
    pub scope_key: String,
    pub title_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    /// "movie" | "episode" | "series_movie".
    pub media_type: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    /// Recency lane (`true` = hot). Upgrades are always cold.
    pub is_hot: bool,
    /// The activity-driven acquisition-state row, when one exists for this scope.
    pub state: Option<AcquisitionScopeState>,
    /// The re-derived canonical bar of the file occupying this scope, or `None`
    /// when nothing does.
    ///
    /// It lives on the **view**, not on `state`, because the Wanted page's rows
    /// come from the projection: a scope that was never searched or grabbed has
    /// no `acquisition_scope_states` row, and an occupied cutoff-unmet scope
    /// produced by a library scan is exactly that shape. Decorating only the
    /// rows showed `currentScore: null` for the common case.
    pub landed_bar: Option<i32>,
    /// Number of saved fallback candidates keyed to this scope.
    pub standby_count: i64,
    pub convergence: WantedViewConvergence,
}

/// Resolved `(fingerprint, facet, routed indexers)` for one title — identical
/// across all of a title's scopes (same profile, routing and match identity), so
/// it is resolved once per title and reused for every scope of that title (#12).
#[derive(Clone)]
struct TitleConvergenceContext {
    fingerprint: String,
    routed_indexer_ids: Vec<String>,
}

fn wanted_projection_time_bucket(kind: WantedKind, now: chrono::DateTime<Utc>) -> Option<i64> {
    match kind {
        WantedKind::Missing => Some(now.timestamp().div_euclid(60)),
        WantedKind::CutoffUpgrade => None,
    }
}

#[cfg(test)]
mod wanted_projection_time_bucket_tests {
    use super::{Utc, WantedKind, wanted_projection_time_bucket};
    use chrono::TimeZone;

    #[test]
    fn missing_rolls_at_the_next_utc_minute_while_cutoff_does_not() {
        let before = Utc.timestamp_opt(119, 999_999_999).single().unwrap();
        let after = Utc.timestamp_opt(120, 0).single().unwrap();
        assert_eq!(
            wanted_projection_time_bucket(WantedKind::Missing, before),
            Some(1)
        );
        assert_eq!(
            wanted_projection_time_bucket(WantedKind::Missing, after),
            Some(2)
        );
        assert_eq!(
            wanted_projection_time_bucket(WantedKind::CutoffUpgrade, after),
            None
        );
    }
}

impl AppUseCase {
    /// One page of the derived Missing/Upgrades view. Mirrors the
    /// cutoff-unmet authorization: results are limited to the actor's authorized
    /// libraries. `MISSING` derives from the same fileless-scope query the cursor
    /// uses; `CUTOFF_UPGRADE` reuses the cutoff-unmet compute. Both join the state
    /// row (excluding paused/grabbed-active scopes), enrich title/library names,
    /// sort deterministically, then slice — the convergence progress for the sliced
    /// page is derived in one batched coverage round-trip.
    #[expect(
        clippy::too_many_arguments,
        reason = "the derived wanted view is parameterized by kind, facet, library scope, search, and paging"
    )]
    pub async fn list_wanted_scope_views(
        &self,
        actor: &User,
        kind: WantedKind,
        facet: Option<MediaFacet>,
        library_ids: Vec<String>,
        title_search: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<WantedScopeView>, i64)> {
        let authorized = self
            .list_libraries_for_permission(
                actor,
                facet.clone(),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut authorized_ids: HashSet<String> = authorized
            .iter()
            .map(|library| library.id.clone())
            .collect();
        let requested: HashSet<String> = library_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if !requested.is_empty() {
            authorized_ids.retain(|id| requested.contains(id));
        }
        if authorized_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let facet_str = facet.as_ref().map(|facet| facet.as_str().to_string());
        let title_needle = title_search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        let cached = self.current_wanted_projection(kind).await?;
        let excluded_scope_keys = self.non_wanted_state_scope_keys().await?;
        let filtered = cached
            .iter()
            .filter(|view| !excluded_scope_keys.contains(&view.scope_key))
            .filter(|view| authorized_ids.contains(&view.library_id))
            .filter(|view| {
                facet_str
                    .as_deref()
                    .is_none_or(|facet| view.facet.as_str() == facet)
            })
            .filter(|view| {
                title_needle.as_deref().is_none_or(|needle| {
                    view.title_name
                        .as_deref()
                        .is_some_and(|name| name.to_ascii_lowercase().contains(needle))
                })
            })
            .collect::<Vec<_>>();

        let total = filtered.len() as i64;
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let mut page: Vec<WantedScopeView> = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        // Join the state row + derive convergence for the sliced page only.
        self.attach_state_rows(&mut page).await?;
        self.attach_page_convergence(&mut page).await?;

        Ok((page, total))
    }

    async fn current_wanted_projection(
        &self,
        kind: WantedKind,
    ) -> AppResult<Arc<[WantedScopeView]>> {
        use std::sync::atomic::Ordering;

        let generation = self
            .runtime
            .acquisition
            .wanted_projection_generation
            .load(Ordering::Acquire);
        let time_bucket = wanted_projection_time_bucket(kind, Utc::now());
        if let Some(cached) = self
            .runtime
            .acquisition
            .wanted_projection_cache
            .read()
            .await
            .get(&kind)
            .filter(|cached| cached.generation == generation && cached.time_bucket == time_bucket)
        {
            metrics::counter!("scryer_wanted_projection_cache_total", "result" => "hit")
                .increment(1);
            return Ok(cached.rows.clone());
        }

        metrics::counter!("scryer_wanted_projection_cache_total", "result" => "miss").increment(1);
        let _build_guard = self
            .runtime
            .acquisition
            .wanted_projection_build_lock
            .lock()
            .await;
        for attempt in 0..2 {
            let generation = self
                .runtime
                .acquisition
                .wanted_projection_generation
                .load(Ordering::Acquire);
            let now = Utc::now();
            let time_bucket = wanted_projection_time_bucket(kind, now);
            if let Some(cached) = self
                .runtime
                .acquisition
                .wanted_projection_cache
                .read()
                .await
                .get(&kind)
                .filter(|cached| {
                    cached.generation == generation && cached.time_bucket == time_bucket
                })
            {
                return Ok(cached.rows.clone());
            }

            let started_at = Instant::now();
            let mut rows = match kind {
                WantedKind::Missing => self
                    .derive_missing_targets(&now)
                    .await?
                    .into_iter()
                    .map(missing_target_to_view)
                    .collect::<Vec<_>>(),
                WantedKind::CutoffUpgrade => self
                    .compute_cutoff_unmet_items(None, None)
                    .await?
                    .into_iter()
                    .filter_map(cutoff_item_to_view)
                    .collect::<Vec<_>>(),
            };
            self.enrich_view_titles(&mut rows).await;
            let libraries = self.services.catalog.libraries.list(None).await?;
            let library_presentation = libraries
                .into_iter()
                .map(|library| (library.id, (library.name, library.slug)))
                .collect::<HashMap<_, _>>();
            for view in &mut rows {
                if let Some((name, slug)) = library_presentation.get(&view.library_id) {
                    view.library_name = Some(name.clone());
                    view.library_slug = Some(slug.clone());
                }
            }
            sort_wanted_views(&mut rows);

            let generation_changed = self
                .runtime
                .acquisition
                .wanted_projection_generation
                .load(Ordering::Acquire)
                != generation;
            let time_bucket_changed =
                wanted_projection_time_bucket(kind, Utc::now()) != time_bucket;
            if generation_changed || time_bucket_changed {
                if attempt == 0 {
                    continue;
                }
                return Err(AppError::Repository(
                    "wanted projection changed while it was being rebuilt; retry the request"
                        .to_string(),
                ));
            }
            let rows: Arc<[WantedScopeView]> = Arc::from(rows);
            self.runtime
                .acquisition
                .wanted_projection_cache
                .write()
                .await
                .insert(
                    kind,
                    crate::services::CachedWantedProjection {
                        generation,
                        time_bucket,
                        rows: rows.clone(),
                    },
                );
            metrics::histogram!("scryer_wanted_projection_rebuild_duration_seconds", "kind" => kind.as_str())
                .record(started_at.elapsed().as_secs_f64());
            metrics::gauge!("scryer_wanted_projection_items", "kind" => kind.as_str())
                .set(rows.len() as f64);
            return Ok(rows);
        }
        unreachable!("wanted projection rebuild attempts are bounded")
    }

    /// Scope keys whose state row is paused or grabbed — excluded from the active
    /// derived view. One list query, keyed by the same scope identity the cursor
    /// uses.
    async fn non_wanted_state_scope_keys(&self) -> AppResult<HashSet<String>> {
        let mut excluded = HashSet::new();
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec![
                    AcquisitionScopeStatus::Paused.as_str().to_string(),
                    AcquisitionScopeStatus::Grabbed.as_str().to_string(),
                ],
                limit: i64::MAX,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        for item in items.into_iter().filter(|item| {
            matches!(
                item.status,
                AcquisitionScopeStatus::Paused | AcquisitionScopeStatus::Grabbed
            )
        }) {
            let scope = SubmissionScope::from_persisted(
                &item.title_id,
                item.episode_id.clone(),
                item.collection_id.clone(),
                item.series_movie_link_id.clone(),
                None,
            );
            if let Some(scope_key) = convergence_scope_key(&scope, &item.title_id) {
                excluded.insert(scope_key);
            }
        }
        Ok(excluded)
    }

    /// Fill in `title_name`/`title_slug` for the derived rows with one batch read.
    async fn enrich_view_titles(&self, rows: &mut [WantedScopeView]) {
        let unique_title_ids = rows
            .iter()
            .map(|view| view.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let names = self
            .services
            .catalog
            .titles
            .get_by_ids(&unique_title_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|title| (title.id, (Some(title.name), title.slug)))
            .collect::<HashMap<_, _>>();
        for view in rows.iter_mut() {
            if view.title_name.is_some() {
                continue;
            }
            if let Some((name, slug)) = names.get(&view.title_id) {
                view.title_name = name.clone();
                view.title_slug = slug.clone();
            }
        }
    }

    /// Attach the activity-driven state rows with one repository read for the page.
    async fn attach_state_rows(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        if page.is_empty() {
            return Ok(());
        }
        let wanted_scope_keys = page
            .iter()
            .map(|view| view.scope_key.clone())
            .collect::<HashSet<_>>();
        let title_ids = page
            .iter()
            .map(|view| view.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let states = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_for_title_ids(&title_ids)
            .await?;
        let mut states_by_scope = HashMap::new();
        for state in states {
            let scope = SubmissionScope::from_persisted(
                &state.title_id,
                state.episode_id.clone(),
                state.collection_id.clone(),
                state.series_movie_link_id.clone(),
                None,
            );
            let Some(scope_key) = convergence_scope_key(&scope, &state.title_id) else {
                continue;
            };
            if wanted_scope_keys.contains(scope_key.as_str()) {
                states_by_scope.insert(scope_key, state);
            }
        }
        for view in page.iter_mut() {
            view.state = states_by_scope.get(&view.scope_key).cloned();
        }
        // One grouped query for the page's items — never a read of the whole
        // standby table, which is uncapped by design.
        let wanted_item_ids = page
            .iter()
            .filter_map(|view| view.state.as_ref().map(|state| state.id.clone()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !wanted_item_ids.is_empty() {
            let standby_counts = self
                .services
                .workflow
                .pending_releases
                .count_standby_pending_releases_for_wanted_items(&wanted_item_ids)
                .await?;
            for view in page.iter_mut() {
                view.standby_count = view
                    .state
                    .as_ref()
                    .and_then(|state| standby_counts.get(&state.id))
                    .copied()
                    .unwrap_or_default();
            }
        }
        // `landed_bar` is resolved on read, never stored, so neither the
        // projection nor the state row carries it. Decorated per **view** so a
        // scope with no state row still reports a number (D10).
        let scopes: Vec<crate::acquisition_workflow::LandedBarScope> = page
            .iter()
            .map(|view| crate::acquisition_workflow::LandedBarScope {
                title_id: view.title_id.clone(),
                episode_id: view.episode_id.clone(),
                collection_id: view.collection_id.clone(),
                series_movie_link_id: view.series_movie_link_id.clone(),
            })
            .collect();
        let bars = self.landed_bars_for_scopes(&scopes).await;
        for (view, bar) in page.iter_mut().zip(bars) {
            view.landed_bar = bar;
            if let Some(state) = view.state.as_mut() {
                state.landed_bar = bar;
            }
        }
        Ok(())
    }

    /// Derive per-scope convergence progress for a page in ONE coverage round-trip
    /// and attach it to each view.
    async fn attach_page_convergence(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        if page.is_empty() {
            return Ok(());
        }
        let scopes: Vec<(String, String)> = page
            .iter()
            .map(|view| (view.title_id.clone(), view.scope_key.clone()))
            .collect();
        let by_scope = self.page_convergence_by_scope_key(&scopes).await;
        for view in page.iter_mut() {
            if let Some(convergence) = by_scope.get(&view.scope_key) {
                view.convergence = *convergence;
            }
        }
        Ok(())
    }

    /// Batched per-scope convergence progress for a page, keyed by
    /// scope key. Resolves `(fingerprint, routed indexers)` once per title, fetches
    /// all coverage rows for the page's scope keys in one round-trip, computes
    /// covered/routed counts in memory, and folds in the scheduler availability
    /// snapshot to distinguish `Deferred` from `Queued`. Shared by the Missing /
    /// Upgrades views and the cutoff-unmet page so both show identical convergence.
    pub async fn page_convergence_by_scope_key(
        &self,
        scopes: &[(String, String)],
    ) -> HashMap<String, WantedViewConvergence> {
        let mut result = HashMap::new();
        if scopes.is_empty() {
            return result;
        }

        // One (fingerprint, routed) resolution per unique title — identical across a
        // title's scopes.
        let title_ids = scopes
            .iter()
            .map(|(title_id, _)| title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let titles = self
            .services
            .catalog
            .titles
            .get_by_ids(&title_ids)
            .await
            .unwrap_or_default();
        let mut title_context: HashMap<String, Option<TitleConvergenceContext>> = HashMap::new();
        for title in titles {
            let context = self
                .resolve_title_convergence_context_for_title(&title)
                .await;
            title_context.insert(title.id.clone(), context);
        }

        // One coverage fetch for the whole page.
        let scope_keys: Vec<String> = scopes.iter().map(|(_, key)| key.clone()).collect();
        let coverage_rows = self
            .services
            .integrations
            .scope_indexer_coverage
            .list_coverage_for_scope_keys(&scope_keys)
            .await
            .unwrap_or_default();
        let mut coverage_by_scope: HashMap<String, Vec<crate::ScopeCoverageRow>> = HashMap::new();
        for row in coverage_rows {
            coverage_by_scope
                .entry(row.scope_key.clone())
                .or_default()
                .push(row);
        }

        let availability = self.scheduler_availability().await;
        let host_keys = self.indexer_scheduler_host_keys().await;

        for (title_id, scope_key) in scopes {
            let Some(Some(context)) = title_context.get(title_id) else {
                // No routing/profile resolvable — nothing to converge; present as
                // converged (0/0) so the UI does not show a perpetual sweep.
                result.insert(
                    scope_key.clone(),
                    WantedViewConvergence {
                        state: WantedConvergenceState::Converged,
                        indexers_covered: 0,
                        indexers_routed: 0,
                    },
                );
                continue;
            };
            let routed = &context.routed_indexer_ids;
            let covered: HashSet<&str> = coverage_by_scope
                .get(scope_key)
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.fingerprint == context.fingerprint)
                        .map(|row| row.indexer_id.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let covered_count = routed
                .iter()
                .filter(|id| covered.contains(id.as_str()))
                .count();
            let routed_count = routed.len();
            let uncovered: Vec<&String> = routed
                .iter()
                .filter(|id| !covered.contains(id.as_str()))
                .collect();

            let state = if uncovered.is_empty() {
                WantedConvergenceState::Converged
            } else if uncovered.iter().all(|id| {
                !availability.indexer_available(host_keys.get(id.as_str()).map(String::as_str), id)
            }) {
                WantedConvergenceState::Deferred
            } else if covered_count == 0 {
                WantedConvergenceState::Queued
            } else {
                WantedConvergenceState::Searching
            };

            result.insert(
                scope_key.clone(),
                WantedViewConvergence {
                    state,
                    indexers_covered: covered_count as i32,
                    indexers_routed: routed_count as i32,
                },
            );
        }

        result
    }

    /// Resolve `(fingerprint, routed indexers)` for a title via its title-level
    /// search subject — the values every scope of the title shares. `None` when the
    /// title is gone or nothing is routed.
    async fn resolve_title_convergence_context_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> Option<TitleConvergenceContext> {
        let subject = self
            .resolve_release_search_subject_for_title(title)
            .await
            .ok()?;
        let convergence = self.resolve_scope_convergence(title, &subject).await?;
        Some(TitleConvergenceContext {
            fingerprint: convergence.fingerprint,
            routed_indexer_ids: convergence.routed_indexer_ids,
        })
    }
}

/// A missing target's coordinates as a view row (state/enrichment/convergence
/// filled in later).
fn missing_target_to_view(target: AcquisitionTarget) -> WantedScopeView {
    WantedScopeView {
        scope_key: target.scope_key,
        title_id: target.title_id,
        library_id: target.library_id,
        facet: target.facet,
        media_type: target.media_type,
        episode_id: target.episode_id,
        collection_id: target.collection_id,
        series_movie_link_id: target.series_movie_link_id,
        season_number: target.season_number,
        episode_number: target.episode_number,
        title_name: None,
        title_slug: None,
        library_name: None,
        library_slug: None,
        is_hot: target.is_hot,
        state: None,
        landed_bar: None,
        standby_count: 0,
        convergence: pending_convergence(),
    }
}

/// A cutoff-unmet item as a view row. Upgrades are always cold. `None` when the
/// item's scope has no derivable convergence key.
fn cutoff_item_to_view(item: CutoffUnmetItem) -> Option<WantedScopeView> {
    let scope =
        SubmissionScope::from_persisted(&item.title_id, item.episode_id.clone(), None, None, None);
    let scope_key = convergence_scope_key(&scope, &item.title_id)?;
    let media_type = if item.episode_id.is_some() {
        "episode"
    } else {
        "movie"
    };
    Some(WantedScopeView {
        scope_key,
        title_id: item.title_id,
        library_id: item.library_id,
        facet: item.title_facet,
        media_type: media_type.to_string(),
        episode_id: item.episode_id,
        collection_id: None,
        series_movie_link_id: None,
        season_number: item.season_number,
        episode_number: item.episode_number,
        title_name: Some(item.title_name),
        title_slug: item.title_slug,
        library_name: item.library_name,
        library_slug: item.library_slug,
        is_hot: false,
        state: None,
        landed_bar: None,
        standby_count: 0,
        convergence: pending_convergence(),
    })
}

/// Placeholder convergence used before the batched per-page derivation fills it in.
fn pending_convergence() -> WantedViewConvergence {
    WantedViewConvergence {
        state: WantedConvergenceState::Queued,
        indexers_covered: 0,
        indexers_routed: 0,
    }
}

/// Digits of `value`, or `i64::MAX` when absent — matches the cutoff-unmet sort so
/// Missing and Upgrades order identically.
fn parse_sort_number(value: Option<&str>) -> i64 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            let digits: String = value.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty())
                .then(|| digits.parse::<i64>().ok())
                .flatten()
        })
        .unwrap_or(i64::MAX)
}

// ── Interactive acquisition-search job ─────────────

/// One scope to search in an interactive acquisition-search job.
#[derive(Clone, Debug)]
pub(crate) struct AcquisitionSearchScope {
    title_id: String,
    scope: SubmissionScope,
    /// The convergence scope key this row resolved to, so a plan that runs the
    /// staged walk can restrict the walk to exactly the scopes the request
    /// narrowed to.
    scope_key: String,
    /// Human label for the progress `currentTitle` field.
    label: String,
}

/// What an acquisition-search job will actually run.
#[derive(Clone, Debug)]
pub(crate) enum AcquisitionSearchPlan {
    /// A title-scoped request for an episodic title: the same staged walk the
    /// convergence cycle runs, under interactive intent, so the pack stages
    /// exist before any episode scope grabs.
    TitleWalk {
        title_id: String,
        /// Restrict the walk to one season, when the request named one.
        season_number: Option<u32>,
        /// The scopes the request resolved to. The walk derives the title's
        /// acquisition targets and keeps only these, because the request
        /// narrowed by wanted kind, facet and library and the derivation does
        /// not: a cutoff-unmet request must not go on to search the title's
        /// missing scopes. The pack stages are then derived from what is left.
        scope_keys: HashSet<String>,
    },
    /// Row-anchored requests and facet/library-wide sweeps: one
    /// `queue_best_release` per scope, unchanged.
    Scopes(Vec<AcquisitionSearchScope>),
}

/// What a title walk contributed to the job's terminal status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AcquisitionSearchWalkOutcome {
    total: usize,
    processed: usize,
    grabbed: usize,
    failed: usize,
    /// A work item could not submit because its download client was
    /// unavailable. `acquisition_search_job_status` fails the job on that even
    /// when other work items merely found nothing.
    submit_unavailable: bool,
}

impl AcquisitionSearchPlan {
    /// The job's starting `total`. A walk replaces it with its own work-item
    /// count (pack stages included) on the first progress step.
    fn scope_count(&self) -> usize {
        match self {
            Self::TitleWalk { scope_keys, .. } => scope_keys.len(),
            Self::Scopes(scopes) => scopes.len(),
        }
    }
}

/// Request for the interactive acquisition-search job. A bare
/// request searches every derived target of `wanted_kind`; the narrowing fields
/// filter that set, and `wanted_item_id` (a state-row id or a scope key) selects a
/// single scope.
#[derive(Clone, Debug, Default)]
pub struct AcquisitionSearchRequest {
    pub wanted_kind: WantedKind,
    pub facet: Option<MediaFacet>,
    pub library_ids: Vec<String>,
    pub title_id: Option<String>,
    pub season_number: Option<i32>,
    pub wanted_item_id: Option<String>,
}

/// `Missing` is the default target set for the interactive search request — matching the `wantedItems` query default. Defined here because that's
/// the only consumer of a defaulted `WantedKind`.
impl Default for WantedKind {
    fn default() -> Self {
        Self::Missing
    }
}

/// Progress snapshot persisted in the job run's `progress_json`,
/// read back by the `acquisitionSearchJob` query and pushed via `jobRunEvents`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionSearchProgress {
    /// One of the `AcquisitionSearchJobStateValue` snake_case names.
    pub state: String,
    pub total: usize,
    pub processed: usize,
    pub grabbed_count: usize,
    pub failed_count: usize,
    pub current_title: Option<String>,
}

/// App-side view of the interactive acquisition-search job for the GraphQL query
///. Built from the persisted run record + its progress json.
#[derive(Clone, Debug)]
pub struct AcquisitionSearchJobView {
    pub id: String,
    /// Snake_case `AcquisitionSearchJobStateValue` name.
    pub state: String,
    pub total: i32,
    pub processed: i32,
    pub grabbed_count: i32,
    pub failed_count: i32,
    pub current_title: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Map a terminal/running job-run status onto the acquisition-search job state
/// vocabulary. Partial failures use `Warning` internally but are still a
/// completed search; only the explicit cancellation signal is cancelled.
/// Whether a scope's search error counts against the job. A search that finds
/// nothing grabbable — nothing at all, or nothing auto-eligible — is a completed
/// search, not a failure: the typed `NoAutoEligibleRelease` expresses the same
/// outcome the old `Validation("no auto-eligible release found")` did.
pub(crate) fn scope_search_error_is_failure(error: &AppError) -> bool {
    !matches!(
        error,
        AppError::Validation(_) | AppError::NoAutoEligibleRelease { .. }
    )
}
/// The job's terminal status. It completes when no scope failed; it fails when
/// nothing was grabbed and either every scope failed or a scope could not submit
/// because its download client was unavailable (a mapped client that is
/// disabled fails the job, even though the release itself is only parked —
/// `Pending`, never blocklisted — for convergence); otherwise it carries a
/// warning for the partial result.
fn acquisition_search_job_status(
    grabbed: usize,
    processed: usize,
    failed: usize,
    submit_unavailable: bool,
    cancelled: bool,
) -> JobRunStatus {
    if cancelled {
        JobRunStatus::Warning
    } else if failed == 0 {
        JobRunStatus::Completed
    } else if grabbed == 0 && (processed == failed || submit_unavailable) {
        JobRunStatus::Failed
    } else {
        JobRunStatus::Warning
    }
}
fn acquisition_search_state_for_status(status: JobRunStatus, cancelled: bool) -> &'static str {
    if cancelled {
        return "cancelled";
    }
    match status {
        JobRunStatus::Completed => "completed",
        JobRunStatus::Failed => "failed",
        JobRunStatus::Warning => "completed",
        _ => "running",
    }
}

#[cfg(test)]
mod acquisition_search_state_tests {
    use super::*;
    #[test]
    fn a_search_that_finds_nothing_grabbable_is_not_a_failed_scope() {
        assert!(!scope_search_error_is_failure(&AppError::Validation(
            "no auto-eligible release found".into()
        )));
        assert!(!scope_search_error_is_failure(
            &AppError::NoAutoEligibleRelease {
                candidate_count: 3,
                reasons: Vec::new(),
            }
        ));
        assert!(scope_search_error_is_failure(&AppError::Repository(
            "indexer exploded".into()
        )));
        assert!(scope_search_error_is_failure(
            &AppError::download_submit_unavailable("mapped download client is globally disabled")
        ));
    }
    #[test]
    fn an_all_empty_search_completes_and_a_disabled_mapped_client_fails_it() {
        // 98 scopes, none grabbable: completed, not failed.
        assert_eq!(
            acquisition_search_job_status(0, 98, 0, false, false),
            JobRunStatus::Completed
        );
        // One scope could not submit (mapped client disabled), the rest found
        // nothing: the job fails even though only one scope counted against it.
        assert_eq!(
            acquisition_search_job_status(0, 98, 1, true, false),
            JobRunStatus::Failed
        );
        // A definitive failure on one scope among many is a warning.
        assert_eq!(
            acquisition_search_job_status(0, 98, 1, false, false),
            JobRunStatus::Warning
        );
        // Every scope failed: failed.
        assert_eq!(
            acquisition_search_job_status(0, 2, 2, false, false),
            JobRunStatus::Failed
        );
        // Something was grabbed elsewhere: a partial result, not a failure.
        assert_eq!(
            acquisition_search_job_status(1, 98, 1, true, false),
            JobRunStatus::Warning
        );
        assert_eq!(
            acquisition_search_job_status(0, 98, 1, true, true),
            JobRunStatus::Warning
        );
    }

    #[test]
    fn warning_is_completed_unless_the_search_was_cancelled() {
        assert_eq!(
            acquisition_search_state_for_status(JobRunStatus::Warning, false),
            "completed"
        );
        assert_eq!(
            acquisition_search_state_for_status(JobRunStatus::Warning, true),
            "cancelled"
        );
        assert_eq!(
            acquisition_search_state_for_status(JobRunStatus::Completed, true),
            "cancelled"
        );
    }
}

impl AppUseCase {
    /// Start the interactive acquisition-search job:
    /// single-flight guarded, permission-checked (ManageTitles for a title-scoped
    /// request, ManageCatalogSettings for a facet/library-wide one — mirroring
    /// `scanLibrary`), then runs the per-scope best-release search+grab off a
    /// spawned task under a cancellation token. Returns the started run for the
    /// payload; progress is polled via `acquisition_search_job` and pushed via
    /// `jobRunEvents`.
    pub async fn start_acquisition_search_job(
        &self,
        actor: &User,
        request: AcquisitionSearchRequest,
    ) -> AppResult<JobRun> {
        let search_guard = self
            .runtime
            .jobs
            .acquisition_search_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| {
                AppError::Validation("an acquisition search job is already running".into())
            })?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::AcquisitionSearch)
            .await
        {
            return Err(AppError::Validation(
                "an acquisition search job is already running".into(),
            ));
        }

        self.authorize_acquisition_search(actor, &request).await?;
        let scopes = self
            .resolve_acquisition_search_scopes(actor, &request)
            .await?;
        let plan = self.acquisition_search_plan(&request, scopes).await?;

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::AcquisitionSearch,
            operation_type: format!(
                "acquisition_search:{}:{}",
                request.wanted_kind.as_str(),
                plan.scope_count()
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&AcquisitionSearchProgress {
                state: "running".to_string(),
                total: plan.scope_count(),
                processed: 0,
                grabbed_count: 0,
                failed_count: 0,
                current_title: None,
            })
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;

        let cancellation = tokio_util::sync::CancellationToken::new();
        self.runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .insert(run.id.clone(), cancellation.clone());

        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            app.run_acquisition_search_job(
                run,
                actor,
                actor_event,
                plan,
                cancellation,
                search_guard,
            )
            .await;
        });

        Ok(run_payload)
    }

    /// Which of the two shapes an acquisition-search request runs as.
    ///
    /// A title-scoped request for an episodic title runs the staged walk: the
    /// pack stages have to exist before any episode scope grabs, and expanding
    /// the title to one episode-subject `queue_best_release` per scope cannot
    /// produce them. Everything else keeps `queue_best_release`: a row-anchored
    /// request names one scope the operator picked, and a facet- or
    /// library-wide sweep is not a walk over one title.
    pub(crate) async fn acquisition_search_plan(
        &self,
        request: &AcquisitionSearchRequest,
        scopes: Vec<AcquisitionSearchScope>,
    ) -> AppResult<AcquisitionSearchPlan> {
        let row_anchored = request
            .wanted_item_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let Some(title_id) = request
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(AcquisitionSearchPlan::Scopes(scopes));
        };
        if row_anchored {
            return Ok(AcquisitionSearchPlan::Scopes(scopes));
        }
        let Some(title) = self.services.catalog.titles.get_by_id(title_id).await? else {
            return Ok(AcquisitionSearchPlan::Scopes(scopes));
        };
        if !self
            .facet_registry
            .get(&title.facet)
            .is_some_and(|handler| handler.has_episodes())
        {
            return Ok(AcquisitionSearchPlan::Scopes(scopes));
        }
        Ok(AcquisitionSearchPlan::TitleWalk {
            title_id: title_id.to_string(),
            season_number: request
                .season_number
                .and_then(|value| u32::try_from(value).ok()),
            scope_keys: scopes
                .into_iter()
                .map(|scope| scope.scope_key)
                .filter(|key| !key.is_empty())
                .collect(),
        })
    }

    /// Permission split: a title-scoped
    /// request (an explicit `title_id`, or a `wanted_item_id` resolving to one
    /// title) requires `ManageTitles` on that title's library; a facet- or
    /// library-wide request requires `ManageCatalogSettings`.
    async fn authorize_acquisition_search(
        &self,
        actor: &User,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<()> {
        if let Some(title_id) = self.acquisition_search_scoped_title(request).await? {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            return self
                .require_library_permission(
                    actor,
                    &title.library_id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await;
        }
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await
    }

    /// The single title a request is scoped to, if any — the explicit `title_id` or
    /// the title behind a `wanted_item_id`. `None` for a facet/library-wide request.
    async fn acquisition_search_scoped_title(
        &self,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<Option<String>> {
        if let Some(title_id) = request
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(title_id.to_string()));
        }
        if let Some(identifier) = request
            .wanted_item_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(self
                .resolve_scope_identifier(identifier)
                .await?
                .map(|(title_id, _)| title_id));
        }
        Ok(None)
    }

    /// The set of scopes an acquisition-search request targets. `wanted_item_id`
    /// yields exactly one scope; otherwise the derived target set of the requested
    /// kind is filtered by facet/library/title/season.
    pub(crate) async fn resolve_acquisition_search_scopes(
        &self,
        actor: &User,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<Vec<AcquisitionSearchScope>> {
        if let Some(identifier) = request
            .wanted_item_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let (title_id, scope) = self
                .resolve_scope_identifier(identifier)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("no acquisition scope for '{identifier}'"))
                })?;
            let label = self.acquisition_scope_label(&title_id, &scope).await;
            let scope_key = convergence_scope_key(&scope, &title_id).unwrap_or_default();
            return Ok(vec![AcquisitionSearchScope {
                title_id,
                scope,
                scope_key,
                label,
            }]);
        }

        let authorized = self
            .list_libraries_for_permission(
                actor,
                request.facet.clone(),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut authorized_ids = authorized
            .into_iter()
            .map(|library| library.id)
            .collect::<HashSet<_>>();
        let requested_ids = request
            .library_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect::<HashSet<_>>();
        if !requested_ids.is_empty() {
            authorized_ids.retain(|id| requested_ids.contains(id));
        }
        if authorized_ids.is_empty() {
            return Ok(Vec::new());
        }

        let facet_filter = request
            .facet
            .as_ref()
            .map(|facet| facet.as_str().to_string());
        let season_filter = request.season_number.map(|value| value.to_string());
        let current = self.current_wanted_projection(request.wanted_kind).await?;
        let excluded_scope_keys = self.non_wanted_state_scope_keys().await?;
        let scopes = current
            .iter()
            .filter(|view| !excluded_scope_keys.contains(&view.scope_key))
            .filter(|view| authorized_ids.contains(&view.library_id))
            .filter(|view| {
                facet_filter
                    .as_deref()
                    .is_none_or(|facet| view.facet.as_str() == facet)
            })
            .filter(|view| {
                request
                    .title_id
                    .as_deref()
                    .is_none_or(|title_id| view.title_id == title_id)
            })
            .filter(|view| {
                season_filter
                    .as_deref()
                    .is_none_or(|season| view.season_number.as_deref() == Some(season))
            })
            .filter_map(|view| {
                let scope = submission_scope_for_view(view)?;
                Some(AcquisitionSearchScope {
                    label: view
                        .title_name
                        .clone()
                        .unwrap_or_else(|| view.title_id.clone()),
                    title_id: view.title_id.clone(),
                    scope,
                    scope_key: view.scope_key.clone(),
                })
            })
            .collect();
        Ok(scopes)
    }

    /// Resolve a wanted identifier — a state-row id, else a convergence scope key —
    /// into `(title_id, SubmissionScope)`. Scope-key prefixes are parsed directly;
    /// an `episode:` key loads the episode to recover its title.
    pub(crate) async fn resolve_scope_identifier(
        &self,
        identifier: &str,
    ) -> AppResult<Option<(String, SubmissionScope)>> {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Ok(None);
        }

        // State-row id first.
        if let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(identifier)
            .await?
        {
            let scope = SubmissionScope::from_persisted(
                &item.title_id,
                item.episode_id.clone(),
                item.collection_id.clone(),
                item.series_movie_link_id.clone(),
                None,
            );
            return Ok(Some((item.title_id, scope)));
        }

        // Otherwise a convergence scope key.
        if let Some(episode_id) = identifier.strip_prefix("episode:") {
            let Some(episode) = self
                .services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                episode.title_id,
                SubmissionScope::Episode {
                    episode_id: episode_id.to_string(),
                },
            )));
        }
        if let Some(title_id) = identifier.strip_prefix("title:") {
            return Ok(Some((title_id.to_string(), SubmissionScope::Title)));
        }
        if let Some(link_id) = identifier.strip_prefix("series_movie:") {
            let Some(link) = self
                .services
                .catalog
                .shows
                .get_series_movie_link_by_id(link_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                link.series_title_id,
                SubmissionScope::SeriesMovie {
                    series_movie_link_id: link_id.to_string(),
                },
            )));
        }
        if let Some(collection_id) = identifier.strip_prefix("collection:") {
            let Some(collection) = self
                .services
                .catalog
                .shows
                .get_collection_by_id(collection_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                collection.title_id,
                SubmissionScope::Collection {
                    collection_id: collection_id.to_string(),
                },
            )));
        }
        Ok(None)
    }

    /// Resolve a wanted identifier (state-row id, else convergence scope key) to a
    /// persisted acquisition-state row, creating one if the scope has none yet
    ///.
    /// Returns the loaded row so callers see its real id/status.
    pub(crate) async fn resolve_or_create_wanted_state_row(
        &self,
        identifier: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        // An existing state-row id resolves directly.
        if let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(identifier.trim())
            .await?
        {
            return Ok(Some(item));
        }

        let Some((title_id, scope)) = self.resolve_scope_identifier(identifier).await? else {
            return Ok(None);
        };
        // Already a row for this scope? (e.g. an episode key whose row exists.)
        let (episode_id, collection_id, series_movie_link_id) = match &scope {
            SubmissionScope::Episode { episode_id } => (Some(episode_id.clone()), None, None),
            SubmissionScope::Collection { collection_id } => {
                (None, Some(collection_id.clone()), None)
            }
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => (None, None, Some(series_movie_link_id.clone())),
            _ => (None, None, None),
        };
        if let Some(existing) = self
            .find_wanted_state_for_scope(
                &title_id,
                episode_id.as_deref(),
                collection_id.as_deref(),
                series_movie_link_id.as_deref(),
            )
            .await?
        {
            return Ok(Some(existing));
        }

        let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
            return Ok(None);
        };
        let (media_type, season_number) = match &scope {
            SubmissionScope::Episode { episode_id } => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?;
                ("episode", episode.and_then(|episode| episode.season_number))
            }
            SubmissionScope::SeriesMovie { .. } => ("series_movie", Some("0".to_string())),
            SubmissionScope::Collection { .. } => ("episode", None),
            _ => ("movie", None),
        };
        let view = self.new_wanted_state_view(
            &title,
            media_type,
            episode_id,
            collection_id,
            series_movie_link_id,
            season_number,
        );
        let row_id = self
            .services
            .workflow
            .acquisition_scope_states
            .ensure_acquisition_scope_state(&view)
            .await?;
        self.services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&row_id)
            .await
    }

    async fn acquisition_scope_label(&self, title_id: &str, _scope: &SubmissionScope) -> String {
        self.services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten()
            .map(|title| title.name)
            .unwrap_or_else(|| title_id.to_string())
    }

    async fn run_acquisition_search_job(
        &self,
        mut run: JobRunRecord,
        actor: User,
        actor_event: DomainEventActor,
        plan: AcquisitionSearchPlan,
        cancellation: tokio_util::sync::CancellationToken,
        _search_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let scopes = match plan {
            AcquisitionSearchPlan::TitleWalk {
                title_id,
                season_number,
                scope_keys,
            } => {
                let outcome = self
                    .run_acquisition_search_title_walk(
                        &mut run,
                        &title_id,
                        season_number,
                        &scope_keys,
                        &cancellation,
                    )
                    .await;
                self.finish_acquisition_search_job(
                    run,
                    actor_event,
                    outcome.total,
                    outcome.processed,
                    outcome.grabbed,
                    outcome.failed,
                    outcome.submit_unavailable,
                    cancellation.is_cancelled(),
                )
                .await;
                return;
            }
            AcquisitionSearchPlan::Scopes(scopes) => scopes,
        };

        let total = scopes.len();
        let mut processed = 0usize;
        let mut grabbed = 0usize;
        let mut failed = 0usize;
        let mut submit_unavailable = false;
        let mut cancelled = false;

        for scope in scopes {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }
            let _ = self
                .update_acquisition_search_progress(
                    &mut run,
                    AcquisitionSearchProgress {
                        state: "running".to_string(),
                        total,
                        processed,
                        grabbed_count: grabbed,
                        failed_count: failed,
                        current_title: Some(scope.label.clone()),
                    },
                )
                .await;

            // Interactive intent: `queue_best_release` runs the Auto search+grab
            // path (bypasses the background convergence read-gate) and records
            // coverage via the search hook. A search that finds nothing grabbable is
            // a completed search, not a failure.
            match self
                .queue_best_release(
                    &actor,
                    &scope.title_id,
                    scope.scope.clone(),
                    SubmissionConflictPolicy::Skip,
                )
                .await
            {
                Ok(QueueDownloadOutcome::Queued(_)) => grabbed += 1,
                Ok(QueueDownloadOutcome::Conflict(_)) => {}
                Err(error) if !scope_search_error_is_failure(&error) => {}
                Err(error) => {
                    failed += 1;
                    if error.is_retryable_download_submit_failure() {
                        submit_unavailable = true;
                    }
                    tracing::warn!(
                        title_id = scope.title_id.as_str(),
                        error = %error,
                        "acquisition search job: scope search failed"
                    );
                }
            }
            processed += 1;
        }

        self.finish_acquisition_search_job(
            run,
            actor_event,
            total,
            processed,
            grabbed,
            failed,
            submit_unavailable,
            cancelled,
        )
        .await;
    }

    /// Run a title-scoped request as the staged acquisition walk and report the
    /// counts the job's terminal status is computed from.
    ///
    /// The walk's progress callback is synchronous — it runs inside the walk,
    /// which cannot await a database write mid-stage — so steps arrive over a
    /// channel and are written out here, concurrently with the walk. That keeps
    /// progress at stage-and-scope granularity without the walk having to know
    /// what a job run is.
    async fn run_acquisition_search_title_walk(
        &self,
        run: &mut JobRunRecord,
        title_id: &str,
        season_number: Option<u32>,
        scope_keys: &HashSet<String>,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> AcquisitionSearchWalkOutcome {
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<
            crate::acquisition_workflow::AcquisitionWalkProgress,
        >();
        let walk = crate::acquisition_workflow::run_interactive_title_acquisition_walk(
            self,
            title_id,
            season_number,
            Some(scope_keys),
            cancellation.clone(),
            move |progress| {
                // A closed receiver only means the job stopped reading progress;
                // the walk itself carries on.
                let _ = progress_tx.send(progress);
            },
        );
        tokio::pin!(walk);

        let mut total = scope_keys.len();
        let mut processed = 0usize;
        let result = loop {
            tokio::select! {
                Some(progress) = progress_rx.recv() => {
                    total = progress.total.max(progress.processed);
                    processed = progress.processed;
                    let _ = self
                        .update_acquisition_search_progress(
                            run,
                            AcquisitionSearchProgress {
                                state: "running".to_string(),
                                total,
                                processed,
                                grabbed_count: 0,
                                failed_count: 0,
                                current_title: Some(progress.stage_label),
                            },
                        )
                        .await;
                }
                result = &mut walk => break result,
            }
        };
        // Steps the walk emitted after the last poll of the channel.
        while let Ok(progress) = progress_rx.try_recv() {
            total = progress.total.max(progress.processed);
            processed = progress.processed;
        }

        match result {
            Ok(stats) => AcquisitionSearchWalkOutcome {
                total: total.max(stats.stages),
                processed: stats.stages,
                // Both halves count: the grabs the walk committed inline and the
                // proposals the end-of-walk arbitration committed.
                grabbed: stats.inline_grabs + stats.committed,
                // Work items whose submissions all failed. Without this a job
                // whose every submission was refused — a mapped download client
                // that is globally disabled, say — would terminate `completed`
                // with nothing grabbed, which is exactly what the old per-scope
                // path reported as failed.
                failed: stats.failed,
                submit_unavailable: stats.submit_unavailable,
            },
            Err(error) => {
                let is_failure = scope_search_error_is_failure(&error);
                if is_failure {
                    tracing::warn!(
                        title_id,
                        error = %error,
                        "acquisition search job: title walk failed"
                    );
                }
                AcquisitionSearchWalkOutcome {
                    total: total.max(1),
                    // A walk that failed outright reports one processed unit so
                    // an all-failed job still reads as failed rather than as a
                    // job that did nothing.
                    processed: processed.max(1),
                    grabbed: 0,
                    failed: usize::from(is_failure),
                    submit_unavailable: error.is_retryable_download_submit_failure(),
                }
            }
        }
    }

    async fn update_acquisition_search_progress(
        &self,
        run: &mut JobRunRecord,
        progress: AcquisitionSearchProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_acquisition_search_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        total: usize,
        processed: usize,
        grabbed: usize,
        failed: usize,
        submit_unavailable: bool,
        cancelled: bool,
    ) {
        let status = acquisition_search_job_status(
            grabbed,
            processed,
            failed,
            submit_unavailable,
            cancelled,
        );
        let state = acquisition_search_state_for_status(status, cancelled);
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&AcquisitionSearchProgress {
            state: state.to_string(),
            total,
            processed,
            grabbed_count: grabbed,
            failed_count: failed,
            current_title: None,
        })
        .ok();
        run.summary_text = Some(if cancelled {
            format!("Acquisition search cancelled after {processed} scope(s); grabbed {grabbed}")
        } else {
            format!("Searched {processed} scope(s); grabbed {grabbed}, failed {failed}")
        });
        run.error_text =
            (status == JobRunStatus::Failed).then(|| "all acquisition searches failed".to_string());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;

        match self.services.events.job_runs.update_job_run(&run).await {
            Ok(updated) => {
                self.runtime
                    .jobs
                    .job_run_tracker
                    .upsert_active_run(JobRun::from_record(&updated, None))
                    .await;
                let payload = if status == JobRunStatus::Failed {
                    DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                        run_id: updated.id.clone(),
                        job_key: updated.job_key.as_str().to_string(),
                        error_text: updated.error_text.clone(),
                    })
                } else {
                    DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                        run_id: updated.id.clone(),
                        job_key: updated.job_key.as_str().to_string(),
                        summary_text: updated.summary_text.clone(),
                    })
                };
                let _ = self
                    .append_domain_event(crate::domain_events::new_job_run_domain_event(
                        actor,
                        updated.id.clone(),
                        payload,
                    ))
                    .await;
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to finish acquisition search job");
            }
        }

        self.runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .remove(&run.id);
    }

    /// The current state of an interactive acquisition-search job,
    /// for the `acquisitionSearchJob` query. Visible to any actor who may read job
    /// runs (`ManageSystemSettings`, matching the jobs surface).
    pub async fn acquisition_search_job(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<Option<AcquisitionSearchJobView>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let Some(run) = self.services.events.job_runs.get_job_run(run_id).await? else {
            return Ok(None);
        };
        if run.job_key != JobKey::AcquisitionSearch {
            return Ok(None);
        }
        Ok(Some(self.acquisition_search_job_view(&run)))
    }

    fn acquisition_search_job_view(&self, run: &JobRunRecord) -> AcquisitionSearchJobView {
        let progress = run
            .progress_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<AcquisitionSearchProgress>(json).ok());
        let state = progress
            .as_ref()
            .map(|progress| progress.state.clone())
            .unwrap_or_else(|| acquisition_search_state_for_status(run.status, false).to_string());
        AcquisitionSearchJobView {
            id: run.id.clone(),
            state,
            total: progress.as_ref().map(|p| p.total as i32).unwrap_or(0),
            processed: progress.as_ref().map(|p| p.processed as i32).unwrap_or(0),
            grabbed_count: progress
                .as_ref()
                .map(|p| p.grabbed_count as i32)
                .unwrap_or(0),
            failed_count: progress
                .as_ref()
                .map(|p| p.failed_count as i32)
                .unwrap_or(0),
            current_title: progress.and_then(|p| p.current_title),
            started_at: run.started_at.to_rfc3339(),
            finished_at: run.completed_at.map(|at| at.to_rfc3339()),
        }
    }

    /// Cancel a running interactive acquisition-search job. Requires
    /// `ManageSystemSettings` (the jobs surface); signals the job's cancellation
    /// token so it stops between scopes. Returns whether a running job was signalled.
    pub async fn cancel_acquisition_search(&self, actor: &User, run_id: &str) -> AppResult<bool> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let token = self
            .runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .get(run_id)
            .cloned();
        match token {
            Some(token) => {
                token.cancel();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// The best-release search scope for a derived view row. Episodes/movies/series
/// movies map to their single-scope submission target; collection/pack rows are
/// not individually searchable by this job (handled by the cursor), so they are
/// skipped.
fn submission_scope_for_view(view: &WantedScopeView) -> Option<SubmissionScope> {
    match view.media_type.as_str() {
        "episode" => view
            .episode_id
            .clone()
            .map(|episode_id| SubmissionScope::Episode { episode_id }),
        "series_movie" => view
            .series_movie_link_id
            .clone()
            .map(|series_movie_link_id| SubmissionScope::SeriesMovie {
                series_movie_link_id,
            }),
        "movie" => Some(SubmissionScope::Title),
        _ => None,
    }
}

/// Deterministic order: title name, then numeric season, then numeric episode —
/// the same ordering the cutoff-unmet view uses.
fn sort_wanted_views(rows: &mut [WantedScopeView]) {
    rows.sort_by(|left, right| {
        left.title_name
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(
                &right
                    .title_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            )
            .then_with(|| {
                parse_sort_number(left.season_number.as_deref())
                    .cmp(&parse_sort_number(right.season_number.as_deref()))
            })
            .then_with(|| {
                parse_sort_number(left.episode_number.as_deref())
                    .cmp(&parse_sort_number(right.episode_number.as_deref()))
            })
            .then_with(|| left.scope_key.cmp(&right.scope_key))
    });
}
