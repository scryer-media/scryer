use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_application::subtitles::scoring::{
    MOVIE_WEIGHTS, SERIES_WEIGHTS, SubtitleScoreKind, compute_verified_score,
    normalized_score_percent,
};
use scryer_application::subtitles::{
    SubtitleFile, SubtitleMatch, SubtitleMediaKind, SubtitleQuery,
};
use scryer_application::{
    AppError, AppResult, ArchiveExtractorPluginProvider, ParsedReleaseMetadata,
    SubtitleGenerationInput, SubtitleProviderClient, SubtitleProviderValidationResult,
    parse_release_metadata,
};
use scryer_domain::{PluginHostBindingId, SubtitleProviderConfig};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginSubtitleCommand,
    PluginSubtitleCommandResult,
};
use scryer_plugin_sdk::PluginResult;

use crate::blocking::run_blocking_plugin_call;
use crate::legacy_runtime::{LegacyPlugin, LegacyPluginSpec};
use crate::loader::{allowed_hosts_for_descriptor, parse_config_json_entries};
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking, PreopenSpec};
use crate::wasmtime_host::{SubtitleComponentInvocation, process_subtitle_component};
use crate::types::{
    EXPORT_SUBTITLE_DOWNLOAD, EXPORT_SUBTITLE_GENERATE, EXPORT_SUBTITLE_SEARCH,
    EXPORT_VALIDATE_CONFIG, PluginDescriptor, SubtitleMatchHint, SubtitleMatchHintKind,
    SubtitlePluginCandidate, SubtitlePluginDownloadRequest, SubtitlePluginDownloadResponse,
    SubtitlePluginGenerateRequest, SubtitlePluginGenerateResponse, SubtitlePluginSearchRequest,
    SubtitlePluginSearchResponse, SubtitlePluginValidateConfigRequest,
    SubtitlePluginValidateConfigResponse, SubtitleProviderMode, SubtitleQueryMediaKind,
    SubtitleValidateConfigStatus, decode_plugin_result, host_binding_to_domain,
};
use crate::wasmtime_host::command_host::CommandHost;

const GENERATOR_MAX_INPUT_SIZE_BYTES: i64 = 512 * 1024 * 1024;
const GENERATOR_MAX_DURATION_SECONDS: i64 = 4 * 60 * 60;
const GUEST_INPUT_ROOT: &str = "/input";
const SUBTITLE_PLUGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Which runtime serves this provider's operations.
///
/// The two arms carry different state because their models differ: a legacy
/// reactor is instantiated once and re-entered under a mutex, while a component
/// is instance-per-request and so retains only the spec it will instantiate
/// from. Everything else about the client — config, host bindings, the archive
/// provider threaded into the [`CommandHost`] — is shared.
enum SubtitleRuntime {
    Legacy(Arc<Mutex<LegacyPlugin>>),
    Component(PluginInstanceSpec),
}

pub struct WasmSubtitleClient {
    runtime: SubtitleRuntime,
    wasm_bytes: Arc<Vec<u8>>,
    descriptor: PluginDescriptor,
    provider_name: String,
    config_name: String,
    config_json: String,
    host_bindings: HashMap<PluginHostBindingId, String>,
    missing_host_bindings: Vec<PluginHostBindingId>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
}

