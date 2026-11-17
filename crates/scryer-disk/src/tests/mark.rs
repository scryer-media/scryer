//! Marks: what each verb answers, what it could not tell, and how many
//! probes it spends, against the scripted backend and then on real files.

use super::*;
use crate::backend::Extra;

const A: &str = "/srv/fixture-media/Fixture Show/a.mkv";
const B: &str = "/srv/fixture-media/Fixture Show/b.mkv";

pub(super) fn numbered(device: Option<u64>, inode: Option<u64>) -> RawStat {
    RawStat {
        extra: Extra {
            device,
            inode,
            ..Default::default()
        },
        ..file(5)
    }
}

fn scripted() -> (Disk, Arc<Scripted>) {
    owner(&ManualClock::at(at(0)))
}

fn mark(stat: RawStat) -> Mark {
    Mark::of_stat(&stat)
}

#[test]
fn a_fingerprint_is_one_probe_and_says_why_there_is_none() {
    let (disk, backend) = scripted();
    backend.stat_is(A, Ok(file(5)));
    assert!(disk.fingerprint_blocking(&p(A)).is_ok());
    assert!(disk.fingerprint_blocking(&p(A)).is_ok());
    assert_eq!(backend.paths_of("lstat"), [p(A), p(A)], "never cached");
    assert!(disk.fingerprint_followed_blocking(&p(A)).is_ok());
    assert_eq!(backend.paths_of("stat"), [p(A)]);

    let missing = disk.fingerprint_blocking(&p(B)).expect_err("missing");
    assert_eq!(missing.exists(), Some(false));
    backend.stat_is(B, Err(io::ErrorKind::PermissionDenied));
    let unread = disk.fingerprint_blocking(&p(B)).expect_err("unreadable");
    assert_eq!(unread.exists(), None, "unreadable is never absent");
    assert_eq!(
        unread.unreachable_cause(),
        Some(reason::DISK_PERMISSION_DENIED)
    );
}

#[test]
fn two_marks_are_one_file_only_when_the_platform_can_tell() {
    let at = |device, inode| mark(numbered(device, inode));
    let same = |left: &Mark, right: &Mark| left.same_file(right);
    assert_eq!(
        same(&at(Some(7), Some(9)), &at(Some(7), Some(9))),
        Sameness::Same
    );
    assert_eq!(
        same(&at(Some(7), Some(9)), &at(Some(7), Some(10))),
        Sameness::Different
    );
    assert_eq!(
        same(&at(Some(7), Some(9)), &at(Some(8), Some(9))),
        Sameness::Different
    );
    // A zero inode is carried, not dropped: two of them prove nothing, and
    // one beside a real number is another file.
    assert_eq!(
        same(&at(Some(7), Some(0)), &at(Some(7), Some(0))),
        Sameness::CannotTell
    );
    assert_eq!(
        same(&at(Some(7), Some(0)), &at(Some(7), Some(9))),
        Sameness::Different
    );
    assert_eq!(same(&at(None, None), &at(None, None)), Sameness::CannotTell);
    assert_eq!(
        same(&at(Some(7), Some(9)), &at(None, None)),
        Sameness::CannotTell
    );

    let volume = |left: &Mark, right: &Mark| left.same_volume(right);
    assert_eq!(
        volume(&at(Some(7), Some(0)), &at(Some(7), Some(9))),
        Sameness::Same
    );
    assert_eq!(
        volume(&at(Some(7), Some(9)), &at(Some(8), Some(9))),
        Sameness::Different
    );
    assert_eq!(
        volume(&at(Some(7), Some(9)), &at(None, None)),
        Sameness::CannotTell
    );
}

