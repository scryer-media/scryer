//! FR-047 / SC-007: the backfill job converges, skips what it must, and
//! resumes.

use super::*;

use crate::location::backfill::FullHashBackfillOptions;
use crate::location::ownership_guard::OwnedEntity;
use crate::location::test_support::InMemoryLocationOperationStore;

/// A media file row on disk, in the catalog, with no full hash yet.
struct SeededFile {
    id: String,
    path: PathBuf,
}

fn seed_row(id: &str, path: &Path, title_id: &str) -> TitleMediaFile {
    TitleMediaFile {
        id: id.to_string(),
        title_id: title_id.to_string(),
        episode_id: None,
        series_movie_link_ids: Vec::new(),
        role: crate::MediaFileRole::Primary,
        file_path: crate::stored_paths::path_to_stored_string(path),
        size_bytes: std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0),
        announced_size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
        content_hashes: None,
        quality_label: None,
        scan_status: "scanned".into(),
        created_at: String::new(),
        video_codec: None,
        video_width: None,
        video_height: None,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: None,
        scene_name: None,
        release_group: None,
        source_type: None,
        resolution: None,
        video_codec_parsed: None,
        audio_codec_parsed: None,
        audio_channels_parsed: None,
        acquisition_score: None,
        scoring_log: None,
        indexer_source: None,
        grabbed_release_title: None,
        grabbed_at: None,
        edition: None,
        original_file_path: None,
        release_hash: None,
    }
}

/// Writes `count` files on disk and seeds a catalog row for each.
async fn seed_unhashed_files(
    media_files: &Arc<MockMediaFileRepo>,
    dir: &Path,
    count: usize,
) -> Vec<SeededFile> {
    let mut seeded = Vec::new();
    for index in 0..count {
        // Zero-padded so id order and creation order agree; the cursor is an id
        // comparison, and a test that relies on a coincidence of ordering
        // proves nothing.
        let id = format!("file-{index:03}");
        let path = dir.join(format!("{id}.mkv"));
        std::fs::write(&path, format!("contents of {id}").as_bytes()).expect("write media file");
        media_files
            .store
            .lock()
            .await
            .push(seed_row(&id, &path, "title-1"));
        seeded.push(SeededFile { id, path });
    }
    seeded
}

async fn hashed_ids(media_files: &Arc<MockMediaFileRepo>) -> Vec<String> {
    media_files
        .store
        .lock()
        .await
        .iter()
        .filter(|row| row.content_hashes.is_some())
        .map(|row| row.id.clone())
        .collect()
}

fn bootstrap_backfill(media_files: Arc<MockMediaFileRepo>) -> AppUseCase {
    let (app, _actor) = bootstrap();
    app.with_test_overrides(|services| services.with_media_files(media_files))
}

fn bootstrap_backfill_with_operations(
    media_files: Arc<MockMediaFileRepo>,
    operations: Arc<InMemoryLocationOperationStore>,
) -> AppUseCase {
    let (app, _actor) = bootstrap();
    app.with_test_overrides(|services| {
        services
            .with_media_files(media_files)
            .with_location_operation_repository(operations)
    })
}

/// US9 scenario 4 / SC-007: an idle catalog converges to full coverage.
#[tokio::test]
async fn backfill_hashes_every_queued_file_and_persists_both_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    let seeded = seed_unhashed_files(&media_files, dir.path(), 5).await;

    let app = bootstrap_backfill(media_files.clone());
    let summary = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    assert_eq!(summary.hashed, seeded.len());
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.skipped_unavailable, 0);
    assert_eq!(summary.skipped_owned, 0);
    assert!(
        summary.completed_sweep,
        "a queue this small drains inside one run"
    );
    assert_eq!(hashed_ids(&media_files).await.len(), seeded.len());

    // Both values, algorithm-tagged, with a vintage (FR-041): a hash with no
    // `hash_computed_at` reads back as stale, so persisting one would be worse
    // than persisting nothing.
    let rows = media_files.store.lock().await;
    for row in rows.iter() {
        let hashes = row.content_hashes.as_ref().expect("persisted hashes");
        assert_eq!(hashes.full_blake3.len(), 64, "BLAKE3 hex digest");
        assert!(hashes.move_crc.is_some(), "the CRC rides along for free");
        assert_eq!(
            hashes.crc_algorithm,
            Some(crate::location::model::MoveCrcAlgorithm::Crc64Nvme)
        );
        assert!(hashes.hash_computed_at.is_some());
    }
}

