use super::*;

#[tokio::test]
async fn probe_stability_metadata_matrix() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let fixture = fail_closed_pack_fixture().await;
        let profile = fixture
            .app
            .resolve_quality_profile_for_title(&fixture.title)
            .await
            .unwrap();
        let mut original = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scryer-mediainfo/tests/media/h264_aac.mkv"),
        )
        .unwrap();
        original.extend_from_slice(&[0; 16]);
        for case in [
            "unchanged",
            "read",
            "atime",
            "hardlink",
            "mtime",
            "inode",
            "append",
            "sample",
        ] {
            // The application's native version key includes inode identity on Unix.
            if case == "inode" && !cfg!(unix) {
                continue;
            }
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("source.mkv");
            std::fs::write(&path, &original).unwrap();
            let parsed = crate::release_parser::parse_release_metadata("Example.S01E01.mkv");
            let before = std::fs::metadata(&path).unwrap();
            let (release, references) =
                crate::media::analyzer::hold_native_probe_for_test(path.clone()).await;
            let (decision, ()) = tokio::join!(
                crate::post_download_gate::prepare_import_candidate_with_disc_selection(
                    &fixture.app,
                    &fixture.title,
                    &parsed,
                    &profile,
                    &path,
                    original.len() as i64,
                    false,
                    None,
                    false,
                    crate::post_download_gate::RuntimeSampleValidation::manual_override(None),
                    None,
                    None,
                ),
                async {
                    while references() < 3 {
                        tokio::task::yield_now().await;
                    }
                    let later = before.modified().unwrap() + std::time::Duration::from_secs(60);
                    match case {
                        "unchanged" => {}
                        "read" => {
                            assert_eq!(std::fs::read(&path).unwrap(), original);
                        }
                        "atime" => {
                            std::fs::OpenOptions::new()
                                .write(true)
                                .open(&path)
                                .unwrap()
                                .set_times(std::fs::FileTimes::new().set_accessed(later))
                                .unwrap();
                        }
                        "hardlink" => {
                            std::fs::hard_link(&path, directory.path().join("linked.mkv")).unwrap();
                        }
                        "mtime" => {
                            std::fs::OpenOptions::new()
                                .write(true)
                                .open(&path)
                                .unwrap()
                                .set_modified(later)
                                .unwrap();
                        }
                        "inode" => {
                            let replacement = directory.path().join("replacement.mkv");
                            std::fs::write(&replacement, &original).unwrap();
                            std::fs::OpenOptions::new()
                                .write(true)
                                .open(&replacement)
                                .unwrap()
                                .set_modified(before.modified().unwrap())
                                .unwrap();
                            std::fs::rename(replacement, &path).unwrap();
                        }
                        "append" => {
                            use std::io::Write;
                            std::fs::OpenOptions::new()
                                .append(true)
                                .open(&path)
                                .unwrap()
                                .write_all(&[0; 16])
                                .unwrap();
                        }
                        "sample" => {
                            use std::io::{Seek, SeekFrom, Write};
                            let mut file =
                                std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                            file.seek(SeekFrom::End(-1)).unwrap();
                            file.write_all(&[1]).unwrap();
                            file.set_modified(before.modified().unwrap()).unwrap();
                        }
                        _ => unreachable!(),
                    }
                    release.send(()).unwrap();
                }
            );
            let should_reject = matches!(case, "inode" | "append" | "sample");
            match decision {
                Ok(_) => assert!(!should_reject, "{case}"),
                Err(rejection) => {
                    assert!(should_reject, "{case}: {rejection:?}");
                    assert_eq!(
                        rejection.recycle_reason,
                        "import_source_changed_after_probe"
                    );
                    assert!(rejection.requires_review());
                    eprintln!("{case}: {}", rejection.message);
                }
            }
            let retried = crate::post_download_gate::prepare_import_candidate_with_disc_selection(
                &fixture.app,
                &fixture.title,
                &parsed,
                &profile,
                &path,
                std::fs::metadata(&path).unwrap().len() as i64,
                false,
                None,
                false,
                crate::post_download_gate::RuntimeSampleValidation::manual_override(None),
                None,
                None,
            )
            .await;
            assert!(retried.is_ok(), "{case}: stable retry should pass");
            eprintln!("{case}: first_rejected={should_reject}, stable_retry=accepted");
        }
    })
    .await
    .expect("bounded reproduction");
}

