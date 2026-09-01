use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_application::{AppError, AppResult, SubtitleSyncClient, SubtitleSyncJob};

use crate::blocking::run_blocking_plugin_call;
use crate::legacy_runtime::{LegacyPlugin, LegacyPluginSpec};
use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking, PreopenSpec};
use crate::types::{
    AudioStreamSelector, EXPORT_SUBSYNC_ALIGN, PluginDescriptor, SubtitleSyncAlignInputRef,
    SubtitleSyncAlignRequest, SubtitleSyncAlignResponse, SubtitleSyncCommandAlignRequest,
    SubtitleSyncCommandAlignResponse, SubtitleSyncCommandInputFile,
    SubtitleSyncCommandOutputSubtitle, SubtitleSyncCommandOutputTarget,
    SubtitleSyncCommandSubtitleFile, SubtitleSyncInputSubtitle, SubtitleSyncPluginOperation,
    SubtitleSyncPluginProcessRequest, SubtitleSyncPluginResponse, SubtitleSyncReferenceSubtitle,
    SubtitleSyncRewrittenSubtitle, decode_plugin_result,
};
use crate::wasmtime_host::{SubtitleSyncInvocation, process_subtitle_sync};

const GUEST_INPUT_ROOT: &str = "/input";
const GUEST_SUBTITLE_ROOT: &str = "/subtitle";
const GUEST_REFERENCE_ROOT: &str = "/reference";
const GUEST_OUTPUT_ROOT: &str = "/output";
const GUEST_SCRATCH_ROOT: &str = "/scratch";
const SUBTITLE_SYNC_TIMEOUT_SECONDS: u64 = 60 * 60;
const MAX_REWRITTEN_SUBTITLE_BYTES: u64 = 16 * 1024 * 1024;

pub struct WasmSubtitleSyncClient {
    wasm_bytes: Arc<Vec<u8>>,
    descriptor: PluginDescriptor,
    plugin_id: String,
    plugin_version: String,
    backing: PluginRuntimeBacking,
}

impl WasmSubtitleSyncClient {
    pub fn new(wasm_bytes: Vec<u8>, descriptor: PluginDescriptor) -> Self {
        let backing = PluginRuntimeBacking::for_descriptor(&descriptor);
        Self {
            wasm_bytes: Arc::new(wasm_bytes),
            plugin_id: descriptor.id.clone(),
            plugin_version: descriptor.version.clone(),
            descriptor,
            backing,
        }
    }
}

#[async_trait]
impl SubtitleSyncClient for WasmSubtitleSyncClient {
    async fn align_subtitle(&self, job: SubtitleSyncJob) -> AppResult<SubtitleSyncAlignResponse> {
        match self.backing {
            PluginRuntimeBacking::LegacyReactor => self.align_subtitle_legacy(job).await,
            PluginRuntimeBacking::WasmtimeSubtitleSync => self.align_subtitle_command(job).await,
            PluginRuntimeBacking::WasmtimeArchiveComponent => Err(AppError::Repository(
                "subtitle sync plugin cannot use the archive component runtime backing".to_string(),
            )),
            PluginRuntimeBacking::WasmtimeCommand => Err(AppError::Repository(
                "subtitle sync plugin cannot use the generic command runtime backing".to_string(),
            )),
            PluginRuntimeBacking::WasmtimeIndexerComponent => Err(AppError::Repository(
                "subtitle sync plugin cannot use the indexer component runtime backing".to_string(),
            )),
            // Alignment is its own contract (`SubtitleSyncPluginProcessRequest`
            // on a wasip1 command), not a `PluginSubtitleCommand`, so the
            // subtitle-provider world does not carry it.
            PluginRuntimeBacking::WasmtimeSubtitleComponent => Err(AppError::Repository(
                "subtitle sync plugin cannot use the subtitle provider component runtime backing"
                    .to_string(),
            )),
        }
    }
}

