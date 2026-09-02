use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use async_graphql::{Context, ID, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::external_import::{
    self, ArrDownloadClient, ArrEpisode, ArrIndexer, ArrMovie, ArrSeries, DetectedProwlarrIndexer,
    ExternalArrClient,
};
use scryer_application::{
    ANIME_MONITOR_SPECIALS_KEY, AppError, CHOWN_GROUP_KEY, ExternalIdHint, ExternalIdProvider,
    ExternalImportArrSourceKind as AppArrSourceKind, ExternalImportArrSourceSeriesEntry,
    ExternalImportArrSourceWarmupResult, ExternalImportLibrarySettingsAutoApplyDraft,
    ExternalImportMonitorEpisodeEntry, ExternalImportMonitorMovieEntry,
    ExternalImportMonitorSeasonEntry, ExternalImportMonitorSeriesEntry,
    ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorWarmupPhase, ExternalImportMonitorWarmupProgressSnapshot,
    ExternalImportMonitorWarmupStatus, ExternalImportProwlarrWarmupResult,
    ExternalImportSetupInstanceApiKeyDraft,
    ExternalImportSetupSecretDraftInput as AppExternalImportSetupSecretDraftInput,
    ExternalImportSetupSecretDraftSaveResult, ExternalImportSetupSecretInstanceKind,
    ExternalImportSetupSecretOverrideDraft, FOLDER_CHMOD_KEY, IndexerConfigUpdate, LibraryScanHint,
    LibraryScanHintFacet, LibraryScanHintSet, LibraryScanHintSource, NFO_WRITE_ON_IMPORT_ANIME_KEY,
    NFO_WRITE_ON_IMPORT_MOVIE_KEY, NFO_WRITE_ON_IMPORT_SERIES_KEY,
    PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY, PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
    QUALITY_PROFILE_ID_KEY, RENAME_ENABLED_KEY, REQUEST_QUALITY_PROFILE_IDS_KEY,
    SET_PERMISSIONS_LINUX_KEY, library_scan_file_full_path_key, library_scan_file_leaf_key,
    library_scan_folder_full_path_key, library_scan_folder_leaf_key,
};
use scryer_domain::{AppPermission, MediaFacet, NewDownloadClientConfig, NewIndexerConfig};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use scryer_interface_core::{
    actor_from_ctx, app_from_ctx, require_app_permission, require_config_app_permission,
    to_gql_error,
};
use scryer_interface_media::mappers::from_external_import_monitor_warmup_progress;
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct ExternalImportMutations;

const SONARR_EPISODE_FETCH_CONCURRENCY_PER_INSTANCE: usize = 16;
const SONARR_ACTIVE_EPISODE_INSTANCE_CONCURRENCY: usize = 2;
const SNAPSHOT_CHUNK_FLUSH_BYTES: usize = 4 * 1024 * 1024;
const SOURCE_CHUNK_READ_BATCH_SIZE: i32 = 32;

static SONARR_ACTIVE_EPISODE_INSTANCE_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn sonarr_active_episode_instance_semaphore() -> Arc<Semaphore> {
    SONARR_ACTIVE_EPISODE_INSTANCE_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(SONARR_ACTIVE_EPISODE_INSTANCE_CONCURRENCY)))
        .clone()
}

#[derive(Clone)]
struct ExternalArrImportSource {
    kind: AppArrSourceKind,
    source_key: String,
    base_url: String,
    api_key: String,
}

struct SnapshotChunkWriter {
    app: scryer_application::AppUseCase,
    actor: scryer_domain::User,
    session_id: String,
    facet: MediaFacet,
    entry_kind: ExternalImportMonitorSnapshotEntryKind,
    chunk_index: i32,
    buffered_ndjson: String,
}

impl SnapshotChunkWriter {
    fn new(
        app: scryer_application::AppUseCase,
        actor: scryer_domain::User,
        session_id: String,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
    ) -> Self {
        Self {
            app,
            actor,
            session_id,
            facet,
            entry_kind,
            chunk_index: 0,
            buffered_ndjson: String::new(),
        }
    }

    async fn push<T: Serialize>(&mut self, value: &T) -> scryer_application::AppResult<()> {
        let line = serde_json::to_string(value).map_err(|err| {
            AppError::Repository(format!("failed to serialize snapshot entry: {err}"))
        })?;
        self.buffered_ndjson.push_str(&line);
        self.buffered_ndjson.push('\n');

        if self.buffered_ndjson.len() >= SNAPSHOT_CHUNK_FLUSH_BYTES {
            self.flush().await?;
        }

        Ok(())
    }

    async fn flush(&mut self) -> scryer_application::AppResult<()> {
        if self.buffered_ndjson.is_empty() {
            return Ok(());
        }

        let payload_ndjson = std::mem::take(&mut self.buffered_ndjson);
        let chunk = ExternalImportMonitorSnapshotChunk {
            session_id: self.session_id.clone(),
            facet: self.facet.clone(),
            entry_kind: self.entry_kind.clone(),
            chunk_index: self.chunk_index,
            payload_ndjson,
            created_at: Utc::now().to_rfc3339(),
        };
        self.app
            .append_external_import_monitor_snapshot_chunk(&self.actor, chunk)
            .await?;

        self.chunk_index += 1;
        Ok(())
    }

    async fn finish(&mut self) -> scryer_application::AppResult<()> {
        self.flush().await?;
        Ok(())
    }
}

async fn process_external_import_source_chunk_entries<T, F>(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
    facet: MediaFacet,
    entry_kind: ExternalImportMonitorSnapshotEntryKind,
    mut process_entry: F,
) -> scryer_application::AppResult<()>
where
    T: DeserializeOwned,
    F: FnMut(T) -> scryer_application::AppResult<()>,
{
    let mut after_chunk_index = None;

    loop {
        let chunks = app
            .list_external_import_monitor_snapshot_chunks_for_session(
                actor,
                session_id,
                facet.clone(),
                entry_kind.clone(),
                after_chunk_index,
                SOURCE_CHUNK_READ_BATCH_SIZE,
            )
            .await?;
        if chunks.is_empty() {
            break;
        }

        for chunk in chunks {
            after_chunk_index = Some(chunk.chunk_index);
            for line in chunk
                .payload_ndjson
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let entry = serde_json::from_str::<T>(line).map_err(|err| {
                    AppError::Repository(format!(
                        "failed to parse external import source chunk entry: {err}"
                    ))
                })?;
                process_entry(entry)?;
            }
        }
    }

    Ok(())
}

async fn clear_external_import_monitor_apply_target(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    facet: MediaFacet,
) -> scryer_application::AppResult<()> {
    app.clear_external_import_monitor_snapshot_chunks(actor, facet)
        .await
}

async fn clear_external_import_monitor_apply_targets(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
) -> scryer_application::AppResult<()> {
    for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
        clear_external_import_monitor_apply_target(app, actor, facet).await?;
    }
    Ok(())
}

fn app_arr_source_kind(kind: ExternalArrSourceKind) -> AppArrSourceKind {
    match kind {
        ExternalArrSourceKind::Sonarr => AppArrSourceKind::Sonarr,
        ExternalArrSourceKind::Radarr => AppArrSourceKind::Radarr,
    }
}

fn gql_arr_source_kind(kind: AppArrSourceKind) -> ExternalArrSourceKind {
    match kind {
        AppArrSourceKind::Sonarr => ExternalArrSourceKind::Sonarr,
        AppArrSourceKind::Radarr => ExternalArrSourceKind::Radarr,
    }
}

fn app_secret_instance_kind(
    kind: ExternalImportConnectionKind,
) -> ExternalImportSetupSecretInstanceKind {
    match kind {
        ExternalImportConnectionKind::Sonarr => ExternalImportSetupSecretInstanceKind::Sonarr,
        ExternalImportConnectionKind::Radarr => ExternalImportSetupSecretInstanceKind::Radarr,
        ExternalImportConnectionKind::Prowlarr => ExternalImportSetupSecretInstanceKind::Prowlarr,
    }
}

fn secret_override_from_api_key_input(
    input: DownloadClientApiKeyOverrideInput,
) -> ExternalImportSetupSecretOverrideDraft {
    ExternalImportSetupSecretOverrideDraft {
        dedup_key: input.dedup_key,
        secret: input.api_key,
    }
}

fn secret_override_from_password_input(
    input: DownloadClientPasswordOverrideInput,
) -> ExternalImportSetupSecretOverrideDraft {
    ExternalImportSetupSecretOverrideDraft {
        dedup_key: input.dedup_key,
        secret: input.password,
    }
}

fn secret_override_from_indexer_api_key_input(
    input: IndexerApiKeyOverrideInput,
) -> ExternalImportSetupSecretOverrideDraft {
    ExternalImportSetupSecretOverrideDraft {
        dedup_key: input.dedup_key,
        secret: input.api_key,
    }
}

fn external_import_setup_secret_draft_input_from_gql(
    input: SaveExternalImportSetupSecretDraftInput,
) -> AppExternalImportSetupSecretDraftInput {
    AppExternalImportSetupSecretDraftInput {
        instance_api_keys: input
            .instance_api_keys
            .into_iter()
            .map(|entry| ExternalImportSetupInstanceApiKeyDraft {
                instance_id: entry.instance_id.to_string(),
                kind: app_secret_instance_kind(entry.kind),
                api_key: entry.api_key,
            })
            .collect(),
        download_client_api_key_overrides: input
            .download_client_api_key_overrides
            .into_iter()
            .map(secret_override_from_api_key_input)
            .collect(),
        download_client_password_overrides: input
            .download_client_password_overrides
            .into_iter()
            .map(secret_override_from_password_input)
            .collect(),
        indexer_api_key_overrides: input
            .indexer_api_key_overrides
            .into_iter()
            .map(secret_override_from_indexer_api_key_input)
            .collect(),
    }
}

fn save_external_import_setup_secret_draft_payload(
    result: ExternalImportSetupSecretDraftSaveResult,
) -> SaveExternalImportSetupSecretDraftPayload {
    let _ = result.saved;
    SaveExternalImportSetupSecretDraftPayload {
        overwrote_another_user_draft: result.overwrote_another_user_draft,
        updated_at: result.updated_at,
    }
}

fn normalized_external_arr_source_key(
    kind: AppArrSourceKind,
    base_url: &str,
) -> scryer_application::AppResult<String> {
    let trimmed = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(AppError::Validation("Arr base URL is required".into()));
    }
    Ok(format!("{}:{trimmed}", kind.as_str()))
}

fn source_from_input(
    kind: ExternalArrSourceKind,
    connection: ExternalImportConnectionInput,
) -> scryer_application::AppResult<ExternalArrImportSource> {
    let kind = app_arr_source_kind(kind);
    let source_key = normalized_external_arr_source_key(kind, &connection.base_url)?;
    Ok(ExternalArrImportSource {
        kind,
        source_key,
        base_url: connection.base_url,
        api_key: connection.api_key,
    })
}

fn client_for_arr_source(
    source: &ExternalArrImportSource,
) -> scryer_application::AppResult<ExternalArrClient> {
    match source.kind {
        AppArrSourceKind::Sonarr => {
            ExternalArrClient::for_sonarr_v4(source.base_url.clone(), source.api_key.clone())
        }
        AppArrSourceKind::Radarr => {
            ExternalArrClient::for_radarr_v6(source.base_url.clone(), source.api_key.clone())
        }
    }
}

fn source_connection_fingerprint(source: &ExternalArrImportSource) -> String {
    format!(
        "{}|{}|{}",
        source.kind.as_str(),
        source
            .base_url
            .trim()
            .trim_end_matches('/')
            .to_ascii_lowercase(),
        source.api_key.trim()
    )
}

fn prowlarr_connection_fingerprint(base_url: &str, api_key: &str) -> String {
    format!(
        "{}|{}",
        base_url.trim().trim_end_matches('/').to_ascii_lowercase(),
        api_key.trim()
    )
}

fn gql_warmup_status(
    status: ExternalImportMonitorWarmupStatus,
) -> ExternalImportMonitorWarmupStatusValue {
    match status {
        ExternalImportMonitorWarmupStatus::Queued => ExternalImportMonitorWarmupStatusValue::Queued,
        ExternalImportMonitorWarmupStatus::Running => {
            ExternalImportMonitorWarmupStatusValue::Running
        }
        ExternalImportMonitorWarmupStatus::Completed => {
            ExternalImportMonitorWarmupStatusValue::Completed
        }
        ExternalImportMonitorWarmupStatus::Canceled => {
            ExternalImportMonitorWarmupStatusValue::Canceled
        }
        ExternalImportMonitorWarmupStatus::Failed => ExternalImportMonitorWarmupStatusValue::Failed,
    }
}

