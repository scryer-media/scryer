//! Placing a file: each outcome and refusal against the scripted backend,
//! then the same verb on synthetic files in a temporary directory.

use super::*;
use crate::place::{Job, StagedHolds, paths_are_same_file};

const SOURCE: &str = "/srv/fixture-media/incoming/sample.mkv";
const DEST: &str = "/srv/fixture-media/Fixture Show/sample.mkv";
const STAGED: &str = "/srv/fixture-media/Fixture Show/.scryer-0a1b2c3d4.partial";

fn policy(method: Method, fallback: Fallback) -> Placement {
    Placement {
        method,
        fallback,
        replace: Replace::Never,
        verify: Verify::Quick,
        parents: Parents::MustExist,
    }
}

fn scripted() -> (Disk, Arc<Scripted>, Arc<ManualClock>) {
    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    (disk, backend, clock)
}

fn place(disk: &Disk, policy: Placement) -> Result<Placed, PlaceRefusal> {
    disk.place_blocking(&p(SOURCE), &p(DEST), policy)
}

fn refusal(disk: &Disk, policy: Placement) -> Refused {
    let refusal = place(disk, policy).expect_err("the placement is refused");
    assert_eq!(refusal.asked, policy.method);
    refusal.refused
}

/// A source that holds `bytes`, and a staged copy that reads back as `copy`.
fn copies(backend: &Scripted, bytes: &[u8], copy: &[u8]) {
    backend.stat_is(SOURCE, Ok(file(bytes.len() as u64)));
    backend.holds(SOURCE, bytes);
    *backend.staged_contents.lock().unwrap() = Some(copy.to_vec());
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(copy.len() as u64)));
}

/// Every removal was of a staging name this placement chose.
fn removed_only_staging(backend: &Scripted) -> usize {
    let removed = backend.paths_of("remove_file");
    assert!(
        removed.iter().all(|path| is_staging_name(path)),
        "{removed:?}"
    );
    removed.len()
}

fn job<'a>(disk: &'a Disk, source: &'a Path, dest: &'a Path, policy: Placement) -> Job<'a> {
    Job {
        disk,
        source,
        dest,
        policy,
        watch: Box::leak(Box::new(Watch::nobody())),
        permit: Default::default(),
        staged_free: Default::default(),
        copied: Default::default(),
        prove: None,
        unproven: Default::default(),
        left: Default::default(),
        source_read: Default::default(),
        folder_is_dir: false,
        wrote_dest: Default::default(),
    }
}

fn recorded_staged(method: Method, fallback: Fallback) -> Placement {
    Placement {
        verify: Verify::Recorded(Evidence {
            depth: Depth::Full,
            landing: Landing::Staged,
        }),
        ..policy(method, fallback)
    }
}

#[test]
fn a_move_renames_and_says_so() {
    let (disk, backend, _) = scripted();
    let placed = place(&disk, policy(Method::Move, Fallback::Copy)).expect("placed");
    assert_eq!(placed.asked, Method::Move);
    assert_eq!(placed.done, Done::Renamed);
    assert!(!placed.fell_back());
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("lstat"), 0, "a rename probes nothing");
    assert_eq!(backend.calls_of("create_dir_all"), 0);
}

#[test]
fn only_a_policy_that_creates_folders_creates_them_and_never_for_a_link() {
    let (disk, backend, _) = scripted();
    let mut creating = policy(Method::Move, Fallback::None);
    creating.parents = Parents::Create;
    place(&disk, creating).expect("placed");
    assert_eq!(
        backend.paths_of("create_dir_all"),
        [p("/srv/fixture-media/Fixture Show")]
    );
    creating.method = Method::HardLink;
    place(&disk, creating).expect("linked");
    assert_eq!(backend.calls_of("create_dir_all"), 1);
}

#[test]
fn an_occupied_destination_is_refused_and_nothing_is_touched() {
    for method in [Method::Move, Method::HardLink] {
        let (disk, backend, _) = scripted();
        backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
        backend.fails("hard_link", DEST, io::ErrorKind::AlreadyExists);
        let refused = place(&disk, policy(method, Fallback::Copy)).expect_err("refused");
        assert_eq!(refused.refused, Refused::DestinationExists);
        assert_eq!(refused.refused.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(backend.calls_of("open"), 0, "no copy is tried instead");
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
}

#[test]
fn a_move_to_another_volume_copies_proves_and_only_then_removes_the_source() {
    let (disk, backend, _) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
    copies(&backend, b"synthetic payload", b"synthetic payload");

    let placed = place(&disk, policy(Method::Move, Fallback::Copy)).expect("placed");
    assert_eq!(placed.done, Done::CopiedThenRemovedSource);
    assert!(placed.fell_back());
    assert_eq!(backend.paths_of("remove_file"), [p(SOURCE)]);
    // The copy went under a staging name and reached the real one by a
    // rename that replaces nothing.
    let promoted = backend.paths_of("rename_no_replace");
    assert_eq!(promoted.len(), 2);
    assert!(is_staging_name(&promoted[1]));
    assert_eq!(backend.paths_of("lstat"), [p(SOURCE)], "one probe");
}

#[test]
fn a_move_to_another_volume_is_refused_when_no_copy_is_allowed() {
    let (disk, backend, _) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
    let refused = refusal(&disk, policy(Method::Move, Fallback::None));
    assert_eq!(refused, Refused::NeedsCopy);
    assert_eq!(refused.reason(), reason::DISK_MOVE_NEEDS_COPY);
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("remove_file"), 0);
    assert_eq!(backend.calls_of("lstat"), 0);
}

/// Where the filesystem has no exclusive rename the move reserves a name
/// beside the destination; a move that then needs a copy it may not make
/// releases only that empty reservation.
#[test]
fn a_reserved_name_is_released_when_the_move_is_refused() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
    backend.stat_is(SOURCE, Ok(file(9)));
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(0)));
    assert_eq!(
        refusal(&disk, policy(Method::Move, Fallback::None)),
        Refused::NeedsCopy
    );
    assert_eq!(removed_only_staging(&backend), 1);
    assert_eq!(backend.paths_of("lstat").last(), Some(&p(SOURCE)));
    assert_eq!(
        backend.calls_of("lstat"),
        2,
        "the reserved name, then the source"
    );
}

/// A symlinked source that a rename onto the reserved name cannot move to
/// another volume is made again there, as a move with an exclusive rename
/// makes it: the reservation is released first, the link's target is never
/// opened or copied, and the source link goes. A link that cannot be made
/// releases the reservation too and keeps the source.
#[test]
fn a_symlinked_source_is_linked_again_on_the_reserved_name_path_too() {
    let symlink = RawStat {
        kind: EntryKind::Symlink,
        ..file(9)
    };
    for made in [true, false] {
        let (disk, backend, _) = scripted();
        backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
        backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
        backend.stat_is(SOURCE, Ok(symlink));
        if !made {
            backend.fails("symlink", DEST, io::ErrorKind::PermissionDenied);
        }
        let placed = place(&disk, policy(Method::Move, Fallback::Copy));
        let removed = backend.paths_of("remove_file");
        assert!(is_staging_name(&removed[0]), "{removed:?}");
        match made {
            true => {
                assert_eq!(
                    placed.expect("relinked").done,
                    Done::RelinkedThenRemovedSource
                );
                assert_eq!(removed[1..], [p(SOURCE)]);
            }
            false => {
                assert_eq!(
                    placed.expect_err("refused").refused,
                    Refused::Denied(Side::Destination)
                );
                assert_eq!(removed.len(), 1, "the source stays: {removed:?}");
            }
        }
        assert_eq!(backend.paths_of("symlink"), [p(DEST)]);
        // Only the reservation was opened; the link's target was not copied.
        let opened = backend.paths_of("open");
        assert_eq!(opened.len(), 1, "{opened:?}");
        assert!(is_staging_name(&opened[0]));
        assert_eq!(
            backend.paths_of("lstat"),
            [p(SOURCE)],
            "one look, at the source"
        );
    }
}

/// An empty source can be renamed onto the reserved name by a rename whose
/// reply is lost. The empty file there is then the only copy, so it is not
/// released unless the source still reads as present. A source that reads
/// as gone gets it back under its name, by a link that replaces nothing,
/// after which only the reserved name is unlinked; a source that cannot be
/// read leaves it where it is. The refusal says which.
#[test]
fn an_empty_file_at_the_reserved_name_is_kept_when_the_source_is_not_there() {
    for (source, put_back) in [
        (None, true),
        (Some(Err(io::ErrorKind::PermissionDenied)), false),
    ] {
        let (disk, backend, _) = scripted();
        backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
        // The rename landed and its reply did not come back.
        backend.fails("rename", SOURCE, io::ErrorKind::TimedOut);
        *backend.staged_stat.lock().unwrap() = Some(Ok(file(0)));
        if let Some(stat) = source {
            backend.stat_is(SOURCE, stat);
        }
        let refused = place(&disk, policy(Method::Move, Fallback::Copy)).expect_err("refused");
        assert_eq!(refused.refused, Refused::Unreachable(Side::Either));
        assert_eq!(backend.calls_of("lstat"), 2, "one look at each name");
        let said = refused.to_string();
        assert!(said.contains(SOURCE), "{said}");
        let linked = backend.paths_of("hard_link");
        let removed = backend.paths_of("remove_file");
        if put_back {
            assert!(said.contains("was put back"), "{said}");
            // The link is made at the source's name; only then is the
            // reserved name unlinked.
            assert_eq!(linked, [p(SOURCE)]);
            assert_eq!(removed.len(), 1, "{removed:?}");
            assert!(is_staging_name(&removed[0]), "{removed:?}");
        } else {
            assert!(said.contains("is left in place"), "{said}");
            assert!(linked.is_empty() && removed.is_empty(), "{source:?}");
        }
    }
}

