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

/// `stat` as Windows reads it: on the volume `serial` names, or on none,
/// with its index as the file's number.
pub(super) fn on_serial(stat: RawStat, serial: Option<u64>) -> RawStat {
    RawStat {
        identity: serial.and_then(|serial| FileIdentity::from_raw(serial, stat.extra.inode?)),
        extra: Extra {
            device: None,
            serial,
            ..stat.extra
        },
        ..stat
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
            ..Default::default()
        },
    };
    let marks = [
        mark(full),
        mark(dir()),
        Mark::from_stored("mark1:f:5:-:-:-:w42:-:-").expect("a Windows mark"),
        mark(file(5)).volume(),
        Mark::from_stored("windows-volume:42").expect("a Windows volume"),
        mark(dir()).volume(),
        mark(on_serial(file(5), Some(42))),
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
    assert_eq!(marks[6].to_stored(), "mark1:f:5:-:-:-:w42:105:-");
    // A Windows mark stored before file indexes were read carries none, so
    // against a fresh read of the same file it cannot tell, and its facts
    // still match.
    assert_eq!(marks[2].same_file(&marks[6]), Sameness::CannotTell);
    assert_eq!(marks[2].compare(&marks[6]), Drift::SameFacts);
    assert!(marks[2].still_marks(&marks[6]));
    for unread in [
        "",
        "mark1",
        "mark2:f:5:-:-:-:u1:105:-",
        "mark1:x:5:-:-:-:u1:105:-",
        "mark1:f:5:-:-:-:q1:105:-",
        "mark1:f:5:-:-:-:w+42:99:-",
        "mark1:f:5:-:-:-:w42:099:-",
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

/// A path's own stat on Windows names its volume and, on a volume that
/// keeps them, its file number, from one handle: two names of one file are
/// one file, and the volume a path lands on is the one its nearest folder
/// reads as. std's own metadata names neither.
#[cfg(windows)]
#[test]
fn windows_reads_a_volume_and_a_file_number_from_one_handle() {
    let temp = tempfile::tempdir().unwrap();
    let (first, second, other) = (
        temp.path().join("first.mkv"),
        temp.path().join("second.mkv"),
        temp.path().join("other.mkv"),
    );
    std::fs::write(&first, b"synthetic").unwrap();
    std::fs::write(&other, b"synthetic").unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let plain = |path: &Path| disk.fingerprint_blocking(path).unwrap();
    assert_eq!(plain(&first).same_file(&plain(&second)), Sameness::Same);
    assert_eq!(plain(&first).same_file(&plain(&other)), Sameness::Different);
    assert_eq!(plain(&first).same_volume(&plain(&other)), Sameness::Same);
    assert_eq!(plain(&first).compare(&plain(&first)), Drift::Same);
    assert!(plain(&first).has_other_names() && !plain(&other).has_other_names());
    assert!(plain(&first).to_stored().contains(":w"));

    let opened = Mark::of(&std::fs::File::open(&first).unwrap().metadata().unwrap());
    assert_eq!(opened.same_file(&plain(&first)), Sameness::CannotTell);
    assert_eq!(opened.compare(&plain(&first)), Drift::SameFacts);

    let volume = disk.landing_volume_blocking(&first).expect("volume");
    let missing = temp.path().join("not-created/third.mkv");
    let landing = disk.landing_volume_blocking(&missing).expect("volume");
    assert_eq!(volume.same_volume(&landing), Sameness::Same);
    assert!(volume.to_stored().starts_with("windows-volume:"));
    assert_eq!(volume.to_stored(), disk.copy_lane(&missing));
}

/// On Windows two paths share a volume only when they name one drive or
/// share and both read as one known volume serial. A serial either side
/// does not answer is no answer. A source that is a link is judged by the
/// volume holding the link, as on unix.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_volumes_are_shared_only_with_one_prefix_and_one_known_serial() {
    let folder = "/srv/fixture-media/Fixture Show";
    let setup = |source: Option<u64>, landing: Option<u64>| {
        let (disk, backend) = scripted();
        backend.stat_is(B, Ok(on_serial(file(5), source)));
        backend.stat_is(folder, Ok(on_serial(dir(), landing)));
        (disk, backend)
    };

    let (equal, backend) = setup(Some(7), Some(7));
    assert!(equal.shares_volume(&p(B), &p(A)).await);
    // The source's own read names its volume: one stat, of the folder the
    // destination lands in, besides the reads that find it.
    assert_eq!(backend.paths_of("stat"), [p(folder)]);
    assert_eq!(backend.paths_of("lstat")[0], p(B));

    let (different, _) = setup(Some(7), Some(8));
    assert!(!different.shares_volume(&p(B), &p(A)).await);

    let (source_unread, _) = setup(None, Some(7));
    assert!(!source_unread.shares_volume(&p(B), &p(A)).await);

    let (landing_unread, _) = setup(Some(7), None);
    assert!(!landing_unread.shares_volume(&p(B), &p(A)).await);

    let (linked, backend) = setup(Some(7), Some(7));
    backend.stat_is(
        B,
        Ok(on_serial(
            RawStat {
                kind: EntryKind::Symlink,
                ..file(5)
            },
            Some(7),
        )),
    );
    assert!(linked.shares_volume(&p(B), &p(A)).await);

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
        backend.stat_is(drive, Ok(on_serial(file(5), Some(7))));
        backend.stat_is(other_folder, Ok(on_serial(dir(), Some(7))));
        assert_eq!(
            disk.shares_volume(&p(drive), &p(other)).await,
            shared,
            "{other}"
        );
        if !shared {
            assert_eq!(backend.calls_of("stat"), 0, "{other}");
            assert_eq!(backend.calls_of("lstat"), 0, "{other}");
        }
    }
    let share = r"\\fixture-host\media\b.mkv";
    let (disk, backend) = scripted();
    backend.stat_is(share, Ok(on_serial(file(5), Some(7))));
    backend.stat_is(r"\\fixture-host\other", Ok(on_serial(dir(), Some(7))));
    assert!(
        !disk
            .shares_volume(&p(share), &p(r"\\fixture-host\other\a.mkv"))
            .await
    );
}

