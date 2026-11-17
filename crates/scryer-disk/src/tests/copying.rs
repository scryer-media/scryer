//! The copy the disk writes and proves: the write loop's claim, the turns on
//! a volume, and a recorded copy on synthetic files in a temporary directory.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

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
        carry_permissions: true,
        chunk_bytes: 1024 * 1024,
        seen: None,
        phases: false,
        mark_target: false,
        watch,
    }
}

fn payload() -> Vec<u8> {
    (0..(3 * 1024 * 1024u32 + 7))
        .map(|index| (index % 251) as u8)
        .collect()
}

/// How many turns of a lane are out, without waiting for one.
pub(super) fn turns_held(admission: &CopyAdmission, lane: &str) -> usize {
    let mut taken = Vec::new();
    loop {
        let turn = std::pin::pin!(admission.acquire_keyed(lane));
        match turn.poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(permit) => taken.push(permit),
            Poll::Pending => return admission.capacity() - taken.len(),
        }
    }
}

/// Whether a turn in `lane` is free right now, asked without waiting.
pub(super) fn turn_is_free(admission: &CopyAdmission, lane: &str) -> bool {
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
    assert!(turn_is_free(&admission, "b"), "another volume is not held");
    drop(a);
    let next = admission.acquire_keyed("a").await;
    drop((b, next));
    assert!(turn_is_free(&admission, "a"));

    // A mark of a folder names the lane a path in it is given, and so does
    // the mark read back from its stored volume.
    let folder = tempfile::tempdir().unwrap();
    let mark = disk()
        .landing_volume_blocking(folder.path())
        .expect("folder");
    let stored = crate::Mark::from_stored(&mark.volume().to_stored()).expect("stored volume");
    assert_eq!(mark.lane(), disk().copy_lane(&folder.path().join("a.mkv")));
    assert_eq!(stored.lane(), mark.lane());
    let (c, d) = (
        admission.acquire_on(&mark).await,
        admission.acquire_keyed(&mark.lane()).await,
    );
    assert!(!turn_is_free(&admission, &stored.lane()));
    drop((c, d));

    // Asked for by path, two names in one folder are one volume.
    let temp = tempfile::tempdir().unwrap();
    let one = CopyAdmission::new(1);
    let held = one
        .acquire_keyed(&disk().copy_lane(&temp.path().join("first.mkv")))
        .await;
    let lane = disk().copy_lane(&temp.path().join("second.mkv"));
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

/// A held name that is a link is refused, so a copy never writes through it
/// into, or empties, the file it points at.
#[cfg(unix)]
#[test]
fn a_copy_never_writes_through_a_link_at_a_held_name() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    let elsewhere = temp.path().join("elsewhere.mkv");
    std::fs::write(&source, b"incoming").unwrap();
    std::fs::write(&elsewhere, b"someone else's file").unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dest).unwrap();

    let policy = recorded(Depth::Quick, Landing::Direct, Replace::Allowed);
    let refusal = disk()
        .place_watched_blocking(&source, &dest, policy, &Watch::nobody())
        .expect_err("a link is not a held file");
    assert_eq!(refusal.refused.reason(), reason::DISK_LINK_NOT_FOLLOWED);
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"someone else's file");
    assert!(
        std::fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"incoming");
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
                Phase::Waiting | Phase::Finishing => {}
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
        prove: None,
        unproven: Default::default(),
        left: Default::default(),
        source_read: Default::default(),
        folder_is_dir: false,
        wrote_dest: Default::default(),
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
    let lane = disk.copy_lane(&dest);
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
/// to the copy, which does not ask for a second. The volume has two turns,
/// so a second ask would be granted and counted rather than wait forever.
#[tokio::test]
async fn an_admitted_copy_does_not_ask_for_a_second_turn() {
    let temp = tempfile::tempdir().unwrap();
    let (source, dest) = (temp.path().join("source.mkv"), temp.path().join("dest.mkv"));
    std::fs::write(&source, b"incoming").unwrap();
    let disk = disk();
    disk.install_copy_workers(2);
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
    let placing = disk.place_watched(&source, &dest, policy, watch);
    let placed = super::place::bounded(placing).await;
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
        prove: None,
        unproven: Default::default(),
        left: Default::default(),
        source_read: Default::default(),
        folder_is_dir: false,
        wrote_dest: Default::default(),
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

// ── A copy staged under the caller's name ────────────────────────────────────

const PAUSES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// The raw code of a file handle the system dropped.
const DROPPED: i32 = 9;

fn staged_copy<'a>(staged: &'a Path, seen: &'a Mark) -> StagedCopy<'a> {
    StagedCopy {
        staged,
        seen,
        sync_folder: false,
        carry_permissions: false,
        bad_handle_pauses: &PAUSES,
        chunk_bytes: 1024 * 1024,
    }
}

/// A watch that keeps every report it is given.
fn recording() -> (Watch, Arc<Mutex<Vec<(Phase, u64)>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let watch = Watch {
        progress: Progress::to(move |phase, bytes| sink.lock().unwrap().push((phase, bytes))),
        cancel: Cancel::never(),
    };
    (watch, seen)
}

/// A synthetic source, the names a staged copy of it uses, and a disk whose
/// clock the test reads.
struct StagedFixture {
    _temp: tempfile::TempDir,
    disk: Disk,
    clock: Arc<ManualClock>,
    source: PathBuf,
    staged: PathBuf,
    dest: PathBuf,
    seen: Mark,
}

impl StagedFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, payload()).unwrap();
        let clock = ManualClock::at(at(0));
        Self {
            disk: Disk::with_seams(Arc::new(SystemBackend), clock.clone()),
            clock,
            seen: Mark::of(&std::fs::symlink_metadata(&source).unwrap()),
            staged: temp.path().join("sample-dest.staged"),
            dest: temp.path().join("sample-dest.mkv"),
            source,
            _temp: temp,
        }
    }

    fn copy(&self, watch: &Watch) -> Result<StagedCopied, StagedCopyRefusal> {
        let how = staged_copy(&self.staged, &self.seen);
        self.disk
            .copy_staged_blocking(&self.source, &self.dest, &how, watch)
    }

    fn fails_at(&self, faults: &[(CopyFault, i32)]) {
        self.disk.inner.copy_faults.lock().unwrap().extend(faults);
    }

    fn paused(&self) -> chrono::Duration {
        self.clock.now() - at(0)
    }

    fn left_nothing(&self) -> bool {
        !self.dest.exists() && !self.staged.exists()
    }
}