/// The hash the job persists is the hash of the bytes, not an artifact of how
/// they were read.
#[tokio::test]
async fn backfilled_hash_matches_the_file_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    let seeded = seed_unhashed_files(&media_files, dir.path(), 1).await;
    let expected = blake3::hash(&std::fs::read(&seeded[0].path).expect("read seeded file"))
        .to_hex()
        .to_string();

    let app = bootstrap_backfill(media_files.clone());
    app.run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    let rows = media_files.store.lock().await;
    assert_eq!(
        rows[0]
            .content_hashes
            .as_ref()
            .expect("persisted hashes")
            .full_blake3,
        expected
    );
}

/// Already-hashed rows never leave the queue predicate, so a second run does no
/// work at all — the "re-run without rework" half of SC-007.
#[tokio::test]
async fn a_second_run_over_a_hashed_catalog_does_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    seed_unhashed_files(&media_files, dir.path(), 3).await;

    let app = bootstrap_backfill(media_files.clone());
    let first = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("first run");
    assert_eq!(first.hashed, 3);

    let second = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("second run");
    assert_eq!(second.examined, 0);
    assert_eq!(second.hashed, 0);
    assert!(second.completed_sweep);
}

/// SC-007: interrupt and re-run. A bounded run stops early and the next one
/// picks up where it left off instead of restarting the sweep.
#[tokio::test]
async fn a_bounded_run_resumes_from_its_persisted_cursor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    let seeded = seed_unhashed_files(&media_files, dir.path(), 6).await;

    let app = bootstrap_backfill(media_files.clone());
    let bounded = FullHashBackfillOptions {
        files_per_run: 2,
        page_size: 2,
        ..FullHashBackfillOptions::unthrottled()
    };

    let first = app
        .run_full_hash_backfill_with_options(bounded)
        .await
        .expect("first bounded run");
    assert_eq!(first.hashed, 2);
    assert!(!first.completed_sweep);
    assert_eq!(first.resume_after_id.as_deref(), Some(seeded[1].id.as_str()));
    assert_eq!(hashed_ids(&media_files).await, vec![
        seeded[0].id.clone(),
        seeded[1].id.clone()
    ]);

    // A *fresh* use case, as a restart would produce: the cursor has to come
    // back from the settings store, not from in-process state.
    let restarted = bootstrap_backfill(media_files.clone());
    let second = restarted
        .run_full_hash_backfill_with_options(bounded)
        .await
        .expect("second bounded run");
    assert_eq!(second.hashed, 2);
    assert_eq!(
        second.resume_after_id.as_deref(),
        Some(seeded[3].id.as_str())
    );
    assert_eq!(hashed_ids(&media_files).await.len(), 4);
}

/// A sweep that reaches the end clears the cursor, so the next run starts over
/// and re-examines whatever was skipped.
#[tokio::test]
async fn a_completed_sweep_clears_the_cursor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    seed_unhashed_files(&media_files, dir.path(), 2).await;

    let app = bootstrap_backfill(media_files.clone());
    let summary = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    assert!(summary.completed_sweep);
    assert_eq!(summary.resume_after_id, None);
    assert_eq!(
        app.read_setting_json_value::<crate::location::backfill::FullHashBackfillCursor>(
            crate::settings::keys::FULL_HASH_BACKFILL_CURSOR_KEY,
            None,
        )
        .await
        .expect("read cursor")
        .expect("cursor row")
        .after_id,
        None
    );
}

