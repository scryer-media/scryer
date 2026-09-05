#[cfg(test)]
mod indexer_config_reconciliation_tests {
    use super::*;

    fn field(
        key: &str,
        role: Option<scryer_domain::ConfigFieldRole>,
        field_type: scryer_domain::ConfigFieldType,
        required: bool,
        default_value: Option<&str>,
    ) -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            key: key.to_string(),
            label: key.to_string(),
            field_type,
            required,
            default_value: default_value.map(str::to_string),
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role,
            host_binding: None,
            options: Vec::new(),
            help_text: None,
            ..Default::default()
        }
    }

    #[test]
    fn auto_create_allows_defaulted_connection_url_only() {
        let fields = vec![field(
            "base_url",
            Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            scryer_domain::ConfigFieldType::String,
            true,
            Some("https://indexer.example"),
        )];

        assert!(indexer_config_can_be_auto_created(&fields));
    }

    #[test]
    fn auto_create_skips_required_user_secret_without_default() {
        let fields = vec![
            field(
                "base_url",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                scryer_domain::ConfigFieldType::String,
                true,
                Some("https://indexer.example"),
            ),
            field(
                "api_key",
                None,
                scryer_domain::ConfigFieldType::Password,
                true,
                None,
            ),
        ];

        assert!(!indexer_config_can_be_auto_created(&fields));
    }
}
#[cfg(test)]
mod sdk_compatibility_tests {
    use super::*;

    fn current_sdk_minor_line_constraint() -> String {
        let sdk_version = semver::Version::parse(SDK_VERSION).expect("valid SDK_VERSION");
        format!(
            ">={}.{}.0, <{}.{}.0",
            sdk_version.major,
            sdk_version.minor,
            sdk_version.major,
            sdk_version.minor + 1
        )
    }

    #[test]
    fn downloaded_plugin_release_host_compatibility_rejects_legacy_minor_line_constraint() {
        let release = DownloadedPluginReleaseContract {
            version: "0.2.0".to_string(),
            sdk_version: Some("2.3.0".to_string()),
            sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
            scryer_constraint: None,
        };

        assert!(!downloaded_plugin_release_is_host_compatible(
            "jellyfin", &release
        ));
    }

    #[test]
    fn downloaded_plugin_release_preserves_explicit_minor_line_override() {
        let release = DownloadedPluginReleaseContract {
            version: "0.2.0".to_string(),
            sdk_version: Some(SDK_VERSION.to_string()),
            sdk_constraint: current_sdk_minor_line_constraint(),
            scryer_constraint: None,
        };

        assert_eq!(
            normalized_release_sdk_constraint(&release),
            current_sdk_minor_line_constraint()
        );
        assert!(downloaded_plugin_release_is_host_compatible(
            "jellyfin", &release
        ));
    }

    #[test]
    fn installation_sdk_contract_is_host_compatible_rejects_legacy_minor_line_constraint() {
        let installation = PluginInstallation {
            id: "install-1".to_string(),
            plugin_id: "jellyfin".to_string(),
            name: "Jellyfin".to_string(),
            description: "Jellyfin notifications".to_string(),
            version: "0.2.0".to_string(),
            sdk_version: "2.3.0".to_string(),
            sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
            scryer_constraint: None,
            plugin_type: "notification".to_string(),
            provider_type: "jellyfin".to_string(),
            source_kind: PluginSourceKind::Downloaded,
            is_enabled: true,
            is_builtin: false,
            wasm_encoding: PluginWasmEncoding::Identity,
            wasm_digest_algo: None,
            source_url: None,
            support_tier: PluginSupportTier::Official,
            publisher: None,
            docs_url: None,
            source_repo: None,
            manifest_url: None,
            wasm_digest: None,
            artifact_digest: None,
            descriptor_json: None,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!installation_sdk_contract_is_host_compatible(&installation));
    }

