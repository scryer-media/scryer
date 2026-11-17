use super::*;
#[allow(unused_imports)]
use crate::test_wait::{TEST_WAIT_DEADLINE, wait_for, wait_until, within_deadline};
use async_trait::async_trait;
use base64::Engine as _;
use scryer_domain::{
    Collection, CollectionType, DomainEventFilter, DomainEventPayload, DomainEventType, Episode,
    EpisodeType, EventType, ImportSkipReason, ImportType, JobRunCompletedEventData,
    JobRunStartedEventData, MediaRequestRequester, MediaRequestStatus, RootFolderEntry,
    TrackedDownloadState,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, Instant, sleep, timeout};

mod acquisition_recovery;
mod activity_read_paths;
mod bridge_cour_titles;
mod consolidation;
mod cross_library_transfer;
mod custom_job_shutdown;
mod diacritic_release_matching;
mod discovery_sync;
mod download_enrichment;
mod downloads;
pub(crate) use downloads::RecordingDownloadRegistry;
mod episode_file_delete;
mod episode_metadata_refresh;
mod file_privacy;
mod folder_match;
mod full_hash_backfill;
mod golden_resolution_corpus;
mod grab_job;
mod import_existing_score;
mod import_rejection_reopen;
mod indexer_backoff_reset;
mod indexer_download_client_mappings;
mod indexer_monthly_limit_grabs;
mod interactive_release_search;
mod job_run_terminal_write;
mod landed_bar_memo;
mod libraries;
mod library_scan;
mod lifecycle_action_sequences;
mod lifecycle_claims;
mod lifecycle_evaluation;
mod lifecycle_execution;
mod lifecycle_rules;
mod lifecycle_safety;
mod lifecycle_sequence_deletion;
mod lifecycle_sequence_execution;
mod lifecycle_sequence_history;
mod lifecycle_storage_execution;
mod lifecycle_title_completion;
mod lifecycle_watch_facts;
mod lifecycle_watch_recheck;
mod list_account_keys;
mod list_accounts;
mod list_actions;
mod list_experimental_gate;
mod list_provider_settings;
mod list_user_events;
mod media_analysis;
mod media_requests;
mod media_server_signals;
mod metadata_search;
mod multilingual_title_matching;
mod queueing;
mod recycle_bin_roots;
mod recycle_purge_selection;
mod release_anchor_prefetch;
mod rename_folder_record;
mod request_rules;
mod request_rules_facts;
mod romaji_release_matching;
mod root_change;
mod root_move;
mod routing_settings;
mod rss_match_batching;
mod rule_tester_listing;
mod russian_release_matching;
mod score_evaluation;
mod script_interpreter_settings;
mod search_cutoff;
mod security_auth;
mod seeding_gate;
mod seeding_profiles;
mod series_metadata;
mod series_title_hydration;
mod subtitle_circuit;
mod subtitle_permissions;
mod title_catalog_reads;
mod title_delete_missing_folder;
mod title_hydration;
mod title_image_cache;
mod title_matcher_invalidation;
mod title_updates;
mod unconfirmed_grab_release;
mod user_permissions;
mod users_admin_titles;
mod verification_settings;
mod wanted_scope_lookup;

mod request_rules_support;
mod support_acquisition_downloads;
mod support_bootstrap_fixtures;
mod support_catalog;
mod support_events_requests;
mod support_imports;
mod support_indexers_metadata;
mod support_library_show;
mod support_settings_scan;
pub(crate) use library_scan::{NestedDiscoveryGateway, bootstrap_nested_discovery_app};
use support_acquisition_downloads::*;
#[cfg(unix)]
pub(crate) use support_bootstrap_fixtures::bootstrap_application_upgrade;
use support_bootstrap_fixtures::*;
#[cfg_attr(not(unix), allow(unused_imports))]
pub(crate) use support_bootstrap_fixtures::{
    bootstrap, bootstrap_import_tracking_spy, bootstrap_with_title_repo,
};
pub(crate) use support_catalog::MockTitleRepo;
use support_catalog::*;
pub(crate) use support_events_requests::MockDomainEventRepo;
use support_events_requests::*;
pub(crate) use support_imports::MockMediaFileRepo;
use support_imports::*;
use support_indexers_metadata::*;
use support_library_show::*;
pub(crate) use support_library_show::{replace_library_roots, with_stored_root_ids};
use support_settings_scan::*;

