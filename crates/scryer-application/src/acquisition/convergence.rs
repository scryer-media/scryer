//! Convergence model: fingerprinted proof that an indexer's corpus was
//! completely searched for a scope under the current acquisition policy.
//!
//! A scope's *fingerprint* captures what a "correct" search is — the effective
//! quality profile (identity + a version that bumps on edits) and the required
//! audio languages (subtitles are a separate subsystem and never a factor). It
//! is stored on `scope_indexer_coverage` rows; when the live fingerprint differs
//! from a row's, that coverage is stale (the scope re-converges). The
//! fingerprint only governs *coverage staleness* — target membership
//! (missing / below-cutoff / missing-audio) is a separate, prior gate, so a
//! fingerprint change never resurrects a requirements-met scope.

use super::*;
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::app_usecase_discovery::QualityProfileLookup;
use crate::quality_profile::QualityProfileCriteria;
use scryer_domain::{IndexerConfig, Title};

/// Per-tick evaluation cost ceiling for the convergence cursor (§D3): how many
/// scopes the cursor may *evaluate* per cycle (coverage lookup, routing resolve,
/// fingerprint compute). Sized above the scheduler's realistic per-tick
/// admission capacity so plan-112 backpressure — never this count — is what
/// paces actual requests.
pub(crate) const ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY: &str =
    "acquisition.long_tail_backfill_max_scopes_per_cycle";

/// Optional slow re-converge backstop: coverage older than this many days is
/// treated as stale and re-converged (insurance against a lossy RSS feed).
/// `0` remains an explicit opt-out. Missing scopes otherwise revalidate on a
/// slow cadence so a previously complete empty result cannot live forever.
pub(crate) const ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY: &str =
    "acquisition.long_tail_reconverge_days";

pub(crate) const DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE: i64 = 500;
pub(crate) const DEFAULT_LONG_TAIL_RECONVERGE_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub(crate) struct ConvergenceSettings {
    /// `None` when the backstop is off.
    pub long_tail_reconverge: Option<chrono::Duration>,
}

impl AppUseCase {
    pub(crate) async fn convergence_settings(&self) -> AppResult<ConvergenceSettings> {
        let reconverge_days = self
            .read_setting_i64_value(ACQUISITION_LONG_TAIL_RECONVERGE_DAYS_KEY, None)
            .await?
            .unwrap_or(DEFAULT_LONG_TAIL_RECONVERGE_DAYS);
        let long_tail_reconverge = (reconverge_days > 0)
            .then(|| chrono::Duration::days(reconverge_days))
            .filter(|d| *d > chrono::Duration::zero());

        Ok(ConvergenceSettings {
            long_tail_reconverge,
        })
    }
}

/// System-settings key persisting the cold-lane rotation position across
/// cycles and restarts (§D3: the cursor is keyed on the last-considered
/// scope_key, not a numeric offset, so it survives target-set changes).
pub(crate) const BACKGROUND_ACQUISITION_RESUME_AFTER_KEY: &str =
    "acquisition.convergence_resume_after";

/// Marker set once the run-once cutover seed has completed.
pub(crate) const ACQUISITION_CONVERGENCE_SEEDED_AT_KEY: &str = "acquisition.convergence_seeded_at";

/// Scopes the legacy scheduler searched within this window start converged at
/// cutover instead of being re-swept on first boot.
const CUTOVER_SEED_RECENT_SEARCH_DAYS: i64 = 14;

impl AppUseCase {
    /// Run-once cutover reconciliation: scopes with a recent
    /// legacy search start *converged* — coverage recorded for every routed
    /// indexer under the current fingerprint — so the first convergence sweep
    /// only covers what the old scheduler had genuinely not searched.
    /// Best-effort and idempotent: imperfect seeding only causes a safe
    /// re-converge, so any failure is logged and skipped.
    pub(crate) async fn seed_convergence_from_legacy_history(&self) {
        let already_seeded = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                ACQUISITION_CONVERGENCE_SEEDED_AT_KEY,
                None,
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<String>(&value).ok())
            .is_some_and(|value| !value.trim().is_empty());
        if already_seeded {
            return;
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::days(CUTOVER_SEED_RECENT_SEARCH_DAYS);
        let items = match self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(crate::contracts::AcquisitionScopeStatesQuery {
                limit: i64::MAX,
                ..crate::contracts::AcquisitionScopeStatesQuery::default()
            })
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(error = %error, "convergence seed: failed to list legacy state rows");
                return;
            }
        };

        let mut seeded_scopes = 0usize;
        for item in items {
            let recently_searched = item
                .last_search_at
                .as_deref()
                .and_then(crate::quality_profile::parse_published_at)
                .is_some_and(|searched_at| searched_at >= cutoff);
            if !recently_searched || item.status != AcquisitionScopeStatus::Wanted {
                continue;
            }
            let Ok(Some(title)) = self.services.catalog.titles.get_by_id(&item.title_id).await
            else {
                continue;
            };
            let episode = match item.episode_id.as_deref() {
                Some(episode_id) => self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let subject = self
                .resolve_release_search_subject_for_wanted_item(
                    &title,
                    &title,
                    &item,
                    episode.as_ref(),
                )
                .await;
            let Some(convergence) = self.resolve_scope_convergence(&title, &subject).await else {
                continue;
            };
            self.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
                .await;
            seeded_scopes += 1;
        }

        let now = chrono::Utc::now().to_rfc3339();
        if let Ok(value_json) = serde_json::to_string(&now)
            && let Err(error) = self
                .services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    ACQUISITION_CONVERGENCE_SEEDED_AT_KEY,
                    None,
                    value_json,
                    "system",
                    None,
                )
                .await
        {
            tracing::warn!(error = %error, "convergence seed: failed to persist completion marker");
        }
        tracing::info!(
            seeded_scopes,
            "convergence cutover seed complete: recently-searched scopes start converged"
        );
    }
}