impl WasmSubtitleClient {
    pub fn new_with_archive_provider(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        config: SubtitleProviderConfig,
        host_bindings: HashMap<PluginHostBindingId, String>,
        archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
    ) -> Result<Self, AppError> {
        let missing_host_bindings = missing_host_bindings(&descriptor, &host_bindings);
        let spec = build_subtitle_spec(
            &wasm_bytes,
            &descriptor,
            &config.config_json,
            &host_bindings,
            None,
            archive_provider.clone(),
        )?;
        // Classify from the artifact, not the descriptor: a subtitle descriptor
        // is identical whether the artifact is a legacy reactor or a
        // `scryer:subtitle/subtitle-provider@1.0.0` component.
        let backing = PluginRuntimeBacking::for_artifact(&descriptor, &wasm_bytes)
            .map_err(AppError::Repository)?;
        let runtime = if backing == PluginRuntimeBacking::WasmtimeSubtitleComponent {
            SubtitleRuntime::Component(component_instance_spec(&spec))
        } else {
            // Every non-component backing keeps its existing path: the legacy
            // reactor is instantiated eagerly so a broken artifact fails at
            // provider construction rather than at first search.
            SubtitleRuntime::Legacy(Arc::new(Mutex::new(
                LegacyPlugin::instantiate(spec).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to compile subtitle provider plugin for {}: {error}",
                        config.name
                    ))
                })?,
            )))
        };

        Ok(Self {
            runtime,
            wasm_bytes: Arc::new(wasm_bytes),
            descriptor,
            provider_name: config.provider_type,
            config_name: config.name,
            config_json: config.config_json,
            host_bindings,
            missing_host_bindings,
            archive_provider,
        })
    }

    fn missing_host_binding_error(&self) -> AppError {
        let binding_names = self
            .missing_host_bindings
            .iter()
            .map(|binding| binding.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        AppError::Validation(format!(
            "subtitle provider '{}' is missing required host bindings: {binding_names}",
            self.provider_name
        ))
    }

    fn missing_host_binding_validation(&self) -> SubtitleProviderValidationResult {
        SubtitleProviderValidationResult {
            status: subtitle_validate_status_string(
                SubtitleValidateConfigStatus::MissingHostBinding,
            ),
            message: Some(self.missing_host_binding_error().to_string()),
            retry_after_seconds: None,
        }
    }

    /// The component spec for this provider, or `None` on a legacy artifact.
    fn component_spec(&self) -> Option<&PluginInstanceSpec> {
        match &self.runtime {
            SubtitleRuntime::Component(spec) => Some(spec),
            SubtitleRuntime::Legacy(_) => None,
        }
    }

    /// The instantiated legacy reactor. Reached only after `component_spec()`
    /// has returned `None`, so the `else` is unreachable in practice and exists
    /// to keep the two runtimes from sharing a panic.
    fn legacy_plugin(&self) -> AppResult<Arc<Mutex<LegacyPlugin>>> {
        match &self.runtime {
            SubtitleRuntime::Legacy(plugin) => Ok(Arc::clone(plugin)),
            SubtitleRuntime::Component(_) => Err(AppError::Repository(format!(
                "subtitle provider plugin {} is a component and has no legacy exports",
                self.descriptor.id
            ))),
        }
    }

    /// Run one `PluginSubtitleCommand` through the component world and return
    /// its family-matched result.
    async fn invoke_component(
        &self,
        spec: &PluginInstanceSpec,
        command: PluginSubtitleCommand,
        operation: &'static str,
    ) -> AppResult<PluginSubtitleCommandResult> {
        let response = process_subtitle_component(
            spec,
            &PluginCommandRequest::new(PluginCommand::Subtitle(command)),
            SubtitleComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await?;
        match response.response {
            PluginCommandResult::Subtitle(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "subtitle provider plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }
}

/// Reuse the legacy spec's sandbox decisions for a component invocation.
///
/// The provider's config, allowed hosts, timeout, preopens and — critically —
/// the [`CommandHost`] carrying the archive provider are all decided once in
/// [`build_subtitle_spec`]. The component path re-projects that same spec
/// rather than making a second set of decisions that could drift.
fn component_instance_spec(spec: &LegacyPluginSpec) -> PluginInstanceSpec {
    PluginInstanceSpec {
        wasm: Arc::clone(&spec.wasm),
        preopens: spec.preopens.clone(),
        timeout: spec.timeout,
        memory_max_bytes: spec.memory_max_bytes,
        command_host: spec.command_host.clone(),
    }
}

/// Unwrap a `PluginResult` from a component subtitle operation, mirroring what
/// `decode_plugin_result` does for the legacy JSON envelope.
fn decode_component_result<T>(result: PluginResult<T>, context: &str) -> AppResult<T> {
    match result {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(error) => Err(AppError::Repository(format!(
            "{context}: plugin error {:?}: {}",
            error.code, error.public_message
        ))),
    }
}

fn wrong_operation_error(plugin_id: &str, expected: &str) -> AppError {
    AppError::Repository(format!(
        "subtitle provider plugin {plugin_id} answered a {expected} request with another operation"
    ))
}

#[async_trait]
impl SubtitleProviderClient for WasmSubtitleClient {
    async fn search(&self, query: &SubtitleQuery) -> AppResult<Vec<SubtitleMatch>> {
        if !self.missing_host_bindings.is_empty() {
            return Err(self.missing_host_binding_error());
        }

        let request = SubtitlePluginSearchRequest {
            media_kind: map_media_kind(query.media_kind),
            facet: query.facet.clone(),
            file_hash: query.file_hash.clone(),
            imdb_id: query.imdb_id.clone(),
            series_imdb_id: query.series_imdb_id.clone(),
            title: query.title.clone(),
            title_aliases: query.title_aliases.clone(),
            title_candidates: query.title_candidates.clone(),
            year: query.year,
            season: query.season,
            episode: query.episode,
            absolute_episode: query.absolute_episode,
            external_ids: query.external_ids.clone(),
            languages: query.languages.clone(),
            release_group: query.release_group.clone(),
            source: query.source.clone(),
            video_codec: query.video_codec.clone(),
            audio_codec: query.audio_codec.clone(),
            resolution: query.resolution.clone(),
            hearing_impaired: query.hearing_impaired,
            include_ai_translated: query.include_ai_translated,
            include_machine_translated: query.include_machine_translated,
        };
        let response: SubtitlePluginSearchResponse = if let Some(spec) = self.component_spec() {
            let result = self
                .invoke_component(spec, PluginSubtitleCommand::Search(request), "Search")
                .await?;
            let PluginSubtitleCommandResult::Search(result) = result else {
                return Err(wrong_operation_error(&self.descriptor.id, "search"));
            };
            decode_component_result(result, EXPORT_SUBTITLE_SEARCH)?
        } else {
            let input = serde_json::to_string(&request).map_err(|error| {
                AppError::Repository(format!(
                    "failed to serialize subtitle search request: {error}"
                ))
            })?;

            let plugin = self.legacy_plugin()?;
            let output =
                run_blocking_plugin_call(SUBTITLE_PLUGIN_TIMEOUT, "subtitle plugin", move || {
                    let mut guard = plugin.lock().map_err(|error| {
                        AppError::Repository(format!("plugin mutex poisoned: {error}"))
                    })?;
                    guard
                        .call_string(EXPORT_SUBTITLE_SEARCH, &input)
                        .map_err(|error| {
                            plugin_call_error(&format!("{EXPORT_SUBTITLE_SEARCH}()"), error)
                        })
                })
                .await?;

            decode_plugin_result(&output, EXPORT_SUBTITLE_SEARCH)?
        };

        let mut results = response
            .results
            .into_iter()
            .map(|candidate| map_candidate_to_match(&self.provider_name, query, candidate))
            .collect::<Vec<_>>();
        results.sort_by_key(|result| std::cmp::Reverse(result.score));
        Ok(results)
    }

    async fn download(&self, provider_file_id: &str) -> AppResult<SubtitleFile> {
        if !self.missing_host_bindings.is_empty() {
            return Err(self.missing_host_binding_error());
        }

        let request = SubtitlePluginDownloadRequest {
            provider_file_id: provider_file_id.to_string(),
        };

        let response: SubtitlePluginDownloadResponse = if let Some(spec) = self.component_spec() {
            let result = self
                .invoke_component(spec, PluginSubtitleCommand::Download(request), "Download")
                .await?;
            let PluginSubtitleCommandResult::Download(result) = result else {
                return Err(wrong_operation_error(&self.descriptor.id, "download"));
            };
            decode_component_result(result, EXPORT_SUBTITLE_DOWNLOAD)?
        } else {
            let input = serde_json::to_string(&request).map_err(|error| {
                AppError::Repository(format!(
                    "failed to serialize subtitle download request: {error}"
                ))
            })?;

            let plugin = self.legacy_plugin()?;
            let output =
                run_blocking_plugin_call(SUBTITLE_PLUGIN_TIMEOUT, "subtitle plugin", move || {
                    let mut guard = plugin.lock().map_err(|error| {
                        AppError::Repository(format!("plugin mutex poisoned: {error}"))
                    })?;
                    guard
                        .call_string(EXPORT_SUBTITLE_DOWNLOAD, &input)
                        .map_err(|error| {
                            plugin_call_error(&format!("{EXPORT_SUBTITLE_DOWNLOAD}()"), error)
                        })
                })
                .await?;

            decode_plugin_result(&output, EXPORT_SUBTITLE_DOWNLOAD)?
        };

        Ok(SubtitleFile {
            content: BASE64.decode(response.content_base64).map_err(|error| {
                AppError::Repository(format!(
                    "subtitle plugin returned invalid base64 download payload: {error}"
                ))
            })?,
            format: response.format,
            filename: response.filename,
            content_type: response.content_type,
        })
    }

    async fn validate_connection(&self) -> AppResult<SubtitleProviderValidationResult> {
        if !self.missing_host_bindings.is_empty() {
            return Ok(self.missing_host_binding_validation());
        }

        let request = SubtitlePluginValidateConfigRequest {
            config_instance_name: Some(self.config_name.clone()),
        };

        let response: SubtitlePluginValidateConfigResponse = if let Some(spec) =
            self.component_spec()
        {
            let result = self
                .invoke_component(
                    spec,
                    PluginSubtitleCommand::ValidateConfig(request),
                    "ValidateConfig",
                )
                .await?;
            let PluginSubtitleCommandResult::ValidateConfig(result) = result else {
                return Err(wrong_operation_error(&self.descriptor.id, "validate config"));
            };
            decode_component_result(result, EXPORT_VALIDATE_CONFIG)?
        } else {
            let input = serde_json::to_string(&request).map_err(|error| {
                AppError::Repository(format!(
                    "failed to serialize subtitle validate request: {error}"
                ))
            })?;

            let plugin = self.legacy_plugin()?;
            let output =
                run_blocking_plugin_call(SUBTITLE_PLUGIN_TIMEOUT, "subtitle plugin", move || {
                    let mut guard = plugin.lock().map_err(|error| {
                        AppError::Repository(format!("plugin mutex poisoned: {error}"))
                    })?;
                    guard
                        .call_string(EXPORT_VALIDATE_CONFIG, &input)
                        .map_err(|error| {
                            plugin_call_error(&format!("{EXPORT_VALIDATE_CONFIG}()"), error)
                        })
                })
                .await?;

            decode_plugin_result(&output, EXPORT_VALIDATE_CONFIG)?
        };

        Ok(SubtitleProviderValidationResult {
            status: subtitle_validate_status_string(response.status),
            message: response.message,
            retry_after_seconds: response.retry_after_seconds,
        })
    }

    async fn generate(&self, request: &SubtitleGenerationInput) -> AppResult<SubtitleFile> {
        if !self.missing_host_bindings.is_empty() {
            return Err(self.missing_host_binding_error());
        }

        let Some(subtitle) = self.descriptor.subtitle() else {
            return Err(AppError::Repository(format!(
                "subtitle provider '{}' does not declare subtitle capabilities",
                self.provider_name
            )));
        };
        if subtitle.capabilities.mode != SubtitleProviderMode::Generator {
            return Err(AppError::Repository(format!(
                "subtitle provider '{}' does not support subtitle generation",
                self.provider_name
            )));
        }
        if request.size_bytes > GENERATOR_MAX_INPUT_SIZE_BYTES {
            return Err(AppError::Validation(format!(
                "subtitle generator input exceeds size limit ({} > {})",
                request.size_bytes, GENERATOR_MAX_INPUT_SIZE_BYTES
            )));
        }
        if request.duration_seconds > GENERATOR_MAX_DURATION_SECONDS {
            return Err(AppError::Validation(format!(
                "subtitle generator input exceeds duration limit ({} > {})",
                request.duration_seconds, GENERATOR_MAX_DURATION_SECONDS
            )));
        }

        let guest_input_path = guest_input_path(&request.input_path)?;
        let spec = build_subtitle_spec(
            self.wasm_bytes.as_slice(),
            &self.descriptor,
            &self.config_json,
            &self.host_bindings,
            Some((request.input_path.as_path(), GUEST_INPUT_ROOT)),
            self.archive_provider.clone(),
        )?;
        let generate_request = SubtitlePluginGenerateRequest {
            media_kind: match request.media_kind.as_str() {
                "episode" => SubtitleQueryMediaKind::Episode,
                _ => SubtitleQueryMediaKind::Movie,
            },
            facet: request.facet.clone(),
            input: crate::types::SubtitleGeneratorInputRef {
                path: guest_input_path,
                mime_type: request.mime_type.clone(),
                duration_seconds: request.duration_seconds,
                size_bytes: request.size_bytes,
                checksum: request.checksum.clone(),
            },
            languages: request.languages.clone(),
        };

        // A generator invocation is the one subtitle operation with filesystem
        // authority, and it is instance-per-request on both runtimes: the spec
        // built above carries the read-only input preopen, so the component
        // path re-projects *this* spec rather than the client's ambient one.
        let response: SubtitlePluginGenerateResponse = if self.component_spec().is_some() {
            let spec = component_instance_spec(&spec);
            let result = self
                .invoke_component(
                    &spec,
                    PluginSubtitleCommand::Generate(generate_request),
                    "Generate",
                )
                .await?;
            let PluginSubtitleCommandResult::Generate(result) = result else {
                return Err(wrong_operation_error(&self.descriptor.id, "generate"));
            };
            decode_component_result(result, EXPORT_SUBTITLE_GENERATE)?
        } else {
            let input = serde_json::to_string(&generate_request).map_err(|error| {
                AppError::Repository(format!(
                    "failed to serialize subtitle generate request: {error}"
                ))
            })?;

            let output = run_blocking_plugin_call(
                SUBTITLE_PLUGIN_TIMEOUT,
                "subtitle generator plugin",
                move || {
                    let mut plugin = LegacyPlugin::instantiate(spec).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to compile subtitle generator plugin: {error}"
                        ))
                    })?;
                    plugin
                        .call_string(EXPORT_SUBTITLE_GENERATE, &input)
                        .map_err(|error| {
                            plugin_call_error(&format!("{EXPORT_SUBTITLE_GENERATE}()"), error)
                        })
                },
            )
            .await?;

            decode_plugin_result(&output, EXPORT_SUBTITLE_GENERATE)?
        };

        Ok(SubtitleFile {
            content: BASE64.decode(response.content_base64).map_err(|error| {
                AppError::Repository(format!(
                    "subtitle plugin returned invalid base64 generation payload: {error}"
                ))
            })?,
            format: response.format,
            filename: None,
            content_type: None,
        })
    }

    fn name(&self) -> &str {
        self.provider_name.as_str()
    }
}

