use std::path::Path;

use crate::{AppServicesBuilder, AppUseCase};

pub struct UpgradeForTestInput<'a> {
    pub actor: &'a scryer_domain::User,
    pub title: &'a scryer_domain::Title,
    pub existing_file: &'a crate::TitleMediaFile,
    pub source_path: &'a Path,
    pub dest_path: &'a Path,
    pub parsed: crate::ParsedReleaseMetadata,
    pub final_score: i32,
    pub target_episode_ids: &'a [String],
    pub media_root: Option<&'a str>,
    pub recycle_config: &'a crate::recycle_bin::RecycleBinConfig,
}

fn import_source_snapshot_for_test(
    path: &Path,
) -> crate::AppResult<scryer_domain::ImportSourceSnapshot> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(path).map_err(|err| {
        crate::AppError::Repository(format!(
            "failed to stat test import source {}: {err}",
            path.display()
        ))
    })?;
    let bytes = std::fs::read(path).map_err(|err| {
        crate::AppError::Repository(format!(
            "failed to read test import source {}: {err}",
            path.display()
        ))
    })?;

    Ok(scryer_domain::ImportSourceSnapshot {
        identity: scryer_domain::ImportSourceIdentity {
            file: scryer_domain::ImportFileIdentity {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                #[cfg(unix)]
                dev: metadata.dev(),
                #[cfg(unix)]
                ino: metadata.ino(),
            },
            kind: scryer_domain::ImportSourceIdentityKind::Regular,
        },
        proof: scryer_domain::ImportContentProof {
            size_bytes: metadata.len(),
            sample_bytes: bytes.len() as u64,
            sample_blake3: blake3::hash(&bytes).to_hex().to_string(),
        },
    })
}

pub async fn execute_upgrade_for_test(
    app: &AppUseCase,
    input: UpgradeForTestInput<'_>,
) -> crate::AppResult<crate::upgrade::UpgradeResult> {
    execute_upgrade_for_test_with_import_mode(app, input, scryer_domain::ImportMode::HardlinkOrCopy)
        .await
}

pub async fn execute_upgrade_for_test_with_import_mode(
    app: &AppUseCase,
    input: UpgradeForTestInput<'_>,
    import_mode: scryer_domain::ImportMode,
) -> crate::AppResult<crate::upgrade::UpgradeResult> {
    let UpgradeForTestInput {
        actor,
        title,
        existing_file,
        source_path,
        dest_path,
        parsed,
        final_score,
        target_episode_ids,
        media_root,
        recycle_config,
    } = input;
    let prepared = crate::post_download_gate::PreparedImportCandidate {
        parsed,
        accepted: Box::new(crate::post_download_gate::ImportedFileAcceptance {
            analysis: None,
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        }),
        rescore_changes: Vec::new(),
        source_snapshot: import_source_snapshot_for_test(source_path)?,
    };
    let old_score = existing_file.acquisition_score.unwrap_or(0);
    let old_file_media_root = crate::fs_safety::most_specific_containing_root(
        &crate::stored_paths::stored_path_to_path_buf(&existing_file.file_path),
        &recycle_config.source_roots,
    )
    .map(|root| crate::stored_paths::path_to_stored_string(&root));

    // Tests exercise the upgrade without a queued import record; the
    // progress writes for this id simply match no row.
    let result = crate::upgrade::execute_upgrade(
        app,
        actor,
        "upgrade-for-test",
        title,
        existing_file,
        source_path,
        dest_path,
        &prepared,
        prepared.parsed.quality.as_deref(),
        final_score,
        old_score,
        None,
        target_episode_ids,
        media_root,
        old_file_media_root.as_deref().or(media_root),
        recycle_config,
        import_mode,
        None,
        None,
    )
    .await?;
    if let crate::upgrade::UpgradeResult::Upgraded(outcome) = &result {
        crate::upgrade::finalize_upgrade_source_cleanup(app, outcome, None).await?;
    }
    Ok(result)
}

pub trait AppUseCaseTestExt {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder;

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64>;
}

impl AppUseCaseTestExt for AppUseCase {
    fn with_test_overrides<F>(&self, configure: F) -> AppUseCase
    where
        F: FnOnce(AppServicesBuilder) -> AppServicesBuilder,
    {
        AppUseCase::with_test_overrides(self, configure)
    }

    fn notification_wake_receiver(&self) -> tokio::sync::broadcast::Receiver<i64> {
        self.runtime.events.notification_event_broadcast.subscribe()
    }
}