/// The caller's staged name is cleared and reused, the copy takes the
/// destination name, and the source's permissions are not carried. The start
/// is reported once, and the end of the stream after its last chunk.
#[test]
fn a_staged_copy_clears_its_own_name_and_lands_whole() {
    let fixture = StagedFixture::new();
    std::fs::write(&fixture.staged, b"left by an earlier attempt, and longer").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let read_only = std::fs::Permissions::from_mode(0o444);
        std::fs::set_permissions(&fixture.source, read_only).unwrap();
    }
    let (watch, reports) = recording();

    let copied = fixture.copy(&watch).expect("the copy lands");

    let mut expected = ContentHasher::new();
    expected.update(&payload());
    assert_eq!(copied.hashes, expected.finalize());
    assert_eq!(std::fs::read(&fixture.dest).unwrap(), payload());
    assert!(!fixture.staged.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&fixture.dest)
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o200,
            0,
            "the copy is not read-only as its source is"
        );
    }
    let whole = payload().len() as u64;
    let reports = reports.lock().unwrap();
    assert_eq!(reports.first(), Some(&(Phase::Copying, 0)));
    let end = [(Phase::Copying, whole), (Phase::Finishing, whole)];
    assert!(reports.ends_with(&end), "{reports:?}");
    let starts = reports.iter().filter(|report| report.1 == 0).count();
    assert_eq!(starts, 1, "{reports:?}");
    assert_eq!(fixture.paused(), chrono::Duration::zero());
}