#[test]
fn a_later_mark_says_what_differs() {
    let base = numbered(Some(7), Some(9));
    let with = |change: fn(&mut RawStat)| {
        let mut stat = base;
        change(&mut stat);
        mark(base).compare(&mark(stat))
    };
    assert_eq!(with(|_| {}), Drift::Same);
    assert_eq!(with(|stat| stat.extra.inode = Some(10)), Drift::Replaced);
    assert_eq!(with(|stat| stat.len = 6), Drift::Rewritten);
    assert_eq!(
        with(|stat| stat.modified = Some(std::time::UNIX_EPOCH)),
        Drift::Rewritten
    );
    assert_eq!(
        with(|stat| stat.extra.changed = Some((1, 2))),
        Drift::Touched
    );
    assert_eq!(
        with(|stat| stat.extra.created = Some(std::time::UNIX_EPOCH)),
        Drift::Touched
    );
    assert_eq!(with(|stat| stat.extra.owner = Some(501)), Drift::Reowned);
    assert_eq!(with(|stat| stat.links = Some(2)), Drift::Same);
    // Where the platform cannot tell the files apart, equal facts are
    // reported as only that.
    let unnumbered = mark(numbered(Some(7), Some(0)));
    assert_eq!(unnumbered.compare(&unnumbered), Drift::SameFacts);
    assert!(Drift::SameFacts.is_untouched() && Drift::Same.is_untouched());
    assert!(Drift::Touched.is_unwritten() && !Drift::Touched.is_untouched());
    // A new owner is not a rewrite, and is still told from none.
    assert!(Drift::Reowned.is_unwritten() && !Drift::Reowned.is_untouched());
    assert!(Drift::Touched.is_unwritten_by_its_owner());
    assert!(!Drift::Reowned.is_unwritten_by_its_owner());
    for unshown in [Drift::Rewritten, Drift::Replaced, Drift::CannotTell] {
        assert!(!unshown.is_unwritten() && !unshown.is_untouched());
    }
    // A mark of a volume alone says nothing about a file on it.
    let volume = mark(base).volume();
    assert_eq!(volume.compare(&mark(base)), Drift::CannotTell);
    assert_eq!(mark(base).compare(&volume), Drift::CannotTell);
    let stored = Mark::from_stored("unix-dev:7").expect("volume");
    assert_eq!(stored.compare(&mark(base)), Drift::CannotTell);
}

#[test]
fn a_mark_answers_about_its_length_without_handing_it_out() {
    let taken = mark(file(5));
    assert!(taken.is_len(5) && !taken.is_len(6));
    assert!(taken.exceeds(4) && !taken.exceeds(5));
    assert_eq!(taken.kind(), Some(EntryKind::File));
    assert_eq!(taken.volume().kind(), None);
    assert!(!taken.volume().is_len(5) && !taken.volume().exceeds(0));
}

/// A time before the epoch carries one sign, in front, however near the
/// epoch it is.
#[test]
fn a_label_writes_a_time_on_either_side_of_the_epoch() {
    let epoch = std::time::UNIX_EPOCH;
    let pause = std::time::Duration::new;
    let label = |modified| {
        let unnumbered = RawStat {
            modified,
            ..numbered(None, None)
        };
        mark(unnumbered).label()
    };
    assert_eq!(label(None), "len=5 modified=unknown");
    for (modified, written) in [
        (
            epoch + pause(1_700_000_000, 123_456_789),
            "1700000000.123456789",
        ),
        (epoch + pause(0, 500_000_000), "0.500000000"),
        (epoch - pause(0, 500_000_000), "-0.500000000"),
        (epoch - pause(5, 300_000_000), "-5.300000000"),
    ] {
        assert_eq!(label(Some(modified)), format!("len=5 modified={written}"));
    }
}

#[test]
fn a_stored_mark_reads_back_as_the_same_mark() {
    let epoch = std::time::UNIX_EPOCH;
    let full = RawStat {
        kind: EntryKind::Symlink,
        len: 17,
        modified: Some(epoch + std::time::Duration::new(1_700_000_000, 123_456_789)),
        identity: None,
        links: Some(2),
        extra: Extra {
            device: Some(7),
            inode: Some(0),
            changed: Some((-3, 5)),
            created: Some(epoch - std::time::Duration::new(2, 500)),
            owner: Some(501),
        },
    };
    let marks = [
        mark(full),
        mark(dir()),
        Mark::from_stored("mark1:f:5:-:-:-:w42:-:-").expect("a Windows mark"),
        mark(file(5)).volume(),
        Mark::from_stored("windows-volume:42").expect("a Windows volume"),
        mark(dir()).volume(),
    ];
    for taken in &marks {
        let stored = taken.to_stored();
        let read = Mark::from_stored(&stored).unwrap_or_else(|| panic!("{stored}"));
        assert_eq!(read.to_stored(), stored);
        assert_eq!(format!("{read:?}"), format!("{:?}", taken.as_stored()));
    }
    assert_eq!(marks[3].to_stored(), "unix-dev:1");
    assert_eq!(marks[4].to_stored(), "windows-volume:42");
    assert_eq!(marks[5].to_stored(), "unknown-volume");
    assert!(marks[3].names_volume() && !marks[5].names_volume());
    for unread in [
        "",
        "mark1",
        "mark2:f:5:-:-:-:u1:105:-",
        "mark1:x:5:-:-:-:u1:105:-",
        "mark1:f:5:-:-:-:q1:105:-",
        // A Windows file index is nothing this service writes.
        "mark1:f:5:-:-:-:w42:99:-",
        "mark1:f:5:-:-:-:u1:105:-:extra",
        "unix-dev:",
        "unix-dev:seven",
        // Another spelling of a number is not what was written.
        "unix-dev:+7",
        "unix-dev:07",
        "windows-volume:+7",
        "mark1:f:05:-:-:-:u1:105:-",
        "mark1:f:5:-:-:-:u+1:105:-",
        "mark1:f:5:-:5.-3:-:u1:105:-",
        "mark1:f:5:-:5.1000000000:-:u1:105:-",
        "mark1:f:5:5.1000000000:-:-:u1:105:-",
        "mark1:f:5:-:+5.3:-:u1:105:-",
    ] {
        assert!(Mark::from_stored(unread).is_none(), "{unread}");
    }
}

