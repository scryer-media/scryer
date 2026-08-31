use super::*;

macro_rules! app_services_builder_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self
        }
    };
}

macro_rules! app_services_builder_required_setter {
    ($name:ident, $($field:ident).+, $config_field:ident, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = value;
            self.configured.$config_field = true;
            self
        }
    };
}

macro_rules! app_services_builder_runtime_feature_setter {
    ($name:ident, $($field:ident).+, $ty:ty) => {
        pub fn $name(mut self, value: $ty) -> Self {
            self.services.$($field).+ = RuntimeFeature::enabled(value);
            self
        }
    };
}

pub struct AppServicesBuilder {
    pub(super) services: AppServices,
    pub(super) runtime: AppRuntimeState,
    pub(super) configured: AppServicesBuildConfiguration,
}

#[derive(Default)]
pub(super) struct AppServicesBuildConfiguration {
    domain_events: bool,
    metadata_gateway: bool,
    library_scanner: bool,
    imports: bool,
    workflow_operations: bool,
    import_artifacts: bool,
    media_files: bool,
    acquisition_state: bool,
    download_submissions: bool,
    acquisition_scope_states: bool,
    rule_sets: bool,
    pp_scripts: bool,
    plugin_installations: bool,
    system_info: bool,
    title_images: bool,
    housekeeping: bool,
    pending_releases: bool,
    blocklist_repo: bool,
    subtitle_downloads: bool,
    job_runs: bool,
    library_probe_signatures: bool,
    library_scan_unmatched_items: bool,
}

impl AppServicesBuildConfiguration {
    fn missing_runtime_services(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();

        if !self.domain_events {
            missing.push("domain_events");
        }
        if !self.metadata_gateway {
            missing.push("metadata_gateway");
        }
        if !self.library_scanner {
            missing.push("library_scanner");
        }
        if !self.imports {
            missing.push("imports");
        }
        if !self.workflow_operations {
            missing.push("workflow_operations");
        }
        if !self.import_artifacts {
            missing.push("import_artifacts");
        }
        if !self.media_files {
            missing.push("media_files");
        }
        if !self.acquisition_state {
            missing.push("acquisition_state");
        }
        if !self.download_submissions {
            missing.push("download_submissions");
        }
        if !self.acquisition_scope_states {
            missing.push("acquisition_scope_states");
        }
        if !self.rule_sets {
            missing.push("rule_sets");
        }
        if !self.pp_scripts {
            missing.push("pp_scripts");
        }
        if !self.plugin_installations {
            missing.push("plugin_installations");
        }
        if !self.system_info {
            missing.push("system_info");
        }
        if !self.title_images {
            missing.push("title_images");
        }
        if !self.housekeeping {
            missing.push("housekeeping");
        }
        if !self.pending_releases {
            missing.push("pending_releases");
        }
        if !self.blocklist_repo {
            missing.push("blocklist_repo");
        }
        if !self.subtitle_downloads {
            missing.push("subtitle_downloads");
        }
        if !self.job_runs {
            missing.push("job_runs");
        }
        if !self.library_probe_signatures {
            missing.push("library_probe_signatures");
        }
        if !self.library_scan_unmatched_items {
            missing.push("library_scan_unmatched_items");
        }

        missing
    }
}

impl AppServicesBuilder {
    app_services_builder_setter!(
        with_indexer_error_repository,
        integrations.indexer_errors,
        Arc<dyn IndexerErrorRepository>
    );
    app_services_builder_runtime_feature_setter!(
        with_plugin_http_trust_runtime,
        config.plugin_http_trust_runtime,
        Arc<dyn PluginHttpTrustConfigRuntime>
    );

    pub fn with_runtime_environment<I, S>(
        mut self,
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.runtime =
            AppRuntimeState::new(build_lane, config_dir, supported_plugin_required_features);
        self
    }

    pub fn with_download_client_category_snapshot_store(
        mut self,
        store: DownloadClientCategorySnapshotStore,
    ) -> Self {
        self.runtime.acquisition.download_client_category_admission = store;
        self
    }

    pub fn with_supported_plugin_required_features<I, S>(mut self, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.runtime.environment.supported_plugin_required_features =
            normalize_supported_plugin_required_features(features);
        self
    }
}