fn build_subtitle_spec(
    wasm_bytes: &[u8],
    descriptor: &PluginDescriptor,
    config_json: &str,
    host_bindings: &HashMap<PluginHostBindingId, String>,
    allowed_path: Option<(&Path, &str)>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<LegacyPluginSpec> {
    let mut spec = LegacyPluginSpec::new(wasm_bytes.to_vec(), descriptor.id.clone());
    spec.allowed_hosts = allowed_hosts_for_descriptor(descriptor, None, Some(config_json));
    spec.timeout = std::time::Duration::from_secs(30);

    match parse_config_json_entries(config_json) {
        Ok(entries) => {
            for (key, value) in entries {
                spec.config.insert(key, value);
            }
        }
        Err(error) => {
            return Err(AppError::Validation(format!(
                "subtitle provider config_json must be a JSON object: {error}"
            )));
        }
    }

    for field in descriptor.config_fields() {
        if let Some(binding) = field.host_binding
            && let Some(value) = host_bindings.get(&host_binding_to_domain(binding))
        {
            spec.config.insert(field.key.clone(), value.clone());
        }
    }

    if let Some((host_path, guest_root)) = allowed_path {
        spec.preopens
            .push(PreopenSpec::read_only(host_path.to_path_buf(), guest_root));
    }

    spec.command_host = CommandHost::with_archive_provider(
        descriptor.id.clone(),
        spec.config.clone(),
        spec.allowed_hosts.clone(),
        spec.timeout,
        spec.memory_max_bytes.map(|value| value as u64),
        archive_provider,
    );

    Ok(spec)
}

fn missing_host_bindings(
    descriptor: &PluginDescriptor,
    host_bindings: &HashMap<PluginHostBindingId, String>,
) -> Vec<PluginHostBindingId> {
    let mut seen = HashSet::new();
    let mut missing = Vec::new();
    for field in descriptor.config_fields() {
        let Some(binding) = field.host_binding else {
            continue;
        };
        let domain_binding = host_binding_to_domain(binding);
        if host_bindings.contains_key(&domain_binding) || !seen.insert(domain_binding) {
            continue;
        }
        missing.push(domain_binding);
    }
    missing
}

fn subtitle_validate_status_string(status: SubtitleValidateConfigStatus) -> String {
    match status {
        SubtitleValidateConfigStatus::Valid => "valid",
        SubtitleValidateConfigStatus::InvalidConfig => "invalid_config",
        SubtitleValidateConfigStatus::AuthFailed => "auth_failed",
        SubtitleValidateConfigStatus::RateLimited => "rate_limited",
        SubtitleValidateConfigStatus::Unreachable => "unreachable",
        SubtitleValidateConfigStatus::Unsupported => "unsupported",
        SubtitleValidateConfigStatus::MissingHostBinding => "missing_host_binding",
    }
    .to_string()
}

fn plugin_call_error(operation: &str, error: AppError) -> AppError {
    AppError::Repository(format!("subtitle plugin {operation} failed: {error}"))
}

fn map_media_kind(kind: SubtitleMediaKind) -> SubtitleQueryMediaKind {
    match kind {
        SubtitleMediaKind::Movie => SubtitleQueryMediaKind::Movie,
        SubtitleMediaKind::Episode => SubtitleQueryMediaKind::Episode,
    }
}

fn map_candidate_to_match(
    provider_name: &str,
    query: &SubtitleQuery,
    candidate: SubtitlePluginCandidate,
) -> SubtitleMatch {
    let mut matches = HashSet::new();

    for hint in &candidate.match_hints {
        apply_match_hint(query, &mut matches, hint);
    }

    if let Some(parsed_release) = candidate
        .release_info
        .as_deref()
        .map(parse_release_metadata)
    {
        if let Some(year) = parsed_release.year
            && query.year == Some(year)
        {
            matches.insert("year".to_string());
        }
        if release_metadata_title_matches(&parsed_release, query) {
            match query.media_kind {
                SubtitleMediaKind::Movie => {
                    matches.insert("title".to_string());
                }
                SubtitleMediaKind::Episode => {
                    matches.insert("series".to_string());
                }
            }
        }
        if release_group_matches(
            query.release_group.as_deref(),
            parsed_release.release_group.as_deref(),
        ) {
            matches.insert("release_group".to_string());
        }
        if source_matches(
            query.source.as_deref(),
            parsed_release.source.as_ref().map(|source| source.as_str()),
        ) {
            matches.insert("source".to_string());
        }
        if resolution_matches(
            query.resolution.as_deref(),
            parsed_release.quality.as_deref(),
        ) {
            matches.insert("resolution".to_string());
        }
        if video_codec_matches(
            query.video_codec.as_deref(),
            parsed_release
                .video_codec
                .as_ref()
                .map(scryer_application::VideoCodec::as_str),
        ) {
            matches.insert("video_codec".to_string());
        }
        if audio_codec_matches(query.audio_codec.as_deref(), &parsed_release) {
            matches.insert("audio_codec".to_string());
        }
    }

    if let Some(preferred_hi) = query.hearing_impaired
        && preferred_hi == candidate.hearing_impaired
    {
        matches.insert("hearing_impaired".to_string());
    }

    let (weights, kind) = match query.media_kind {
        SubtitleMediaKind::Movie => (MOVIE_WEIGHTS.weights(), SubtitleScoreKind::Movie),
        SubtitleMediaKind::Episode => (SERIES_WEIGHTS.weights(), SubtitleScoreKind::Episode),
    };
    let score = compute_verified_score(&weights, kind, &matches, query.season == Some(0));
    let score_percent = normalized_score_percent(kind, score);

    SubtitleMatch {
        provider: provider_name.to_string(),
        provider_file_id: candidate.provider_file_id,
        language: candidate.language,
        release_info: candidate.release_info,
        score,
        score_percent,
        hearing_impaired: candidate.hearing_impaired,
        forced: candidate.forced,
        ai_translated: candidate.ai_translated,
        machine_translated: candidate.machine_translated,
        uploader: candidate.uploader,
        download_count: candidate.download_count,
        hash_matched: matches.contains("hash"),
    }
}

fn apply_match_hint(
    query: &SubtitleQuery,
    matches: &mut HashSet<String>,
    hint: &SubtitleMatchHint,
) {
    match hint.kind {
        SubtitleMatchHintKind::Hash => {
            matches.insert("hash".to_string());
        }
        SubtitleMatchHintKind::ImdbId => {
            matches.insert("imdb_id".to_string());
        }
        SubtitleMatchHintKind::SeriesImdbId => {
            matches.insert("series_imdb_id".to_string());
        }
        SubtitleMatchHintKind::ExternalId => {
            if let Some(value) = hint.value.as_deref()
                && external_id_matches(query, value)
            {
                matches.insert("external_id".to_string());
            }
        }
        SubtitleMatchHintKind::AbsoluteEpisode => {
            let hint_matches = hint
                .value
                .as_deref()
                .and_then(|value| value.trim().parse::<i32>().ok())
                .is_none_or(|value| query.absolute_episode == Some(value));
            if query.absolute_episode.is_some() && hint_matches {
                matches.insert("absolute_episode".to_string());
                matches.insert("episode".to_string());
            }
        }
        SubtitleMatchHintKind::Title => match query.media_kind {
            SubtitleMediaKind::Movie => {
                matches.insert("title".to_string());
            }
            SubtitleMediaKind::Episode => {
                matches.insert("series".to_string());
            }
        },
        SubtitleMatchHintKind::SeasonEpisode => {
            if query.season.is_some() {
                matches.insert("season".to_string());
            }
            if query.episode.is_some() {
                matches.insert("episode".to_string());
            }
        }
        SubtitleMatchHintKind::Release | SubtitleMatchHintKind::Language => {}
    }
}

fn external_id_matches(query: &SubtitleQuery, hint_value: &str) -> bool {
    let Some((source, value)) = hint_value.split_once(':') else {
        return false;
    };
    let source = source.trim().to_ascii_lowercase();
    let value = value.trim();
    if source.is_empty() || value.is_empty() {
        return false;
    }

    query
        .external_ids
        .get(&source)
        .is_some_and(|values| values.iter().any(|candidate| candidate == value))
}

fn collect_title_candidates(query: &SubtitleQuery) -> Vec<String> {
    let mut candidates =
        Vec::with_capacity(query.title_candidates.len() + query.title_aliases.len() + 1);
    let mut seen = HashSet::new();

    for candidate in query
        .title_candidates
        .iter()
        .chain(std::iter::once(&query.title))
        .chain(query.title_aliases.iter())
    {
        let normalized = normalize_title_for_match(candidate);
        if normalized.is_empty() || !seen.insert(normalized) {
            continue;
        }
        candidates.push(candidate.trim().to_string());
    }

    candidates
}

fn normalize_title_for_match(title: &str) -> String {
    let normalized = title
        .chars()
        .fold(String::with_capacity(title.len()), |mut acc, ch| {
            if ch.is_alphanumeric() {
                acc.push(ch.to_ascii_lowercase());
            } else if ch == '&' {
                acc.push_str(" and ");
            } else if ch.is_whitespace() || matches!(ch, '.' | '-' | '_') {
                acc.push(' ');
            }
            acc
        });

    collapse_title_initialisms(normalized.split_whitespace().collect::<Vec<_>>()).join(" ")
}

fn collapse_title_initialisms(tokens: Vec<&str>) -> Vec<String> {
    let mut collapsed = Vec::with_capacity(tokens.len());
    let mut idx = 0;

    while idx < tokens.len() {
        if tokens[idx].len() == 1 && tokens[idx].chars().all(|ch| ch.is_ascii_alphabetic()) {
            let start = idx;
            while idx < tokens.len()
                && tokens[idx].len() == 1
                && tokens[idx].chars().all(|ch| ch.is_ascii_alphabetic())
            {
                idx += 1;
            }

            if idx - start > 1 {
                collapsed.push(tokens[start..idx].concat());
                continue;
            }

            idx = start;
        }

        collapsed.push(tokens[idx].to_string());
        idx += 1;
    }

    collapsed
}

fn release_metadata_title_matches(parsed: &ParsedReleaseMetadata, query: &SubtitleQuery) -> bool {
    let mut release_titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };
    if release_titles.is_empty() {
        release_titles.push(parsed.normalized_title.clone());
    }

    let candidate_titles = collect_title_candidates(query);
    release_titles.into_iter().any(|release_title| {
        let normalized_release = normalize_title_for_match(&release_title);
        candidate_titles
            .iter()
            .any(|candidate| normalize_title_for_match(candidate) == normalized_release)
    })
}

