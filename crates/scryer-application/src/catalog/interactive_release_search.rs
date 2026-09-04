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

use super::release_search::{
    QualityProfileLookup, compare_release_search_results, dedupe_cross_indexer_release_results,
    incomplete_indexer_reason,
};
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::domain_events::{new_global_domain_event, title_context_snapshot};
use crate::quality_profile::evaluate_against_profile_for_category;
use scryer_domain::{DomainEventPayload, ReleaseGrabbedEventData};
use scryer_logging::{ActorContext, LogContext, ResourceContext, WorkflowContext, context_span};
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info, warn};

/// Overall deadline for a job; stragglers past this are marked failed.
const INTERACTIVE_RELEASE_SEARCH_DEADLINE: std::time::Duration =
    scryer_outbound_http::INDEXER_HTTP_TIMEOUT;
/// Terminal jobs are evicted this long after completion.
const COMPLETED_JOB_TTL_MINUTES: i64 = 5;
/// Defensive cap: running jobs older than this are cancelled and evicted.
const RUNNING_JOB_TTL_MINUTES: i64 = 10;
/// Per-actor cap on concurrently running jobs.
const MAX_RUNNING_JOBS_PER_ACTOR: usize = 8;
/// Releases one browser download may bundle (D17). Each one is a separate
/// upstream fetch, so the cap bounds how long a single request can hold.
const MAX_INTERACTIVE_SEARCH_ARTIFACT_DOWNLOADS: usize = 50;
/// Room for the extension and a dedupe suffix inside every filesystem's limit.
const ARTIFACT_FILE_NAME_STEM_MAX_BYTES: usize = 180;

/// One release the browser asked for, addressed by the job that produced it.
/// "Retry failed" merges a second job's rows into the same table, so one
/// selection can span several jobs and still has to come back as one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveSearchArtifactTarget {
    pub search_id: String,
    pub download_url: String,
}

/// One file for the browser: a release's own artifact, or the `tar.gz` holding
/// several of them (D17).
pub struct InteractiveSearchArtifactBundle {
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Summarises the payload rather than dumping it: a bundle carries megabytes.
impl std::fmt::Debug for InteractiveSearchArtifactBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InteractiveSearchArtifactBundle")
            .field("file_name", &self.file_name)
            .field("content_type", &self.content_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

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
    /// Routing priority for this indexer, `0` when routing states none.
    pub priority: i64,
    pub status: InteractiveReleaseSearchIndexerStatus,
    /// The indexer's own batch size (before cross-indexer dedup).
    pub result_count: usize,
    /// Wall time of this indexer's own call, once it has answered (D15).
    pub elapsed_ms: Option<i64>,
    pub failure_reason: Option<String>,
}

/// What a title-less query subject searches as. The kind picks the search
/// facet, the id-search facet and the default newznab categories; `Raw` picks
/// none of them and sends the query as plain text (D2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveSearchKind {
    Movie,
    Series,
    Anime,
    Raw,
}

impl InteractiveSearchKind {
    /// The media facet this kind searches as, or `None` for a raw text search.
    pub fn facet(self) -> Option<MediaFacet> {
        match self {
            Self::Movie => Some(MediaFacet::Movie),
            Self::Series => Some(MediaFacet::Series),
            Self::Anime => Some(MediaFacet::Anime),
            Self::Raw => None,
        }
    }

    fn as_str(self) -> &'static str {
        self.facet().as_ref().map_or("raw", MediaFacet::as_str)
    }
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

/// Start request — mirrors `SearchReleasesInput` field-for-field. Exactly one
/// of `title_id` (a catalog title) and `query` (an operator's raw text) names
/// the subject; the query fields are ignored by a title search and vice versa.
#[derive(Clone, Debug, Default)]
pub struct InteractiveReleaseSearchRequest {
    pub title_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season: Option<String>,
    pub episode: Option<String>,
    pub limit: Option<i32>,
    /// Raw operator query; required (and only meaningful) without a title.
    pub query: Option<String>,
    /// Required with `query`; picks the facet and the default categories.
    pub kind: Option<InteractiveSearchKind>,
    /// Restricts the fan-out to these indexer config ids (both subjects).
    pub indexer_ids: Option<Vec<String>>,
    /// Newznab categories for a query subject; defaults from the kind's facet.
    pub categories: Option<Vec<String>>,
}

/// What an unlinked grab (D8) reports back: the download the client accepted,
/// the client that took it and the release name Activity will show.
#[derive(Clone, Debug)]
pub struct QueueUnlinkedReleaseOutcome {
    /// The download client's own item id — the key Activity and the
    /// tracked-download poller surface this download under.
    pub download_id: String,
    pub client_name: String,
    /// Release name exactly as the indexer announced it.
    pub source_title: String,
}