#[tokio::test]
async fn probe_stability_missing_source_has_distinct_retryable_reason() {
    let fixture = fail_closed_pack_fixture().await;
    let profile = fixture
        .app
        .resolve_quality_profile_for_title(&fixture.title)
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("never-existed.mkv");
    let parsed = crate::release_parser::parse_release_metadata("Example.S01E01.mkv");
    let result = crate::post_download_gate::prepare_import_candidate_with_disc_selection(
        &fixture.app,
        &fixture.title,
        &parsed,
        &profile,
        &path,
        0,
        false,
        None,
        false,
        crate::post_download_gate::RuntimeSampleValidation::manual_override(None),
        None,
        None,
    )
    .await;
    let Err(rejection) = result else {
        panic!("missing source must not pass")
    };
    assert_eq!(
        rejection.recycle_reason,
        "import_source_snapshot_unavailable"
    );
    assert!(rejection.requires_review());
    assert_eq!(
        rejection.review_hold_skip_reason(),
        scryer_domain::ImportSkipReason::SourceUnavailable
    );
    assert!(rejection.message.contains("failed to stat import source"));
    eprintln!("{}", rejection.message);
}

struct MutatingSnapshotImporter {
    calls: std::sync::atomic::AtomicUsize,
    repeat: bool,
    fail_snapshot: Option<usize>,
}

#[async_trait]
impl FileImporter for MutatingSnapshotImporter {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail_snapshot == Some(call) {
            return Err(AppError::Repository(
                "synthetic snapshot read failure".into(),
            ));
        }
        let snapshot = CopyingFileImporter.snapshot_import_source(source).await?;
        if call == 0 || (self.repeat && call.is_multiple_of(2)) {
            std::fs::OpenOptions::new()
                .write(true)
                .open(source)
                .unwrap()
                .set_modified(
                    snapshot.identity.file.modified.unwrap() + std::time::Duration::from_secs(60),
                )
                .unwrap();
        }
        Ok(snapshot)
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        CopyingFileImporter
            .import_file(source, dest, mode, expected_source)
            .await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        guard: scryer_domain::ImportSourceCleanupGuard,
        dest: &Path,
    ) -> AppResult<()> {
        CopyingFileImporter
            .remove_import_source_after_verified_import(guard, dest)
            .await
    }
}

#[tokio::test]
async fn probe_stability_timestamp_finalization_imports_without_rejected_artifact() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (fixture, _) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
            file_importer: Arc::new(MutatingSnapshotImporter {
                calls: std::sync::atomic::AtomicUsize::new(0),
                repeat: false,
                fail_snapshot: None,
            }),
            ..Default::default()
        })
        .await;
        let source = tempfile::tempdir().unwrap();
        let name = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv";
        write_pack_video(source.path(), name);
        let completed = series_pack_completed_download(
            "probe-reproduction",
            &fixture.title.id,
            "Fail.Closed.Pack.S01E01.1080p.WEB-DL",
            source.path(),
        );
        let _probe = probe_sequence_agrees_with_the_names([(1920, 1080), (1920, 1080)]);
        let first = crate::import::import::import_completed_download(
            &fixture.app,
            &fixture.user,
            &completed,
        )
        .await
        .unwrap();
        assert_eq!(
            first.decision,
            scryer_domain::ImportDecision::Imported,
            "{first:?}"
        );
        assert!(!first.release_burned);
        let artifacts = fixture.import_artifacts.artifacts_for_file(name).await;
        assert_eq!(artifacts.len(), 1, "{artifacts:?}");
        assert_eq!(artifacts[0].result, "imported");
        let records = fixture.import_repo.records.lock().await.clone();
        let record = records
            .iter()
            .find(|record| record.id == first.import_id)
            .unwrap();
        assert_eq!(record.status, scryer_domain::ImportStatus::Completed);
        let events = fixture
            .app
            .services
            .events
            .domain_events
            .list(&DomainEventFilter {
                event_types: Some(vec![DomainEventType::ImportRejected]),
                title_id: Some(fixture.title.id.clone()),
                facet: None,
                stream_id: None,
                after_sequence: Some(0),
                before_sequence: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert!(events.is_empty(), "{events:?}");
        let events = import_completed_events_for_title(&fixture.app, &fixture.title.id).await;
        assert_eq!(events.len(), 1);
        eprintln!("first: imported artifact, Completed import, one ImportCompleted event");
    })
    .await
    .expect("bounded import reproduction");
}