fn normalize_compare_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_uppercase())
        .collect()
}

fn release_group_matches(left: Option<&str>, right: Option<&str>) -> bool {
    const EQUIVALENT_RELEASE_GROUPS: &[&[&str]] = &[
        &["FRAMESTOR", "W4NK3R", "BHDSTUDIO"],
        &["LOL", "DIMENSION"],
        &["ASAP", "IMMERSE", "FLEET"],
        &["AVS", "SVA"],
    ];

    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };

    let left = normalize_compare_token(left);
    let right = normalize_compare_token(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }

    EQUIVALENT_RELEASE_GROUPS.iter().any(|group| {
        let members: HashSet<String> = group
            .iter()
            .map(|member| normalize_compare_token(member))
            .collect();
        members.contains(&left) && members.contains(&right)
    })
}

fn source_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_source_family(left) == normalize_source_family(right)
}

fn resolution_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_compare_token(left) == normalize_compare_token(right)
}

fn normalize_video_codec(value: &str) -> String {
    match normalize_compare_token(value).as_str() {
        "H264" | "X264" | "AVC" => "H264".to_string(),
        "H265" | "X265" | "HEVC" => "H265".to_string(),
        "XVID" => "XVID".to_string(),
        "AV1" => "AV1".to_string(),
        other => other.to_string(),
    }
}