pub(crate) struct InteractiveReleaseSearchJobEntry {
    pub(crate) snapshot: InteractiveReleaseSearchSnapshot,
    pub(crate) actor_id: String,
    pub(crate) scope_key: String,
    /// The query subject's kind, kept because a grab out of this search has no
    /// title of its own to read a facet from (D8). `None` for a title subject.
    pub(crate) kind: Option<InteractiveSearchKind>,
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

/// Replace key: starting a search cancels the actor's running job for the same
/// subject. A query subject is the same subject when the kind, the text and the
/// restricted indexer set all match.
fn interactive_release_search_scope_key(request: &InteractiveReleaseSearchRequest) -> String {
    match request.query.as_deref() {
        Some(query) => format!(
            "q|{}|{}|{}",
            request.kind.map_or("raw", InteractiveSearchKind::as_str),
            query.trim(),
            request.indexer_ids.as_deref().unwrap_or_default().join(","),
        ),
        None => format!(
            "{}|{}|{}|{}",
            request.title_id.as_deref().unwrap_or("-"),
            request.series_movie_link_id.as_deref().unwrap_or("-"),
            request.season.as_deref().unwrap_or("-"),
            request.episode.as_deref().unwrap_or("-"),
        ),
    }
}

/// The two subjects one job can search for (D3). Everything downstream of the
/// per-indexer call — merge, dedupe, status, registry, TTLs, cancel, poll — is
/// shared; only the call itself branches.
enum InteractiveReleaseSearchSubject {
    /// A catalog title: the existing scored, token-attaching path.
    Title {
        /// For series-movie searches this is the synthesized movie search title.
        /// Boxed: a `Title` dwarfs the query variant, and the subject is moved
        /// into the job context once and then only borrowed.
        title_for_search: Box<Title>,
        subject: ResolvedReleaseSearchSubject,
        preserve_subject_scope: bool,
    },
    /// An operator's raw query, with no title to score or judge against (D6).
    Query {
        query: String,
        /// Search facet and id-search facet; `None` for a raw text query.
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        /// Facet default profile, its weights and the facet name. `None` for
        /// the raw kind, and when the profile could not be resolved.
        judge: Option<(QualityProfile, ScoringWeights, String)>,
    },
}

/// Everything the spawned runner needs, resolved once at start.
struct InteractiveReleaseSearchJobContext {
    job_id: String,
    actor: User,
    subject: InteractiveReleaseSearchSubject,
    /// Routing entries for every enabled indexer. The query subject restricts a
    /// copy of this to one indexer per task; the title subject's restriction is
    /// applied inside the search pipeline, so it leaves this empty.
    routing_base: HashMap<String, IndexerRoutingEntry>,
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
        // Resolve the subject once per job, not once per indexer, together with
        // the routing plan its cross-indexer dedup metadata is derived from.
        // Best-effort routing: an unresolved plan degrades to an empty priority
        // map, never a failed job.
        let (subject, indexer_routing) = match (
            request.title_id.as_deref(),
            request.query.as_deref().map(str::trim),
        ) {
            (Some(title_id), None) => {
                validate_interactive_search_subject_shape(
                    request.series_movie_link_id.as_deref(),
                    request.season.as_deref(),
                    request.episode.as_deref(),
                )?;
                let title = self
                    .services
                    .catalog
                    .titles
                    .get_by_id(title_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
                self.require_library_permission(
                    actor,
                    &title.library_id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?;
                let (title_for_search, subject, preserve_subject_scope) = self
                    .resolve_interactive_search_title_subject(
                        &title,
                        request.series_movie_link_id.as_deref(),
                        request.season.as_deref(),
                        request.episode.as_deref(),
                    )
                    .await?;

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
                (
                    InteractiveReleaseSearchSubject::Title {
                        title_for_search: Box::new(title_for_search),
                        subject,
                        preserve_subject_scope,
                    },
                    indexer_routing,
                )
            }
            (None, Some(query)) if !query.is_empty() => {
                // The Indexers page's own gate (D13): a title-less search
                // reaches every configured indexer regardless of library.
                self.require_app_permission(
                    actor,
                    scryer_domain::AppPermission::ManageSystemSettings,
                )
                .await?;
                let Some(kind) = request.kind else {
                    return Err(AppError::Validation(
                        "a search kind is required with a query".to_string(),
                    ));
                };
                let facet = kind.facet();
                let facet_name = facet.as_ref().map(|facet| facet.as_str().to_string());
                let indexer_routing = self
                    .resolve_indexer_routing(None, facet_name.as_deref())
                    .await;
                let newznab_categories = request
                    .categories
                    .clone()
                    .map(|values| {
                        values
                            .into_iter()
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .filter(|values| !values.is_empty())
                    .or_else(|| {
                        facet.as_ref().map(|facet| {
                            crate::settings::keys::default_indexer_routing_categories_for_scope(
                                facet.as_str(),
                            )
                        })
                    })
                    .filter(|values| !values.is_empty());
                let judge = match facet {
                    Some(facet) => self.resolve_query_subject_judge(facet).await,
                    None => None,
                };
                (
                    InteractiveReleaseSearchSubject::Query {
                        query: query.to_string(),
                        facet: facet_name,
                        newznab_categories,
                        judge,
                    },
                    indexer_routing,
                )
            }
            (None, Some(_)) => {
                return Err(AppError::Validation("search query is required".to_string()));
            }
            _ => {
                return Err(AppError::Validation(
                    "provide exactly one of title id or query".to_string(),
                ));
            }
        };

        let indexer_priority_by_name = self
            .build_indexer_priority_by_name(indexer_routing.as_ref())
            .await;
        let preferred_source_kind = self.download_source_capabilities().await.2;

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        if let Some(requested) = request.indexer_ids.as_deref().filter(|ids| !ids.is_empty()) {
            let known = configs
                .iter()
                .map(|config| config.id.as_str())
                .collect::<HashSet<_>>();
            if let Some(unknown) = requested.iter().find(|id| !known.contains(id.as_str())) {
                return Err(AppError::Validation(format!("unknown indexer {unknown}")));
            }
        }

        // Config-visible eligibility only: enabled + interactive, minus any
        // indexer the request did not name. A config cooldown lists the indexer
        // as Skipped without dispatching; a routing-disabled indexer is omitted
        // entirely. Backoff state stays with the search client (a backed-off
        // indexer's restricted call just returns empty).
        let wants_routing_base = matches!(subject, InteractiveReleaseSearchSubject::Query { .. });
        let now = self.runtime.environment.now();
        let mut indexer_views = Vec::new();
        let mut dispatch = Vec::new();
        let requested_indexers = request
            .indexer_ids
            .as_ref()
            .filter(|ids| !ids.is_empty())
            .map(|ids| ids.iter().cloned().collect::<HashSet<String>>());
        let mut routing_base = HashMap::new();
        for config in configs {
            if !config.is_enabled {
                continue;
            }
            let routing_entry = indexer_routing
                .as_ref()
                .and_then(|plan| plan.entries.get(&config.id));
            if wants_routing_base {
                // Indexers missing from a plan are searched by default, so the
                // base must cover every enabled one for the per-task
                // restriction to hold.
                routing_base.insert(
                    config.id.clone(),
                    routing_entry.cloned().unwrap_or(IndexerRoutingEntry {
                        enabled: true,
                        categories: Vec::new(),
                        priority: 0,
                    }),
                );
            }
            if !config.enable_interactive_search {
                continue;
            }
            // A routing-disabled indexer and one the request did not name are
            // both ineligible; the shared contract decides, so this stays in
            // step with the search client's own dispatch rule.
            if crate::contracts::indexer_search_eligibility(
                indexer_routing.as_ref(),
                requested_indexers.as_ref(),
                &config.id,
            ) != crate::contracts::IndexerSearchEligibility::Eligible
            {
                continue;
            }
            let priority = indexer_priority_by_name
                .get(config.name.as_str())
                .copied()
                .unwrap_or(0);
            if config.disabled_until.is_some_and(|until| until > now) {
                indexer_views.push(InteractiveReleaseSearchIndexerView {
                    indexer_id: config.id,
                    name: config.name,
                    priority,
                    status: InteractiveReleaseSearchIndexerStatus::Skipped,
                    result_count: 0,
                    elapsed_ms: None,
                    failure_reason: Some("temporarily disabled".to_string()),
                });
                continue;
            }
            dispatch.push(config.id.clone());
            indexer_views.push(InteractiveReleaseSearchIndexerView {
                indexer_id: config.id,
                name: config.name,
                priority,
                status: InteractiveReleaseSearchIndexerStatus::Pending,
                result_count: 0,
                elapsed_ms: None,
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
                    kind: request.kind,
                    cancel: cancel.clone(),
                },
            );
        }

        let context = InteractiveReleaseSearchJobContext {
            job_id,
            actor: actor.clone(),
            subject,
            routing_base,
            dispatch,
            indexer_priority_by_name,
            preferred_source_kind,
            cancel,
        };
        // A query subject has no title to name in the log context.
        let (title_id, logged_query, category) = match &context.subject {
            InteractiveReleaseSearchSubject::Title {
                title_for_search,
                subject,
                ..
            } => (
                Some(title_for_search.id.clone()),
                subject.queries.first().map(String::as_str).unwrap_or(""),
                subject.category.as_str(),
            ),
            InteractiveReleaseSearchSubject::Query { query, facet, .. } => {
                (None, query.as_str(), facet.as_deref().unwrap_or("raw"))
            }
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
                title_id: title_id.clone(),
                job_id: Some(context.job_id.clone()),
                ..ResourceContext::default()
            }),
        );
        log_span.in_scope(|| {
            info!(
                actor = context.actor.id.as_str(),
                title_id = title_id.as_deref().unwrap_or(""),
                job_id = context.job_id.as_str(),
                query = logged_query,
                category = category,
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
            subject,
            routing_base,
            dispatch,
            indexer_priority_by_name,
            preferred_source_kind,
            cancel,
        } = context;
        // Shared rather than cloned per task: the title subject carries a whole
        // Title and its resolved search subject.
        let subject = Arc::new(subject);
        let routing_base = Arc::new(routing_base);

        let mut set = JoinSet::new();
        for indexer_id in dispatch {
            let app = self.clone();
            let actor = actor.clone();
            let subject = Arc::clone(&subject);
            let routing_base = Arc::clone(&routing_base);
            let job_id = job_id.clone();
            let child_token = cancel.child_token();
            set.spawn(async move {
                app.set_interactive_indexer_status(
                    &job_id,
                    &indexer_id,
                    InteractiveReleaseSearchIndexerStatus::Searching,
                    None,
                    None,
                )
                .await;
                let began = std::time::Instant::now();
                let outcome = match subject.as_ref() {
                    InteractiveReleaseSearchSubject::Title {
                        title_for_search,
                        subject,
                        preserve_subject_scope,
                    } => {
                        match app
                            .search_and_evaluate_subject_restricted_with_outcome(
                                title_for_search,
                                subject,
                                &actor.id,
                                SearchMode::Interactive,
                                child_token,
                                Some(HashSet::from([indexer_id.clone()])),
                                None,
                            )
                            .await
                        {
                            Ok(mut search_outcome) => {
                                let failure_reason = search_outcome
                                    .incomplete_indexer_reasons
                                    .remove(&indexer_id);
                                app.attach_candidate_tokens(
                                    &actor,
                                    title_for_search,
                                    subject,
                                    &mut search_outcome.results,
                                    *preserve_subject_scope,
                                )
                                .await;
                                Ok((search_outcome.results, failure_reason))
                            }
                            Err(error) => Err(error),
                        }
                    }
                    InteractiveReleaseSearchSubject::Query {
                        query,
                        facet,
                        newznab_categories,
                        judge,
                    } => {
                        app.search_query_subject_on_indexer(
                            query,
                            facet,
                            newznab_categories,
                            judge,
                            &routing_base,
                            &indexer_id,
                            child_token,
                        )
                        .await
                    }
                };
                let elapsed_ms = i64::try_from(began.elapsed().as_millis()).unwrap_or(i64::MAX);
                (indexer_id, elapsed_ms, outcome)
            });
        }

        let drain = async {
            while let Some(joined) = set.join_next().await {
                let (indexer_id, elapsed_ms, outcome) = match joined {
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
                            elapsed_ms,
                        )
                        .await;
                        if let Some(reason) = failure_reason {
                            self.set_interactive_indexer_status(
                                &job_id,
                                &indexer_id,
                                InteractiveReleaseSearchIndexerStatus::Failed,
                                Some(reason),
                                Some(elapsed_ms),
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
                            Some(elapsed_ms),
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
        elapsed_ms: Option<i64>,
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
            if elapsed_ms.is_some() {
                indexer.elapsed_ms = elapsed_ms;
            }
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
        elapsed_ms: i64,
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
            indexer.elapsed_ms = Some(elapsed_ms);
            indexer.failure_reason = None;
        }
    }

    /// One indexer's answer to a query subject: its own catalogue, parsed and —
    /// for a faceted kind — judged against the facet's default profile (D6).
    /// No candidate tokens: those are minted at grab time, once a title is
    /// chosen (D4).
    #[expect(
        clippy::too_many_arguments,
        reason = "the call carries the whole query envelope plus the indexer it is restricted to"
    )]
    async fn search_query_subject_on_indexer(
        &self,
        query: &str,
        facet: &Option<String>,
        newznab_categories: &Option<Vec<String>>,
        judge: &Option<(QualityProfile, ScoringWeights, String)>,
        routing_base: &HashMap<String, IndexerRoutingEntry>,
        indexer_id: &str,
        cancel_token: CancellationToken,
    ) -> AppResult<(Vec<IndexerSearchResult>, Option<String>)> {
        // The only routing surface the multi-indexer client reads: restrict to
        // one indexer by disabling every other entry, as `catalog::discovery`
        // does for the convergence subset.
        let plan = IndexerRoutingPlan {
            entries: routing_base
                .iter()
                .map(|(id, entry)| {
                    (
                        id.clone(),
                        IndexerRoutingEntry {
                            enabled: id == indexer_id,
                            ..entry.clone()
                        },
                    )
                })
                .collect(),
        };
        let response = self
            .services
            .integrations
            .indexer_client
            .search(
                query.to_string(),
                HashMap::new(),
                None,
                // The id-search facet mirrors the search facet: this subject
                // never carries external ids.
                facet.clone(),
                facet.clone(),
                newznab_categories.clone(),
                Some(plan),
                SearchMode::Interactive,
                IndexerErrorOperation::InteractiveSearch,
                None,
                None,
                None,
                None,
                Vec::new(),
                None,
                cancel_token,
            )
            .await?;
        let failure_reason = response
            .indexer_outcomes
            .iter()
            .find(|outcome| outcome.indexer_id == indexer_id)
            .and_then(|outcome| incomplete_indexer_reason(outcome.outcome));
        let results = response
            .results
            .into_iter()
            .map(|mut result| {
                if result
                    .indexer_id
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    result.indexer_id = Some(indexer_id.to_string());
                }
                let parsed = result
                    .parsed_release_metadata
                    .take()
                    .unwrap_or_else(|| crate::parse_release_metadata(&result.title));
                if let Some((profile, weights, category)) = judge {
                    // `has_existing_file: false` — with no title there is no
                    // incumbent, so cutoff and upgrade blocks cannot fire.
                    result.quality_profile_decision = Some(evaluate_against_profile_for_category(
                        profile,
                        &parsed,
                        false,
                        weights,
                        Some(category.as_str()),
                    ));
                }
                result.parsed_release_metadata = Some(parsed);
                result
            })
            .collect();
        Ok((results, failure_reason))
    }

    /// The facet's default quality profile and its weights — everything a
    /// context-free rejection can be based on (D6). Best effort: an
    /// unresolvable profile means the pane shows releases without profile
    /// rejections, never a failed search.
    async fn resolve_query_subject_judge(
        &self,
        facet: MediaFacet,
    ) -> Option<(QualityProfile, ScoringWeights, String)> {
        let category = facet.as_str().to_string();
        let profile = match self
            .resolve_quality_profile(QualityProfileLookup {
                title_tags: &[],
                library_id: None,
                imdb_id: None,
                tvdb_id: None,
                category_hint: Some(category.as_str()),
            })
            .await
        {
            Ok(profile) => profile,
            Err(error) => {
                warn!(
                    error = %error,
                    facet = category.as_str(),
                    "interactive query search: default quality profile unresolved; no rejections"
                );
                return None;
            }
        };
        let persona = self
            .resolve_scoring_persona(None, Some(category.as_str()))
            .await
            .unwrap_or_default();
        let weights = crate::build_weights_for_category(
            &persona,
            &profile.criteria.scoring_overrides,
            Some(category.as_str()),
        );
        Some((profile, weights, category))
    }

    /// Mint a candidate token for one release of an existing search (D4).
    ///
    /// The release is named by the download URL the search payload already
    /// handed the browser and must still be in that actor's job snapshot, so a
    /// token can never be minted for a release the operator did not see. The
    /// subject is resolved exactly as a search of the same shape would.
    pub async fn issue_interactive_release_candidate_token(
        &self,
        actor: &User,
        search_id: &str,
        download_url: &str,
        title_id: &str,
        season: Option<String>,
        episode: Option<String>,
    ) -> AppResult<IndexerSearchResult> {
        validate_interactive_search_subject_shape(None, season.as_deref(), episode.as_deref())?;
        let (mut result, _) = self
            .find_interactive_search_result(actor, search_id, download_url)
            .await?;

        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let (title_for_search, subject, preserve_subject_scope) = self
            .resolve_interactive_search_title_subject(
                &title,
                None,
                season.as_deref(),
                episode.as_deref(),
            )
            .await?;
        self.attach_candidate_tokens(
            actor,
            &title_for_search,
            &subject,
            std::slice::from_mut(&mut result),
            preserve_subject_scope,
        )
        .await;
        Ok(result)
    }

    /// Grab one release of an existing search with no title at all (D8).
    ///
    /// The release is submitted to the client the operator picked and recorded
    /// the way the tracker records an adopted foreign item: title-less, orphan
    /// scope. Nothing claims the download for a catalog title, so the completed
    /// download surfaces in Activity for a manual import instead of being
    /// auto-imported.
    pub async fn queue_unlinked_release(
        &self,
        actor: &User,
        search_id: &str,
        download_url: &str,
        download_client_id: &str,
    ) -> AppResult<QueueUnlinkedReleaseOutcome> {
        // The Indexers page's own gate (D13): an unlinked grab bypasses every
        // library, so it is gated on system settings rather than a library.
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (result, kind) = self
            .find_interactive_search_result(actor, search_id, download_url)
            .await?;
        let client = self
            .services
            .integrations
            .download_client_configs
            .get_by_id(download_client_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "download client {download_client_id} does not exist"
                ))
            })?;
        if !client.is_enabled {
            return Err(AppError::Validation(format!(
                "download client {} is disabled",
                client.name
            )));
        }
        let Some((source_hint, source_kind)) = result.canonical_download_source() else {
            return Err(AppError::Validation(
                "release has no usable download source".to_string(),
            ));
        };

        // With no title there is no owner facet. The operator's search kind is
        // the stated intent; a raw search falls back to what the release parses
        // as, the same read the tracked-download reconciler makes.
        let facet = kind
            .and_then(InteractiveSearchKind::facet)
            .unwrap_or_else(|| {
                if result
                    .parsed_release_metadata
                    .as_ref()
                    .is_some_and(|parsed| parsed.episode.is_some())
                {
                    MediaFacet::Series
                } else {
                    MediaFacet::Movie
                }
            });
        let stand_in_title =
            unlinked_grab_title(&result.title, facet.clone(), self.runtime.environment.now());
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let info_hash_hint = result
            .extra
            .get("info_hash")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let grab = self
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: stand_in_title.clone(),
                search_facet: None,
                purpose: DownloadSubmissionPurpose::OperatorQueued,
                download_id: Some(download_id),
                source_hint: Some(source_hint.clone()),
                staged_nzb: None,
                resolved_download_artifact: None,
                source_kind: Some(source_kind),
                source_title: Some(result.title.clone()),
                source_password: result.password_hint.clone(),
                // Left to the router's grab-time choke point: the routing entry
                // for the pinned client decides the category, and "no entry"
                // means the download client's own default (D16).
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: Some(result.source.clone()),
                indexer_id: result.indexer_id.clone(),
                info_hash_hint: info_hash_hint.clone(),
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                // No title means no quality profile to read tracker-minimum
                // honouring off, so the release's own minimums are not clamped
                // in — the same position the manual queue path takes.
                tracker_min_seed_ratio: None,
                tracker_min_seed_time_minutes: None,
                season_pack_seed_ratio: None,
                season_pack_seed_time_minutes: None,
                is_recent: None,
                season_pack: None,
                pinned_download_client_id: Some(client.id.clone()),
            })
            .await?;