/// An isolated writable acquisition ledger for fixtures that record grabs.
pub(crate) fn in_memory_acquisition_scope_states() -> Arc<dyn AcquisitionScopeStateRepository> {
    Arc::new(TrackingAcquisitionScopeStateRepo::default())
}

/// Answer a movie `titles` request the way SMG answers it for TVDB-backed
/// movies, from a test double's TVDB-keyed bulk answer. A ref without a TVDB
/// id, or one the double has no movie for, is reported missing.
async fn movie_titles_from_tvdb_bulk<G: MetadataGateway + ?Sized>(
    gateway: &G,
    refs: &[MovieTitleRef],
    language: &str,
) -> AppResult<MovieTitleBulkResult> {
    let tvdb_ids = refs
        .iter()
        .filter_map(|movie_ref| movie_ref.tvdb_id)
        .collect::<Vec<_>>();
    let bulk = if tvdb_ids.is_empty() {
        BulkMetadataResult::default()
    } else {
        gateway.get_metadata_bulk(&tvdb_ids, &[], language).await?
    };
    let mut result = MovieTitleBulkResult::default();
    for (index, movie_ref) in refs.iter().enumerate() {
        match movie_ref
            .tvdb_id
            .and_then(|tvdb_id| bulk.movies.get(&tvdb_id))
        {
            Some(movie) => {
                result.by_ref_index.insert(index, movie.clone());
            }
            None => result.missing_ref_indexes.push(index),
        }
    }
    Ok(result)
}

/// Answer a movie `searchTitlesBatch` request with a test double's TVDB batch
/// answers. Other kinds are not served.
async fn movie_title_batch_from_tvdb<G: MetadataGateway + ?Sized>(
    gateway: &G,
    queries: &[MetadataSearchQuery],
    kind: &str,
    language: &str,
) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
    if kind != "movie" {
        return Err(AppError::Repository(
            "metadata gateway searchTitlesBatch is not implemented".into(),
        ));
    }
    gateway.search_tvdb_batch(queries, language).await
}

/// Location operations run in the background by contract (FR-030): a story
/// test watches the operation row until it reaches a terminal state, the way
/// Activity does. Shared by every location story fixture.
async fn settle_location_operation(
    app: &AppUseCase,
    operation_id: &str,
) -> crate::location::model::LocationOperation {
    wait_for(
        "the location operation to reach a terminal state",
        || async {
            let operation = app
                .location_operation(operation_id)
                .await
                .expect("read operation")
                .expect("operation row exists");
            operation.state.is_terminal().then_some(operation)
        },
    )
    .await
}

#[derive(Default)]
pub(super) struct RecordingScopeIndexerCoverageRepo {
    rows: Mutex<Vec<(String, String, String, String)>>,
    /// How many times `list_coverage_for_scope_keys` was called.
    list_calls: AtomicUsize,
}

impl RecordingScopeIndexerCoverageRepo {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) async fn recorded(&self) -> Vec<(String, String, String, String)> {
        self.rows.lock().await.clone()
    }

    pub(super) fn list_calls(&self) -> usize {
        self.list_calls.load(Ordering::SeqCst)
    }

    pub(super) async fn indexers_for_scope(&self, scope_key: &str) -> Vec<String> {
        self.rows
            .lock()
            .await
            .iter()
            .filter(|(recorded_scope_key, _, _, _)| recorded_scope_key == scope_key)
            .map(|(_, _, indexer_id, _)| indexer_id.clone())
            .collect()
    }
}

#[async_trait]
impl ScopeIndexerCoverageRepository for RecordingScopeIndexerCoverageRepo {
    async fn record_coverage(
        &self,
        scope_key: &str,
        facet: &str,
        indexer_id: &str,
        fingerprint: &str,
    ) -> AppResult<()> {
        let mut rows = self.rows.lock().await;
        rows.retain(|(sk, f, id, _)| !(sk == scope_key && f == facet && id == indexer_id));
        rows.push((
            scope_key.to_string(),
            facet.to_string(),
            indexer_id.to_string(),
            fingerprint.to_string(),
        ));
        Ok(())
    }

