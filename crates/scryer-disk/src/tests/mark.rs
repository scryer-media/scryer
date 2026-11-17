//! Marks: what each verb answers, what it could not tell, and how many
//! probes it spends, against the scripted backend and then on real files.

use super::*;
use crate::backend::Extra;

const A: &str = "/srv/fixture-media/Fixture Show/a.mkv";
const B: &str = "/srv/fixture-media/Fixture Show/b.mkv";

fn numbered(device: Option<u64>, inode: Option<u64>) -> RawStat {
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
    Mark::of_stat(&stat, None)
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
    assert_eq!(with(|stat| stat.extra.owner = Some(501)), Drift::Touched);
    assert_eq!(with(|stat| stat.links = Some(2)), Drift::Same);
    // Where the platform cannot tell the files apart, equal facts are
    // reported as only that.
    let unnumbered = mark(numbered(Some(7), Some(0)));
    assert_eq!(unnumbered.compare(&unnumbered), Drift::SameFacts);
    assert!(Drift::SameFacts.is_untouched() && Drift::Same.is_untouched());
    assert!(Drift::Touched.is_unwritten() && !Drift::Touched.is_untouched());
    for unshown in [
        Drift::Rewritten,
        Drift::Replaced,
        Drift::Gone,
        Drift::CannotTell,
    ] {
        assert!(!unshown.is_unwritten() && !unshown.is_untouched());
    }
}

#[test]
fn a_path_is_checked_against_its_mark_with_one_probe() {
    let (disk, backend) = scripted();
    backend.stat_is(A, Ok(file(5)));
    let taken = disk.fingerprint_blocking(&p(A)).unwrap();
    assert_eq!(disk.still_matches_blocking(&taken, &p(A)), Drift::Same);
    backend.stat_is(A, Ok(file(6)));
    assert_eq!(disk.still_matches_blocking(&taken, &p(A)), Drift::Replaced);
    backend.stat_is(A, Err(io::ErrorKind::NotFound));
    assert_eq!(disk.still_matches_blocking(&taken, &p(A)), Drift::Gone);
    backend.stat_is(A, Err(io::ErrorKind::PermissionDenied));
    assert_eq!(
        disk.still_matches_blocking(&taken, &p(A)),
        Drift::CannotTell
    );
    assert_eq!(backend.calls_of("lstat"), 5);
    assert_eq!(backend.calls_of("stat"), 0);
}

#[test]
fn two_paths_are_compared_with_one_probe_each() {
    let (disk, backend) = scripted();
    backend.stat_is(A, Ok(file(5)));
    backend.stat_is(B, Ok(file(5)));
    assert_eq!(disk.same_file_blocking(&p(A), &p(B)), Sameness::Same);
    assert_eq!(backend.paths_of("lstat"), [p(A), p(B)]);
    backend.stat_is(B, Ok(file(6)));
    assert_eq!(disk.same_file_blocking(&p(A), &p(B)), Sameness::Different);
    backend.stat_is(A, Err(io::ErrorKind::NotFound));
    assert_eq!(disk.same_file_blocking(&p(A), &p(B)), Sameness::CannotTell);
    assert_eq!(backend.calls_of("lstat"), 5, "the second path is not read");
}

#[test]
fn other_names_are_reported_only_from_a_link_count() {
    let links = |links| mark(RawStat { links, ..file(5) }).other_links();
    assert_eq!(links(Some(1)), Links::Sole);
    assert_eq!(links(Some(2)), Links::Shared);
    assert_eq!(links(None), Links::CannotTell);
    assert_eq!(mark(file(5)).volume().other_links(), Links::CannotTell);
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
        Mark::of_stat(&file(5), Some((42, 99))),
        mark(file(5)).volume(),
        Mark::of_stat(&file(5), Some((42, 99))).volume(),
        mark(dir()).volume(),
    ];
    for taken in &marks {
        let stored = taken.to_stored();
        let read = Mark::from_stored(&stored).unwrap_or_else(|| panic!("{stored}"));
        assert_eq!(read.to_stored(), stored);
        assert_eq!(format!("{read:?}"), format!("{taken:?}"));
    }
    assert_eq!(marks[3].to_stored(), "unix-dev:1");
    assert_eq!(marks[4].to_stored(), "windows-volume:42");
    assert_eq!(marks[5].to_stored(), "unknown-volume");
    for unread in [
        "",
        "mark1",
        "mark2:f:5:-:-:-:u1:105:1:-",
        "mark1:x:5:-:-:-:u1:105:1:-",
        "mark1:f:5:-:-:-:q1:105:1:-",
        "mark1:f:5:-:-:-:u1:105:1:-:extra",
        "unix-dev:",
        "unix-dev:seven",
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
    let taken = Mark::of_stat(&file(5), None);
    assert_eq!(disk.still_matches(&taken, A).await, Drift::CannotTell);
    open.send(()).unwrap();
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
    let alone = disk.fingerprint_blocking(&first).unwrap();
    assert_eq!(alone.other_links(), Links::Sole);
    std::fs::hard_link(&first, &second).unwrap();
    assert_eq!(disk.same_file_blocking(&first, &second), Sameness::Same);
    assert_eq!(disk.same_file_blocking(&first, &other), Sameness::Different);
    let linked = disk.fingerprint_blocking(&second).unwrap();
    assert_eq!(linked.other_links(), Links::Shared);
    assert_eq!(alone.same_volume(&linked), Sameness::Same);

    // An open handle's own metadata gives the mark the path gives.
    let handle = std::fs::File::open(&first).unwrap();
    let opened = Mark::of(&handle.metadata().unwrap());
    assert_eq!(disk.still_matches_blocking(&opened, &second), Drift::Same);
    assert_eq!(
        disk.still_matches_blocking(&opened, &other),
        Drift::Replaced
    );
    std::fs::write(&first, b"synthetic, rewritten").unwrap();
    assert_eq!(
        disk.still_matches_blocking(&opened, &first),
        Drift::Rewritten
    );
    std::fs::remove_file(&first).unwrap();
    assert_eq!(disk.still_matches_blocking(&opened, &first), Drift::Gone);
}

/// A path's own stat carries no file index on Windows; the identified read
/// asks for it, and then two names of one file are told as one.
#[cfg(windows)]
#[test]
fn windows_tells_files_apart_only_from_an_identified_read() {
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
    assert_eq!(
        plain(&first).same_file(&plain(&second)),
        Sameness::CannotTell
    );
    assert_eq!(plain(&first).compare(&plain(&first)), Drift::SameFacts);
    assert_eq!(disk.same_file_blocking(&first, &second), Sameness::Same);
    assert_eq!(disk.same_file_blocking(&first, &other), Sameness::Different);
    let identified = disk.fingerprint_identified_blocking(&first).unwrap();
    assert_eq!(
        identified.volume().to_stored(),
        crate::copy::volume_key(&first)
    );
}