fn video_codec_matches(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    normalize_video_codec(left) == normalize_video_codec(right)
}

fn normalize_audio_codec(value: &str) -> String {
    let token = strip_audio_channel_suffix(&normalize_compare_token(value));
    match token.as_str() {
        "DDP"
        | "DDPLUS"
        | "DDPLUSATMOS"
        | "EAC3"
        | "EAC3ATMOS"
        | "EC3"
        | "DOLBYDIGITALPLUS"
        | "DOLBYDIGITALPLUSATMOS" => "DDP".to_string(),
        "DD" | "AC3" | "DOLBYDIGITAL" => "DD".to_string(),
        "AAC" | "AACLC" | "HEAAC" => "AAC".to_string(),
        "FLAC" => "FLAC".to_string(),
        "DTS" | "DTSHD" | "DTSHDMA" | "DTSMA" | "DTSX" | "DTSHDHRA" => "DTS".to_string(),
        "TRUEHD" | "TRUEHDATMOS" | "DOLBYTRUEHD" | "DOLBYTRUEHDATMOS" => "TRUEHD".to_string(),
        other if other.starts_with("AAC") => "AAC".to_string(),
        other if other.starts_with("DTS") => "DTS".to_string(),
        other if other.starts_with("FLAC") => "FLAC".to_string(),
        other if other.starts_with("OPUS") => "OPUS".to_string(),
        other if other.starts_with("VORBIS") => "VORBIS".to_string(),
        other if other.starts_with("LPCM") || other.starts_with("PCM") => "PCM".to_string(),
        other => other.to_string(),
    }
}