/// Account quota at or below this remaining fraction counts as exhausted for
/// the cursor's pre-skip (mirrors the scheduler's own quota gate — the cursor
/// only avoids spending evaluation budget on requests the scheduler would
/// refuse anyway).
const QUOTA_EXHAUSTED_REMAINING_FRACTION: f64 = 0.01;

/// Per-cycle view of which scheduler hosts/accounts can take background work
/// right now, derived from the plan-112 snapshot. A stale quota observation
/// counts as available so a wedged probe can never starve the lane. This is a
/// *pre-skip* only — admission stays entirely the scheduler's inside the
/// search; the cursor just declines to spend evaluation budget on scopes whose
/// every routed indexer is currently unreachable.
pub(crate) struct SchedulerAvailability {
    cooled_hosts: std::collections::HashSet<String>,
    exhausted_accounts: std::collections::HashSet<String>,
}

impl SchedulerAvailability {
    /// An indexer can be searched when its host is not cooling down and its
    /// account quota (keyed by indexer config id) is not exhausted.
    pub fn indexer_available(&self, host_key: Option<&str>, indexer_id: &str) -> bool {
        if let Some(host) = host_key
            && self.cooled_hosts.contains(host)
        {
            return false;
        }
        !self
            .exhausted_accounts
            .contains(&indexer_id.trim().to_ascii_lowercase())
    }
}

/// The scheduler host key for an indexer base URL — the URL's host, matching
/// the keys the plan-112 snapshot reports.
pub(crate) fn indexer_scheduler_host_key(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    url::Url::parse(trimmed)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()))
        .or_else(|| Some(trimmed.to_ascii_lowercase()))
}

impl AppUseCase {
    pub(crate) async fn scheduler_availability(&self) -> SchedulerAvailability {
        let now = chrono::Utc::now();
        let mut cooled_hosts = std::collections::HashSet::new();
        let mut exhausted_accounts = std::collections::HashSet::new();
        match self
            .upstream_scheduler_snapshot(
                crate::upstream_scheduler::SchedulerSnapshotFilter::default(),
            )
            .await
        {
            Ok(snapshot) => {
                for entry in snapshot.entries {
                    if entry.cooldown_until.is_some_and(|until| until > now) {
                        cooled_hosts.insert(entry.host_key.as_str().to_string());
                    }
                    if !entry.quota_stale
                        && entry
                            .api_remaining_fraction
                            .is_some_and(|fraction| fraction <= QUOTA_EXHAUSTED_REMAINING_FRACTION)
                        && let Some(account) = entry.account_quota_key.as_ref()
                    {
                        exhausted_accounts.insert(account.as_str().to_string());
                    }
                }
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "scheduler snapshot unavailable; cursor pre-skip disabled this cycle"
                );
            }
        }
        SchedulerAvailability {
            cooled_hosts,
            exhausted_accounts,
        }
    }

    /// Indexer config id → scheduler host key, for the cursor's pre-skip.
    pub(crate) async fn indexer_scheduler_host_keys(
        &self,
    ) -> std::collections::HashMap<String, String> {
        self.services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|config| {
                indexer_scheduler_host_key(&config.base_url).map(|host| (config.id, host))
            })
            .collect()
    }

    /// The persisted cold-lane rotation position (§D3), if any.
    pub(crate) async fn background_acquisition_resume_position(&self) -> Option<String> {
        let value_json = self
            .services
            .config
            .settings
            .get_setting_json_explicit(
                SETTINGS_SCOPE_SYSTEM,
                BACKGROUND_ACQUISITION_RESUME_AFTER_KEY,
                None,
            )
            .await
            .ok()
            .flatten()?;
        serde_json::from_str::<String>(&value_json)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// Persist the cold-lane rotation position for the next cycle.
    pub(crate) async fn store_background_acquisition_resume_position(
        &self,
        position: Option<&str>,
    ) {
        let value = position.unwrap_or_default();
        let Ok(value_json) = serde_json::to_string(value) else {
            return;
        };
        if let Err(error) = self
            .services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                BACKGROUND_ACQUISITION_RESUME_AFTER_KEY,
                None,
                value_json,
                "system",
                None,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to persist convergence cursor position");
        }
    }
}