        self.services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                download_id,
                title_id: String::new(),
                facet: facet.as_str().to_string(),
                download_client_id: grab.client_id.clone(),
                download_client_type: grab.client_type.clone(),
                download_client_item_id: grab.job_id.clone(),
                source_hint: Some(source_hint.clone()),
                source_provider_id: result.indexer_id.clone(),
                source_provider_name: Some(result.source.clone()),
                source_kind: Some(source_kind),
                source_title: Some(result.title.clone()),
                info_hash: info_hash_hint,
                release_size_bytes: result.size_bytes,
                request_signature: None,
                purpose: DownloadSubmissionPurpose::OperatorQueued,
                scope: SubmissionScope::Orphan,
            })
            .await?;

        self.append_domain_event(new_global_domain_event(
            actor,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                // No catalog title to snapshot: the release name stands in for
                // one so history reads as the operator saw it.
                title: title_context_snapshot(&stand_in_title),
                source_title: Some(result.title.clone()),
                source_hint: Some(source_hint),
                source_provider: Some(result.source.clone()),
                download_id: Some(grab.job_id.clone()),
                episode_ids: Vec::new(),
            }),
        ))
        .await?;

        Ok(QueueUnlinkedReleaseOutcome {
            download_id: grab.job_id,
            client_name: client.name,
            source_title: result.title,
        })
    }

    /// Hand the operator the selected releases' own files (D17, FR-028).
    ///
    /// A third grab mode next to "assign to a title" and "grab unlinked":
    /// nothing is submitted, queued or tracked, so there is no submission row
    /// to follow — but from the indexer's perspective these are grabs, so each
    /// one lands in History exactly as an unlinked grab does. All or nothing:
    /// one failed fetch fails the whole request and emits no history at all.
    pub async fn download_interactive_search_artifacts(
        &self,
        actor: &User,
        targets: &[InteractiveSearchArtifactTarget],
    ) -> AppResult<InteractiveSearchArtifactBundle> {
        // Same gate as the unlinked grab (D13): this bypasses every library.
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        if targets.is_empty() {
            return Err(AppError::Validation(
                "select at least one release to download".to_string(),
            ));
        }
        if targets.len() > MAX_INTERACTIVE_SEARCH_ARTIFACT_DOWNLOADS {
            return Err(AppError::Validation(format!(
                "at most {MAX_INTERACTIVE_SEARCH_ARTIFACT_DOWNLOADS} releases can be downloaded at once"
            )));
        }

        let now = self.runtime.environment.now();
        let mut artifacts: Vec<FetchedSearchArtifact> = Vec::with_capacity(targets.len());
        for target in targets {
            let (result, kind) = self
                .find_interactive_search_result(actor, &target.search_id, &target.download_url)
                .await?;
            let Some((source_hint, source_kind)) = result.canonical_download_source() else {
                return Err(AppError::Validation(format!(
                    "{} has no usable download source",
                    result.title
                )));
            };
            // With no title there is no owner facet, so the search kind stands
            // in for one exactly as the unlinked grab reads it.
            let facet = kind
                .and_then(InteractiveSearchKind::facet)
                .unwrap_or_else(|| {
                    if result
                        .parsed_release_metadata
                        .as_ref()
                        .is_some_and(|parsed| parsed.episode.is_some())
                    {
                        MediaFacet::Series
                    } else {
                        MediaFacet::Movie
                    }
                });
            let stand_in_title = unlinked_grab_title(&result.title, facet, now);
            let info_hash_hint = result
                .extra
                .get("info_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let artifact = self
                .services
                .integrations
                .download_client
                .fetch_release_artifact(&DownloadClientAddRequest {
                    title: stand_in_title.clone(),
                    search_facet: None,
                    purpose: DownloadSubmissionPurpose::OperatorQueued,
                    // Nothing is submitted, so there is no download identity to
                    // mint and nothing for the tracker to reconcile later.
                    download_id: None,
                    source_hint: Some(source_hint.clone()),
                    staged_nzb: None,
                    resolved_download_artifact: None,
                    source_kind: Some(source_kind),
                    source_title: Some(result.title.clone()),
                    source_password: result.password_hint.clone(),
                    category: None,
                    queue_priority: None,
                    download_directory: None,
                    release_title: None,
                    indexer_name: Some(result.source.clone()),
                    indexer_id: result.indexer_id.clone(),
                    info_hash_hint,
                    seed_goal_ratio: None,
                    seed_goal_seconds: None,
                    tracker_min_seed_ratio: None,
                    tracker_min_seed_time_minutes: None,
                    season_pack_seed_ratio: None,
                    season_pack_seed_time_minutes: None,
                    is_recent: None,
                    season_pack: None,
                    pinned_download_client_id: None,
                })
                .await
                .map_err(|error| prefix_app_error(error, &result.title))?;

            let (extension, default_content_type, bytes, content_type) = match artifact {
                ResolvedDownloadArtifact::Nzb {
                    bytes,
                    content_type,
                    ..
                } => (".nzb", "application/x-nzb", bytes, content_type),
                ResolvedDownloadArtifact::TorrentFile {
                    bytes,
                    content_type,
                    ..
                } => (".torrent", "application/x-bittorrent", bytes, content_type),
                ResolvedDownloadArtifact::Magnet { .. } => {
                    return Err(AppError::Validation(format!(
                        "{} is a magnet link and has no file to download",
                        result.title
                    )));
                }
            };
            artifacts.push(FetchedSearchArtifact {
                file_name: artifact_file_name(&result.title, extension),
                content_type: content_type.unwrap_or_else(|| default_content_type.to_string()),
                bytes,
                stand_in_title,
                source_title: result.title,
                source_hint,
                source_provider: result.source,
            });
        }

        let bundle = if artifacts.len() == 1 {
            let artifact = &artifacts[0];
            InteractiveSearchArtifactBundle {
                file_name: artifact.file_name.clone(),
                content_type: artifact.content_type.clone(),
                bytes: artifact.bytes.clone(),
            }
        } else {
            InteractiveSearchArtifactBundle {
                file_name: format!("scryer-releases-{}.tar.gz", now.format("%Y%m%d-%H%M%S")),
                content_type: "application/gzip".to_string(),
                bytes: build_release_artifact_archive(&artifacts, now)?,
            }
        };

        // Every fetch succeeded, so every release was grabbed as far as the
        // indexer is concerned; a failed bundle above emitted nothing.
        for artifact in artifacts {
            self.append_domain_event(new_global_domain_event(
                actor,
                DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                    title: title_context_snapshot(&artifact.stand_in_title),
                    source_title: Some(artifact.source_title),
                    source_hint: Some(artifact.source_hint),
                    source_provider: Some(artifact.source_provider),
                    // Nothing was submitted, so there is no client item id.
                    download_id: None,
                    episode_ids: Vec::new(),
                }),
            ))
            .await?;
        }

        Ok(bundle)
    }

    /// Locate one release of the actor's own live search by the download URL
    /// the search payload already handed the browser, with the search's kind.
    ///
    /// Shared by candidate-token issuance (D4) and the unlinked grab (D8) so
    /// neither can act on a release the operator never saw.
    async fn find_interactive_search_result(
        &self,
        actor: &User,
        search_id: &str,
        download_url: &str,
    ) -> AppResult<(IndexerSearchResult, Option<InteractiveSearchKind>)> {
        let mut registry = self
            .runtime
            .acquisition
            .interactive_release_searches
            .lock()
            .await;
        evict_stale_entries(&mut registry, self.runtime.environment.now());
        let entry = registry
            .get(search_id)
            .filter(|entry| entry.actor_id == actor.id)
            .ok_or_else(|| AppError::NotFound(format!("interactive release search {search_id}")))?;
        let result = entry
            .snapshot
            .results
            .iter()
            .find(|result| {
                result
                    .download_url
                    .as_deref()
                    .or(result.link.as_deref())
                    .is_some_and(|value| value == download_url)
            })
            .cloned()
            .ok_or_else(|| AppError::NotFound("release is no longer in this search".to_string()))?;
        Ok((result, entry.kind))
    }

    /// Resolve the title-subject search target for `(series_movie_link_id,
    /// season, episode)`, returning the search title, the subject and whether
    /// the subject's own submission scope must be preserved. Shared by the
    /// start path and by candidate-token issuance so a token is minted against
    /// exactly the subject a search of the same shape would have used.
    async fn resolve_interactive_search_title_subject(
        &self,
        title: &Title,
        series_movie_link_id: Option<&str>,
        season: Option<&str>,
        episode: Option<&str>,
    ) -> AppResult<(Title, ResolvedReleaseSearchSubject, bool)> {
        match (series_movie_link_id, season, episode) {
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
                    .resolve_release_search_subject_for_series_movie(title, &link)
                    .await?;
                Ok((search_title, subject, true))
            }
            (None, Some(season), Some(episode)) => {
                let subject = self
                    .resolve_release_search_subject_for_episode(title, season, episode)
                    .await?;
                Ok((title.clone(), subject, false))
            }
            _ => {
                let subject = self.resolve_release_search_subject_for_title(title).await?;
                Ok((title.clone(), subject, false))
            }
        }
    }
}

