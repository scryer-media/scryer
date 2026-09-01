//! Subtitle-*sync* alignment, on the subtitle component world.
//!
//! # Why this file came back
//!
//! Alignment was never a `PluginSubtitleCommand`. It rode its own
//! [`SubtitleSyncPluginProcessRequest`] over the Preview 1 stdin/stdout command
//! transport, and when that transport was deleted the capability was orphaned:
//! the loader logged a warning and handed the application `None`, so every
//! align job degraded to "no sync plugin installed".
//!
//! It is restored here without a WIT revision. The
//! `scryer:subtitle/subtitle-provider@1.0.0` world's `process` payload is
//! opaque UTF-8 JSON carrying the SDK's command envelope, and the SDK — not the
//! WIT — owns the operation set. Adding `PluginSubtitleCommand::Sync`, whose
//! payload is the Preview 1 request type verbatim, was therefore enough: a sync
//! plugin already ships a `ProviderDescriptor::Subtitle`, so it validates,
//! classifies and instantiates as the subtitle world exactly like a catalog
//! provider, and [`process_subtitle_component`] moves the bytes.
//!
//! # Why this is a separate client from `WasmSubtitleClient`
//!
//! The two share a world but not a shape. A catalog provider is configured
//! (`SubtitleProviderConfig`, host bindings, allowed hosts) and is instantiated
//! once per configured provider row; a sync plugin has no configuration at all
//! and is instantiated once per *job*, because its preopens are derived from
//! the media file being aligned. Folding them together would mean a spec that
//! is rebuilt per call for one caller and cached for the other, so they stay
//! apart — as they were before the teardown.
//!
//! # The filesystem contract
//!
//! Alignment is the one plugin operation that moves real files. The guest sees
//! five fixed roots and nothing else:
//!
//! | Guest root   | Backed by                       | Mode     |
//! |--------------|---------------------------------|----------|
//! | `/input`     | the media file's parent dir     | read-only  |
//! | `/subtitle`  | a per-job temp dir              | read-only  |
//! | `/reference` | a per-job temp dir (optional)   | read-only  |
//! | `/output`    | a per-job temp dir              | writable |
//! | `/scratch`   | a per-job temp dir              | writable |
//!
//! The host stages the subtitle bytes in, and reads the rewritten subtitle back
//! out under the symlink-rejecting containment check in
//! [`read_guest_output_file`]. Every temp dir is owned by the prepared job and
//! is removed when it drops.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_application::{AppError, AppResult, SubtitleSyncClient, SubtitleSyncJob};
use scryer_plugin_sdk::command::{
    PluginCommand, PluginCommandRequest, PluginCommandResult, PluginSubtitleCommand,
    PluginSubtitleCommandResult,
};
use scryer_plugin_sdk::{
    AudioStreamSelector, PluginResult, SubtitleSyncAlignResponse, SubtitleSyncCommandAlignRequest,
    SubtitleSyncCommandAlignResponse, SubtitleSyncCommandInputFile,
    SubtitleSyncCommandOutputSubtitle, SubtitleSyncCommandOutputTarget,
    SubtitleSyncCommandSubtitleFile, SubtitleSyncPluginOperation, SubtitleSyncPluginProcessRequest,
    SubtitleSyncPluginResponse, SubtitleSyncRewrittenSubtitle,
};

use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking, PreopenSpec};
use crate::types::PluginDescriptor;
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::{SubtitleComponentInvocation, process_subtitle_component};

const GUEST_INPUT_ROOT: &str = "/input";
const GUEST_SUBTITLE_ROOT: &str = "/subtitle";
const GUEST_REFERENCE_ROOT: &str = "/reference";
const GUEST_OUTPUT_ROOT: &str = "/output";
const GUEST_SCRATCH_ROOT: &str = "/scratch";

/// Alignment decodes and correlates a whole audio track, so it keeps the
/// hour-long budget the Preview 1 runner used rather than the 30s budget a
/// catalog lookup runs under.
const SUBTITLE_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 60);
const MAX_REWRITTEN_SUBTITLE_BYTES: u64 = 16 * 1024 * 1024;