/// A move never takes the recorded copy, whichever rename found that it
/// needs one. Here it is the rename onto the reserved name, on a filesystem
/// with no exclusive rename: nothing is copied and the source stays.
#[test]
fn a_recorded_move_is_refused_on_the_reserved_name_path_too() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
    copies(&backend, b"synthetic payload", b"synthetic payload");
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(0)));
    let refused = place(&disk, recorded_staged(Method::Move, Fallback::Copy)).expect_err("refused");
    assert_eq!(refused.refused, Refused::NeedsCopy);
    // Only the reservation was opened, and only it was removed.
    let opened = backend.paths_of("open");
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert!(is_staging_name(&opened[0]));
    assert_eq!(removed_only_staging(&backend), 1);
    assert_eq!(backend.calls_of("hard_link"), 0);
}

/// The plain copy, with no hashes and nobody watching: the bytes go under a
/// staging name and never under the real one, and a destination that was
/// taken meanwhile is not replaced and does not cost the source.
#[test]
fn a_plain_cross_device_copy_never_shows_an_unproven_file_or_replaces_a_newcomer() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
    backend.fails("hard_link", DEST, io::ErrorKind::AlreadyExists);
    copies(&backend, b"synthetic payload", b"synthetic payload");
    let mut unproven = policy(Method::Move, Fallback::Copy);
    unproven.verify = Verify::None;
    assert_eq!(refusal(&disk, unproven), Refused::DestinationExists);
    let opened = backend.paths_of("open");
    assert!(
        opened
            .iter()
            .all(|path| *path == p(SOURCE) || is_staging_name(path)),
        "{opened:?}"
    );
    // The one rename is the refused move of the source: the staged copy was
    // never renamed over the name the newcomer holds.
    assert_eq!(backend.paths_of("rename"), [p(SOURCE)]);
    assert_eq!(removed_only_staging(&backend), 1);
}

#[test]
fn a_move_without_an_exclusive_rename_goes_through_a_reserved_name() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    let placed = place(&disk, policy(Method::Move, Fallback::Copy)).expect("placed");
    assert_eq!(placed.done, Done::Renamed);
    assert_eq!(backend.paths_of("rename"), [p(SOURCE)]);
    // Linked onto the real name, which refuses an occupant in the same call.
    assert_eq!(backend.paths_of("hard_link"), [p(DEST)]);
    assert_eq!(removed_only_staging(&backend), 1);
}

/// The source was renamed to the reserved name and the destination was then
/// taken: the file goes back to its own name and is never removed.
#[test]
fn a_file_stranded_at_its_reserved_name_is_put_back() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("hard_link", DEST, io::ErrorKind::AlreadyExists);
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(17)));
    assert_eq!(
        refusal(&disk, policy(Method::Move, Fallback::Copy)),
        Refused::DestinationExists
    );
    assert_eq!(backend.paths_of("hard_link"), [p(DEST), p(SOURCE)]);
    // The one removal is of the staging link, after the source name holds
    // the file again.
    assert_eq!(removed_only_staging(&backend), 1);
}

/// A destination that appears while the copy is written wins: the copy is
/// taken back and the source stays.
#[test]
fn a_copy_that_finds_the_destination_taken_is_removed_and_the_source_kept() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
    backend.fails("hard_link", DEST, io::ErrorKind::AlreadyExists);
    copies(&backend, b"synthetic payload", b"synthetic payload");
    assert_eq!(
        refusal(&disk, policy(Method::Move, Fallback::Copy)),
        Refused::DestinationExists
    );
    assert_eq!(removed_only_staging(&backend), 1);
}

/// A copy that does not match is looked at once more after a pause, which
/// the manual clock passes at once; still wrong, it is removed and the
/// source kept.
#[test]
fn a_copy_that_does_not_match_is_refused_after_one_more_look() {
    for method in [Method::Move, Method::Copy, Method::HardLink] {
        let (disk, backend, clock) = scripted();
        backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
        backend.fails("hard_link", DEST, io::ErrorKind::CrossesDevices);
        copies(&backend, b"synthetic payload", b"synthetic pay");
        let refused = refusal(&disk, policy(method, Fallback::Copy));
        assert_eq!(refused, Refused::NotVerified);
        assert_eq!(refused.reason(), reason::DISK_COPY_NOT_VERIFIED);
        assert_eq!(clock.now(), at(3), "one pause before the second look");
        assert_eq!(removed_only_staging(&backend), 1);
    }
}

/// A move that falls back to a copy removes its source next, so the copy is
/// proven even for a caller that proves copies itself.
#[test]
fn a_move_that_copies_proves_the_copy_whatever_the_policy_says() {
    for replace in [Replace::Never, Replace::Allowed] {
        let (disk, backend, _) = scripted();
        backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
        backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
        copies(&backend, b"synthetic payload", b"something else");
        let mut policy = policy(Method::Move, Fallback::Copy);
        policy.verify = Verify::None;
        policy.replace = replace;
        assert_eq!(refusal(&disk, policy), Refused::NotVerified);
        assert!(
            !backend.paths_of("remove_file").contains(&p(SOURCE)),
            "the source is kept"
        );
    }
}

/// A caller that proves the copy itself is still not trusted with a copy the
/// filesystem could not sync.
#[test]
fn an_unsynced_copy_is_proven_whatever_the_policy_says() {
    let run = |unconfirmed: bool| {
        let (disk, backend, _) = scripted();
        if unconfirmed {
            backend.always_fails("unconfirmed_sync", io::ErrorKind::Other);
        }
        copies(&backend, b"synthetic payload", b"something else");
        let mut policy = policy(Method::Copy, Fallback::None);
        policy.verify = Verify::None;
        place(&disk, policy).map(|placed| placed.done)
    };
    assert_eq!(run(false).expect("unproven by request"), Done::Copied);
    assert_eq!(
        run(true).expect_err("proven anyway").refused,
        Refused::NotVerified
    );
}

#[test]
fn a_copy_keeps_its_source_and_replaces_only_when_told() {
    for replace in [Replace::Never, Replace::Allowed] {
        let (disk, backend, _) = scripted();
        copies(&backend, b"synthetic payload", b"synthetic payload");
        let mut policy = policy(Method::Copy, Fallback::None);
        policy.replace = replace;
        let placed = place(&disk, policy).expect("copied");
        assert_eq!(placed.done, Done::Copied);
        assert!(!placed.fell_back());
        assert_eq!(backend.calls_of("remove_file"), 0);
        let replacing = backend.calls_of("rename");
        assert_eq!(replacing, usize::from(replace == Replace::Allowed));
        assert_eq!(backend.calls_of("rename_no_replace"), 1 - replacing);
    }
}

#[test]
fn a_hard_link_is_made_or_refused_with_the_reason_it_cannot_be() {
    let (disk, backend, _) = scripted();
    let placed = place(&disk, policy(Method::HardLink, Fallback::None)).expect("linked");
    assert_eq!(placed.done, Done::HardLinked);
    assert!(!placed.fell_back());
    assert_eq!(backend.paths_of("hard_link"), [p(DEST)]);

    let cases = [
        (
            io::ErrorKind::CrossesDevices,
            CannotLink::DifferentVolume,
            reason::DISK_LINK_DIFFERENT_VOLUME,
        ),
        (
            io::ErrorKind::Unsupported,
            CannotLink::NotSupported,
            reason::DISK_LINK_NOT_SUPPORTED,
        ),
    ];
    for (kind, cannot, code) in cases {
        let (disk, backend, _) = scripted();
        backend.fails("hard_link", DEST, kind);
        let refused = refusal(&disk, policy(Method::HardLink, Fallback::None));
        assert_eq!(refused, Refused::CannotLink(cannot));
        assert_eq!(refused.reason(), code);
        assert_eq!(backend.calls_of("open"), 0, "the fallback was refused");
    }
}

#[test]
fn a_link_that_cannot_be_made_is_copied_when_the_policy_allows() {
    for kind in [io::ErrorKind::CrossesDevices, io::ErrorKind::Other] {
        let (disk, backend, _) = scripted();
        backend.fails("hard_link", DEST, kind);
        copies(&backend, b"synthetic payload", b"synthetic payload");
        let placed = place(&disk, policy(Method::HardLink, Fallback::Copy)).expect("copied");
        assert_eq!(
            (placed.asked, placed.done),
            (Method::HardLink, Done::Copied)
        );
        assert!(placed.fell_back());
        assert_eq!(backend.calls_of("remove_file"), 0, "the source is kept");
    }
}