/// Two names on one drive, the destination's folder not made yet, are on one
/// volume; a source that is not there answers no.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_two_names_on_one_drive_share_a_volume() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.mkv");
    std::fs::write(&source, b"synthetic").unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let beside = temp.path().join("beside.mkv");
    assert!(disk.shares_volume(&source, &beside).await);
    let nested = temp.path().join("not-created/yet/destination.mkv");
    assert!(disk.shares_volume(&source, &nested).await);
    let missing = temp.path().join("missing.mkv");
    assert!(!disk.shares_volume(&missing, &nested).await);
}

/// A file with `links` names, numbered on its volume.
fn named(inode: u64, links: u64) -> RawStat {
    RawStat {
        links: Some(links),
        ..numbered(Some(1), Some(inode))
    }
}

const SOURCE: &str = "/srv/fixture-media/downloads/sample.mkv";

/// Paths that are one as written are one entry and read nothing; a
/// destination that shows another name reads nothing and is not judged here.
#[test]
fn the_early_look_reads_nothing_for_matching_paths_or_a_named_destination() {
    let source = p(SOURCE);
    let (disk, backend) = scripted();
    assert_eq!(
        disk.entry_alias_blocking(&source, &p(SOURCE), &mark(named(7, 1))),
        EntryAlias::Same
    );
    let second_name = p("/srv/fixture-media/library/second.mkv");
    assert_eq!(
        disk.entry_alias_blocking(&source, &second_name, &mark(named(7, 2))),
        EntryAlias::Distinct
    );
    assert!(backend.calls.lock().unwrap().is_empty());
}