/// Canonical fingerprint for a scope's effective search criteria. Stable across
/// audio-language ordering. `profile_version` must change whenever the profile's
/// acceptance criteria (cutoff, allowed qualities, scoring) change, and
/// `match_identity` (the scope's SMG match — its resolved external ids) must change
/// on a rematch; either re-opens convergence for still-unsatisfied scopes
///. The profile inputs are the *effective* profile (resolved with
/// library/tag/category scoping), so overrides fold in.
pub(crate) fn compute_search_fingerprint(
    profile_id: &str,
    profile_version: &str,
    required_audio_languages: &[String],
    match_identity: &str,
) -> String {
    let mut langs: Vec<String> = required_audio_languages
        .iter()
        .map(|lang| lang.trim().to_ascii_lowercase())
        .filter(|lang| !lang.is_empty())
        .collect();
    langs.sort();
    langs.dedup();
    let canonical = format!(
        "v4;profile={};version={};audio={};match={}",
        profile_id.trim(),
        profile_version.trim(),
        langs.join(","),
        match_identity.trim(),
    );
    crate::helpers::blake3_identity_hex(crate::helpers::HashDomain::ConvergenceScope, canonical)
}

fn indexer_coverage_fingerprint(
    scope_fingerprint: &str,
    config: &IndexerConfig,
    search_semantics_version: Option<u32>,
) -> String {
    let mut identity = crate::indexer_search_identity(config, search_semantics_version);
    if let Some(identity) = identity.as_object_mut() {
        identity.insert(
            "scope".to_string(),
            serde_json::Value::String(scope_fingerprint.to_string()),
        );
    }
    crate::helpers::blake3_identity_hex(
        crate::helpers::HashDomain::IndexerCoverage,
        canonical_json_string(&identity),
    )
}

/// Canonical identity of a scope's SMG match — its resolved external ids. A rematch
/// re-maps the title to a different canonical subject, changing these ids, so folding
/// them into the fingerprint re-opens convergence. Plain metadata edits
/// that leave the match unchanged do not.
fn scope_match_identity(subject: &ResolvedReleaseSearchSubject) -> String {
    fn part(label: &str, value: &Option<String>) -> String {
        format!(
            "{label}={}",
            value.as_deref().map(str::trim).unwrap_or_default()
        )
    }
    [
        part("imdb", &subject.imdb_id),
        part("tmdb", &subject.tmdb_id),
        part("tvdb", &subject.tvdb_id),
        part("anidb", &subject.anidb_id),
        part("mal", &subject.mal_id),
    ]
    .join(";")
}

impl AppUseCase {
    /// Indexers among `routed_indexer_ids` that still need a convergence search
    /// for `scope_key`/`facet` under `fingerprint` — routed minus current-
    /// fingerprint coverage. The optional slow re-converge backstop treats
    /// coverage older than the configured window as uncovered.
    pub(crate) async fn uncovered_indexers_for_scope(
        &self,
        scope_key: &str,
        _facet: &str,
        fingerprint: &str,
        routed_indexer_ids: &[String],
    ) -> AppResult<Vec<String>> {
        if routed_indexer_ids.is_empty() {
            return Ok(Vec::new());
        }
        let stale_before = self
            .convergence_settings()
            .await?
            .long_tail_reconverge
            .map(|window| chrono::Utc::now() - window);
        let coverage_rows = self
            .services
            .integrations
            .scope_indexer_coverage
            .list_coverage_for_scope_keys(&[scope_key.to_string()])
            .await?;
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .map(|config| (config.id.clone(), config))
            .collect::<std::collections::HashMap<_, _>>();
        let provider = self.services.integrations.plugin_provider.available();
        Ok(routed_indexer_ids
            .iter()
            .filter(|id| {
                let Some(config) = configs.get(id.as_str()) else {
                    return true;
                };
                let semantics = provider.as_ref().and_then(|provider| {
                    provider.search_semantics_version_for_provider(&config.provider_type)
                });
                let expected = indexer_coverage_fingerprint(fingerprint, config, semantics);
                !coverage_rows.iter().any(|row| {
                    row.indexer_id == **id
                        && row.fingerprint == expected
                        && stale_before.is_none_or(|cutoff| {
                            chrono::DateTime::parse_from_rfc3339(&row.searched_at)
                                .map(|searched_at| {
                                    searched_at.with_timezone(&chrono::Utc) >= cutoff
                                })
                                .unwrap_or(false)
                        })
                })
            })
            .cloned()
            .collect())
    }
}

