use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, ArchiveExtractorClient};
use scryer_plugin_sdk::{
    ArchiveExtractionLimits, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, PluginDescriptor,
};

use crate::runtime_backing::{PluginInstanceSpec, PluginRuntimeBacking, PreopenSpec};
use crate::wasmtime_host::{ArchiveInvocation, process_archive_component};

const GUEST_SOURCE_ROOT: &str = "/scryer/source";
const GUEST_OUTPUT_ROOT: &str = "/scryer/output";
const ARCHIVE_PROCESS_TIMEOUT_SECONDS: u64 = 60 * 60;
// All archive entry points share this process-local resource gate, including
// subtitles and guest command delegation. Import coordination alone cannot
// bound the combined guest memory footprint.
static ARCHIVE_INVOCATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub struct WasmArchiveExtractorClient {
    wasm_bytes: Arc<Vec<u8>>,
    plugin_id: String,
    plugin_version: String,
    enforced_limits: bool,
}

impl WasmArchiveExtractorClient {
    pub fn new(wasm_bytes: Vec<u8>, descriptor: PluginDescriptor) -> AppResult<Self> {
        // Classify from the artifact, not the descriptor: a descriptor alone
        // cannot tell a component from the removed core-module build, and the
        // upgrade diagnostic belongs here rather than at first extraction. An
        // archive descriptor can only select the archive component host, so the
        // selection itself is not retained — only the refusal matters.
        PluginRuntimeBacking::for_artifact(&descriptor, &wasm_bytes)
            .map_err(AppError::Repository)?;
        Ok(Self {
            wasm_bytes: Arc::new(wasm_bytes),
            enforced_limits: descriptor
                .archive_extractor()
                .is_some_and(|provider| provider.capabilities.enforced_limits),
            plugin_id: descriptor.id,
            plugin_version: descriptor.version,
        })
    }
}

#[async_trait]
impl ArchiveExtractorClient for WasmArchiveExtractorClient {
    async fn process(
        &self,
        request: ArchivePluginProcessRequest,
    ) -> AppResult<ArchivePluginProcessResponse> {
        self.invoke(request, None).await
    }

    async fn process_with_limits(
        &self,
        request: ArchivePluginProcessRequest,
        limits: ArchiveExtractionLimits,
    ) -> AppResult<ArchivePluginProcessResponse> {
        if !self.enforced_limits {
            return Err(AppError::ArchiveExtractionPluginRequired {
                message: "Archive Extraction requires a plugin that enforces caller resource limits; update the plugin and retry".into(),
                source_path: None,
            });
        }
        self.invoke(request, Some(limits)).await
    }
}

impl WasmArchiveExtractorClient {
    async fn invoke(
        &self,
        request: ArchivePluginProcessRequest,
        limits: Option<ArchiveExtractionLimits>,
    ) -> AppResult<ArchivePluginProcessResponse> {
        let prepared = PreparedArchiveRequest::new(request)?;
        let mut envelope = serde_json::to_value(&prepared.request).map_err(|error| {
            AppError::Repository(format!("failed to encode archive request: {error}"))
        })?;
        if let Some(limits) = limits {
            envelope["limits"] = serde_json::to_value(limits).map_err(|error| {
                AppError::Repository(format!("failed to encode archive limits: {error}"))
            })?;
        }
        let input = serde_json::to_string(&envelope).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize archive process request: {error}"
            ))
        })?;
        let operation = operation_label(&prepared.request.operation);
        let spec = prepared.instance_spec(Arc::clone(&self.wasm_bytes));
        let plugin_id = self.plugin_id.clone();
        let plugin_version = self.plugin_version.clone();

        tokio::time::timeout(
            Duration::from_secs(ARCHIVE_PROCESS_TIMEOUT_SECONDS),
            async move {
                let _permit = ARCHIVE_INVOCATIONS.acquire().await.map_err(|_| {
                    AppError::Repository("archive execution coordinator is closed".into())
                })?;
                // Keep `prepared` alive for the invocation so the preopened paths
                // remain owned for the full plugin call.
                let _prepared = prepared;
                let invocation = ArchiveInvocation {
                    plugin_id: &plugin_id,
                    plugin_version: &plugin_version,
                    operation,
                };
                process_archive_component(&spec, &input, invocation).await
            },
        )
        .await
        .map_err(|_| {
            AppError::archive_extraction_timed_out(format!(
                "archive plugin timed out after {ARCHIVE_PROCESS_TIMEOUT_SECONDS} seconds"
            ))
        })?
    }
}