/// FR-047: an unreachable mount is skipped, not failed, and never blocks the
/// files behind it.
#[tokio::test]
async fn files_on_an_unavailable_mount_are_skipped_and_do_not_stall_the_sweep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    let seeded = seed_unhashed_files(&media_files, dir.path(), 3).await;

    // Row 000 points into a directory that does not exist — the shape an
    // unmounted share takes.
    {
        let mut rows = media_files.store.lock().await;
        rows[0].file_path = crate::stored_paths::path_to_stored_string(
            &dir.path().join("unmounted-share").join("gone.mkv"),
        );
    }

    let app = bootstrap_backfill(media_files.clone());
    let summary = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    assert_eq!(summary.skipped_unavailable, 1);
    assert_eq!(summary.failed, 0, "an absent mount is a skip, not a failure");
    assert_eq!(summary.hashed, 2);
    assert_eq!(hashed_ids(&media_files).await, vec![
        seeded[1].id.clone(),
        seeded[2].id.clone()
    ]);
}

/// SC-007: "never touches a file owned by an active operation."
#[tokio::test]
async fn files_owned_by_an_active_operation_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    seed_unhashed_files(&media_files, dir.path(), 2).await;
    {
        let mut rows = media_files.store.lock().await;
        rows[0].title_id = "owned-title".to_string();
    }

    let operations = Arc::new(InMemoryLocationOperationStore::new());
    operations
        .claim_location_operation_ownership(
            "operation-1",
            &[OwnedEntity::Title("owned-title".to_string())],
        )
        .await
        .expect("claim ownership");

    let app = bootstrap_backfill_with_operations(media_files.clone(), operations.clone());
    let summary = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    assert_eq!(summary.skipped_owned, 1);
    assert_eq!(summary.hashed, 1);
    assert_eq!(hashed_ids(&media_files).await, vec!["file-001".to_string()]);

    // Once the operation releases, the next sweep picks the file up — the
    // wrap-around is the whole retry mechanism.
    operations
        .release_location_operation_ownership("operation-1")
        .await
        .expect("release ownership");
    let after = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("second backfill run");
    assert_eq!(after.hashed, 1);
    assert_eq!(hashed_ids(&media_files).await.len(), 2);
}

/// A row hashed by something else (an import, a move) between the queue page
/// and its turn is skipped rather than re-read.
#[tokio::test]
async fn a_file_hashed_since_the_queue_page_is_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    seed_unhashed_files(&media_files, dir.path(), 1).await;

    let app = bootstrap_backfill(media_files.clone());
    // Simulate the race by hashing the row through the same accessor an import
    // would use, before the job runs.
    app.services
        .library
        .media_files
        .update_media_file_content_hashes(
            "file-000",
            &crate::location::model::PersistedContentHashes {
                full_blake3: "deadbeef".repeat(8),
                move_crc: Some(1),
                crc_algorithm: Some(crate::location::model::MoveCrcAlgorithm::Crc64Nvme),
                hash_computed_at: Some(Utc::now()),
            },
        )
        .await
        .expect("pre-hash the row");

    let summary = app
        .run_full_hash_backfill_with_options(FullHashBackfillOptions::unthrottled())
        .await
        .expect("backfill run");

    assert_eq!(summary.hashed, 0);
    assert_eq!(summary.examined, 0, "the queue predicate already excluded it");
}

/// The throttle is real work, not a comment: production options carry a pause
/// between files, and a run with three queued files must wait for at least two
/// of them.
#[tokio::test(start_paused = true)]
async fn the_job_pauses_between_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let media_files = Arc::new(MockMediaFileRepo::default());
    seed_unhashed_files(&media_files, dir.path(), 3).await;

    let app = bootstrap_backfill(media_files.clone());
    let options = FullHashBackfillOptions {
        chunk_pause: Duration::ZERO,
        ..FullHashBackfillOptions::default()
    };

    let started = Instant::now();
    let summary = app
        .run_full_hash_backfill_with_options(options)
        .await
        .expect("backfill run");

    assert_eq!(summary.hashed, 3);
    assert!(
        started.elapsed() >= options.file_pause * 3,
        "three files must have paused three times, saw {:?}",
        started.elapsed()
    );
}

// ── FR-046: scan-side invalidation ──────────────────────────────────────────

