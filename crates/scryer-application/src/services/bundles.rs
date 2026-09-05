use super::*;

#[derive(Clone)]
pub struct AppAssembly {
    pub services: AppServices,
    pub runtime: AppRuntimeState,
}

#[derive(Clone)]
pub struct AppCatalogServices {
    pub(crate) titles: Arc<dyn TitleRepository>,
    pub(crate) shows: Arc<dyn ShowRepository>,
    pub(crate) libraries: Arc<dyn LibraryRepository>,
    pub(crate) media_requests: Arc<dyn MediaRequestRepository>,
    /// Title leases and keep claims (spec 0003 FR-041…FR-044). Catalog-side
    /// because a claim is a hold on a title, not on the request that produced
    /// it: an operator pin has no request at all.
    pub(crate) lifecycle_claims: Arc<dyn crate::ports::LifecycleClaimRepository>,
}

#[derive(Clone)]
pub struct AppIdentityServices {
    pub(crate) users: Arc<dyn UserRepository>,
    pub(crate) ui_settings: Arc<dyn UserUiSettingsRepository>,
    pub(crate) external_accounts: Arc<dyn UserExternalAccountRepository>,
    pub(crate) webauthn: Arc<dyn WebauthnRepository>,
    pub(crate) totp: Arc<dyn TotpRepository>,
    pub(crate) oauth: Arc<dyn OAuthRepository>,
}

#[derive(Clone)]
pub struct AppEventServices {
    pub(crate) domain_events: Arc<dyn DomainEventRepository>,
    pub(crate) job_runs: Arc<dyn JobRunRepository>,
}

#[derive(Clone, Default)]
pub enum RuntimeFeature<T> {
    #[default]
    Disabled,
    Enabled(T),
}

impl<T> RuntimeFeature<T> {
    pub fn enabled(value: T) -> Self {
        Self::Enabled(value)
    }

    pub fn available(&self) -> Option<&T> {
        match self {
            Self::Disabled => None,
            Self::Enabled(value) => Some(value),
        }
    }
}

#[derive(Clone)]
pub struct AppLibraryServices {
    pub(crate) metadata_gateway: Arc<dyn MetadataGateway>,
    pub(crate) discovery: Arc<dyn DiscoveryRepository>,
    pub(crate) library_scanner: Arc<dyn LibraryScanner>,
    pub(crate) library_renamer: Arc<dyn LibraryRenamer>,
    pub(crate) media_files: Arc<dyn MediaFileRepository>,
    pub(crate) media_analyzer: Arc<dyn MediaAnalyzer>,
    pub(crate) title_images: Arc<dyn TitleImageRepository>,
    pub(crate) image_proxy: Arc<dyn ImageProxyRepository>,
    pub(crate) image_proxy_cache_control: Arc<dyn ImageProxyCacheControl>,
    pub(crate) title_image_processor: Arc<dyn TitleImageProcessor>,
    pub(crate) library_probe_signatures: Arc<dyn LibraryProbeRepository>,
    pub(crate) library_scan_unmatched_items: Arc<dyn LibraryScanUnmatchedItemRepository>,
    /// Persisted location-operation state, read by the ownership guard (FR-084).
    pub(crate) location_operations: Arc<dyn crate::ports::LocationOperationRepository>,
    /// The US7 merge engine's Group 0 read and Groups 1–5 transaction. Read at
    /// preview time to plan the merge (FR-066/FR-071) and again at the title
    /// checkpoint to run it (FR-063–FR-067).
    pub(crate) title_merges: Arc<dyn crate::location::merge::engine::TitleMergeRepository>,
}