    async fn covered_indexers(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        _stale_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<Vec<String>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(sk, f, _, fp)| sk == scope_key && f == facet && fp == fingerprint)
            .map(|(_, _, indexer_id, _)| indexer_id.clone())
            .collect())
    }

    async fn prune_scope(&self, scope_key: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .retain(|(sk, _, _, _)| sk != scope_key);
        Ok(())
    }

    async fn prune_scope_indexer(&self, scope_key: &str, indexer_id: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .retain(|(sk, _, id, _)| sk != scope_key || id != indexer_id);
        Ok(())
    }

    async fn list_coverage_for_scope_keys(
        &self,
        scope_keys: &[String],
    ) -> AppResult<Vec<ScopeCoverageRow>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let wanted: HashSet<&str> = scope_keys.iter().map(String::as_str).collect();
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(scope_key, _, _, _)| wanted.contains(scope_key.as_str()))
            .map(|(scope_key, _, indexer_id, fingerprint)| ScopeCoverageRow {
                scope_key: scope_key.clone(),
                indexer_id: indexer_id.clone(),
                fingerprint: fingerprint.clone(),
                searched_at: chrono::Utc::now().to_rfc3339(),
            })
            .collect())
    }
}

/// A `match`-form fixture source with `heads`, each firing when `match` holds.
pub(crate) fn heads_source(
    source: impl AsRef<str>,
    heads: &[scryer_rules::lifecycle_actions::LifecycleActionHead],
) -> String {
    append_heads_to_match_source(source.as_ref(), heads)
}

/// [`heads_source`] for a fixture that reads a season or an episode: its heads
/// act at that level and the facts it reads as the subject's are read from that
/// level's document.
pub(crate) fn heads_source_at(
    source: impl AsRef<str>,
    heads: &[scryer_rules::lifecycle_actions::LifecycleActionHead],
    scope: scryer_domain::LifecycleRuleSubjectKind,
) -> String {
    use scryer_rules::lifecycle::LifecycleSubjectKind as Level;
    let (level, path) = match scope {
        scryer_domain::LifecycleRuleSubjectKind::Title => return heads_source(source, heads),
        scryer_domain::LifecycleRuleSubjectKind::Season => (Level::Season, "season"),
        scryer_domain::LifecycleRuleSubjectKind::Episode => (Level::Episode, "episode"),
    };
    let source = source
        .as_ref()
        .replace("input.facts.", &format!("input.{path}.facts."))
        .replace(
            "input.observations.",
            &format!("input.{path}.observations."),
        );
    narrow_heads(&heads_source(&source, heads), level)
}

/// Rename a title-level source's bare heads to their heads at `level`.
fn narrow_heads(source: &str, level: scryer_rules::lifecycle::LifecycleSubjectKind) -> String {
    use scryer_rules::lifecycle::LifecycleSubjectKind as Level;
    let mut narrowed = source.to_string();
    for head in scryer_rules::lifecycle_actions::LifecycleActionHead::ALL {
        if head.level() != Level::Title {
            continue;
        }
        if let Some(at) = head.at_level(level) {
            let bare = format!("\n{} if", head.rule_name());
            narrowed = narrowed.replace(&bare, &format!("\n{} if", at.rule_name()));
        }
    }
    narrowed
}

/// A title-level source whose heads act on episodes instead.
pub(crate) fn episode_heads(source: &str) -> String {
    narrow_heads(
        source,
        scryer_rules::lifecycle::LifecycleSubjectKind::Episode,
    )
}