/// Otherwise the source is read once, links not followed: one file with one
/// name is one entry, another file is not, and a read that fails cannot tell.
/// No folder is read or resolved, and no link followed.
#[test]
fn the_early_look_reads_the_source_once() {
    let source = p(SOURCE);
    let dest = p("/srv/fixture-media/elsewhere/SAMPLE~1.MKV");
    let answer = |source_read: Result<RawStat, io::ErrorKind>, dest_read: RawStat| {
        let (disk, backend) = scripted();
        backend.stat_is(SOURCE, source_read);
        let alias = disk.entry_alias_blocking(&source, &dest, &mark(dest_read));
        assert_eq!(backend.paths_of("lstat"), [source.clone()]);
        assert_eq!(backend.calls.lock().unwrap().len(), 1);
        alias
    };
    assert_eq!(answer(Ok(named(7, 1)), named(7, 1)), EntryAlias::Same);
    assert_eq!(answer(Ok(named(7, 1)), named(8, 1)), EntryAlias::Distinct);
    assert_eq!(
        answer(Err(io::ErrorKind::TimedOut), named(8, 1)),
        EntryAlias::CannotTell
    );
}

/// What the early look no longer refuses: two files whose names differ only
/// in case in one folder, two files a share numbers alike, a folder that
/// cannot be resolved, and a destination link. Each is left to the set-aside
/// removal, which asks the filesystem.
#[test]
fn the_early_look_leaves_separate_files_to_the_set_aside() {
    let source = p(SOURCE);
    let other_case = p("/srv/fixture-media/downloads/Sample.mkv");
    let (disk, backend) = scripted();
    backend.stat_is(SOURCE, Ok(named(7, 1)));
    backend.fails(
        "canonicalize",
        "/srv/fixture-media/downloads",
        io::ErrorKind::TimedOut,
    );
    let mut link = named(9, 1);
    link.kind = EntryKind::Symlink;
    let separate = p("/srv/fixture-media/other/sample.mkv");
    for (dest, read) in [
        (&other_case, named(8, 1)),
        (&separate, numbered(Some(1), Some(0))),
        (&separate, numbered(None, None)),
        (&separate, link),
    ] {
        assert_eq!(
            disk.entry_alias_blocking(&source, dest, &mark(read)),
            EntryAlias::Distinct,
            "{}",
            dest.display()
        );
    }
    assert_eq!(backend.calls_of("lstat"), 4);
    assert_eq!(
        backend.calls_of("stat") + backend.calls_of("canonicalize") + backend.calls_of("read_link"),
        0
    );
}

/// A Windows read carries its volume serial and, where the volume keeps
/// it, its file index: two reads of one index are one file, another index
/// is another file, and an index the volume does not keep tells nothing.
#[test]
fn a_windows_read_tells_files_apart_only_by_a_kept_index() {
    let read = |serial, inode, unnumbered| {
        let mut stat = on_serial(numbered(None, Some(inode)), Some(serial));
        stat.extra.unnumbered = unnumbered;
        mark(stat)
    };
    assert_eq!(
        read(7, 11, false).same_file(&read(7, 11, false)),
        Sameness::Same
    );
    assert_eq!(
        read(7, 11, false).same_file(&read(7, 12, false)),
        Sameness::Different
    );
    assert_eq!(
        read(7, 11, false).same_file(&read(8, 11, false)),
        Sameness::Different
    );
    assert_eq!(
        read(7, 11, true).same_file(&read(7, 11, true)),
        Sameness::CannotTell
    );
    assert_eq!(
        read(7, 11, true).same_file(&read(7, 12, false)),
        Sameness::CannotTell
    );
    assert_eq!(
        read(7, 11, true).same_volume(&read(7, 12, true)),
        Sameness::Same
    );
    assert_eq!(read(7, 11, true).to_stored(), "mark1:f:5:-:-:-:w7:-:-");
    assert_eq!(read(7, 11, false).label(), "volume=7 index=11");
    let unnamed = mark(on_serial(numbered(None, Some(11)), None));
    assert_eq!(unnamed.same_file(&read(7, 11, false)), Sameness::CannotTell);
    assert!(!unnamed.names_volume());
}