#[tokio::test]
async fn probe_stability_repeated_timestamp_changes_remain_pending_and_preserve_source() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let importer = Arc::new(MutatingSnapshotImporter {
            calls: std::sync::atomic::AtomicUsize::new(0),
            repeat: true,
            fail_snapshot: None,
        });
        let (fixture, _) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
            file_importer: importer.clone(),
            ..Default::default()
        })
        .await;
        let source = tempfile::tempdir().unwrap();
        let name = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv";
        let path = write_pack_video(source.path(), name);
        let unrelated = source.path().join("unrelated.txt");
        std::fs::write(&unrelated, b"preserve me").unwrap();
        let completed = series_pack_completed_download(
            "probe-repeated",
            &fixture.title.id,
            "Fail.Closed.Pack.S01E01.1080p.WEB-DL",
            source.path(),
        );
        let _probe = probe_sequence_agrees_with_the_names([(1920, 1080), (1920, 1080)]);
        let result = crate::import::import::import_completed_download(
            &fixture.app,
            &fixture.user,
            &completed,
        )
        .await
        .unwrap();
        assert!(
            crate::import_workflow::completed_import_result_is_retryable(&result),
            "{result:?}"
        );
        assert!(!result.release_burned);
        assert!(path.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"preserve me");
        assert!(library_video_file_names(fixture.library_dir.path()).is_empty());
        assert_eq!(importer.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
        let artifacts = fixture.import_artifacts.artifacts_for_file(name).await;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0].reason_code.as_deref(),
            Some("import_source_changed_after_probe")
        );
        assert!(
            result
                .error_message
                .as_deref()
                .unwrap()
                .contains("modification time")
        );
        let records = fixture.import_repo.records.lock().await;
        assert_eq!(
            records
                .iter()
                .find(|record| record.id == result.import_id)
                .unwrap()
                .status,
            scryer_domain::ImportStatus::Pending
        );
    })
    .await
    .expect("bounded import retry");
}

#[tokio::test]
async fn probe_stability_snapshot_io_failures_stay_retryable_and_preserve_source() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        for fail_snapshot in [0, 1] {
            let (fixture, _) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
                file_importer: Arc::new(MutatingSnapshotImporter {
                    calls: std::sync::atomic::AtomicUsize::new(0),
                    repeat: false,
                    fail_snapshot: Some(fail_snapshot),
                }),
                ..Default::default()
            })
            .await;
            let source = tempfile::tempdir().unwrap();
            let name = "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv";
            let path = write_pack_video(source.path(), name);
            let completed = series_pack_completed_download(
                "probe-io",
                &fixture.title.id,
                "Fail.Closed.Pack.S01E01.1080p.WEB-DL",
                source.path(),
            );
            let _probe = probe_agrees_with_the_name(1920, 1080);
            let result = crate::import::import::import_completed_download(
                &fixture.app,
                &fixture.user,
                &completed,
            )
            .await
            .unwrap();
            assert!(
                crate::import_workflow::completed_import_result_is_retryable(&result),
                "{result:?}"
            );
            assert!(!result.release_burned);
            assert!(path.exists());
            assert!(library_video_file_names(fixture.library_dir.path()).is_empty());
            let artifacts = fixture.import_artifacts.artifacts_for_file(name).await;
            assert_eq!(artifacts.len(), 1);
            assert_eq!(
                artifacts[0].reason_code.as_deref(),
                Some("import_source_snapshot_unavailable")
            );
            assert!(
                result
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("synthetic snapshot read failure")
            );
        }
    })
    .await
    .expect("bounded snapshot failure reproduction");
}

