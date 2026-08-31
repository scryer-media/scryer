#[derive(Clone, Debug)]
pub struct ManualPluginPreview {
    pub plugin: RegistryPlugin,
    pub github_repo_url: String,
}
fn single_manual_catalog_plugin(
    catalog: &CatalogV3,
    repo: &GitHubRepo,
) -> AppResult<CatalogV3PluginEntry> {
    if catalog.plugins.len() != 1 {
        return Err(AppError::Validation(format!(
            "manual plugin repo '{}' must publish exactly one plugin entry",
            repo.slug()
        )));
    }
    let plugin = catalog.plugins[0].clone();
    let source_repo = GitHubRepo::parse(&plugin.source_repo)?;
    if source_repo != *repo {
        return Err(AppError::Validation(format!(
            "manual plugin repo '{}' published plugin '{}' from source repo '{}'",
            repo.slug(),
            plugin.id,
            source_repo.slug()
        )));
    }
    Ok(plugin)
}
fn manual_catalog_source_key(repo: &GitHubRepo) -> String {
    format!("manual:{}", repo.slug())
}
fn source_kind_label(source_kind: PluginSourceKind) -> String {
    match source_kind {
        PluginSourceKind::Bundled => "bundled".to_string(),
        PluginSourceKind::Downloaded => "downloaded".to_string(),
        PluginSourceKind::Community => "community".to_string(),
        PluginSourceKind::Manual => "manual".to_string(),
    }
}
fn uploaded_plugin_file_is_zstd(file_name: &str) -> AppResult<bool> {
    let normalized = file_name.trim().to_ascii_lowercase();
    if normalized.ends_with(".wasm.zst") {
        return Ok(true);
    }
    if normalized.ends_with(".wasm") {
        return Ok(false);
    }
    Err(AppError::Validation(
        "manual plugin upload must be a .wasm or .wasm.zst file".to_string(),
    ))
}

async fn decode_uploaded_plugin_wasm_with_limit(
    uploaded_bytes: Vec<u8>,
    uploaded_is_zstd: bool,
    max_output_bytes: u64,
) -> AppResult<Vec<u8>> {
    if uploaded_is_zstd {
        decompress_zstd(
            uploaded_bytes,
            max_output_bytes,
            "manual plugin WASM upload",
        )
        .await
    } else {
        bound_uncompressed_bytes(
            uploaded_bytes,
            max_output_bytes,
            "manual plugin WASM upload",
        )
    }
}

async fn decode_uploaded_plugin_wasm(
    uploaded_bytes: Vec<u8>,
    uploaded_is_zstd: bool,
) -> AppResult<Vec<u8>> {
    decode_uploaded_plugin_wasm_with_limit(
        uploaded_bytes,
        uploaded_is_zstd,
        MANUAL_PLUGIN_WASM_OUTPUT_LIMIT,
    )
    .await
}

