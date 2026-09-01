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
    /// Versioned WASI Preview 2 component ABI for archive extractors.
    WasmtimeArchiveComponent,
    /// Native wasmtime command host for SDK 3.5 subtitle-sync plugins.
    WasmtimeSubtitleSync,
    /// Versioned native command ABI for all descriptor families.
    WasmtimeCommand,
    /// Versioned WASI Preview 2 component ABI for indexer plugins.
    WasmtimeIndexerComponent,
    /// Versioned WASI Preview 2 component ABI for subtitle providers.
    WasmtimeSubtitleComponent,
    /// Versioned WASI Preview 2 component ABI for download clients.
    WasmtimeDownloadClientComponent,
}

impl PluginRuntimeBacking {
    /// Backing selected from the descriptor kind.
    pub(crate) fn for_descriptor(descriptor: &PluginDescriptor) -> Self {
        match descriptor.kind() {
            PluginKind::ArchiveExtractor => Self::WasmtimeArchiveComponent,
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
        if crate::wasmtime_host::component_host::is_component_binary(wasm)? {
            return match descriptor.provider {
                scryer_plugin_sdk::ProviderDescriptor::Indexer(_) => {
                    Ok(Self::WasmtimeIndexerComponent)
                }
                scryer_plugin_sdk::ProviderDescriptor::ArchiveExtractor(_) => {
                    Ok(Self::WasmtimeArchiveComponent)
                }
                // The component funnel is per-family by descriptor, not one
                // catch-all: each family has its own world, so accepting an
                // artifact here is the same statement as "a host exists for
                // that world". Notifications join this arm as their world
                // lands.
                scryer_plugin_sdk::ProviderDescriptor::Subtitle(_) => {
                    Ok(Self::WasmtimeSubtitleComponent)
                }
                scryer_plugin_sdk::ProviderDescriptor::DownloadClient(_) => {
                    Ok(Self::WasmtimeDownloadClientComponent)
                }
                _ => Err(
                    "WASI component artifacts are currently supported only for indexers, \
                     archive extractors, subtitle providers, and download clients"
                        .into(),
                ),
            };
        }
        // Hard cut: the archive extractor has no core-module form any more.
        // Saying so here — rather than failing later on a missing crypto import
        // — is what turns a stale installed artifact into an actionable
        // "upgrade the plugin" message.
        if descriptor.kind() == PluginKind::ArchiveExtractor {
            return Err(crate::wasmtime_host::ARCHIVE_CORE_MODULE_REJECTED.to_string());
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

    fn archive_descriptor() -> PluginDescriptor {
        PluginDescriptor {
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
        }
    }

    fn download_client_descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "download-client".to_string(),
            name: "Download Client".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::DownloadClient(
                scryer_plugin_sdk::DownloadClientDescriptor {
                    provider_type: "fixture-download-client".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    accepted_inputs: Vec::new(),
                    isolation_modes: Vec::new(),
                    capabilities: scryer_plugin_sdk::DownloadClientCapabilities::default(),
                },
            ),
        }
    }

    /// The component funnel accepts download clients: a component artifact with
    /// a download-client descriptor selects the download-client component host
    /// rather than being refused as an unsupported family.
    #[test]
    fn component_download_client_artifacts_select_the_download_client_component_backing() {
        let component = wat::parse_str("(component)").expect("component WAT must parse");

        assert_eq!(
            PluginRuntimeBacking::for_artifact(&download_client_descriptor(), &component)
                .expect("a download client component must select a runtime"),
            PluginRuntimeBacking::WasmtimeDownloadClientComponent
        );
    }

    /// The component path is additive for this family too: an unmarked
    /// core-module download client still routes to the legacy reactor, and a
    /// command-marked one still routes to the command runtime. H4 deletes those
    /// paths; until then they must keep working.
    #[test]
    fn non_component_download_client_artifacts_keep_their_existing_backings() {
        let descriptor = download_client_descriptor();

        assert_eq!(
            PluginRuntimeBacking::for_artifact(
                &descriptor,
                &command_abi::test_support::unmarked_wasm()
            )
            .expect("a legacy download client artifact must select a runtime"),
            PluginRuntimeBacking::LegacyReactor
        );
        assert_eq!(
            PluginRuntimeBacking::for_artifact(
                &descriptor,
                &command_abi::test_support::command_marked_wasm()
            )
            .expect("a marked download client artifact must select a runtime"),
            PluginRuntimeBacking::WasmtimeCommand
        );
    }

    fn subtitle_descriptor(mode: SubtitleProviderMode) -> PluginDescriptor {
        PluginDescriptor {
            id: "subtitles".to_string(),
            name: "Subtitles".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::Subtitle(
                scryer_plugin_sdk::SubtitleDescriptor {
                    provider_type: "fixture-subtitles".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    capabilities: scryer_plugin_sdk::SubtitleCapabilities {
                        mode,
                        ..Default::default()
                    },
                },
            ),
        }
    }

    /// The component funnel is per-family: a subtitle component now selects the
    /// subtitle component host rather than being refused as an unsupported
    /// family.
    #[test]
    fn component_subtitle_artifacts_select_the_subtitle_component_backing() {
        let descriptor = subtitle_descriptor(SubtitleProviderMode::Catalog);
        let component = wat::parse_str("(component)").expect("component WAT must parse");

        assert_eq!(
            PluginRuntimeBacking::for_artifact(&descriptor, &component)
                .expect("a subtitle component must select a runtime"),
            PluginRuntimeBacking::WasmtimeSubtitleComponent
        );
    }

    /// The component path is additive: an unmarked core-module subtitle
    /// artifact still routes to the legacy reactor, and a command-marked one
    /// still routes to the command runtime.
    #[test]
    fn non_component_subtitle_artifacts_keep_their_existing_backings() {
        let descriptor = subtitle_descriptor(SubtitleProviderMode::Catalog);

        assert_eq!(
            PluginRuntimeBacking::for_artifact(
                &descriptor,
                &command_abi::test_support::unmarked_wasm()
            )
            .expect("a legacy subtitle artifact must select a runtime"),
            PluginRuntimeBacking::LegacyReactor
        );
        assert_eq!(
            PluginRuntimeBacking::for_artifact(
                &descriptor,
                &command_abi::test_support::command_marked_wasm()
            )
            .expect("a marked subtitle artifact must select a runtime"),
            PluginRuntimeBacking::WasmtimeCommand
        );
    }

    /// A sync-capable subtitle descriptor keeps selecting the subtitle-sync
    /// command runtime from a plain core module: the component funnel only
    /// fires on a component artifact.
    #[test]
    fn a_command_model_sync_subtitle_still_selects_the_sync_backing() {
        let mut descriptor = subtitle_descriptor(SubtitleProviderMode::Sync);
        if let scryer_plugin_sdk::ProviderDescriptor::Subtitle(subtitle) = &mut descriptor.provider {
            subtitle.capabilities.sync = Some(scryer_plugin_sdk::SubtitleSyncCapabilities {
                command_model: true,
                ..Default::default()
            });
        }

        assert_eq!(
            PluginRuntimeBacking::for_artifact(
                &descriptor,
                &command_abi::test_support::unmarked_wasm()
            )
            .expect("a sync subtitle artifact must select a runtime"),
            PluginRuntimeBacking::WasmtimeSubtitleSync
        );
    }

    /// The hard cut's operator-facing contract: a pre-component archive
    /// artifact is refused at runtime selection with an upgrade instruction,
    /// not deep inside instantiation with a missing-import trap.
    #[test]
    fn core_module_archive_artifacts_are_rejected_with_an_upgrade_diagnostic() {
        let descriptor = archive_descriptor();
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse");

        let error = PluginRuntimeBacking::for_artifact(&descriptor, &core_module)
            .expect_err("a core-module archive artifact must not select a runtime");

        assert_eq!(error, crate::wasmtime_host::ARCHIVE_CORE_MODULE_REJECTED);
        assert!(error.contains("wasm32-wasip2"), "{error}");
    }

    #[test]
    fn component_archive_artifacts_select_the_component_backing() {
        let descriptor = archive_descriptor();
        let component = wat::parse_str("(component)").expect("component WAT must parse");

        assert_eq!(
            PluginRuntimeBacking::for_artifact(&descriptor, &component)
                .expect("an archive component must select a runtime"),
            PluginRuntimeBacking::WasmtimeArchiveComponent
        );
    }

    #[test]
    fn descriptor_kind_selects_expected_runtime_backing() {
        let mut descriptor = archive_descriptor();
        assert_eq!(
            PluginRuntimeBacking::for_descriptor(&descriptor),
            PluginRuntimeBacking::WasmtimeArchiveComponent
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