/// A landed staged copy names the file it promoted, read once through the
/// staged file's own handle and never by path: that file is the one under
/// the destination name, and a file put there afterwards is not.
#[cfg(unix)]
#[test]
fn a_staged_copy_names_the_file_it_promoted() {
    let fixture = StagedFixture::new();
    let lstats_before = fixture.disk.calls("lstat");

    let copied = fixture.copy(&Watch::nobody()).expect("the copy lands");

    assert_eq!(fixture.disk.calls("handle_stat"), 1);
    assert_eq!(
        fixture.disk.calls("lstat") - lstats_before,
        2,
        "the source before and after, as before"
    );
    let promoted = copied.promoted.expect("the handle answered");
    let landed = Mark::of(&std::fs::symlink_metadata(&fixture.dest).unwrap());
    assert_eq!(promoted.same_file(&landed), Sameness::Same);

    let other = fixture.dest.with_file_name("sample-other.mkv");
    std::fs::write(&other, payload()).unwrap();
    std::fs::rename(&other, &fixture.dest).unwrap();
    let replaced = Mark::of(&std::fs::symlink_metadata(&fixture.dest).unwrap());
    assert_eq!(promoted.same_file(&replaced), Sameness::Different);
}

/// Nothing is said to have started until the staged file is open.
#[test]
fn a_staged_copy_that_cannot_open_its_staged_file_reports_no_start() {
    let mut fixture = StagedFixture::new();
    fixture.staged = fixture
        .source
        .with_file_name("no-such-folder/sample.staged");
    let (watch, reports) = recording();

    let refusal = fixture
        .copy(&watch)
        .expect_err("the staged file cannot be made");

    assert_eq!(refusal.step, StagedStep::OpenStaged);
    assert_eq!(*reports.lock().unwrap(), []);
}

/// A write, and then a sync, that fails on a dropped file handle is tried
/// again after the next pause, which passes on the disk's clock, and each
/// partial file is cleared.
#[test]
fn a_staged_copy_tries_again_after_a_dropped_handle() {
    let fixture = StagedFixture::new();
    fixture.fails_at(&[(CopyFault::Write, DROPPED), (CopyFault::Sync, DROPPED)]);

    fixture
        .copy(&Watch::nobody())
        .expect("the third attempt lands");

    assert_eq!(std::fs::read(&fixture.dest).unwrap(), payload());
    assert!(!fixture.staged.exists());
    assert_eq!(fixture.paused(), chrono::Duration::seconds(4));
}

/// Three attempts, a second and then three apart, and no more. Nothing is
/// left at either name and the source is as it was.
#[test]
fn a_staged_copy_gives_up_after_its_pauses() {
    let fixture = StagedFixture::new();
    fixture.fails_at(&[(CopyFault::Write, DROPPED); 4]);

    let refusal = fixture
        .copy(&Watch::nobody())
        .expect_err("no attempt lands");

    assert_eq!((refusal.step, refusal.attempts), (StagedStep::Transfer, 3));
    assert!(
        matches!(refusal.cause, StagedCause::Io(error) if error.raw_os_error() == Some(DROPPED))
    );
    assert_eq!(fixture.paused(), chrono::Duration::seconds(4));
    assert!(fixture.left_nothing());
    assert_eq!(std::fs::read(&fixture.source).unwrap(), payload());
}

/// Any other failure is not tried again, and neither is a failure to read
/// the source's name, whatever code it carries.
#[test]
fn a_staged_copy_does_not_retry_another_failure() {
    for (fault, step) in [
        ((CopyFault::Write, 5), StagedStep::Transfer),
        ((CopyFault::SourceName, DROPPED), StagedStep::SourceBefore),
    ] {
        let fixture = StagedFixture::new();
        fixture.fails_at(&[fault]);

        let refusal = fixture.copy(&Watch::nobody()).expect_err("the copy fails");

        assert_eq!((refusal.step, refusal.attempts), (step, 1), "{fault:?}");
        assert_eq!(fixture.paused(), chrono::Duration::zero(), "{fault:?}");
        assert!(fixture.left_nothing(), "{fault:?}");
    }
}