    #[test]
    fn latest_compatible_child_release_skips_legacy_minor_line_constraint() {
        let catalog = ChildCatalog {
            schema_version: "scryer.plugin.child_catalog.v2".to_string(),
            id: "email".to_string(),
            name: "Email".to_string(),
            description: "Email notifications".to_string(),
            plugin_type: "notification".to_string(),
            provider_type: "email".to_string(),
            publisher: "scryer".to_string(),
            support_tier: PluginSupportTier::Official,
            docs_url: "https://github.com/scryer-media/scryer-plugins".to_string(),
            source_repo: "https://github.com/scryer-media/scryer-plugins".to_string(),
            releases: vec![
                ChildCatalogRelease {
                    version: "0.1.0".to_string(),
                    sdk_constraint: ">=2.3.0, <3.0.0".to_string(),
                    artifact_manifest_url: "https://example.invalid/email-v0.1.0.manifest.json"
                        .to_string(),
                },
                ChildCatalogRelease {
                    version: "0.2.0".to_string(),
                    sdk_constraint: current_sdk_minor_line_constraint(),
                    artifact_manifest_url: "https://example.invalid/email-v0.2.0.manifest.json"
                        .to_string(),
                },
            ],
        };

        let selected = latest_compatible_child_release(&catalog).expect("compatible release");

        assert_eq!(selected.version, "0.2.0");
    }
}
#[cfg(test)]
mod catalog_artifact_selection_tests {
    use super::*;
    use crate::services::RuntimePerformanceClass;
    use std::collections::HashSet;

    /// A host capability set: the WASI target this build declares plus the
    /// wasm features named. Production builds this the same way, via
    /// `scryer_plugins::detect_plugin_runtime_capabilities`.
    fn host_capabilities(features: &[&str]) -> HashSet<String> {
        let mut capabilities = features
            .iter()
            .map(|feature| (*feature).to_string())
            .collect::<HashSet<_>>();
        capabilities.insert(CATALOG_V3_RUNTIME_WASIP2.to_string());
        capabilities
    }

    fn artifact(required_features: &[&str], url: &str) -> CatalogV3PluginArtifact {
        CatalogV3PluginArtifact {
            runtime: CATALOG_V3_RUNTIME_WASIP2.to_string(),
            required_features: required_features
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            url: url.to_string(),
            mirror_urls: Vec::new(),
            signature_url: format!("{url}.sig"),
            signature_mirror_urls: Vec::new(),
            digests: vec!["sha256:artifact".to_string()],
            wasm_digests: vec!["sha256:wasm".to_string()],
            bytes: 1234,
        }
    }

    fn artifact_with_digests(
        required_features: &[&str],
        url: &str,
        artifact_digest: &str,
        wasm_digest: &str,
    ) -> CatalogV3PluginArtifact {
        let mut artifact = artifact(required_features, url);
        artifact.digests = vec![artifact_digest.to_string()];
        artifact.wasm_digests = vec![wasm_digest.to_string()];
        artifact
    }

    fn release(artifacts: Vec<CatalogV3PluginArtifact>) -> CatalogV3PluginRelease {
        CatalogV3PluginRelease {
            version: "1.0.0".to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            min_scryer_version: None,
            max_scryer_version: None,
            artifacts,
        }
    }

    fn plugin(releases: Vec<CatalogV3PluginRelease>) -> CatalogV3PluginEntry {
        CatalogV3PluginEntry {
            id: "alpha".to_string(),
            name: "Alpha".to_string(),
            description: "Alpha plugin".to_string(),
            plugin_type: "indexer".to_string(),
            provider_type: "alpha".to_string(),
            publisher: "scryer".to_string(),
            support_tier: PluginSupportTier::Official,
            status: PluginLifecycleStatus::Active,
            docs_url: "https://example.invalid/docs".to_string(),
            source_repo: "https://github.com/scryer-media/alpha".to_string(),
            required_signer: RequiredSigner {
                github_repository: "scryer-media/alpha".to_string(),
                github_workflow: None,
                github_ref: None,
            },
            releases,
        }
    }

    fn installed_plugin(
        version: &str,
        wasm_digest: Option<&str>,
        artifact_digest: Option<&str>,
        source_url: Option<&str>,
    ) -> PluginInstallation {
        let now = Utc::now();
        let (wasm_digest_algo, wasm_digest) = wasm_digest
            .map(parse_digest_string)
            .transpose()
            .unwrap()
            .map(|(algorithm, digest)| (Some(algorithm), Some(digest)))
            .unwrap_or((None, None));
        PluginInstallation {
            id: "install-1".to_string(),
            plugin_id: "alpha".to_string(),
            name: "Alpha".to_string(),
            description: "Alpha plugin".to_string(),
            version: version.to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            scryer_constraint: None,
            plugin_type: "indexer".to_string(),
            provider_type: "alpha".to_string(),
            source_kind: PluginSourceKind::Downloaded,
            is_enabled: true,
            is_builtin: false,
            wasm_encoding: PluginWasmEncoding::Zstd,
            wasm_digest_algo,
            source_url: source_url.map(str::to_string),
            support_tier: PluginSupportTier::Official,
            publisher: None,
            docs_url: None,
            source_repo: None,
            manifest_url: source_url.map(str::to_string),
            wasm_digest,
            artifact_digest: artifact_digest.map(str::to_string),
            descriptor_json: None,
            installed_at: now,
            updated_at: now,
        }
    }