/// Seeds a scanned movie file with persisted full hashes and returns
/// (app, user, title, path, file id).
async fn seed_scanned_movie_with_hashes(
    tempdir: &tempfile::TempDir,
    contents: &[u8],
) -> (AppUseCase, User, Title, PathBuf, String) {
    let title_dir = tempdir.path().join("Invalidation Subject (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let movie_path = title_dir.join("Invalidation.Subject.2026.1080p.WEB-DL.mkv");
    std::fs::write(&movie_path, contents).expect("write movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        Vec::new(),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Invalidation Subject", title_dir.as_path())
            .await;

    // Scan once so the row carries the signature the next scan compares
    // against.
    app.scan_title_library_with_discovered_files(
        &user,
        title.clone(),
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("seed scan");

    let file_id = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list seeded media files")
        .first()
        .expect("seeded media file")
        .id
        .clone();

    app.services
        .library
        .media_files
        .update_media_file_content_hashes(
            &file_id,
            &crate::location::model::PersistedContentHashes {
                full_blake3: "ab".repeat(32),
                move_crc: Some(42),
                crc_algorithm: Some(crate::location::model::MoveCrcAlgorithm::Crc64Nvme),
                hash_computed_at: Some(Utc::now()),
            },
        )
        .await
        .expect("seed persisted hashes");

    (app, user, title, movie_path, file_id)
}

async fn content_hashes_of(app: &AppUseCase, file_id: &str) -> Option<PersistedContentHashes> {
    app.services
        .library
        .media_files
        .get_media_file_by_id(file_id)
        .await
        .expect("load media file")
        .expect("media file exists")
        .content_hashes
}

use crate::location::model::PersistedContentHashes;

/// FR-046: a scan that sees the sampled proof change clears the stored full
/// hashes, putting the file back on the backfill queue.
#[tokio::test]
async fn a_changed_quick_proof_clears_the_persisted_full_hashes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let (app, user, title, movie_path, file_id) =
        seed_scanned_movie_with_hashes(&tempdir, b"original media bytes").await;
    assert!(content_hashes_of(&app, &file_id).await.is_some());

    // Rewrite the file: different size, different mtime — the quick proof
    // changed.
    std::fs::write(&movie_path, b"the file was replaced with different bytes")
        .expect("rewrite movie file");

    app.scan_title_library_with_discovered_files(
        &user,
        title,
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("rescan changed file");

    assert!(
        content_hashes_of(&app, &file_id).await.is_none(),
        "a changed quick proof must invalidate the persisted full hashes"
    );

    // And the file is back on the backfill queue, which is what invalidation is
    // *for*.
    let queued = app
        .services
        .library
        .media_files
        .list_media_files_missing_full_hash(None, 10)
        .await
        .expect("read backfill queue");
    assert!(queued.iter().any(|candidate| candidate.id == file_id));
}

/// The other half of FR-046: an unchanged file keeps its hash. Re-hashing every
/// scanned file would make the expensive value worthless.
#[tokio::test]
async fn an_unchanged_file_keeps_its_persisted_full_hashes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let (app, user, title, movie_path, file_id) =
        seed_scanned_movie_with_hashes(&tempdir, b"stable media bytes").await;
    let before = content_hashes_of(&app, &file_id).await.expect("seeded hashes");

    app.scan_title_library_with_discovered_files(
        &user,
        title,
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("rescan unchanged file");

    assert_eq!(
        content_hashes_of(&app, &file_id).await,
        Some(before),
        "an unchanged file must keep the hash something already paid to compute"
    );
}

/// FR-046: "scans never compute full hashes." A file the scan discovers for the
/// first time comes out of the scan with no full hash at all — the backfill job
/// is the only thing that fills one in.
#[tokio::test]
async fn a_scan_never_computes_a_full_hash() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Never Hashed (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let movie_path = title_dir.join("Never.Hashed.2026.1080p.WEB-DL.mkv");
    std::fs::write(&movie_path, b"freshly discovered media bytes").expect("write movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        Vec::new(),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title = create_movie_title_with_folder(&app, &user, "Never Hashed", title_dir.as_path())
        .await;

    app.scan_title_library_with_discovered_files(
        &user,
        title.clone(),
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("scan new file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert!(
        files[0].content_hashes.is_none(),
        "the scan must leave the expensive columns for the backfill job"
    );
    // The sampled proof, by contrast, is exactly what a scan is allowed to
    // compute — and did.
    assert!(files[0].source_signature_value.is_some());
}