fn normalize_import_path_key(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn remap_import_path(path: Option<String>, arr_root: &str, scryer_root: &str) -> Option<String> {
    let path = path?;
    let trimmed_path = path.trim();
    let arr_root_trimmed = arr_root.trim().trim_end_matches('/').trim_end_matches('\\');
    let scryer_root_trimmed = scryer_root
        .trim()
        .trim_end_matches('/')
        .trim_end_matches('\\');
    if trimmed_path.is_empty() || arr_root_trimmed.is_empty() || scryer_root_trimmed.is_empty() {
        return Some(path);
    }
    let path_key = normalize_import_path_key(trimmed_path);
    let root_key = normalize_import_path_key(arr_root_trimmed);
    if path_key == root_key {
        return Some(scryer_root_trimmed.to_string());
    }
    let slash_prefix = format!("{root_key}/");
    let backslash_prefix = format!("{root_key}\\");
    let suffix = if path_key.starts_with(&slash_prefix) || path_key.starts_with(&backslash_prefix) {
        trimmed_path.get(arr_root_trimmed.len() + 1..)
    } else {
        None
    };
    let Some(suffix) = suffix else {
        return Some(path);
    };
    let separator = if scryer_root_trimmed.contains('\\') && !scryer_root_trimmed.contains('/') {
        "\\"
    } else {
        "/"
    };
    Some(format!("{scryer_root_trimmed}{separator}{suffix}"))
}

#[derive(Clone)]
struct ResolvedSourceMapping {
    library_id: String,
    source_warmup_session_id: Option<String>,
    arr_root_path: String,
    scryer_root_path: String,
    facet: MediaFacet,
}

fn mapping_key(session_id: &str, source_key: &str, arr_root_path: &str) -> String {
    format!(
        "{}|{}|{}",
        session_id,
        source_key,
        normalize_import_path_key(arr_root_path)
    )
}

/// Dedup key for a manually-added root (one no warmup surfaced). Keyed by the
/// target library + normalized Scryer-host path, since manual roots have no
/// source session / source key to disambiguate them.
fn manual_mapping_key(library_id: &str, scryer_root_key: &str) -> String {
    format!("manual|{library_id}|{scryer_root_key}")
}

fn source_mapping_root_paths(source: &ExternalImportArrSourceWarmupResult) -> Vec<String> {
    let mut roots = BTreeMap::<String, String>::new();
    for root in &source.root_folders {
        let key = normalize_import_path_key(&root.path);
        if !key.is_empty() {
            roots.entry(key).or_insert_with(|| root.path.clone());
        }
    }
    for root_path in &source.title_root_paths {
        let key = normalize_import_path_key(root_path);
        if !key.is_empty() {
            roots.entry(key).or_insert_with(|| root_path.clone());
        }
    }
    roots.into_values().collect()
}

const EXTERNAL_IMPORT_DERIVED_SETTING_MIN_SAMPLE: i32 = 3;
const EXTERNAL_IMPORT_DERIVED_SETTING_CONFIDENCE_NUMERATOR: i32 = 85;
const EXTERNAL_IMPORT_DERIVED_SETTING_CONFIDENCE_DENOMINATOR: i32 = 100;

#[derive(Clone)]
struct ExternalImportSourceSettingSignal {
    source_key: String,
    source_kind: AppArrSourceKind,
    title_count: i32,
    rename_enabled: Option<bool>,
    rename_template: Option<String>,
    folder_template: Option<String>,
    nfo_write_on_import: Option<bool>,
    plexmatch_write_on_import: Option<bool>,
    set_permissions_linux: Option<bool>,
    folder_chmod: Option<String>,
    chown_group: Option<String>,
}

struct ExternalImportLibrarySettingAccumulator {
    library_id: String,
    facet: MediaFacet,
    sources: BTreeMap<String, ExternalImportSourceSettingSignal>,
    quality_profile_counts: BTreeMap<(String, i64), i32>,
    quality_profile_total: i32,
    monitor_specials_true: i32,
    monitor_specials_total: i32,
}

impl ExternalImportLibrarySettingAccumulator {
    fn new(library_id: String, facet: MediaFacet) -> Self {
        Self {
            library_id,
            facet,
            sources: BTreeMap::new(),
            quality_profile_counts: BTreeMap::new(),
            quality_profile_total: 0,
            monitor_specials_true: 0,
            monitor_specials_total: 0,
        }
    }
}

fn build_external_import_library_setting_accumulators(
    source_results: &BTreeMap<String, ExternalImportArrSourceWarmupResult>,
    mappings: &HashMap<String, ResolvedSourceMapping>,
) -> BTreeMap<String, ExternalImportLibrarySettingAccumulator> {
    let mut accumulators = BTreeMap::<String, ExternalImportLibrarySettingAccumulator>::new();

    for mapping in mappings.values() {
        let Some(session_id) = mapping.source_warmup_session_id.as_deref() else {
            continue;
        };
        let Some(source_result) = source_results.get(session_id) else {
            continue;
        };
        let accumulator = accumulators
            .entry(mapping.library_id.clone())
            .or_insert_with(|| {
                ExternalImportLibrarySettingAccumulator::new(
                    mapping.library_id.clone(),
                    mapping.facet.clone(),
                )
            });
        accumulator
            .sources
            .entry(session_id.to_string())
            .or_insert_with(|| external_import_source_setting_signal(source_result));
    }

    accumulators
}

fn external_import_source_setting_signal(
    source_result: &ExternalImportArrSourceWarmupResult,
) -> ExternalImportSourceSettingSignal {
    ExternalImportSourceSettingSignal {
        source_key: source_result.source_key.clone(),
        source_kind: source_result.kind,
        title_count: 0,
        rename_enabled: source_result
            .naming_config
            .as_ref()
            .and_then(|config| config.rename_enabled),
        rename_template: source_result
            .naming_config
            .as_ref()
            .and_then(|config| config.standard_format.clone()),
        folder_template: source_result
            .naming_config
            .as_ref()
            .and_then(|config| config.folder_format.clone()),
        nfo_write_on_import: source_result_nfo_write_signal(source_result),
        plexmatch_write_on_import: source_result_plexmatch_write_signal(source_result),
        set_permissions_linux: source_result
            .media_management_config
            .as_ref()
            .and_then(|config| config.set_permissions_linux),
        folder_chmod: source_result
            .media_management_config
            .as_ref()
            .and_then(|config| config.chmod_folder.clone()),
        chown_group: source_result
            .media_management_config
            .as_ref()
            .and_then(|config| config.chown_group.clone()),
    }
}

fn source_result_nfo_write_signal(
    source_result: &ExternalImportArrSourceWarmupResult,
) -> Option<bool> {
    if source_result.metadata_providers.is_empty() {
        return None;
    }
    Some(source_result.metadata_providers.iter().any(|provider| {
        if !provider.enable {
            return false;
        }
        let implementation = provider.implementation.to_ascii_lowercase();
        if !implementation.contains("xbmc") && !implementation.contains("kodi") {
            return false;
        }
        match source_result.kind {
            AppArrSourceKind::Radarr => {
                external_import::field_bool(&provider.fields, "movieMetadata").unwrap_or(false)
                    || external_import::field_bool(&provider.fields, "useMovieNfo").unwrap_or(false)
            }
            AppArrSourceKind::Sonarr => {
                external_import::field_bool(&provider.fields, "seriesMetadata").unwrap_or(false)
                    || external_import::field_bool(&provider.fields, "episodeMetadata")
                        .unwrap_or(false)
            }
        }
    }))
}

fn source_result_plexmatch_write_signal(
    source_result: &ExternalImportArrSourceWarmupResult,
) -> Option<bool> {
    if source_result.metadata_providers.is_empty() {
        return None;
    }
    Some(source_result.metadata_providers.iter().any(|provider| {
        provider.enable
            && provider
                .implementation
                .to_ascii_lowercase()
                .contains("plex")
    }))
}

fn record_movie_setting_sample(
    accumulators: &mut BTreeMap<String, ExternalImportLibrarySettingAccumulator>,
    mapping: &ResolvedSourceMapping,
    movie: &ArrMovie,
) {
    let Some(session_id) = mapping.source_warmup_session_id.as_deref() else {
        return;
    };
    let Some(accumulator) = accumulators.get_mut(&mapping.library_id) else {
        return;
    };
    if let Some(source) = accumulator.sources.get_mut(session_id) {
        source.title_count = source.title_count.saturating_add(1);
    }
    if let Some(profile_id) = movie.quality_profile_id {
        accumulator.quality_profile_total = accumulator.quality_profile_total.saturating_add(1);
        *accumulator
            .quality_profile_counts
            .entry((session_id.to_string(), profile_id))
            .or_insert(0) += 1;
    }
}

fn record_series_setting_sample(
    accumulators: &mut BTreeMap<String, ExternalImportLibrarySettingAccumulator>,
    mapping: &ResolvedSourceMapping,
    series: &ArrSeries,
) {
    let Some(session_id) = mapping.source_warmup_session_id.as_deref() else {
        return;
    };
    let Some(accumulator) = accumulators.get_mut(&mapping.library_id) else {
        return;
    };
    if let Some(source) = accumulator.sources.get_mut(session_id) {
        source.title_count = source.title_count.saturating_add(1);
    }
    if let Some(profile_id) = series.quality_profile_id {
        accumulator.quality_profile_total = accumulator.quality_profile_total.saturating_add(1);
        *accumulator
            .quality_profile_counts
            .entry((session_id.to_string(), profile_id))
            .or_insert(0) += 1;
    }
    if mapping.facet == MediaFacet::Anime
        && let Some(season_zero) = series
            .seasons
            .iter()
            .find(|season| season.season_number == 0)
    {
        accumulator.monitor_specials_total = accumulator.monitor_specials_total.saturating_add(1);
        if season_zero.monitored {
            accumulator.monitor_specials_true = accumulator.monitor_specials_true.saturating_add(1);
        }
    }
}

fn derive_external_import_library_setting_applications(
    accumulators: &BTreeMap<String, ExternalImportLibrarySettingAccumulator>,
    source_results: &BTreeMap<String, ExternalImportArrSourceWarmupResult>,
    catalog_profiles: &[scryer_application::QualityProfile],
) -> Vec<ExternalImportLibrarySettingApplicationPayload> {
    let mut applications = Vec::new();
    for accumulator in accumulators.values() {
        push_bool_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::RenameEnabled,
            |source| source.rename_enabled,
            true,
            true,
        );
        push_string_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::RenameTemplate,
            |source| source.rename_template.as_deref(),
            false,
        );
        push_string_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::FolderTemplate,
            |source| source.folder_template.as_deref(),
            false,
        );
        push_bool_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::NfoWriteOnImport,
            |source| source.nfo_write_on_import,
            true,
            false,
        );
        if accumulator.facet != MediaFacet::Movie {
            push_bool_consensus_application(
                &mut applications,
                accumulator,
                ExternalImportLibrarySettingKey::PlexmatchWriteOnImport,
                |source| source.plexmatch_write_on_import,
                true,
                false,
            );
        }
        push_bool_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::SetPermissionsLinux,
            |source| source.set_permissions_linux,
            true,
            true,
        );
        push_string_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::FolderChmod,
            |source| source.folder_chmod.as_deref(),
            true,
        );
        push_string_consensus_application(
            &mut applications,
            accumulator,
            ExternalImportLibrarySettingKey::ChownGroup,
            |source| source.chown_group.as_deref(),
            true,
        );
        push_quality_profile_application(
            &mut applications,
            accumulator,
            source_results,
            catalog_profiles,
        );
        push_monitor_specials_application(&mut applications, accumulator);
    }
    applications
}

fn push_bool_consensus_application<F>(
    applications: &mut Vec<ExternalImportLibrarySettingApplicationPayload>,
    accumulator: &ExternalImportLibrarySettingAccumulator,
    setting: ExternalImportLibrarySettingKey,
    value_for_source: F,
    auto_apply: bool,
    include_false: bool,
) where
    F: Fn(&ExternalImportSourceSettingSignal) -> Option<bool>,
{
    let sources = accumulator.sources.values().collect::<Vec<_>>();
    let values = sources
        .iter()
        .copied()
        .filter_map(|source| value_for_source(source).map(|value| (source, value)))
        .collect::<Vec<_>>();
    if values.is_empty() {
        if !sources.is_empty() {
            applications.push(setting_application(
                accumulator,
                setting,
                empty_setting_value(),
                ExternalImportLibrarySettingConfidence::Low,
                ExternalImportLibrarySettingDisposition::Skipped,
                source_evidence(sources.iter().copied()),
                Some("one or more contributing sources did not report this setting".to_string()),
            ));
        }
        return;
    }
    if values.len() != sources.len() {
        applications.push(setting_application(
            accumulator,
            setting,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            source_evidence(sources.iter().copied()),
            Some("one or more contributing sources did not report this setting".to_string()),
        ));
        return;
    }
    let first = values[0].1;
    if values.iter().all(|(_, value)| *value == first) {
        if first || include_false {
            applications.push(setting_application(
                accumulator,
                setting,
                bool_setting_value(first),
                ExternalImportLibrarySettingConfidence::High,
                if auto_apply {
                    ExternalImportLibrarySettingDisposition::AutoApplied
                } else {
                    ExternalImportLibrarySettingDisposition::Suggested
                },
                source_evidence(values.iter().map(|(source, _)| *source)),
                None,
            ));
        }
    } else {
        applications.push(setting_application(
            accumulator,
            setting,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            source_evidence(values.iter().map(|(source, _)| *source)),
            Some("contributing sources disagree".to_string()),
        ));
    }
}

fn push_string_consensus_application<F>(
    applications: &mut Vec<ExternalImportLibrarySettingApplicationPayload>,
    accumulator: &ExternalImportLibrarySettingAccumulator,
    setting: ExternalImportLibrarySettingKey,
    value_for_source: F,
    auto_apply: bool,
) where
    F: Fn(&ExternalImportSourceSettingSignal) -> Option<&str>,
{
    let sources = accumulator.sources.values().collect::<Vec<_>>();
    let values = sources
        .iter()
        .copied()
        .filter_map(|source| {
            value_for_source(source)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| (source, value))
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        if !sources.is_empty() {
            applications.push(setting_application(
                accumulator,
                setting,
                empty_setting_value(),
                ExternalImportLibrarySettingConfidence::Low,
                ExternalImportLibrarySettingDisposition::Skipped,
                source_evidence(sources.iter().copied()),
                Some("one or more contributing sources did not report this setting".to_string()),
            ));
        }
        return;
    }
    if values.len() != sources.len() {
        applications.push(setting_application(
            accumulator,
            setting,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            source_evidence(sources.iter().copied()),
            Some("one or more contributing sources did not report this setting".to_string()),
        ));
        return;
    }
    let first = values[0].1;
    if values.iter().all(|(_, value)| *value == first) {
        applications.push(setting_application(
            accumulator,
            setting,
            string_setting_value(first.to_string()),
            ExternalImportLibrarySettingConfidence::High,
            if auto_apply {
                ExternalImportLibrarySettingDisposition::AutoApplied
            } else {
                ExternalImportLibrarySettingDisposition::Suggested
            },
            source_evidence(values.iter().map(|(source, _)| *source)),
            None,
        ));
    } else {
        applications.push(setting_application(
            accumulator,
            setting,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            source_evidence(values.iter().map(|(source, _)| *source)),
            Some("contributing sources disagree".to_string()),
        ));
    }
}

fn push_quality_profile_application(
    applications: &mut Vec<ExternalImportLibrarySettingApplicationPayload>,
    accumulator: &ExternalImportLibrarySettingAccumulator,
    source_results: &BTreeMap<String, ExternalImportArrSourceWarmupResult>,
    catalog_profiles: &[scryer_application::QualityProfile],
) {
    if accumulator.quality_profile_total < EXTERNAL_IMPORT_DERIVED_SETTING_MIN_SAMPLE {
        return;
    }

    let mut mapped_counts = BTreeMap::<String, i32>::new();
    for ((session_id, arr_profile_id), count) in &accumulator.quality_profile_counts {
        let Some(source_result) = source_results.get(session_id) else {
            continue;
        };
        let Some(arr_profile) = source_result
            .quality_profiles
            .iter()
            .find(|profile| profile.id == *arr_profile_id)
        else {
            continue;
        };
        let Some(profile_id) = resolve_arr_quality_profile_id(arr_profile, catalog_profiles) else {
            continue;
        };
        *mapped_counts.entry(profile_id).or_insert(0) += *count;
    }

    if mapped_counts.is_empty() {
        applications.push(setting_application(
            accumulator,
            ExternalImportLibrarySettingKey::QualityProfileId,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            title_count_evidence(accumulator, 0, accumulator.quality_profile_total),
            Some("no Arr quality profile mapped unambiguously to Scryer".to_string()),
        ));
        return;
    }

    let Some((profile_id, count)) = mapped_counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| left.0.cmp(right.0)))
    else {
        return;
    };
    if !meets_dominance_threshold(*count, accumulator.quality_profile_total) {
        applications.push(setting_application(
            accumulator,
            ExternalImportLibrarySettingKey::QualityProfileId,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            title_count_evidence(accumulator, *count, accumulator.quality_profile_total),
            Some("dominant mapped Arr quality profile is below confidence threshold".to_string()),
        ));
        return;
    }

    let profile_id = profile_id.clone();
    let evidence = title_count_evidence(accumulator, *count, accumulator.quality_profile_total);
    applications.push(setting_application(
        accumulator,
        ExternalImportLibrarySettingKey::QualityProfileId,
        string_setting_value(profile_id.clone()),
        ExternalImportLibrarySettingConfidence::High,
        ExternalImportLibrarySettingDisposition::AutoApplied,
        evidence.clone(),
        None,
    ));
    applications.push(setting_application(
        accumulator,
        ExternalImportLibrarySettingKey::RequestQualityProfileIds,
        string_list_setting_value(vec![profile_id]),
        ExternalImportLibrarySettingConfidence::High,
        ExternalImportLibrarySettingDisposition::AutoApplied,
        evidence,
        None,
    ));
}

fn push_monitor_specials_application(
    applications: &mut Vec<ExternalImportLibrarySettingApplicationPayload>,
    accumulator: &ExternalImportLibrarySettingAccumulator,
) {
    if accumulator.facet != MediaFacet::Anime
        || accumulator.monitor_specials_total < EXTERNAL_IMPORT_DERIVED_SETTING_MIN_SAMPLE
    {
        return;
    }

    let true_count = accumulator.monitor_specials_true;
    let false_count = accumulator
        .monitor_specials_total
        .saturating_sub(accumulator.monitor_specials_true);
    let (value, matching_count) = if true_count >= false_count {
        (true, true_count)
    } else {
        (false, false_count)
    };
    if !meets_dominance_threshold(matching_count, accumulator.monitor_specials_total) {
        applications.push(setting_application(
            accumulator,
            ExternalImportLibrarySettingKey::MonitorSpecials,
            empty_setting_value(),
            ExternalImportLibrarySettingConfidence::Low,
            ExternalImportLibrarySettingDisposition::Skipped,
            title_count_evidence(
                accumulator,
                matching_count,
                accumulator.monitor_specials_total,
            ),
            Some("season zero monitoring is not dominant enough".to_string()),
        ));
        return;
    }

    applications.push(setting_application(
        accumulator,
        ExternalImportLibrarySettingKey::MonitorSpecials,
        bool_setting_value(value),
        ExternalImportLibrarySettingConfidence::High,
        ExternalImportLibrarySettingDisposition::AutoApplied,
        title_count_evidence(
            accumulator,
            matching_count,
            accumulator.monitor_specials_total,
        ),
        None,
    ));
}

fn setting_application(
    accumulator: &ExternalImportLibrarySettingAccumulator,
    setting: ExternalImportLibrarySettingKey,
    value: ExternalImportLibrarySettingValuePayload,
    confidence: ExternalImportLibrarySettingConfidence,
    disposition: ExternalImportLibrarySettingDisposition,
    evidence: Vec<ExternalImportLibrarySettingEvidencePayload>,
    reason: Option<String>,
) -> ExternalImportLibrarySettingApplicationPayload {
    ExternalImportLibrarySettingApplicationPayload {
        library_id: ID::from(accumulator.library_id.clone()),
        facet: MediaFacetValue::from_domain(accumulator.facet.clone()),
        setting,
        value,
        confidence,
        disposition,
        evidence,
        reason,
    }
}

fn source_evidence<'a>(
    sources: impl Iterator<Item = &'a ExternalImportSourceSettingSignal>,
) -> Vec<ExternalImportLibrarySettingEvidencePayload> {
    sources
        .map(|source| {
            let count = source.title_count.max(1);
            ExternalImportLibrarySettingEvidencePayload {
                source_key: source.source_key.clone(),
                source_kind: gql_arr_source_kind(source.source_kind),
                matching_count: count,
                total_count: count,
                detail: None,
            }
        })
        .collect()
}

fn title_count_evidence(
    accumulator: &ExternalImportLibrarySettingAccumulator,
    matching_count: i32,
    total_count: i32,
) -> Vec<ExternalImportLibrarySettingEvidencePayload> {
    accumulator
        .sources
        .values()
        .map(|source| ExternalImportLibrarySettingEvidencePayload {
            source_key: source.source_key.clone(),
            source_kind: gql_arr_source_kind(source.source_kind),
            matching_count,
            total_count,
            detail: None,
        })
        .collect()
}

fn empty_setting_value() -> ExternalImportLibrarySettingValuePayload {
    ExternalImportLibrarySettingValuePayload {
        bool_value: None,
        string_value: None,
        string_list_value: None,
    }
}

fn bool_setting_value(value: bool) -> ExternalImportLibrarySettingValuePayload {
    ExternalImportLibrarySettingValuePayload {
        bool_value: Some(value),
        string_value: None,
        string_list_value: None,
    }
}

fn string_setting_value(value: String) -> ExternalImportLibrarySettingValuePayload {
    ExternalImportLibrarySettingValuePayload {
        bool_value: None,
        string_value: Some(value),
        string_list_value: None,
    }
}

fn string_list_setting_value(values: Vec<String>) -> ExternalImportLibrarySettingValuePayload {
    ExternalImportLibrarySettingValuePayload {
        bool_value: None,
        string_value: None,
        string_list_value: Some(values),
    }
}

