#[derive(Clone, Debug)]
struct DownloadedPluginReleaseContract {
    version: String,
    sdk_version: Option<String>,
    sdk_constraint: String,
    scryer_constraint: Option<String>,
}
fn downloaded_plugin_release_scryer_constraint(
    release: &DownloadedPluginReleaseContract,
) -> Option<&str> {
    release.scryer_constraint.as_deref()
}
fn normalized_release_sdk_constraint(release: &DownloadedPluginReleaseContract) -> String {
    release.sdk_version.as_deref().map_or_else(
        || release.sdk_constraint.trim().to_string(),
        |sdk_version| sdk_constraint_or_legacy(sdk_version, &release.sdk_constraint),
    )
}
fn downloaded_plugin_release_is_host_compatible(
    plugin_id: &str,
    release: &DownloadedPluginReleaseContract,
) -> bool {
    let constraint =
        effective_host_sdk_constraint(release.sdk_version.as_deref(), &release.sdk_constraint);
    let sdk_req = semver::VersionReq::parse(constraint.trim()).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                sdk_constraint = constraint.as_str(),
                error = %error,
                "skipping plugin release with invalid sdk_constraint"
            );
            None
        },
        Some,
    );
    let Some(sdk_req) = sdk_req else {
        return false;
    };
    if !sdk_req.matches(current_sdk_version()) {
        return false;
    }
    let Some(constraint) = downloaded_plugin_release_scryer_constraint(release) else {
        return true;
    };
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return true;
    }
    match host_version_matches_constraint(CURRENT_SCRYER_VERSION, constraint) {
        Ok(matches) => matches,
        Err(error) => {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                scryer_constraint = constraint,
                error = %error,
                "skipping plugin release with invalid scryer_constraint"
            );
            false
        }
    }
}
fn preferred_plugin_artifact_encoding(
    cpu_class: crate::services::RuntimePerformanceClass,
) -> &'static str {
    match cpu_class {
        crate::services::RuntimePerformanceClass::Fast => "br",
        crate::services::RuntimePerformanceClass::Slow => "zst",
    }
}
fn blake3_digest_components(digests: &[String], label: &str) -> AppResult<(String, String)> {
    for digest in digests {
        let (algorithm, value) = parse_digest_string(digest)?;
        if algorithm == "blake3" {
            return Ok((algorithm, value));
        }
    }
    Err(AppError::Validation(format!(
        "{label} does not include a blake3 digest"
    )))
}
fn blake3_digest_string(digests: &[String], label: &str) -> AppResult<String> {
    let (algorithm, value) = blake3_digest_components(digests, label)?;
    Ok(format!("{algorithm}:{value}"))
}
fn runtime_plugin_load_from_validated(
    descriptor: PluginDescriptor,
    wasm_bytes: Vec<u8>,
    first_party: bool,
) -> RuntimePluginLoad {
    RuntimePluginLoad {
        descriptor,
        wasm_bytes,
        first_party,
    }
}
fn persisted_plugin_descriptor_json(descriptor: &PluginDescriptor) -> AppResult<String> {
    serde_json::to_string(descriptor).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize plugin descriptor '{}': {error}",
            descriptor.id
        ))
    })
}
fn installation_runtime_release(
    installation: &PluginInstallation,
) -> DownloadedPluginReleaseContract {
    DownloadedPluginReleaseContract {
        version: installation.version.clone(),
        sdk_version: Some(installation.sdk_version.clone()),
        sdk_constraint: installation.sdk_constraint.clone(),
        scryer_constraint: installation.scryer_constraint.clone(),
    }
}
pub async fn decode_persisted_plugin_wasm_payload(
    installation: &PluginInstallation,
    payload: &PersistedPluginWasmPayload,
) -> AppResult<Vec<u8>> {
    let wasm_bytes = match payload.encoding {
        PluginWasmEncoding::Identity => bound_uncompressed_bytes(
            payload.bytes.clone(),
            MANUAL_PLUGIN_WASM_OUTPUT_LIMIT,
            "persisted plugin WASM",
        )?,
        PluginWasmEncoding::Brotli => {
            decompress_brotli(
                payload.bytes.clone(),
                MANUAL_PLUGIN_WASM_OUTPUT_LIMIT,
                "persisted plugin WASM",
            )
            .await?
        }
        PluginWasmEncoding::Zstd => {
            decompress_zstd(
                payload.bytes.clone(),
                MANUAL_PLUGIN_WASM_OUTPUT_LIMIT,
                "persisted plugin WASM",
            )
            .await?
        }
    };
    let algorithm = installation.wasm_digest_algo.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "plugin '{}' is missing persisted wasm digest algorithm",
            installation.plugin_id
        ))
    })?;
    let expected_digest = installation.wasm_digest.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "plugin '{}' is missing persisted wasm digest value",
            installation.plugin_id
        ))
    })?;
    verify_split_digest(
        "persisted plugin WASM",
        algorithm,
        expected_digest,
        &wasm_bytes,
    )?;
    Ok(wasm_bytes)
}
fn parse_persisted_plugin_descriptor(
    installation: &PluginInstallation,
) -> AppResult<PluginDescriptor> {
    let descriptor_json = installation
        .descriptor_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Validation(format!(
                "plugin '{}' is missing persisted descriptor_json",
                installation.plugin_id
            ))
        })?;
    serde_json::from_str(descriptor_json).map_err(|error| {
        AppError::Validation(format!(
            "plugin '{}' has invalid persisted descriptor_json: {error}",
            installation.plugin_id
        ))
    })
}
pub async fn load_runtime_plugin_from_persisted_installation_payload(
    installation: &PluginInstallation,
    payload: &PersistedPluginWasmPayload,
) -> AppResult<RuntimePluginLoad> {
    let wasm_bytes = decode_persisted_plugin_wasm_payload(installation, payload).await?;
    let descriptor = parse_persisted_plugin_descriptor(installation)?;
    let validated = validate_downloaded_plugin_descriptor(
        &installation.plugin_id,
        &installation.plugin_type,
        &installation.provider_type,
        &installation_runtime_release(installation),
        &descriptor,
        installation.support_tier,
        false,
    )?;
    Ok(runtime_plugin_load_from_validated(
        validated.descriptor,
        wasm_bytes,
        installation_is_first_party(installation),
    ))
}
fn installation_sdk_contract_is_host_compatible(installation: &PluginInstallation) -> bool {
    match validate_sdk_contract(
        installation.plugin_id.as_str(),
        installation.sdk_version.as_str(),
        installation.sdk_constraint.as_str(),
        SDK_VERSION,
    ) {
        Ok(()) => true,
        Err(error) => {
            warn!(
                plugin_id = installation.plugin_id.as_str(),
                version = installation.version.as_str(),
                sdk_version = installation.sdk_version.as_str(),
                sdk_constraint = installation.sdk_constraint.as_str(),
                error = %error,
                "skipping installed plugin with incompatible sdk contract"
            );
            false
        }
    }
}
/// Support tiers permitted to run the host-process capability.
///
/// The host-process host lets a plugin spawn real OS processes on the Scryer
/// host. It is reserved for Scryer's own first-party plugins (`Official`).
/// Community plugins, including `VerifiedCommunity`, must not be installable
/// with this capability because runtime host bindings are only enabled for
/// first-party artifacts.
fn support_tier_permits_host_process(tier: PluginSupportTier) -> bool {
    matches!(tier, PluginSupportTier::Official)
}

