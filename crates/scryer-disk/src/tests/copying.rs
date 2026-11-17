//! The copy the disk writes and proves: the write loop's claim, the turns on
//! a volume, and a recorded copy on synthetic files in a temporary directory.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use super::*;
use crate::copy::{Opening, Writing, write_copy};
use crate::place::Job;

fn disk() -> Disk {
    Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)))
}

fn recorded(depth: Depth, landing: Landing, replace: Replace) -> Placement {
    Placement {
        method: Method::Copy,
        fallback: Fallback::None,
        replace,
        verify: Verify::Recorded(Evidence { depth, landing }),
        parents: Parents::MustExist,
    }
}

fn claiming(watch: &Watch) -> Writing<'_> {
    Writing {
        opening: Opening::Claim,
        hash: true,
        sync_folder: false,
        watch,
    }
}

fn payload() -> Vec<u8> {
    (0..(3 * 1024 * 1024u32 + 7))
        .map(|index| (index % 251) as u8)
        .collect()
}

/// Whether a turn in `lane` is free right now, asked without waiting.
fn turn_is_free(admission: &CopyAdmission, lane: &str) -> bool {
    let turn = std::pin::pin!(admission.acquire_keyed(lane));
    matches!(
        turn.poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(_)
    )
}

/// A name that was taken before the exclusive create was never this
/// attempt's, so the failure reports it unclaimed and it is left in place.
#[test]
fn a_claim_lost_to_an_existing_file_is_not_this_copys_file() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mkv");
    let partial = temp.path().join("dest.mkv.partial");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&partial, b"someone else's").unwrap();
    let watch = Watch::nobody();
    let Err(failure) = write_copy(&disk(), &source, &partial, claiming(&watch)) else {
        panic!("the exclusive create fails");
    };
    assert!(!failure.claimed);
    assert_eq!(std::fs::read(&partial).unwrap(), b"someone else's");
}

/// A failure after this attempt created the file reports it claimed.
#[cfg(unix)]
#[test]
fn a_failure_after_the_claim_is_this_copys_file() {
    let temp = tempfile::tempdir().unwrap();
    // Opening a directory succeeds on unix; reading it fails after the claim.
    let source = temp.path().join("source-dir");
    std::fs::create_dir(&source).unwrap();
    let partial = temp.path().join("dest.mkv.partial");
    let watch = Watch::nobody();
    let Err(failure) = write_copy(&disk(), &source, &partial, claiming(&watch)) else {
        panic!("reading a directory fails");
    };
    assert!(failure.claimed);
}

/// Copies to one volume share its turns, and another volume has its own.
#[tokio::test]
async fn copies_share_a_volume_budget() {
    let admission = CopyAdmission::new(2);
    let a = admission.acquire_keyed("a").await;
    let b = admission.acquire_keyed("a").await;
    assert!(!turn_is_free(&admission, "a"));
    let other = admission.acquire_keyed("b").await;
    drop(a);
    let next = admission.acquire_keyed("a").await;
    drop((b, next, other));
    assert!(turn_is_free(&admission, "a"));

    // Asked for by path, two names in one folder are one volume.
    let temp = tempfile::tempdir().unwrap();
    let one = CopyAdmission::new(1);
    let held = one.acquire_for(&temp.path().join("first.mkv")).await;
    let lane = crate::copy::volume_key(&temp.path().join("second.mkv"));
    assert!(!turn_is_free(&one, &lane));
    drop(held);
    assert!(turn_is_free(&one, &lane));

    assert_eq!(CopyAdmission::new(0).capacity(), 1);
    assert_eq!(CopyAdmission::new(100).capacity(), 8);
}