    fn resolution(
        release: CatalogV3PluginRelease,
        artifact: CatalogV3PluginArtifact,
    ) -> CatalogPluginResolution {
        CatalogPluginResolution {
            catalog_entry: plugin(vec![release.clone()]),
            release,
            artifact,
            source_kind: PluginSourceKind::Downloaded,
            effective_support_tier: PluginSupportTier::Official,
            github_repo: GitHubRepo::parse("https://github.com/scryer-media/alpha").unwrap(),
        }
    }

    #[test]
    fn catalog_selection_skips_sdk2_release_and_selects_sdk3_release() {
        let mut sdk2_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-sdk2.zst",
        )]);
        sdk2_release.version = "1.0.0".to_string();
        sdk2_release.sdk_constraint = ">=2.3.0, <3.0.0".to_string();

        let mut sdk3_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-sdk3.zst",
        )]);
        sdk3_release.version = "2.0.0".to_string();
        sdk3_release.sdk_constraint = ">=3.0.0, <4.0.0".to_string();

        let plugin = plugin(vec![sdk2_release, sdk3_release]);

        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("SDK 3 release");

        assert_eq!(selected_release.version, "2.0.0");
        assert_eq!(selected_artifact.url, "https://example.invalid/plugin-sdk3.zst");
    }

    #[test]
    fn empty_feature_set_selects_baseline_artifact() {
        let release = release(vec![
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-simd.br",
            ),
            artifact(&[], "https://example.invalid/plugin.zst"),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("baseline artifact");

        assert_eq!(selected.required_features, Vec::<String>::new());
        assert_eq!(selected.url, "https://example.invalid/plugin.zst");
    }

    #[test]
    fn wasip2_artifact_is_selectable() {
        let mut component = artifact(&[], "https://example.invalid/plugin.wasm.zst");
        component.runtime = "wasm32-wasip2".to_string();
        let release = release(vec![component]);

        let selected = select_catalog_release_artifact(
            &release,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("WASIp2 artifact");

        assert_eq!(selected.runtime, "wasm32-wasip2");
    }

    #[test]
    fn simd128_feature_set_selects_simd128_but_not_relaxed_simd() {
        let release = release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(&["simd128"], "https://example.invalid/plugin-simd.br"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.br",
            ),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &host_capabilities(&["simd128"]),
            RuntimePerformanceClass::Slow,
        )
        .expect("simd128 artifact");

        assert_eq!(selected.required_features, vec!["simd128".to_string()]);
        assert_eq!(selected.url, "https://example.invalid/plugin-simd.br");
    }

    #[test]
    fn full_simd_feature_set_selects_relaxed_simd_artifact() {
        let release = release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(&["simd128"], "https://example.invalid/plugin-simd.zst"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.br",
            ),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.zst",
            ),
        ]);

        let selected = select_catalog_release_artifact(
            &release,
            &host_capabilities(&["simd128", "relaxed-simd"]),
            RuntimePerformanceClass::Slow,
        )
        .expect("relaxed simd artifact");

        assert_eq!(
            selected.required_features,
            vec!["simd128".to_string(), "relaxed-simd".to_string()]
        );
        assert_eq!(selected.url, "https://example.invalid/plugin-relaxed.zst");
    }

    #[test]
    fn portable_native_build_can_select_simd_artifact_from_runtime_features() {
        let plugin = plugin(vec![release(vec![
            artifact(&[], "https://example.invalid/plugin.zst"),
            artifact(
                &["simd128", "relaxed-simd"],
                "https://example.invalid/plugin-relaxed.zst",
            ),
        ])]);

        let (_, selected) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&["simd128", "relaxed-simd"]),
            RuntimePerformanceClass::Slow,
        )
        .expect("runtime feature selection should not depend on native build class");

        assert_eq!(
            selected.required_features,
            vec!["simd128".to_string(), "relaxed-simd".to_string()]
        );
    }

    #[test]
    fn same_version_simd_artifact_counts_as_update_when_installed_artifact_differs() {
        let selected = artifact_with_digests(
            &["simd128"],
            "https://example.invalid/plugin-simd.zst",
            "blake3:3333333333333333333333333333333333333333333333333333333333333333",
            "blake3:4444444444444444444444444444444444444444444444444444444444444444",
        );
        let release = release(vec![selected.clone()]);
        let installation = installed_plugin(
            "1.0.0",
            Some("blake3:1111111111111111111111111111111111111111111111111111111111111111"),
            Some("blake3:2222222222222222222222222222222222222222222222222222222222222222"),
            Some("https://example.invalid/plugin.zst"),
        );
        let resolved = resolution(release.clone(), selected.clone());

        assert!(same_version_simd_artifact_update_available(
            &installation,
            &release,
            &selected
        ));
        assert!(catalog_plugin_update_available(&installation, &resolved));
    }

    #[test]
    fn same_version_selected_simd_artifact_does_not_count_as_update() {
        let selected = artifact_with_digests(
            &["simd128", "relaxed-simd"],
            "https://example.invalid/plugin-relaxed.zst",
            "blake3:3333333333333333333333333333333333333333333333333333333333333333",
            "blake3:4444444444444444444444444444444444444444444444444444444444444444",
        );
        let release = release(vec![selected.clone()]);
        let installation = installed_plugin(
            "1.0.0",
            Some("blake3:4444444444444444444444444444444444444444444444444444444444444444"),
            None,
            Some("https://example.invalid/plugin-relaxed.zst"),
        );

        assert!(!same_version_simd_artifact_update_available(
            &installation,
            &release,
            &selected
        ));
    }

    #[test]
    fn same_version_non_simd_artifact_does_not_count_as_update() {
        let selected = artifact_with_digests(
            &[],
            "https://example.invalid/plugin.zst",
            "blake3:3333333333333333333333333333333333333333333333333333333333333333",
            "blake3:4444444444444444444444444444444444444444444444444444444444444444",
        );
        let release = release(vec![selected.clone()]);
        let installation = installed_plugin(
            "1.0.0",
            Some("blake3:1111111111111111111111111111111111111111111111111111111111111111"),
            Some("blake3:2222222222222222222222222222222222222222222222222222222222222222"),
            Some("https://example.invalid/plugin-old.zst"),
        );

        assert!(!same_version_simd_artifact_update_available(
            &installation,
            &release,
            &selected
        ));
    }

    #[test]
    fn same_wasm_digest_does_not_count_encoding_only_change_as_update() {
        let selected = artifact_with_digests(
            &["simd128"],
            "https://example.invalid/plugin-simd.br",
            "blake3:3333333333333333333333333333333333333333333333333333333333333333",
            "blake3:1111111111111111111111111111111111111111111111111111111111111111",
        );
        let release = release(vec![selected.clone()]);
        let installation = installed_plugin(
            "1.0.0",
            Some("blake3:1111111111111111111111111111111111111111111111111111111111111111"),
            Some("blake3:2222222222222222222222222222222222222222222222222222222222222222"),
            Some("https://example.invalid/plugin-simd.zst"),
        );

        assert!(!same_version_simd_artifact_update_available(
            &installation,
            &release,
            &selected
        ));
    }

    #[test]
    fn catalog_selection_skips_release_requiring_newer_scryer() {
        let compatible_release = release(vec![artifact(&[], "https://example.invalid/plugin.zst")]);
        let mut newer_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-v2.zst",
        )]);
        newer_release.version = "2.0.0".to_string();
        newer_release.min_scryer_version = Some("999.0.0".to_string());

        let plugin = plugin(vec![compatible_release, newer_release]);

        let (selected_release, _) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("compatible release");

        assert_eq!(selected_release.version, "1.0.0");
    }

    #[test]
    fn catalog_selection_skips_release_past_its_max_scryer_version() {
        let compatible_release = release(vec![artifact(&[], "https://example.invalid/plugin.zst")]);
        let mut retired_release = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-v2.zst",
        )]);
        retired_release.version = "2.0.0".to_string();
        retired_release.min_scryer_version = Some("0.17.0".to_string());
        retired_release.max_scryer_version = Some("0.18.11".to_string());
        assert_eq!(
            catalog_release_scryer_constraint(&retired_release).as_deref(),
            Some(">=0.17.0, <=0.18.11")
        );

        let plugin = plugin(vec![compatible_release, retired_release]);

        let (selected_release, _) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("compatible release");

        assert_eq!(selected_release.version, "1.0.0");
    }

    #[test]
    fn catalog_selection_checks_all_releases_before_picking_highest_compatible() {
        let mut newest_incompatible_first = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-v3.zst",
        )]);
        newest_incompatible_first.version = "3.0.0".to_string();
        newest_incompatible_first.min_scryer_version = Some("999.0.0".to_string());

        let older_compatible =
            release(vec![artifact(&[], "https://example.invalid/plugin-v1.zst")]);

        let mut incompatible_after_compatible = release(vec![artifact(
            &[],
            "https://example.invalid/plugin-v4.zst",
        )]);
        incompatible_after_compatible.version = "4.0.0".to_string();
        incompatible_after_compatible.min_scryer_version = Some("999.0.0".to_string());

        let mut highest_compatible =
            release(vec![artifact(&[], "https://example.invalid/plugin-v2.zst")]);
        highest_compatible.version = "2.0.0".to_string();

        let plugin = plugin(vec![
            newest_incompatible_first,
            older_compatible,
            incompatible_after_compatible,
            highest_compatible,
        ]);

        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("compatible release");

        assert_eq!(selected_release.version, "2.0.0");
        assert_eq!(
            selected_artifact.url,
            "https://example.invalid/plugin-v2.zst"
        );
    }

    fn artifact_for(runtime: &str, url: &str) -> CatalogV3PluginArtifact {
        let mut artifact = artifact(&[], url);
        artifact.runtime = runtime.to_string();
        artifact
    }

    #[test]
    fn a_preview1_only_host_keeps_the_newest_release_it_can_run() {
        let mut legacy = release(vec![artifact_for(
            CATALOG_V3_RUNTIME_WASIP1,
            "https://example.invalid/plugin-p1.zst",
        )]);
        legacy.version = "2.0.3".to_string();
        let mut component = release(vec![artifact_for(
            CATALOG_V3_RUNTIME_WASIP2,
            "https://example.invalid/plugin-p2.zst",
        )]);
        component.version = "2.0.4".to_string();
        let plugin = plugin(vec![legacy, component]);

        let preview1_host = HashSet::from([CATALOG_V3_RUNTIME_WASIP1.to_string()]);
        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &preview1_host,
            RuntimePerformanceClass::Slow,
        )
        .expect("a Preview 1 host must fall back rather than come up empty");

        assert_eq!(selected_release.version, "2.0.3");
        assert_eq!(selected_artifact.runtime, CATALOG_V3_RUNTIME_WASIP1);
    }

    #[test]
    fn a_component_host_takes_the_newer_component_release() {
        let mut legacy = release(vec![artifact_for(
            CATALOG_V3_RUNTIME_WASIP1,
            "https://example.invalid/plugin-p1.zst",
        )]);
        legacy.version = "2.0.3".to_string();
        let mut component = release(vec![artifact_for(
            CATALOG_V3_RUNTIME_WASIP2,
            "https://example.invalid/plugin-p2.zst",
        )]);
        component.version = "2.0.4".to_string();
        let plugin = plugin(vec![legacy, component]);

        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("a component host must take the component release");

        assert_eq!(selected_release.version, "2.0.4");
        assert_eq!(selected_artifact.runtime, CATALOG_V3_RUNTIME_WASIP2);
    }

    #[test]
    fn a_future_target_only_release_is_skipped_not_fatal() {
        let mut current = release(vec![artifact_for(
            CATALOG_V3_RUNTIME_WASIP2,
            "https://example.invalid/plugin-p2.zst",
        )]);
        current.version = "3.0.0".to_string();
        let mut future = release(vec![artifact_for(
            "wasm32-wasip3",
            "https://example.invalid/plugin-p3.zst",
        )]);
        future.version = "4.0.0".to_string();
        let plugin = plugin(vec![current, future]);

        let (selected_release, selected_artifact) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("a wasip3-only release must not strand a wasip2 host");

        assert_eq!(selected_release.version, "3.0.0");
        assert_eq!(selected_artifact.runtime, CATALOG_V3_RUNTIME_WASIP2);
    }

    #[test]
    fn a_release_shipping_both_targets_serves_each_host_its_own_artifact() {
        let both = release(vec![
            artifact_for(
                CATALOG_V3_RUNTIME_WASIP2,
                "https://example.invalid/plugin-p2.zst",
            ),
            artifact_for("wasm32-wasip3", "https://example.invalid/plugin-p3.zst"),
        ]);
        let plugin = plugin(vec![both]);

        let (_, on_wasip2) = select_catalog_release_and_artifact(
            &plugin,
            &host_capabilities(&[]),
            RuntimePerformanceClass::Slow,
        )
        .expect("wasip2 host");
        assert_eq!(on_wasip2.runtime, CATALOG_V3_RUNTIME_WASIP2);

        let wasip3_host = HashSet::from([
            CATALOG_V3_RUNTIME_WASIP2.to_string(),
            "wasm32-wasip3".to_string(),
        ]);
        let (_, on_wasip3) = select_catalog_release_and_artifact(
            &plugin,
            &wasip3_host,
            RuntimePerformanceClass::Slow,
        )
        .expect("wasip3 host");
        assert_eq!(
            on_wasip3.runtime, "wasm32-wasip3",
            "a host that declares the newer target must prefer it within the same release"
        );
    }

    #[test]
    fn an_unrunnable_newest_release_does_not_hide_the_plugin() {
        let mut only_future = release(vec![artifact_for(
            "wasm32-wasip3",
            "https://example.invalid/plugin-p3.zst",
        )]);
        only_future.version = "4.0.0".to_string();
        let plugin = plugin(vec![only_future]);

        assert!(
            select_catalog_release_and_artifact(
                &plugin,
                &host_capabilities(&[]),
                RuntimePerformanceClass::Slow,
            )
            .is_none(),
            "with nothing runnable the plugin resolves to nothing — it must not panic or \
             poison the rest of the catalog"
        );
    }
}

