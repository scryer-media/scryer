use super::*;
use crate::location::{
    execution::RootMoveFileMover,
    executor::{PlannedFile, PlannedTitle, TitleOutcomeCounts},
    model::TitleCheckpointPlacement,
    test_support::InMemoryLocationOperationStore,
};

fn title(source: &Path, destination: &Path, name: &str, size: u64) -> PlannedTitle {
    PlannedTitle {
        title_id: "title".into(),
        sequence: 0,
        classification: None,
        placement: TitleCheckpointPlacement::default(),
        outcomes: TitleOutcomeCounts::default(),
        files: vec![PlannedFile {
            media_file_id: Some("media".into()),
            source_path: source.join(name),
            destination_path: destination.join(name),
            size_bytes: size,
            source_content: None,
        }],
    }
}

async fn resolve(
    resolver: &ConflictResolver,
    title: &PlannedTitle,
    index: usize,
) -> AppResult<VerifiedFile> {
    resolver
        .resolve(
            &RootMoveFileMover::without_permissions(),
            FileMoveRequest {
                operation_id: "op",
                title,
                file: &title.files[index],
                depth: VerificationDepth::Quick,
                progress: &CopyProgress::none(),
            },
        )
        .await
}

fn fixture(
    temp: &tempfile::TempDir,
) -> (
    Arc<InMemoryLocationOperationStore>,
    ConflictResolver,
    PlannedTitle,
) {
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    let title = title(&source, &destination, "season.nfo", 8);
    let store = Arc::new(InMemoryLocationOperationStore::new());
    let resolver = ConflictResolver::new(
        store.clone(),
        BTreeMap::from([("title".into(), "AnimeTesting".into())]),
    );
    (store, resolver, title)
}

/// A comparison sink that records the highest byte count it was told about
/// and whether it was told anything at all: a pair the quick check settles
/// never reaches the full comparison, so the sink stays silent.
fn counting_progress() -> (
    CopyProgress,
    Arc<std::sync::atomic::AtomicU64>,
    Arc<std::sync::atomic::AtomicBool>,
) {
    let reads = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let count = reads.clone();
    let flag = called.clone();
    let progress = CopyProgress::none().with_comparison_sink(move |bytes, _| {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        count.fetch_max(bytes, std::sync::atomic::Ordering::Relaxed);
    });
    (progress, reads, called)
}

#[tokio::test]
async fn different_existing_files_keep_destination_and_reuse_durable_provenance_on_resume() {
    let temp = tempfile::tempdir().unwrap();
    let (store, resolver, title) = fixture(&temp);
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::write(&title.files[0].destination_path, b"original").unwrap();
    let first = resolve(&resolver, &title, 0).await.unwrap();
    assert!(first.permits_source_removal());
    assert_eq!(
        first.destination_path.file_name().unwrap(),
        "season (from AnimeTesting).nfo"
    );
    assert_eq!(
        std::fs::read(&title.files[0].destination_path).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read(&first.destination_path).unwrap(), b"incoming");
    let record = store
        .file_resolutions("op", "title")
        .await
        .unwrap()
        .remove(0);
    assert!(record.completed && record.warning.is_some() && record.proof_is_current().await);
    let resumed = ConflictResolver::new(store, resolver.contexts.clone());
    assert_eq!(
        resolve(&resumed, &title, 0).await.unwrap().destination_path,
        first.destination_path
    );
}

#[tokio::test]
async fn identical_existing_content_is_fully_compared_even_when_quick_copy_policy_was_requested() {
    let temp = tempfile::tempdir().unwrap();
    let (store, resolver, title) = fixture(&temp);
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::write(&title.files[0].destination_path, b"incoming").unwrap();
    let proof = resolve(&resolver, &title, 0).await.unwrap();
    assert!(proof.permits_source_removal() && proof.hashes.is_some());
    assert_eq!(
        proof.depth,
        AppliedVerificationDepth::exact(VerificationDepth::Full)
    );
    let row = store
        .file_resolutions("op", "title")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.disposition, ResolutionDisposition::Identical);
    assert!(row.identity_hash.is_some());
    std::fs::remove_file(&title.files[0].source_path).unwrap();
    assert!(
        row.proof_is_current().await,
        "cleanup before a checkpoint is recoverable"
    );
    std::fs::write(&title.files[0].destination_path, b"modified").unwrap();
    assert!(!row.proof_is_current().await);
}