#[derive(Clone)]
pub struct AppIntegrationServices {
    pub(crate) indexer_configs: Arc<dyn IndexerConfigRepository>,
    pub(crate) indexer_errors: Arc<dyn IndexerErrorRepository>,
    pub(crate) proxy_configs: Arc<dyn ProxyConfigRepository>,
    pub(crate) scope_indexer_coverage: Arc<dyn ScopeIndexerCoverageRepository>,
    pub(crate) indexer_caps_refresher: RuntimeFeature<Arc<dyn IndexerCapsSnapshotRefresher>>,
    pub(crate) indexer_client: Arc<dyn IndexerClient>,
    pub(crate) download_client: Arc<dyn DownloadClient>,
    pub(crate) builtin_download_client_connection_tester:
        Arc<dyn BuiltinDownloadClientConnectionTester>,
    pub(crate) download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub(crate) seeding_profiles: Arc<dyn SeedingProfileRepository>,
    pub(crate) subtitle_provider_configs: RuntimeFeature<Arc<dyn SubtitleProviderConfigRepository>>,
    pub(crate) external_identity_verifier: Arc<dyn ExternalIdentityVerifier>,
    pub(crate) media_server_connections: Arc<dyn MediaServerConnectionRepository>,
    pub(crate) indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub(crate) upstream_scheduler: Arc<dyn UpstreamScheduler>,
    pub(crate) plugin_provider: RuntimeFeature<Arc<dyn IndexerPluginProvider>>,
    pub(crate) download_client_plugin_provider:
        RuntimeFeature<Arc<dyn DownloadClientPluginProvider>>,
    pub(crate) subtitle_plugin_provider: RuntimeFeature<Arc<dyn SubtitlePluginProvider>>,
    pub(crate) archive_extractor_plugin_provider:
        RuntimeFeature<Arc<dyn ArchiveExtractorPluginProvider>>,
    /// srrdb.com filename recovery for obfuscated automatic imports. Disabled
    /// in every assembly that does not wire the production adapter, which is
    /// indistinguishable from the admin setting being off.
    pub(crate) srrdb_filename_lookup: RuntimeFeature<Arc<dyn crate::ports::SrrdbFilenameLookup>>,
    /// Live playback observation across the media-server connections above
    /// (RFC 137 §9.10, WP-G). Read-only; consulted by maintenance safety.
    pub(crate) media_server_playback_probe: Arc<dyn crate::ports::MediaServerPlaybackProbe>,
    /// Per-participant played-item reads from the same connections
    /// (RFC 137 §7.3, WP-M). Provider dispatch lives inside the adapter.
    pub(crate) media_server_signal_source: Arc<dyn crate::ports::MediaServerSignalSource>,
    /// Durable normalized watch signals produced by that adapter.
    pub(crate) media_server_signals: Arc<dyn crate::ports::MediaServerSignalRepository>,
}