/// A recorded copy lands through its staging name, reports the hashes of
/// what it streamed and proves the destination against them.
#[test]
fn a_recorded_copy_reports_what_it_streamed_and_proved() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    let bytes = payload();
    std::fs::write(&source, &bytes).unwrap();
    let mut whole = ContentHasher::new();
    whole.update(&bytes);

    let policy = recorded(Depth::Full, Landing::Staged, Replace::Never);
    let placed = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect("copied");
    assert_eq!(placed.done, Done::Copied);
    let copied = placed.copied.expect("a recorded copy reports");
    assert_eq!(copied.hashes, whole.finalize());
    assert_eq!(copied.hashes.crc_kind, CrcKind::Crc64Nvme);
    let proof = copied.proof.expect("proven");
    assert_eq!(
        (proof.asked, proof.verdict),
        (Depth::Full, Verdict::Verified)
    );
    assert_eq!(std::fs::read(&dest).unwrap(), bytes);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert!(!resumable_staging_path(&dest).exists());

    // Unproven by request: the hashes are still reported.
    let second = temp.path().join("second.mkv");
    let policy = recorded(Depth::Unproven, Landing::Staged, Replace::Never);
    let placed = disk()
        .place_watched_blocking(&source, &second, policy, &Watch::nobody())
        .expect("copied");
    assert!(placed.copied.expect("reported").proof.is_none());
}

/// A staging name that is occupied is refused, not cleared.
#[test]
fn a_recorded_copy_refuses_an_occupied_staging_name() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    let staged = resumable_staging_path(&dest);
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&staged, b"someone else's").unwrap();
    let policy = recorded(Depth::Quick, Landing::Staged, Replace::Never);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect_err("refused");
    assert_eq!(refusal.refused, Refused::StagingOccupied);
    assert_eq!(refusal.refused.reason(), reason::DISK_COPY_STAGING_OCCUPIED);
    assert_eq!(std::fs::read(&staged).unwrap(), b"someone else's");
    assert!(!dest.exists());
}

/// A destination taken while the copy ran is not replaced: the staged copy
/// this call made is dropped and the newcomer stays.
#[test]
fn a_recorded_copy_never_replaces_a_destination_it_does_not_hold() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&dest, b"already here").unwrap();
    let policy = recorded(Depth::Quick, Landing::Staged, Replace::Never);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect_err("refused");
    assert_eq!(refusal.refused, Refused::DestinationExists);
    assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
    assert!(!resumable_staging_path(&dest).exists());
}

/// A cancelled copy removes the staging file this call created and nothing
/// else; one written straight onto a name the caller holds removes nothing.
#[test]
fn a_cancelled_copy_removes_only_the_staging_file_it_created() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let cancelled = || Watch {
        progress: Progress::none(),
        cancel: Cancel::when(|| true),
    };

    let policy = recorded(Depth::Quick, Landing::Staged, Replace::Never);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &cancelled())
        .expect_err("cancelled");
    assert_eq!(refusal.refused, Refused::Cancelled);
    assert_eq!(refusal.refused.reason(), reason::DISK_COPY_CANCELLED);
    assert_eq!(std::fs::read(&source).unwrap(), b"incoming");
    assert!(!dest.exists());
    assert!(!resumable_staging_path(&dest).exists());

    std::fs::write(&dest, b"").unwrap();
    let policy = recorded(Depth::Quick, Landing::Direct, Replace::Allowed);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &cancelled())
        .expect_err("cancelled");
    assert_eq!(refusal.refused, Refused::Cancelled);
    assert!(dest.exists(), "the held name is the caller's to release");
    assert_eq!(std::fs::read(&source).unwrap(), b"incoming");
}

/// A copy written straight onto a held name needs the name to be there.
#[test]
fn a_direct_copy_writes_only_onto_a_name_the_caller_holds() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let policy = recorded(Depth::Quick, Landing::Direct, Replace::Allowed);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect_err("nothing is held");
    assert_eq!(refusal.refused, Refused::DestinationMissing);
    assert!(!dest.exists());

    std::fs::write(&dest, b"").unwrap();
    let placed = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect("copied");
    let proof = placed.copied.expect("reported").proof.expect("proven");
    assert_eq!(proof.verdict, Verdict::Verified);
    assert_eq!(std::fs::read(&dest).unwrap(), b"incoming");
}

