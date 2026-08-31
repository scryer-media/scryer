//! Runtime-strategy seam.
//!
//! A `PluginInstanceSpec` describes an instantiation in runtime-agnostic terms;
//! a `PluginRuntimeBacking` says which runtime executes it. Adapters express
//! their sandbox/timeout requirements through the spec so command-model archive
//! plugins can run with the native Wasmtime host.
//!
//! Legacy Extism-PDK reactor plugins run through `LegacyReactor`, a Wasmtime
//! compatibility host that preserves the guest ABI without depending on the
//! Extism crate.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use scryer_plugin_sdk::{PluginDescriptor, PluginKind, SubtitleProviderMode};

use crate::command_abi;
use crate::wasmtime_host::command_host::CommandHost;

/// One preopened directory mapping for a plugin instance.
#[derive(Debug, Clone)]
pub(crate) struct PreopenSpec {
    pub(crate) host_path: PathBuf,
    pub(crate) guest_path: String,
    /// `false` = read-only (wasmtime `DirPerms::READ`).
    pub(crate) writable: bool,
}

impl PreopenSpec {
    pub(crate) fn read_only(host_path: impl Into<PathBuf>, guest_path: impl Into<String>) -> Self {
        Self {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            writable: false,
        }
    }

    pub(crate) fn writable(host_path: impl Into<PathBuf>, guest_path: impl Into<String>) -> Self {
        Self {
            host_path: host_path.into(),
            guest_path: guest_path.into(),
            writable: true,
        }
    }
}

/// Runtime-agnostic description of a single plugin invocation.
#[derive(Clone)]
pub(crate) struct PluginInstanceSpec {
    /// The verified artifact bytes (from `LoadedPlugin::materialize_wasm()`).
    pub(crate) wasm: Arc<Vec<u8>>,
    pub(crate) preopens: Vec<PreopenSpec>,
    pub(crate) timeout: Duration,
    /// Hard memory cap; `None` = the runtime's default cap.
    pub(crate) memory_max_bytes: Option<usize>,
    /// Descriptor-scoped native host services for command guests.
    pub(crate) command_host: CommandHost,
}

/// Which runtime executes a `PluginInstanceSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginRuntimeBacking {
    /// Existing Extism-PDK reactor artifacts on Scryer's native Wasmtime host.
    LegacyReactor,
    /// The native wasmtime archive host.
    WasmtimeArchive,
    /// Native wasmtime command host for SDK 3.5 subtitle-sync plugins.
    WasmtimeSubtitleSync,
    /// Versioned native command ABI for all descriptor families.
    WasmtimeCommand,
    /// Versioned WASI Preview 2 component ABI for indexer plugins.
    WasmtimeIndexerComponent,
}

impl PluginRuntimeBacking {
    /// Backing selected from the descriptor kind.
    pub(crate) fn for_descriptor(descriptor: &PluginDescriptor) -> Self {
        match descriptor.kind() {
            PluginKind::ArchiveExtractor => Self::WasmtimeArchive,
            PluginKind::SubtitleProvider => {
                let command_sync = descriptor.subtitle().is_some_and(|subtitle| {
                    subtitle.capabilities.mode == SubtitleProviderMode::Sync
                        && subtitle
                            .capabilities
                            .sync
                            .as_ref()
                            .is_some_and(|sync| sync.command_model)
                });
                if command_sync {
                    Self::WasmtimeSubtitleSync
                } else {
                    Self::LegacyReactor
                }
            }
            _ => Self::LegacyReactor,
        }
    }

    /// Select a runtime from the artifact marker before descriptor heuristics.
    pub(crate) fn for_artifact(descriptor: &PluginDescriptor, wasm: &[u8]) -> Result<Self, String> {
        if crate::wasmtime_host::component_host::is_indexer_component(wasm)? {
            return match descriptor.provider {
                scryer_plugin_sdk::ProviderDescriptor::Indexer(_) => {
                    Ok(Self::WasmtimeIndexerComponent)
                }
                _ => {
                    Err("WASI component artifacts are currently supported only for indexers".into())
                }
            };
        }
        if command_abi::command_abi_version(wasm)?.is_some() {
            return Ok(Self::WasmtimeCommand);
        }
        Ok(Self::for_descriptor(descriptor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_kind_selects_expected_runtime_backing() {
        let mut descriptor = PluginDescriptor {
            id: "archive".to_string(),
            name: "Archive".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::ArchiveExtractor(
                scryer_plugin_sdk::ArchiveExtractorDescriptor {
                    provider_type: "archive-extraction".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    capabilities: scryer_plugin_sdk::ArchiveExtractorCapabilities::default(),
                },
            ),
        };
        assert_eq!(
            PluginRuntimeBacking::for_descriptor(&descriptor),
            PluginRuntimeBacking::WasmtimeArchive
        );

        descriptor.provider = scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: "email".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: scryer_plugin_sdk::NotificationCapabilities::default(),
            },
        );
        assert_eq!(
            PluginRuntimeBacking::for_descriptor(&descriptor),
            PluginRuntimeBacking::LegacyReactor
        );

        descriptor.provider = scryer_plugin_sdk::ProviderDescriptor::Subtitle(
            scryer_plugin_sdk::SubtitleDescriptor {
                provider_type: "enhanced-subtitle-sync".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: scryer_plugin_sdk::SubtitleCapabilities {
                    mode: scryer_plugin_sdk::SubtitleProviderMode::Sync,
                    sync: Some(scryer_plugin_sdk::SubtitleSyncCapabilities {
                        command_model: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
        );
        assert_eq!(
            PluginRuntimeBacking::for_descriptor(&descriptor),
            PluginRuntimeBacking::WasmtimeSubtitleSync
        );
    }
}