fn meets_dominance_threshold(matching_count: i32, total_count: i32) -> bool {
    matching_count.saturating_mul(EXTERNAL_IMPORT_DERIVED_SETTING_CONFIDENCE_DENOMINATOR)
        >= total_count.saturating_mul(EXTERNAL_IMPORT_DERIVED_SETTING_CONFIDENCE_NUMERATOR)
}

fn resolve_arr_quality_profile_id(
    arr_profile: &scryer_application::external_import::ArrQualityProfile,
    catalog_profiles: &[scryer_application::QualityProfile],
) -> Option<String> {
    let arr_id_key = normalize_quality_profile_match_key(&arr_profile.id.to_string());
    let arr_name_key = normalize_quality_profile_match_key(&arr_profile.name);
    let mut matches = catalog_profiles
        .iter()
        .filter(|profile| {
            let id_key = normalize_quality_profile_match_key(&profile.id);
            let name_key = normalize_quality_profile_match_key(&profile.name);
            id_key == arr_id_key
                || id_key == arr_name_key
                || name_key == arr_id_key
                || name_key == arr_name_key
        })
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn normalize_quality_profile_match_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

async fn apply_external_import_library_setting_applications(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    applications: &mut [ExternalImportLibrarySettingApplicationPayload],
) -> scryer_application::AppResult<()> {
    let mut library_drafts = BTreeMap::<String, ExternalImportLibrarySettingsAutoApplyDraft>::new();
    let mut rename_by_facet = BTreeMap::<String, (MediaFacet, bool)>::new();
    let mut rename_conflict_facets = HashSet::<String>::new();

    for application in applications.iter() {
        if application.disposition != ExternalImportLibrarySettingDisposition::AutoApplied {
            continue;
        }
        let library_id = application.library_id.to_string();
        match application.setting {
            ExternalImportLibrarySettingKey::RenameEnabled => {
                let Some(value) = application.value.bool_value else {
                    continue;
                };
                let facet = application.facet.into_domain();
                let facet_key = facet.as_str().to_string();
                match rename_by_facet.get(&facet_key) {
                    Some((_, existing)) if *existing != value => {
                        rename_conflict_facets.insert(facet_key);
                    }
                    Some(_) => {}
                    None => {
                        rename_by_facet.insert(facet_key, (facet, value));
                    }
                }
            }
            ExternalImportLibrarySettingKey::NfoWriteOnImport => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .nfo_write_on_import = application.value.bool_value;
            }
            ExternalImportLibrarySettingKey::PlexmatchWriteOnImport => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .plexmatch_write_on_import = application.value.bool_value;
            }
            ExternalImportLibrarySettingKey::SetPermissionsLinux => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .set_permissions_linux = application.value.bool_value;
            }
            ExternalImportLibrarySettingKey::FolderChmod => {
                library_drafts.entry(library_id).or_default().folder_chmod =
                    application.value.string_value.clone();
            }
            ExternalImportLibrarySettingKey::ChownGroup => {
                library_drafts.entry(library_id).or_default().chown_group =
                    application.value.string_value.clone();
            }
            ExternalImportLibrarySettingKey::QualityProfileId => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .quality_profile_id = application.value.string_value.clone();
            }
            ExternalImportLibrarySettingKey::RequestQualityProfileIds => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .request_quality_profile_ids = application.value.string_list_value.clone();
            }
            ExternalImportLibrarySettingKey::MonitorSpecials => {
                library_drafts
                    .entry(library_id)
                    .or_default()
                    .monitor_specials = application.value.bool_value;
            }
            ExternalImportLibrarySettingKey::RenameTemplate
            | ExternalImportLibrarySettingKey::FolderTemplate
            | ExternalImportLibrarySettingKey::RequiredAudioLanguages => {}
        }
    }

    for conflict_facet in rename_conflict_facets {
        for application in applications.iter_mut().filter(|application| {
            application.setting == ExternalImportLibrarySettingKey::RenameEnabled
                && application.facet.as_scope_id() == conflict_facet
                && application.disposition == ExternalImportLibrarySettingDisposition::AutoApplied
        }) {
            application.disposition = ExternalImportLibrarySettingDisposition::Skipped;
            application.reason =
                Some("facet-level rename signals disagree across selected libraries".to_string());
        }
        rename_by_facet.remove(&conflict_facet);
    }

    for (_, (facet, value)) in rename_by_facet {
        let changed_keys = app
            .apply_external_import_media_settings_auto_apply(actor, facet.clone(), Some(value))
            .await?;
        let changed = changed_keys.iter().any(|key| key == RENAME_ENABLED_KEY);
        for application in applications.iter_mut().filter(|application| {
            application.setting == ExternalImportLibrarySettingKey::RenameEnabled
                && application.facet == MediaFacetValue::from_domain(facet.clone())
                && application.disposition == ExternalImportLibrarySettingDisposition::AutoApplied
        }) {
            if !changed {
                application.disposition = ExternalImportLibrarySettingDisposition::Skipped;
                application.reason =
                    Some("target setting already has an explicit override".to_string());
            }
        }
    }

    for (library_id, draft) in library_drafts {
        let result = app
            .apply_external_import_library_settings_auto_apply(actor, &library_id, draft)
            .await?;
        let changed_keys = result.changed_keys;
        let skipped_reasons = result
            .skipped_keys
            .into_iter()
            .map(|skipped| (skipped.key_name, skipped.reason))
            .collect::<BTreeMap<_, _>>();
        for application in applications.iter_mut().filter(|application| {
            application.library_id.to_string() == library_id
                && application.disposition == ExternalImportLibrarySettingDisposition::AutoApplied
                && is_external_import_library_auto_apply_setting(application.setting)
        }) {
            let Some(key_name) = external_import_application_setting_key_name(
                application.setting,
                application.facet.into_domain(),
            ) else {
                continue;
            };
            if !changed_keys.iter().any(|key| key == key_name) {
                application.disposition = ExternalImportLibrarySettingDisposition::Skipped;
                application.reason =
                    Some(skipped_reasons.get(key_name).cloned().unwrap_or_else(|| {
                        "target setting already has an explicit override".to_string()
                    }));
            }
        }
    }

    Ok(())
}

fn is_external_import_library_auto_apply_setting(setting: ExternalImportLibrarySettingKey) -> bool {
    matches!(
        setting,
        ExternalImportLibrarySettingKey::NfoWriteOnImport
            | ExternalImportLibrarySettingKey::PlexmatchWriteOnImport
            | ExternalImportLibrarySettingKey::SetPermissionsLinux
            | ExternalImportLibrarySettingKey::FolderChmod
            | ExternalImportLibrarySettingKey::ChownGroup
            | ExternalImportLibrarySettingKey::QualityProfileId
            | ExternalImportLibrarySettingKey::RequestQualityProfileIds
            | ExternalImportLibrarySettingKey::MonitorSpecials
    )
}

fn external_import_application_setting_key_name(
    setting: ExternalImportLibrarySettingKey,
    facet: MediaFacet,
) -> Option<&'static str> {
    match setting {
        ExternalImportLibrarySettingKey::RenameEnabled => Some(RENAME_ENABLED_KEY),
        ExternalImportLibrarySettingKey::NfoWriteOnImport => Some(match facet {
            MediaFacet::Movie => NFO_WRITE_ON_IMPORT_MOVIE_KEY,
            MediaFacet::Series => NFO_WRITE_ON_IMPORT_SERIES_KEY,
            MediaFacet::Anime => NFO_WRITE_ON_IMPORT_ANIME_KEY,
        }),
        ExternalImportLibrarySettingKey::PlexmatchWriteOnImport => match facet {
            MediaFacet::Movie => None,
            MediaFacet::Series => Some(PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY),
            MediaFacet::Anime => Some(PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY),
        },
        ExternalImportLibrarySettingKey::SetPermissionsLinux => Some(SET_PERMISSIONS_LINUX_KEY),
        ExternalImportLibrarySettingKey::FolderChmod => Some(FOLDER_CHMOD_KEY),
        ExternalImportLibrarySettingKey::ChownGroup => Some(CHOWN_GROUP_KEY),
        ExternalImportLibrarySettingKey::QualityProfileId => Some(QUALITY_PROFILE_ID_KEY),
        ExternalImportLibrarySettingKey::RequestQualityProfileIds => {
            Some(REQUEST_QUALITY_PROFILE_IDS_KEY)
        }
        ExternalImportLibrarySettingKey::MonitorSpecials => Some(ANIME_MONITOR_SPECIALS_KEY),
        ExternalImportLibrarySettingKey::RenameTemplate
        | ExternalImportLibrarySettingKey::FolderTemplate
        | ExternalImportLibrarySettingKey::RequiredAudioLanguages => None,
    }
}

fn movie_monitor_merge_key(entry: &ExternalImportMonitorMovieEntry) -> String {
    entry
        .tmdb_id
        .as_deref()
        .map(|value| format!("tmdb:{value}"))
        .or_else(|| {
            entry
                .imdb_id
                .as_deref()
                .map(|value| format!("imdb:{value}"))
        })
        .or_else(|| entry.path.as_deref().map(|value| format!("path:{value}")))
        .unwrap_or_else(|| "unknown".to_string())
}

fn movie_monitor_merge_key_for_source(
    entry: &ExternalImportMonitorMovieEntry,
    source_key: &str,
    arr_movie_id: i64,
) -> String {
    let key = movie_monitor_merge_key(entry);
    if key == "unknown" {
        format!("source:{source_key}:movie:{arr_movie_id}")
    } else {
        key
    }
}

fn series_monitor_merge_key(entry: &ExternalImportMonitorSeriesEntry) -> String {
    entry
        .tvdb_id
        .as_deref()
        .map(|value| format!("tvdb:{value}"))
        .or_else(|| entry.path.as_deref().map(|value| format!("path:{value}")))
        .unwrap_or_else(|| "unknown".to_string())
}

fn series_monitor_merge_key_for_source(
    facet: &MediaFacet,
    entry: &ExternalImportMonitorSeriesEntry,
    source_key: &str,
    arr_series_id: i64,
) -> String {
    let key = series_monitor_merge_key(entry);
    if key == "unknown" {
        format!(
            "facet:{}:source:{source_key}:series:{arr_series_id}",
            facet.as_str()
        )
    } else {
        format!("facet:{}:{key}", facet.as_str())
    }
}