/// A subtitle-sync plugin, backed by the subtitle component world.
///
/// Components are instance-per-invocation, so this holds only the verified
/// artifact bytes and the descriptor; the [`PluginInstanceSpec`] is built per
/// job because its preopens depend on the media file being aligned.
pub struct WasmSubtitleSyncClient {
    wasm_bytes: Arc<Vec<u8>>,
    plugin_id: String,
    plugin_version: String,
}

impl WasmSubtitleSyncClient {
    /// Classify the artifact, then build the client.
    ///
    /// Classification happens here rather than at first align for the same
    /// reason it does in [`crate::subtitle_adapter::WasmSubtitleClient`]: a
    /// subtitle descriptor looks identical whether the artifact is a stale
    /// pre-component build or a `scryer:subtitle/subtitle-provider@1.0.0`
    /// component, so the operator-facing upgrade diagnostic belongs at provider
    /// construction. An installed Preview 1 subtitle-sync plugin therefore
    /// reports "rebuild against the component ABI", not a missing-import trap
    /// an hour into an align job.
    pub fn new(wasm_bytes: Vec<u8>, descriptor: &PluginDescriptor) -> Result<Self, String> {
        PluginRuntimeBacking::for_artifact(descriptor, &wasm_bytes)?;
        Ok(Self {
            wasm_bytes: Arc::new(wasm_bytes),
            plugin_id: descriptor.id.clone(),
            plugin_version: descriptor.version.clone(),
        })
    }
}

#[async_trait]
impl SubtitleSyncClient for WasmSubtitleSyncClient {
    async fn align_subtitle(&self, job: SubtitleSyncJob) -> AppResult<SubtitleSyncAlignResponse> {
        let prepared = PreparedSubtitleSyncCommand::new(job)?;
        let spec = prepared.instance_spec(Arc::clone(&self.wasm_bytes));

        let response = process_subtitle_component(
            &spec,
            &PluginCommandRequest::new(PluginCommand::Subtitle(PluginSubtitleCommand::Sync(
                prepared.request.clone(),
            ))),
            SubtitleComponentInvocation {
                plugin_id: &self.plugin_id,
                plugin_version: &self.plugin_version,
                operation: "Sync",
            },
        )
        .await?;

        let PluginCommandResult::Subtitle(result) = response.response else {
            return Err(AppError::Repository(format!(
                "subtitle sync plugin {} returned a response for another plugin family",
                self.plugin_id
            )));
        };
        let PluginSubtitleCommandResult::Sync(result) = result else {
            return Err(AppError::Repository(format!(
                "subtitle sync plugin {} answered an align request with another operation",
                self.plugin_id
            )));
        };
        let process = match result {
            PluginResult::Ok(process) => process,
            PluginResult::Err(error) => {
                return Err(AppError::Repository(format!(
                    "subtitle sync plugin {} refused the align: plugin error {:?}: {}",
                    self.plugin_id, error.code, error.public_message
                )));
            }
        };
        let align = match process.response {
            SubtitleSyncPluginResponse::Align { response } => *response,
            other => {
                return Err(AppError::Repository(format!(
                    "subtitle sync plugin returned unexpected response kind: {other:?}"
                )));
            }
        };
        prepared.align_response_to_port(align)
    }
}

impl std::fmt::Debug for WasmSubtitleSyncClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmSubtitleSyncClient")
            .field("plugin_id", &self.plugin_id)
            .field("plugin_version", &self.plugin_version)
            .finish()
    }
}