/// The turn on the volume is given back once the copy has landed, before it
/// is proven, and progress says which phase the copy is in.
#[tokio::test]
async fn the_turn_is_given_back_before_the_copy_is_proven() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let admission = CopyAdmission::new(1);
    let permit = admission.acquire_keyed("volume").await;
    let (held_while_copying, free_while_proving) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let watch = Watch {
        progress: Progress::to({
            let (admission, copying, proving) = (
                admission.clone(),
                held_while_copying.clone(),
                free_while_proving.clone(),
            );
            move |phase, _| match phase {
                Phase::Copying => {
                    copying.store(!turn_is_free(&admission, "volume"), Ordering::SeqCst)
                }
                Phase::Verifying => {
                    proving.store(turn_is_free(&admission, "volume"), Ordering::SeqCst)
                }
                Phase::Waiting => {}
            }
        }),
        cancel: Cancel::never(),
    };
    let disk = disk();
    let job = Job {
        disk: &disk,
        source: &source,
        dest: &dest,
        policy: recorded(Depth::Full, Landing::Staged, Replace::Never),
        watch: &watch,
        permit: std::cell::RefCell::new(Some(permit)),
        staged_free: Default::default(),
        copied: Default::default(),
    };
    job.recorded_copy(Evidence {
        depth: Depth::Full,
        landing: Landing::Staged,
    })
    .map_err(|fail| fail.refused)
    .expect("copied");
    assert!(held_while_copying.load(Ordering::SeqCst));
    assert!(free_while_proving.load(Ordering::SeqCst));
}

/// A recorded copy started on a blocking thread takes its volume's turn
/// itself, holds it while it writes and gives it back before it proves.
#[test]
fn a_blocking_recorded_copy_takes_its_own_turn() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let disk = disk();
    disk.install_copy_workers(1);
    let lane = crate::copy::volume_key(&dest);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let watch = Watch {
        progress: Progress::to({
            let (admission, seen) = (disk.copy_admission(), seen.clone());
            move |phase, _| {
                let state = (phase, turn_is_free(&admission, &lane));
                let mut seen = seen.lock().unwrap();
                if seen.last() != Some(&state) {
                    seen.push(state);
                }
            }
        }),
        cancel: Cancel::never(),
    };
    disk.place_watched_blocking(
        &source,
        &dest,
        recorded(Depth::Full, Landing::Staged, Replace::Never),
        &watch,
    )
    .expect("copied");
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (Phase::Waiting, true),
            (Phase::Copying, false),
            (Phase::Verifying, true)
        ]
    );
}

/// The async form waits for the turn before it takes a thread and hands it
/// to the copy, which does not ask for a second: with one turn on the
/// volume the copy still runs to its end.
#[tokio::test]
async fn an_admitted_copy_does_not_ask_for_a_second_turn() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let disk = disk();
    disk.install_copy_workers(1);
    let waits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = Watch {
        progress: Progress::to({
            let waits = waits.clone();
            move |phase, _| {
                if phase == Phase::Waiting {
                    waits.fetch_add(1, Ordering::SeqCst);
                }
            }
        }),
        cancel: Cancel::never(),
    };
    let policy = recorded(Depth::Quick, Landing::Staged, Replace::Never);
    let placed = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        disk.place_watched(&source, &dest, policy, watch),
    )
    .await
    .expect("a copy holding its turn does not wait for another");
    assert_eq!(placed.expect("copied").done, Done::Copied);
    assert_eq!(waits.load(Ordering::SeqCst), 1);
}

/// A move never takes the recorded copy: it is refused before a byte moves.
#[test]
fn a_move_that_would_need_a_recorded_copy_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let disk = disk();
    let watch = Watch::nobody();
    let job = Job {
        disk: &disk,
        source: &source,
        dest: &dest,
        policy: Placement {
            method: Method::Move,
            fallback: Fallback::Copy,
            ..recorded(Depth::Full, Landing::Staged, Replace::Never)
        },
        watch: &watch,
        permit: Default::default(),
        staged_free: Default::default(),
        copied: Default::default(),
    };
    let crossing = DiskError::new(
        DiskVerb::Move,
        &source,
        io::ErrorKind::CrossesDevices.into(),
    );
    let refused = job.across_volumes(crossing).map_err(|fail| fail.refused);
    assert_eq!(refused, Err(Refused::NeedsCopy));
    assert!(!dest.exists());
    assert_eq!(std::fs::read(&source).unwrap(), b"incoming");
}