/// A link that falls back to a recorded copy waits for its volume's turn
/// like any other recorded copy, once.
#[test]
fn a_link_that_falls_back_to_a_recorded_copy_takes_a_turn() {
    let (disk, backend, _) = scripted();
    backend.fails("hard_link", DEST, io::ErrorKind::CrossesDevices);
    copies(&backend, b"synthetic payload", b"synthetic payload");
    // Two workers, so a second turn would be counted instead of parking.
    disk.install_copy_workers(2);
    let lane = disk.copy_lane(&p(DEST));
    let held = Arc::new(Mutex::new(None));
    let waits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let phases = Arc::new(Mutex::new(Vec::new()));
    let watch = Watch {
        progress: Progress::to({
            let (phases, held, disk) = (phases.clone(), held.clone(), disk.clone());
            let waits = waits.clone();
            move |phase, _| {
                if phase == Phase::Waiting {
                    waits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if phase == Phase::Copying {
                    let turns = super::copying::turns_held(&disk.copy_admission(), &lane);
                    held.lock().unwrap().get_or_insert(turns);
                }
                let mut phases = phases.lock().unwrap();
                if phases.last() != Some(&phase) {
                    phases.push(phase);
                }
            }
        }),
        cancel: Cancel::never(),
    };
    let policy = Placement {
        verify: Verify::Recorded(Evidence {
            depth: Depth::Unproven,
            landing: Landing::Direct,
        }),
        ..policy(Method::HardLink, Fallback::Copy)
    };
    let placed = disk
        .place_watched_blocking(&p(SOURCE), &p(DEST), policy, &watch)
        .expect("copied");
    assert_eq!(placed.done, Done::Copied);
    assert!(placed.copied.is_some());
    assert_eq!(*phases.lock().unwrap(), [Phase::Waiting, Phase::Copying]);
    assert_eq!(waits.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        *held.lock().unwrap(),
        Some(1),
        "one turn is held while copying"
    );
}

/// A link refused for a reason that says something about the source, the
/// access or the space is not answered by copying.
#[test]
fn a_link_refused_for_a_named_reason_is_not_copied() {
    let cases = [
        (io::ErrorKind::NotFound, Refused::SourceMissing),
        (
            io::ErrorKind::PermissionDenied,
            Refused::Denied(Side::Either),
        ),
        (io::ErrorKind::StorageFull, Refused::NoSpace),
        (
            io::ErrorKind::NotConnected,
            Refused::Unreachable(Side::Either),
        ),
    ];
    for (kind, expected) in cases {
        let (disk, backend, _) = scripted();
        backend.fails("hard_link", DEST, kind);
        copies(&backend, b"synthetic payload", b"synthetic payload");
        assert_eq!(
            refusal(&disk, policy(Method::HardLink, Fallback::Copy)),
            expected
        );
        assert_eq!(backend.calls_of("open"), 0, "nothing was copied");
    }
}

#[test]
fn failures_are_classified_once_into_typed_refusals() {
    let moved = |kind: io::ErrorKind| {
        let (disk, backend, _) = scripted();
        backend.fails("rename_no_replace", SOURCE, kind);
        refusal(&disk, policy(Method::Move, Fallback::Copy))
    };
    assert_eq!(moved(io::ErrorKind::NotFound), Refused::SourceMissing);
    assert_eq!(
        moved(io::ErrorKind::PermissionDenied),
        Refused::Denied(Side::Either)
    );
    assert_eq!(moved(io::ErrorKind::StorageFull), Refused::NoSpace);
    assert_eq!(
        moved(io::ErrorKind::NotConnected),
        Refused::Unreachable(Side::Either)
    );
    assert_eq!(moved(io::ErrorKind::InvalidInput), Refused::Failed);

    // The reservation of the copy's name is the destination's answer.
    let claimed = |kind: io::ErrorKind| {
        let (disk, backend, _) = scripted();
        backend.always_fails("open", kind);
        refusal(&disk, policy(Method::Copy, Fallback::None))
    };
    assert_eq!(claimed(io::ErrorKind::StorageFull), Refused::NoSpace);
    // A reservation with no folder to go in names the destination, where a
    // rename, which names both paths at once, cannot.
    assert_eq!(
        claimed(io::ErrorKind::NotFound),
        Refused::DestinationMissing
    );
    assert_eq!(
        claimed(io::ErrorKind::NotConnected),
        Refused::Unreachable(Side::Destination)
    );

    // A source that cannot be read is not a source that is gone.
    let copied_from = |stat: Option<io::ErrorKind>| {
        let (disk, backend, _) = scripted();
        backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
        if let Some(kind) = stat {
            backend.stat_is(SOURCE, Err(kind));
        }
        let refused = refusal(&disk, policy(Method::Move, Fallback::Copy));
        assert_eq!(backend.calls_of("open"), 0);
        refused
    };
    assert_eq!(copied_from(None), Refused::SourceMissing);
    assert_eq!(
        copied_from(Some(io::ErrorKind::NotConnected)),
        Refused::Unreachable(Side::Source)
    );
    // A lasting fault is not unreachable storage.
    assert_eq!(
        copied_from(Some(io::ErrorKind::PermissionDenied)),
        Refused::Denied(Side::Source)
    );
    assert_eq!(
        copied_from(Some(io::ErrorKind::NotADirectory)),
        Refused::Failed
    );
}

#[test]
fn a_refusal_reads_as_the_systems_own_words() {
    let (disk, backend, _) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let refused = place(&disk, policy(Method::Move, Fallback::Copy)).expect_err("refused");
    assert_eq!(
        refused.to_string(),
        io::Error::from(io::ErrorKind::AlreadyExists).to_string()
    );
}

/// What a failed move does with the name it reserved, by what the name and
/// the source read as. `put_back` is the non-replacing promotion back to the
/// source's name.
#[test]
fn a_rollback_removes_only_a_copy_or_an_empty_claim_and_puts_the_only_copy_back() {
    type Stat = Option<Result<RawStat, io::ErrorKind>>;
    let unread: Stat = Some(Err(io::ErrorKind::PermissionDenied));
    let (claim, data, present): (Stat, Stat, Stat) =
        (Some(Ok(file(0))), Some(Ok(file(9))), Some(Ok(file(9))));
    let unproven = StagedHolds::Copy { proven: false };
    let proven = StagedHolds::Copy { proven: true };
    // (held, staged reads, source reads) -> (removed, put back)
    let cases: [(StagedHolds, Stat, Stat, bool, bool); 16] = [
        // An empty file may be an empty source: it goes only while the
        // source reads as present, goes back while the source reads as gone,
        // and stays while the source cannot be read.
        (StagedHolds::EmptyClaim, claim, present, true, false),
        (StagedHolds::EmptyClaim, claim, None, false, true),
        (StagedHolds::EmptyClaim, claim, unread, false, false),
        (StagedHolds::EmptyClaim, None, None, false, false),
        // An unread claim goes back only when the source reads as gone.
        (StagedHolds::EmptyClaim, unread, None, false, true),
        (StagedHolds::EmptyClaim, unread, present, false, false),
        (StagedHolds::EmptyClaim, unread, unread, false, false),
        // A claim that holds data may be the source itself.
        (StagedHolds::EmptyClaim, data, present, false, true),
        (unproven, data, present, true, false),
        (unproven, data, None, false, false),
        (unproven, data, unread, false, false),
        (proven, data, present, true, false),
        (proven, data, None, false, true),
        (proven, data, unread, false, false),
        (StagedHolds::OnlyCopy, data, present, false, true),
        (StagedHolds::OnlyCopy, unread, present, false, true),
    ];
    for (index, (held, staged, source, removed, put_back)) in cases.into_iter().enumerate() {
        let (disk, backend, _) = scripted();
        if let Some(stat) = staged {
            backend.stat_is(STAGED, stat);
        }
        if let Some(stat) = source {
            backend.stat_is(SOURCE, stat);
        }
        let (source, dest) = (p(SOURCE), p(DEST));
        let left = job(&disk, &source, &dest, policy(Method::Move, Fallback::Copy))
            .roll_back(&p(STAGED), held);
        // Only an empty file that was put back or kept adds to the refusal.
        assert_eq!(left.is_some(), matches!(index, 1 | 2), "case {index}");
        if let Some(said) = &left {
            let expected = ["was put back", "is left in place"][index - 1];
            assert!(said.contains(expected), "case {index}: {said}");
        }
        let removals = backend.paths_of("remove_file");
        assert_eq!(
            removals,
            vec![p(STAGED); usize::from(removed)],
            "case {index}"
        );
        let promoted = backend.paths_of("rename_no_replace");
        assert_eq!(
            promoted,
            vec![p(STAGED); usize::from(put_back)],
            "case {index}"
        );
        // Never more than one look at each of the two names.
        assert!(backend.calls_of("lstat") <= 2, "case {index}");
    }
}

/// Once the copy is in place the source is unlinked. When that fails, one
/// look at the source decides.
#[test]
fn a_source_that_cannot_be_unlinked_decides_by_whether_it_is_still_there() {
    let run = |source: Option<Result<RawStat, io::ErrorKind>>| {
        let (disk, backend, _) = scripted();
        backend.fails("remove_file", SOURCE, io::ErrorKind::PermissionDenied);
        if let Some(stat) = source {
            backend.stat_is(SOURCE, stat);
        }
        let (source, dest) = (p(SOURCE), p(DEST));
        let done = job(&disk, &source, &dest, policy(Method::Move, Fallback::Copy))
            .unlink_moved_source(Done::CopiedThenRemovedSource)
            .map_err(|fail| fail.refused);
        (
            done,
            backend.paths_of("remove_file"),
            backend.calls_of("lstat"),
        )
    };
    // Gone: the move finished.
    let (done, removed, looks) = run(None);
    assert_eq!(done, Ok(Done::CopiedThenRemovedSource));
    assert_eq!((removed, looks), (vec![p(SOURCE)], 1));
    // Still there: the new destination is taken back, so a retry is not
    // refused by a duplicate.
    let (done, removed, _) = run(Some(Ok(file(9))));
    assert_eq!(done, Err(Refused::Denied(Side::Source)));
    assert_eq!(removed, [p(SOURCE), p(DEST)]);
    // Unknown: both are left.
    let (done, removed, _) = run(Some(Err(io::ErrorKind::TimedOut)));
    assert_eq!(done, Err(Refused::Denied(Side::Source)));
    assert_eq!(removed, [p(SOURCE)]);
}

/// A source that is already gone when its unlink is tried is what the move
/// wanted: the move finishes, and nothing is looked at or taken back.
#[test]
fn an_unlink_that_finds_the_source_already_gone_finishes_the_move() {
    let (disk, backend, _) = scripted();
    backend.fails("remove_file", SOURCE, io::ErrorKind::NotFound);
    let (source, dest) = (p(SOURCE), p(DEST));
    let done = job(&disk, &source, &dest, policy(Method::Move, Fallback::Copy))
        .unlink_moved_source(Done::CopiedThenRemovedSource)
        .map_err(|fail| fail.refused);
    assert_eq!(done, Ok(Done::CopiedThenRemovedSource));
    assert_eq!(backend.paths_of("remove_file"), [p(SOURCE)]);
    assert_eq!(backend.calls_of("lstat"), 0);
}

/// A name that differs only in case is the same file on a case-insensitive
/// volume: it is renamed through a reserved sibling, and neither hop
/// replaces anything.
#[test]
fn a_case_only_rename_goes_through_an_intermediate_name() {
    let (disk, backend, _) = scripted();
    let (source, dest) = (p(SOURCE), p("/srv/fixture-media/incoming/SAMPLE.mkv"));
    let placed = disk
        .place_blocking(&source, &dest, policy(Method::Move, Fallback::Copy))
        .expect("renamed");
    assert_eq!(placed.done, Done::Renamed);
    assert_eq!(backend.paths_of("rename"), [p(SOURCE)]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    // A rename that reports failure may still have landed: an intermediate
    // name that is not the empty claim is left alone.
    let (disk, backend, _) = scripted();
    backend.fails("rename", SOURCE, io::ErrorKind::TimedOut);
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(9)));
    let refused = disk
        .place_blocking(&source, &dest, policy(Method::Move, Fallback::Copy))
        .expect_err("refused");
    assert_eq!(refused.refused, Refused::Unreachable(Side::Either));
    assert_eq!(backend.calls_of("rename"), 1, "no second attempt");
    assert_eq!(backend.calls_of("remove_file"), 0);
    let intermediate = backend.paths_of("open").pop().expect("claimed");
    assert_eq!(
        refused.left,
        vec![Leftover::AtReservedName { at: intermediate }],
        "the source's only copy may be at the intermediate name"
    );

    // An empty intermediate name is released, and the rename tried again,
    // only while the source still reads as present: it may be an empty
    // source. While the source reads as gone it goes back under the source's
    // name, never over anything; while the source cannot be read it stays.
    let unread = Some(Err(io::ErrorKind::PermissionDenied));
    for (source_stat, attempts, said_of_it) in [
        (Some(Ok(file(0))), 10, None),
        (None, 1, Some("was put back")),
        (unread, 1, Some("is left in place")),
    ] {
        let (disk, backend, _) = scripted();
        backend.fails("rename", SOURCE, io::ErrorKind::TimedOut);
        *backend.staged_stat.lock().unwrap() = Some(Ok(file(0)));
        if let Some(stat) = source_stat {
            backend.stat_is(SOURCE, stat);
        }
        let refused = disk
            .place_blocking(&source, &dest, policy(Method::Move, Fallback::Copy))
            .expect_err("refused");
        assert_eq!(refused.refused, Refused::Unreachable(Side::Either));
        assert_eq!(backend.calls_of("rename"), attempts);
        let released = if said_of_it.is_none() { attempts } else { 0 };
        assert_eq!(removed_only_staging(&backend), released);
        let promoted = backend.paths_of("rename_no_replace");
        assert_eq!(
            promoted.len(),
            usize::from(said_of_it == Some("was put back"))
        );
        let said = refused.to_string();
        for words in ["was put back", "is left in place"] {
            assert_eq!(said.contains(words), said_of_it == Some(words), "{said}");
        }
    }
}