/// A source that is no longer the file the caller read stops the copy before
/// anything is written, and one that stops being it is not given the name.
#[test]
fn a_staged_copy_stops_when_its_source_is_not_the_file_read() {
    let fixture = StagedFixture::new();
    std::fs::write(&fixture.source, b"rewritten, and shorter").unwrap();

    let refusal = fixture
        .copy(&Watch::nobody())
        .expect_err("the source changed");

    assert_eq!(
        (refusal.step, refusal.attempts),
        (StagedStep::SourceBefore, 1)
    );
    assert!(matches!(refusal.cause, StagedCause::SourceChanged));
    assert!(fixture.left_nothing());

    // Rewritten while its last chunk is reported: caught by name afterwards.
    let fixture = StagedFixture::new();
    let (source, whole) = (fixture.source.clone(), payload().len() as u64);
    let watch = Watch {
        progress: Progress::to(move |phase, bytes| {
            if (phase, bytes) == (Phase::Copying, whole) {
                std::fs::write(&source, b"rewritten under the copy").unwrap();
            }
        }),
        cancel: Cancel::never(),
    };
    let refusal = fixture.copy(&watch).expect_err("the source changed");
    assert_eq!(refusal.step, StagedStep::SourceAfter);
    assert!(matches!(refusal.cause, StagedCause::SourceChanged));
    assert!(fixture.left_nothing());
}

/// A new owner alone makes the source another file to this copy: nothing is
/// opened.
#[test]
fn a_staged_copy_stops_when_only_the_owner_of_its_source_changed() {
    const SOURCE: &str = "/srv/fixture-media/incoming/sample.mkv";
    let owned_by = |owner| {
        let mut stat = file(17);
        stat.extra.owner = Some(owner);
        stat
    };
    let (disk, backend) = owner(&ManualClock::at(at(0)));
    backend.stat_is(SOURCE, Ok(owned_by(502)));
    let seen = Mark::of_stat(&owned_by(501));
    let staged = p("/srv/fixture-media/Fixture Show/sample.staged");
    let dest = p("/srv/fixture-media/Fixture Show/sample.mkv");

    let refusal = disk
        .copy_staged_blocking(
            &p(SOURCE),
            &dest,
            &staged_copy(&staged, &seen),
            &Watch::nobody(),
        )
        .expect_err("the owner changed");

    assert_eq!(
        (refusal.step, refusal.attempts),
        (StagedStep::SourceBefore, 1)
    );
    assert!(matches!(refusal.cause, StagedCause::SourceChanged));
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("stat"), 0, "a file is not followed");
}

/// A source whose name reads as the file the caller saw, but which opens as
/// something that is no regular file, stops the copy at once: no second
/// attempt, nothing promoted, and the staged name cleared again.
#[cfg(unix)]
#[test]
fn a_staged_copy_stops_when_its_opened_source_is_no_regular_file() {
    const SOURCE: &str = "/srv/fixture-media/incoming/sample.mkv";
    let (disk, backend) = owner(&ManualClock::at(at(0)));
    backend.stat_is(SOURCE, Ok(file(17)));
    *backend.opens_as_folder.lock().unwrap() = Some(p(SOURCE));
    let seen = Mark::of_stat(&file(17));
    let staged = p("/srv/fixture-media/Fixture Show/sample.staged");
    let dest = p("/srv/fixture-media/Fixture Show/sample.mkv");

    let refusal = disk
        .copy_staged_blocking(
            &p(SOURCE),
            &dest,
            &staged_copy(&staged, &seen),
            &Watch::nobody(),
        )
        .expect_err("the source opened as a folder");

    assert_eq!(
        (refusal.step, refusal.attempts),
        (StagedStep::SourceHandle, 1)
    );
    assert!(matches!(refusal.cause, StagedCause::SourceNotAFile));
    // Cleared before the attempt and after it; nothing else was removed.
    assert_eq!(backend.paths_of("remove_file"), [staged.clone(), staged]);
    // Only the source was opened: no staged file was written.
    assert_eq!(backend.paths_of("open"), [p(SOURCE)]);
    assert_eq!(backend.calls_of("rename_claimed"), 0);
    assert_eq!(backend.calls_of("hard_link"), 0);
}