/// One release's file, held until every release in the request has resolved.
struct FetchedSearchArtifact {
    file_name: String,
    content_type: String,
    bytes: Vec<u8>,
    stand_in_title: Title,
    source_title: String,
    source_hint: String,
    source_provider: String,
}

/// Restate which release an artifact fetch failed on: the operator picked
/// several and the message is all they see. The kind is flattened to a
/// validation failure because that is what it is for the caller — one of the
/// releases they chose cannot be downloaded.
fn prefix_app_error(error: AppError, release_title: &str) -> AppError {
    let rendered = error.to_string();
    let detail = rendered.strip_prefix("validation: ").unwrap_or(&rendered);
    AppError::Validation(format!("{release_title}: {detail}"))
}

/// `<sanitized release title><extension>`, safe to put in a
/// `Content-Disposition` header and in a tar entry.
pub(crate) fn artifact_file_name(title: &str, extension: &str) -> String {
    let mut stem = String::new();
    let mut pending_space = false;
    for character in title.chars() {
        // Path separators and control characters would let a release name
        // escape the archive root or forge a header line.
        if character.is_control() || character == '/' || character == '\\' {
            continue;
        }
        if character.is_whitespace() {
            pending_space = !stem.is_empty();
            continue;
        }
        if pending_space {
            stem.push(' ');
            pending_space = false;
        }
        stem.push(character);
    }
    // A leading dot would make the file hidden, and `.`/`..` are directories.
    let mut stem = stem.trim_start_matches('.').trim().to_string();
    while stem.len() > ARTIFACT_FILE_NAME_STEM_MAX_BYTES {
        stem.pop();
    }
    let stem = stem.trim_end();
    if stem.is_empty() {
        return format!("release{extension}");
    }
    format!("{stem}{extension}")
}