fn merge_series_monitor_entry(
    existing: &mut ExternalImportMonitorSeriesEntry,
    incoming: ExternalImportMonitorSeriesEntry,
) {
    existing.monitored |= incoming.monitored;
    let mut seasons = existing
        .seasons
        .iter()
        .map(|season| (season.season_number, season.monitored))
        .collect::<HashMap<_, _>>();
    for season in incoming.seasons {
        seasons
            .entry(season.season_number)
            .and_modify(|monitored| *monitored |= season.monitored)
            .or_insert(season.monitored);
    }
    let mut season_entries = seasons
        .into_iter()
        .map(
            |(season_number, monitored)| ExternalImportMonitorSeasonEntry {
                season_number,
                monitored,
            },
        )
        .collect::<Vec<_>>();
    season_entries.sort_by_key(|season| season.season_number);
    existing.seasons = season_entries;

    let mut episodes = existing
        .episodes
        .iter()
        .map(|episode| {
            (
                episode
                    .tvdb_id
                    .as_deref()
                    .map(|value| format!("tvdb:{value}"))
                    .unwrap_or_else(|| {
                        format!(
                            "number:{}:{}",
                            episode.season_number, episode.episode_number
                        )
                    }),
                episode.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    for episode in incoming.episodes {
        let key = episode
            .tvdb_id
            .as_deref()
            .map(|value| format!("tvdb:{value}"))
            .unwrap_or_else(|| {
                format!(
                    "number:{}:{}",
                    episode.season_number, episode.episode_number
                )
            });
        episodes
            .entry(key)
            .and_modify(|existing| existing.monitored |= episode.monitored)
            .or_insert(episode);
    }
    let mut episode_entries = episodes.into_values().collect::<Vec<_>>();
    episode_entries.sort_by_key(|episode| {
        (
            episode.season_number,
            episode.episode_number,
            episode.tvdb_id.clone(),
        )
    });
    existing.episodes = episode_entries;
}

#[derive(Debug, Clone)]
struct ProwlarrImportGroup {
    base_url: String,
    sources: Vec<String>,
    child_names: Vec<String>,
    api_key: Option<String>,
    api_key_conflict: bool,
    /// The key came from an operator-verified direct connection (Connect step
    /// or warmup session), not from an arr-reported field. Direct keys are
    /// authoritative: arr-side keys can neither replace nor conflict them,
    /// regardless of merge order.
    has_direct_api_key: bool,
}

impl ProwlarrImportGroup {
    fn new(detected: DetectedProwlarrIndexer, source: &str) -> Self {
        let mut group = Self {
            base_url: detected.base_url.clone(),
            sources: Vec::new(),
            child_names: Vec::new(),
            api_key: None,
            api_key_conflict: false,
            has_direct_api_key: false,
        };
        group.merge(detected, source);
        group
    }

    fn merge(&mut self, detected: DetectedProwlarrIndexer, source: &str) {
        push_unique(&mut self.sources, source.to_string());
        push_unique(&mut self.child_names, detected.child_name);
        if self.has_direct_api_key {
            return;
        }
        if let Some(api_key) = detected.api_key {
            match self.api_key.as_deref() {
                Some(existing) if existing != api_key => {
                    self.api_key = None;
                    self.api_key_conflict = true;
                }
                None if !self.api_key_conflict => {
                    self.api_key = Some(api_key);
                }
                _ => {}
            }
        }
    }

    fn requires_api_key_override(&self) -> bool {
        self.api_key_conflict || self.api_key.is_none()
    }

    fn dedup_key(&self) -> String {
        prowlarr_dedup_key(&self.base_url)
    }

    fn to_payload(&self) -> ExternalImportIndexerPayload {
        ExternalImportIndexerPayload {
            source_keys: self.sources.clone(),
            name: prowlarr_display_name(&self.base_url),
            implementation: "Prowlarr".to_string(),
            scryer_provider_type: Some("prowlarr".to_string()),
            base_url: Some(self.base_url.clone()),
            api_key_present: self.api_key.is_some(),
            dedup_key: self.dedup_key(),
            supported: true,
            child_count: i32::try_from(self.child_names.len()).unwrap_or(i32::MAX),
            child_names: self.child_names.clone(),
            requires_api_key_override: self.requires_api_key_override(),
            api_key_help_url: prowlarr_api_key_help_url(&self.base_url),
        }
    }
}

fn merge_direct_prowlarr_group(
    groups: &mut HashMap<String, ProwlarrImportGroup>,
    base_url: &str,
    api_key: &str,
    child_names: &[String],
) {
    let normalized_base_url = base_url.trim().trim_end_matches('/').to_string();
    let dedup_key = prowlarr_dedup_key(&normalized_base_url);
    let group = groups
        .entry(dedup_key)
        .or_insert_with(|| ProwlarrImportGroup {
            base_url: normalized_base_url.clone(),
            sources: Vec::new(),
            child_names: Vec::new(),
            api_key: None,
            api_key_conflict: false,
            has_direct_api_key: false,
        });

    push_unique(&mut group.sources, "prowlarr".to_string());
    for child_name in child_names {
        push_unique(&mut group.child_names, child_name.clone());
    }
    group.api_key_conflict = false;
    group.api_key = Some(api_key.trim().to_string());
    group.has_direct_api_key = true;
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn prowlarr_dedup_key(base_url: &str) -> String {
    format!("prowlarr:{}", base_url.trim().trim_end_matches('/'))
}

fn prowlarr_display_name(base_url: &str) -> String {
    let host = url::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| base_url.trim().trim_end_matches('/').to_string());
    format!("Prowlarr ({host})")
}

fn prowlarr_api_key_help_url(base_url: &str) -> Option<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    url::Url::parse(normalized).ok()?;
    Some(format!("{normalized}/settings/general"))
}

fn prowlarr_parent_config_json(base_url: &str, api_key: &str) -> String {
    serde_json::json!({
        "base_url": base_url.trim().trim_end_matches('/'),
        "api_key": api_key.trim(),
    })
    .to_string()
}

fn indexer_config_base_url(config_json: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(config_json?).ok()?;
    value
        .get("base_url")
        .or_else(|| value.get("baseUrl"))?
        .as_str()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

fn merge_prowlarr_group(
    groups: &mut HashMap<String, ProwlarrImportGroup>,
    detected: DetectedProwlarrIndexer,
    source: &str,
) {
    let dedup_key = prowlarr_dedup_key(&detected.base_url);
    if let Some(group) = groups.get_mut(&dedup_key) {
        group.merge(detected, source);
    } else {
        groups.insert(dedup_key, ProwlarrImportGroup::new(detected, source));
    }
}

fn detect_imported_prowlarr_proxy_indexer(
    indexer: &ArrIndexer,
    linked_prowlarr_base_url: Option<&str>,
) -> Option<DetectedProwlarrIndexer> {
    if let Some(linked_prowlarr_base_url) = linked_prowlarr_base_url {
        external_import::detect_linked_prowlarr_proxy_indexer(indexer, linked_prowlarr_base_url)
    } else {
        external_import::detect_prowlarr_proxy_indexer(indexer)
    }
}

fn version_from_validation_result(
    result: &scryer_application::IndexerValidationResult,
) -> Option<String> {
    let message = result.message.as_deref()?.trim();
    message
        .strip_prefix("Connected to Prowlarr ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn same_base_url(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

fn imported_indexer_config_json(
    fields: &[scryer_domain::ConfigFieldDef],
    base_url: &str,
    api_key: Option<&str>,
    api_path: Option<&str>,
) -> String {
    let mut object = serde_json::Map::new();
    if let Some(connection_field) = fields
        .iter()
        .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))
        && !base_url.trim().is_empty()
    {
        object.insert(
            connection_field.key.clone(),
            serde_json::Value::String(base_url.trim().to_string()),
        );
    }
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty())
        && let Some(api_key_field) = fields.iter().find(|field| {
            field.key == "api_key"
                || (field.field_type == scryer_domain::ConfigFieldType::Password
                    && field.key.to_ascii_lowercase().contains("api"))
        })
    {
        object.insert(
            api_key_field.key.clone(),
            serde_json::Value::String(api_key.to_string()),
        );
    }
    if let Some(api_path) = api_path.map(str::trim).filter(|value| !value.is_empty())
        && let Some(api_path_field) = fields.iter().find(|field| field.key == "api_path")
    {
        object.insert(
            api_path_field.key.clone(),
            serde_json::Value::String(api_path.to_string()),
        );
    }

    serde_json::Value::Object(object).to_string()
}

#[Object]
impl ExternalImportMutations {
    /// Save the caller's external-import secret draft and report whether another draft was replaced.
    async fn save_external_import_setup_secret_draft(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Instance secrets and masked provider credentials keyed by their import deduplication keys."
        )]
        input: SaveExternalImportSetupSecretDraftInput,
    ) -> GqlResult<SaveExternalImportSetupSecretDraftPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.save_external_import_setup_secret_draft(
            &actor,
            external_import_setup_secret_draft_input_from_gql(input),
        )
        .await
        .map(save_external_import_setup_secret_draft_payload)
        .map_err(to_gql_error)
    }

    /// Clear the caller's external-import secret draft; clearing an absent draft is harmless.
    async fn clear_external_import_setup_secret_draft(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ClearExternalImportSetupSecretDraftPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.clear_external_import_setup_secret_draft(&actor)
            .await
            .map(|cleared| ClearExternalImportSetupSecretDraftPayload { cleared })
            .map_err(to_gql_error)
    }

    /// Start or reuse a background Sonarr or Radarr warmup and return its current progress snapshot.
    async fn start_external_import_arr_source_warmup(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Arr source kind and connection credentials used to identify the warmup session."
        )]
        input: StartExternalImportArrSourceWarmupInput,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        maintain_external_import_source_sessions(&app, &actor).await?;
        let source = source_from_input(input.kind, input.connection).map_err(to_gql_error)?;
        let fingerprint = format!("arr-source={}", source_connection_fingerprint(&source));
        let begin = app
            .begin_external_import_monitor_warmup(&actor, &fingerprint)
            .await?;

        if let Some(replaced_session_id) = begin.replaced_session_id.as_deref() {
            let _ =
                clear_external_import_arr_source_snapshot_chunks(&app, &actor, replaced_session_id)
                    .await;
        }

        if begin.created {
            let session_id = begin.snapshot.session_id.clone();
            app.set_external_import_arr_source_warmup_result(
                &session_id,
                ExternalImportArrSourceWarmupResult {
                    source_key: source.source_key.clone(),
                    kind: source.kind,
                    base_url: source.base_url.clone(),
                    version: None,
                    root_folders: Vec::new(),
                    title_root_paths: Vec::new(),
                    naming_config: None,
                    media_management_config: None,
                    metadata_providers: Vec::new(),
                    quality_profiles: Vec::new(),
                    signal_warnings: Vec::new(),
                    download_clients: Vec::new(),
                    indexers: Vec::new(),
                },
            )
            .await;
            let app_for_task = app.clone();
            let actor_for_task = actor.clone();
            let snapshot_for_task = begin.snapshot.clone();
            tokio::spawn(async move {
                run_external_import_arr_source_warmup_job(
                    app_for_task,
                    actor_for_task,
                    session_id,
                    source,
                    begin.cancel_token,
                    snapshot_for_task,
                )
                .await;
            });
        }

        Ok(from_external_import_monitor_warmup_progress(begin.snapshot))
    }

    /// Start or reuse a background Prowlarr warmup and return its current progress snapshot.
    async fn start_external_import_prowlarr_warmup(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Prowlarr connection credentials used to identify the warmup session.")]
        input: StartExternalImportProwlarrWarmupInput,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        maintain_external_import_source_sessions(&app, &actor).await?;

        let base_url = input
            .connection
            .base_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        let api_key = input.connection.api_key.trim().to_string();
        let fingerprint = format!(
            "prowlarr-source={}",
            prowlarr_connection_fingerprint(&base_url, &api_key)
        );
        let mut begin = app
            .begin_external_import_monitor_warmup(&actor, &fingerprint)
            .await?;

        if begin.created {
            begin.snapshot.phase = ExternalImportMonitorWarmupPhase::LoadingIndexers;
            let session_id = begin.snapshot.session_id.clone();
            publish_warmup_progress(&app, &session_id, &mut begin.snapshot).await;
            let app_for_task = app.clone();
            let actor_for_task = actor.clone();
            let snapshot_for_task = begin.snapshot.clone();
            let cancel_token = begin.cancel_token.clone();
            tokio::spawn(async move {
                run_external_import_prowlarr_warmup_job(
                    app_for_task,
                    actor_for_task,
                    session_id,
                    base_url,
                    api_key,
                    cancel_token,
                    snapshot_for_task,
                )
                .await;
            });
        }

        Ok(from_external_import_monitor_warmup_progress(begin.snapshot))
    }

    /// Request cancellation of an Arr source warmup and report whether cancellation was accepted.
    async fn cancel_external_import_arr_source_warmup(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "Warmup session identity to cancel.")] session_id: ID,
    ) -> GqlResult<CancelExternalImportMonitorWarmupPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        maintain_external_import_source_sessions(&app, &actor).await?;
        let session_id_string = session_id.to_string();
        let canceled = app
            .cancel_external_import_monitor_warmup(&actor, &session_id_string)
            .await?;
        if canceled {
            let _ =
                clear_external_import_arr_source_snapshot_chunks(&app, &actor, &session_id_string)
                    .await;
        }
        Ok(CancelExternalImportMonitorWarmupPayload {
            session_id,
            canceled,
        })
    }

    /// Test one Sonarr, Radarr, or Prowlarr connection without starting a full warmup.
    async fn validate_external_import_connection(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "External application kind and connection credentials to validate.")]
        input: ValidateExternalImportConnectionInput,
    ) -> GqlResult<ExternalImportConnectionValidationPayload> {
        let kind = input.kind;
        // Prowlarr validation uses only the provider's lightweight connection
        // check; child discovery belongs to the detached warmup job.
        let required_permission = match kind {
            ExternalImportConnectionKind::Prowlarr => AppPermission::ManageSystemSettings,
            ExternalImportConnectionKind::Sonarr | ExternalImportConnectionKind::Radarr => {
                AppPermission::ManageCatalogSettings
            }
        };
        require_app_permission(ctx, required_permission).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        let base_url = input.connection.base_url.clone();

        let outcome: scryer_application::AppResult<Option<String>> = match kind {
            ExternalImportConnectionKind::Sonarr | ExternalImportConnectionKind::Radarr => {
                let arr_kind = if matches!(kind, ExternalImportConnectionKind::Radarr) {
                    ExternalArrSourceKind::Radarr
                } else {
                    ExternalArrSourceKind::Sonarr
                };
                match source_from_input(arr_kind, input.connection)
                    .and_then(|source| client_for_arr_source(&source))
                {
                    Ok(client) => client
                        .test_connection()
                        .await
                        .map(|(_app_name, version)| Some(version)),
                    Err(err) => Err(err),
                }
            }
            ExternalImportConnectionKind::Prowlarr => {
                let config_json = prowlarr_parent_config_json(
                    &input.connection.base_url,
                    &input.connection.api_key,
                );
                app.test_indexer_connection(&actor, "prowlarr", Some(&config_json), None, None)
                    .await
                    .map(|()| None)
            }
        };

        Ok(match outcome {
            Ok(version) => ExternalImportConnectionValidationPayload {
                kind,
                base_url,
                connected: true,
                version,
                error: None,
            },
            Err(err) => ExternalImportConnectionValidationPayload {
                kind,
                base_url,
                connected: false,
                version: None,
                error: Some(err.to_string()),
            },
        })
    }

    /// Connect to Sonarr and/or Radarr, fetch their configs, return a preview.
    async fn preview_external_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Completed or running warmup sessions and optional direct Prowlarr credentials."
        )]
        input: PreviewExternalImportInput,
    ) -> GqlResult<ExternalImportPreviewPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        maintain_external_import_source_sessions(&app, &actor).await?;

        if input.prowlarr_warmup_session_id.is_some() && input.prowlarr.is_some() {
            return Err(to_gql_error(AppError::Validation(
                "prowlarrWarmupSessionId and deprecated prowlarr input cannot both be provided"
                    .to_string(),
            )));
        }

        if input.source_warmup_session_ids.is_empty()
            && input.prowlarr_warmup_session_id.is_none()
            && input.prowlarr.is_none()
        {
            return Err(to_gql_error(AppError::Validation(
                "at least one Arr source warmup session or Prowlarr warmup session must be provided"
                    .to_string(),
            )));
        }

        let mut payload = ExternalImportPreviewPayload {
            prowlarr_connected: false,
            prowlarr_version: None,
            prowlarr_error: None,
            arr_sources: Vec::new(),
            root_folders: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        };

        // Map from dedup_key → index in payload vecs, so duplicates merge sources.
        let mut dc_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut idx_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut prowlarr_groups: HashMap<String, ProwlarrImportGroup> = HashMap::new();
        let mut linked_prowlarr_base_url =
            input.prowlarr.as_ref().map(|conn| conn.base_url.clone());

        if let Some(session_id) = &input.prowlarr_warmup_session_id {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            match snapshot.status {
                ExternalImportMonitorWarmupStatus::Queued
                | ExternalImportMonitorWarmupStatus::Running => {
                    payload.prowlarr_connected = true;
                }
                ExternalImportMonitorWarmupStatus::Completed => {
                    let result = app
                        .external_import_prowlarr_warmup_result(&actor, &session_id_string)
                        .await
                        .map_err(to_gql_error)?;
                    payload.prowlarr_connected = true;
                    payload.prowlarr_version = result.version.clone();
                    linked_prowlarr_base_url = Some(result.base_url.clone());
                    let child_names = result
                        .plan
                        .children
                        .iter()
                        .map(|child| child.name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect::<Vec<_>>();
                    merge_direct_prowlarr_group(
                        &mut prowlarr_groups,
                        &result.base_url,
                        &result.api_key,
                        &child_names,
                    );
                }
                ExternalImportMonitorWarmupStatus::Failed => {
                    payload.prowlarr_error = snapshot
                        .error_message
                        .or_else(|| Some("Prowlarr discovery failed".to_string()));
                }
                ExternalImportMonitorWarmupStatus::Canceled => {
                    payload.prowlarr_error = Some("Prowlarr discovery was canceled".to_string());
                }
            }
        }

        if let Some(conn) = &input.prowlarr {
            let config_json = prowlarr_parent_config_json(&conn.base_url, &conn.api_key);
            match app
                .preview_managed_indexer_children(&actor, "prowlarr", Some(&config_json))
                .await
            {
                Ok((validation, plan)) => {
                    payload.prowlarr_connected = true;
                    payload.prowlarr_version = version_from_validation_result(&validation);
                    let child_names = plan
                        .children
                        .into_iter()
                        .map(|child| child.name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect::<Vec<_>>();
                    merge_direct_prowlarr_group(
                        &mut prowlarr_groups,
                        &conn.base_url,
                        &conn.api_key,
                        &child_names,
                    );
                }
                Err(error) => {
                    payload.prowlarr_error = Some(error.to_string());
                }
            }
        }

        for session_id in input.source_warmup_session_ids {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let result = app
                .external_import_arr_source_warmup_result(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let kind = gql_arr_source_kind(result.kind);
            payload.arr_sources.push(ExternalImportArrSourcePayload {
                session_id: session_id.clone(),
                source_key: result.source_key.clone(),
                kind,
                base_url: result.base_url.clone(),
                connected: result.version.is_some(),
                version: result.version.clone(),
                status: gql_warmup_status(snapshot.status),
                error: snapshot.error_message.clone(),
            });

            for arr_root_path in source_mapping_root_paths(&result) {
                payload.root_folders.push(ExternalImportRootFolderPayload {
                    source_warmup_session_id: session_id.clone(),
                    source_key: result.source_key.clone(),
                    kind,
                    arr_root_path,
                });
            }

            for dc in &result.download_clients {
                let mapped = map_download_client(dc, &result.source_key);
                if let Some(&existing) = dc_key_idx.get(&mapped.dedup_key) {
                    push_unique(
                        &mut payload.download_clients[existing].source_keys,
                        result.source_key.clone(),
                    );
                } else {
                    dc_key_idx.insert(mapped.dedup_key.clone(), payload.download_clients.len());
                    payload.download_clients.push(mapped);
                }
            }

            for idx in &result.indexers {
                if external_import::should_skip_imported_indexer(idx) {
                    continue;
                }
                if let Some(detected) =
                    detect_imported_prowlarr_proxy_indexer(idx, linked_prowlarr_base_url.as_deref())
                {
                    merge_prowlarr_group(&mut prowlarr_groups, detected, &result.source_key);
                    continue;
                }

                let mapped = map_indexer(idx, &result.source_key);
                if let Some(&existing) = idx_key_idx.get(&mapped.dedup_key) {
                    push_unique(
                        &mut payload.indexers[existing].source_keys,
                        result.source_key.clone(),
                    );
                } else {
                    idx_key_idx.insert(mapped.dedup_key.clone(), payload.indexers.len());
                    payload.indexers.push(mapped);
                }
            }
        }

        let mut prowlarr_payloads = prowlarr_groups
            .into_values()
            .map(|group| group.to_payload())
            .collect::<Vec<_>>();
        prowlarr_payloads.sort_by(|left, right| left.dedup_key.cmp(&right.dedup_key));
        payload.indexers.extend(prowlarr_payloads);

        Ok(payload)
    }

    /// Apply selected source mappings, imported monitoring state, scan hints, and safe settings.
    /// The operation consumes completed warmup sessions after successful reconciliation.
    async fn finalize_external_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Warmup sessions and validated library root mappings to reconcile into the catalog."
        )]
        input: FinalizeExternalImportInput,
    ) -> GqlResult<FinalizeExternalImportPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        maintain_external_import_source_sessions(&app, &actor).await?;

        let mut source_results = BTreeMap::<String, ExternalImportArrSourceWarmupResult>::new();
        for session_id in &input.source_warmup_session_ids {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await?;
            if snapshot.status != ExternalImportMonitorWarmupStatus::Completed {
                return Err(to_gql_error(AppError::Validation(format!(
                    "source warmup session {session_id_string} is not completed"
                ))));
            }
            let result = app
                .external_import_arr_source_warmup_result(&actor, &session_id_string)
                .await?;
            source_results.insert(session_id_string, result);
        }
        if source_results.is_empty() && input.mappings.is_empty() {
            return Err(to_gql_error(AppError::Validation(
                "at least one source warmup session or mapping is required".into(),
            )));
        }

        let mut source_order = source_results.keys().cloned().collect::<Vec<_>>();
        source_order.sort_by(|left, right| {
            let left_source = source_results
                .get(left)
                .map(|source| source.source_key.as_str())
                .unwrap_or_default();
            let right_source = source_results
                .get(right)
                .map(|source| source.source_key.as_str())
                .unwrap_or_default();
            left_source.cmp(right_source).then_with(|| left.cmp(right))
        });
        let source_roots = source_results
            .iter()
            .map(|(session_id, source)| (session_id.clone(), source_mapping_root_paths(source)))
            .collect::<HashMap<_, _>>();
        let mut mappings = HashMap::<String, ResolvedSourceMapping>::new();
        for mapping in input.mappings {
            let facet = mapping.facet.into_domain();
            let library_id = mapping.library_id.to_string();

            // Common to sourced and manual roots: the target library must exist,
            // its facet must match, and the Scryer-host path must be one of its
            // roots (finalize never silently invents a library root).
            let library = app.external_import_library(&actor, &library_id).await?;
            if library.facet != facet {
                return Err(to_gql_error(AppError::Validation(format!(
                    "library {library_id} has facet {}, not {}",
                    library.facet.as_str(),
                    facet.as_str()
                ))));
            }
            let scryer_root_key =
                scryer_domain::normalize_library_root_path(mapping.scryer_root_path.as_str());
            if !library.roots.iter().any(|root| {
                scryer_domain::normalize_library_root_path(&root.path) == scryer_root_key
            }) {
                return Err(to_gql_error(AppError::Validation(format!(
                    "root '{}' does not belong to library {library_id}",
                    mapping.scryer_root_path
                ))));
            }

            let dedup_key = match mapping.source_warmup_session_id.as_ref() {
                // ── Sourced root: discovered by a Sonarr/Radarr warmup. Must
                // identify a selected, warmed source root whose kind is
                // compatible with the chosen facet. ──
                Some(session_id_raw) => {
                    let session_id = session_id_raw.to_string();
                    let Some(source_result) = source_results.get(&session_id) else {
                        return Err(to_gql_error(AppError::Validation(format!(
                            "mapping references unselected source warmup session {session_id}"
                        ))));
                    };
                    let (Some(source_key), Some(kind_value)) =
                        (mapping.source_key.as_deref(), mapping.kind)
                    else {
                        return Err(to_gql_error(AppError::Validation(format!(
                            "sourced mapping for session {session_id} must include sourceKey and kind"
                        ))));
                    };
                    let kind = app_arr_source_kind(kind_value);
                    if kind != source_result.kind || source_key != source_result.source_key {
                        return Err(to_gql_error(AppError::Validation(format!(
                            "mapping for session {session_id} does not match warmed source"
                        ))));
                    }
                    let Some(known_roots) = source_roots.get(&session_id) else {
                        return Err(to_gql_error(AppError::Validation(format!(
                            "mapping references unselected source warmup session {session_id}"
                        ))));
                    };
                    let mapping_root_key = normalize_import_path_key(&mapping.arr_root_path);
                    if !known_roots
                        .iter()
                        .any(|root| normalize_import_path_key(root) == mapping_root_key)
                    {
                        return Err(to_gql_error(AppError::Validation(format!(
                            "mapping references unknown source root '{}'",
                            mapping.arr_root_path
                        ))));
                    }
                    match (kind, &facet) {
                        (AppArrSourceKind::Radarr, MediaFacet::Movie)
                        | (AppArrSourceKind::Sonarr, MediaFacet::Series)
                        | (AppArrSourceKind::Sonarr, MediaFacet::Anime) => {}
                        _ => {
                            return Err(to_gql_error(AppError::Validation(format!(
                                "mapping facet {} is not compatible with {}",
                                facet.as_str(),
                                kind.as_str()
                            ))));
                        }
                    }
                    mapping_key(&session_id, source_key, &mapping.arr_root_path)
                }
                // ── Manual root: no warmup surfaced it, so there is no
                // monitored-status snapshot to apply. It only registers its
                // Scryer-host path on the target library (validated above) and
                // is keyed by library + path for duplicate detection. ──
                None => manual_mapping_key(&library_id, &scryer_root_key),
            };

            if mappings
                .insert(
                    dedup_key,
                    ResolvedSourceMapping {
                        library_id,
                        source_warmup_session_id: mapping
                            .source_warmup_session_id
                            .as_ref()
                            .map(|session_id| session_id.to_string()),
                        arr_root_path: mapping.arr_root_path,
                        scryer_root_path: mapping.scryer_root_path,
                        facet,
                    },
                )
                .is_some()
            {
                return Err(to_gql_error(AppError::Validation(
                    "duplicate source root mapping".into(),
                )));
            }
        }

        for (session_id, source_result) in &source_results {
            for root_path in source_roots.get(session_id).into_iter().flatten() {
                let key = mapping_key(session_id, &source_result.source_key, root_path);
                if !mappings.contains_key(&key) {
                    return Err(to_gql_error(AppError::Validation(format!(
                        "missing mapping for source {} root '{}'",
                        source_result.source_key, root_path
                    ))));
                }
            }
        }

        let quality_profile_settings = app.get_quality_profile_settings(&actor).await?;
        let mut library_setting_accumulators =
            build_external_import_library_setting_accumulators(&source_results, &mappings);

        let _apply_guard = app.acquire_external_import_apply_guard().await;
        clear_external_import_monitor_apply_targets(&app, &actor).await?;
        let apply_session_id = scryer_application::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID;
        let mut scan_hints = LibraryScanHintSet::new();
        let mut movie_entries =
            BTreeMap::<(String, String), ExternalImportMonitorMovieEntry>::new();
        let mut series_entries =
            BTreeMap::<(String, String), (MediaFacet, ExternalImportMonitorSeriesEntry)>::new();

        for session_id in &source_order {
            let source_result = source_results
                .get(session_id)
                .expect("source order references loaded source");
            match source_result.kind {
                AppArrSourceKind::Radarr => {
                    process_external_import_source_chunk_entries::<ArrMovie, _>(
                        &app,
                        &actor,
                        session_id,
                        MediaFacet::Movie,
                        ExternalImportMonitorSnapshotEntryKind::Movie,
                        |movie| {
                            let key = mapping_key(
                                session_id,
                                &source_result.source_key,
                                &movie.root_folder_path,
                            );
                            let Some(mapping) = mappings.get(&key) else {
                                return Err(AppError::Validation(format!(
                                    "missing mapping for source {} root '{}'",
                                    source_result.source_key, movie.root_folder_path
                                )));
                            };
                            record_movie_setting_sample(
                                &mut library_setting_accumulators,
                                mapping,
                                &movie,
                            );
                            let mut remapped = movie.clone();
                            remapped.path = remap_import_path(
                                remapped.path,
                                &mapping.arr_root_path,
                                &mapping.scryer_root_path,
                            );
                            remapped.file_path = remap_import_path(
                                remapped.file_path,
                                &mapping.arr_root_path,
                                &mapping.scryer_root_path,
                            );
                            if let Some(hint) = movie_scan_hint_from_arr(&remapped) {
                                scan_hints.push(hint);
                            }
                            let entry = movie_monitor_entry_from_arr(&remapped);
                            let merge_key = movie_monitor_merge_key_for_source(
                                &entry,
                                &source_result.source_key,
                                movie.id,
                            );
                            movie_entries
                                .entry((mapping.library_id.clone(), merge_key))
                                .and_modify(|existing| existing.monitored |= entry.monitored)
                                .or_insert(entry);
                            Ok(())
                        },
                    )
                    .await
                    .map_err(to_gql_error)?;
                }
                AppArrSourceKind::Sonarr => {
                    process_external_import_source_chunk_entries::<
                        ExternalImportArrSourceSeriesEntry,
                        _,
                    >(
                        &app,
                        &actor,
                        session_id,
                        MediaFacet::Series,
                        ExternalImportMonitorSnapshotEntryKind::Series,
                        |series_entry| {
                            let key = mapping_key(
                                session_id,
                                &source_result.source_key,
                                &series_entry.series.root_folder_path,
                            );
                            let Some(mapping) = mappings.get(&key) else {
                                return Err(AppError::Validation(format!(
                                    "missing mapping for source {} root '{}'",
                                    source_result.source_key, series_entry.series.root_folder_path
                                )));
                            };
                            record_series_setting_sample(
                                &mut library_setting_accumulators,
                                mapping,
                                &series_entry.series,
                            );
                            let mut remapped_series = series_entry.series.clone();
                            remapped_series.path = remap_import_path(
                                remapped_series.path,
                                &mapping.arr_root_path,
                                &mapping.scryer_root_path,
                            );
                            let remapped_episodes = series_entry
                                .episodes
                                .iter()
                                .cloned()
                                .map(|mut episode| {
                                    episode.file_path = remap_import_path(
                                        episode.file_path,
                                        &mapping.arr_root_path,
                                        &mapping.scryer_root_path,
                                    );
                                    episode
                                })
                                .collect::<Vec<_>>();
                            push_sonarr_scan_hints_for_mapping(
                                &mut scan_hints,
                                &mapping.facet,
                                &remapped_series,
                                &remapped_episodes,
                            );
                            let entry =
                                series_monitor_entry_from_arr(remapped_series, remapped_episodes);
                            let merge_key = series_monitor_merge_key_for_source(
                                &mapping.facet,
                                &entry,
                                &source_result.source_key,
                                series_entry.series.id,
                            );
                            series_entries
                                .entry((mapping.library_id.clone(), merge_key))
                                .and_modify(|(_, existing)| {
                                    merge_series_monitor_entry(existing, entry.clone())
                                })
                                .or_insert((mapping.facet.clone(), entry));
                            Ok(())
                        },
                    )
                    .await
                    .map_err(to_gql_error)?;
                }
            }
        }

        let mut movie_entries_by_library =
            BTreeMap::<String, Vec<ExternalImportMonitorMovieEntry>>::new();
        for ((library_id, _), entry) in movie_entries {
            movie_entries_by_library
                .entry(library_id)
                .or_default()
                .push(entry);
        }
        for (library_id, entries) in movie_entries_by_library {
            let mut writer = SnapshotChunkWriter::new(
                app.clone(),
                actor.clone(),
                scryer_application::external_import_monitor_apply_session_id_for_library(
                    &library_id,
                ),
                MediaFacet::Movie,
                ExternalImportMonitorSnapshotEntryKind::Movie,
            );
            for entry in entries {
                writer.push(&entry).await?;
            }
            writer.finish().await?;
        }

        let mut series_entries_by_library =
            BTreeMap::<(String, String), (MediaFacet, Vec<ExternalImportMonitorSeriesEntry>)>::new(
            );
        for ((library_id, _), (facet, entry)) in series_entries {
            series_entries_by_library
                .entry((library_id, facet.as_str().to_string()))
                .or_insert_with(|| (facet.clone(), Vec::new()))
                .1
                .push(entry);
        }
        for ((library_id, _), (facet, entries)) in series_entries_by_library {
            let mut writer = SnapshotChunkWriter::new(
                app.clone(),
                actor.clone(),
                scryer_application::external_import_monitor_apply_session_id_for_library(
                    &library_id,
                ),
                facet,
                ExternalImportMonitorSnapshotEntryKind::Series,
            );
            for entry in entries {
                writer.push(&entry).await?;
            }
            writer.finish().await?;
        }
        app.set_external_import_monitor_warmup_scan_hints(&actor, apply_session_id, scan_hints)
            .await;
        let mut library_setting_applications = derive_external_import_library_setting_applications(
            &library_setting_accumulators,
            &source_results,
            &quality_profile_settings.profiles,
        );
        apply_external_import_library_setting_applications(
            &app,
            &actor,
            &mut library_setting_applications,
        )
        .await
        .map_err(to_gql_error)?;
        for session_id in &source_order {
            let _ =
                clear_external_import_arr_source_snapshot_chunks(&app, &actor, session_id).await;
            let _ = app
                .remove_external_import_monitor_warmup_session(&actor, session_id)
                .await;
        }

        Ok(FinalizeExternalImportPayload {
            monitor_warmup_session_id: ID::from(apply_session_id),
        })
    }

    /// Re-connect to Sonarr/Radarr, fetch configs, and create selected items in Scryer.
    async fn execute_external_import(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "Selected imported clients and indexers plus any credentials needed to create them."
        )]
        input: ExecuteExternalImportInput,
    ) -> GqlResult<ExternalImportResultPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;

        let selected_dc_keys: HashSet<String> = input
            .selected_download_client_dedup_keys
            .into_iter()
            .collect();
        let selected_idx_keys: HashSet<String> =
            input.selected_indexer_dedup_keys.into_iter().collect();
        let dc_api_key_overrides: HashMap<String, String> = input
            .download_client_api_key_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.api_key))
            .collect();
        let dc_password_overrides: HashMap<String, String> = input
            .download_client_password_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.password))
            .collect();
        let idx_api_key_overrides: HashMap<String, String> = input
            .indexer_api_key_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.api_key))
            .collect();

        let mut result = ExternalImportResultPayload {
            media_paths_saved: false,
            download_clients_created: 0,
            indexers_created: 0,
            plugins_installed: Vec::new(),
            errors: Vec::new(),
        };

        // ── Collect download clients + indexers from external apps ─────────
        let mut all_download_clients: Vec<(ArrDownloadClient, String)> = Vec::new();
        let mut all_indexers: Vec<(ArrIndexer, String)> = Vec::new();
        let mut seen_dc_keys: HashSet<String> = HashSet::new();
        let mut seen_idx_keys: HashSet<String> = HashSet::new();
        let mut prowlarr_groups: HashMap<String, ProwlarrImportGroup> = HashMap::new();
        let linked_prowlarr_base_url = input.prowlarr.as_ref().map(|conn| conn.base_url.as_str());

        if let Some(conn) = &input.prowlarr {
            let dedup_key = prowlarr_dedup_key(&conn.base_url);
            if selected_idx_keys.contains(&dedup_key) {
                merge_direct_prowlarr_group(
                    &mut prowlarr_groups,
                    &conn.base_url,
                    &conn.api_key,
                    &[],
                );
            }
        }

        for session_id in input.source_warmup_session_ids {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await?;
            if snapshot.status != ExternalImportMonitorWarmupStatus::Completed {
                result.errors.push(format!(
                    "source warmup session {session_id_string} is not completed"
                ));
                continue;
            }
            let warmup = app
                .external_import_arr_source_warmup_result(&actor, &session_id_string)
                .await?;

            for dc in warmup.download_clients {
                let mapped = map_download_client(&dc, &warmup.source_key);
                if mapped.supported
                    && seen_dc_keys.insert(mapped.dedup_key.clone())
                    && selected_dc_keys.contains(&mapped.dedup_key)
                {
                    all_download_clients.push((dc, warmup.source_key.clone()));
                }
            }

            for idx in warmup.indexers {
                if external_import::should_skip_imported_indexer(&idx) {
                    continue;
                }

                if let Some(detected) =
                    detect_imported_prowlarr_proxy_indexer(&idx, linked_prowlarr_base_url)
                {
                    let dedup_key = prowlarr_dedup_key(&detected.base_url);
                    if selected_idx_keys.contains(&dedup_key) {
                        merge_prowlarr_group(&mut prowlarr_groups, detected, &warmup.source_key);
                    }
                    continue;
                }

                let mapped = map_indexer(&idx, &warmup.source_key);
                if mapped.supported
                    && seen_idx_keys.insert(mapped.dedup_key.clone())
                    && selected_idx_keys.contains(&mapped.dedup_key)
                {
                    all_indexers.push((idx, warmup.source_key.clone()));
                }
            }
        }

        // ── Create download clients ───────────────────────────────────────
        for (dc, _source) in &all_download_clients {
            let Some(scryer_type) = external_import::map_download_client_type(&dc.implementation)
            else {
                continue;
            };

            let host = external_import::field_str(&dc.fields, "host").unwrap_or_default();
            let port = external_import::field_str_or_number(&dc.fields, "port").unwrap_or_default();
            let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
            let url_base = external_import::field_str(&dc.fields, "urlBase").unwrap_or_default();

            let mut config_obj = imported_download_client_connection_config(
                scryer_type,
                &host,
                &port,
                use_ssl,
                &url_base,
            );

            if scryer_type == "sabnzbd" || scryer_type == "weaver" {
                // Prefer a user-supplied override (needed when Sonarr/Radarr masked
                // the key), then fall back to the value fetched from the arr API.
                let dedup_key = format!("{}:{}:{}", scryer_type, host, port);
                let api_key = dc_api_key_overrides
                    .get(&dedup_key)
                    .cloned()
                    .or_else(|| external_import::field_str_sensitive(&dc.fields, "apiKey"));
                if let Some(api_key) = api_key {
                    config_obj.insert("api_key".into(), serde_json::Value::String(api_key));
                }
            } else {
                let dedup_key = format!("{}:{}:{}", scryer_type, host, port);
                if let Some(username) = external_import::field_str(&dc.fields, "username") {
                    config_obj.insert("username".into(), serde_json::Value::String(username));
                }
                let password = dc_password_overrides
                    .get(&dedup_key)
                    .cloned()
                    .or_else(|| external_import::field_str_sensitive(&dc.fields, "password"));
                if let Some(password) = password {
                    config_obj.insert("password".into(), serde_json::Value::String(password));
                }
            }

            let config_json = serde_json::Value::Object(config_obj).to_string();

            match app
                .create_download_client_config(
                    &actor,
                    NewDownloadClientConfig {
                        name: dc.name.clone(),
                        client_type: scryer_type.to_string(),
                        config_json,
                        client_priority: 0,
                        is_enabled: true,
                        proxy_config_id: None,
                    },
                )
                .await
            {
                Ok(config) => {
                    result.download_clients_created += 1;
                    if scryer_type == "nzbget"
                        || scryer_type == "sabnzbd"
                        || scryer_type == "weaver"
                    {
                        let _ = app
                            .ensure_download_client_routing_entry_for_client(&actor, &config.id)
                            .await;
                    }
                }
                Err(err) => {
                    result.errors.push(format!(
                        "failed to create download client '{}': {err}",
                        dc.name
                    ));
                }
            }
        }

        // ── Create native Prowlarr parents and sync managed children ───────
        for group in prowlarr_groups.values() {
            let dedup_key = group.dedup_key();
            let override_api_key = idx_api_key_overrides
                .get(&dedup_key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let api_key = override_api_key.or_else(|| {
                if group.api_key_conflict {
                    None
                } else {
                    group.api_key.clone()
                }
            });
            let Some(api_key) = api_key else {
                let help = prowlarr_api_key_help_url(&group.base_url)
                    .map(|url| format!(" ({url})"))
                    .unwrap_or_default();
                let reason = if group.api_key_conflict {
                    "visible API keys conflicted"
                } else {
                    "API key is missing or masked"
                };
                result.errors.push(format!(
                    "failed to import {}: {reason}; enter the Prowlarr API key from Prowlarr -> Settings -> General{help}",
                    prowlarr_display_name(&group.base_url)
                ));
                continue;
            };

            let name = prowlarr_display_name(&group.base_url);
            let config_json = prowlarr_parent_config_json(&group.base_url, &api_key);
            let existing_parents = match app
                .list_indexer_configs(&actor, Some("prowlarr".to_string()))
                .await
            {
                Ok(configs) => configs,
                Err(err) => {
                    result.errors.push(format!(
                        "failed to inspect existing Prowlarr configs for '{name}': {err}"
                    ));
                    continue;
                }
            };

            let existing_parent = existing_parents.into_iter().find(|config| {
                indexer_config_base_url(config.config_json.as_deref()).is_some_and(
                    |existing_base_url| same_base_url(&existing_base_url, &group.base_url),
                )
            });

            if let Some(existing_config) = existing_parent {
                match app
                    .update_indexer_config(
                        &actor,
                        IndexerConfigUpdate {
                            id: existing_config.id.clone(),
                            name: None,
                            provider_type: None,
                            derived_base_url: None,
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            is_enabled: Some(true),
                            enable_interactive_search: None,
                            enable_auto_search: None,
                            proxy_config_id: None,
                            download_client_id: None,
                            seeding_profile_id: None,
                            managed_parent_config_id: None,
                            managed_child_key: None,
                            managed_metadata_json: None,
                            caps_snapshot_json: None,
                            config_json: Some(config_json.clone()),
                        },
                    )
                    .await
                {
                    Ok(_) => {}
                    Err(err) => {
                        result
                            .errors
                            .push(format!("failed to update Prowlarr config '{name}': {err}"));
                        continue;
                    }
                }
            } else {
                match app
                    .create_indexer_config(
                        &actor,
                        NewIndexerConfig {
                            name: name.clone(),
                            provider_type: "prowlarr".to_string(),
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            is_enabled: true,
                            enable_interactive_search: false,
                            enable_auto_search: false,
                            proxy_config_id: None,
                            download_client_id: None,
                            config_json: Some(config_json.clone()),
                        },
                    )
                    .await
                {
                    Ok(_config) => {
                        result.indexers_created += 1;
                    }
                    Err(err) => {
                        result
                            .errors
                            .push(format!("failed to create Prowlarr config '{name}': {err}"));
                        continue;
                    }
                }
            }
        }

        // ── Auto-install non-builtin plugins needed by selected indexers ──
        let available_providers: HashSet<String> = app
            .available_indexer_provider_types()
            .iter()
            .map(|(pt, _, _, _)| pt.clone())
            .collect();

        let mut auto_installed: HashSet<String> = HashSet::new();
        for (idx, _) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };
            if available_providers.contains(scryer_type) || auto_installed.contains(scryer_type) {
                continue;
            }
            // Plugin not loaded — try to install from registry
            let install_result = match app.install_plugin(&actor, scryer_type).await {
                Ok(inst) => Ok(inst),
                Err(_) => {
                    // Catalog might not be cached yet — refresh and retry
                    let _ = app.refresh_plugin_catalog_internal().await;
                    app.install_plugin(&actor, scryer_type).await
                }
            };
            match install_result {
                Ok(inst) => {
                    auto_installed.insert(scryer_type.to_string());
                    result.plugins_installed.push(inst.name);
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to install {} plugin: {err}", scryer_type));
                }
            }
        }

        // ── Create indexers ───────────────────────────────────────────────
        for (idx, _source) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };

            let base_url = external_import::field_str(&idx.fields, "baseUrl").unwrap_or_default();
            let api_path = external_import::field_str(&idx.fields, "apiPath");
            let dedup_key = format!("{}:{}", scryer_type, base_url);
            let api_key = idx_api_key_overrides
                .get(&dedup_key)
                .cloned()
                .or_else(|| external_import::field_str_sensitive(&idx.fields, "apiKey"));
            let fields = match app.indexer_config_fields_for_provider_type(scryer_type) {
                Ok(fields) => fields,
                Err(_) => continue,
            };
            let config_json = imported_indexer_config_json(
                &fields,
                &base_url,
                api_key.as_deref(),
                api_path.as_deref(),
            );

            // If the plugin was just auto-installed, it may have auto-created a
            // default IndexerConfig. Update that config instead of creating a
            // duplicate. Once claimed, further indexers of the same type create
            // new configs normally.
            if auto_installed.remove(scryer_type) {
                let existing = app
                    .list_indexer_configs(&actor, Some(scryer_type.to_string()))
                    .await
                    .unwrap_or_default();
                if let Some(existing_config) = existing.first() {
                    if existing_config.config_json.as_deref() != Some(config_json.as_str()) {
                        let _ = app
                            .update_indexer_config(
                                &actor,
                                IndexerConfigUpdate {
                                    id: existing_config.id.clone(),
                                    name: Some(idx.name.clone()),
                                    provider_type: None,
                                    derived_base_url: None,
                                    rate_limit_seconds: None,
                                    rate_limit_burst: None,
                                    is_enabled: None,
                                    enable_interactive_search: None,
                                    enable_auto_search: None,
                                    proxy_config_id: None,
                                    download_client_id: None,
                                    seeding_profile_id: None,
                                    managed_parent_config_id: None,
                                    managed_child_key: None,
                                    managed_metadata_json: None,
                                    caps_snapshot_json: None,
                                    config_json: Some(config_json.clone()),
                                },
                            )
                            .await;
                    }
                    result.indexers_created += 1;
                    continue;
                }
            }

            match app
                .create_indexer_config(
                    &actor,
                    NewIndexerConfig {
                        name: idx.name.clone(),
                        provider_type: scryer_type.to_string(),
                        rate_limit_seconds: None,
                        rate_limit_burst: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        proxy_config_id: None,
                        download_client_id: None,
                        config_json: Some(config_json),
                    },
                )
                .await
            {
                Ok(_) => {
                    result.indexers_created += 1;
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to create indexer '{}': {err}", idx.name));
                }
            }
        }

        Ok(result)
    }
}