fn normalize_source_family(value: &str) -> String {
    match normalize_compare_token(value).as_str() {
        "WEB" | "WEBDL" | "WEBRIP" | "WEBHD" | "WEBCAP" => "WEB".to_string(),
        "HDTV" | "SDTV" | "AHDTV" | "ULTRAHDTV" => "TV".to_string(),
        "SATRIP" | "DVB" | "PPV" | "DIGITALTV" => "AIR".to_string(),
        "HDDVD" | "BLURAY" | "BLURAYREMUX" | "BD" | "BDMV" | "BDRIP" | "BRRIP" | "UHDBLURAY"
        | "ULTRAHDBLURAY" => "DISKHD".to_string(),
        "DVD" | "DVDRIP" | "VHS" => "DISKSD".to_string(),
        other => other.to_string(),
    }
}

fn strip_audio_channel_suffix(token: &str) -> String {
    const CHANNEL_SUFFIXES: &[&str] = &["10", "20", "51", "61", "71", "81"];

    CHANNEL_SUFFIXES
        .iter()
        .find_map(|suffix| token.strip_suffix(suffix))
        .filter(|stripped| !stripped.is_empty())
        .unwrap_or(token)
        .to_string()
}

fn audio_codec_matches(left: Option<&str>, parsed: &ParsedReleaseMetadata) -> bool {
    let Some(left) = left else {
        return false;
    };
    let wanted = normalize_audio_codec(left);

    if let Some(audio) = parsed.audio.as_ref()
        && normalize_audio_codec(audio.as_str()) == wanted
    {
        return true;
    }

    parsed
        .audio_codecs
        .iter()
        .any(|codec| normalize_audio_codec(codec.as_str()) == wanted)
}