impl WasmSubtitleSyncClient {
    async fn align_subtitle_legacy(
        &self,
        job: SubtitleSyncJob,
    ) -> AppResult<SubtitleSyncAlignResponse> {
        let scratch_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!(
                "failed to create subtitle sync scratch directory: {error}"
            ))
        })?;
        let input_path = job.input_path.clone();
        let guest_input_path = guest_file_path(GUEST_INPUT_ROOT, &input_path)?;
        let request = SubtitleSyncAlignRequest {
            input: SubtitleSyncAlignInputRef {
                path: guest_input_path,
            },
            subtitle: SubtitleSyncInputSubtitle {
                content_base64: BASE64.encode(&job.subtitle_content),
                format: job.subtitle_format,
                file_name: job.subtitle_file_name,
                encoding_hint: job.subtitle_encoding_hint,
            },
            reference_subtitle: job.reference_subtitle.map(|subtitle| {
                SubtitleSyncReferenceSubtitle {
                    content_base64: BASE64.encode(&subtitle.content),
                    format: subtitle.format,
                    file_name: subtitle.file_name,
                    encoding_hint: subtitle.encoding_hint,
                }
            }),
            subtitle_spans: Vec::new(),
            max_offset_seconds: job.max_offset_seconds,
            sync_options: Some(job.sync_options),
            selector: Some(AudioStreamSelector::Default),
            expected_codec: job.expected_codec,
        };
        let input = serde_json::to_string(&request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize subtitle sync request: {error}"
            ))
        })?;

        let spec = build_subtitle_sync_spec(
            self.wasm_bytes.as_slice(),
            &self.descriptor,
            &input_path,
            scratch_dir.path(),
        );
        let output = run_blocking_plugin_call(
            std::time::Duration::from_secs(SUBTITLE_SYNC_TIMEOUT_SECONDS),
            "subtitle sync plugin",
            move || {
                let mut plugin = LegacyPlugin::instantiate(spec).map_err(|error| {
                    AppError::Repository(format!("failed to compile subtitle sync plugin: {error}"))
                })?;
                if !plugin.function_exists(EXPORT_SUBSYNC_ALIGN) {
                    return Err(AppError::Repository(format!(
                        "plugin does not export {EXPORT_SUBSYNC_ALIGN}"
                    )));
                }
                plugin
                    .call_string(EXPORT_SUBSYNC_ALIGN, &input)
                    .map_err(|error| plugin_call_error(EXPORT_SUBSYNC_ALIGN, error))
            },
        )
        .await?;

        decode_plugin_result(&output, EXPORT_SUBSYNC_ALIGN)
    }

    async fn align_subtitle_command(
        &self,
        job: SubtitleSyncJob,
    ) -> AppResult<SubtitleSyncAlignResponse> {
        let prepared = PreparedSubtitleSyncCommand::new(job)?;
        let input = serde_json::to_string(&prepared.request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize subtitle sync command request: {error}"
            ))
        })?;
        let spec = prepared.instance_spec(Arc::clone(&self.wasm_bytes));
        let plugin_id = self.plugin_id.clone();
        let plugin_version = self.plugin_version.clone();

        tokio::time::timeout(
            std::time::Duration::from_secs(SUBTITLE_SYNC_TIMEOUT_SECONDS),
            async move {
                let invocation = SubtitleSyncInvocation {
                    plugin_id: &plugin_id,
                    plugin_version: &plugin_version,
                    operation: "Align",
                };
                let response = process_subtitle_sync(&spec, &input, invocation).await?;
                let align = match response.response {
                    SubtitleSyncPluginResponse::Align { response } => *response,
                    other => {
                        return Err(AppError::Repository(format!(
                            "subtitle sync plugin returned unexpected response kind: {other:?}"
                        )));
                    }
                };
                prepared.command_response_to_legacy(align)
            },
        )
        .await
        .map_err(|_| {
            AppError::Repository(format!(
                "subtitle sync command plugin timed out after {SUBTITLE_SYNC_TIMEOUT_SECONDS} seconds"
            ))
        })?
    }
}

impl std::fmt::Debug for WasmSubtitleSyncClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmSubtitleSyncClient")
            .field("plugin_id", &self.descriptor.id)
            .field("plugin_name", &self.descriptor.name)
            .finish()
    }
}

pub(crate) fn build_subtitle_sync_spec(
    wasm_bytes: &[u8],
    descriptor: &PluginDescriptor,
    input_path: &Path,
    scratch_dir: &Path,
) -> LegacyPluginSpec {
    let input_root = input_path.parent().unwrap_or_else(|| Path::new("."));

    let mut spec = LegacyPluginSpec::new(wasm_bytes.to_vec(), descriptor.id.clone());
    spec.timeout = std::time::Duration::from_secs(SUBTITLE_SYNC_TIMEOUT_SECONDS);
    spec.preopens
        .push(PreopenSpec::read_only(input_root, GUEST_INPUT_ROOT));
    spec.preopens
        .push(PreopenSpec::writable(scratch_dir, GUEST_SCRATCH_ROOT));
    spec
}