#[tokio::test]
async fn the_async_form_reports_like_the_blocking_one() {
    let (disk, backend, _) = scripted();
    let placed = disk.place(SOURCE, DEST, policy(Method::Move, Fallback::None));
    assert_eq!(bounded(placed).await.expect("placed").done, Done::Renamed);
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let refused = disk.place(SOURCE, DEST, policy(Method::Move, Fallback::None));
    assert_eq!(
        bounded(refused).await.expect_err("refused").refused,
        Refused::DestinationExists
    );
    // Before it takes a thread the async form reads the source's name and
    // the destination's folder, once each for each placement.
    let folder = p("/srv/fixture-media/Fixture Show");
    assert_eq!(backend.paths_of("lstat"), [p(SOURCE), p(SOURCE)]);
    assert_eq!(backend.paths_of("stat"), [folder.clone(), folder]);
    assert!(disk.destination_is_free_for(&p(SOURCE), &p(DEST)).await);
    backend.stat_is(DEST, Ok(file(1)));
    assert!(!disk.destination_is_free_for(&p(SOURCE), &p(DEST)).await);
}

/// A root that does not answer refuses the placement before it takes a
/// thread, for the source's side and for the destination folder's, and
/// nothing is renamed, linked or opened. The refusal is the unreachable one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_placement_whose_root_does_not_answer_is_refused_before_it_runs() {
    let folder = "/srv/fixture-media/Fixture Show";
    let seen = Mark::of_stat(&file(17));
    for (silenced, side) in [(SOURCE, Side::Source), (folder, Side::Destination)] {
        for proven in [false, true] {
            let (disk, backend, _) = scripted();
            backend.stat_is(SOURCE, Ok(file(17)));
            let silent = crate::test_support::silence_path(&disk, &p(silenced)).expect("a root");
            let refusal = match proven {
                false => {
                    bounded(disk.place(SOURCE, DEST, policy(Method::Move, Fallback::Copy))).await
                }
                true => {
                    let linking = policy(Method::HardLink, Fallback::Copy);
                    let proof = LinkProof::NameAndLength;
                    bounded(disk.place_proven(SOURCE, DEST, linking, seen.clone(), proof)).await
                }
            }
            .expect_err("refused");
            drop(silent);
            assert_eq!(refusal.refused, Refused::Unreachable(side), "{silenced}");
            assert_eq!(refusal.refused.reason(), reason::DISK_PATH_UNREACHABLE);
            assert!(!refusal.destination_written);
            for call in [
                "rename_no_replace",
                "rename",
                "hard_link",
                "open",
                "remove_file",
            ] {
                assert_eq!(backend.calls_of(call), 0, "{silenced}: {call}");
            }
        }
    }
}

/// The two looks taken before the thread stand for the job's own: a folder
/// that read as a folder is not created again, one that did not is, and a
/// move a rename cannot make decides what its source is from the first look.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_looks_before_the_thread_stand_for_the_jobs_own() {
    let folder = "/srv/fixture-media/Fixture Show";
    let mut creating = policy(Method::Move, Fallback::Copy);
    creating.parents = Parents::Create;
    for folder_there in [true, false] {
        let (disk, backend, _) = scripted();
        if folder_there {
            backend.stat_is(folder, Ok(dir()));
        }
        bounded(disk.place(SOURCE, DEST, creating))
            .await
            .expect("placed");
        let created = backend.paths_of("create_dir_all");
        assert_eq!(created.len(), usize::from(!folder_there), "{created:?}");
    }

    // A symlinked source on a volume with no exclusive rename: the source
    // is looked at once in all, before the thread.
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.fails("rename", SOURCE, io::ErrorKind::CrossesDevices);
    let symlink = RawStat {
        kind: EntryKind::Symlink,
        ..file(9)
    };
    backend.stat_is(SOURCE, Ok(symlink));
    let placed = bounded(disk.place(SOURCE, DEST, policy(Method::Move, Fallback::Copy))).await;
    assert_eq!(
        placed.expect("relinked").done,
        Done::RelinkedThenRemovedSource
    );
    assert_eq!(backend.paths_of("lstat"), [p(SOURCE)]);
}

/// Whether a refused copy changed what is under the destination name: only
/// a copy that lands directly and got as far as emptying or creating the
/// file there says it did.
#[test]
fn a_refusal_says_whether_the_destination_name_was_written() {
    let landing = |landing, replace| Placement {
        replace,
        verify: Verify::Recorded(Evidence {
            depth: Depth::Unproven,
            landing,
        }),
        ..policy(Method::Copy, Fallback::None)
    };
    let held = landing(Landing::Direct, Replace::Allowed);
    let claimed = landing(Landing::Direct, Replace::Never);
    let staged = landing(Landing::Staged, Replace::Never);
    // (policy, the source opens, the destination opens, a write fails, written)
    let cases = [
        (held, false, true, false, false),
        (held, true, false, false, false),
        (held, true, true, true, true),
        (claimed, true, false, false, false),
        (claimed, true, true, true, true),
        (staged, false, true, false, false),
        (staged, true, true, true, false),
    ];
    for (index, (policy, source_opens, dest_opens, write_fails, written)) in
        cases.into_iter().enumerate()
    {
        let (disk, backend, _) = scripted();
        disk.install_copy_workers(1);
        if source_opens {
            backend.holds(SOURCE, b"synthetic payload");
        }
        if !dest_opens {
            backend.fails("open", DEST, io::ErrorKind::PermissionDenied);
        }
        if write_fails {
            let fault = (crate::CopyFault::Write, 5);
            disk.inner.copy_faults.lock().unwrap().push_back(fault);
        }
        let refusal = place(&disk, policy).expect_err("refused");
        assert_eq!(refusal.destination_written, written, "case {index}");
        assert_eq!(backend.calls_of("rename"), 0, "case {index}");
    }
}