#[tokio::test]
async fn probe_stability_second_validation_still_enforces_runtime_policy() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (fixture, _) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
            file_importer: Arc::new(MutatingSnapshotImporter {
                calls: std::sync::atomic::AtomicUsize::new(0),
                repeat: false,
                fail_snapshot: None,
            }),
            ..Default::default()
        })
        .await;
        let profile = fixture
            .app
            .resolve_quality_profile_for_title(&fixture.title)
            .await
            .unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("source.mkv");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scryer-mediainfo/tests/media/h264_aac.mkv"),
            &path,
        )
        .unwrap();
        let original = std::fs::read(&path).unwrap();
        let parsed = crate::release_parser::parse_release_metadata("Example.S01E01.mkv");
        // Only the first probe is accepted by the seam. The second must run
        // the real native probe and reject this one-second fixture as a sample.
        let _probe = probe_agrees_with_the_name(1920, 1080);
        let result = crate::post_download_gate::prepare_import_candidate_with_disc_selection(
            &fixture.app,
            &fixture.title,
            &parsed,
            &profile,
            &path,
            original.len() as i64,
            false,
            None,
            false,
            crate::post_download_gate::RuntimeSampleValidation::automatic(Some(1500)),
            None,
            None,
        )
        .await;
        let Err(rejection) = result else {
            panic!("a second validation must not bypass the runtime policy")
        };
        assert_eq!(rejection.recycle_reason, "sample_runtime_too_short");
        assert_eq!(std::fs::read(&path).unwrap(), original);
    })
    .await
    .expect("bounded policy revalidation");
}

struct RestoringSnapshotImporter {
    original_modified: std::time::SystemTime,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl FileImporter for RestoringSnapshotImporter {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            // Restore only after the native probe has observed the change.
            std::fs::OpenOptions::new()
                .write(true)
                .open(source)
                .unwrap()
                .set_modified(self.original_modified)
                .unwrap();
        }
        CopyingFileImporter.snapshot_import_source(source).await
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        CopyingFileImporter
            .import_file(source, dest, mode, expected_source)
            .await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        guard: scryer_domain::ImportSourceCleanupGuard,
        dest: &Path,
    ) -> AppResult<()> {
        CopyingFileImporter
            .remove_import_source_after_verified_import(guard, dest)
            .await
    }
}

#[tokio::test]
async fn probe_stability_preserves_inner_rejection_when_outer_snapshots_match() {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("source.mkv");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../scryer-mediainfo/tests/media/h264_aac.mkv"),
            &path,
        ).unwrap();
        let before = CopyingFileImporter.snapshot_import_source(&path).await.unwrap();
        let original_modified = before.identity.file.modified.unwrap();
        let importer = Arc::new(RestoringSnapshotImporter {
            original_modified,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let (fixture, _) = build_fail_closed_pack_fixture(FailClosedPackFixtureOptions {
            file_importer: importer.clone(),
            ..Default::default()
        }).await;
        let profile = fixture.app.resolve_quality_profile_for_title(&fixture.title).await.unwrap();
        let parsed = crate::release_parser::parse_release_metadata("Example.S01E01.mkv");
        let (release, references) = crate::media::analyzer::hold_native_probe_for_test(path.clone()).await;
        let (result, ()) = tokio::join!(
            crate::post_download_gate::prepare_import_candidate_with_disc_selection(
                &fixture.app, &fixture.title, &parsed, &profile, &path, before.identity.file.len as i64,
                false, None, false, crate::post_download_gate::RuntimeSampleValidation::manual_override(None), None, None,
            ),
            async {
                while references() < 3 { tokio::task::yield_now().await; }
                std::fs::OpenOptions::new().write(true).open(&path).unwrap()
                    .set_modified(original_modified + std::time::Duration::from_secs(60)).unwrap();
                release.send(()).unwrap();
            },
        );
        let Err(rejection) = result else { panic!("the inner source change must still reject") };
        assert_eq!(rejection.message, format!(
            "import source changed after validation probe: {} (validation: media source changed during inspection)",
            path.display(),
        ));
        assert_eq!(rejection.recycle_reason, "import_source_changed_after_probe");
        assert_eq!(rejection.blocking_rule_codes, vec!["source_changed_after_probe"]);
        assert!(rejection.requires_review());
        assert_eq!(CopyingFileImporter.snapshot_import_source(&path).await.unwrap(), before);
        assert_eq!(importer.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }).await.expect("bounded inner-rejection preservation");
}