/// A source handed to another owner while it is copied is not given the
/// destination name: the staged file is removed and nothing lands.
#[test]
fn a_staged_copy_stops_when_its_source_is_reowned_under_it() {
    let fixture = StagedFixture::new();
    let (disk, whole) = (fixture.disk.clone(), payload().len() as u64);
    let watch = Watch {
        progress: Progress::to(move |phase, bytes| {
            if (phase, bytes) == (Phase::Copying, whole) {
                let reowned = (CopyFault::SourceOwner, 64_001);
                disk.inner.copy_faults.lock().unwrap().push_back(reowned);
            }
        }),
        cancel: Cancel::never(),
    };

    let refusal = fixture.copy(&watch).expect_err("the owner changed");

    assert_eq!(
        (refusal.step, refusal.attempts),
        (StagedStep::SourceAfter, 1)
    );
    assert!(matches!(refusal.cause, StagedCause::SourceChanged));
    assert!(fixture.left_nothing());
    assert_eq!(std::fs::read(&fixture.source).unwrap(), payload());
}

/// A link found where the source was is a source that changed, wherever it
/// leads, and it is not followed: what it is now is the caller's to say.
#[cfg(unix)]
#[test]
fn a_staged_copy_stops_at_a_link_where_its_source_was() {
    let fixture = StagedFixture::new();
    std::fs::remove_file(&fixture.source).unwrap();
    let gone = fixture.source.with_file_name("sample-gone.mkv");
    std::os::unix::fs::symlink(&gone, &fixture.source).unwrap();

    for leads_somewhere in [false, true] {
        if leads_somewhere {
            std::fs::write(&gone, payload()).unwrap();
        }
        let refusal = fixture
            .copy(&Watch::nobody())
            .expect_err("the source is a link");
        assert_eq!(
            (refusal.step, refusal.attempts),
            (StagedStep::SourceBefore, 1)
        );
        assert!(matches!(refusal.cause, StagedCause::SourceChanged));
        assert!(fixture.left_nothing());
    }
}

/// A cancelled copy says so and leaves nothing staged.
#[test]
fn a_cancelled_staged_copy_leaves_nothing_staged() {
    let fixture = StagedFixture::new();
    let copying = Arc::new(AtomicBool::new(false));
    let (started, asked) = (copying.clone(), copying);
    let watch = Watch {
        progress: Progress::to(move |_, bytes| started.store(bytes > 0, Ordering::SeqCst)),
        cancel: Cancel::when(move || asked.load(Ordering::SeqCst)),
    };

    let refusal = fixture.copy(&watch).expect_err("the copy is cancelled");

    assert!(matches!(refusal.cause, StagedCause::Cancelled));
    assert_eq!(refusal.attempts, 1);
    assert!(fixture.left_nothing());
}

/// A claimed move whose rename fails beside a source that does not read as
/// present leaves its reservation, and the failure says where, in a typed
/// field.
#[test]
fn a_claimed_move_that_fails_says_what_it_left_at_the_reserved_name() {
    let temp = tempfile::tempdir().unwrap();
    let (missing, dest) = (
        temp.path().join("missing.mkv"),
        temp.path().join("dest.mkv"),
    );
    let error = disk()
        .move_claimed_blocking(&missing, &dest)
        .expect_err("nothing to move");
    assert_eq!(
        error.left(),
        Some(&Leftover::AtReservedName { at: dest.clone() })
    );
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        b"",
        "the reservation is kept"
    );

    let folder = temp.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let free = temp.path().join("free.mkv");
    let error = disk()
        .move_claimed_blocking(&folder, &free)
        .expect_err("a folder does not go onto a file");
    assert_eq!(error.left(), None, "the reservation is released");
    assert!(!free.exists());
}
