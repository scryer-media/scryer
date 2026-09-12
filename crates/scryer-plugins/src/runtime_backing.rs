//! Runtime-strategy seam.
//!
//! A `PluginInstanceSpec` describes an instantiation in runtime-agnostic terms;
//! a `PluginRuntimeBacking` says which runtime executes it. Adapters express
//! their sandbox/timeout requirements through the spec so every family's
//! component host is handed the same shape.
//!
//! There is exactly one runtime family left: WASI Preview 2 components. The
//! Extism-PDK reactor and the wasip1 stdin/stdout command ABI are gone, so a
//! non-component artifact is not a fallback — it is an artifact that has to be
//! rebuilt, and [`PluginRuntimeBacking::for_artifact`] says so in those words.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use scryer_plugin_sdk::{PluginDescriptor, ProviderDescriptor};

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
    /// Descriptor-scoped native host services for component guests.
    pub(crate) command_host: CommandHost,
}

/// Which component host executes a `PluginInstanceSpec`.
///
/// One variant per family world. There is no non-component variant: an artifact
/// that is not a component never reaches this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginRuntimeBacking {
    /// `scryer:archive/archive-extractor@1.0.0`.
    Archive,
    /// `scryer:indexer/indexer-plugin@1.1.0` (and its 1.0 predecessor).
    Indexer,
    /// `scryer:subtitle/subtitle-provider@1.0.0`.
    Subtitle,
    /// `scryer:download-client/download-client@1.0.0`.
    DownloadClient,
    /// `scryer:notification/notification@1.0.0`.
    Notification,
}

/// The operator-facing diagnostic for a pre-component artifact of `provider`'s
/// family.
///
/// This is the hard cut's whole user experience, and it is deliberately the
/// same sentence in every family: an installed plugin built against a removed
/// ABI must say what to do, not merely fail to instantiate deeper down with a
/// missing-import trap. The archive family's wording — which shipped first,
/// while the other four still had a legacy fallback — is the template.
pub(crate) fn core_module_rejected(provider: &ProviderDescriptor) -> String {
    let (family, world) = match provider {
        ProviderDescriptor::Indexer(_) => {
            ("indexer plugins", "scryer:indexer/indexer-plugin@1.1.0")
        }
        ProviderDescriptor::ArchiveExtractor(_) => (
            "archive extractor plugins",
            "scryer:archive/archive-extractor@1.0.0",
        ),
        ProviderDescriptor::Subtitle(_) => (
            "subtitle provider plugins",
            "scryer:subtitle/subtitle-provider@1.0.0",
        ),
        ProviderDescriptor::DownloadClient(_) => (
            "download client plugins",
            "scryer:download-client/download-client@1.0.0",
        ),
        ProviderDescriptor::Notification(_) => (
            "notification plugins",
            "scryer:notification/notification@1.0.0",
        ),
    };
    format!(
        "{family} must be WASI Preview 2 components (world {world}); this artifact is a legacy \
         core wasm module. Upgrade the plugin to a build that targets wasm32-wasip2."
    )
}

impl PluginRuntimeBacking {
    /// Select the component host for one artifact, or explain the upgrade.
    ///
    /// The component funnel is per-family by descriptor, not one catch-all:
    /// each family has its own world, so accepting an artifact here is the same
    /// statement as "a host exists for that world". Every `ProviderDescriptor`
    /// variant now names one — so this `match` has no fallback arm on purpose,
    /// and a family added to the SDK fails to compile here until its world
    /// exists rather than silently reporting "components are not supported for
    /// you".
    pub(crate) fn for_artifact(descriptor: &PluginDescriptor, wasm: &[u8]) -> Result<Self, String> {
        if !crate::wasmtime_host::component_host::is_component_binary(wasm)? {
            return Err(core_module_rejected(&descriptor.provider));
        }
        Ok(match descriptor.provider {
            ProviderDescriptor::Indexer(_) => Self::Indexer,
            ProviderDescriptor::ArchiveExtractor(_) => Self::Archive,
            ProviderDescriptor::Subtitle(_) => Self::Subtitle,
            ProviderDescriptor::DownloadClient(_) => Self::DownloadClient,
            ProviderDescriptor::Notification(_) => Self::Notification,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_plugin_sdk::SubtitleProviderMode;

    /// A minimal core module: exports a memory and `_start`, imports nothing.
    /// Every family's pre-component artifact reduces to this shape as far as
    /// runtime selection is concerned.
    fn core_module() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse")
    }

    fn component() -> Vec<u8> {
        wat::parse_str("(component)").expect("component WAT must parse")
    }

    fn descriptor_with(provider: ProviderDescriptor) -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture".to_string(),
            name: "Fixture".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider,
        }
    }