/// A `match`-form fixture source with the heads a step list means. A profile id is used
/// as its name, the way the shared bootstrap names its test profiles.
pub(crate) fn steps_source(
    source: impl AsRef<str>,
    steps: &[crate::lifecycle_rules::LifecycleActionStep],
) -> String {
    use crate::lifecycle_rules::{LifecycleActionStepKind as Kind, LifecycleActionStepParameters};
    let mut actions = scryer_rules::lifecycle_actions::LifecycleActions::default();
    for step in steps {
        match step.kind {
            Kind::Unmonitor => actions.unmonitor = true,
            Kind::DeleteFiles => actions.delete_files = true,
            Kind::DeleteTitleAndFiles => actions.delete_title = true,
            Kind::Search => actions.search = true,
            Kind::AddTags => actions
                .add_tags
                .extend(step.parameters.tag_labels().iter().cloned()),
            Kind::RemoveTags => actions
                .remove_tags
                .extend(step.parameters.tag_labels().iter().cloned()),
            Kind::ChangeQualityProfile => {
                let LifecycleActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id,
                } = &step.parameters
                else {
                    panic!("profile step without a profile");
                };
                // The shared bootstrap names each test profile after its id.
                actions.set_quality_profile = Some(target_quality_profile_id.clone());
            }
            Kind::Monitor => actions.monitor = true,
            Kind::FetchSubtitles => actions.fetch_subtitles = true,
            Kind::MoveToLibrary | Kind::MoveToRoot => match &step.parameters {
                LifecycleActionStepParameters::Library { library } => {
                    actions.move_to_library = Some(library.clone());
                }
                LifecycleActionStepParameters::Root { root } => {
                    actions.move_to_root = Some(root.clone());
                }
                _ => panic!("move step without a target"),
            },
        }
    }
    append_actions_to_match_source(source.as_ref(), &actions)
}

/// Append one head per action to a `match` fixture source, each firing when
/// `match` holds, in the fixed execution order.
pub(crate) fn append_heads_to_match_source(
    source: &str,
    heads: &[scryer_rules::lifecycle_actions::LifecycleActionHead],
) -> String {
    use scryer_rules::lifecycle_actions::LifecycleActionHead as Head;
    let actions = scryer_rules::lifecycle_actions::LifecycleActions {
        unmonitor: heads.contains(&Head::Unmonitor),
        delete_files: heads.contains(&Head::DeleteFiles),
        delete_title: heads.contains(&Head::DeleteTitle),
        search: heads.contains(&Head::Search),
        ..Default::default()
    };
    append_actions_to_match_source(source, &actions)
}

fn append_actions_to_match_source(
    source: &str,
    actions: &scryer_rules::lifecycle_actions::LifecycleActions,
) -> String {
    fn rego_string(value: &str) -> String {
        serde_json::to_string(value).expect("a string always serializes")
    }
    fn rego_strings(values: &[String]) -> String {
        serde_json::to_string(values).expect("strings always serialize")
    }
    let mut heads = Vec::new();
    if !actions.add_tags.is_empty() {
        heads.push(format!("add_tags := {}", rego_strings(&actions.add_tags)));
    }
    if !actions.remove_tags.is_empty() {
        heads.push(format!(
            "remove_tags := {}",
            rego_strings(&actions.remove_tags)
        ));
    }
    if let Some(profile) = actions.set_quality_profile.as_deref() {
        heads.push(format!("set_quality_profile := {}", rego_string(profile)));
    }
    if actions.monitor {
        heads.push("monitor".to_string());
    }
    if actions.search {
        heads.push("search".to_string());
    }
    if actions.fetch_subtitles {
        heads.push("fetch_subtitles".to_string());
    }
    if let Some(library) = actions.move_to_library.as_deref() {
        heads.push(format!("move_to_library := {}", rego_string(library)));
    }
    if let Some(root) = actions.move_to_root.as_deref() {
        heads.push(format!("move_to_root := {}", rego_string(root)));
    }
    if let Some(message) = actions.notify.as_deref() {
        heads.push(format!("notify := {}", rego_string(message)));
    }
    if actions.unmonitor && !actions.delete_files {
        heads.push("unmonitor".to_string());
    }
    if actions.delete_files {
        heads.push("delete_files".to_string());
    }
    if actions.delete_title {
        heads.push("delete_title".to_string());
    }
    let mut converted = source.trim_end().to_string();
    converted.push('\n');
    for head in heads {
        converted.push_str(&format!("\n{head} if {{\n\tmatch\n}}\n"));
    }
    converted
}

/// The deletion runs among a candidate history's action runs. A sequence
/// also writes one history row per completed sequence; deletion assertions
/// look only at the rows a file delete wrote.
pub(crate) fn deletion_runs(
    runs: Vec<scryer_domain::LifecycleActionRun>,
) -> Vec<scryer_domain::LifecycleActionRun> {
    runs.into_iter()
        .filter(|run| {
            serde_json::from_str::<serde_json::Value>(&run.detail)
                .is_ok_and(|detail| detail.get("scoped_deletion").is_some())
        })
        .collect()
}