#[cfg(test)]
mod signature_bundle_decode_tests {
    use super::*;

    #[tokio::test]
    async fn plain_signature_bundle_is_left_unchanged() {
        let bundle = br#"{"base64Signature":"signature"}"#.to_vec();

        let decoded = decode_signature_bundle(
            bundle.clone(),
            "https://example.test/plugin.tar.zst.bundle",
        )
        .await
        .expect("plain bundle should decode");

        assert_eq!(decoded, bundle);
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[tokio::test]
    async fn zstd_signature_bundle_is_decoded_from_url() {
        let bundle = br#"{"base64Signature":"signature"}"#.to_vec();
        let compressed = compress_zstd(bundle.clone(), 3)
            .await
            .expect("bundle should compress");

        let decoded = decode_signature_bundle(
            compressed,
            "https://example.test/catalog.json.bundle.zst",
        )
        .await
        .expect("zstd bundle should decode");

        assert_eq!(decoded, bundle);
    }
}

#[cfg(all(test, feature = "runtime-plugin-trust"))]
mod bounded_decompression_tests {
    use super::*;
    use crate::AppError;
    use std::io::Write;

    fn compress_brotli_for_test(bytes: &[u8]) -> Vec<u8> {
        let mut compressed = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 5, 22);
            writer
                .write_all(bytes)
                .expect("test brotli compression should write");
        }
        compressed
    }

    fn assert_limit_error(error: AppError) {
        assert!(
            error.to_string().contains("exceeds maximum size"),
            "unexpected error: {error}"
        );
    }

    fn rule_pack_release(rule_pack_bytes: Option<u64>) -> CatalogV3RulePackRelease {
        CatalogV3RulePackRelease {
            version: "1.0.0".to_string(),
            min_scryer_version: None,
            rule_pack_digests: Vec::new(),
            rule_pack_bytes,
            artifacts: Vec::new(),
        }
    }

    #[tokio::test]
    async fn zstd_exact_size_succeeds_and_one_byte_over_fails() {
        let payload = b"bounded zstd payload".to_vec();
        let compressed = compress_zstd(payload.clone(), 3)
            .await
            .expect("zstd payload should compress");

        let decoded = decompress_zstd(
            compressed.clone(),
            payload.len() as u64,
            "zstd exact-size test",
        )
        .await
        .expect("exact-size zstd payload should decode");
        assert_eq!(decoded, payload);

        let error = decompress_zstd(
            compressed,
            (payload.len() - 1) as u64,
            "zstd one-byte-over test",
        )
        .await
        .expect_err("one-byte-over zstd payload should fail");
        assert_limit_error(error);
    }

    #[tokio::test]
    async fn brotli_exact_size_succeeds_and_one_byte_over_fails() {
        let payload = b"bounded brotli payload".to_vec();
        let compressed = compress_brotli_for_test(&payload);

        let decoded = decompress_brotli(
            compressed.clone(),
            payload.len() as u64,
            "brotli exact-size test",
        )
        .await
        .expect("exact-size brotli payload should decode");
        assert_eq!(decoded, payload);

        let error = decompress_brotli(
            compressed,
            (payload.len() - 1) as u64,
            "brotli one-byte-over test",
        )
        .await
        .expect_err("one-byte-over brotli payload should fail");
        assert_limit_error(error);
    }

    #[tokio::test]
    async fn zstd_and_brotli_bombs_stop_at_cap() {
        let payload = vec![b'x'; 1024 * 1024];
        let zstd = compress_zstd(payload.clone(), 3)
            .await
            .expect("zstd bomb payload should compress");
        let brotli = compress_brotli_for_test(&payload);

        let zstd_error = decompress_zstd(zstd, 1024, "zstd bomb test")
            .await
            .expect_err("zstd bomb should fail at cap");
        assert_limit_error(zstd_error);

        let brotli_error = decompress_brotli(brotli, 1024, "brotli bomb test")
            .await
            .expect_err("brotli bomb should fail at cap");
        assert_limit_error(brotli_error);
    }

    #[tokio::test]
    async fn catalog_artifact_uses_expected_bytes_as_cap() {
        let payload = b"catalog wasm bytes".to_vec();
        let compressed = compress_zstd(payload.clone(), 3)
            .await
            .expect("catalog artifact should compress");

        let decoded = decode_catalog_wasm_artifact(
            compressed.clone(),
            PluginWasmEncoding::Zstd,
            payload.len() as u64,
            "cap-test",
        )
        .await
        .expect("catalog artifact at expected bytes should decode");
        assert_eq!(decoded, payload);

        let error = decode_catalog_wasm_artifact(
            compressed,
            PluginWasmEncoding::Zstd,
            (payload.len() - 1) as u64,
            "cap-test",
        )
        .await
        .expect_err("catalog artifact over expected bytes should fail");
        assert_limit_error(error);
    }

    #[tokio::test]
    async fn catalog_artifact_rejects_declared_size_over_universal_wasm_cap() {
        let error = decode_catalog_wasm_artifact(
            Vec::new(),
            PluginWasmEncoding::Zstd,
            MANUAL_PLUGIN_WASM_OUTPUT_LIMIT + 1,
            "cap-test",
        )
        .await
        .expect_err("catalog artifact over universal WASM cap should fail before decompression");

        assert_limit_error(error);
    }

    #[tokio::test]
    async fn manual_upload_decode_uses_fixed_wasm_cap_before_descriptor_parsing() {
        let payload = b"manual upload wasm bytes".to_vec();
        let compressed = compress_zstd(payload.clone(), 3)
            .await
            .expect("manual upload should compress");

        let decoded = decode_uploaded_plugin_wasm_with_limit(
            compressed.clone(),
            true,
            payload.len() as u64,
        )
        .await
        .expect("manual upload at cap should decode");
        assert_eq!(decoded, payload);

        let compressed_error = decode_uploaded_plugin_wasm_with_limit(
            compressed,
            true,
            (payload.len() - 1) as u64,
        )
        .await
        .expect_err("compressed manual upload over cap should fail");
        assert_limit_error(compressed_error);

        let raw_error = decode_uploaded_plugin_wasm_with_limit(
            vec![0; payload.len()],
            false,
            (payload.len() - 1) as u64,
        )
        .await
        .expect_err("raw manual upload over cap should fail");
        assert_limit_error(raw_error);
    }

    #[tokio::test]
    async fn rule_pack_manifest_uses_expected_bytes_when_present() {
        let payload = br#"{"schema_version":1,"id":"pack","rules":[]}"#.to_vec();
        let compressed = compress_zstd(payload.clone(), 3)
            .await
            .expect("rule pack manifest should compress");
        let release = rule_pack_release(Some(payload.len() as u64));

        let decoded = decode_rule_pack_manifest_bytes(
            compressed.clone(),
            "https://example.test/rules.min.json.zst",
            "pack",
            &release,
        )
        .await
        .expect("rule pack manifest at expected bytes should decode");
        assert_eq!(decoded, payload);

        let release = rule_pack_release(Some((payload.len() - 1) as u64));
        let error = decode_rule_pack_manifest_bytes(
            compressed,
            "https://example.test/rules.min.json.zst",
            "pack",
            &release,
        )
        .await
        .expect_err("rule pack manifest over expected bytes should fail");
        assert_limit_error(error);
    }

    #[tokio::test]
    async fn rule_pack_manifest_falls_back_to_hard_cap_when_expected_bytes_absent() {
        let payload = vec![0; (RULE_PACK_MANIFEST_FALLBACK_OUTPUT_LIMIT as usize) + 1];
        let compressed = compress_zstd(payload, 3)
            .await
            .expect("fallback cap payload should compress");
        let release = rule_pack_release(None);

        let error = decode_rule_pack_manifest_bytes(
            compressed,
            "https://example.test/rules.min.json.zst",
            "pack",
            &release,
        )
        .await
        .expect_err("rule pack manifest over fallback cap should fail");
        assert_limit_error(error);
    }

    #[tokio::test]
    async fn catalog_json_signature_bundle_and_redirect_caps_are_enforced() {
        let catalog_payload = vec![0; (PLUGIN_CATALOG_JSON_OUTPUT_LIMIT as usize) + 1];
        let catalog_compressed = compress_zstd(catalog_payload, 3)
            .await
            .expect("catalog payload should compress");
        let catalog_error = decode_catalog_json(
            catalog_compressed,
            "https://example.test/catalog-v3.min.json.zst",
            "plugin catalog",
        )
        .await
        .expect_err("catalog JSON over cap should fail");
        assert_limit_error(catalog_error);

        let bundle_payload = vec![0; (PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT as usize) + 1];
        let bundle_compressed = compress_zstd(bundle_payload, 3)
            .await
            .expect("signature bundle payload should compress");
        let bundle_error = decode_signature_bundle(
            bundle_compressed,
            "https://example.test/catalog-v3.min.json.zst.bundle.zst",
        )
        .await
        .expect_err("signature bundle over cap should fail");
        assert_limit_error(bundle_error);

        let redirect_error = bound_uncompressed_bytes(
            vec![0; (PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT as usize) + 1],
            PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT,
            "plugin catalog redirect",
        )
        .expect_err("redirect JSON over cap should fail");
        assert_limit_error(redirect_error);
    }
}