/// A fingerprint that does not answer within the root's bound is
/// unreachable, not missing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fingerprint_is_bounded_off_the_runtime() {
    let (disk, backend) = scripted();
    backend.stat_is(A, Ok(file(5)));
    disk.set_probe_timeout_for_test(std::time::Duration::from_millis(1));
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    let silent = disk.fingerprint(A).await.expect_err("no answer");
    assert_eq!(silent.unreachable_cause(), Some(reason::DISK_TIMED_OUT));
    open.send(()).unwrap();
}

/// The volume a path will land on is read through the backend, one probe
/// for each ancestor tried, and decides both the copy lane and whether a
/// move can be a rename.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_landing_volume_is_its_nearest_answering_ancestor() {
    let (disk, backend) = scripted();
    let folder = "/srv/fixture-media/Fixture Show";
    backend.stat_is(folder, Ok(numbered(Some(7), Some(3))));
    backend.stat_is(B, Ok(numbered(Some(7), Some(4))));
    let volume = disk.landing_volume_blocking(&p(A)).expect("volume");
    assert_eq!(backend.paths_of("stat"), [p(A), p(folder)]);
    assert_eq!(volume.to_stored(), "unix-dev:7");
    assert_eq!(disk.copy_lane(&p(A)), "unix-dev:7");
    assert!(disk.renames_between(&p(B), &p(A)).await);
    backend.stat_is(B, Ok(numbered(Some(8), Some(4))));
    assert!(!disk.renames_between(&p(B), &p(A)).await);

    let (nowhere, silent) = scripted();
    silent.always_fails("stat", io::ErrorKind::PermissionDenied);
    assert!(nowhere.landing_volume_blocking(&p(A)).is_none());
    assert_eq!(nowhere.copy_lane(&p(A)), "unknown-volume");

    // A volume that answers gives a turn on its own lane.
    disk.install_copy_workers(1);
    let free = |lane| super::copying::turn_is_free(&disk.copy_admission(), lane);
    let turn = disk.copy_turn(&p(A)).await;
    assert!(turn.is_ok() && free("unknown-volume") && !free("unix-dev:7"));
}

#[cfg(unix)]
#[test]
fn real_links_and_handles_are_recognised() {
    let temp = tempfile::tempdir().unwrap();
    let (first, second, other) = (
        temp.path().join("first.mkv"),
        temp.path().join("second.mkv"),
        temp.path().join("other.mkv"),
    );
    std::fs::write(&first, b"synthetic").unwrap();
    std::fs::write(&other, b"synthetic").unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let read = |path: &Path| disk.fingerprint_blocking(path).unwrap();
    let alone = read(&first);
    std::fs::hard_link(&first, &second).unwrap();
    assert_eq!(alone.same_file(&read(&second)), Sameness::Same);
    assert_eq!(alone.same_file(&read(&other)), Sameness::Different);
    assert_eq!(alone.same_volume(&read(&other)), Sameness::Same);

    // An open handle's own metadata gives the mark the path gives.
    let handle = std::fs::File::open(&first).unwrap();
    let opened = Mark::of(&handle.metadata().unwrap());
    assert_eq!(opened.compare(&read(&second)), Drift::Same);
    assert_eq!(opened.compare(&read(&other)), Drift::Replaced);
    std::fs::write(&first, b"synthetic, rewritten").unwrap();
    assert_eq!(opened.compare(&read(&first)), Drift::Rewritten);
}