fn movie_scan_hint_from_arr(movie: &ArrMovie) -> Option<LibraryScanHint> {
    let file_path = movie.file_path.as_deref()?;
    let path_key = library_scan_file_leaf_key(file_path)?;
    let full_path_key = library_scan_file_full_path_key(file_path);
    let mut ids = Vec::new();
    if let Some(tmdb_id) = movie
        .tmdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tmdb, value))
    {
        ids.push(tmdb_id);
    }
    if let Some(imdb_id) = movie
        .imdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Imdb, value))
    {
        ids.push(imdb_id);
    }

    (!ids.is_empty()).then_some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportRadarr,
        facet: LibraryScanHintFacet::Movie,
        path_key,
        full_path_key,
        ids,
    })
}

fn series_folder_scan_hint_from_arr(series: &ArrSeries) -> Option<LibraryScanHint> {
    let series_path = series.path.as_deref()?;
    let path_key = library_scan_folder_leaf_key(series_path)?;
    let full_path_key = library_scan_folder_full_path_key(series_path);
    let ids = series
        .tvdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tvdb, value))
        .map(|id| vec![id])?;

    Some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportSonarr,
        facet: LibraryScanHintFacet::Series,
        path_key,
        full_path_key,
        ids,
    })
}

fn series_episode_scan_hint_from_arr(
    series: &ArrSeries,
    episode: &ArrEpisode,
) -> Option<LibraryScanHint> {
    let file_path = episode.file_path.as_deref()?;
    let path_key = library_scan_file_leaf_key(file_path)?;
    let full_path_key = library_scan_file_full_path_key(file_path);
    let ids = series
        .tvdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tvdb, value))
        .map(|id| vec![id])?;

    Some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportSonarr,
        facet: LibraryScanHintFacet::Series,
        path_key,
        full_path_key,
        ids,
    })
}