#[cfg(test)]
mod plugin_http_client_tests {
    use super::{
        PLUGIN_HTTP_MAX_VALIDATED_REDIRECTS, PluginHttpClientProfile, PluginRedirectPolicy,
        combined_plugin_catalog_probe_error, fetch_plugin_bytes_with_redirect_policy,
        plugin_http_client, plugin_redirect_location_url,
    };
    use crate::AppError;

    #[test]
    fn plugin_http_client_profiles_are_cached() {
        let default_a = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should build") as *const _;
        let default_b = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should stay cached")
            as *const _;
        let rule_pack_a = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should build")
            as *const _;
        let rule_pack_b = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should stay cached")
            as *const _;

        assert_eq!(default_a, default_b);
        assert_eq!(rule_pack_a, rule_pack_b);
        assert_ne!(default_a, rule_pack_a);
    }

    #[tokio::test]
    async fn plugin_artifact_fetch_rejects_private_destinations() {
        let error = fetch_plugin_bytes_with_redirect_policy(
            "http://127.0.0.1/plugin.wasm",
            "test plugin artifact",
            "test-plugin-artifact",
            PluginRedirectPolicy::Reject,
        )
        .await
        .expect_err("private plugin artifact URL should be rejected");

        assert!(
            matches!(error, AppError::Validation(_)),
            "expected validation error, got {error:?}"
        );
        assert!(error.to_string().contains("private or local address"));
    }