/// The staged name of a recorded copy is read within the root's bound, before
/// a turn on the volume is waited for and before anything is opened. A name
/// that is held is refused from that one reading.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_copys_staged_name_is_read_within_the_bound_before_its_turn() {
    let waited = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = || Watch {
        progress: Progress::to({
            let waited = waited.clone();
            move |_, _| {
                waited.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }),
        cancel: Cancel::never(),
    };
    let staged = resumable_staging_path(&p(DEST));

    let (disk, backend, _) = scripted();
    backend.stat_is(staged.to_str().unwrap(), Ok(file(1)));
    let policy = recorded_staged(Method::Copy, Fallback::None);
    let held = bounded(disk.place_watched(SOURCE, DEST, policy, watch()))
        .await
        .expect_err("refused");
    assert_eq!(held.refused, Refused::StagingOccupied);
    // The source's name, then the staged name: one reading of each.
    assert_eq!(backend.paths_of("lstat"), [p(SOURCE), staged.clone()]);
    assert_eq!(backend.calls_of("open"), 0);

    // A staged name unread for a lasting reason is not unreachable storage.
    for (kind, refused) in [
        (
            io::ErrorKind::PermissionDenied,
            Refused::Denied(Side::Destination),
        ),
        (io::ErrorKind::NotADirectory, Refused::Failed),
        (
            io::ErrorKind::NotConnected,
            Refused::Unreachable(Side::Destination),
        ),
    ] {
        let (disk, backend, _) = scripted();
        backend.stat_is(staged.to_str().unwrap(), Err(kind));
        let policy = recorded_staged(Method::Copy, Fallback::None);
        let unread = bounded(disk.place_watched(SOURCE, DEST, policy, watch())).await;
        assert_eq!(unread.expect_err("refused").refused, refused, "{kind:?}");
        assert_eq!(backend.calls_of("open"), 0);
    }

    let (disk, backend, _) = scripted();
    disk.set_probe_timeout_for_test(std::time::Duration::from_millis(1));
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    *backend.gate_under.lock().unwrap() = Some(staged.clone());
    // The staged name's reading is held in the backend until the gate opens,
    // so the bound always elapses first.
    let silent = bounded(disk.place_watched(SOURCE, DEST, policy, watch()))
        .await
        .expect_err("refused");
    assert_eq!(silent.refused, Refused::Unreachable(Side::Destination));
    assert_eq!(
        silent.to_string(),
        format!(
            "failed to inspect the destination staging path {}: the disk did not answer in time",
            staged.display()
        )
    );
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(
        waited.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no turn was waited for"
    );
    open.send(()).unwrap();
}

/// How long a test waits for something that must happen. Only a regression
/// reaches it.
const BOUND: std::time::Duration = std::time::Duration::from_secs(120);

/// `placing`, failing the test when it has not ended within [`BOUND`].
pub(super) async fn bounded<T>(placing: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, placing)
        .await
        .expect("the placement ends within the bound")
}

#[cfg(unix)]
fn direct_recorded() -> Placement {
    Placement {
        verify: Verify::Recorded(Evidence {
            depth: Depth::Full,
            landing: Landing::Direct,
        }),
        ..policy(Method::Copy, Fallback::None)
    }
}

/// A recorded copy that lands directly is never refused before it runs: its
/// destination can be a name the caller holds and cleans up on a refusal.
/// While its volume does not answer it waits, the read out on a blocking
/// thread and nothing opened, and once the volume answers it takes one turn
/// on that volume's own lane and copies.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_direct_copy_whose_volume_is_slow_to_answer_waits_and_is_not_refused() {
    use super::copying::turns_held;
    use super::mark::numbered;
    let folder = "/srv/fixture-media/Fixture Show";
    let (disk, backend, _) = scripted();
    // Two workers, so a second turn would be counted instead of parking.
    disk.install_copy_workers(2);
    backend.stat_is(folder, Ok(numbered(Some(7), Some(3))));
    copies(&backend, b"synthetic payload", b"synthetic payload");
    // Every bounded read under the root is refused at once, so a copy that
    // asked for one before it ran would be refused whatever the timing.
    let silent = crate::test_support::silence_root(&disk, &p(DEST)).expect("a configured root");
    assert_eq!(disk.uncached(p(DEST)).await.exists(), None);
    let (open, gate) = mpsc::channel::<()>();
    *backend.gate.lock().unwrap() = Some(gate);
    *backend.gate_under.lock().unwrap() = Some(p(folder));
    let (entered, reading) = mpsc::channel();
    *backend.entered.lock().unwrap() = Some(entered);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let watch = Watch {
        progress: Progress::to({
            let (seen, disk) = (seen.clone(), disk.clone());
            move |phase, _| {
                let held = turns_held(&disk.copy_admission(), "unix-dev:7");
                seen.lock().unwrap().push((phase, held));
            }
        }),
        cancel: Cancel::never(),
    };
    let copy = tokio::spawn({
        let disk = disk.clone();
        async move {
            disk.place_watched(SOURCE, DEST, direct_recorded(), watch)
                .await
        }
    });
    // The volume read is out off the runtime, held in the backend. A copy
    // that ends before that read was refused, and fails the test at once.
    let mut copy = copy;
    let read = tokio::task::spawn_blocking(move || reading.recv_timeout(BOUND));
    tokio::select! {
        read = read => read.expect("joined").expect("the volume is read"),
        ended = &mut copy => {
            // Lets the waiting thread go.
            backend.entered.lock().unwrap().take();
            let ended = ended.expect("joined").map(|placed| placed.done);
            panic!("the copy ended before its volume was read: {ended:?}");
        }
    }
    assert_eq!(backend.calls_of("open"), 0, "nothing is written yet");
    drop(open);
    let placed = tokio::time::timeout(BOUND, copy)
        .await
        .expect("the copy ends")
        .expect("joined");
    drop(silent);
    assert_eq!(placed.expect("copied, not refused").done, Done::Copied);
    let seen = seen.lock().unwrap();
    let waits = seen.iter().filter(|(phase, _)| *phase == Phase::Waiting);
    assert_eq!(waits.count(), 1, "one wait: {seen:?}");
    assert!(
        seen.contains(&(Phase::Copying, 1)),
        "one turn, on its own lane: {seen:?}"
    );
    assert!(
        seen.iter().all(|(_, held)| *held <= 1),
        "never a second turn: {seen:?}"
    );
}

/// A recorded copy that lands directly waits for its turn without holding a
/// blocking thread: with one such thread and the volume's only turn taken,
/// other blocking work still runs, and the copy runs once the turn is free.
#[cfg(unix)]
#[test]
fn a_direct_copy_waiting_for_its_turn_holds_no_blocking_thread() {
    use super::mark::numbered;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let (disk, backend, _) = scripted();
        disk.install_copy_workers(1);
        let folder = "/srv/fixture-media/Fixture Show";
        backend.stat_is(folder, Ok(numbered(Some(7), Some(3))));
        copies(&backend, b"synthetic payload", b"synthetic payload");
        let held = disk.copy_admission().acquire_keyed("unix-dev:7").await;
        let (open, gate) = mpsc::channel::<()>();
        *backend.gate.lock().unwrap() = Some(gate);
        *backend.gate_under.lock().unwrap() = Some(p(folder));
        let (entered, reading) = mpsc::channel();
        *backend.entered.lock().unwrap() = Some(entered);
        let copy = tokio::spawn({
            let disk = disk.clone();
            async move {
                disk.place_watched(SOURCE, DEST, direct_recorded(), Watch::nobody())
                    .await
            }
        });
        // This thread is not one of the runtime's workers, so it may block
        // until the copy has started reading.
        reading.recv_timeout(BOUND).expect("the volume is read");
        drop(open);
        // The copy is now asking for the turn this test holds. Work given to
        // the one blocking thread at this point runs only if the copy is not
        // parked on that thread while it asks.
        bounded(disk.copy_admission().asking_is(1)).await;
        let other = tokio::task::spawn_blocking(|| ());
        bounded(other)
            .await
            .expect("the one blocking thread is free");
        assert_eq!(backend.calls_of("open"), 0, "nothing is written yet");
        assert!(!copy.is_finished());
        drop(held);
        let placed = bounded(copy).await.expect("joined");
        assert_eq!(placed.expect("copied").done, Done::Copied);
    });
}

/// A recorded copy that lands under a staged name is refused before it takes
/// a thread or a turn when its destination volume does not answer within the
/// bound. Nothing of its own exists yet for a caller to clean up.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_staged_copy_whose_volume_does_not_answer_is_refused_before_its_turn() {
    use super::copying::turn_is_free;
    use super::mark::numbered;
    let folder = "/srv/fixture-media/Fixture Show";
    let (disk, backend, _) = scripted();
    disk.install_copy_workers(1);
    backend.stat_is(folder, Ok(numbered(Some(7), Some(3))));
    // Only a read of the destination itself is out and unanswered, so the
    // staged name's own reading answers and the volume read, which asks by
    // the destination, is refused at once.
    let silent = crate::test_support::silence_path(&disk, &p(DEST)).expect("a configured root");
    let staged = recorded_staged(Method::Copy, Fallback::None);
    let refusal = bounded(disk.place_watched(SOURCE, DEST, staged, Watch::nobody()))
        .await
        .expect_err("refused");
    drop(silent);
    assert_eq!(refusal.refused, Refused::Unreachable(Side::Destination));
    // The source's name and the staged name were read and answered, and the
    // destination's folder once; the volume was never asked for.
    assert_eq!(
        backend.paths_of("lstat"),
        [p(SOURCE), resumable_staging_path(&p(DEST))]
    );
    assert_eq!(backend.paths_of("stat"), [p(folder)]);
    // Nothing was opened: no thread ran the placement.
    assert_eq!(backend.calls_of("open"), 0);
    let free = |lane| turn_is_free(&disk.copy_admission(), lane);
    assert!(free("unknown-volume") && free("unix-dev:7"));
}