/// A path's own stat names no volume and no file on Windows; the volume a
/// path lands on comes from a handle, and two names in one folder share it.
#[cfg(windows)]
#[test]
fn windows_names_a_volume_from_a_handle_and_tells_no_files_apart() {
    let temp = tempfile::tempdir().unwrap();
    let (first, second) = (
        temp.path().join("first.mkv"),
        temp.path().join("second.mkv"),
    );
    std::fs::write(&first, b"synthetic").unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let plain = |path: &Path| disk.fingerprint_blocking(path).unwrap();
    assert_eq!(
        plain(&first).same_file(&plain(&second)),
        Sameness::CannotTell
    );
    assert_eq!(
        plain(&first).same_volume(&plain(&second)),
        Sameness::CannotTell
    );
    assert_eq!(plain(&first).compare(&plain(&first)), Drift::SameFacts);
    let volume = disk.landing_volume_blocking(&first).expect("volume");
    let missing = temp.path().join("not-created/third.mkv");
    let landing = disk.landing_volume_blocking(&missing).expect("volume");
    assert_eq!(volume.same_volume(&landing), Sameness::Same);
    assert!(volume.to_stored().starts_with("windows-volume:"));
    assert_eq!(volume.to_stored(), disk.copy_lane(&missing));
}

/// On Windows two paths share a volume only when they name one drive or
/// share and answer one non-zero volume serial. A serial either side does
/// not answer, a serial of zero, or a linked source is no answer. A move
/// asks none of this: it copies on Windows, reading nothing first.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_volumes_are_shared_only_with_one_prefix_and_one_nonzero_serial() {
    let folder = "/srv/fixture-media/Fixture Show";
    let setup = |source: Option<u64>, landing: Option<u64>| {
        let (disk, backend) = scripted();
        backend.stat_is(B, Ok(file(5)));
        backend.stat_is(folder, Ok(dir()));
        if let Some(serial) = source {
            backend.serial_is(B, serial);
        }
        if let Some(serial) = landing {
            backend.serial_is(folder, serial);
        }
        (disk, backend)
    };

    let (equal, backend) = setup(Some(7), Some(7));
    assert!(equal.shares_volume(&p(B), &p(A)).await);
    assert_eq!(backend.paths_of("volume_serial"), [p(B), p(folder)]);
    // A move reads nothing to decide: it copies.
    let before = backend.paths_of("volume_serial").len();
    assert!(!equal.renames_between(&p(B), &p(A)).await);
    assert_eq!(backend.paths_of("volume_serial").len(), before);

    let (different, _) = setup(Some(7), Some(8));
    assert!(!different.shares_volume(&p(B), &p(A)).await);

    let (zero, _) = setup(Some(0), Some(0));
    assert!(!zero.shares_volume(&p(B), &p(A)).await);

    let (source_unread, _) = setup(None, Some(7));
    assert!(!source_unread.shares_volume(&p(B), &p(A)).await);

    let (landing_unread, _) = setup(Some(7), None);
    assert!(!landing_unread.shares_volume(&p(B), &p(A)).await);

    let (linked, backend) = setup(Some(7), Some(7));
    backend.stat_is(
        B,
        Ok(RawStat {
            kind: EntryKind::Symlink,
            ..file(5)
        }),
    );
    assert!(!linked.shares_volume(&p(B), &p(A)).await);
    assert!(backend.paths_of("volume_serial").is_empty());

    let (missing, _) = setup(Some(7), Some(7));
    assert!(!missing.shares_volume(&p(A), &p(B)).await);

    // One serial on two drives or two shares is still two volumes, and
    // nothing is read to say so; one drive spelt in two cases is one.
    let drive = r"C:\fixture-media\b.mkv";
    for (other, other_folder, shared) in [
        (r"D:\fixture-media\a.mkv", r"D:\fixture-media", false),
        (
            r"\\fixture-host\media\a.mkv",
            r"\\fixture-host\media",
            false,
        ),
        (r"c:\fixture-media\a.mkv", r"c:\fixture-media", true),
    ] {
        let (disk, backend) = scripted();
        backend.stat_is(drive, Ok(file(5)));
        backend.serial_is(drive, 7);
        backend.stat_is(other_folder, Ok(dir()));
        backend.serial_is(other_folder, 7);
        assert_eq!(
            disk.shares_volume(&p(drive), &p(other)).await,
            shared,
            "{other}"
        );
        if !shared {
            assert!(backend.paths_of("volume_serial").is_empty(), "{other}");
            assert_eq!(backend.calls_of("lstat"), 0, "{other}");
        }
    }
    let share = r"\\fixture-host\media\b.mkv";
    let (disk, backend) = scripted();
    backend.stat_is(share, Ok(file(5)));
    backend.serial_is(share, 7);
    backend.stat_is(r"\\fixture-host\other", Ok(dir()));
    backend.serial_is(r"\\fixture-host\other", 7);
    assert!(
        !disk
            .shares_volume(&p(share), &p(r"\\fixture-host\other\a.mkv"))
            .await
    );
}