/// Every occupied candidate name is quick-checked in turn (FR-073): the
/// original holds different bytes of the same length (the samples disagree)
/// and the first rename candidate holds a different length, so both are
/// proven different without a full read and the incoming copy lands on the
/// numbered name after them.
#[tokio::test]
async fn occupied_candidate_names_are_quick_checked_without_a_full_read() {
    let temp = tempfile::tempdir().unwrap();
    let (_, resolver, title) = fixture(&temp);
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::write(&title.files[0].destination_path, b"original").unwrap();
    let candidate = title.files[0]
        .destination_path
        .with_file_name("season (from AnimeTesting).nfo");
    std::fs::write(&candidate, b"different candidate").unwrap();
    let (progress, _, full_comparison) = counting_progress();
    let proof = resolver
        .resolve(
            &RootMoveFileMover::without_permissions(),
            FileMoveRequest {
                operation_id: "op",
                title: &title,
                file: &title.files[0],
                depth: VerificationDepth::Full,
                progress: &progress,
            },
        )
        .await
        .unwrap();
    assert!(
        !full_comparison.load(std::sync::atomic::Ordering::Relaxed),
        "the quick check settles both occupied names; neither earns a full comparison"
    );
    assert_eq!(
        proof.destination_path.file_name().unwrap(),
        "season (from AnimeTesting) (2).nfo"
    );
    assert!(proof.permits_source_removal());
    assert!(proof.detail.is_some());
    assert_eq!(std::fs::read(&proof.destination_path).unwrap(), b"incoming");
    assert_eq!(std::fs::read(&candidate).unwrap(), b"different candidate");
    assert_eq!(
        std::fs::read(&title.files[0].destination_path).unwrap(),
        b"original"
    );
    assert!(
        title.files[0].source_path.exists(),
        "placement never cleans the source"
    );
}

/// A size difference is proof enough: neither file is read at all.
#[tokio::test]
async fn a_size_difference_is_proven_different_without_reading_either_file() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"original bytes").unwrap();
    let (progress, _, full_comparison) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, None)
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Mismatch);
    assert!(
        proof.hashes.is_none(),
        "nothing was verified against anything"
    );
    assert!(!full_comparison.load(std::sync::atomic::Ordering::Relaxed));
}

/// Same length, different bytes inside the sampled head: the sample settles it
/// and the full comparison never starts.
#[tokio::test]
async fn a_differing_sample_is_proven_different_without_the_full_comparison() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"original").unwrap();
    let (progress, _, full_comparison) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, None)
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Mismatch);
    assert!(proof.hashes.is_none());
    assert!(!full_comparison.load(std::sync::atomic::Ordering::Relaxed));
}

/// A pair the quick check cannot tell apart is read end to end on both sides
/// before it is called identical; the proof carries the full BLAKE3.
#[tokio::test]
async fn a_quick_identical_pair_earns_the_full_comparison_of_both_sides() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"incoming").unwrap();
    let (progress, reads, full_comparison) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, None)
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Verified);
    assert!(full_comparison.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::Relaxed),
        16,
        "both sides are read in full"
    );
    let hashes = proof.hashes.expect("the full comparison's hashes");
    assert_eq!(
        hashes.full_blake3,
        blake3::hash(b"incoming").to_hex().to_string()
    );
    assert!(source.exists() && destination.exists());
}

/// The catalog's attested hash and signature for a file, as the plan carries
/// them.
fn known_content_of(path: &Path) -> KnownSourceContent {
    let metadata = std::fs::metadata(path).unwrap();
    let signature =
        crate::file_source_signature::file_source_signature_from_metadata(&metadata).unwrap();
    KnownSourceContent {
        full_blake3: blake3::hash(&std::fs::read(path).unwrap())
            .to_hex()
            .to_string(),
        size_bytes: metadata.len(),
        signature_scheme: signature.scheme,
        signature_value: signature.value,
    }
}