/// The early reading of the staged name is only a fast path: a file that
/// takes the name while the copy waits for its turn is met by the exclusive
/// create, refused in the same words, and left alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_staged_name_taken_after_it_read_free_is_refused_and_kept() {
    let staged = resumable_staging_path(&p(DEST));
    let (disk, backend, _) = scripted();
    backend.stat_is(SOURCE, Ok(file(17)));
    backend.holds(SOURCE, b"synthetic payload");
    backend.fails(
        "open",
        staged.to_str().unwrap(),
        io::ErrorKind::AlreadyExists,
    );
    let policy = recorded_staged(Method::Copy, Fallback::None);
    let refused = bounded(disk.place_watched(SOURCE, DEST, policy, Watch::nobody()))
        .await
        .expect_err("refused");
    assert_eq!(refused.refused, Refused::StagingOccupied);
    assert_eq!(
        refused.to_string(),
        format!(
            "The destination staging path {} is occupied. Nothing was copied and the source was preserved; move or remove that file and try again.",
            staged.display()
        )
    );
    assert_eq!(backend.calls_of("remove_file"), 0, "the occupant is left");
    assert_eq!(backend.calls_of("rename_no_replace"), 0);
}

/// A staged copy that was admitted off the thread reads its staged name
/// once, before its turn, and not again on the thread that copies.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admitted_staged_copy_reads_its_staged_name_once() {
    let staged = resumable_staging_path(&p(DEST));
    let (disk, backend, _) = scripted();
    backend.stat_is(SOURCE, Ok(file(17)));
    backend.holds(SOURCE, b"synthetic payload");
    *backend.staged_contents.lock().unwrap() = Some(b"synthetic payload".to_vec());
    let policy = recorded_staged(Method::Copy, Fallback::None);
    let placed = bounded(disk.place_watched(SOURCE, DEST, policy, Watch::nobody()))
        .await
        .expect("copied");
    assert_eq!(placed.done, Done::Copied);
    let read = backend.paths_of("lstat");
    assert_eq!(
        read.iter().filter(|path| **path == staged).count(),
        1,
        "{read:?}"
    );
}

/// A move onto a free name says whether it moved, was not offered, or was
/// refused, and never falls back to a rename that replaces.
#[tokio::test]
async fn a_move_onto_a_free_name_moves_or_says_why_not() {
    let (disk, backend, _) = scripted();
    assert_eq!(
        disk.move_onto_free_name(SOURCE, DEST).await.expect("moved"),
        FreeNameMove::Moved
    );

    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let refused = disk.move_onto_free_name(SOURCE, DEST).await;
    assert_eq!(
        refused.expect_err("occupied").reason(),
        reason::DISK_DESTINATION_EXISTS
    );
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::CrossesDevices);
    let refused = disk.move_onto_free_name(SOURCE, DEST).await;
    assert_eq!(
        refused.expect_err("another volume").reason(),
        reason::DISK_MOVE_NEEDS_COPY
    );

    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    assert_eq!(
        disk.move_onto_free_name(SOURCE, DEST)
            .await
            .expect("not offered"),
        FreeNameMove::NotOffered
    );
    assert_eq!(backend.calls_of("rename"), 0);
    assert_eq!(backend.calls_of("open"), 0);
    assert_eq!(backend.calls_of("hard_link"), 0);
}

/// A name written precomposed comes back decomposed from an SMB share, so
/// the two spellings have to be recognized as one file.
#[test]
fn paths_differing_only_by_unicode_form_are_the_same_file() {
    let nfc = Path::new("/media/Pok\u{e9}mon/ep.mkv");
    let nfd = Path::new("/media/Poke\u{301}mon/ep.mkv");
    assert_ne!(nfc, nfd, "the two spellings differ as byte strings");
    assert!(paths_are_same_file(nfc, nfd));
}

#[test]
fn paths_differing_by_more_than_form_are_not_the_same_file() {
    assert!(!paths_are_same_file(
        Path::new("/media/one.mkv"),
        Path::new("/media/two.mkv")
    ));
}

mod real {
    //! The verb over the real filesystem, on synthetic files in a temporary
    //! directory. The clock is the manual one, so no path here waits.
    use super::*;