/// A scope's convergence coordinates: its stable coverage key, media facet,
/// current search-criteria fingerprint, and the indexer ids routed to it. The
/// coverage write-hook (after a search) and the convergence read-gate (the
/// RSS-only decision) both derive this from the same resolution, so writer and
/// reader agree on the fingerprint by construction.
#[derive(Debug, Clone)]
pub(crate) struct ScopeConvergence {
    pub scope_key: String,
    pub facet: String,
    pub fingerprint: String,
    pub routed_indexer_ids: Vec<String>,
}

/// Coverage invalidation policy used when an acquisition scope is re-opened.
///
/// Operator-triggered searches and mismatch recovery use [`Self::All`] to
/// override convergence. Failed grabs and rejected imports use
/// [`Self::Indexer`] to retry only the provider that failed. [`Self::Keep`]
/// resets only the state row, retaining deliberately-valid coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoverageReopen {
    /// Reset the scope's state row only. Every failure path uses this: the
    /// scope's saved search results are tried before any indexer is queried,
    /// and a scope whose results are exhausted stays converged.
    Keep,
    /// Forget every indexer's coverage for the scope (operator triggers:
    /// search-again, queue replacement, search-monitored).
    All,
}

/// Stable coverage key for a submission scope, or `None` for a true `Orphan` (no
/// derivable target identity), which is never a convergence unit. Episode sets /
/// season packs converge as first-class units keyed on their canonical member set
///; a member-set change yields a new key (re-converges).
pub(crate) fn convergence_scope_key(scope: &SubmissionScope, title_id: &str) -> Option<String> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(format!("episode:{episode_id}")),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => Some(format!("series_movie:{series_movie_link_id}")),
        SubmissionScope::Collection { collection_id } => {
            Some(format!("collection:{collection_id}"))
        }
        SubmissionScope::Title => {
            let title_id = title_id.trim();
            (!title_id.is_empty()).then(|| format!("title:{title_id}"))
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids: Vec<&str> = episode_ids
                .iter()
                .map(|id| id.trim())
                .filter(|id| !id.is_empty())
                .collect();
            ids.sort_unstable();
            ids.dedup();
            (!ids.is_empty()).then(|| {
                format!(
                    "episode_set:b3:{}",
                    crate::helpers::blake3_identity_hex(
                        crate::helpers::HashDomain::EpisodeSetScope,
                        ids.join(","),
                    )
                )
            })
        }
        SubmissionScope::Orphan => None,
    }
}

/// Stable title-lane key for series-pack discovery. It deliberately has no
/// title id: eligibility changes when its canonical collection membership does.
pub(crate) fn series_pack_set_scope_key(collection_ids: &[String]) -> Option<String> {
    let mut ids = collection_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    (!ids.is_empty()).then(|| {
        format!(
            "series_pack_set:b3:{}",
            crate::helpers::blake3_identity_hex(
                crate::helpers::HashDomain::SeriesPackSetScope,
                ids.join(","),
            )
        )
    })
}

/// Stable per-collection receipt for a qualifying series pack. This is kept
/// distinct from ordinary `collection:` coverage so it cannot suppress the
/// established season search lane.
pub(crate) fn series_pack_collection_scope_key(collection_id: &str) -> Option<String> {
    let collection_id = collection_id.trim();
    (!collection_id.is_empty()).then(|| format!("series_pack_collection:{collection_id}"))
}

/// Stable convergence key derived from a persisted acquisition scope state.
/// Returns `None` only when the state has no derivable target identity.
pub(crate) fn convergence_scope_key_for_state(item: &AcquisitionScopeState) -> Option<String> {
    convergence_scope_key(
        &SubmissionScope::from_persisted(
            &item.title_id,
            item.episode_id.clone(),
            item.collection_id.clone(),
            item.series_movie_link_id.clone(),
            None,
        ),
        &item.title_id,
    )
}

/// Deterministic version string for a quality profile's acceptance criteria. Any
/// edit that changes acceptance (cutoff, tiers, codecs, required audio) changes
/// this, so the fingerprint changes and still-unsatisfied scopes re-open for
/// convergence. Canonical (recursively sorted-key) JSON keeps the hash stable
/// regardless of map iteration order.
///
/// Hashes [`AcceptanceCriteria`], not the whole profile: a ranking-only edit
/// (persona, score overrides, preference flags) re-orders the results a scope
/// already has and needs no new indexer data, so it must not invalidate
/// convergence coverage for every scope inheriting the profile and trigger a
/// library-wide re-search. That projection is where a new criteria field gets
/// classified acceptance-vs-ranking.
pub(crate) fn profile_criteria_version(criteria: &QualityProfileCriteria) -> String {
    let acceptance = crate::quality_profile::AcceptanceCriteria::from(criteria);
    let value = serde_json::to_value(&acceptance).unwrap_or(serde_json::Value::Null);
    crate::helpers::blake3_identity_hex(
        crate::helpers::HashDomain::QualityProfileCriteria,
        canonical_json_string(&value),
    )
}