/// Hard-fail validation of a plugin that declares the host-process capability
/// unless its resolved support tier is first-party. Applied on every
/// install path and when loading a persisted installation, so an untrusted
/// notifier can never turn an allowlisted interpreter into arbitrary host code.
fn ensure_host_process_capability_allowed(
    descriptor: &PluginDescriptor,
    support_tier: PluginSupportTier,
) -> AppResult<()> {
    if matches!(
        &descriptor.provider,
        ProviderDescriptor::Notification(notification)
            if notification.capabilities.requires_host_process
    ) && !support_tier_permits_host_process(support_tier)
    {
        warn!(
            plugin = descriptor.id.as_str(),
            provider_type = descriptor.provider_type(),
            support_tier = ?support_tier,
            "rejecting plugin: host-process capability is reserved for official plugins"
        );
        return Err(AppError::Validation(format!(
            "plugin '{}' requests the host-process capability, which is reserved for official plugins",
            descriptor.id
        )));
    }
    Ok(())
}

fn validate_downloaded_plugin_descriptor(
    plugin_id: &str,
    expected_plugin_type: &str,
    expected_provider_type: &str,
    release: &DownloadedPluginReleaseContract,
    descriptor: &PluginDescriptor,
    support_tier: PluginSupportTier,
    enforce_release_host_compatibility: bool,
) -> AppResult<ValidatedDownloadedPlugin> {
    validate_plugin_descriptor_sdk_contract(descriptor, SDK_VERSION)
        .map_err(AppError::Validation)?;
    validate_plugin_descriptor_host_permissions(descriptor).map_err(AppError::Validation)?;
    ensure_host_process_capability_allowed(descriptor, support_tier)?;

    if descriptor.id != plugin_id {
        return Err(AppError::Validation(format!(
            "downloaded plugin descriptor id '{}' does not match registry id '{}'",
            descriptor.id, plugin_id
        )));
    }
    if descriptor.plugin_type() != expected_plugin_type {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has plugin_type '{}' but registry expects '{}'",
            descriptor.id,
            descriptor.plugin_type(),
            expected_plugin_type
        )));
    }
    if normalize_provider_key(descriptor.provider_type())
        != normalize_provider_key(expected_provider_type)
    {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has provider_type '{}' but registry expects '{}'",
            descriptor.id,
            descriptor.provider_type(),
            expected_provider_type
        )));
    }
    if descriptor.version != release.version {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has version '{}' but the selected release is '{}'",
            descriptor.id, descriptor.version, release.version
        )));
    }
    if release
        .sdk_version
        .as_deref()
        .is_some_and(|sdk_version| !sdk_version.trim().is_empty())
        && release.sdk_version.as_deref() != Some(descriptor.sdk_version.as_str())
    {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has sdk_version '{}' but the selected release is '{}'",
            descriptor.id,
            descriptor.sdk_version,
            release.sdk_version.as_deref().unwrap_or_default()
        )));
    }
    let descriptor_sdk_constraint = plugin_descriptor_sdk_constraint(descriptor);
    let release_sdk_constraint = normalized_release_sdk_constraint(release);
    if descriptor_sdk_constraint != release_sdk_constraint {
        warn!(
            plugin_id = descriptor.id.as_str(),
            version = release.version.as_str(),
            descriptor_sdk_constraint = descriptor_sdk_constraint.as_str(),
            selected_sdk_constraint = release_sdk_constraint.as_str(),
            "downloaded plugin sdk_constraint differs from selected release metadata; using selected release constraint"
        );
    }
    if enforce_release_host_compatibility
        && !downloaded_plugin_release_is_host_compatible(plugin_id, release)
    {
        return Err(AppError::Validation(format!(
            "plugin '{}' no longer has a host-compatible release for this Scryer version",
            plugin_id
        )));
    }

    Ok(ValidatedDownloadedPlugin {
        descriptor: descriptor.clone(),
        sdk_constraint: release_sdk_constraint,
    })
}
#[derive(Debug)]
struct ValidatedDownloadedPlugin {
    descriptor: PluginDescriptor,
    sdk_constraint: String,
}
