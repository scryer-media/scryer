//! Derived acquisition targets. A scope is a target iff it is
//! monitored and its current primary file does not satisfy the effective
//! requirements — computed from library state on demand, never materialized.
//! `missing` targets have no primary file; `cutoff_upgrade` targets have a file
//! strictly below the profile cutoff. The background acquisition cursor and the
//! wanted views read this same derivation, so the searcher and the UI agree on
//! the target set by construction. A scope whose file satisfies requirements is
//! never in this set and is therefore never actively searched (§D1a).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use scryer_domain::MediaFacet;

use super::*;
use crate::acquisition::convergence::convergence_scope_key;
use crate::acquisition_policy::{episode_search_window_is_open, parse_schedule_baseline_date};
use crate::contracts::SubmissionScope;

/// An aired episode stays **hot** (converges promptly, high candidate value to
/// the plan-112 scheduler) this long after air; beyond it the scope drains via
/// the paced cold lane.
const HOT_EPISODE_AIR_WINDOW_DAYS: i64 = 14;

/// A released movie stays hot this long after its digital/theatrical date.
const HOT_MOVIE_RELEASE_WINDOW_DAYS: i64 = 30;

/// A freshly added title's scopes are hot this long after the add, regardless
/// of air/release age — the user just asked for it, so it converges promptly
/// (still paced per-host by the scheduler).
const HOT_RECENTLY_ADDED_WINDOW_DAYS: i64 = 3;

/// One derived acquisition target: a monitored scope whose current files do not
/// satisfy requirements, plus the coordinates the acquisition cursor and the
/// search pipeline need to act on it.
#[derive(Clone, Debug)]
pub struct AcquisitionTarget {
    /// Stable coverage key (`convergence_scope_key`) shared by cursor rotation
    /// and the convergence ledger.
    pub scope_key: String,
    pub title_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    /// "movie" | "episode" | "series_movie" — drives query building downstream.
    pub media_type: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    /// Recent air/release/add → hot lane (high candidate value); long-tail and
    /// upgrades → cold lane (low value, drained under scheduler backpressure).
    pub is_hot: bool,
    /// Whether this scope is hot because *its own* air/release date is recent,
    /// rather than only because its title was added in the last few days.
    ///
    /// Both halves are hot, but they are not equally urgent. A freshly added
    /// series contributes its whole back catalog to the hot lane at once, and a
    /// newly aired episode of a show that is already in the library must not
    /// queue behind that block. The batch selector therefore orders air-date
    /// heat ahead of recently-added heat (§D3).
    pub hot_by_air_date: bool,
    /// Whether a primary file already occupies this scope.
    ///
    /// The two derivations differ by exactly this: `derive_missing_targets`
    /// yields scopes with nothing on disk, `derive_cutoff_targets` yields scopes
    /// that have a file but sit below cutoff. Recording it here replaces reading
    /// `wanted_items.current_score.is_some()` as a proxy for "something landed",
    /// which was true in only one of that column's five lifecycle states.
    pub occupied: bool,
}

/// Whether `date` (RFC3339 or `YYYY-MM-DD`) falls within the trailing
/// `window_days` before `now`. Future dates count as recent: an episode inside
/// its pre-air window is the hottest target there is.
fn date_is_recent(date: Option<&str>, now: &DateTime<Utc>, window_days: i64) -> bool {
    parse_schedule_baseline_date(date)
        .is_some_and(|baseline| *now - baseline < Duration::days(window_days))
}