fn canonical_json_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json_string(&map[key]));
            }
            out.push('}');
            out
        }
        serde_json::Value::Array(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json_string(item));
            }
            out.push(']');
            out
        }
        other => other.to_string(),
    }
}

impl AppUseCase {
    /// Convergence coordinates for an active background search of `subject` under
    /// `title`, or `None` when the scope is not a single convergence unit or no
    /// indexers are routed to it.
    pub(crate) async fn resolve_scope_convergence(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
    ) -> Option<ScopeConvergence> {
        let scope_key = convergence_scope_key(&subject.submission_scope, &subject.title_id)?;
        self.resolve_scope_convergence_for_key(title, subject, scope_key)
            .await
    }

    /// Convergence coordinates for the series-pack title lane. The set key is
    /// based on eligible collection membership, never `title:<id>`.
    pub(crate) async fn resolve_series_pack_convergence(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        collection_ids: &[String],
    ) -> Option<ScopeConvergence> {
        let scope_key = series_pack_set_scope_key(collection_ids)?;
        self.resolve_scope_convergence_for_key(title, subject, scope_key)
            .await
    }

    async fn resolve_scope_convergence_for_key(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        scope_key: String,
    ) -> Option<ScopeConvergence> {
        let facet = subject.owner_facet.as_str().to_string();

        let context = match self
            .resolve_upgrade_context_for_title_with_category_and_quality(
                title,
                Some(subject.category.as_str()),
                None,
            )
            .await
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    title_id = subject.title_id.as_str(),
                    error = %error,
                    "convergence: failed to resolve quality profile; leaving scope unresolved"
                );
                return None;
            }
        };
        let required_audio_languages = match self
            .resolve_required_audio_languages_for_title(title)
            .await
        {
            Ok(languages) => languages,
            Err(error) => {
                tracing::warn!(
                    title_id = subject.title_id.as_str(),
                    error = %error,
                    "convergence: failed to resolve required audio languages; leaving scope unresolved"
                );
                return None;
            }
        };
        let fingerprint = compute_search_fingerprint(
            &context.profile.id,
            &profile_criteria_version(&context.profile.criteria),
            &required_audio_languages,
            &scope_match_identity(subject),
        );

        let routed_indexer_ids = self.routed_indexer_ids_for_search(title, subject).await;
        if routed_indexer_ids.is_empty() {
            return None;
        }

        Some(ScopeConvergence {
            scope_key,
            facet,
            fingerprint,
            routed_indexer_ids,
        })
    }

    /// The indexer ids an active search of `subject` targets — the enabled routing
    /// entries for the scope, or every configured indexer when no routing is set.
    /// Mirrors the indexer selection in `search_and_score_releases` so coverage is
    /// recorded for exactly the indexers a search would query.
    async fn routed_indexer_ids_for_search(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
    ) -> Vec<String> {
        let lookup = QualityProfileLookup {
            title_tags: &subject.title_tags,
            library_id: Some(title.library_id.as_str()),
            imdb_id: subject.imdb_id.as_deref(),
            tvdb_id: subject.tvdb_id.as_deref(),
            category_hint: Some(subject.owner_facet.as_str()),
        };
        let scope_id = self.quality_profile_scope_id(lookup);
        match self
            .resolve_indexer_routing(Some(title.library_id.as_str()), scope_id.as_deref())
            .await
        {
            Some(plan) => plan
                .entries
                .into_iter()
                .filter(|(_, entry)| entry.enabled)
                .map(|(indexer_id, _)| indexer_id)
                .collect(),
            None => self
                .services
                .integrations
                .indexer_configs
                .list(None)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|config| config.is_enabled)
                .map(|config| config.id)
                .collect(),
        }
    }

    /// Record convergence coverage for a search of `subject`. A search is a
    /// search (§D5): background, interactive, and season-pack searches all
    /// record coverage, since any of them proves what the indexer's catalog
    /// holds for this scope. Records **only the indexers that actually fired a
    /// query and returned a response** (`fired_indexer_ids`, from the augmented
    /// search return) intersected with the scope's routed set — never a routed
    /// indexer the scheduler deferred/skipped or whose query errored (§D2). An
    /// empty response still counts: "no results" is coverage. A no-op when the
    /// scope is not a convergence unit or when nothing fired. Best-effort: a
    /// failed write is logged, never propagated, so it can never break the
    /// acquisition path.
    pub(crate) async fn record_search_coverage(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        fired_indexer_ids: &[String],
    ) {
        let Some(convergence) = self.resolve_scope_convergence(title, subject).await else {
            return;
        };
        self.record_convergence_coverage(&convergence, fired_indexer_ids)
            .await;
    }

    /// Write receipts for an already-resolved convergence unit. This lets the
    /// series-pack title lane use its membership and collection keys without
    /// changing the generic search coverage behavior.
    pub(crate) async fn record_convergence_coverage(
        &self,
        convergence: &ScopeConvergence,
        fired_indexer_ids: &[String],
    ) {
        // Only routed indexers that actually fired are recorded as covered; a
        // deferred/skipped/errored routed indexer stays uncovered so the cursor
        // retries it.
        let fired: std::collections::HashSet<&str> =
            fired_indexer_ids.iter().map(String::as_str).collect();
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|config| (config.id.clone(), config))
            .collect::<std::collections::HashMap<_, _>>();
        let provider = self.services.integrations.plugin_provider.available();
        for indexer_id in &convergence.routed_indexer_ids {
            if !fired.contains(indexer_id.as_str()) {
                continue;
            }
            let Some(config) = configs.get(indexer_id) else {
                continue;
            };
            let semantics = provider.as_ref().and_then(|provider| {
                provider.search_semantics_version_for_provider(&config.provider_type)
            });
            let fingerprint =
                indexer_coverage_fingerprint(&convergence.fingerprint, config, semantics);
            if let Err(error) = self
                .services
                .integrations
                .scope_indexer_coverage
                .record_coverage(
                    &convergence.scope_key,
                    &convergence.facet,
                    indexer_id,
                    &fingerprint,
                )
                .await
            {
                tracing::warn!(
                    scope_key = convergence.scope_key.as_str(),
                    facet = convergence.facet.as_str(),
                    indexer_id = indexer_id.as_str(),
                    error = %error,
                    "failed to record convergence coverage"
                );
            }
        }
    }

    /// Re-open an acquisition state row and apply its coverage invalidation
    /// policy before waking the convergence cursor. Operator triggers use
    /// [`CoverageReopen::All`]; failures and fingerprint rematches use
    /// [`CoverageReopen::Keep`].
    pub(crate) async fn reopen_wanted_scope_for_acquisition(
        &self,
        item: &AcquisitionScopeState,
        coverage: CoverageReopen,
    ) {
        if let Err(error) = self
            .services
            .workflow
            .acquisition_scope_states
            .transition_acquisition_scope_to_reopened(&item.id)
            .await
        {
            tracing::warn!(
                wanted_item_id = item.id.as_str(),
                error = %error,
                "failed to reset wanted state row while re-opening scope"
            );
        }

        if let Some(scope_key) = convergence_scope_key_for_state(item) {
            match coverage {
                CoverageReopen::Keep => {}
                CoverageReopen::All => self.prune_scope_key_coverage(&scope_key, None).await,
            }
        }
        self.runtime.acquisition.acquisition_wake.notify_one();
    }

    /// Best-effort coverage invalidation for a caller that already has a
    /// convergence scope key. `None` removes the whole scope; `Some` removes
    /// only that indexer's coverage row.
    pub(crate) async fn prune_scope_key_coverage(&self, scope_key: &str, indexer_id: Option<&str>) {
        let result = match indexer_id {
            Some(indexer_id) => {
                self.services
                    .integrations
                    .scope_indexer_coverage
                    .prune_scope_indexer(scope_key, indexer_id)
                    .await
            }
            None => {
                self.services
                    .integrations
                    .scope_indexer_coverage
                    .prune_scope(scope_key)
                    .await
            }
        };
        if let Err(error) = result {
            tracing::warn!(
                scope_key,
                indexer_id = indexer_id.unwrap_or(""),
                error = %error,
                "failed to prune convergence coverage while re-opening acquisition scope"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_json_string, compute_search_fingerprint, convergence_scope_key,
        profile_criteria_version, series_pack_collection_scope_key, series_pack_set_scope_key,
    };
    use crate::contracts::SubmissionScope;
    use crate::quality_profile::{AcceptanceCriteria, QualityProfileCriteria};

    #[test]
    fn fingerprint_is_stable_and_order_independent_for_audio() {
        let a = compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()], "m1");
        let b = compute_search_fingerprint("p1", "v1", &["JA".into(), " en ".into()], "m1");
        assert_eq!(a, b, "audio-language order/case/whitespace must not matter");
    }

    #[test]
    fn fingerprint_changes_on_profile_version() {
        let a = compute_search_fingerprint("p1", "v1", &["en".into()], "m1");
        let b = compute_search_fingerprint("p1", "v2", &["en".into()], "m1");
        assert_ne!(
            a, b,
            "a profile edit (version bump) must change the fingerprint"
        );
    }

    #[test]
    fn fingerprint_changes_on_profile_id_and_audio() {
        let base = compute_search_fingerprint("p1", "v1", &["en".into()], "m1");
        assert_ne!(
            base,
            compute_search_fingerprint("p2", "v1", &["en".into()], "m1")
        );
        assert_ne!(
            base,
            compute_search_fingerprint("p1", "v1", &["en".into(), "ja".into()], "m1")
        );
        assert_ne!(base, compute_search_fingerprint("p1", "v1", &[], "m1"));
    }

    #[test]
    fn fingerprint_changes_when_resolved_original_language_changes() {
        let japanese = compute_search_fingerprint("p1", "v1", &["jpn".into()], "m1");
        let korean = compute_search_fingerprint("p1", "v1", &["kor".into()], "m1");

        assert_ne!(japanese, korean);
    }

    #[test]
    fn fingerprint_changes_on_rematch() {
        // A rematch changes the scope's external-id identity → new fingerprint →
        // convergence re-opens.
        let a = compute_search_fingerprint("p1", "v1", &["en".into()], "imdb=tt1");
        let b = compute_search_fingerprint("p1", "v1", &["en".into()], "imdb=tt2");
        assert_ne!(
            a, b,
            "a rematch (changed SMG match id) must change the fingerprint"
        );
    }

    fn test_criteria() -> QualityProfileCriteria {
        crate::quality_profile::builtin_4k_profile().criteria
    }

    #[test]
    fn convergence_scope_key_maps_each_scope_kind() {
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::Episode {
                    episode_id: "e1".into()
                },
                "t1"
            ),
            Some("episode:e1".to_string())
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::SeriesMovie {
                    series_movie_link_id: "l1".into()
                },
                "t1"
            ),
            Some("series_movie:l1".to_string())
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::Collection {
                    collection_id: "c1".into()
                },
                "t1"
            ),
            Some("collection:c1".to_string())
        );
        assert_eq!(
            convergence_scope_key(&SubmissionScope::Title, "t1"),
            Some("title:t1".to_string())
        );
        // A true orphan (and an empty title) is never a convergence unit.
        assert_eq!(convergence_scope_key(&SubmissionScope::Title, "   "), None);
        assert_eq!(convergence_scope_key(&SubmissionScope::Orphan, "t1"), None);

        // Episode sets / season packs DO converge, keyed on their canonical member
        // set — order/whitespace/duplicate independent, empty-set excluded.
        let pack_ab = convergence_scope_key(
            &SubmissionScope::EpisodeSet {
                episode_ids: vec!["e1".into(), "e2".into()],
            },
            "t1",
        );
        assert!(
            pack_ab
                .as_deref()
                .is_some_and(|key| key.starts_with("episode_set:"))
        );
        assert_eq!(
            pack_ab,
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec![" e2 ".into(), "e1".into(), "e1".into()],
                },
                "t1",
            ),
            "canonical member set is order/whitespace/duplicate independent"
        );
        assert_ne!(
            pack_ab,
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec!["e1".into(), "e3".into()],
                },
                "t1",
            ),
            "a different member set is a different pack scope"
        );
        assert_eq!(
            convergence_scope_key(
                &SubmissionScope::EpisodeSet {
                    episode_ids: vec![]
                },
                "t1"
            ),
            None
        );

        let set_ab = series_pack_set_scope_key(&["c1".into(), "c2".into()]);
        assert!(
            set_ab
                .as_deref()
                .is_some_and(|key| key.starts_with("series_pack_set:"))
        );
        assert_eq!(
            set_ab,
            series_pack_set_scope_key(&[" c2 ".into(), "c1".into(), "c1".into()])
        );
        assert_ne!(
            set_ab,
            series_pack_set_scope_key(&["c1".into(), "c3".into()])
        );
        assert_eq!(series_pack_set_scope_key(&[]), None);
        assert_eq!(
            series_pack_collection_scope_key(" c1 "),
            Some("series_pack_collection:c1".to_string())
        );
    }

    #[test]
    fn canonical_json_string_is_key_order_independent() {
        let a = serde_json::json!({ "b": 1, "a": [ { "y": 1, "x": 2 } ] });
        let b = serde_json::json!({ "a": [ { "x": 2, "y": 1 } ], "b": 1 });
        assert_eq!(canonical_json_string(&a), canonical_json_string(&b));
        assert!(canonical_json_string(&a).starts_with("{\"a\":"));
    }

    #[test]
    fn profile_criteria_version_is_stable_and_edit_sensitive() {
        let base = test_criteria();
        assert_eq!(
            profile_criteria_version(&base),
            profile_criteria_version(&base.clone()),
            "the same criteria must hash to the same version"
        );

        let mut edited = base.clone();
        edited.allow_upgrades = !base.allow_upgrades;
        assert_ne!(
            profile_criteria_version(&base),
            profile_criteria_version(&edited),
            "an acceptance-criteria edit must change the version"
        );

        let mut audio_edited = base.clone();
        audio_edited.required_audio_languages.push("ja".to_string());
        assert_ne!(
            profile_criteria_version(&base),
            profile_criteria_version(&audio_edited),
            "a required-audio change must change the version"
        );
    }

    /// A ranking-only profile edit must leave the criteria version alone.
    ///
    /// Re-ordering candidates is decided against results the scope already
    /// holds; it needs no new indexer data. Hashing the persona, the score
    /// overrides or the preference flags would re-open convergence for every
    /// scope inheriting the profile and re-search the whole library to arrive at
    /// the same corpus in a different order.
    #[test]
    fn a_ranking_only_profile_edit_does_not_move_the_criteria_version() {
        let base = test_criteria();
        let mut ranked_differently = base.clone();
        ranked_differently.scoring_persona = crate::scoring_weights::ScoringPersona::Efficient;
        ranked_differently.prefer_remux = !base.prefer_remux;
        ranked_differently.atmos_preferred = !base.atmos_preferred;
        ranked_differently.prefer_dual_audio = !base.prefer_dual_audio;
        ranked_differently.scoring_overrides.prefer_compact_encodes = Some(true);
        ranked_differently.facet_persona_overrides.insert(
            "movie".to_string(),
            crate::scoring_weights::ScoringPersona::Audiophile,
        );

        assert_ne!(
            base.scoring_persona, ranked_differently.scoring_persona,
            "the edit must actually differ, or this test proves nothing"
        );
        assert_eq!(
            profile_criteria_version(&base),
            profile_criteria_version(&ranked_differently),
            "a ranking-only edit must not invalidate convergence coverage"
        );
    }

    #[test]
    fn every_acceptance_edit_moves_the_criteria_version() {
        let base = test_criteria();
        let baseline = profile_criteria_version(&base);

        let mut tiers = base.clone();
        tiers.quality_tiers.push("480p".to_string());

        let mut upgrades = base.clone();
        upgrades.allow_upgrades = !base.allow_upgrades;

        let mut audio = base.clone();
        audio.required_audio_languages.push("ja".to_string());

        let mut cutoff = base.clone();
        assert_eq!(cutoff.cutoff_score, None);
        cutoff.cutoff_score = Some(400);

        let mut unknown = base.clone();
        unknown.allow_unknown_quality = !base.allow_unknown_quality;

        for (label, edited) in [
            ("quality_tiers", tiers),
            ("allow_upgrades", upgrades),
            ("required_audio_languages", audio),
            ("cutoff_score", cutoff),
            ("allow_unknown_quality", unknown),
        ] {
            assert_ne!(
                baseline,
                profile_criteria_version(&edited),
                "an edit to {label} changes which releases are acceptable and must re-open convergence"
            );
        }
    }

    /// **D19.** Adding a field to `QualityProfileCriteria` must not move the
    /// fingerprint of a profile that does not set it.
    ///
    /// This hash feeds `compute_search_fingerprint`, which decides whether a
    /// scope's convergence coverage is still valid. A new key in the
    /// serialization invalidates **every scope in the library** on upgrade and
    /// triggers a full re-search — a library-wide indexer sweep as the side
    /// effect of a settings field nobody set. `cutoff_score` therefore carries
    /// `skip_serializing_if = "Option::is_none"`, and this pins it two ways: the
    /// builtin 4K profile's fingerprint is hard-coded, and the key must be
    /// absent from the serialization entirely — which is what makes the
    /// hard-coded value the same one the profile hashed to before the field
    /// existed. Any future criteria field has to clear both.
    ///
    /// The constant has been re-pinned twice, both times for a deliberate change
    /// to what is hashed: the SHA-256 → BLAKE3 switch (plan 149), and the
    /// narrowing of the hash input from the whole `QualityProfileCriteria` to the
    /// acceptance-only [`AcceptanceCriteria`] projection. Those are the only
    /// reasons it may ever change: a moved value with the hash input unchanged is
    /// the library-wide re-sweep this test exists to catch.
    #[test]
    fn an_unset_new_criteria_field_does_not_move_the_profile_fingerprint() {
        let base = test_criteria();
        assert_eq!(base.cutoff_score, None);
        assert_eq!(
            profile_criteria_version(&base),
            "5d83bd034035f542fae433da6567546566f520a818aa0f425f309a4e10f1e99f",
            "the fingerprint of a profile that sets no new field must not move"
        );

        // The projection, not the source struct, is what the hash reads.
        let serialized = serde_json::to_value(AcceptanceCriteria::from(&base))
            .expect("criteria serialize for the fingerprint");
        assert!(
            serialized.get("cutoff_score").is_none(),
            "an unset `cutoff_score` must not appear in the fingerprint input at all"
        );
        for ranking_key in [
            "scoring_persona",
            "scoring_overrides",
            "facet_persona_overrides",
            "atmos_preferred",
            "prefer_remux",
            "prefer_dual_audio",
        ] {
            assert!(
                serialized.get(ranking_key).is_none(),
                "{ranking_key} ranks results the scope already has; it must not reach the fingerprint"
            );
        }

        // …and a profile that *does* set it is genuinely a different profile.
        let mut with_cutoff = base.clone();
        with_cutoff.cutoff_score = Some(400);
        assert_ne!(
            profile_criteria_version(&base),
            profile_criteria_version(&with_cutoff)
        );
    }
}