fn push_sonarr_scan_hints_for_mapping(
    scan_hints: &mut LibraryScanHintSet,
    facet: &MediaFacet,
    series: &ArrSeries,
    episodes: &[ArrEpisode],
) {
    if !matches!(facet, MediaFacet::Series | MediaFacet::Anime) {
        return;
    }
    if let Some(hint) = series_folder_scan_hint_from_arr(series) {
        scan_hints.push(hint);
    }
    for episode in episodes {
        if let Some(hint) = series_episode_scan_hint_from_arr(series, episode) {
            scan_hints.push(hint);
        }
    }
}

fn movie_monitor_entry_from_arr(movie: &ArrMovie) -> ExternalImportMonitorMovieEntry {
    ExternalImportMonitorMovieEntry {
        tmdb_id: movie.tmdb_id.clone(),
        imdb_id: movie.imdb_id.clone(),
        path: movie.path.clone(),
        monitored: movie.monitored,
    }
}

fn series_monitor_entry_from_arr(
    series: ArrSeries,
    episodes: Vec<ArrEpisode>,
) -> ExternalImportMonitorSeriesEntry {
    let title_monitored = series.monitored;
    let season_defaults = series
        .seasons
        .iter()
        .map(|season| (season.season_number, season.monitored))
        .collect::<HashMap<_, _>>();

    ExternalImportMonitorSeriesEntry {
        tvdb_id: series.tvdb_id,
        path: series.path,
        monitored: title_monitored,
        seasons: series
            .seasons
            .into_iter()
            .filter(|season| season.monitored != title_monitored)
            .map(|season| ExternalImportMonitorSeasonEntry {
                season_number: season.season_number,
                monitored: season.monitored,
            })
            .collect(),
        episodes: episodes
            .into_iter()
            .filter(|episode| {
                let effective_default = season_defaults
                    .get(&episode.season_number)
                    .copied()
                    .unwrap_or(title_monitored);
                episode.monitored != effective_default
            })
            .map(|episode| ExternalImportMonitorEpisodeEntry {
                tvdb_id: episode.tvdb_id,
                season_number: episode.season_number,
                episode_number: episode.episode_number,
                monitored: episode.monitored,
            })
            .collect(),
    }
}

fn should_publish_progress(count: i32) -> bool {
    count <= 10 || count % 25 == 0
}

fn recompute_warmup_overall_progress(snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot) {
    let components = [
        (
            snapshot.movies_total_known,
            snapshot.movies_progress.clone(),
        ),
        (
            snapshot.series_total_known,
            snapshot.series_progress.clone(),
        ),
        (
            snapshot.episode_fetch_total_known,
            snapshot.episode_fetch_progress.clone(),
        ),
        (
            snapshot.snapshot_build_total_known,
            snapshot.snapshot_build_progress.clone(),
        ),
    ];

    snapshot.overall_total_known = components.iter().all(|(known, _)| *known);
    snapshot.overall_progress.total = components.iter().map(|(_, progress)| progress.total).sum();
    snapshot.overall_progress.completed = components
        .iter()
        .map(|(_, progress)| progress.completed)
        .sum();
    snapshot.overall_progress.failed = components.iter().map(|(_, progress)| progress.failed).sum();
}

async fn publish_warmup_progress(
    app: &scryer_application::AppUseCase,
    session_id: &str,
    snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot,
) {
    recompute_warmup_overall_progress(snapshot);
    app.update_external_import_monitor_warmup_progress(session_id, snapshot.clone())
        .await;
}

async fn clear_external_import_arr_source_snapshot_chunks(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
) -> scryer_application::AppResult<()> {
    app.clear_external_import_arr_source_session_chunks(actor, session_id)
        .await
}

pub(crate) async fn maintain_external_import_source_sessions(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
) -> scryer_application::AppResult<()> {
    app.maintain_external_import_arr_source_sessions(actor)
        .await
}

async fn capture_external_import_arr_source_warmup(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
    source: &ExternalArrImportSource,
    cancel_token: &CancellationToken,
    snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot,
) -> scryer_application::AppResult<()> {
    clear_external_import_arr_source_snapshot_chunks(app, actor, session_id).await?;

    snapshot.status = ExternalImportMonitorWarmupStatus::Running;
    snapshot.phase = match source.kind {
        AppArrSourceKind::Radarr => ExternalImportMonitorWarmupPhase::LoadingMovies,
        AppArrSourceKind::Sonarr => ExternalImportMonitorWarmupPhase::LoadingSeries,
    };
    publish_warmup_progress(app, session_id, snapshot).await;

    let client = client_for_arr_source(source)?;
    let (_app_name, version) = client.test_connection().await?;
    let root_folders = client.list_root_folders().await.unwrap_or_default();
    let download_clients = client.list_download_clients().await.unwrap_or_default();
    let indexers = client.list_indexers().await.unwrap_or_default();
    let mut signal_warnings = Vec::new();
    let naming_config = match client.get_naming_config().await {
        Ok(config) => Some(config),
        Err(error) => {
            signal_warnings.push(format!("naming config unavailable: {error}"));
            None
        }
    };
    let media_management_config = match client.get_media_management_config().await {
        Ok(config) => Some(config),
        Err(error) => {
            signal_warnings.push(format!("media management config unavailable: {error}"));
            None
        }
    };
    let metadata_providers = match client.list_metadata_providers().await {
        Ok(providers) => providers,
        Err(error) => {
            signal_warnings.push(format!("metadata providers unavailable: {error}"));
            Vec::new()
        }
    };
    let quality_profiles = match client.list_quality_profiles().await {
        Ok(profiles) => profiles,
        Err(error) => {
            signal_warnings.push(format!("quality profiles unavailable: {error}"));
            Vec::new()
        }
    };
    let mut result = ExternalImportArrSourceWarmupResult {
        source_key: source.source_key.clone(),
        kind: source.kind,
        base_url: source.base_url.clone(),
        version: Some(version),
        root_folders,
        title_root_paths: Vec::new(),
        naming_config,
        media_management_config,
        metadata_providers,
        quality_profiles,
        signal_warnings,
        download_clients,
        indexers,
    };
    app.set_external_import_arr_source_warmup_result(session_id, result.clone())
        .await;

    match source.kind {
        AppArrSourceKind::Radarr => {
            let mut movie_writer = SnapshotChunkWriter::new(
                app.clone(),
                actor.clone(),
                session_id.to_string(),
                MediaFacet::Movie,
                ExternalImportMonitorSnapshotEntryKind::Movie,
            );
            let movies = client.list_movies().await?;
            let total = i32::try_from(movies.len()).unwrap_or(i32::MAX);
            snapshot.movies_total_known = true;
            snapshot.movies_progress.total = total;
            snapshot.matched_movie_count = total;
            publish_warmup_progress(app, session_id, snapshot).await;

            for movie in movies {
                if cancel_token.is_cancelled() {
                    return Ok(());
                }
                movie_writer.push(&movie).await?;
                push_unique(&mut result.title_root_paths, movie.root_folder_path.clone());
                snapshot.movies_progress.completed =
                    snapshot.movies_progress.completed.saturating_add(1);
                if should_publish_progress(snapshot.movies_progress.completed) {
                    publish_warmup_progress(app, session_id, snapshot).await;
                }
            }
            movie_writer.finish().await?;
        }
        AppArrSourceKind::Sonarr => {
            let all_series = client.list_series().await?;
            let total = i32::try_from(all_series.len()).unwrap_or(i32::MAX);
            snapshot.series_total_known = true;
            snapshot.series_progress.total = total;
            snapshot.matched_series_count = total;
            publish_warmup_progress(app, session_id, snapshot).await;

            snapshot.phase = ExternalImportMonitorWarmupPhase::LoadingEpisodes;
            publish_warmup_progress(app, session_id, snapshot).await;

            // Episode fetching is throttled to a few instances at a time via a
            // global semaphore. Acquire it cancellation-aware: a warmup that was
            // superseded/cancelled (e.g. the operator edited the connection,
            // spawning a fresh session) must NOT keep competing for — or briefly
            // hold — one of the scarce permits, or it would starve the live
            // warmup and leave it stuck at "loading episodes" forever.
            if cancel_token.is_cancelled() {
                return Ok(());
            }
            let _active_instance_permit: OwnedSemaphorePermit = tokio::select! {
                biased;
                () = cancel_token.cancelled() => return Ok(()),
                permit = sonarr_active_episode_instance_semaphore().acquire_owned() => {
                    permit.map_err(|err| {
                        AppError::Repository(format!(
                            "failed to acquire Sonarr episode warmup slot: {err}"
                        ))
                    })?
                }
            };
            let mut series_writer = SnapshotChunkWriter::new(
                app.clone(),
                actor.clone(),
                session_id.to_string(),
                MediaFacet::Series,
                ExternalImportMonitorSnapshotEntryKind::Series,
            );
            let mut pending_series = all_series.into_iter();
            let mut join_set = JoinSet::new();

            let spawn_episode_fetch = |join_set: &mut JoinSet<(
                ArrSeries,
                scryer_application::AppResult<Vec<ArrEpisode>>,
            )>,
                                       client: &ExternalArrClient,
                                       series: ArrSeries| {
                let client = client.clone();
                join_set.spawn(async move {
                    let series_path = series.path.clone();
                    let result = client
                        .list_episodes_for_series(series.id, series_path.as_deref())
                        .await;
                    (series, result)
                });
            };

            for _ in 0..SONARR_EPISODE_FETCH_CONCURRENCY_PER_INSTANCE {
                let Some(series) = pending_series.next() else {
                    break;
                };
                spawn_episode_fetch(&mut join_set, &client, series);
            }

            while let Some(join_result) = join_set.join_next().await {
                if cancel_token.is_cancelled() {
                    return Ok(());
                }
                let (series, episodes_result) = join_result.map_err(|err| {
                    AppError::Repository(format!("failed to join Sonarr episode fetch task: {err}"))
                })?;
                let episodes = episodes_result?;
                push_unique(
                    &mut result.title_root_paths,
                    series.root_folder_path.clone(),
                );
                let entry = ExternalImportArrSourceSeriesEntry { series, episodes };
                series_writer.push(&entry).await?;
                snapshot.series_progress.completed =
                    snapshot.series_progress.completed.saturating_add(1);
                if should_publish_progress(snapshot.series_progress.completed) {
                    publish_warmup_progress(app, session_id, snapshot).await;
                }
                if let Some(next_series) = pending_series.next() {
                    spawn_episode_fetch(&mut join_set, &client, next_series);
                }
            }
            series_writer.finish().await?;
        }
    }

    app.set_external_import_arr_source_warmup_result(session_id, result)
        .await;
    Ok(())
}