/// Keep every archive member's name unique: two indexers routinely answer with
/// the same release name.
pub(crate) fn dedupe_archive_file_name(
    taken: &mut std::collections::HashSet<String>,
    file_name: &str,
) -> String {
    if taken.insert(file_name.to_string()) {
        return file_name.to_string();
    }
    let (stem, extension) = match file_name.rfind('.') {
        Some(index) if index > 0 => file_name.split_at(index),
        _ => (file_name, ""),
    };
    let mut ordinal = 2usize;
    loop {
        let candidate = format!("{stem} ({ordinal}){extension}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        ordinal += 1;
    }
}

/// Every file at the archive root, so an extract drops them beside each other.
fn build_release_artifact_archive(
    artifacts: &[FetchedSearchArtifact],
    now: DateTime<Utc>,
) -> AppResult<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut taken = std::collections::HashSet::new();
    let mtime = now.timestamp().max(0) as u64;
    for artifact in artifacts {
        let file_name = dedupe_archive_file_name(&mut taken, &artifact.file_name);
        let mut header = tar::Header::new_gnu();
        header.set_path(&file_name).map_err(|error| {
            AppError::Repository(format!("failed to name {file_name}: {error}"))
        })?;
        header.set_size(artifact.bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(mtime);
        header.set_cksum();
        archive
            .append(&header, artifact.bytes.as_slice())
            .map_err(|error| {
                AppError::Repository(format!("failed to archive {file_name}: {error}"))
            })?;
    }
    archive
        .into_inner()
        .map_err(|error| AppError::Repository(format!("failed to finish archive: {error}")))?
        .finish()
        .map_err(|error| AppError::Repository(format!("failed to compress archive: {error}")))
}

/// The title-less stand-in an unlinked grab submits under (D8).
///
/// Every download client reads the request's title for its own bookkeeping —
/// the file name it falls back to when a release name is missing, the
/// `*scryer_title_id` / `*scryer_facet` parameters, the routing scope the
/// category is read from. An empty id is what the adopted-foreign-item path
/// already records for a title-less download, and the tracked-download
/// reconciler treats an empty `*scryer_title_id` as absent, so the grab stays
/// unowned end to end and lands in Activity for a manual import.
fn unlinked_grab_title(release_title: &str, facet: MediaFacet, now: DateTime<Utc>) -> Title {
    Title {
        id: String::new(),
        library_id: String::new(),
        name: release_title.to_string(),
        facet,
        monitored: false,
        tags: Vec::new(),
        external_ids: Vec::new(),
        root_folder_id: String::new(),
        created_by: None,
        created_at: now,
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        canonical_tags: Vec::new(),
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

/// Same input-shape validation (and messages) as the one-shot `searchReleases`
/// resolver.
fn validate_interactive_search_subject_shape(
    series_movie_link_id: Option<&str>,
    season: Option<&str>,
    episode: Option<&str>,
) -> AppResult<()> {
    match (series_movie_link_id, season, episode) {
        (Some(_), None, None) | (None, Some(_), Some(_)) | (None, None, None) => Ok(()),
        (None, Some(_), None) | (None, None, Some(_)) => Err(AppError::Validation(
            "episode searches require both season and episode".to_string(),
        )),
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => Err(AppError::Validation(
            "series movie searches cannot include season or episode".to_string(),
        )),
    }
}