fn guest_input_path(input_path: &Path) -> AppResult<PathBuf> {
    let file_name = input_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "subtitle generator input path '{}' has no file name",
            input_path.display()
        ))
    })?;
    Ok(Path::new(GUEST_INPUT_ROOT).join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_query() -> SubtitleQuery {
        SubtitleQuery {
            media_kind: SubtitleMediaKind::Movie,
            facet: Some("movie".into()),
            file_hash: None,
            imdb_id: Some("tt2024544".into()),
            series_imdb_id: None,
            title: "Movie Title".into(),
            title_aliases: vec![],
            title_candidates: vec![],
            year: Some(2024),
            season: None,
            episode: None,
            absolute_episode: None,
            external_ids: Default::default(),
            languages: vec!["eng".into()],
            release_group: Some("GROUP".into()),
            source: Some("WEB".into()),
            video_codec: Some("H.264".into()),
            audio_codec: Some("EAC3 Atmos".into()),
            resolution: Some("1080p".into()),
            hearing_impaired: Some(false),
            include_ai_translated: false,
            include_machine_translated: false,
        }
    }

    #[test]
    fn source_matching_uses_bazarr_source_families() {
        assert!(source_matches(Some("WEB"), Some("WEB-DL")));
        assert!(source_matches(Some("BluRay"), Some("Ultra HD Blu-ray")));
        assert!(source_matches(Some("HDTV"), Some("SDTV")));
        assert!(!source_matches(Some("WEB"), Some("BluRay")));
    }

    #[test]
    fn title_normalization_handles_trailing_single_letter_token() {
        assert_eq!(normalize_title_for_match("Mystery I"), "mystery i");
        assert_eq!(normalize_title_for_match("Mystery U.S.A."), "mystery usa");
    }

    #[test]
    fn audio_matching_accepts_media_analysis_and_release_name_forms() {
        let parsed = parse_release_metadata("Movie.Title.2024.1080p.WEB-DL.DDP5.1.H.264-GROUP");

        assert!(audio_codec_matches(Some("EAC3 Atmos"), &parsed));
        assert!(audio_codec_matches(Some("Dolby Digital Plus"), &parsed));
        assert!(!audio_codec_matches(Some("TrueHD"), &parsed));
    }

    #[test]
    fn plugin_scoring_uses_bazarr_metadata_fields() {
        let query = movie_query();
        let stronger = map_candidate_to_match(
            "opensubtitles",
            &query,
            SubtitlePluginCandidate {
                provider_file_id: "1".into(),
                language: "eng".into(),
                release_info: Some("Movie.Title.2024.1080p.WEB-DL.DDP5.1.H.264-GROUP".into()),
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                match_hints: vec![],
            },
        );
        let weaker = map_candidate_to_match(
            "opensubtitles",
            &query,
            SubtitlePluginCandidate {
                provider_file_id: "2".into(),
                language: "eng".into(),
                release_info: Some("Movie.Title.2024.1080p.BluRay.TrueHD.x265-GROUP".into()),
                hearing_impaired: false,
                forced: false,
                ai_translated: false,
                machine_translated: false,
                uploader: None,
                download_count: None,
                match_hints: vec![],
            },
        );

        assert_eq!(stronger.score, MOVIE_WEIGHTS.weights().max_score());
        assert!(stronger.score > weaker.score);
    }
}