async fn run_external_import_arr_source_warmup_job(
    app: scryer_application::AppUseCase,
    actor: scryer_domain::User,
    session_id: String,
    source: ExternalArrImportSource,
    cancel_token: CancellationToken,
    mut snapshot: ExternalImportMonitorWarmupProgressSnapshot,
) {
    let outcome = capture_external_import_arr_source_warmup(
        &app,
        &actor,
        &session_id,
        &source,
        &cancel_token,
        &mut snapshot,
    )
    .await;

    if cancel_token.is_cancelled() {
        let _ = clear_external_import_arr_source_snapshot_chunks(&app, &actor, &session_id).await;
        snapshot.status = ExternalImportMonitorWarmupStatus::Canceled;
        snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
        snapshot.error_message = None;
        publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        return;
    }

    match outcome {
        Ok(()) => {
            snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        }
        Err(err) => {
            let _ =
                clear_external_import_arr_source_snapshot_chunks(&app, &actor, &session_id).await;
            snapshot.status = ExternalImportMonitorWarmupStatus::Failed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            snapshot.error_message = Some(err.to_string());
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        }
    }
}

async fn run_external_import_prowlarr_warmup_job(
    app: scryer_application::AppUseCase,
    actor: scryer_domain::User,
    session_id: String,
    base_url: String,
    api_key: String,
    cancel_token: CancellationToken,
    mut snapshot: ExternalImportMonitorWarmupProgressSnapshot,
) {
    let started_at = Instant::now();
    snapshot.status = ExternalImportMonitorWarmupStatus::Running;
    snapshot.phase = ExternalImportMonitorWarmupPhase::LoadingIndexers;
    snapshot.error_message = None;
    publish_warmup_progress(&app, &session_id, &mut snapshot).await;

    let config_json = prowlarr_parent_config_json(&base_url, &api_key);
    let outcome = tokio::select! {
        _ = cancel_token.cancelled() => None,
        result = app.preview_managed_indexer_children(
            &actor,
            "prowlarr",
            Some(&config_json),
        ) => Some(result),
    };

    match outcome {
        None => {
            snapshot.status = ExternalImportMonitorWarmupStatus::Canceled;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            snapshot.error_message = None;
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
            tracing::info!(
                session_id = %session_id,
                child_count = 0,
                duration_ms = started_at.elapsed().as_millis() as u64,
                terminal_status = "canceled",
                "external Prowlarr import warmup finished"
            );
        }
        Some(Ok((validation, plan))) => {
            let child_count = plan.children.len().min(i32::MAX as usize) as i32;
            app.set_external_import_prowlarr_warmup_result(
                &session_id,
                ExternalImportProwlarrWarmupResult {
                    base_url,
                    api_key,
                    version: version_from_validation_result(&validation),
                    plan,
                },
            )
            .await;
            snapshot.overall_total_known = true;
            snapshot.overall_progress.total = child_count;
            snapshot.overall_progress.completed = child_count;
            snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
            tracing::info!(
                session_id = %session_id,
                child_count,
                duration_ms = started_at.elapsed().as_millis() as u64,
                terminal_status = "completed",
                "external Prowlarr import warmup finished"
            );
        }
        Some(Err(error)) => {
            snapshot.status = ExternalImportMonitorWarmupStatus::Failed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            snapshot.error_message = Some(error.to_string());
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
            tracing::warn!(
                session_id = %session_id,
                child_count = 0,
                duration_ms = started_at.elapsed().as_millis() as u64,
                terminal_status = "failed",
                "external Prowlarr import warmup finished"
            );
        }
    }
}

/// Build the only configuration values copied from an Arr download client.
/// Routing categories are Scryer-owned settings and deliberately stay out of
/// this connection payload.
fn imported_download_client_connection_config(
    client_type: &str,
    host: &str,
    port: &str,
    use_ssl: bool,
    url_base: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut config = serde_json::Map::new();
    config.insert("host".into(), serde_json::Value::String(host.to_string()));
    config.insert("port".into(), serde_json::Value::String(port.to_string()));
    config.insert("use_ssl".into(), serde_json::Value::Bool(use_ssl));
    config.insert(
        "url_base".into(),
        serde_json::Value::String(url_base.to_string()),
    );
    config.insert(
        "client_type".into(),
        serde_json::Value::String(client_type.to_string()),
    );
    config
}

fn map_download_client(
    dc: &ArrDownloadClient,
    source: &str,
) -> ExternalImportDownloadClientPayload {
    let scryer_type = external_import::map_download_client_type(&dc.implementation);
    let host = external_import::field_str(&dc.fields, "host");
    let port = external_import::field_str_or_number(&dc.fields, "port");
    let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
    let url_base = external_import::field_str(&dc.fields, "urlBase");
    let username = external_import::field_str(&dc.fields, "username");
    // Use field_str_sensitive so that Sonarr/Radarr's "********" mask becomes
    // None — callers can then detect that the key must be entered manually.
    let api_key_present = external_import::field_str_sensitive(&dc.fields, "apiKey").is_some();
    let password = external_import::field_str_sensitive(&dc.fields, "password");

    let dedup_key = format!(
        "{}:{}:{}",
        scryer_type.unwrap_or("unsupported"),
        host.as_deref().unwrap_or(""),
        port.as_deref().unwrap_or("")
    );

    ExternalImportDownloadClientPayload {
        source_keys: vec![source.to_string()],
        name: dc.name.clone(),
        implementation: dc.implementation.clone(),
        scryer_client_type: scryer_type.map(str::to_string),
        host,
        port,
        use_ssl,
        url_base,
        username,
        api_key_present,
        dedup_key,
        supported: scryer_type.is_some(),
        requires_password_override: password.is_none()
            && scryer_type.is_some_and(|client_type| client_type == "nzbget"),
    }
}