impl AppUseCase {
    async fn upsert_manual_plugin_catalog_source(
        &self,
        repo: &GitHubRepo,
        source_url: &str,
        child_json: Option<String>,
        last_error: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now();
        let last_success_at = child_json.as_ref().map(|_| now);
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: manual_catalog_source_key(repo),
                source_kind: "manual".to_string(),
                source_url: source_url.to_string(),
                github_repo: Some(repo.slug()),
                support_tier: PluginSupportTier::Unverified,
                catalog_json: child_json,
                last_success_at,
                last_error,
                updated_at: now,
            })
            .await
    }
}
impl AppUseCase {
    async fn resolve_manual_plugin_repo(
        &self,
        github_repo_url: &str,
    ) -> AppResult<(CatalogPluginResolution, String)> {
        let repo = GitHubRepo::parse(github_repo_url)?;
        let catalog_url = repo.catalog_v3_url();
        self.resolve_manual_plugin_repo_at_url(repo, &catalog_url)
            .await
    }
}
impl AppUseCase {
    async fn resolve_manual_plugin_repo_at_url(
        &self,
        repo: GitHubRepo,
        catalog_url: &str,
    ) -> AppResult<(CatalogPluginResolution, String)> {
        let signer = RequiredSigner {
            github_repository: repo.slug(),
            github_workflow: None,
            github_ref: None,
        };
        let data_urls = vec![catalog_url.to_string()];
        let signature_urls = vec![signed_catalog_json_bundle_url(catalog_url)];
        let (catalog_raw, actual_url) = self
            .fetch_verified_blob_from_locations(
                &data_urls,
                &signature_urls,
                &signer,
                "manual plugin catalog",
            )
            .await?;
        let catalog_raw = decode_catalog_json(catalog_raw, &actual_url, "manual plugin catalog")
            .await?;
        let catalog = parse_and_validate_catalog_v3(&catalog_raw)?;
        let plugin = single_manual_catalog_plugin(&catalog, &repo)?;
        let cpu_class = self.runtime_performance().await.cpu_class;
        let supported_plugin_features = self.runtime_supported_plugin_required_features();
        let (release, artifact) =
            select_catalog_release_and_artifact(&plugin, &supported_plugin_features, cpu_class)
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "manual plugin repo '{}' has no SDK-compatible release",
                        repo.slug()
                    ))
                })?;
        let catalog_json = String::from_utf8(catalog_raw).map_err(|e| {
            AppError::Validation(format!("manual plugin catalog is not UTF-8: {e}"))
        })?;
        Ok((
            CatalogPluginResolution {
                catalog_entry: plugin,
                release,
                artifact,
                source_kind: PluginSourceKind::Manual,
                effective_support_tier: PluginSupportTier::Unverified,
                github_repo: repo,
            },
            catalog_json,
        ))
    }
}
impl AppUseCase {
    pub async fn inspect_manual_plugin_repo(
        &self,
        actor: &User,
        github_repo_url: &str,
    ) -> AppResult<ManualPluginPreview> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (resolved, _) = self.resolve_manual_plugin_repo(github_repo_url).await?;
        let plugin_type = resolved.catalog_entry.plugin_type.clone();
        Ok(ManualPluginPreview {
            github_repo_url: format!("https://github.com/{}", resolved.github_repo.slug()),
            plugin: RegistryPlugin {
                id: resolved.catalog_entry.id.clone(),
                name: resolved.catalog_entry.name.clone(),
                description: resolved.catalog_entry.description.clone(),
                version: resolved.release.version.clone(),
                latest_version: None,
                plugin_type: plugin_type.clone(),
                provider_type: resolved.catalog_entry.provider_type.clone(),
                author: resolved.catalog_entry.publisher.clone(),
                official: false,
                publisher: Some(resolved.catalog_entry.publisher.clone()),
                support_tier: PluginSupportTier::Unverified,
                status: Some(lifecycle_status_label(resolved.catalog_entry.status)),
                docs_url: Some(resolved.catalog_entry.docs_url.clone()),
                source_repo: Some(resolved.catalog_entry.source_repo.clone()),
                builtin: false,
                source_url: Some(resolved.artifact.url.clone()),
                source_kind: Some(source_kind_label(PluginSourceKind::Manual)),
                blocked_reason: None,
                wasm_url: Some(resolved.artifact.url.clone()),
                wasm_sha256: None,
                min_scryer_version: None,
                bytes: Some(resolved.artifact.bytes),
                is_installed: self
                    .services
                    .customization
                    .plugin_installations
                    .get_plugin_installation(&resolved.catalog_entry.id)
                    .await?
                    .is_some(),
                is_enabled: false,
                installed_version: None,
                update_available: false,
                install_in_progress: false,
                default_base_url: self.default_base_url_for_plugin(
                    &plugin_type,
                    &resolved.catalog_entry.provider_type,
                ),
            },
        })
    }
}
impl AppUseCase {
    pub async fn install_uploaded_plugin(
        &self,
        actor: &User,
        file_name: &str,
        wasm_base64: &str,
        acknowledge_risk: bool,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        if !acknowledge_risk {
            return Err(AppError::Validation(
                "manual plugin upload requires explicit risk acknowledgement".to_string(),
            ));
        }

        let file_name = file_name.trim();
        if file_name.is_empty() {
            return Err(AppError::Validation(
                "manual plugin upload file name is required".to_string(),
            ));
        }
        let uploaded_is_zstd = uploaded_plugin_file_is_zstd(file_name)?;
        let uploaded_bytes = base64::engine::general_purpose::STANDARD
            .decode(wasm_base64.trim())
            .map_err(|error| {
                AppError::Validation(format!(
                    "manual plugin upload payload is not valid base64: {error}"
                ))
            })?;
        let wasm_bytes = decode_uploaded_plugin_wasm(uploaded_bytes, uploaded_is_zstd).await?;
        let descriptor_loader = self.services.customization.plugin_descriptor_loader.clone();
        let (wasm_bytes, descriptor) = tokio::task::spawn_blocking(move || {
            let descriptor = descriptor_loader.load_descriptor_from_wasm_bytes(&wasm_bytes)?;
            Ok::<_, AppError>((wasm_bytes, descriptor))
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("plugin descriptor loading task failed: {error}"))
        })??;
        validate_plugin_descriptor_sdk_contract(&descriptor, SDK_VERSION)
            .map_err(AppError::Validation)?;
        validate_plugin_descriptor_host_permissions(&descriptor).map_err(AppError::Validation)?;
        // Manually uploaded plugins are always Unverified, so the host-process
        // capability is never permitted on this path.
        ensure_host_process_capability_allowed(&descriptor, PluginSupportTier::Unverified)?;

        let plugin_id = descriptor.id.clone();
        if is_reserved_first_party_provider(descriptor.provider_type()) {
            return Err(AppError::Validation(format!(
                "provider type '{}' is reserved for first-party code",
                descriptor.provider_type()
            )));
        }

