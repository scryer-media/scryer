//! In-memory interactive release-search jobs (hotfix 0.17.1).
//!
//! Deliberate deviation from the acquisition-job pattern: no `JobRunRecord`/`JobRunTracker`/`JobKey`.
//! Those persist job history forever, enforce single-flight per key, and gate
//! poll/cancel on `ManageSystemSettings` — all wrong for ephemeral, concurrent,
//! per-user searches carrying heavy result payloads. Jobs here live in an
//! owner-scoped in-memory registry with TTL eviction; results stream into the
//! snapshot as each indexer completes so one slow or broken indexer no longer
//! blanks or delays the whole interactive search.

use super::*;

use super::discovery::{
    QualityProfileLookup, compare_release_search_results, dedupe_cross_indexer_release_results,
};
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use scryer_logging::{ActorContext, LogContext, ResourceContext, WorkflowContext, context_span};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info};

/// Overall deadline for a job; stragglers past this are marked failed.
const INTERACTIVE_RELEASE_SEARCH_DEADLINE: std::time::Duration =
    scryer_outbound_http::INDEXER_HTTP_TIMEOUT;
/// Terminal jobs are evicted this long after completion.
const COMPLETED_JOB_TTL_MINUTES: i64 = 5;
/// Defensive cap: running jobs older than this are cancelled and evicted.
const RUNNING_JOB_TTL_MINUTES: i64 = 10;
/// Per-actor cap on concurrently running jobs.
const MAX_RUNNING_JOBS_PER_ACTOR: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveReleaseSearchState {
    Running,
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveReleaseSearchIndexerStatus {
    Pending,
    Searching,
    Completed,
    Failed,
    Skipped,
}

/// Per-indexer progress inside an interactive release-search job.
#[derive(Clone, Debug)]
pub struct InteractiveReleaseSearchIndexerView {
    pub indexer_id: String,
    pub name: String,
    pub status: InteractiveReleaseSearchIndexerStatus,
    /// The indexer's own batch size (before cross-indexer dedup).
    pub result_count: usize,
    pub failure_reason: Option<String>,
}

/// Point-in-time snapshot of an interactive release-search job. `results` is
/// the scored, cross-indexer-deduped merge of every completed indexer batch.
#[derive(Clone, Debug)]
pub struct InteractiveReleaseSearchSnapshot {
    pub id: String,
    pub state: InteractiveReleaseSearchState,
    pub results: Vec<IndexerSearchResult>,
    pub indexers: Vec<InteractiveReleaseSearchIndexerView>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Requested result limit (SearchReleasesInput.limit); applied when the
    /// GraphQL payload is built, mirroring the one-shot resolver.
    pub limit: Option<i32>,
}

/// Start request — mirrors `SearchReleasesInput` field-for-field.
#[derive(Clone, Debug)]
pub struct InteractiveReleaseSearchRequest {
    pub title_id: String,
    pub series_movie_link_id: Option<String>,
    pub season: Option<String>,
    pub episode: Option<String>,
    pub limit: Option<i32>,
}

pub(crate) struct InteractiveReleaseSearchJobEntry {
    pub(crate) snapshot: InteractiveReleaseSearchSnapshot,
    pub(crate) actor_id: String,
    pub(crate) scope_key: String,
    pub(crate) cancel: CancellationToken,
}

/// Evict stale registry entries: terminal jobs past their TTL, plus (defensive)
/// running jobs older than the running TTL, whose tokens are cancelled first.
fn evict_stale_entries(
    entries: &mut HashMap<String, InteractiveReleaseSearchJobEntry>,
    now: DateTime<Utc>,
) {
    entries.retain(|_, entry| match entry.snapshot.state {
        InteractiveReleaseSearchState::Running => {
            if now - entry.snapshot.started_at > Duration::minutes(RUNNING_JOB_TTL_MINUTES) {
                entry.cancel.cancel();
                false
            } else {
                true
            }
        }
        _ => entry.snapshot.completed_at.is_some_and(|completed| {
            now - completed <= Duration::minutes(COMPLETED_JOB_TTL_MINUTES)
        }),
    });
}

fn interactive_release_search_scope_key(request: &InteractiveReleaseSearchRequest) -> String {
    format!(
        "{}|{}|{}|{}",
        request.title_id,
        request.series_movie_link_id.as_deref().unwrap_or("-"),
        request.season.as_deref().unwrap_or("-"),
        request.episode.as_deref().unwrap_or("-"),
    )
}

/// Everything the spawned runner needs, resolved once at start.
struct InteractiveReleaseSearchJobContext {
    job_id: String,
    actor: User,
    /// For series-movie searches this is the synthesized movie search title.
    title_for_search: Title,
    subject: ResolvedReleaseSearchSubject,
    preserve_subject_scope: bool,
    /// Indexer config ids to fan out to (skipped ones excluded).
    dispatch: Vec<String>,
    indexer_priority_by_name: HashMap<String, i64>,
    preferred_source_kind: String,
    cancel: CancellationToken,
}

impl AppUseCase {
    pub async fn start_interactive_release_search(
        &self,
        actor: &User,
        request: InteractiveReleaseSearchRequest,
    ) -> AppResult<InteractiveReleaseSearchSnapshot> {
        // Same input-shape validation (and messages) as the one-shot
        // `searchReleases` resolver.
        match (
            &request.series_movie_link_id,
            &request.season,
            &request.episode,
        ) {
            (Some(_), None, None) | (None, Some(_), Some(_)) | (None, None, None) => {}
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err(AppError::Validation(
                    "episode searches require both season and episode".to_string(),
                ));
            }
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(AppError::Validation(
                    "series movie searches cannot include season or episode".to_string(),
                ));
            }
        }

        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&request.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", request.title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        // Resolve the search subject once per job, not once per indexer.
        let (title_for_search, subject, preserve_subject_scope) = match (
            &request.series_movie_link_id,
            &request.season,
            &request.episode,
        ) {
            (Some(series_movie_link_id), None, None) => {
                let link = self
                    .services
                    .catalog
                    .shows
                    .get_series_movie_link_by_id(series_movie_link_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("series movie {series_movie_link_id}"))
                    })?;
                if link.series_title_id != title.id {
                    return Err(AppError::Validation(
                        "series movie does not belong to title".into(),
                    ));
                }
                let (search_title, subject) = self
                    .resolve_release_search_subject_for_series_movie(&title, &link)
                    .await?;
                (search_title, subject, true)
            }
            (None, Some(season), Some(episode)) => {
                let subject = self
                    .resolve_release_search_subject_for_episode(&title, season, episode)
                    .await?;
                (title.clone(), subject, false)
            }
            _ => {
                let subject = self
                    .resolve_release_search_subject_for_title(&title)
                    .await?;
                (title.clone(), subject, false)
            }
        };

        // Cross-indexer dedup metadata, derived exactly like
        // `search_and_score_releases`. Best-effort: an unresolved routing plan
        // degrades to an empty priority map, never a failed job.
        let lookup = QualityProfileLookup {
            title_tags: &subject.title_tags,
            library_id: Some(title_for_search.library_id.as_str()),
            imdb_id: subject.imdb_id.as_deref(),
            tvdb_id: subject.tvdb_id.as_deref(),
            category_hint: Some(subject.owner_facet.as_str()),
        };
        let scope_id = self.quality_profile_scope_id(lookup);
        let indexer_routing = self
            .resolve_indexer_routing(
                Some(title_for_search.library_id.as_str()),
                scope_id.as_deref(),
            )
            .await;
        let indexer_priority_by_name = self
            .build_indexer_priority_by_name(indexer_routing.as_ref())
            .await;
        let preferred_source_kind = self.download_source_capabilities().await.2;

        // Config-visible eligibility only: enabled + interactive. A config
        // cooldown lists the indexer as Skipped without dispatching; a
        // routing-disabled indexer is omitted entirely. Backoff state stays
        // with the search client (a backed-off indexer's restricted call just
        // returns empty).
        let now = self.runtime.environment.now();
        let mut indexer_views = Vec::new();
        let mut dispatch = Vec::new();
        for config in self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
        {
            if !config.is_enabled || !config.enable_interactive_search {
                continue;
            }
            if crate::contracts::indexer_search_eligibility(
                indexer_routing.as_ref(),
                None,
                &config.id,
            ) != crate::contracts::IndexerSearchEligibility::Eligible
            {
                continue;
            }
            if config.disabled_until.is_some_and(|until| until > now) {
                indexer_views.push(InteractiveReleaseSearchIndexerView {
                    indexer_id: config.id,
                    name: config.name,
                    status: InteractiveReleaseSearchIndexerStatus::Skipped,
                    result_count: 0,
                    failure_reason: Some("temporarily disabled".to_string()),
                });
                continue;
            }
            dispatch.push(config.id.clone());
            indexer_views.push(InteractiveReleaseSearchIndexerView {
                indexer_id: config.id,
                name: config.name,
                status: InteractiveReleaseSearchIndexerStatus::Pending,
                result_count: 0,
                failure_reason: None,
            });
        }

        let job_id = Id::new().0;
        let scope_key = interactive_release_search_scope_key(&request);
        let cancel = CancellationToken::new();
        let snapshot = InteractiveReleaseSearchSnapshot {
            id: job_id.clone(),
            state: InteractiveReleaseSearchState::Running,
            results: Vec::new(),
            indexers: indexer_views,
            started_at: now,
            completed_at: None,
            limit: request.limit,
        };

        {
            let mut registry = self
                .runtime
                .acquisition
                .interactive_release_searches
                .lock()
                .await;
            evict_stale_entries(&mut registry, now);
            // Replace semantics: a running job for the same actor+scope is
            // cancelled before the new one is inserted.
            for entry in registry.values_mut() {
                if entry.snapshot.state == InteractiveReleaseSearchState::Running
                    && entry.actor_id == actor.id
                    && entry.scope_key == scope_key
                {
                    entry.cancel.cancel();
                    entry.snapshot.state = InteractiveReleaseSearchState::Cancelled;
                    entry.snapshot.completed_at = Some(now);
                }
            }
            let running_for_actor = registry
                .values()
                .filter(|entry| {
                    entry.actor_id == actor.id
                        && entry.snapshot.state == InteractiveReleaseSearchState::Running
                })
                .count();
            if running_for_actor >= MAX_RUNNING_JOBS_PER_ACTOR {
                return Err(AppError::Validation(
                    "too many concurrent interactive searches".to_string(),
                ));
            }
            registry.insert(
                job_id.clone(),
                InteractiveReleaseSearchJobEntry {
                    snapshot: snapshot.clone(),
                    actor_id: actor.id.clone(),
                    scope_key,
                    cancel: cancel.clone(),
                },
            );
        }

        let context = InteractiveReleaseSearchJobContext {
            job_id,
            actor: actor.clone(),
            title_for_search,
            subject,
            preserve_subject_scope,
            dispatch,
            indexer_priority_by_name,
            preferred_source_kind,
            cancel,
        };
        let log_span = context_span(
            LogContext::workflow(WorkflowContext {
                kind: "interactive_release_search".to_owned(),
                id: context.job_id.clone(),
            })
            .with_actor(ActorContext {
                kind: if context.actor.is_system_execution_actor() {
                    "system".to_owned()
                } else {
                    "user".to_owned()
                },
                id: Some(context.actor.id.clone()),
                display_name: Some(context.actor.username.clone()),
                source: None,
            })
            .with_resource(ResourceContext {
                title_id: Some(context.title_for_search.id.clone()),
                job_id: Some(context.job_id.clone()),
                ..ResourceContext::default()
            }),
        );
        log_span.in_scope(|| {
            info!(
                actor = context.actor.id.as_str(),
                title_id = context.title_for_search.id.as_str(),
                job_id = context.job_id.as_str(),
                query = context
                    .subject
                    .queries
                    .first()
                    .map(String::as_str)
                    .unwrap_or(""),
                category = context.subject.category.as_str(),
                indexers = context.dispatch.len(),
                "starting interactive release search"
            );
        });
        let app = self.clone();
        tokio::spawn(
            async move {
                app.run_interactive_release_search_job(context).await;
            }
            .instrument(log_span),
        );

        Ok(snapshot)
    }

    /// Poll a job snapshot. `None` for unknown, evicted, or another actor's
    /// job (no information leak).
    pub async fn interactive_release_search(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<InteractiveReleaseSearchSnapshot>> {
        let mut registry = self
            .runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await;
        evict_stale_entries(&mut registry, self.runtime.environment.now());
        Ok(registry
            .get(id)
            .filter(|entry| entry.actor_id == actor.id)
            .map(|entry| entry.snapshot.clone()))
    }

    /// Cancel a running job. `false` (not an error) when the job is unknown,
    /// foreign, or already finished.
    pub async fn cancel_interactive_release_search(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<bool> {
        let now = self.runtime.environment.now();
        let token = {
            let mut registry = self
                .runtime
                .acquisition
                .interactive_release_searches
                .lock()
                .await;
            evict_stale_entries(&mut registry, now);
            let Some(entry) = registry.get_mut(id) else {
                return Ok(false);
            };
            if entry.actor_id != actor.id
                || entry.snapshot.state != InteractiveReleaseSearchState::Running
            {
                return Ok(false);
            }
            entry.snapshot.state = InteractiveReleaseSearchState::Cancelled;
            entry.snapshot.completed_at = Some(now);
            entry.cancel.clone()
        };
        token.cancel();
        Ok(true)
    }

    async fn run_interactive_release_search_job(
        &self,
        context: InteractiveReleaseSearchJobContext,
    ) {
        let InteractiveReleaseSearchJobContext {
            job_id,
            actor,
            title_for_search,
            subject,
            preserve_subject_scope,
            dispatch,
            indexer_priority_by_name,
            preferred_source_kind,
            cancel,
        } = context;

        let mut set = JoinSet::new();
        for indexer_id in dispatch {
            let app = self.clone();
            let actor = actor.clone();
            let title = title_for_search.clone();
            let subject = subject.clone();
            let job_id = job_id.clone();
            let child_token = cancel.child_token();
            set.spawn(async move {
                app.set_interactive_indexer_status(
                    &job_id,
                    &indexer_id,
                    InteractiveReleaseSearchIndexerStatus::Searching,
                    None,
                )
                .await;
                let outcome = app
                    .search_and_evaluate_subject_restricted_with_outcome(
                        &title,
                        &subject,
                        &actor.id,
                        SearchMode::Interactive,
                        child_token,
                        Some(HashSet::from([indexer_id.clone()])),
                        None,
                    )
                    .await;
                let outcome = match outcome {
                    Ok(mut search_outcome) => {
                        let failure_reason = search_outcome
                            .incomplete_indexer_reasons
                            .remove(&indexer_id);
                        app.attach_candidate_tokens(
                            &actor,
                            &title,
                            &subject,
                            &mut search_outcome.results,
                            preserve_subject_scope,
                        )
                        .await;
                        Ok((search_outcome.results, failure_reason))
                    }
                    Err(error) => Err(error),
                };
                (indexer_id, outcome)
            });
        }

        let drain = async {
            while let Some(joined) = set.join_next().await {
                let (indexer_id, outcome) = match joined {
                    Ok(joined) => joined,
                    Err(error) => {
                        tracing::warn!(
                            job_id = job_id.as_str(),
                            error = %error,
                            "interactive release search indexer task panicked"
                        );
                        continue;
                    }
                };
                match outcome {
                    Ok((batch, failure_reason)) => {
                        self.merge_interactive_indexer_batch(
                            &job_id,
                            &indexer_id,
                            batch,
                            &indexer_priority_by_name,
                            &preferred_source_kind,
                        )
                        .await;
                        if let Some(reason) = failure_reason {
                            self.set_interactive_indexer_status(
                                &job_id,
                                &indexer_id,
                                InteractiveReleaseSearchIndexerStatus::Failed,
                                Some(reason),
                            )
                            .await;
                        }
                    }
                    Err(error) if error.is_canceled() => {
                        // Job is being cancelled; leave the status as-is.
                    }
                    Err(error) => {
                        self.set_interactive_indexer_status(
                            &job_id,
                            &indexer_id,
                            InteractiveReleaseSearchIndexerStatus::Failed,
                            Some(error.to_string()),
                        )
                        .await;
                    }
                }
            }
        };
        let timed_out = tokio::time::timeout(INTERACTIVE_RELEASE_SEARCH_DEADLINE, drain)
            .await
            .is_err();
        if timed_out {
            cancel.cancel();
            set.abort_all();
        }

        let now = self.runtime.environment.now();
        {
            let mut registry = self
                .runtime
                .acquisition
                .interactive_release_searches
                .lock()
                .await;
            let Some(entry) = registry.get_mut(&job_id) else {
                return;
            };
            if timed_out {
                for indexer in entry.snapshot.indexers.iter_mut() {
                    if matches!(
                        indexer.status,
                        InteractiveReleaseSearchIndexerStatus::Pending
                            | InteractiveReleaseSearchIndexerStatus::Searching
                    ) {
                        indexer.status = InteractiveReleaseSearchIndexerStatus::Failed;
                        indexer.failure_reason = Some("timed out".to_string());
                    }
                }
            }
            if entry.snapshot.state == InteractiveReleaseSearchState::Running {
                entry.snapshot.state = InteractiveReleaseSearchState::Completed;
                entry.snapshot.completed_at = Some(now);
            }
        }
    }

    async fn set_interactive_indexer_status(
        &self,
        job_id: &str,
        indexer_id: &str,
        status: InteractiveReleaseSearchIndexerStatus,
        failure_reason: Option<String>,
    ) {
        let mut registry = self
            .runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await;
        let Some(entry) = registry.get_mut(job_id) else {
            return;
        };
        if entry.snapshot.state != InteractiveReleaseSearchState::Running {
            return;
        }
        if let Some(indexer) = entry
            .snapshot
            .indexers
            .iter_mut()
            .find(|indexer| indexer.indexer_id == indexer_id)
        {
            indexer.status = status;
            indexer.failure_reason = failure_reason;
        }
    }

    /// Merge one indexer's evaluated batch into the job snapshot and re-run
    /// cross-indexer dedup over the merged set (per-indexer restricted calls
    /// only dedup within themselves). Late batches for a non-running job are
    /// dropped.
    async fn merge_interactive_indexer_batch(
        &self,
        job_id: &str,
        indexer_id: &str,
        batch: Vec<IndexerSearchResult>,
        indexer_priority_by_name: &HashMap<String, i64>,
        preferred_source_kind: &str,
    ) {
        let mut registry = self
            .runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await;
        let Some(entry) = registry.get_mut(job_id) else {
            return;
        };
        if entry.snapshot.state != InteractiveReleaseSearchState::Running {
            return;
        }
        let batch_len = batch.len();
        let mut merged = std::mem::take(&mut entry.snapshot.results);
        merged.extend(batch);
        let mut merged = dedupe_cross_indexer_release_results(
            merged,
            indexer_priority_by_name,
            preferred_source_kind,
        );
        // Re-sort the merged set with the one-shot path's comparator: the
        // GraphQL payload truncates to the requested limit, and without a
        // global re-sort a later indexer's top-scored releases would be cut
        // in arrival order.
        merged.sort_by(compare_release_search_results);
        entry.snapshot.results = merged;
        if let Some(indexer) = entry
            .snapshot
            .indexers
            .iter_mut()
            .find(|indexer| indexer.indexer_id == indexer_id)
        {
            indexer.status = InteractiveReleaseSearchIndexerStatus::Completed;
            indexer.result_count = batch_len;
            indexer.failure_reason = None;
        }
    }
}