fn map_indexer(idx: &ArrIndexer, source: &str) -> ExternalImportIndexerPayload {
    let scryer_type = external_import::map_indexer_provider_type(&idx.implementation, &idx.fields);
    let base_url = external_import::field_str(&idx.fields, "baseUrl");
    let api_key_present = external_import::field_str_sensitive(&idx.fields, "apiKey").is_some();

    let dedup_key = format!(
        "{}:{}",
        scryer_type.unwrap_or("unsupported"),
        base_url.as_deref().unwrap_or("")
    );

    ExternalImportIndexerPayload {
        source_keys: vec![source.to_string()],
        name: idx.name.clone(),
        implementation: idx.implementation.clone(),
        scryer_provider_type: scryer_type.map(str::to_string),
        base_url,
        api_key_present,
        dedup_key,
        supported: scryer_type.is_some(),
        child_count: 0,
        child_names: Vec::new(),
        requires_api_key_override: false,
        api_key_help_url: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use scryer_application::external_import::{
        ArrDownloadClient, ArrEpisode, ArrIndexer, ArrMediaManagementConfig, ArrMetadataProvider,
        ArrMovie, ArrNamingConfig, ArrQualityProfile, ArrSeries, ArrSeriesSeason,
        ArrSeriesStatistics,
    };
    use scryer_application::{
        ExternalIdProvider, ExternalImportArrSourceKind, ExternalImportArrSourceWarmupResult,
        LibraryScanHintFacet, LibraryScanHintSet, LibraryScanHintSource, QualityProfile,
        QualityProfileCriteria, library_scan_file_full_path_key, library_scan_file_leaf_key,
        library_scan_folder_full_path_key, library_scan_folder_leaf_key,
    };
    use scryer_domain::{
        ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource, MediaFacet,
    };
    use serde_json::Value;

    use super::{
        ExternalImportLibrarySettingDisposition, ExternalImportLibrarySettingKey,
        ResolvedSourceMapping, SONARR_ACTIVE_EPISODE_INSTANCE_CONCURRENCY,
        SONARR_EPISODE_FETCH_CONCURRENCY_PER_INSTANCE,
        build_external_import_library_setting_accumulators,
        derive_external_import_library_setting_applications,
        detect_imported_prowlarr_proxy_indexer, imported_download_client_connection_config,
        imported_indexer_config_json, is_external_import_library_auto_apply_setting,
        map_download_client, map_indexer, merge_direct_prowlarr_group, merge_prowlarr_group,
        movie_scan_hint_from_arr, prowlarr_dedup_key, push_sonarr_scan_hints_for_mapping,
        record_series_setting_sample, remap_import_path, series_episode_scan_hint_from_arr,
        series_folder_scan_hint_from_arr,
    };

    #[test]
    fn radarr_warmup_builds_movie_hint_with_tmdb_and_imdb() {
        let path = "/Movies/The Lantern Supremacy (2004)";
        let file_path = "/Movies/The Lantern Supremacy (2004)/The Lantern Supremacy.mkv";
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some(path.into()),
            file_path: Some(file_path.into()),
            tmdb_id: Some("2502".into()),
            imdb_id: Some("tt0372183".into()),
            monitored: true,
            quality_profile_id: None,
            minimum_availability: None,
            original_language: None,
            tags: Vec::new(),
        })
        .expect("movie hint");

        assert_eq!(hint.source, LibraryScanHintSource::ExternalImportRadarr);
        assert_eq!(hint.facet, LibraryScanHintFacet::Movie);
        assert_eq!(
            hint.path_key,
            library_scan_file_leaf_key(file_path).unwrap()
        );
        assert_eq!(
            hint.full_path_key,
            library_scan_file_full_path_key(file_path)
        );
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "2502" })
        );
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Imdb && id.value == "tt0372183" })
        );
    }

    #[test]
    fn radarr_warmup_omits_numeric_only_imdb_hint() {
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some("/Movies/Children of Men (2006)".into()),
            file_path: Some("/Movies/Children of Men (2006)/Children of Men.mkv".into()),
            tmdb_id: Some("9693".into()),
            imdb_id: Some("9693".into()),
            monitored: true,
            quality_profile_id: None,
            minimum_availability: None,
            original_language: None,
            tags: Vec::new(),
        })
        .expect("movie hint");

        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "9693" })
        );
        assert!(
            !hint
                .ids
                .iter()
                .any(|id| id.provider == ExternalIdProvider::Imdb)
        );
    }

    #[test]
    fn radarr_warmup_omits_malformed_imdb_hint() {
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some("/Movies/Children of Men (2006)".into()),
            file_path: Some("/Movies/Children of Men (2006)/Children of Men.mkv".into()),
            tmdb_id: Some("9693".into()),
            imdb_id: Some("tt0206634-extra".into()),
            monitored: true,
            quality_profile_id: None,
            minimum_availability: None,
            original_language: None,
            tags: Vec::new(),
        })
        .expect("movie hint");

        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "9693" })
        );
        assert!(
            !hint
                .ids
                .iter()
                .any(|id| id.provider == ExternalIdProvider::Imdb)
        );
    }

    #[test]
    fn sonarr_warmup_builds_series_hint_with_tvdb() {
        let path = "/Series/Fathomline (2021)";
        let series = ArrSeries {
            id: 1,
            root_folder_path: "/Series".into(),
            path: Some(path.into()),
            tvdb_id: Some("366972".into()),
            monitored: true,
            quality_profile_id: None,
            series_type: None,
            season_folder: None,
            monitor_new_items: None,
            original_language: None,
            tags: Vec::new(),
            seasons: Vec::new(),
            statistics: ArrSeriesStatistics {
                total_episode_count: None,
                monitored_episode_count: None,
            },
        };
        let hint = series_folder_scan_hint_from_arr(&series).expect("series hint");

        assert_eq!(hint.source, LibraryScanHintSource::ExternalImportSonarr);
        assert_eq!(hint.facet, LibraryScanHintFacet::Series);
        assert_eq!(hint.path_key, library_scan_folder_leaf_key(path).unwrap());
        assert_eq!(hint.full_path_key, library_scan_folder_full_path_key(path));
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tvdb && id.value == "366972" })
        );

        let episode_path = "/Series/Fathomline (2021)/Season 01/Fathomline.S01E01.mkv";
        let episode_hint = series_episode_scan_hint_from_arr(
            &series,
            &ArrEpisode {
                id: 1,
                series_id: 1,
                tvdb_id: Some("777001".into()),
                season_number: 1,
                episode_number: 1,
                file_path: Some(episode_path.into()),
                monitored: true,
            },
        )
        .expect("episode hint");
        assert_eq!(
            episode_hint.path_key,
            library_scan_file_leaf_key(episode_path).unwrap()
        );
        assert_eq!(
            episode_hint.full_path_key,
            library_scan_file_full_path_key(episode_path)
        );
        assert!(
            episode_hint
                .ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tvdb && id.value == "366972" })
        );
    }

    #[test]
    fn sonarr_mapping_scan_hints_include_series_and_anime_facets() {
        let series_path = "/srv/media/tv/Fathomline (2021)";
        let episode_path = "/srv/media/tv/Fathomline (2021)/Season 01/Fathomline.S01E01.mkv";
        let series = ArrSeries {
            id: 1,
            root_folder_path: "/srv/media/tv".into(),
            path: Some(series_path.into()),
            tvdb_id: Some("366972".into()),
            monitored: true,
            quality_profile_id: None,
            series_type: None,
            season_folder: None,
            monitor_new_items: None,
            original_language: None,
            tags: Vec::new(),
            seasons: Vec::new(),
            statistics: ArrSeriesStatistics {
                total_episode_count: None,
                monitored_episode_count: None,
            },
        };
        let episode = ArrEpisode {
            id: 1,
            series_id: 1,
            tvdb_id: Some("777001".into()),
            season_number: 1,
            episode_number: 1,
            file_path: Some(episode_path.into()),
            monitored: true,
        };

        for facet in [MediaFacet::Series, MediaFacet::Anime] {
            let mut scan_hints = LibraryScanHintSet::new();
            push_sonarr_scan_hints_for_mapping(
                &mut scan_hints,
                &facet,
                &series,
                std::slice::from_ref(&episode),
            );

            let folder_key = library_scan_folder_leaf_key(series_path).expect("folder key");
            let folder_full_path =
                library_scan_folder_full_path_key(series_path).expect("folder full path key");
            let folder_hint = scan_hints
                .hint_for_scan_path(
                    LibraryScanHintFacet::Series,
                    &folder_key,
                    Some(&folder_full_path),
                )
                .expect("folder hint");
            assert!(
                folder_hint
                    .ids
                    .iter()
                    .any(|id| id.provider == ExternalIdProvider::Tvdb && id.value == "366972")
            );

            let episode_key = library_scan_file_leaf_key(episode_path).expect("episode key");
            let episode_full_path =
                library_scan_file_full_path_key(episode_path).expect("episode full path key");
            let episode_hint = scan_hints
                .hint_for_scan_path(
                    LibraryScanHintFacet::Series,
                    &episode_key,
                    Some(&episode_full_path),
                )
                .expect("episode hint");
            assert!(
                episode_hint
                    .ids
                    .iter()
                    .any(|id| id.provider == ExternalIdProvider::Tvdb && id.value == "366972")
            );
        }
    }

    fn test_warmup_source(
        source_key: &str,
        root_path: &str,
        naming_config: Option<ArrNamingConfig>,
        quality_profiles: Vec<ArrQualityProfile>,
    ) -> ExternalImportArrSourceWarmupResult {
        ExternalImportArrSourceWarmupResult {
            source_key: source_key.into(),
            kind: ExternalImportArrSourceKind::Sonarr,
            base_url: format!("http://{source_key}.local"),
            version: Some("4.0.0".into()),
            root_folders: Vec::new(),
            title_root_paths: vec![root_path.into()],
            naming_config,
            media_management_config: None,
            metadata_providers: Vec::new(),
            quality_profiles,
            signal_warnings: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        }
    }

    fn test_source_mapping(
        session_id: &str,
        source_key: &str,
        arr_root_path: &str,
        library_id: &str,
    ) -> (String, ResolvedSourceMapping) {
        (
            super::mapping_key(session_id, source_key, arr_root_path),
            ResolvedSourceMapping {
                library_id: library_id.into(),
                source_warmup_session_id: Some(session_id.into()),
                arr_root_path: arr_root_path.into(),
                scryer_root_path: format!("/media/{library_id}"),
                facet: MediaFacet::Anime,
            },
        )
    }

    fn test_arr_series(id: i64, root_path: &str, quality_profile_id: i64) -> ArrSeries {
        ArrSeries {
            id,
            root_folder_path: root_path.into(),
            path: Some(format!("{root_path}/Show {id}")),
            tvdb_id: Some(format!("10{id}")),
            monitored: true,
            quality_profile_id: Some(quality_profile_id),
            series_type: Some("anime".into()),
            season_folder: Some(true),
            monitor_new_items: Some("all".into()),
            original_language: Some("Japanese".into()),
            tags: Vec::new(),
            seasons: vec![ArrSeriesSeason {
                season_number: 0,
                monitored: true,
            }],
            statistics: ArrSeriesStatistics::default(),
        }
    }

    #[test]
    fn external_import_setting_derivation_uses_warmed_arr_signals() {
        let session_id = "source-session";
        let library_id = "anime-library";
        let source_result = ExternalImportArrSourceWarmupResult {
            source_key: "sonarr-main".into(),
            kind: ExternalImportArrSourceKind::Sonarr,
            base_url: "http://sonarr.local".into(),
            version: Some("4.0.0".into()),
            root_folders: Vec::new(),
            title_root_paths: vec!["/srv/anime".into()],
            naming_config: Some(ArrNamingConfig {
                rename_enabled: Some(true),
                replace_illegal_characters: Some(true),
                colon_replacement_format: Some("dash".into()),
                standard_format: Some("{Series Title} - S{season:00}E{episode:00}".into()),
                folder_format: Some("{Series Title} ({Release Year})".into()),
                season_folder_format: None,
                specials_folder_format: None,
            }),
            media_management_config: Some(ArrMediaManagementConfig {
                set_permissions_linux: Some(true),
                chmod_folder: Some("775".into()),
                chown_group: Some("media".into()),
            }),
            metadata_providers: vec![ArrMetadataProvider {
                id: 1,
                name: "Kodi".into(),
                implementation: "XbmcMetadata".into(),
                enable: true,
                fields: HashMap::from([
                    ("seriesMetadata".into(), Value::Bool(true)),
                    ("episodeMetadata".into(), Value::Bool(true)),
                ]),
            }],
            quality_profiles: vec![ArrQualityProfile {
                id: 7,
                name: "HD 1080p".into(),
                language: None,
            }],
            signal_warnings: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        };
        let source_results = BTreeMap::from([(session_id.to_string(), source_result)]);
        let mappings = HashMap::from([(
            super::mapping_key(session_id, "sonarr-main", "/srv/anime"),
            ResolvedSourceMapping {
                library_id: library_id.into(),
                source_warmup_session_id: Some(session_id.into()),
                arr_root_path: "/srv/anime".into(),
                scryer_root_path: "/media/anime".into(),
                facet: MediaFacet::Anime,
            },
        )]);
        let mut accumulators =
            build_external_import_library_setting_accumulators(&source_results, &mappings);
        let mapping = mappings.values().next().expect("mapping");
        for id in 1..=3 {
            record_series_setting_sample(
                &mut accumulators,
                mapping,
                &ArrSeries {
                    id,
                    root_folder_path: "/srv/anime".into(),
                    path: Some(format!("/srv/anime/Show {id}")),
                    tvdb_id: Some(format!("10{id}")),
                    monitored: true,
                    quality_profile_id: Some(7),
                    series_type: Some("anime".into()),
                    season_folder: Some(true),
                    monitor_new_items: Some("all".into()),
                    original_language: Some("Japanese".into()),
                    tags: Vec::new(),
                    seasons: vec![ArrSeriesSeason {
                        season_number: 0,
                        monitored: true,
                    }],
                    statistics: ArrSeriesStatistics::default(),
                },
            );
        }
        let catalog_profiles = vec![QualityProfile {
            id: "hd-1080p".into(),
            name: "HD 1080p".into(),
            criteria: QualityProfileCriteria::default(),
        }];

        let applications = derive_external_import_library_setting_applications(
            &accumulators,
            &source_results,
            &catalog_profiles,
        );

        let find = |setting| {
            applications
                .iter()
                .find(|application| application.setting == setting)
                .expect("setting application")
        };
        let rename = find(ExternalImportLibrarySettingKey::RenameEnabled);
        assert_eq!(
            rename.disposition,
            ExternalImportLibrarySettingDisposition::AutoApplied
        );
        assert_eq!(rename.value.bool_value, Some(true));

        let rename_template = find(ExternalImportLibrarySettingKey::RenameTemplate);
        assert_eq!(
            rename_template.disposition,
            ExternalImportLibrarySettingDisposition::Suggested
        );
        assert_eq!(
            rename_template.value.string_value.as_deref(),
            Some("{Series Title} - S{season:00}E{episode:00}")
        );

        let nfo = find(ExternalImportLibrarySettingKey::NfoWriteOnImport);
        assert_eq!(nfo.value.bool_value, Some(true));
        assert_eq!(
            nfo.disposition,
            ExternalImportLibrarySettingDisposition::AutoApplied
        );

        let profile = find(ExternalImportLibrarySettingKey::QualityProfileId);
        assert_eq!(profile.value.string_value.as_deref(), Some("hd-1080p"));
        assert_eq!(
            profile.disposition,
            ExternalImportLibrarySettingDisposition::AutoApplied
        );

        let request_profiles = find(ExternalImportLibrarySettingKey::RequestQualityProfileIds);
        assert_eq!(
            request_profiles.value.string_list_value.as_deref(),
            Some(&["hd-1080p".to_string()][..])
        );

        let monitor_specials = find(ExternalImportLibrarySettingKey::MonitorSpecials);
        assert_eq!(monitor_specials.value.bool_value, Some(true));
    }

    #[test]
    fn external_import_setting_derivation_skips_missing_source_signal() {
        let library_id = "anime-library";
        let root_path = "/srv/anime";
        let source_results = BTreeMap::from([
            (
                "source-a-session".to_string(),
                test_warmup_source(
                    "sonarr-a",
                    root_path,
                    Some(ArrNamingConfig {
                        rename_enabled: Some(true),
                        replace_illegal_characters: None,
                        colon_replacement_format: None,
                        standard_format: None,
                        folder_format: None,
                        season_folder_format: None,
                        specials_folder_format: None,
                    }),
                    Vec::new(),
                ),
            ),
            (
                "source-b-session".to_string(),
                test_warmup_source("sonarr-b", root_path, None, Vec::new()),
            ),
        ]);
        let mappings = HashMap::from([
            test_source_mapping("source-a-session", "sonarr-a", root_path, library_id),
            test_source_mapping("source-b-session", "sonarr-b", root_path, library_id),
        ]);
        let accumulators =
            build_external_import_library_setting_accumulators(&source_results, &mappings);

        let applications = derive_external_import_library_setting_applications(
            &accumulators,
            &source_results,
            &[],
        );
        let rename = applications
            .iter()
            .find(|application| {
                application.setting == ExternalImportLibrarySettingKey::RenameEnabled
            })
            .expect("rename application");

        assert_eq!(
            rename.disposition,
            ExternalImportLibrarySettingDisposition::Skipped
        );
        assert!(
            rename
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("did not report"))
        );
    }

    #[test]
    fn external_import_setting_derivation_reports_all_missing_source_signal() {
        let session_id = "source-session";
        let library_id = "anime-library";
        let root_path = "/srv/anime";
        let source_results = BTreeMap::from([(
            session_id.to_string(),
            test_warmup_source("sonarr-main", root_path, None, Vec::new()),
        )]);
        let mappings = HashMap::from([test_source_mapping(
            session_id,
            "sonarr-main",
            root_path,
            library_id,
        )]);
        let accumulators =
            build_external_import_library_setting_accumulators(&source_results, &mappings);

        let applications = derive_external_import_library_setting_applications(
            &accumulators,
            &source_results,
            &[],
        );
        let rename = applications
            .iter()
            .find(|application| {
                application.setting == ExternalImportLibrarySettingKey::RenameEnabled
            })
            .expect("rename application");

        assert_eq!(
            rename.disposition,
            ExternalImportLibrarySettingDisposition::Skipped
        );
        assert!(
            rename
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("did not report"))
        );
    }

    #[test]
    fn external_import_quality_profile_dominance_counts_unmapped_profiles() {
        let session_id = "source-session";
        let library_id = "anime-library";
        let root_path = "/srv/anime";
        let source_results = BTreeMap::from([(
            session_id.to_string(),
            test_warmup_source(
                "sonarr-main",
                root_path,
                None,
                vec![
                    ArrQualityProfile {
                        id: 7,
                        name: "HD 1080p".into(),
                        language: None,
                    },
                    ArrQualityProfile {
                        id: 8,
                        name: "Imported 4K".into(),
                        language: None,
                    },
                ],
            ),
        )]);
        let mappings = HashMap::from([test_source_mapping(
            session_id,
            "sonarr-main",
            root_path,
            library_id,
        )]);
        let mut accumulators =
            build_external_import_library_setting_accumulators(&source_results, &mappings);
        let mapping = mappings.values().next().expect("mapping");
        for id in 1..=3 {
            record_series_setting_sample(
                &mut accumulators,
                mapping,
                &test_arr_series(id, root_path, 7),
            );
        }
        record_series_setting_sample(
            &mut accumulators,
            mapping,
            &test_arr_series(4, root_path, 8),
        );
        let catalog_profiles = vec![QualityProfile {
            id: "hd-1080p".into(),
            name: "HD 1080p".into(),
            criteria: QualityProfileCriteria::default(),
        }];

        let applications = derive_external_import_library_setting_applications(
            &accumulators,
            &source_results,
            &catalog_profiles,
        );
        let profile = applications
            .iter()
            .find(|application| {
                application.setting == ExternalImportLibrarySettingKey::QualityProfileId
            })
            .expect("quality profile application");

        assert_eq!(
            profile.disposition,
            ExternalImportLibrarySettingDisposition::Skipped
        );
        assert!(
            profile
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("below confidence"))
        );
    }

    #[test]
    fn external_import_library_reconciliation_does_not_claim_rename_setting() {
        assert!(!is_external_import_library_auto_apply_setting(
            ExternalImportLibrarySettingKey::RenameEnabled
        ));
        assert!(is_external_import_library_auto_apply_setting(
            ExternalImportLibrarySettingKey::NfoWriteOnImport
        ));
    }

    #[test]
    fn map_download_client_marks_qbittorrent_as_supported() {
        let payload = map_download_client(
            &ArrDownloadClient {
                id: 1,
                name: "qBittorrent".into(),
                implementation: "qBittorrent".into(),
                fields: HashMap::from([
                    ("host".into(), Value::String("qb.local".into())),
                    ("port".into(), Value::String("8080".into())),
                ]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_client_type.as_deref(), Some("qbittorrent"));
        assert_eq!(payload.dedup_key, "qbittorrent:qb.local:8080");
        assert_eq!(payload.source_keys, vec!["sonarr".to_string()]);
    }

    #[test]
    fn imported_download_client_connection_config_excludes_arr_routing_categories() {
        let config = imported_download_client_connection_config(
            "nzbget",
            "nzbget.local",
            "6789",
            true,
            "/rpc",
        );

        assert_eq!(
            config.get("host"),
            Some(&Value::String("nzbget.local".into()))
        );
        assert_eq!(
            config.get("client_type"),
            Some(&Value::String("nzbget".into()))
        );
        assert!(!config.contains_key("category"));
        assert!(!config.contains_key("tvCategory"));
        assert!(!config.contains_key("movieCategory"));
    }

    #[test]
    fn map_indexer_marks_sonarr_torznab_as_supported() {
        let payload = map_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Indexer".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://torznab.example".into()),
                )]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_provider_type.as_deref(), Some("torznab"));
        assert_eq!(payload.dedup_key, "torznab:https://torznab.example");
        assert_eq!(payload.source_keys, vec!["sonarr".to_string()]);
    }

    #[test]
    fn arr_path_remap_replaces_root_prefix_and_normalizes_trailing_slashes() {
        assert_eq!(
            remap_import_path(
                Some("/arr/movies/Foo (2024)/Foo.mkv".into()),
                "/arr/movies/",
                "/srv/media/movies/"
            ),
            Some("/srv/media/movies/Foo (2024)/Foo.mkv".to_string())
        );
        assert_eq!(
            remap_import_path(
                Some(r"C:\Arr\Series\Show\Season 01\Episode.mkv".into()),
                r"c:\arr\series\",
                r"D:\Media\TV\"
            ),
            Some(r"D:\Media\TV\Show\Season 01\Episode.mkv".to_string())
        );
        assert_eq!(
            remap_import_path(
                Some("C:/Arr/Series/Show/Season 01/Episode.mkv".into()),
                r"c:\arr\series\",
                "/srv/media/tv/"
            ),
            Some("/srv/media/tv/Show/Season 01/Episode.mkv".to_string())
        );
        assert_eq!(
            remap_import_path(
                Some("/arr/movies".into()),
                "/arr/movies/",
                "/srv/media/movies/"
            ),
            Some("/srv/media/movies".to_string())
        );
        assert_eq!(
            remap_import_path(
                Some("/other/Foo.mkv".into()),
                "/arr/movies",
                "/srv/media/movies"
            ),
            Some("/other/Foo.mkv".to_string())
        );
    }

    #[test]
    fn sonarr_episode_fetch_concurrency_is_capped_per_instance_and_globally() {
        assert_eq!(SONARR_EPISODE_FETCH_CONCURRENCY_PER_INSTANCE, 16);
        assert_eq!(SONARR_ACTIVE_EPISODE_INSTANCE_CONCURRENCY, 2);
        assert_eq!(
            SONARR_EPISODE_FETCH_CONCURRENCY_PER_INSTANCE
                * SONARR_ACTIVE_EPISODE_INSTANCE_CONCURRENCY,
            32
        );
    }

    #[test]
    fn map_indexer_marks_sonarr_newznab_preset_as_generic_newznab() {
        let payload = map_indexer(
            &ArrIndexer {
                id: 1,
                name: "NZBGeek".into(),
                implementation: "Newznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://api.nzbgeek.info".into()),
                )]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_provider_type.as_deref(), Some("newznab"));
        assert_eq!(payload.dedup_key, "newznab:https://api.nzbgeek.info");
    }

    #[test]
    fn imported_indexer_config_keeps_base_url_and_api_path_separate() {
        let fields = vec![
            ConfigFieldDef {
                key: "base_url".into(),
                label: "Base URL".into(),
                field_type: ConfigFieldType::String,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: Some(ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: vec![],
                help_text: None,
            },
            ConfigFieldDef {
                key: "api_key".into(),
                label: "API Key".into(),
                field_type: ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            },
            ConfigFieldDef {
                key: "api_path".into(),
                label: "API Path".into(),
                field_type: ConfigFieldType::String,
                required: false,
                default_value: Some("/api".into()),
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            },
        ];

        let config_json = imported_indexer_config_json(
            &fields,
            "https://indexer.example",
            Some("secret"),
            Some("/api/v1"),
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&config_json).expect("config json should parse");

        assert_eq!(parsed["base_url"], "https://indexer.example");
        assert_eq!(parsed["api_key"], "secret");
        assert_eq!(parsed["api_path"], "/api/v1");
    }

    #[test]
    fn direct_prowlarr_merge_overrides_arr_key_conflicts_and_keeps_children() {
        let mut groups = HashMap::new();
        merge_prowlarr_group(
            &mut groups,
            scryer_application::external_import::DetectedProwlarrIndexer {
                base_url: "http://prowlarr.local".into(),
                api_key: Some("arr-key-a".into()),
                child_name: "Indexer A".into(),
            },
            "sonarr",
        );
        merge_prowlarr_group(
            &mut groups,
            scryer_application::external_import::DetectedProwlarrIndexer {
                base_url: "http://prowlarr.local".into(),
                api_key: Some("arr-key-b".into()),
                child_name: "Indexer B".into(),
            },
            "radarr",
        );

        merge_direct_prowlarr_group(
            &mut groups,
            "http://prowlarr.local",
            "direct-key",
            &["Indexer B".into(), "Indexer C".into()],
        );

        let group = groups
            .get(&prowlarr_dedup_key("http://prowlarr.local"))
            .expect("merged prowlarr group");
        assert_eq!(group.api_key.as_deref(), Some("direct-key"));
        assert!(!group.api_key_conflict);
        assert_eq!(group.sources, vec!["sonarr", "radarr", "prowlarr"]);
        assert_eq!(
            group.child_names,
            vec![
                "Indexer A".to_string(),
                "Indexer B".to_string(),
                "Indexer C".to_string()
            ]
        );
    }

    #[test]
    fn arr_keys_cannot_degrade_a_direct_prowlarr_key_merged_first() {
        // Preview merges the direct (operator-verified) group BEFORE the arr
        // loop; a differing arr-reported key must neither replace the direct
        // key nor flag a conflict.
        let mut groups = HashMap::new();
        merge_direct_prowlarr_group(
            &mut groups,
            "http://prowlarr.local",
            "direct-key",
            &["Indexer A".into()],
        );
        merge_prowlarr_group(
            &mut groups,
            scryer_application::external_import::DetectedProwlarrIndexer {
                base_url: "http://prowlarr.local".into(),
                api_key: Some("stale-arr-key".into()),
                child_name: "Indexer B".into(),
            },
            "sonarr",
        );

        let group = groups
            .get(&prowlarr_dedup_key("http://prowlarr.local"))
            .expect("merged prowlarr group");
        assert_eq!(group.api_key.as_deref(), Some("direct-key"));
        assert!(!group.api_key_conflict);
        assert!(!group.requires_api_key_override());
        assert_eq!(group.sources, vec!["prowlarr", "sonarr"]);
        assert_eq!(
            group.child_names,
            vec!["Indexer A".to_string(), "Indexer B".to_string()]
        );
    }

    #[test]
    fn linked_prowlarr_proxy_detection_accepts_torznab_without_api_path() {
        let detected = detect_imported_prowlarr_proxy_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Child".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("http://prowlarr.local/12345".into()),
                )]),
            },
            Some("http://prowlarr.local"),
        )
        .expect("linked prowlarr proxy");

        assert_eq!(detected.base_url, "http://prowlarr.local");
        assert_eq!(detected.child_name, "Torrent Child");
    }

    #[test]
    fn direct_linked_prowlarr_detection_does_not_match_other_parents() {
        let detected = detect_imported_prowlarr_proxy_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Child".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([
                    (
                        "baseUrl".into(),
                        Value::String("http://other-prowlarr.local/12345".into()),
                    ),
                    ("apiPath".into(), Value::String("/api".into())),
                ]),
            },
            Some("http://prowlarr.local"),
        );

        assert!(detected.is_none());
    }
}