impl AppServicesBuilder {
    app_services_builder_setter!(with_shows, catalog.shows, Arc<dyn ShowRepository>);
    app_services_builder_setter!(
        with_libraries,
        catalog.libraries,
        Arc<dyn LibraryRepository>
    );
    app_services_builder_setter!(
        with_media_requests,
        catalog.media_requests,
        Arc<dyn MediaRequestRepository>
    );
    app_services_builder_setter!(
        with_webauthn_store,
        identity.webauthn,
        Arc<dyn WebauthnRepository>
    );
    app_services_builder_setter!(with_totp_store, identity.totp, Arc<dyn TotpRepository>);
    app_services_builder_setter!(
        with_user_ui_settings_store,
        identity.ui_settings,
        Arc<dyn UserUiSettingsRepository>
    );
    app_services_builder_setter!(
        with_external_account_store,
        identity.external_accounts,
        Arc<dyn UserExternalAccountRepository>
    );
    app_services_builder_setter!(with_oauth_store, identity.oauth, Arc<dyn OAuthRepository>);
    pub fn with_customization_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PluginInstallationRepository
            + PostProcessingScriptRepository
            + RuleSetRepository
            + Send
            + Sync
            + 'static,
    {
        self.services.customization.rule_sets = store.clone();
        self.services.customization.pp_scripts = store.clone();
        self.services.customization.plugin_installations = store;
        self.configured.rule_sets = true;
        self.configured.pp_scripts = true;
        self.configured.plugin_installations = true;
        self
    }

    pub fn with_rule_set_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: RuleSetRepository + Send + Sync + 'static,
    {
        self.services.customization.rule_sets = store;
        self.configured.rule_sets = true;
        self
    }

    /// Not a required service: maintenance rules ship dark, and an assembly
    /// that never configures the store simply has no rules to read.
    pub fn with_maintenance_rule_set_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: MaintenanceRuleSetRepository + Send + Sync + 'static,
    {
        self.services.customization.maintenance_rule_sets = store;
        self
    }

    pub fn with_post_processing_script_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PostProcessingScriptRepository + Send + Sync + 'static,
    {
        self.services.customization.pp_scripts = store;
        self.configured.pp_scripts = true;
        self
    }

    pub fn with_plugin_installation_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: PluginInstallationRepository + Send + Sync + 'static,
    {
        self.services.customization.plugin_installations = store;
        self.configured.plugin_installations = true;
        self
    }

    pub fn with_plugin_descriptor_loader<T>(mut self, loader: Arc<T>) -> Self
    where
        T: PluginDescriptorLoader + Send + Sync + 'static,
    {
        self.services.customization.plugin_descriptor_loader = loader;
        self
    }