        let _operation_guard = self
            .runtime
            .plugins
            .plugin_operation_guards
            .acquire(&plugin_id)
            .await;
        let existing = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(&plugin_id)
            .await?;
        if existing
            .as_ref()
            .is_some_and(|installation| installation.is_builtin)
        {
            return Err(AppError::Validation(format!(
                "plugin '{}' is a bundled plugin; uninstall any downloaded override before uploading a local build",
                plugin_id
            )));
        }

        let compressed_wasm_bytes =
            compress_zstd(wasm_bytes.clone(), SQLITE_PLUGIN_WASM_ZSTD_LEVEL).await?;
        let (wasm_digest_algo, wasm_digest) = parse_digest_string(&blake3_digest(&wasm_bytes))?;
        let descriptor_json = Some(persisted_plugin_descriptor_json(&descriptor)?);
        let plugin_type = descriptor.plugin_type().to_string();
        let provider_type = normalize_provider_key(descriptor.provider_type());
        let sdk_constraint = plugin_descriptor_sdk_constraint(&descriptor);
        let now = Utc::now();
        let runtime_plugin =
            runtime_plugin_load_from_validated(descriptor.clone(), wasm_bytes, false);

        let result = match existing {
            Some(mut installation) => {
                let previous_plugin_type = installation.plugin_type.clone();
                let previous_provider_type = installation.provider_type.clone();
                let runtime_touched = installation.is_enabled;
                installation.name = descriptor.name.clone();
                installation.description =
                    format!("Manually uploaded plugin from local file '{file_name}'");
                installation.version = descriptor.version.clone();
                installation.sdk_version = descriptor.sdk_version.clone();
                installation.sdk_constraint = sdk_constraint;
                installation.scryer_constraint = None;
                installation.plugin_type = plugin_type.clone();
                installation.provider_type = provider_type.clone();
                installation.source_kind = PluginSourceKind::Manual;
                installation.wasm_encoding = PluginWasmEncoding::Zstd;
                installation.wasm_digest_algo = Some(wasm_digest_algo.clone());
                installation.source_url = None;
                installation.support_tier = PluginSupportTier::Unverified;
                installation.publisher = None;
                installation.docs_url = None;
                installation.source_repo = None;
                installation.manifest_url = None;
                installation.wasm_digest = Some(wasm_digest.clone());
                installation.artifact_digest = None;
                installation.descriptor_json = descriptor_json;
                installation.updated_at = now;

                let updated = self
                    .services
                    .customization
                    .plugin_installations
                    .update_plugin_installation(
                        &installation,
                        Some(compressed_wasm_bytes.as_slice()),
                    )
                    .await?;

                if runtime_touched {
                    let mut previous_runtime_installation = updated.clone();
                    previous_runtime_installation.plugin_type = previous_plugin_type.clone();
                    previous_runtime_installation.provider_type = previous_provider_type.clone();
                    self.apply_runtime_plugin_replace(
                        &previous_runtime_installation,
                        &updated,
                        runtime_plugin,
                    )?;
                }
                self.finalize_runtime_plugin_mutation_for_types(
                    [previous_plugin_type.as_str(), updated.plugin_type.as_str()],
                    runtime_touched,
                )
                .await?;
                updated
            }
            None => {
                let installation = PluginInstallation {
                    id: Id::new().0,
                    plugin_id,
                    name: descriptor.name.clone(),
                    description: format!("Manually uploaded plugin from local file '{file_name}'"),
                    version: descriptor.version.clone(),
                    sdk_version: descriptor.sdk_version.clone(),
                    sdk_constraint,
                    scryer_constraint: None,
                    plugin_type,
                    provider_type,
                    source_kind: PluginSourceKind::Manual,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: PluginWasmEncoding::Zstd,
                    wasm_digest_algo: Some(wasm_digest_algo),
                    source_url: None,
                    support_tier: PluginSupportTier::Unverified,
                    publisher: None,
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: Some(wasm_digest),
                    artifact_digest: None,
                    descriptor_json,
                    installed_at: now,
                    updated_at: now,
                };

                let created = self
                    .services
                    .customization
                    .plugin_installations
                    .create_plugin_installation(
                        &installation,
                        Some(compressed_wasm_bytes.as_slice()),
                    )
                    .await?;
                self.apply_runtime_plugin_upsert(&created, runtime_plugin)?;
                self.finalize_runtime_plugin_mutation(&created.plugin_type, true)
                    .await?;
                created
            }
        };

        Ok(result)
    }
}
impl AppUseCase {
    async fn ensure_manual_plugin_catalog_source_for_restore(
        &self,
        source_repo: &str,
    ) -> AppResult<()> {
        let repo = GitHubRepo::parse(source_repo)?;
        let source_key = manual_catalog_source_key(&repo);
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_source(&source_key)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let now = Utc::now();
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key,
                source_kind: "manual".to_string(),
                source_url: repo.catalog_v3_url(),
                github_repo: Some(repo.slug()),
                support_tier: PluginSupportTier::Unverified,
                catalog_json: None,
                last_success_at: None,
                last_error: None,
                updated_at: now,
            })
            .await
    }
}