/// Two names on one drive, the destination's folder not made yet, are on one
/// volume; a source that is not there answers no. A move between them still
/// copies on Windows.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_two_names_on_one_drive_share_a_volume_but_a_move_still_copies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mkv");
    std::fs::write(&source, b"synthetic").unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let beside = temp.path().join("beside.mkv");
    assert!(disk.shares_volume(&source, &beside).await);
    assert!(!disk.renames_between(&source, &beside).await);
    let nested = temp.path().join("not-created/yet/destination.mkv");
    assert!(disk.shares_volume(&source, &nested).await);
    let missing = temp.path().join("missing.mkv");
    assert!(!disk.shares_volume(&missing, &nested).await);
}

/// A folder read as a file, numbered on its volume.
fn folder(device: u64, inode: u64) -> RawStat {
    RawStat {
        kind: EntryKind::Directory,
        links: Some(2),
        ..numbered(Some(device), Some(inode))
    }
}

/// A file with `links` names, numbered on its volume.
fn named(inode: u64, links: u64) -> RawStat {
    RawStat {
        links: Some(links),
        ..numbered(Some(1), Some(inode))
    }
}

/// One folder reached by two spellings, as one volume mounted at two paths
/// shows it, holds one entry under each: the folders are read as files, one
/// stat each, and nothing is resolved.
#[test]
fn one_folder_under_two_mounts_holds_one_entry() {
    let (source, dest) = (
        p("/srv/fixture-media/mount-a/downloads/sample.mkv"),
        p("/srv/fixture-media/mount-b/downloads/sample.mkv"),
    );
    let (disk, backend) = scripted();
    backend.stat_is("/srv/fixture-media/mount-a/downloads", Ok(folder(1, 50)));
    backend.stat_is("/srv/fixture-media/mount-b/downloads", Ok(folder(1, 50)));
    assert_eq!(
        disk.entry_alias_blocking(&source, &dest, &mark(named(7, 1))),
        EntryAlias::Same
    );
    assert_eq!(backend.calls_of("stat"), 2);
    assert_eq!(
        backend.calls_of("lstat") + backend.calls_of("canonicalize"),
        0
    );
}

/// Another folder is another entry, a second name of the source's own file
/// included.
#[test]
fn a_second_name_in_another_folder_is_another_entry() {
    let (source, dest) = (
        p("/srv/fixture-media/downloads/sample.mkv"),
        p("/srv/fixture-media/library/sample.mkv"),
    );
    let (disk, backend) = scripted();
    backend.stat_is("/srv/fixture-media/downloads", Ok(folder(1, 50)));
    backend.stat_is("/srv/fixture-media/library", Ok(folder(1, 51)));
    backend.stat_is("/srv/fixture-media/downloads/sample.mkv", Ok(named(7, 2)));
    assert_eq!(
        disk.entry_alias_blocking(&source, &dest, &mark(named(7, 2))),
        EntryAlias::Distinct
    );
    assert_eq!(backend.calls_of("stat"), 2);
    assert_eq!(
        backend.calls_of("lstat") + backend.calls_of("canonicalize"),
        0
    );
}

/// In one folder, two spellings of a name are one entry only when they are
/// one file with no other name, as a volume that ignores case shows it; the
/// source is read once to tell.
#[test]
fn two_spellings_in_one_folder_are_one_entry_only_as_one_file_with_one_name() {
    let source = p("/srv/fixture-media/downloads/sample.mkv");
    let dest = p("/srv/fixture-media/downloads/SAMPLE.mkv");
    let answer = |source_file: RawStat, dest_file: RawStat| {
        let (disk, backend) = scripted();
        backend.stat_is("/srv/fixture-media/downloads", Ok(folder(1, 50)));
        backend.stat_is("/srv/fixture-media/downloads/sample.mkv", Ok(source_file));
        let alias = disk.entry_alias_blocking(&source, &dest, &mark(dest_file));
        assert_eq!(backend.paths_of("lstat"), [source.clone()]);
        assert_eq!(backend.calls_of("canonicalize"), 0);
        alias
    };
    assert_eq!(answer(named(7, 1), named(7, 1)), EntryAlias::Same);
    assert_eq!(answer(named(7, 1), named(8, 1)), EntryAlias::Distinct);
    assert_eq!(answer(named(7, 2), named(7, 2)), EntryAlias::Distinct);
}