#[derive(Clone)]
pub struct AppWorkflowServices {
    pub(crate) imports: Arc<dyn ImportRepository>,
    pub(crate) external_import_monitor_snapshots: Arc<dyn ExternalImportMonitorSnapshotRepository>,
    pub(crate) external_import_setup_secret_drafts:
        Arc<dyn ExternalImportSetupSecretDraftRepository>,
    pub(crate) download_queue_commands: Arc<dyn DownloadQueueCommandRepository>,
    pub(crate) workflow_operations: Arc<dyn WorkflowOperationRepository>,
    pub(crate) file_importer: Arc<dyn FileImporter>,
    pub(crate) import_artifacts: Arc<dyn ImportArtifactRepository>,
    pub(crate) release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub(crate) acquisition_state: Arc<dyn AcquisitionStateRepository>,
    pub(crate) download_registry: Arc<dyn DownloadRegistryRepository>,
    pub(crate) download_submissions: Arc<dyn DownloadSubmissionRepository>,
    pub(crate) acquisition_scope_states: Arc<dyn AcquisitionScopeStateRepository>,
    pub(crate) housekeeping: Arc<dyn HousekeepingRepository>,
    pub(crate) pending_releases: Arc<dyn PendingReleaseRepository>,
    pub(crate) blocklist_repo: Arc<dyn BlocklistRepository>,
    pub(crate) subtitle_downloads: Arc<dyn SubtitleDownloadRepository>,
    pub(crate) staged_nzb_store: Arc<dyn StagedNzbStore>,
    pub(crate) staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppConfigServices {
    pub(crate) settings: Arc<dyn SettingsRepository>,
    pub(crate) quality_profiles: Arc<dyn QualityProfileRepository>,
    pub(crate) system_info: Arc<dyn SystemInfoProvider>,
    pub(crate) plugin_http_trust_runtime: RuntimeFeature<Arc<dyn PluginHttpTrustConfigRuntime>>,
    pub(crate) logical_backup_exporter: Arc<dyn LogicalBackupExporter>,
    pub(crate) backup_dir: PathBuf,
    pub(crate) smg_registration_secret: Option<String>,
    pub(crate) smg_gateway_url: Option<String>,
}

#[derive(Clone)]
pub struct AppCustomizationServices {
    pub(crate) rule_sets: Arc<dyn RuleSetRepository>,
    pub(crate) maintenance_rule_sets: Arc<dyn MaintenanceRuleSetRepository>,
    pub(crate) maintenance_evaluation: Arc<dyn crate::ports::MaintenanceEvaluationRepository>,
    /// User-authored request rules (spec 0003). Ships dark behind the same
    /// experimental gate as maintenance.
    pub(crate) request_rule_sets: Arc<dyn crate::ports::RequestRuleSetRepository>,
    /// Append-only traces of every request evaluation (spec 0003 FR-016).
    pub(crate) request_rule_decisions: Arc<dyn crate::ports::RequestRuleDecisionRepository>,
    /// The compiled request-rules engine and the per-rule library scope map,
    /// rebuilt by every mutating authoring call. It lives beside
    /// [`Self::user_rules`] — the release engine — because both are compiled
    /// artefacts of stored sources, swapped under a lock rather than rebuilt per
    /// evaluation.
    pub(crate) request_rules_engine: crate::request_rules::RequestRulesEngineHandle,
    pub(crate) pp_scripts: Arc<dyn PostProcessingScriptRepository>,
    pub(crate) plugin_installations: Arc<dyn PluginInstallationRepository>,
    pub(crate) plugin_descriptor_loader: Arc<dyn PluginDescriptorLoader>,
    pub(crate) user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
}

#[derive(Clone)]
pub enum AppNotificationServices {
    Disabled,
    Store {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
    },
    Provider {
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
    Runtime {
        notification_channels: Arc<dyn NotificationChannelRepository>,
        notification_subscriptions: Arc<dyn NotificationSubscriptionRepository>,
        notification_provider: Arc<dyn NotificationPluginProvider>,
    },
}

impl AppNotificationServices {
    pub fn notification_channels(&self) -> Option<&Arc<dyn NotificationChannelRepository>> {
        match self {
            Self::Store {
                notification_channels,
                ..
            }
            | Self::Runtime {
                notification_channels,
                ..
            } => Some(notification_channels),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_subscriptions(
        &self,
    ) -> Option<&Arc<dyn NotificationSubscriptionRepository>> {
        match self {
            Self::Store {
                notification_subscriptions,
                ..
            }
            | Self::Runtime {
                notification_subscriptions,
                ..
            } => Some(notification_subscriptions),
            Self::Disabled | Self::Provider { .. } => None,
        }
    }

    pub fn notification_provider(&self) -> Option<&Arc<dyn NotificationPluginProvider>> {
        match self {
            Self::Provider {
                notification_provider,
            }
            | Self::Runtime {
                notification_provider,
                ..
            } => Some(notification_provider),
            Self::Disabled | Self::Store { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct AppServices {
    pub(crate) catalog: AppCatalogServices,
    pub(crate) identity: AppIdentityServices,
    pub(crate) events: AppEventServices,
    pub(crate) library: AppLibraryServices,
    pub(crate) integrations: AppIntegrationServices,
    pub(crate) workflow: AppWorkflowServices,
    pub(crate) config: AppConfigServices,
    pub(crate) customization: AppCustomizationServices,
    pub(crate) notifications: AppNotificationServices,
}

impl AppServices {
    #[expect(
        clippy::too_many_arguments,
        reason = "service assembly intentionally enumerates each root dependency explicitly"
    )]
    pub fn builder(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        backup_dir: impl Into<PathBuf>,
    ) -> AppServicesBuilder {
        AppServicesBuilder {
            services: Self::with_placeholder_defaults(
                titles,
                shows,
                users,
                indexer_configs,
                indexer_client,
                download_client,
                download_client_configs,
                release_attempts,
                settings,
                quality_profiles,
                backup_dir.into(),
            ),
            runtime: AppRuntimeState::default(),
            configured: AppServicesBuildConfiguration::default(),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "placeholder wiring intentionally follows the full service dependency surface"
    )]
    fn with_placeholder_defaults(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        backup_dir: PathBuf,
    ) -> Self {
        Self {
            catalog: AppCatalogServices {
                titles,
                shows,
                libraries: Arc::new(NullLibraryRepository),
                media_requests: Arc::new(NullMediaRequestRepository),
                lifecycle_claims: Arc::new(null_repositories::NullLifecycleClaimRepository),
            },
            identity: AppIdentityServices {
                users,
                ui_settings: Arc::new(null_repositories::NullUserUiSettingsRepository),
                external_accounts: Arc::new(null_repositories::NullUserExternalAccountRepository),
                webauthn: Arc::new(null_repositories::NullWebauthnRepository),
                totp: Arc::new(null_repositories::NullTotpRepository),
                oauth: Arc::new(null_repositories::NullOAuthRepository),
            },
            events: AppEventServices {
                domain_events: Arc::new(NullDomainEventRepository),
                job_runs: Arc::new(null_repositories::NullJobRunRepository),
            },
            library: AppLibraryServices {
                metadata_gateway: Arc::new(crate::library_scan::NullMetadataGateway),
                discovery: Arc::new(null_repositories::NullDiscoveryRepository),
                library_scanner: Arc::new(crate::library_scan::NullLibraryScanner),
                library_renamer: Arc::new(crate::library_rename::NullLibraryRenamer),
                media_files: Arc::new(NullMediaFileRepository),
                media_analyzer: Arc::new(NativeMediaAnalyzer),
                title_images: Arc::new(NullTitleImageRepository),
                image_proxy: Arc::new(null_repositories::NullImageProxyRepository),
                image_proxy_cache_control: Arc::new(null_repositories::NullImageProxyCacheControl),
                title_image_processor: Arc::new(NullTitleImageProcessor),
                library_probe_signatures: Arc::new(null_repositories::NullLibraryProbeRepository),
                library_scan_unmatched_items: Arc::new(
                    null_repositories::NullLibraryScanUnmatchedItemRepository,
                ),
                location_operations: Arc::new(null_repositories::NullLocationOperationRepository),
                title_merges: Arc::new(null_repositories::NullTitleMergeRepository),
            },
            integrations: AppIntegrationServices {
                indexer_configs,
                indexer_errors: Arc::new(null_repositories::NullIndexerErrorRepository),
                proxy_configs: Arc::new(null_repositories::NullProxyConfigRepository),
                scope_indexer_coverage: Arc::new(
                    null_repositories::NullScopeIndexerCoverageRepository,
                ),
                indexer_caps_refresher: RuntimeFeature::Disabled,
                indexer_client,
                download_client,
                builtin_download_client_connection_tester: Arc::new(
                    null_repositories::NullBuiltinDownloadClientConnectionTester,
                ),
                download_client_configs,
                seeding_profiles: Arc::new(null_repositories::NullSeedingProfileRepository),
                subtitle_provider_configs: RuntimeFeature::Disabled,
                external_identity_verifier: Arc::new(
                    null_repositories::NullExternalIdentityVerifier,
                ),
                media_server_connections: Arc::new(
                    null_repositories::NullMediaServerConnectionRepository,
                ),
                indexer_stats: Arc::new(NullIndexerStatsTracker),
                upstream_scheduler: Arc::new(NullUpstreamScheduler),
                plugin_provider: RuntimeFeature::Disabled,
                download_client_plugin_provider: RuntimeFeature::Disabled,
                subtitle_plugin_provider: RuntimeFeature::Disabled,
                archive_extractor_plugin_provider: RuntimeFeature::Disabled,
                srrdb_filename_lookup: RuntimeFeature::Disabled,
                media_server_playback_probe: Arc::new(
                    null_repositories::NullMediaServerPlaybackProbe,
                ),
                media_server_signal_source: Arc::new(
                    null_repositories::NullMediaServerSignalSource,
                ),
                media_server_signals: Arc::new(null_repositories::NullMediaServerSignalRepository),
            },
            workflow: AppWorkflowServices {
                imports: Arc::new(NullImportRepository),
                external_import_monitor_snapshots: Arc::new(
                    null_repositories::NullExternalImportMonitorSnapshotRepository,
                ),
                external_import_setup_secret_drafts: Arc::new(
                    null_repositories::NullExternalImportSetupSecretDraftRepository,
                ),
                download_queue_commands: Arc::new(
                    null_repositories::NullDownloadQueueCommandRepository,
                ),
                workflow_operations: Arc::new(NullWorkflowOperationRepository),
                file_importer: Arc::new(NullFileImporter),
                import_artifacts: Arc::new(null_repositories::NullImportArtifactRepository),
                release_attempts,
                acquisition_state: Arc::new(NullAcquisitionStateRepository),
                download_registry: Arc::new(null_repositories::NullDownloadRegistryRepository),
                download_submissions: Arc::new(NullDownloadSubmissionRepository),
                acquisition_scope_states: Arc::new(NullAcquisitionScopeStateRepository),
                housekeeping: Arc::new(NullHousekeepingRepository),
                pending_releases: Arc::new(NullPendingReleaseRepository),
                blocklist_repo: Arc::new(NullBlocklistRepository),
                subtitle_downloads: Arc::new(null_repositories::NullSubtitleDownloadRepository),
                staged_nzb_store: Arc::new(null_repositories::NullStagedNzbStore),
                staged_nzb_pipeline_limit: Arc::new(Semaphore::new(4)),
            },
            config: AppConfigServices {
                settings,
                quality_profiles,
                system_info: Arc::new(NullSystemInfoProvider),
                plugin_http_trust_runtime: RuntimeFeature::Disabled,
                logical_backup_exporter: Arc::new(NullLogicalBackupExporter),
                backup_dir,
                smg_registration_secret: None,
                smg_gateway_url: None,
            },
            customization: AppCustomizationServices {
                rule_sets: Arc::new(NullRuleSetRepository),
                maintenance_rule_sets: Arc::new(
                    null_repositories::NullMaintenanceRuleSetRepository,
                ),
                maintenance_evaluation: Arc::new(
                    null_repositories::NullMaintenanceEvaluationRepository,
                ),
                request_rule_sets: Arc::new(null_repositories::NullRequestRuleSetRepository),
                request_rule_decisions: Arc::new(
                    null_repositories::NullRequestRuleDecisionRepository,
                ),
                request_rules_engine: Arc::new(std::sync::RwLock::new(
                    crate::request_rules::RequestRulesEngineCache::default(),
                )),
                pp_scripts: Arc::new(NullPostProcessingScriptRepository),
                plugin_installations: Arc::new(NullPluginInstallationRepository),
                plugin_descriptor_loader: Arc::new(NullPluginDescriptorLoader),
                user_rules: Arc::new(std::sync::RwLock::new(
                    scryer_rules::UserRulesEngine::empty(),
                )),
            },
            notifications: AppNotificationServices::Disabled,
        }
    }
}