    #[test]
    fn plugin_catalog_redirects_are_capped_at_three_hops() {
        assert_eq!(PLUGIN_HTTP_MAX_VALIDATED_REDIRECTS, 3);
    }

    #[test]
    fn plugin_catalog_redirect_location_accepts_relative_github_location() {
        let current_url = reqwest::Url::parse(
            "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv3/catalog-v3.redirect.json",
        )
        .expect("valid URL");
        let location = reqwest::header::HeaderValue::from_static(
            "/scryer-media/scryer-plugins/releases/download/catalog%2Fv3/catalog-v3.redirect.bundle.json",
        );

        let redirect_url =
            plugin_redirect_location_url(&current_url, &location, "plugin catalog redirect")
                .expect("relative redirect should resolve");

        assert_eq!(
            redirect_url.as_str(),
            "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv3/catalog-v3.redirect.bundle.json"
        );
    }

    #[test]
    fn plugin_catalog_probe_error_preserves_primary_and_github_failures() {
        let error = combined_plugin_catalog_probe_error(
            Some("failed to download primary: dns failed"),
            Some("failed to download fallback: redirects are not allowed"),
        )
        .expect("combined error");

        assert!(error.contains("primary plugin catalog redirect: failed to download primary"));
        assert!(error.contains("GitHub plugin catalog redirect: failed to download fallback"));
    }

    #[test]
    fn plugin_catalog_probe_error_is_empty_without_probe_failures() {
        assert_eq!(combined_plugin_catalog_probe_error(None, None), None);
    }
}
#[cfg(all(test, feature = "runtime-plugin-trust"))]
#[path = "../app_usecase_plugins_tests.rs"]
mod app_usecase_plugins_tests;