    pub fn with_notification_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: NotificationChannelRepository
            + NotificationSubscriptionRepository
            + Send
            + Sync
            + 'static,
    {
        let notification_channels: Arc<dyn NotificationChannelRepository> = store.clone();
        let notification_subscriptions: Arc<dyn NotificationSubscriptionRepository> = store;
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Store { .. } => {
                AppNotificationServices::Store {
                    notification_channels,
                    notification_subscriptions,
                }
            }
            AppNotificationServices::Provider {
                notification_provider,
            }
            | AppNotificationServices::Runtime {
                notification_provider,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider,
            },
        };
        self
    }

    app_services_builder_setter!(
        with_builtin_download_client_connection_tester,
        integrations.builtin_download_client_connection_tester,
        Arc<dyn BuiltinDownloadClientConnectionTester>
    );
    app_services_builder_setter!(
        with_indexer_proxy_config_store,
        integrations.indexer_proxy_configs,
        Arc<dyn IndexerProxyConfigRepository>
    );
    app_services_builder_setter!(
        with_scope_indexer_coverage_store,
        integrations.scope_indexer_coverage,
        Arc<dyn ScopeIndexerCoverageRepository>
    );
    app_services_builder_setter!(
        with_external_identity_verifier,
        integrations.external_identity_verifier,
        Arc<dyn ExternalIdentityVerifier>
    );
    app_services_builder_setter!(
        with_media_server_connection_store,
        integrations.media_server_connections,
        Arc<dyn MediaServerConnectionRepository>
    );
    app_services_builder_required_setter!(
        with_metadata_gateway,
        library.metadata_gateway,
        metadata_gateway,
        Arc<dyn MetadataGateway>
    );
    app_services_builder_setter!(
        with_discovery_store,
        library.discovery,
        Arc<dyn DiscoveryRepository>
    );
    app_services_builder_required_setter!(
        with_library_scanner,
        library.library_scanner,
        library_scanner,
        Arc<dyn LibraryScanner>
    );
    app_services_builder_setter!(
        with_library_renamer,
        library.library_renamer,
        Arc<dyn LibraryRenamer>
    );
    app_services_builder_setter!(
        with_media_analyzer,
        library.media_analyzer,
        Arc<dyn MediaAnalyzer>
    );
    app_services_builder_required_setter!(
        with_domain_events,
        events.domain_events,
        domain_events,
        Arc<dyn DomainEventRepository>
    );
    app_services_builder_required_setter!(
        with_imports,
        workflow.imports,
        imports,
        Arc<dyn ImportRepository>
    );
    app_services_builder_setter!(
        with_external_import_monitor_snapshots,
        workflow.external_import_monitor_snapshots,
        Arc<dyn ExternalImportMonitorSnapshotRepository>
    );
    app_services_builder_setter!(
        with_external_import_setup_secret_drafts,
        workflow.external_import_setup_secret_drafts,
        Arc<dyn ExternalImportSetupSecretDraftRepository>
    );
    app_services_builder_setter!(
        with_download_queue_commands,
        workflow.download_queue_commands,
        Arc<dyn DownloadQueueCommandRepository>
    );
    app_services_builder_required_setter!(
        with_workflow_operations,
        workflow.workflow_operations,
        workflow_operations,
        Arc<dyn WorkflowOperationRepository>
    );
    app_services_builder_required_setter!(
        with_import_artifacts,
        workflow.import_artifacts,
        import_artifacts,
        Arc<dyn ImportArtifactRepository>
    );
    app_services_builder_setter!(
        with_file_importer,
        workflow.file_importer,
        Arc<dyn FileImporter>
    );
    app_services_builder_required_setter!(
        with_media_files,
        library.media_files,
        media_files,
        Arc<dyn MediaFileRepository>
    );
    app_services_builder_required_setter!(
        with_download_submissions,
        workflow.download_submissions,
        download_submissions,
        Arc<dyn DownloadSubmissionRepository>
    );
    app_services_builder_setter!(
        with_download_registry,
        workflow.download_registry,
        Arc<dyn DownloadRegistryRepository>
    );
    app_services_builder_required_setter!(
        with_acquisition_state,
        workflow.acquisition_state,
        acquisition_state,
        Arc<dyn AcquisitionStateRepository>
    );
    app_services_builder_required_setter!(
        with_acquisition_scope_states,
        workflow.acquisition_scope_states,
        acquisition_scope_states,
        Arc<dyn AcquisitionScopeStateRepository>
    );
    app_services_builder_required_setter!(
        with_pending_releases,
        workflow.pending_releases,
        pending_releases,
        Arc<dyn PendingReleaseRepository>
    );
    app_services_builder_required_setter!(
        with_blocklist_repo,
        workflow.blocklist_repo,
        blocklist_repo,
        Arc<dyn BlocklistRepository>
    );
    app_services_builder_required_setter!(
        with_rule_sets,
        customization.rule_sets,
        rule_sets,
        Arc<dyn RuleSetRepository>
    );
    app_services_builder_required_setter!(
        with_pp_scripts,
        customization.pp_scripts,
        pp_scripts,
        Arc<dyn PostProcessingScriptRepository>
    );
    app_services_builder_required_setter!(
        with_plugin_installations,
        customization.plugin_installations,
        plugin_installations,
        Arc<dyn PluginInstallationRepository>
    );
    app_services_builder_required_setter!(
        with_system_info,
        config.system_info,
        system_info,
        Arc<dyn SystemInfoProvider>
    );
    app_services_builder_setter!(with_settings, config.settings, Arc<dyn SettingsRepository>);
    app_services_builder_setter!(
        with_logical_backup_exporter,
        config.logical_backup_exporter,
        Arc<dyn LogicalBackupExporter>
    );
    app_services_builder_setter!(with_backup_dir, config.backup_dir, PathBuf);
    app_services_builder_setter!(
        with_smg_registration_secret,
        config.smg_registration_secret,
        Option<String>
    );
    app_services_builder_setter!(with_smg_gateway_url, config.smg_gateway_url, Option<String>);
    app_services_builder_required_setter!(
        with_job_runs,
        events.job_runs,
        job_runs,
        Arc<dyn JobRunRepository>
    );
    app_services_builder_required_setter!(
        with_library_probe_signatures,
        library.library_probe_signatures,
        library_probe_signatures,
        Arc<dyn LibraryProbeRepository>
    );
    app_services_builder_required_setter!(
        with_library_scan_unmatched_items,
        library.library_scan_unmatched_items,
        library_scan_unmatched_items,
        Arc<dyn LibraryScanUnmatchedItemRepository>
    );
    app_services_builder_required_setter!(
        with_title_images,
        library.title_images,
        title_images,
        Arc<dyn TitleImageRepository>
    );
    app_services_builder_setter!(
        with_image_proxy,
        library.image_proxy,
        Arc<dyn ImageProxyRepository>
    );
    app_services_builder_setter!(
        with_image_proxy_cache_control,
        library.image_proxy_cache_control,
        Arc<dyn ImageProxyCacheControl>
    );
    app_services_builder_setter!(
        with_title_image_processor,
        library.title_image_processor,
        Arc<dyn TitleImageProcessor>
    );
    app_services_builder_required_setter!(
        with_housekeeping,
        workflow.housekeeping,
        housekeeping,
        Arc<dyn HousekeepingRepository>
    );
    app_services_builder_required_setter!(
        with_subtitle_downloads,
        workflow.subtitle_downloads,
        subtitle_downloads,
        Arc<dyn SubtitleDownloadRepository>
    );
    app_services_builder_setter!(
        with_staged_nzb_store,
        workflow.staged_nzb_store,
        Arc<dyn StagedNzbStore>
    );
    app_services_builder_setter!(
        with_staged_nzb_pipeline_limit,
        workflow.staged_nzb_pipeline_limit,
        Arc<Semaphore>
    );
    app_services_builder_setter!(
        with_indexer_stats,
        integrations.indexer_stats,
        Arc<dyn IndexerStatsTracker>
    );
    app_services_builder_setter!(
        with_upstream_scheduler,
        integrations.upstream_scheduler,
        Arc<dyn UpstreamScheduler>
    );
    app_services_builder_runtime_feature_setter!(
        with_indexer_caps_refresher,
        integrations.indexer_caps_refresher,
        Arc<dyn IndexerCapsSnapshotRefresher>
    );
    app_services_builder_runtime_feature_setter!(
        with_plugin_provider,
        integrations.plugin_provider,
        Arc<dyn IndexerPluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_download_client_plugin_provider,
        integrations.download_client_plugin_provider,
        Arc<dyn DownloadClientPluginProvider>
    );
    app_services_builder_setter!(
        with_seeding_profiles,
        integrations.seeding_profiles,
        Arc<dyn SeedingProfileRepository>
    );
    app_services_builder_runtime_feature_setter!(
        with_subtitle_provider_configs,
        integrations.subtitle_provider_configs,
        Arc<dyn SubtitleProviderConfigRepository>
    );
    app_services_builder_runtime_feature_setter!(
        with_subtitle_plugin_provider,
        integrations.subtitle_plugin_provider,
        Arc<dyn SubtitlePluginProvider>
    );
    app_services_builder_runtime_feature_setter!(
        with_archive_extractor_plugin_provider,
        integrations.archive_extractor_plugin_provider,
        Arc<dyn ArchiveExtractorPluginProvider>
    );
    pub fn with_notification_provider(
        mut self,
        value: Arc<dyn NotificationPluginProvider>,
    ) -> Self {
        self.services.notifications = match self.services.notifications {
            AppNotificationServices::Disabled | AppNotificationServices::Provider { .. } => {
                AppNotificationServices::Provider {
                    notification_provider: value,
                }
            }
            AppNotificationServices::Store {
                notification_channels,
                notification_subscriptions,
            }
            | AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                ..
            } => AppNotificationServices::Runtime {
                notification_channels,
                notification_subscriptions,
                notification_provider: value,
            },
        };
        self
    }
    pub fn with_tracked_download_handle(
        mut self,
        value: tracked_downloads::TrackedDownloadHandle,
    ) -> Self {
        self.runtime.acquisition.tracked_download_handle = Some(value);
        self
    }

    pub fn build(self) -> AppAssembly {
        let missing = self.configured.missing_runtime_services();
        assert!(
            missing.is_empty(),
            "AppServicesBuilder missing required runtime services: {}. Use build_partial_for_tests() only for intentionally partial test assemblies.",
            missing.join(", ")
        );
        self.finish()
    }

    fn finish(self) -> AppAssembly {
        AppAssembly {
            services: self.services,
            runtime: self.runtime,
        }
    }

    pub(crate) fn build_partial_for_tests(self) -> AppAssembly {
        self.finish()
    }
}