    fn disk() -> Disk {
        Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)))
    }

    fn moving() -> Placement {
        Placement {
            parents: Parents::Create,
            ..policy(Method::Move, Fallback::Copy)
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write fixture");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .into_string()
                    .expect("name")
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn crossing(source: &Path) -> DiskError {
        DiskError::new(DiskVerb::Move, source, io::ErrorKind::CrossesDevices.into())
    }

    /// `rename(2)` replaces the destination silently. This is the guarantee
    /// that stops any mover from destroying a file it was not asked to touch.
    #[test]
    fn refuses_to_replace_an_existing_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let refused = disk()
            .place_blocking(&source, &dest, moving())
            .expect_err("an occupied destination must not be replaced");
        assert_eq!(refused.refused, Refused::DestinationExists);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
    }

    #[test]
    fn replaces_only_when_the_caller_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let replacing = Placement {
            replace: Replace::Allowed,
            ..moving()
        };
        disk()
            .place_blocking(&source, &dest, replacing)
            .expect("an explicit overwrite should succeed");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"source-payload");
        assert!(!source.exists());
    }

    /// On a case-insensitive volume the destination resolves to the source,
    /// so this is a rename to perform rather than a collision to reject.
    #[test]
    fn performs_a_case_only_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("sample show.mkv");
        let dest = dir.path().join("SAMPLE SHOW.mkv");
        std::fs::write(&source, b"payload").unwrap();

        disk()
            .place_blocking(&source, &dest, moving())
            .expect("case-only rename should succeed");
        assert_eq!(std::fs::read(&dest).expect("renamed"), b"payload");
        assert_eq!(names(dir.path()), ["SAMPLE SHOW.mkv"]);
    }

    /// The destination name must never exist as an empty file, and no
    /// staging name is left behind.
    #[test]
    fn moves_into_a_folder_it_creates_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("nested").join("dest.mkv");
        std::fs::write(&source, b"payload").unwrap();

        let placed = disk()
            .place_blocking(&source, &dest, moving())
            .expect("a free destination should accept the move");
        assert_eq!(placed.done, Done::Renamed);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        assert_eq!(names(dir.path()), ["nested"]);
        assert_eq!(names(&dir.path().join("nested")), ["dest.mkv"]);
    }

    /// A failed move must never consume the file. Sonarr's
    /// RollbackPartialMove only deletes the target after confirming the
    /// source survived, and RollbackMove puts the file back when it did not.
    #[test]
    fn a_failed_move_leaves_the_file_where_it_started() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        write(&dest, b"occupied");
        // The source has moved to the staging name, the way it does on a
        // filesystem without an exclusive rename.
        let staged = staging_path_for(&dest);
        write(&staged, b"irreplaceable");

        let disk = disk();
        let _ = job(&disk, &source, &dest, moving()).roll_back(&staged, StagedHolds::OnlyCopy);

        assert_eq!(std::fs::read(&source).expect("back"), b"irreplaceable");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"occupied");
        assert!(!staged.exists(), "the staging name is released");
    }

    /// What each rollback leaves on a real volume: `(source, staged)` before
    /// and after, `None` for a name that holds nothing.
    #[test]
    fn rollbacks_never_remove_the_only_copy() {
        type Held = Option<&'static [u8]>;
        let unproven = StagedHolds::Copy { proven: false };
        let cases: [(StagedHolds, Held, Held, Held, Held); 8] = [
            // The source survived, so the staged file is a partial.
            (
                unproven,
                Some(b"payload"),
                Some(b"half"),
                Some(b"payload"),
                None,
            ),
            // A newcomer at the source neither costs the only copy nor is
            // replaced by it.
            (
                StagedHolds::OnlyCopy,
                Some(b"newcomer"),
                Some(b"only"),
                Some(b"newcomer"),
                Some(b"only"),
            ),
            // An empty claim is released while the source is there.
            (
                StagedHolds::EmptyClaim,
                Some(b"payload"),
                Some(b""),
                Some(b"payload"),
                None,
            ),
            // Beside an empty source too: nothing tells the two apart.
            (
                StagedHolds::EmptyClaim,
                Some(b""),
                Some(b""),
                Some(b""),
                None,
            ),
            // With no source it may be an empty source that a rename moved:
            // it goes back under the source's name.
            (StagedHolds::EmptyClaim, None, Some(b""), Some(b""), None),
            // An unproven copy is not promoted even when the source is gone.
            (unproven, None, Some(b"half"), None, Some(b"half")),
            // A claim that holds data may be the source: it goes back, and
            // never over a file there.
            (
                StagedHolds::EmptyClaim,
                None,
                Some(b"only"),
                Some(b"only"),
                None,
            ),
            (
                StagedHolds::EmptyClaim,
                Some(b"newcomer"),
                Some(b"only"),
                Some(b"newcomer"),
                Some(b"only"),
            ),
        ];
        for (index, (held, source_before, staged_before, source_after, staged_after)) in
            cases.into_iter().enumerate()
        {
            let dir = tempfile::tempdir().expect("tempdir");
            let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
            let staged = dir.path().join(".scryer-move.partial");
            for (path, bytes) in [(&source, source_before), (&staged, staged_before)] {
                if let Some(bytes) = bytes {
                    write(path, bytes);
                }
            }
            let disk = disk();
            let _ = job(&disk, &source, &dest, moving()).roll_back(&staged, held);
            let read = |path: &Path| std::fs::read(path).ok();
            assert_eq!(read(&source).as_deref(), source_after, "case {index}");
            assert_eq!(read(&staged).as_deref(), staged_after, "case {index}");
        }
    }

    /// An empty claim beside a source that cannot be read, or that reads as
    /// gone but cannot be put back, stays at its reserved name, and the
    /// refusal says so. Nothing is removed.
    #[cfg(unix)]
    #[test]
    fn an_empty_claim_stays_where_its_source_cannot_be_read_or_put_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let staged = dir.path().join(".scryer-move.partial");
        let dest = dir.path().join("dest.mkv");
        let neighbour = dir.path().join("neighbour.mkv");
        write(&neighbour, b"unrelated");
        let disk = disk();

        // The source's folder is gone, so the put-back has nowhere to go.
        write(&staged, b"");
        let source = dir.path().join("vanished").join("source.mkv");
        let failed = job(&disk, &source, &dest, moving());
        let said = failed.roll_back(&staged, StagedHolds::EmptyClaim);
        assert!(
            said.is_some(),
            "the refusal says what became of the empty file"
        );
        assert_eq!(
            failed.left.take(),
            vec![Leftover::PutBackFailed { at: staged.clone() }]
        );
        assert_eq!(std::fs::read(&staged).expect("kept"), b"");

        // The source's folder does not let its name be read.
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("locked folder");
        let source = locked.join("source.mkv");
        std::fs::write(&source, b"payload").unwrap();
        let set = |mode| std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(mode));
        set(0o000).expect("lock");
        if std::fs::symlink_metadata(&source).is_ok() {
            // This user reads through any mode; the case does not apply.
            set(0o755).expect("unlock");
            return;
        }
        let unread = job(&disk, &source, &dest, moving());
        let said = unread.roll_back(&staged, StagedHolds::EmptyClaim);
        set(0o755).expect("unlock");
        assert!(
            said.is_some(),
            "the refusal says what became of the empty file"
        );
        assert_eq!(
            unread.left.take(),
            vec![Leftover::AtReservedName { at: staged.clone() }]
        );
        assert_eq!(std::fs::read(&staged).expect("kept"), b"");
        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
        assert_eq!(std::fs::read(&neighbour).expect("neighbour"), b"unrelated");
    }

    /// Installing the roots changes only reuse and timeouts: placing a file
    /// into a root, and refusing one onto an occupied name, end the same way.
    #[test]
    fn a_placement_ends_the_same_with_the_roots_installed() {
        fn run(disk: &Disk, root: &Path) -> (Vec<(String, Vec<u8>)>, Option<Refused>) {
            let downloads = root.join("downloads");
            let season = root.join("Invented Show").join("Season 01");
            std::fs::create_dir_all(&downloads).expect("downloads");
            std::fs::create_dir_all(&season).expect("season");
            write(&downloads.join("one.mkv"), b"episode-one");
            write(&downloads.join("two.mkv"), b"episode-two");
            write(&season.join("Invented Show - S01E02.mkv"), b"already-here");
            let first = season.join("Invented Show - S01E01.mkv");
            disk.place_blocking(&downloads.join("one.mkv"), &first, moving())
                .expect("place the first episode");
            let second = season.join("Invented Show - S01E02.mkv");
            let refused = disk
                .place_blocking(&downloads.join("two.mkv"), &second, moving())
                .err()
                .map(|refusal| refusal.refused);
            let mut tree = Vec::new();
            for dir in [&downloads, &season] {
                for name in names(dir) {
                    let bytes = std::fs::read(dir.join(&name)).expect("read");
                    tree.push((name, bytes));
                }
            }
            (tree, refused)
        }

        let unset = tempfile::tempdir().expect("tempdir");
        let installed = tempfile::tempdir().expect("tempdir");
        let without = run(&disk(), unset.path());
        let rooted = disk();
        rooted.set_volumes([(installed.path().to_path_buf(), VolumeClass::LibraryRoot)]);
        let with = run(&rooted, installed.path());

        assert_eq!(without.1, Some(Refused::DestinationExists));
        assert_eq!(with, without);
    }

    /// A cross-device copy is written under the staging name, so nothing is
    /// under the real name while it is unproven, and a destination that
    /// appears meanwhile is neither replaced nor costs the source.
    #[cfg(unix)]
    #[test]
    fn a_cross_device_copy_never_shows_an_unproven_file_or_replaces_a_newcomer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();

        // The copy reports from inside its write loop, which is where the
        // folder is looked at and the newcomer arrives.
        let looked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watch = Watch {
            progress: Progress::to({
                let (folder, dest, looked) =
                    (dir.path().to_path_buf(), dest.clone(), looked.clone());
                move |phase, _| {
                    if phase != Phase::Copying
                        || looked.swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        return;
                    }
                    // Only the source and the staged copy: nothing under the
                    // real name.
                    let mid_copy = names(&folder);
                    assert_eq!(mid_copy.len(), 2, "{mid_copy:?}");
                    assert!(
                        mid_copy[0].starts_with(".scryer-") && mid_copy[0].ends_with(".partial")
                    );
                    assert_eq!(mid_copy[1], "source.mkv");
                    write(&dest, b"newcomer");
                }
            }),
            cancel: Cancel::never(),
        };
        let disk = disk();
        let refused = Job {
            watch: &watch,
            ..job(&disk, &source, &dest, moving())
        }
        .across_volumes(crossing(&source))
        .map_err(|fail| fail.refused)
        .expect_err("the newcomer wins");

        assert!(looked.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(refused, Refused::DestinationExists);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"newcomer");
        // The source is untouched and the staged copy is taken back.
        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
        assert_eq!(names(dir.path()), ["dest.mkv", "source.mkv"]);
    }

    /// On a case-sensitive volume two names that differ only in case are two
    /// files, so the case-only path must refuse rather than replace. Runs
    /// only where the temporary directory is case-sensitive.
    #[test]
    fn a_case_only_name_of_a_different_file_is_not_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("case-probe"), b"probe");
        if dir.path().join("CASE-PROBE").exists() {
            return;
        }
        std::fs::remove_file(dir.path().join("case-probe")).expect("remove probe");
        let source = dir.path().join("sample show.mkv");
        let dest = dir.path().join("SAMPLE SHOW.mkv");
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let refused = disk()
            .place_blocking(&source, &dest, moving())
            .expect_err("a different file under the case-only name must not be replaced");
        assert_eq!(refused.refused, Refused::DestinationExists);
        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(names(dir.path()).len(), 2);
    }

    /// A source that cannot be unlinked but is still there fails the move
    /// and takes the proven copy back, leaving no duplicate.
    #[cfg(unix)]
    #[test]
    fn a_proven_copy_is_dropped_while_the_source_is_still_there() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            // Root unlinks anywhere, so the locked folder would refuse nothing.
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let held = dir.path().join("held");
        std::fs::create_dir(&held).expect("source folder");
        let (source, dest) = (held.join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o555))
            .expect("lock the source folder");

        let disk = disk();
        let outcome = job(&disk, &source, &dest, moving()).across_volumes(crossing(&source));
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755))
            .expect("unlock the source folder");

        assert!(outcome.is_err(), "the source could not be unlinked");
        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
        assert!(!dest.exists(), "the source survives, so no duplicate stays");
    }

    /// A source whose own permissions forbid writing it is unlinked after
    /// its copy is proven, as an unlink has always done on Unix, where only
    /// the folder decides.
    #[cfg(unix)]
    #[test]
    fn a_cross_device_move_unlinks_a_source_without_write_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o444))
            .expect("make the source unwritable");

        let disk = disk();
        let done = job(&disk, &source, &dest, moving()).across_volumes(crossing(&source));
        assert_eq!(done.ok(), Some(Done::CopiedThenRemovedSource));
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
    }

    /// A symbolic link moved to another volume is made again there, and the
    /// placement says so instead of claiming a copy.
    #[cfg(unix)]
    #[test]
    fn a_cross_device_move_of_a_symlink_makes_the_link_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.mkv");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        write(&target, b"payload");
        std::os::unix::fs::symlink(&target, &source).expect("link the source");

        let disk = disk();
        let done = job(&disk, &source, &dest, moving()).across_volumes(crossing(&source));
        assert_eq!(done.ok(), Some(Done::RelinkedThenRemovedSource));
        assert_eq!(std::fs::read_link(&dest).expect("dest link"), target);
        assert!(std::fs::symlink_metadata(&source).is_err());
        assert_eq!(std::fs::read(&target).expect("target"), b"payload");
        let relinked = Placed {
            asked: Method::Move,
            done: Done::RelinkedThenRemovedSource,
            copied: None,
        };
        assert!(relinked.fell_back());
    }

    /// On Windows a read-only source refuses the unlink until its attribute
    /// is cleared, which a cross-device move does once the copy is in place.
    #[cfg(windows)]
    #[test]
    fn a_windows_cross_device_move_unlinks_a_read_only_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();
        set_read_only(&source, true);

        let disk = disk();
        let done = job(&disk, &source, &dest, moving()).across_volumes(crossing(&source));
        assert_eq!(done.ok(), Some(Done::CopiedThenRemovedSource));
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        set_read_only(&dest, false);
    }

    #[cfg(windows)]
    fn set_read_only(path: &Path, read_only: bool) {
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(read_only);
        std::fs::set_permissions(path, permissions).expect("set read-only");
    }

    /// A source that cannot be read might still be there or might not, so
    /// the staged file is neither removed nor moved over it.
    #[cfg(unix)]
    #[test]
    fn a_rollback_that_cannot_read_the_source_leaves_the_staged_file() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            // Root reads anywhere, so the locked folder would hide nothing.
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let held = dir.path().join("held");
        std::fs::create_dir(&held).expect("source folder");
        let (source, dest) = (held.join("source.mkv"), dir.path().join("dest.mkv"));
        let staged = dir.path().join(".scryer-partial.partial");
        std::fs::write(&source, b"payload").unwrap();
        write(&staged, b"staged-payload");
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o000))
            .expect("lock the source folder");

        let disk = disk();
        let held_copy = StagedHolds::Copy { proven: true };
        let _ = job(&disk, &source, &dest, moving()).roll_back(&staged, held_copy);
        std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o755))
            .expect("unlock the source folder");

        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
        assert_eq!(std::fs::read(&source).expect("source"), b"payload");
    }

    /// NTFS renames without replacing, so an occupied destination is refused
    /// in the one call and both files are left as they were.
    #[cfg(windows)]
    #[test]
    fn a_windows_exclusive_rename_refuses_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        write(&source, b"source-payload");
        write(&dest, b"dest-payload");

        let refused = disk().exclusive_rename_blocking(&source, &dest);
        let Some(Err(error)) = refused else {
            panic!("an occupied destination must be refused: {refused:?}");
        };
        assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&source).expect("source"), b"source-payload");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_exclusive_rename_moves_to_a_free_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();

        assert!(matches!(
            disk().exclusive_rename_blocking(&source, &dest),
            Some(Ok(()))
        ));
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
    }

    /// A read-only file moves by rename, so no staging link is left behind
    /// that its attribute would keep from being unlinked.
    #[cfg(windows)]
    #[test]
    fn a_windows_read_only_file_moves_without_a_staging_leftover() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (dir.path().join("source.mkv"), dir.path().join("dest.mkv"));
        std::fs::write(&source, b"payload").unwrap();
        set_read_only(&source, true);

        disk()
            .place_blocking(&source, &dest, moving())
            .expect("the move succeeds");
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        assert_eq!(names(dir.path()), ["dest.mkv"]);
        set_read_only(&dest, false);
    }

    #[test]
    fn blocking_claim_refuses_an_occupied_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (staged, dest) = (dir.path().join("staged.tmp"), dir.path().join("dest.mkv"));
        write(&staged, b"staged-payload");
        write(&dest, b"dest-payload");

        let error = disk()
            .move_claimed_blocking(&staged, &dest)
            .expect_err("a claimed move must not replace an existing destination");
        assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        assert_eq!(std::fs::read(&dest).expect("dest"), b"dest-payload");
        assert_eq!(std::fs::read(&staged).expect("staged"), b"staged-payload");
    }
}