/// One align job's staged filesystem and the request that names it.
///
/// The temp dirs are fields rather than locals because they must outlive the
/// invocation: dropping a [`tempfile::TempDir`] deletes the directory, and the
/// guest reads `/subtitle` and writes `/output` while this is alive.
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

    /// The per-job instance spec.
    ///
    /// `CommandHost::disabled()` is deliberate: a sync plugin has no
    /// configuration, makes no HTTP requests and opens no sockets — it decodes
    /// audio and correlates it against subtitle timings. The
    /// `scryer:host/services@1.0.0` import the component world declares is
    /// still served, and answers every request in-band with `Unsupported`, so a
    /// plugin that reaches for a capability gets a typed refusal rather than a
    /// trap.
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
            timeout: SUBTITLE_SYNC_TIMEOUT,
            memory_max_bytes: None,
            command_host: CommandHost::disabled(),
        }
    }

    /// Turn the guest's align response into the application port's response,
    /// reading the rewritten subtitle back out of `/output` when one was
    /// applied.
    fn align_response_to_port(
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
            "subtitle sync plugin output exceeds {MAX_REWRITTEN_SUBTITLE_BYTES} bytes"
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

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_plugin_sdk::SubtitleSyncOptions;
    use scryer_plugin_sdk::host::{PluginConfigGetRequest, PluginHostRequest, PluginHostResponse};

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
        let port = prepared
            .align_response_to_port(response)
            .expect("port response");

        let rewritten = port.rewritten_subtitle.expect("rewritten subtitle present");
        let decoded = BASE64
            .decode(rewritten.content_base64)
            .expect("decode base64");
        assert_eq!(decoded, b"rewritten-subtitle-bytes");
    }

    // A malicious guest can write anything into its writable `/output` preopen,
    // including a symlink pointing at a sensitive host file. The host must
    // refuse to follow it rather than exfiltrate the target.
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_output_dir() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");

        // A sensitive host file that lives OUTSIDE the writable output preopen.
        let secret_dir = tempfile::tempdir().expect("secret dir");
        let secret_path = secret_dir.path().join("credentials.env");
        std::fs::write(&secret_path, b"INDEXER_API_KEY=super-secret-value").expect("write secret");

        // Guest writes the "rewritten subtitle" as a symlink to the secret.
        std::os::unix::fs::symlink(&secret_path, &prepared.host_output_path)
            .expect("create malicious symlink");

        let response = applied_response(prepared.guest_output_path.clone());
        let error = prepared
            .align_response_to_port(response)
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
            .align_response_to_port(response)
            .expect_err("symlink output must be rejected");
        assert!(
            matches!(error, AppError::Validation(_)),
            "expected a validation error, got {error:?}"
        );
    }

    /// The align job's staged filesystem is what the guest actually sees, so
    /// the preopen set is part of the contract rather than an implementation
    /// detail: five roots, only `/output` and `/scratch` writable.
    #[test]
    fn stages_the_documented_preopen_set() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");
        let spec = prepared.instance_spec(Arc::new(Vec::new()));

        let mut roots = spec
            .preopens
            .iter()
            .map(|preopen| (preopen.guest_path.as_str(), preopen.writable))
            .collect::<Vec<_>>();
        roots.sort_unstable();
        assert_eq!(
            roots,
            vec![
                ("/input", false),
                ("/output", true),
                ("/scratch", true),
                ("/subtitle", false),
            ]
        );
    }

    /// A sync plugin is offered no host services at all; the world's import is
    /// still served and refuses in-band.
    #[test]
    fn offers_no_host_services() {
        let prepared = PreparedSubtitleSyncCommand::new(sample_job()).expect("prepare command");
        let spec = prepared.instance_spec(Arc::new(Vec::new()));
        let request =
            postcard::to_allocvec(&PluginHostRequest::ConfigGet(PluginConfigGetRequest {
                key: "anything".to_string(),
            }))
            .expect("encode host request");

        let encoded = spec
            .command_host
            .call_bytes(&request)
            .expect("host call is served");
        let response: PluginHostResponse =
            postcard::from_bytes(&encoded).expect("decode host response");
        let PluginHostResponse::ConfigGet(PluginResult::Err(error)) = response else {
            panic!("expected an in-band config refusal");
        };
        assert_eq!(error.code, scryer_plugin_sdk::PluginErrorCode::Unsupported);
    }
}