fn operation_label(operation: &ArchivePluginOperation) -> &'static str {
    match operation {
        ArchivePluginOperation::Inspect { .. } => "Inspect",
        ArchivePluginOperation::ExtractArchive { .. } => "ExtractArchive",
    }
}

struct PreparedArchiveRequest {
    request: ArchivePluginProcessRequest,
    source_root: Option<PathBuf>,
    output_root: Option<PathBuf>,
}

impl PreparedArchiveRequest {
    fn new(request: ArchivePluginProcessRequest) -> AppResult<Self> {
        match request.operation {
            ArchivePluginOperation::Inspect {
                source_dir,
                archive_path,
            } => {
                let source_root = PathBuf::from(source_dir);
                let archive_path = archive_path
                    .map(|path| map_child_path(Path::new(&source_root), Path::new(&path)))
                    .transpose()?;
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::Inspect {
                            source_dir: GUEST_SOURCE_ROOT.to_string(),
                            archive_path,
                        },
                    },
                    source_root: Some(source_root),
                    output_root: None,
                })
            }
            ArchivePluginOperation::ExtractArchive {
                archive_path,
                output_dir,
                format,
                password,
            } => {
                let archive_path = PathBuf::from(archive_path);
                let source_root = archive_path.parent().unwrap_or_else(|| Path::new("."));
                let source_root = source_root.to_path_buf();
                let guest_archive_path = map_child_path(&source_root, &archive_path)?;
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::ExtractArchive {
                            archive_path: guest_archive_path,
                            output_dir: GUEST_OUTPUT_ROOT.to_string(),
                            format,
                            password,
                        },
                    },
                    source_root: Some(source_root),
                    output_root: Some(PathBuf::from(output_dir)),
                })
            }
        }
    }

    /// Express this request's sandbox + timeout requirements as a runtime
    /// spec. Archive plugins only extract from a read-only source into a
    /// writable output; PAR2 repair/normalization is native Scryer work.
    fn instance_spec(&self, wasm: Arc<Vec<u8>>) -> PluginInstanceSpec {
        let mut preopens = Vec::new();
        if let Some(source_root) = &self.source_root {
            preopens.push(PreopenSpec::read_only(
                source_root.clone(),
                GUEST_SOURCE_ROOT,
            ));
        }
        if let Some(output_root) = &self.output_root {
            preopens.push(PreopenSpec::writable(
                output_root.clone(),
                GUEST_OUTPUT_ROOT,
            ));
        }
        PluginInstanceSpec {
            wasm,
            preopens,
            timeout: Duration::from_secs(ARCHIVE_PROCESS_TIMEOUT_SECONDS),
            // None = the host's provisional default cap;
            // operator-overridable.
            memory_max_bytes: None,
            // Archive extractors are the terminal delegation boundary. Keeping
            // host services disabled prevents extractor -> host -> extractor
            // recursion even when the invoking plugin used host extraction.
            command_host: crate::wasmtime_host::command_host::CommandHost::disabled(),
        }
    }
}

fn map_child_path(root: &Path, path: &Path) -> AppResult<String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let relative = path.strip_prefix(root).map_err(|_| {
        AppError::Validation(format!(
            "archive plugin path '{}' is outside allowed root '{}'",
            scryer_application::redact_archive_diagnostic(&path.to_string_lossy()),
            scryer_application::redact_archive_diagnostic(&root.to_string_lossy())
        ))
    })?;
    if !is_safe_relative_plugin_path(relative) {
        return Err(AppError::Validation(format!(
            "archive plugin path '{}' is not a safe relative path",
            scryer_application::redact_archive_diagnostic(&path.to_string_lossy())
        )));
    }
    let guest_path = Path::new(GUEST_SOURCE_ROOT).join(relative);
    Ok(guest_path.to_string_lossy().into_owned())
}

fn is_safe_relative_plugin_path(path: &Path) -> bool {
    path.components().all(|component| {
        matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_adapter_path_failures_redact_names_but_guest_paths_remain_literal() {
        let root = Path::new("/synthetic/allowed{{synthetic-root-secret}}");
        for path in [
            Path::new("/other/release{{synthetic-secret}}.rar"),
            Path::new("../release{{synthetic-secret}}.rar"),
        ] {
            let error = map_child_path(root, path).unwrap_err().to_string();
            assert!(!error.contains("synthetic-secret"));
            assert!(!error.contains("synthetic-root-secret"));
            assert!(error.contains("[redacted]"));
        }
        assert_eq!(
            map_child_path(root, Path::new("release{{synthetic-secret}}.rar")).unwrap(),
            "/scryer/source/release{{synthetic-secret}}.rar"
        );
    }
}