/// The source's bytes were hashed end to end when the catalog recorded them.
/// While its signature still matches, that hash stands in for a second read:
/// only the destination is read, and the proof carries the full BLAKE3.
#[tokio::test]
async fn a_stored_source_hash_with_a_matching_signature_reads_only_the_destination() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"incoming").unwrap();
    let known = known_content_of(&source);
    let (progress, reads, full_comparison) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, Some(&known))
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Verified);
    assert!(proof.permits_source_removal());
    assert!(full_comparison.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::Relaxed),
        8,
        "only the destination is read"
    );
    let hashes = proof.hashes.expect("the destination's hashes");
    assert_eq!(hashes.full_blake3, known.full_blake3);
    assert!(proof.detail.is_none(), "{:?}", proof.detail);
    assert!(source.exists() && destination.exists());
}

/// A source touched since the catalog hashed it is not the file the stored
/// hash describes: the stored hash is set aside and both sides are read.
#[tokio::test]
async fn a_stored_source_hash_with_a_stale_signature_falls_back_to_reading_both_sides() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"incoming").unwrap();
    let mut known = known_content_of(&source);
    known.signature_value = format!("{}0", known.signature_value);
    let (progress, reads, _) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, Some(&known))
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Verified);
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::Relaxed),
        16,
        "both sides are read in full"
    );
}

/// A stored hash that no longer matches the bytes, with a signature that still
/// does, calls an identical pair different: the file is renamed in rather than
/// merged, and nothing is removed on the strength of a stale record.
#[tokio::test]
async fn a_stored_source_hash_that_disagrees_with_the_destination_is_a_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&destination, b"incoming").unwrap();
    let mut known = known_content_of(&source);
    known.full_blake3 = blake3::hash(b"something else").to_hex().to_string();
    let (progress, reads, _) = counting_progress();
    let proof = compare_existing(&source, &destination, &progress, Some(&known))
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Mismatch);
    assert!(!proof.permits_source_removal());
    assert!(proof.hashes.is_none());
    assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 8);
    assert!(source.exists() && destination.exists());
}

/// The stored hash covers the whole source, so a destination that differs
/// outside the sampled head and tail is still caught — after reading only the
/// destination.
#[tokio::test]
async fn a_stored_source_hash_detects_differences_outside_the_quick_check_samples() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    let mut bytes = vec![17; 12_000_000];
    std::fs::write(&source, &bytes).unwrap();
    bytes[4_000_000] = 19;
    std::fs::write(&destination, bytes).unwrap();
    let known = known_content_of(&source);
    let reads = Arc::new(AtomicU64::new(0));
    let count = reads.clone();
    let progress = CopyProgress::none().with_comparison_sink(move |bytes, total| {
        assert_eq!(total, 12_000_000);
        count.fetch_max(bytes, Ordering::Relaxed);
    });
    let proof = compare_existing(&source, &destination, &progress, Some(&known))
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Mismatch);
    assert_eq!(reads.load(Ordering::Relaxed), 12_000_000);
    assert!(source.exists() && destination.exists());
}