/// Whether a movie has reached its configured availability threshold and may be
/// acquired. `announced` (the default) is always available; `in_cinemas` and
/// `released` gate on the corresponding release dates.
pub(crate) fn movie_is_available_for_acquisition(
    first_aired: Option<&str>,
    digital_release_date: Option<&str>,
    availability: &str,
    now: &DateTime<Utc>,
) -> bool {
    match availability {
        "in_cinemas" => first_aired
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|date| date <= now.date_naive())
            .unwrap_or(false),
        "released" => {
            if let Some(digital) = digital_release_date {
                chrono::NaiveDate::parse_from_str(digital, "%Y-%m-%d")
                    .map(|d| d <= now.date_naive())
                    .unwrap_or(false)
            } else if let Some(first_aired) = first_aired {
                // Fallback: theatrical + 90 days ≈ digital availability.
                chrono::NaiveDate::parse_from_str(first_aired, "%Y-%m-%d")
                    .map(|d| d + Duration::days(90) <= now.date_naive())
                    .unwrap_or(false)
            } else {
                false
            }
        }
        // "announced" or anything else: available as soon as it is monitored.
        _ => true,
    }
}

/// The batch to evaluate this cycle plus the new per-lane resume positions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CursorSelection {
    /// Indices into the input targets, in evaluation order (hot first — air-date
    /// heat before recently-added heat — then cold in rotation order).
    pub indices: Vec<usize>,
    /// The cold scope_key to resume *after* next cycle (rotating cursor),
    /// carried forward unchanged when the cold lane did not advance.
    pub resume_after: Option<String>,
    /// The hot scope_key to resume *after* next cycle. The hot lane rotates for
    /// the same reason the cold one does: a hot set larger than `max_scopes`
    /// would otherwise re-evaluate its head every cycle and never reach its
    /// tail. Carried forward unchanged when the hot lane did not advance.
    pub hot_resume_after: Option<String>,
}

/// Numeric `(season, episode)` walk order for a target's scope work.
///
/// Derived order is the datastore's row order — `list_missing_scope_candidates`
/// orders by episode id, which is a random UUID — so a title's episodes arrive
/// shuffled. Sorting on this key makes a title's walk predictable: S01E01
/// first, unnumbered scopes last, and a stable sort keeps the derived order
/// among equals.
pub(crate) fn scope_walk_order_key(target: &AcquisitionTarget) -> (u32, u32) {
    fn parse(value: Option<&String>) -> u32 {
        value
            .map(|value| value.trim())
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    }

    (
        parse(target.season_number.as_ref()),
        parse(target.episode_number.as_ref()),
    )
}

/// Select the background acquisition batch for one cycle. Scheduler backpressure is
/// the pace; this only bounds how many scopes get *evaluated* (coverage lookup,
/// routing resolve, fingerprint compute) per tick:
/// - **hot targets first** (recent = high candidate value), rotating after
///   `hot_resume_after` so a hot set larger than `max_scopes` cannot starve its
///   tail, and ordered air-date heat before recently-added heat;
/// - then **cold targets rotating** after `resume_after` (wrapping once), so
///   every cold scope gets a turn and a stuck head never starves the tail;
/// - stop once `max_scopes` are selected — the evaluation cost ceiling, sized
///   above the scheduler's per-tick admission capacity so it never becomes the
///   effective rate limiter.
///
/// Rotation decides *which* hot targets get a turn; the priority sort decides
/// the order they are walked in. Keeping those separate is what lets the hot
/// lane both drain fairly and still put a newly aired episode ahead of a
/// freshly added title's back catalog on every cycle, wherever the cursor
/// happens to sit.
///
/// Host availability is checked during evaluation (after routing resolves),
/// not here — the enumeration stays cheap. Returns the new resume positions:
/// the last scope_key each lane *considered*, so both cursors always advance.
pub(crate) fn select_background_acquisition_batch(
    targets: &[AcquisitionTarget],
    hot_resume_after: Option<&str>,
    resume_after: Option<&str>,
    max_scopes: usize,
) -> CursorSelection {
    let mut selection = CursorSelection {
        indices: Vec::new(),
        resume_after: resume_after.map(str::to_string),
        hot_resume_after: hot_resume_after.map(str::to_string),
    };
    if max_scopes == 0 {
        return selection;
    }

    // Hot targets: always considered first, rotating after `hot_resume_after`
    // and wrapping once. They do not move the cold rotation cursor.
    let hot: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| target.is_hot)
        .map(|(index, _)| index)
        .collect();
    if !hot.is_empty() {
        let start = hot_resume_after
            .and_then(|after| {
                hot.iter()
                    .position(|index| targets[*index].scope_key == after)
            })
            .map(|position| position + 1)
            .unwrap_or(0);
        let mut selected_hot = Vec::new();
        for offset in 0..hot.len() {
            if selected_hot.len() >= max_scopes {
                break;
            }
            let index = hot[(start + offset) % hot.len()];
            selection.hot_resume_after = Some(targets[index].scope_key.clone());
            selected_hot.push(index);
        }
        // Air-date heat leads, then recently-added heat, each in derived order.
        // `sort_by_key` is stable, and the index tie-break makes the order total
        // so a wrap never leaves the batch ordered by where the cursor sat.
        selected_hot.sort_by_key(|index| (!targets[*index].hot_by_air_date, *index));
        selection.indices = selected_hot;
        if selection.indices.len() >= max_scopes {
            return selection;
        }
    }

    // Cold targets: rotate starting after `resume_after`, wrapping once.
    let cold: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(|(_, target)| !target.is_hot)
        .map(|(index, _)| index)
        .collect();
    if cold.is_empty() {
        return selection;
    }
    let start = resume_after
        .and_then(|after| {
            cold.iter()
                .position(|index| targets[*index].scope_key == after)
        })
        .map(|position| position + 1)
        .unwrap_or(0);
    for offset in 0..cold.len() {
        if selection.indices.len() >= max_scopes {
            break;
        }
        let index = cold[(start + offset) % cold.len()];
        selection.resume_after = Some(targets[index].scope_key.clone());
        selection.indices.push(index);
    }
    selection
}