/// Folders that cannot be told apart, as on a platform that gives no file
/// numbers, are resolved and the paths compared ignoring case, which fails
/// closed; a folder that does not resolve is taken as written.
#[test]
fn folders_without_numbers_are_compared_by_their_resolved_paths() {
    let source = p("/srv/fixture-media/downloads/sample.mkv");
    let (disk, backend) = scripted();
    backend.stat_is("/srv/fixture-media/downloads", Ok(dir()));
    backend.stat_is("/srv/fixture-media/library", Ok(dir()));
    let other = p("/srv/fixture-media/library/sample.mkv");
    assert_eq!(
        disk.entry_alias_blocking(&source, &other, &mark(named(7, 1))),
        EntryAlias::Distinct
    );
    assert_eq!(
        backend.paths_of("canonicalize"),
        [
            p("/srv/fixture-media/downloads"),
            p("/srv/fixture-media/library")
        ]
    );
    let other_case = p("/srv/fixture-media/downloads/SAMPLE.mkv");
    assert_eq!(
        disk.entry_alias_blocking(&source, &other_case, &mark(named(8, 1))),
        EntryAlias::Same
    );

    // Neither folder answers: the paths are compared as written.
    let (disk, backend) = scripted();
    backend.fails(
        "canonicalize",
        "/srv/fixture-media/downloads",
        io::ErrorKind::TimedOut,
    );
    backend.fails(
        "canonicalize",
        "/srv/fixture-media/library",
        io::ErrorKind::TimedOut,
    );
    assert_eq!(
        disk.entry_alias_blocking(&source, &other, &mark(named(7, 1))),
        EntryAlias::Distinct
    );
    assert_eq!(
        disk.entry_alias_blocking(&source, &source.clone(), &mark(named(7, 1))),
        EntryAlias::Same
    );
}

/// On a real volume: the source's own entry reached through a symlinked
/// folder, and a link to the source, are one entry with it; a second name
/// of the same file, a link to another file, and a second link to what a
/// linked source reaches, are not.
#[cfg(unix)]
#[test]
fn a_linked_folder_or_a_link_to_the_source_is_the_source_entry() {
    let temp = tempfile::tempdir().unwrap();
    let downloads = temp.path().join("downloads");
    let library = temp.path().join("library");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(&library).unwrap();
    let source = downloads.join("sample.mkv");
    let elsewhere = temp.path().join("elsewhere.mkv");
    std::fs::write(&source, b"payload").unwrap();
    std::fs::write(&elsewhere, b"payload").unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&downloads, &alias).unwrap();
    let (link, second, other) = (
        library.join("link.mkv"),
        library.join("second.mkv"),
        library.join("other.mkv"),
    );
    std::os::unix::fs::symlink(&source, &link).unwrap();
    std::fs::hard_link(&source, &second).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &other).unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));

    let read = |path: &Path| disk.fingerprint_blocking(path).unwrap();

    let through_alias = alias.join("sample.mkv");
    assert_eq!(
        disk.entry_alias_blocking(&source, &through_alias, &read(&through_alias)),
        EntryAlias::Same
    );
    assert_eq!(
        disk.entry_alias_blocking(&source, &link, &read(&link)),
        EntryAlias::Same
    );
    assert_eq!(
        disk.entry_alias_blocking(&source, &second, &read(&second)),
        EntryAlias::Distinct
    );
    assert_eq!(
        disk.entry_alias_blocking(&source, &other, &read(&other)),
        EntryAlias::Distinct
    );

    // A source that is itself a link is its own entry: a second link to
    // its target is another entry, and so is the target.
    let source_link = downloads.join("linked.mkv");
    std::os::unix::fs::symlink(&elsewhere, &source_link).unwrap();
    assert_eq!(
        disk.entry_alias_blocking(&source_link, &other, &read(&other)),
        EntryAlias::Distinct
    );
    assert_eq!(
        disk.entry_alias_blocking(&source_link, &elsewhere, &read(&elsewhere)),
        EntryAlias::Distinct
    );
    let through_alias = alias.join("linked.mkv");
    assert_eq!(
        disk.entry_alias_blocking(&source_link, &through_alias, &read(&through_alias)),
        EntryAlias::Same
    );
}