/// The quick check samples a bounded head and tail; a difference in the
/// middle is invisible to it, and only the full comparison finds it — after
/// reading both files completely.
#[tokio::test]
async fn full_identity_detects_differences_outside_the_quick_check_samples_and_counts_both_reads() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("a");
    let destination = temp.path().join("b");
    let mut bytes = vec![17; 12_000_000];
    std::fs::write(&source, &bytes).unwrap();
    bytes[4_000_000] = 19;
    std::fs::write(&destination, bytes).unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let count = reads.clone();
    let progress = CopyProgress::none().with_comparison_sink(move |bytes, total| {
        assert_eq!(total, 24_000_000);
        count.fetch_max(bytes, Ordering::Relaxed);
    });
    let proof = compare_existing(&source, &destination, &progress, None)
        .await
        .unwrap();
    assert_eq!(proof.outcome, FileVerificationOutcome::Mismatch);
    assert_eq!(reads.load(Ordering::Relaxed), 24_000_000);
    assert!(source.exists() && destination.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn valid_hardlink_resolution_never_reads_content_and_survives_interrupted_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let (store, resolver, title) = fixture(&temp);
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::hard_link(
        &title.files[0].source_path,
        &title.files[0].destination_path,
    )
    .unwrap();
    std::fs::set_permissions(
        &title.files[0].source_path,
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let progress = CopyProgress::none()
        .with_comparison_sink(|_, _| panic!("hardlink attempted content comparison"));
    let proof = resolver
        .resolve(
            &RootMoveFileMover::without_permissions(),
            FileMoveRequest {
                operation_id: "op",
                title: &title,
                file: &title.files[0],
                depth: VerificationDepth::Full,
                progress: &progress,
            },
        )
        .await
        .unwrap();
    assert!(proof.hashes.is_none());
    let row = store
        .file_resolutions("op", "title")
        .await
        .unwrap()
        .remove(0);
    std::fs::remove_file(&title.files[0].source_path).unwrap();
    assert!(row.proof_is_current().await);
    std::fs::set_permissions(
        &title.files[0].destination_path,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}

#[tokio::test]
async fn companions_and_nested_trickplay_follow_the_resolved_media_stem() {
    let temp = tempfile::tempdir().unwrap();
    let (_, resolver, mut title) = fixture(&temp);
    title.files[0].source_path.set_file_name("episode.mkv");
    title.files[0].destination_path.set_file_name("episode.mkv");
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::write(&title.files[0].destination_path, b"original").unwrap();
    for name in ["episode.en.srt", "episode.trickplay/320/0.jpg"] {
        let source = temp.path().join("source").join(name);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"sidecar").unwrap();
        title.files.push(PlannedFile {
            media_file_id: None,
            source_path: source,
            destination_path: temp.path().join("destination").join(name),
            size_bytes: 7,
            source_content: None,
        });
    }
    resolve(&resolver, &title, 0).await.unwrap();
    let subtitle = resolve(&resolver, &title, 1).await.unwrap();
    let trickplay = resolve(&resolver, &title, 2).await.unwrap();
    assert!(
        subtitle
            .destination_path
            .ends_with("episode (from AnimeTesting).en.srt")
    );
    assert!(
        trickplay
            .destination_path
            .ends_with("episode (from AnimeTesting).trickplay/320/0.jpg")
    );
}

#[tokio::test]
async fn a_companion_uses_the_longest_matching_media_stem() {
    let temp = tempfile::tempdir().unwrap();
    let (_, resolver, mut title) = fixture(&temp);
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    title.files[0].source_path = source.join("episode.mkv");
    title.files[0].destination_path = destination.join("episode.mkv");
    for (name, media) in [
        ("episode.part1.mkv", Some("part1")),
        ("episode.part1.en.srt", None),
    ] {
        std::fs::write(source.join(name), b"incoming").unwrap();
        title.files.push(PlannedFile {
            media_file_id: media.map(str::to_string),
            source_path: source.join(name),
            destination_path: destination.join(name),
            size_bytes: 8,
            source_content: None,
        });
    }
    std::fs::write(destination.join("episode.part1.mkv"), b"original").unwrap();
    resolve(&resolver, &title, 1).await.unwrap();
    // The shorter prefix has not run: waiting for it would stall this sibling.
    let companion = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        resolve(&resolver, &title, 2),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        companion
            .destination_path
            .ends_with("episode.part1 (from AnimeTesting).en.srt")
    );
}

#[tokio::test]
async fn changing_files_never_produce_a_completed_identity_proof() {
    let temp = tempfile::tempdir().unwrap();
    let (_, resolver, title) = fixture(&temp);
    std::fs::write(&title.files[0].source_path, b"incoming").unwrap();
    std::fs::write(&title.files[0].destination_path, b"incoming").unwrap();
    let path = title.files[0].destination_path.clone();
    let progress = CopyProgress::none().with_comparison_sink(move |bytes, _| {
        if bytes == 0 {
            std::fs::write(&path, b"changing data").unwrap();
        }
    });
    let result = resolver
        .resolve(
            &RootMoveFileMover::without_permissions(),
            FileMoveRequest {
                operation_id: "op",
                title: &title,
                file: &title.files[0],
                depth: VerificationDepth::Full,
                progress: &progress,
            },
        )
        .await;
    assert!(result.is_err());
    assert!(title.files[0].source_path.exists());
    assert!(
        resolver
            .store
            .file_resolutions("op", "title")
            .await
            .unwrap()
            .iter()
            .all(|row| !row.completed)
    );
}