impl AppUseCase {
    /// The acquisition-state row for a scope, if any. Dispatch mirrors the
    /// state row's uniqueness shapes: episode first (an episode target may also
    /// carry its collection id), then series-movie link, then collection, then
    /// the bare title.
    pub(crate) async fn find_wanted_state_for_scope(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        collection_id: Option<&str>,
        series_movie_link_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        let repo = &self.services.workflow.acquisition_scope_states;
        if episode_id.is_some() {
            return repo
                .get_acquisition_scope_state_for_title(title_id, episode_id)
                .await;
        }
        if let Some(link_id) = series_movie_link_id {
            return Ok(repo
                .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                    title_id: Some(title_id.to_string()),
                    limit: 500,
                    ..AcquisitionScopeStatesQuery::default()
                })
                .await?
                .into_iter()
                .find(|existing| existing.series_movie_link_id.as_deref() == Some(link_id)));
        }
        if let Some(collection_id) = collection_id {
            return Ok(repo
                .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                    title_id: Some(title_id.to_string()),
                    limit: 500,
                    ..AcquisitionScopeStatesQuery::default()
                })
                .await?
                .into_iter()
                .find(|existing| existing.collection_id.as_deref() == Some(collection_id)));
        }
        repo.get_acquisition_scope_state_for_title(title_id, None)
            .await
    }

    /// The derived missing-target set (§D1): monitored scopes with no primary
    /// file that have crossed their availability gate — episodes inside the
    /// pre-air window, movies past `min_availability`, series-movie links with
    /// the filler opt-in honored.
    pub(crate) async fn derive_missing_targets(
        &self,
        now: &DateTime<Utc>,
    ) -> AppResult<Vec<AcquisitionTarget>> {
        let candidates = self
            .services
            .library
            .media_files
            .list_missing_scope_candidates()
            .await?;
        let mut targets = Vec::new();

        for episode in candidates.episodes {
            let Some(facet) = MediaFacet::parse(&episode.title_facet) else {
                continue;
            };
            if !self
                .facet_registry
                .get(&facet)
                .is_some_and(|handler| handler.has_episodes())
            {
                continue;
            }
            // An unaired (or undated) episode is not yet available — RSS still
            // grabs an early posting; active search waits for the air window.
            if !episode_search_window_is_open(episode.air_date.as_deref(), now) {
                continue;
            }
            let scope = SubmissionScope::Episode {
                episode_id: episode.episode_id.clone(),
            };
            let Some(scope_key) = convergence_scope_key(&scope, &episode.title_id) else {
                continue;
            };
            let hot_by_air_date = date_is_recent(
                episode.air_date.as_deref(),
                now,
                HOT_EPISODE_AIR_WINDOW_DAYS,
            );
            let is_hot = hot_by_air_date
                || date_is_recent(
                    Some(episode.title_created_at.as_str()),
                    now,
                    HOT_RECENTLY_ADDED_WINDOW_DAYS,
                );
            targets.push(AcquisitionTarget {
                occupied: false,
                scope_key,
                title_id: episode.title_id,
                library_id: episode.library_id,
                facet,
                media_type: "episode".to_string(),
                episode_id: Some(episode.episode_id),
                collection_id: episode.collection_id,
                series_movie_link_id: None,
                season_number: episode.season_number,
                episode_number: episode.episode_number,
                is_hot,
                hot_by_air_date,
            });
        }

        for title in candidates.titles {
            let Some(facet) = MediaFacet::parse(&title.title_facet) else {
                continue;
            };
            // Episodic facets acquire per episode/link; a fileless series title
            // is not itself a target.
            if self
                .facet_registry
                .get(&facet)
                .is_some_and(|handler| handler.has_episodes())
            {
                continue;
            }
            let availability = title.min_availability.as_deref().unwrap_or("announced");
            if !movie_is_available_for_acquisition(
                title.first_aired.as_deref(),
                title.digital_release_date.as_deref(),
                availability,
                now,
            ) {
                continue;
            }
            let Some(scope_key) = convergence_scope_key(&SubmissionScope::Title, &title.title_id)
            else {
                continue;
            };
            let release_date = title
                .digital_release_date
                .as_deref()
                .or(title.first_aired.as_deref());
            let hot_by_air_date = date_is_recent(release_date, now, HOT_MOVIE_RELEASE_WINDOW_DAYS);
            let is_hot = hot_by_air_date
                || date_is_recent(
                    Some(title.created_at.as_str()),
                    now,
                    HOT_RECENTLY_ADDED_WINDOW_DAYS,
                );
            targets.push(AcquisitionTarget {
                occupied: false,
                scope_key,
                title_id: title.title_id,
                library_id: title.library_id,
                facet,
                media_type: "movie".to_string(),
                episode_id: None,
                collection_id: None,
                series_movie_link_id: None,
                season_number: None,
                episode_number: None,
                is_hot,
                hot_by_air_date,
            });
        }

        // Filler links are opt-in per library; resolve the setting once per
        // library instead of per link.
        let mut filler_allowed_by_library: HashMap<String, bool> = HashMap::new();
        for link in candidates.series_movie_links {
            if link.continuity_status.as_deref() == Some("filler") {
                let allowed = match filler_allowed_by_library.get(&link.library_id) {
                    Some(allowed) => *allowed,
                    None => {
                        let allowed = self
                            .resolve_library_bool_setting(
                                "anime.monitor_filler_movies",
                                Some(&link.library_id),
                                Some(MediaFacet::Anime.as_str()),
                                false,
                            )
                            .await
                            .unwrap_or(false);
                        filler_allowed_by_library.insert(link.library_id.clone(), allowed);
                        allowed
                    }
                };
                if !allowed {
                    continue;
                }
            }
            let Some(facet) = MediaFacet::parse(&link.title_facet) else {
                continue;
            };
            let scope = SubmissionScope::SeriesMovie {
                series_movie_link_id: link.series_movie_link_id.clone(),
            };
            let Some(scope_key) = convergence_scope_key(&scope, &link.title_id) else {
                continue;
            };
            let hot_by_air_date = date_is_recent(
                link.movie_digital_release_date.as_deref(),
                now,
                HOT_MOVIE_RELEASE_WINDOW_DAYS,
            );
            let is_hot = hot_by_air_date
                || date_is_recent(
                    Some(link.link_created_at.as_str()),
                    now,
                    HOT_RECENTLY_ADDED_WINDOW_DAYS,
                );
            targets.push(AcquisitionTarget {
                occupied: false,
                scope_key,
                title_id: link.title_id,
                library_id: link.library_id,
                facet,
                media_type: "series_movie".to_string(),
                episode_id: None,
                collection_id: None,
                series_movie_link_id: Some(link.series_movie_link_id),
                season_number: Some("0".to_string()),
                episode_number: None,
                is_hot,
                hot_by_air_date,
            });
        }

        Ok(targets)
    }

    /// Cutoff-upgrade targets across every library (§D1): scopes whose primary
    /// file sits strictly below the effective profile cutoff. Always cold — the
    /// file already plays, so upgrades drain at scheduler leisure.
    pub(crate) async fn derive_cutoff_targets(&self) -> AppResult<Vec<AcquisitionTarget>> {
        let items = self.compute_cutoff_unmet_items(None, None).await?;
        let mut targets: Vec<AcquisitionTarget> = items
            .into_iter()
            .filter_map(|item| {
                let scope = SubmissionScope::from_persisted(
                    &item.title_id,
                    item.episode_id.clone(),
                    None,
                    None,
                    None,
                );
                let scope_key = convergence_scope_key(&scope, &item.title_id)?;
                let media_type = if item.episode_id.is_some() {
                    "episode"
                } else {
                    "movie"
                };
                Some(AcquisitionTarget {
                    occupied: true,
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
                    is_hot: false,
                    hot_by_air_date: false,
                })
            })
            .collect();

        // D19's second half: a scope whose *quality* is at cutoff but whose
        // score is below the profile's `cutoff_score` is still an upgrade
        // target. Appended rather than folded into `compute_cutoff_unmet_items`
        // — that function also backs the operator-facing "cutoff unmet" listing,
        // which is about quality tiers and would start reporting scopes whose
        // tier is fine.
        let mut seen: HashSet<String> = targets
            .iter()
            .map(|target| target.scope_key.clone())
            .collect();
        for target in self.derive_format_cutoff_targets().await? {
            if seen.insert(target.scope_key.clone()) {
                targets.push(target);
            }
        }
        Ok(targets)
    }

    /// Occupied scopes whose re-derived bar sits below their profile's
    /// `cutoff_score` (D19).
    ///
    /// **The one library-wide re-scoring pass in the design**, so it is gated:
    /// unless some profile actually sets `cutoff_score` it does no work at all,
    /// which is the state every existing install is in. When it does run it
    /// reuses `landed_bars_for_scopes` — the same batched derivation the Wanted
    /// page uses and, since MA4, the same number the grab gate compares against
    /// — in pages, so a large library re-scores in bounded chunks rather than
    /// all at once.
    async fn derive_format_cutoff_targets(&self) -> AppResult<Vec<AcquisitionTarget>> {
        /// How many scopes are re-scored per batch. Each page is one media-file
        /// query plus one scoring context per distinct title on it.
        const PAGE: usize = 200;

        let libraries = self.services.catalog.libraries.list(None).await?;
        let library_ids: Vec<String> = libraries.iter().map(|library| library.id.clone()).collect();
        let titles = self
            .monitored_titles_with_profiles(None, &library_ids)
            .await?;
        let scored: Vec<(scryer_domain::Title, i32)> = titles
            .into_iter()
            .filter(|(_, profile)| profile.criteria.allow_upgrades)
            .filter_map(|(title, profile)| {
                profile
                    .criteria
                    .cutoff_score
                    .map(|cutoff_score| (title, cutoff_score))
            })
            .collect();
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        let cutoff_by_title: HashMap<&str, i32> = scored
            .iter()
            .map(|(title, cutoff)| (title.id.as_str(), *cutoff))
            .collect();
        let title_by_id: HashMap<&str, &scryer_domain::Title> = scored
            .iter()
            .map(|(title, _)| (title.id.as_str(), title))
            .collect();
        let title_ids: Vec<String> = scored.iter().map(|(title, _)| title.id.clone()).collect();

        // The same enumeration the quality sweep uses: one row per occupied
        // scope, already narrowed to titles whose profile asks the question.
        let summaries = self
            .services
            .library
            .media_files
            .list_cutoff_unmet_quality_summaries(&title_ids)
            .await?;

        let mut targets = Vec::new();
        for page in summaries.chunks(PAGE) {
            let scopes: Vec<crate::acquisition_workflow::LandedBarScope> = page
                .iter()
                .map(|summary| crate::acquisition_workflow::LandedBarScope {
                    title_id: summary.title_id.clone(),
                    episode_id: summary.episode_id.clone(),
                    collection_id: None,
                    series_movie_link_id: None,
                })
                .collect();
            let bars = self.landed_bars_for_scopes(&scopes).await;
            for (summary, bar) in page.iter().zip(bars) {
                // No bar means nothing scored for this scope; a scope with no
                // number is not evidence that the number is low.
                let Some(bar) = bar else { continue };
                let Some(cutoff_score) = cutoff_by_title.get(summary.title_id.as_str()).copied()
                else {
                    continue;
                };
                if bar >= cutoff_score {
                    continue;
                }
                let Some(title) = title_by_id.get(summary.title_id.as_str()) else {
                    continue;
                };
                if summary.episode_id.is_none() && title.facet != MediaFacet::Movie {
                    continue;
                }
                let scope = SubmissionScope::from_persisted(
                    &title.id,
                    summary.episode_id.clone(),
                    None,
                    None,
                    None,
                );
                let Some(scope_key) = convergence_scope_key(&scope, &title.id) else {
                    continue;
                };
                targets.push(AcquisitionTarget {
                    occupied: true,
                    scope_key,
                    title_id: title.id.clone(),
                    library_id: title.library_id.clone(),
                    facet: title.facet.clone(),
                    media_type: if summary.episode_id.is_some() {
                        "episode".to_string()
                    } else {
                        "movie".to_string()
                    },
                    episode_id: summary.episode_id.clone(),
                    collection_id: None,
                    series_movie_link_id: None,
                    season_number: summary.season_number.clone(),
                    episode_number: summary.episode_number.clone(),
                    is_hot: false,
                    hot_by_air_date: false,
                });
            }
        }
        Ok(targets)
    }

    /// The full derived target set the background acquisition cursor rotates over:
    /// missing ∪ cutoff-upgrade, minus scopes the user paused. Grab-blocking
    /// (active/completed submissions) is checked per selected scope during
    /// processing, where the download-client snapshot is available.
    pub(crate) async fn derive_acquisition_targets(
        &self,
        now: &DateTime<Utc>,
    ) -> AppResult<Vec<AcquisitionTarget>> {
        let mut targets = self.derive_missing_targets(now).await?;
        targets.extend(self.derive_cutoff_targets().await?);

        let paused = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec![AcquisitionScopeStatus::Paused.as_str().to_string()],
                limit: i64::MAX,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        if !paused.is_empty() {
            let paused_keys: HashSet<String> = paused
                .iter()
                .filter_map(|item| {
                    let scope = SubmissionScope::from_persisted(
                        &item.title_id,
                        item.episode_id.clone(),
                        item.collection_id.clone(),
                        item.series_movie_link_id.clone(),
                        None,
                    );
                    convergence_scope_key(&scope, &item.title_id)
                })
                .collect();
            targets.retain(|target| !paused_keys.contains(&target.scope_key));
        }

        Ok(targets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor_target(scope_key: &str, is_hot: bool) -> AcquisitionTarget {
        AcquisitionTarget {
            occupied: false,
            scope_key: scope_key.to_string(),
            title_id: "t1".to_string(),
            library_id: "lib".to_string(),
            facet: MediaFacet::Movie,
            media_type: "movie".to_string(),
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            is_hot,
            hot_by_air_date: is_hot,
        }
    }

    /// A hot target whose heat comes only from its title having just been added.
    fn recently_added_target(scope_key: &str) -> AcquisitionTarget {
        AcquisitionTarget {
            hot_by_air_date: false,
            ..cursor_target(scope_key, true)
        }
    }

    fn selected_keys(targets: &[AcquisitionTarget], selection: &CursorSelection) -> Vec<String> {
        selection
            .indices
            .iter()
            .map(|index| targets[*index].scope_key.clone())
            .collect()
    }

    #[test]
    fn cursor_takes_hot_targets_first_then_cold() {
        let targets = vec![
            cursor_target("cold-1", false),
            cursor_target("hot-1", true),
            cursor_target("cold-2", false),
            cursor_target("hot-2", true),
        ];
        let selection = select_background_acquisition_batch(&targets, None, None, 10);
        assert_eq!(
            selected_keys(&targets, &selection),
            vec!["hot-1", "hot-2", "cold-1", "cold-2"],
            "hot targets are evaluated first, then cold in order"
        );
    }

    #[test]
    fn cursor_work_cap_bounds_the_batch_and_advances() {
        let targets = vec![
            cursor_target("a", false),
            cursor_target("b", false),
            cursor_target("c", false),
        ];
        let selection = select_background_acquisition_batch(&targets, None, None, 2);
        assert_eq!(selected_keys(&targets, &selection), vec!["a", "b"]);
        assert_eq!(
            selection.resume_after.as_deref(),
            Some("b"),
            "the cursor advances to the last cold scope it considered"
        );
    }

    #[test]
    fn cursor_rotates_and_wraps_across_cycles() {
        let targets = vec![
            cursor_target("a", false),
            cursor_target("b", false),
            cursor_target("c", false),
        ];
        let first = select_background_acquisition_batch(&targets, None, None, 2);
        assert_eq!(selected_keys(&targets, &first), vec!["a", "b"]);
        // Next cycle resumes after "b" → c, then wraps to a.
        let second =
            select_background_acquisition_batch(&targets, None, first.resume_after.as_deref(), 2);
        assert_eq!(selected_keys(&targets, &second), vec!["c", "a"]);
    }

    #[test]
    fn cursor_hot_targets_respect_the_cap_and_leave_the_cold_cursor() {
        let targets = vec![
            cursor_target("hot-1", true),
            cursor_target("hot-2", true),
            cursor_target("cold-1", false),
        ];
        // The cap is filled by hot targets, so the cold lane is not reached and
        // its rotation cursor is carried forward unchanged.
        let selection = select_background_acquisition_batch(&targets, None, Some("cold-1"), 2);
        assert_eq!(selected_keys(&targets, &selection), vec!["hot-1", "hot-2"]);
        assert_eq!(selection.resume_after.as_deref(), Some("cold-1"));
    }

    #[test]
    fn cursor_walks_air_date_heat_before_recently_added_heat() {
        // Derived order interleaves the two: a freshly added series' back
        // catalog must not push the newly aired episode down the batch.
        let targets = vec![
            recently_added_target("added-1"),
            cursor_target("aired-1", true),
            recently_added_target("added-2"),
            cursor_target("aired-2", true),
            cursor_target("cold-1", false),
        ];
        let selection = select_background_acquisition_batch(&targets, None, None, 10);
        assert_eq!(
            selected_keys(&targets, &selection),
            vec!["aired-1", "aired-2", "added-1", "added-2", "cold-1"],
            "air-date heat leads the hot lane, then recently-added heat, then cold"
        );
    }

    #[test]
    fn cursor_rotates_hot_targets_so_a_large_hot_set_cannot_starve_its_tail() {
        let targets = vec![
            cursor_target("hot-a", true),
            cursor_target("hot-b", true),
            cursor_target("hot-c", true),
        ];
        let first = select_background_acquisition_batch(&targets, None, None, 2);
        assert_eq!(selected_keys(&targets, &first), vec!["hot-a", "hot-b"]);
        assert_eq!(
            first.hot_resume_after.as_deref(),
            Some("hot-b"),
            "the hot cursor advances to the last hot scope it considered"
        );
        // Without rotation "hot-c" would never be reached: the cap is filled by
        // the head of the hot lane on every cycle.
        let second = select_background_acquisition_batch(
            &targets,
            first.hot_resume_after.as_deref(),
            None,
            2,
        );
        assert_eq!(selected_keys(&targets, &second), vec!["hot-a", "hot-c"]);
        assert_eq!(second.hot_resume_after.as_deref(), Some("hot-a"));
    }

    #[test]
    fn a_hot_rotation_still_walks_air_date_heat_first() {
        let targets = vec![
            recently_added_target("added-1"),
            recently_added_target("added-2"),
            cursor_target("aired-1", true),
        ];
        // Resuming after "added-1" takes "added-2" and "aired-1"; the batch is
        // still ordered by heat rather than by where the cursor sat.
        let selection = select_background_acquisition_batch(&targets, Some("added-1"), None, 2);
        assert_eq!(
            selected_keys(&targets, &selection),
            vec!["aired-1", "added-2"]
        );
    }

    #[test]
    fn cursor_hot_lane_carries_its_position_when_it_did_not_advance() {
        let targets = vec![cursor_target("cold-1", false)];
        let selection = select_background_acquisition_batch(&targets, Some("hot-1"), None, 4);
        assert_eq!(selected_keys(&targets, &selection), vec!["cold-1"]);
        assert_eq!(selection.hot_resume_after.as_deref(), Some("hot-1"));
    }

    #[test]
    fn scope_walk_order_sorts_by_season_then_episode_with_unparsable_last() {
        let mut target = cursor_target("scope", false);
        target.season_number = Some(" 2 ".to_string());
        target.episode_number = Some("10".to_string());
        assert_eq!(scope_walk_order_key(&target), (2, 10));

        target.episode_number = Some("special".to_string());
        assert_eq!(
            scope_walk_order_key(&target),
            (2, u32::MAX),
            "an unparsable episode number sorts last within its season"
        );

        target.season_number = None;
        target.episode_number = None;
        assert_eq!(scope_walk_order_key(&target), (u32::MAX, u32::MAX));
    }

    #[test]
    fn recent_dates_include_future_and_trailing_window() {
        let now = Utc::now();
        let yesterday = (now - Duration::days(1)).to_rfc3339();
        let last_month = (now - Duration::days(31)).to_rfc3339();
        let tomorrow = (now + Duration::days(1)).to_rfc3339();
        assert!(date_is_recent(Some(&yesterday), &now, 14));
        assert!(!date_is_recent(Some(&last_month), &now, 14));
        assert!(
            date_is_recent(Some(&tomorrow), &now, 14),
            "a pre-air/pre-release date is the hottest target there is"
        );
        assert!(!date_is_recent(None, &now, 14));
    }

    #[test]
    fn movie_availability_gates_match_thresholds() {
        let now = Utc::now();
        let past = (now - Duration::days(10)).date_naive().to_string();
        let future = (now + Duration::days(10)).date_naive().to_string();

        // announced: always available
        assert!(movie_is_available_for_acquisition(
            None,
            None,
            "announced",
            &now
        ));
        // in_cinemas: needs a past theatrical date
        assert!(movie_is_available_for_acquisition(
            Some(&past),
            None,
            "in_cinemas",
            &now
        ));
        assert!(!movie_is_available_for_acquisition(
            Some(&future),
            None,
            "in_cinemas",
            &now
        ));
        assert!(!movie_is_available_for_acquisition(
            None,
            None,
            "in_cinemas",
            &now
        ));
        // released: digital date, else theatrical + 90d
        assert!(movie_is_available_for_acquisition(
            None,
            Some(&past),
            "released",
            &now
        ));
        assert!(!movie_is_available_for_acquisition(
            None,
            Some(&future),
            "released",
            &now
        ));
        let old_theatrical = (now - Duration::days(120)).date_naive().to_string();
        assert!(movie_is_available_for_acquisition(
            Some(&old_theatrical),
            None,
            "released",
            &now
        ));
        assert!(!movie_is_available_for_acquisition(
            Some(&past),
            None,
            "released",
            &now
        ));
    }
}