/// What a placement handed its first reads does not read again: the source's
/// name and the destination's folder, two reads fewer per file.
#[tokio::test]
async fn a_placement_after_reading_reads_neither_the_source_nor_the_folder_again() {
    let reads = |disk: &Disk| disk.calls("lstat") + disk.calls("stat");
    let mut counted = Vec::new();
    for reuse in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = (temp.path().join("one.mkv"), temp.path().join("two.mkv"));
        std::fs::write(&source, b"payload").unwrap();
        let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
        let moving = policy(Method::Move, Fallback::None);
        let before = match reuse {
            false => {
                let before = reads(&disk);
                disk.place(&source, &dest, moving).await.expect("moved");
                before
            }
            true => {
                let read = disk
                    .read_before_placing(&source, &dest, moving)
                    .await
                    .expect("read");
                let before = reads(&disk);
                disk.place_after_reading(&source, &dest, moving, &read)
                    .await
                    .expect("moved");
                before
            }
        };
        assert_eq!(std::fs::read(&dest).expect("moved"), b"payload");
        counted.push(reads(&disk) - before);
    }
    assert_eq!(counted[1] + 2, counted[0], "{counted:?}");
}

/// A staged recorded copy takes its turn on the volume its folder's read
/// named, without reading the destination and its folder again for it.
#[cfg(unix)]
#[tokio::test]
async fn a_staged_copy_takes_its_turn_from_the_folder_it_already_read() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (source, dest) = (temp.path().join("one.mkv"), temp.path().join("two.mkv"));
    std::fs::write(&source, b"payload").unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let copying = Placement {
        method: Method::Copy,
        fallback: Fallback::None,
        replace: Replace::Never,
        verify: Verify::Recorded(Evidence {
            depth: Depth::Quick,
            landing: Landing::Staged,
        }),
        parents: Parents::MustExist,
    };
    disk.place(&source, &dest, copying).await.expect("copied");
    assert_eq!(std::fs::read(&dest).expect("copied"), b"payload");
    // Reading the volume again for the turn made this four: the destination
    // and then its folder, followed.
    assert_eq!(disk.calls("stat"), 2);
}

/// A folder that read as there and is gone when the placement writes into it
/// is made again, and the placement is tried once more.
#[tokio::test]
async fn a_folder_gone_since_it_was_read_is_made_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("one.mkv");
    std::fs::write(&source, b"payload").unwrap();
    let dest = temp
        .path()
        .join("Invented Show")
        .join("Season 01")
        .join("two.mkv");
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let source_read = disk.uncached(&source).await;
    // The folder is said to be made, and is not there.
    let read = ReadBefore::made_folder(source_read, &dest);
    let moving = Placement {
        parents: Parents::Create,
        ..policy(Method::Move, Fallback::None)
    };
    disk.place_after_reading(&source, &dest, moving, &read)
        .await
        .expect("moved into the folders made again");
    assert_eq!(std::fs::read(&dest).expect("moved"), b"payload");
    assert!(!source.exists());
}

/// A put-back that reports failure may still have applied. One look at the
/// name it left says where the file is: gone means it is back under the
/// source's name, and only then; present or unread means it stayed.
#[cfg(unix)]
#[test]
fn a_failed_put_back_says_where_the_file_is() {
    let staged = p(STAGED);
    let unread = Err(io::ErrorKind::PermissionDenied);
    for (after, expected) in [
        (
            Err(io::ErrorKind::NotFound),
            Leftover::PutBack { at: p(SOURCE) },
        ),
        (Ok(file(9)), Leftover::PutBackFailed { at: staged.clone() }),
        (unread, Leftover::PutBackFailed { at: staged.clone() }),
    ] {
        let (disk, backend, _) = scripted();
        backend.stats_in_turn(&staged, &[Ok(file(9)), after]);
        backend.fails("rename_no_replace", STAGED, io::ErrorKind::TimedOut);
        let policy = policy(Method::Move, Fallback::Copy);
        let (source, dest) = (p(SOURCE), p(DEST));
        let failed = job(&disk, &source, &dest, policy);
        let said = failed.roll_back(&staged, StagedHolds::Copy { proven: true });
        assert_eq!(said, None);
        assert_eq!(failed.left.take(), vec![expected]);
        assert_eq!(backend.calls_of("remove_file"), 0, "nothing is removed");
    }
}

/// A copy whose staged name cannot be read may still be there, and the
/// refusal says so.
#[test]
fn an_unread_staged_copy_is_reported_where_it_may_be() {
    let (disk, backend, _) = scripted();
    *backend.staged_stat.lock().unwrap() = Some(Err(io::ErrorKind::PermissionDenied));
    let policy = policy(Method::Move, Fallback::Copy);
    let (source, dest) = (p(SOURCE), p(DEST));
    let failed = job(&disk, &source, &dest, policy);
    let _ = failed.roll_back(&p(STAGED), StagedHolds::Copy { proven: true });
    assert_eq!(
        failed.left.take(),
        vec![Leftover::AtReservedName { at: p(STAGED) }]
    );
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A recorded copy whose staged file cannot be promoted, by a claimed rename
/// that leaves its reservation, says what it left at the destination name.
#[test]
fn a_recorded_copy_whose_promotion_fails_says_what_it_left() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.always_fails("hard_link", io::ErrorKind::Unsupported);
    backend.always_fails("rename_claimed", io::ErrorKind::TimedOut);
    *backend.claim_left.lock().unwrap() = true;
    copies(&backend, b"synthetic payload", b"synthetic payload");
    let refused = place(&disk, recorded_staged(Method::Copy, Fallback::None)).expect_err("refused");
    assert_eq!(
        refused.left,
        vec![Leftover::AtReservedName { at: p(DEST) }],
        "{refused:?}"
    );
}

/// An exclusive rename or a staged write that fails on the claimed path and
/// leaves its reservation says so in a typed field.
#[test]
fn a_claimed_rename_that_fails_says_what_it_left() {
    let (disk, backend, _) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    backend.always_fails("rename_claimed", io::ErrorKind::TimedOut);
    *backend.claim_left.lock().unwrap() = true;
    let left = Some(Leftover::AtReservedName { at: p(DEST) });
    let moved = disk
        .move_exclusive_blocking(&p(SOURCE), &p(DEST))
        .expect_err("refused");
    assert_eq!(moved.left(), left.as_ref());
    let written = disk
        .write_staged_blocking(
            &p(DEST),
            b"synthetic",
            Replace::Never,
            Access::Default,
            Staging::Unique,
            false,
        )
        .expect_err("refused");
    assert_eq!(written.left(), left.as_ref());

    // Without a reservation left, nothing is said.
    *backend.claim_left.lock().unwrap() = false;
    let moved = disk
        .move_exclusive_blocking(&p(SOURCE), &p(DEST))
        .expect_err("refused");
    assert_eq!(moved.left(), None);
}