struct PreparedSubtitleSyncCommand {
    request: SubtitleSyncPluginProcessRequest,
    media_root: PathBuf,
    subtitle_dir: tempfile::TempDir,
    reference_dir: Option<tempfile::TempDir>,
    output_dir: tempfile::TempDir,
    scratch_dir: tempfile::TempDir,
    guest_output_path: PathBuf,
    host_output_path: PathBuf,
}

impl PreparedSubtitleSyncCommand {
    fn new(job: SubtitleSyncJob) -> AppResult<Self> {
        let input_path = job.input_path;
        let media_root = input_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let guest_input_path = guest_file_path(GUEST_INPUT_ROOT, &input_path)?;

        let subtitle_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!(
                "failed to create subtitle sync input directory: {error}"
            ))
        })?;
        let subtitle_file_name =
            safe_guest_file_name(job.subtitle_file_name.as_deref(), "subtitle.input");
        let host_subtitle_path = subtitle_dir.path().join(&subtitle_file_name);
        std::fs::write(&host_subtitle_path, &job.subtitle_content).map_err(|error| {
            AppError::Repository(format!(
                "failed to stage subtitle sync input '{}': {error}",
                host_subtitle_path.display()
            ))
        })?;
        let guest_subtitle_path = Path::new(GUEST_SUBTITLE_ROOT).join(&subtitle_file_name);

        let (reference_subtitle, reference_dir) = if let Some(reference) = job.reference_subtitle {
            let dir = tempfile::tempdir().map_err(|error| {
                AppError::Repository(format!(
                    "failed to create subtitle sync reference directory: {error}"
                ))
            })?;
            let file_name = safe_guest_file_name(reference.file_name.as_deref(), "reference.input");
            let host_path = dir.path().join(&file_name);
            std::fs::write(&host_path, &reference.content).map_err(|error| {
                AppError::Repository(format!(
                    "failed to stage subtitle sync reference '{}': {error}",
                    host_path.display()
                ))
            })?;
            (
                Some(SubtitleSyncCommandSubtitleFile {
                    path: Path::new(GUEST_REFERENCE_ROOT).join(&file_name),
                    format: reference.format,
                    file_name: reference.file_name,
                    encoding_hint: reference.encoding_hint,
                }),
                Some(dir),
            )
        } else {
            (None, None)
        };

        let output_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!(
                "failed to create subtitle sync output directory: {error}"
            ))
        })?;
        let scratch_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!(
                "failed to create subtitle sync scratch directory: {error}"
            ))
        })?;
        let output_file_name = format!("rewritten.{}", output_extension(&job.subtitle_format));
        let host_output_path = output_dir.path().join(&output_file_name);
        let guest_output_path = Path::new(GUEST_OUTPUT_ROOT).join(&output_file_name);

        let align = SubtitleSyncCommandAlignRequest {
            input: SubtitleSyncCommandInputFile {
                path: guest_input_path,
            },
            subtitle: SubtitleSyncCommandSubtitleFile {
                path: guest_subtitle_path,
                format: job.subtitle_format.clone(),
                file_name: job.subtitle_file_name,
                encoding_hint: job.subtitle_encoding_hint,
            },
            reference_subtitle,
            output: SubtitleSyncCommandOutputTarget {
                path: guest_output_path.clone(),
                format: job.subtitle_format,
            },
            scratch_dir: PathBuf::from(GUEST_SCRATCH_ROOT),
            media_metadata: job.media_metadata,
            subtitle_spans: Vec::new(),
            max_offset_seconds: job.max_offset_seconds,
            sync_options: Some(job.sync_options),
            selector: Some(AudioStreamSelector::Default),
            expected_codec: job.expected_codec,
        };

        Ok(Self {
            request: SubtitleSyncPluginProcessRequest {
                operation: SubtitleSyncPluginOperation::Align {
                    request: Box::new(align),
                },
            },
            media_root,
            subtitle_dir,
            reference_dir,
            output_dir,
            scratch_dir,
            guest_output_path,
            host_output_path,
        })
    }

    fn instance_spec(&self, wasm: Arc<Vec<u8>>) -> PluginInstanceSpec {
        let mut preopens = vec![
            PreopenSpec::read_only(self.media_root.clone(), GUEST_INPUT_ROOT),
            PreopenSpec::read_only(self.subtitle_dir.path(), GUEST_SUBTITLE_ROOT),
            PreopenSpec::writable(self.output_dir.path(), GUEST_OUTPUT_ROOT),
            PreopenSpec::writable(self.scratch_dir.path(), GUEST_SCRATCH_ROOT),
        ];
        if let Some(reference_dir) = &self.reference_dir {
            preopens.push(PreopenSpec::read_only(
                reference_dir.path(),
                GUEST_REFERENCE_ROOT,
            ));
        }
        PluginInstanceSpec {
            wasm,
            preopens,
            timeout: std::time::Duration::from_secs(SUBTITLE_SYNC_TIMEOUT_SECONDS),
            memory_max_bytes: None,
            command_host: crate::wasmtime_host::command_host::CommandHost::disabled(),
        }
    }

    fn command_response_to_legacy(
        &self,
        response: SubtitleSyncCommandAlignResponse,
    ) -> AppResult<SubtitleSyncAlignResponse> {
        let rewritten_subtitle = if response.applied {
            let rewritten = response.rewritten_subtitle.as_ref().ok_or_else(|| {
                AppError::Repository(
                    "subtitle sync plugin reported applied without rewritten_subtitle".to_string(),
                )
            })?;
            validate_rewritten_output_path(rewritten, &self.guest_output_path)?;
            let bytes = read_guest_output_file(self.output_dir.path(), &self.host_output_path)?;
            Some(SubtitleSyncRewrittenSubtitle {
                content_base64: BASE64.encode(bytes),
                format: rewritten.format.clone(),
            })
        } else {
            None
        };

        Ok(SubtitleSyncAlignResponse {
            applied: response.applied,
            offset_ms: response.offset_ms,
            rewritten_subtitle,
            score: response.score,
            selected_framerate_ratio: response.selected_framerate_ratio,
            consistency_ratio: response.consistency_ratio,
            nosplit_score: response.nosplit_score,
            split_score: response.split_score,
            skipped_reason: response.skipped_reason,
            backend: response.backend,
            warnings: response.warnings,
            message: response.message,
        })
    }
}