#[cfg(test)]
mod component_routing_tests {
    use super::*;
    use crate::wasmtime_host::subtitle_component_host::tests::fixture_component;

    fn subtitle_descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture-subtitle".to_string(),
            name: "Fixture Subtitle".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: crate::types::SDK_VERSION.to_string(),
            sdk_constraint: crate::types::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: crate::types::ProviderDescriptor::Subtitle(crate::types::SubtitleDescriptor {
                provider_type: "fixture-subtitles".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: crate::types::SubtitleCapabilities::default(),
            }),
        }
    }

    fn provider_config() -> SubtitleProviderConfig {
        let now = chrono::Utc::now();
        SubtitleProviderConfig {
            id: "config-1".to_string(),
            name: "Fixture".to_string(),
            provider_type: "fixture-subtitles".to_string(),
            config_json: "{\"api_key\":\"fixture-host-call-secret\"}".to_string(),
            enabled_facets: Vec::new(),
            is_enabled: true,
            last_health_status: None,
            last_error: None,
            last_error_at: None,
            disabled_until: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn client_for(wasm: Vec<u8>) -> WasmSubtitleClient {
        WasmSubtitleClient::new_with_archive_provider(
            wasm,
            subtitle_descriptor(),
            provider_config(),
            HashMap::new(),
            None,
        )
        .expect("subtitle client must build")
    }

    /// A component artifact routes to the new component host, and the
    /// descriptor-scoped `CommandHost` — config included — is threaded into the
    /// spec the component will be instantiated from.
    #[test]
    fn a_component_artifact_selects_the_component_runtime() {
        let client = client_for(fixture_component());

        let spec = client
            .component_spec()
            .expect("a subtitle component must route to the component host");
        assert!(
            spec.preopens.is_empty(),
            "catalog operations need no filesystem authority"
        );
        assert_eq!(spec.timeout, SUBTITLE_PLUGIN_TIMEOUT);
    }

    /// The component path is additive: an ordinary core-module subtitle
    /// artifact still instantiates a legacy reactor at construction time and
    /// never reaches the component host.
    #[test]
    fn a_core_module_artifact_still_selects_the_legacy_runtime() {
        let core_module =
            wat::parse_str("(module (memory (export \"memory\") 1))").expect("core module WAT");

        let client = client_for(core_module);

        assert!(
            client.component_spec().is_none(),
            "a non-component subtitle artifact must keep the legacy path"
        );
        client
            .legacy_plugin()
            .expect("the legacy reactor must have been instantiated eagerly");
    }
}