    fn indexer_descriptor() -> PluginDescriptor {
        descriptor_with(ProviderDescriptor::Indexer(
            scryer_plugin_sdk::IndexerDescriptor {
                provider_type: "fixture-indexer".to_string(),
                provider_aliases: Vec::new(),
                provider_profiles: Vec::new(),
                search_semantics_version: None,
                strategy_plan: None,
                source_kind: scryer_plugin_sdk::IndexerSourceKind::default(),
                capabilities: scryer_plugin_sdk::IndexerCapabilities::default(),
                scoring_policies: Vec::new(),
                config_fields: Vec::new(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
            },
        ))
    }

    fn archive_descriptor() -> PluginDescriptor {
        descriptor_with(ProviderDescriptor::ArchiveExtractor(
            scryer_plugin_sdk::ArchiveExtractorDescriptor {
                provider_type: "archive-extraction".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: scryer_plugin_sdk::ArchiveExtractorCapabilities::default(),
            },
        ))
    }

    fn download_client_descriptor() -> PluginDescriptor {
        descriptor_with(ProviderDescriptor::DownloadClient(
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
        ))
    }

    fn notification_descriptor() -> PluginDescriptor {
        descriptor_with(ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: "fixture-notification".to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: scryer_plugin_sdk::NotificationCapabilities::default(),
            },
        ))
    }

    fn subtitle_descriptor(mode: SubtitleProviderMode) -> PluginDescriptor {
        descriptor_with(ProviderDescriptor::Subtitle(
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
        ))
    }

    /// The component funnel is per-family: each family's component artifact
    /// selects that family's host, and nothing else.
    #[test]
    fn each_family_component_selects_its_own_component_backing() {
        let component = component();
        for (descriptor, expected) in [
            (indexer_descriptor(), PluginRuntimeBacking::Indexer),
            (archive_descriptor(), PluginRuntimeBacking::Archive),
            (
                subtitle_descriptor(SubtitleProviderMode::Catalog),
                PluginRuntimeBacking::Subtitle,
            ),
            (
                download_client_descriptor(),
                PluginRuntimeBacking::DownloadClient,
            ),
            (
                notification_descriptor(),
                PluginRuntimeBacking::Notification,
            ),
        ] {
            assert_eq!(
                PluginRuntimeBacking::for_artifact(&descriptor, &component)
                    .expect("a component artifact must select a runtime"),
                expected,
                "{} selected the wrong component host",
                descriptor.provider_type()
            );
        }
    }

    /// The hard cut's operator-facing contract, in EVERY family: a
    /// pre-component artifact is refused at runtime selection with an upgrade
    /// instruction, not silently ignored and not failed deep inside
    /// instantiation with a missing-import trap.
    #[test]
    fn every_family_rejects_a_core_module_with_an_upgrade_diagnostic() {
        let core_module = core_module();
        for (descriptor, world) in [
            (indexer_descriptor(), "scryer:indexer/indexer-plugin@1.1.0"),
            (
                archive_descriptor(),
                "scryer:archive/archive-extractor@1.0.0",
            ),
            (
                subtitle_descriptor(SubtitleProviderMode::Catalog),
                "scryer:subtitle/subtitle-provider@1.0.0",
            ),
            (
                download_client_descriptor(),
                "scryer:download-client/download-client@1.0.0",
            ),
            (
                notification_descriptor(),
                "scryer:notification/notification@1.0.0",
            ),
        ] {
            let error = PluginRuntimeBacking::for_artifact(&descriptor, &core_module)
                .expect_err("a core-module artifact must not select a runtime");

            assert!(error.contains(world), "{error}");
            assert!(error.contains("wasm32-wasip2"), "{error}");
            assert!(error.contains("Upgrade the plugin"), "{error}");
        }
    }

    /// A sync-capable subtitle descriptor gets the same answer as any other
    /// subtitle artifact: the mode is no longer a runtime selector, because the
    /// wasip1 subtitle-sync command host it used to select is gone.
    #[test]
    fn a_sync_subtitle_descriptor_selects_no_special_runtime() {
        let descriptor = subtitle_descriptor(SubtitleProviderMode::Sync);

        assert_eq!(
            PluginRuntimeBacking::for_artifact(&descriptor, &component())
                .expect("a subtitle component must select a runtime"),
            PluginRuntimeBacking::Subtitle
        );
        assert!(
            PluginRuntimeBacking::for_artifact(&descriptor, &core_module())
                .expect_err("a core-module sync subtitle must not select a runtime")
                .contains("wasm32-wasip2")
        );
    }
}