fn guest_file_path(root: &str, host_path: &Path) -> AppResult<PathBuf> {
    let file_name = host_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "subtitle sync path '{}' has no file name",
            host_path.display()
        ))
    })?;
    Ok(Path::new(root).join(file_name))
}

fn safe_guest_file_name(file_name: Option<&str>, fallback: &str) -> String {
    file_name
        .and_then(|name| Path::new(name).file_name())
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn output_extension(format: &str) -> String {
    let extension = format
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if extension.is_empty() {
        "subtitle".to_string()
    } else {
        extension
    }
}

fn validate_rewritten_output_path(
    rewritten: &SubtitleSyncCommandOutputSubtitle,
    expected_path: &Path,
) -> AppResult<()> {
    if rewritten.path != expected_path {
        return Err(AppError::Validation(format!(
            "subtitle sync plugin returned unexpected rewritten subtitle path '{}'",
            rewritten.path.display()
        )));
    }
    Ok(())
}

/// Read a file the guest produced inside its writable `/output` preopen.
///
/// The guest can write anything into `/output`, including a symlink that points
/// at a sensitive host file. `validate_rewritten_output_path` only checks the
/// guest-*reported* path string, so it cannot catch that. Before reading with
/// ambient host authority we therefore inspect the filesystem object directly:
/// we reject anything that is not a regular file (which excludes symlinks, since
/// `symlink_metadata` does not follow them) and confirm the resolved path is
/// still contained within the output preopen root. This mirrors the symlink
/// rejection the archive plugin consumer already performs on guest output.
fn read_guest_output_file(output_root: &Path, path: &Path) -> AppResult<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to read subtitle sync output '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() {
        return Err(AppError::Validation(format!(
            "subtitle sync plugin output is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_REWRITTEN_SUBTITLE_BYTES {
        return Err(AppError::Validation(format!(
            "subtitle sync plugin output exceeds {} bytes",
            MAX_REWRITTEN_SUBTITLE_BYTES
        )));
    }

    let output_root = output_root.canonicalize().map_err(|error| {
        AppError::Repository(format!(
            "failed to canonicalize subtitle sync output directory {}: {error}",
            output_root.display()
        ))
    })?;
    let canonical = path.canonicalize().map_err(|error| {
        AppError::Repository(format!(
            "failed to canonicalize subtitle sync output '{}': {error}",
            path.display()
        ))
    })?;
    if !canonical.starts_with(&output_root) {
        return Err(AppError::Validation(format!(
            "subtitle sync plugin output escapes the output directory: {}",
            path.display()
        )));
    }

    std::fs::read(&canonical).map_err(|error| {
        AppError::Repository(format!(
            "failed to read subtitle sync output '{}': {error}",
            path.display()
        ))
    })
}

fn plugin_call_error(export: &str, error: AppError) -> AppError {
    AppError::Repository(format!("{export}() failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SubtitleSyncOptions;

    fn sample_job() -> SubtitleSyncJob {
        SubtitleSyncJob {
            input_path: PathBuf::from("/media/library/movie.mkv"),
            subtitle_content: b"1\n00:00:01,000 --> 00:00:02,000\noriginal\n".to_vec(),
            subtitle_format: "srt".to_string(),
            subtitle_file_name: Some("original.srt".to_string()),
            subtitle_encoding_hint: None,
            reference_subtitle: None,
            max_offset_seconds: 60,
            sync_options: SubtitleSyncOptions::default(),
            expected_codec: None,
            media_metadata: None,
        }
    }

    fn applied_response(reported_path: PathBuf) -> SubtitleSyncCommandAlignResponse {
        SubtitleSyncCommandAlignResponse {
            applied: true,
            offset_ms: 0,
            rewritten_subtitle: Some(SubtitleSyncCommandOutputSubtitle {
                path: reported_path,
                format: "srt".to_string(),
            }),
            score: None,
            selected_framerate_ratio: None,
            consistency_ratio: None,
            nosplit_score: None,
            split_score: None,
            skipped_reason: None,
            backend: "test".to_string(),
            warnings: Vec::new(),
            message: None,
        }
    }

    #[test]
    fn reads_back_regular_output_file() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");
        std::fs::write(&prepared.host_output_path, b"rewritten-subtitle-bytes")
            .expect("stage guest output");

        let response = applied_response(prepared.guest_output_path.clone());
        let legacy = prepared
            .command_response_to_legacy(response)
            .expect("legacy response");

        let rewritten = legacy
            .rewritten_subtitle
            .expect("rewritten subtitle present");
        let decoded = BASE64
            .decode(rewritten.content_base64)
            .expect("decode base64");
        assert_eq!(decoded, b"rewritten-subtitle-bytes");
    }

    // A malicious command-model guest can write anything into its writable
    // `/output` preopen, including a symlink pointing at a sensitive host file.
    // The host must refuse to follow it rather than exfiltrate the target.
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_output_dir() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");

        // A sensitive host file that lives OUTSIDE the writable output preopen.
        let secret_dir = tempfile::tempdir().expect("secret dir");
        let secret_path = secret_dir.path().join("credentials.env");
        let secret = b"INDEXER_API_KEY=super-secret-value";
        std::fs::write(&secret_path, secret).expect("write secret");

        // Guest writes the "rewritten subtitle" as a symlink to the secret.
        std::os::unix::fs::symlink(&secret_path, &prepared.host_output_path)
            .expect("create malicious symlink");

        let response = applied_response(prepared.guest_output_path.clone());
        let error = prepared
            .command_response_to_legacy(response)
            .expect_err("symlink escape must be rejected");

        assert!(
            matches!(error, AppError::Validation(_)),
            "expected a validation error, got {error:?}"
        );
        // The secret bytes must never surface, not even in the error text.
        assert!(
            !format!("{error:?}").contains("super-secret-value"),
            "secret contents leaked into error"
        );
    }

    // Even a symlink whose target is inside the output dir is rejected: guest
    // output must be a plain regular file, never a link.
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_within_output_dir() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");

        let real_path = prepared.output_dir.path().join("real.srt");
        std::fs::write(&real_path, b"contained").expect("write contained file");
        std::os::unix::fs::symlink(&real_path, &prepared.host_output_path).expect("create symlink");

        let response = applied_response(prepared.guest_output_path.clone());
        let error = prepared
            .command_response_to_legacy(response)
            .expect_err("symlink output must be rejected");
        assert!(
            matches!(error, AppError::Validation(_)),
            "expected a validation error, got {error:?}"
        );
    }
}
