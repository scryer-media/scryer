//! Removing a move's source beside an adopted destination: the source is set
//! aside, the destination read, and only then is anything removed. Against
//! the scripted backend, where a destination that is the source's own entry
//! reads as gone once the source is aside, and then on real files.

use super::mark::numbered;
use super::*;

const SOURCE: &str = "/srv/fixture-media/downloads/sample.mkv";

fn scripted() -> (Disk, Arc<Scripted>) {
    owner(&ManualClock::at(at(0)))
}

fn mark(stat: RawStat) -> Mark {
    Mark::of_stat(&stat)
}

/// A file numbered `inode` with `links` names.
fn named(inode: u64, links: u64) -> RawStat {
    RawStat {
        links: Some(links),
        ..numbered(Some(1), Some(inode))
    }
}

#[cfg(unix)]
fn link(inode: u64) -> RawStat {
    RawStat {
        kind: EntryKind::Symlink,
        ..named(inode, 1)
    }
}

fn aside() -> PathBuf {
    aside_path_for(&p(SOURCE))
}

/// The set-aside name is one hidden name in the source's own folder, the same
/// for the same source, with no media extension and nothing a cleanup pass
/// removes by name.
#[test]
fn the_set_aside_name_is_hidden_beside_the_source() {
    let aside = aside();
    assert_eq!(aside.parent(), p(SOURCE).parent());
    assert_eq!(aside, aside_path_for(&p(SOURCE)));
    let name = aside.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with(".scry-ia-"), "{name}");
    assert_eq!(name.len(), ".scry-ia-".len() + 9);
    assert!(aside.extension().is_none());
    assert!(!name.contains("partial") && !name.contains("tmp"));
}

/// The read of the source the caller proved: file 7 on volume 1.
fn source_mark() -> Mark {
    mark(named(7, 1))
}

/// File `inode` on volume `device`, with `links` names.
fn on_volume(device: u64, inode: u64, links: Option<u64>) -> RawStat {
    RawStat {
        links,
        ..numbered(Some(device), Some(inode))
    }
}

/// A destination that reads missing once the source is aside is the source's
/// own entry, whatever its path spells: the source goes back and nothing is
/// removed. Two renames, one read, no listing.
#[test]
fn a_destination_that_reads_missing_once_the_source_is_aside_puts_it_back() {
    let dests = [
        (
            "/srv/fixture-media/mount-b/downloads/sample.mkv",
            named(7, 1),
        ),
        ("/srv/fixture-media/downloads/SAMPLE.mkv", named(7, 1)),
        (
            "/srv/fixture-media/downloads/sa\u{0301}mple.mkv",
            named(7, 1),
        ),
        ("/srv/fixture-media/share/sample.mkv", named(60, 1)),
        ("/srv/fixture-media/share/sample.mkv", numbered(None, None)),
    ];
    for (dest, kept) in dests {
        let (disk, backend) = scripted();
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(
            matches!(answer, AsideRemoval::SameEntry),
            "{dest}: {answer:?}"
        );
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
        assert_eq!(backend.paths_of("lstat"), [p(dest)]);
        assert_eq!(backend.calls_of("stat") + backend.calls_of("list"), 0);
        assert_eq!(backend.calls_of("remove_file"), 0, "{dest}");
    }
}

/// On the source's own volume, a destination that still reads as the file it
/// was adopted as lets the set-aside name go: a second name of the source's
/// file, or another file number. No listing: one rename, one read, one
/// removal.
#[test]
fn on_the_sources_volume_a_destination_still_read_as_adopted_lets_the_set_aside_go() {
    let dests = [
        ("/srv/fixture-media/library/second-name.mkv", named(7, 2)),
        ("/srv/fixture-media/library/sample.mkv", named(8, 1)),
        ("/srv/fixture-media/share/sample.mkv", named(60, 1)),
    ];
    for (dest, kept) in dests {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(
            matches!(answer, AsideRemoval::Removed),
            "{dest}: {answer:?}"
        );
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
        assert_eq!(backend.paths_of("lstat"), [p(dest)]);
        assert_eq!(backend.calls_of("list"), 0);
        assert_eq!(backend.paths_of("remove_file"), [aside()]);
    }
}

/// A destination that shows the source's own nonzero file number and length
/// and no other name, on the source's own volume, is the set-aside file: the
/// source goes back. On another volume the same number cannot tell whether it is a second view of one folder or another
/// volume's file numbered alike, and the source goes back too. No listing
/// either way. With another name it is a second link, judged as any other
/// destination off the source's volume.
#[test]
fn a_destination_with_the_sources_number_and_no_other_name_keeps_the_source() {
    let dest = "/srv/fixture-media/mount-b/downloads/sample.mkv";
    let kepts = [
        (on_volume(1, 7, Some(1)), true),
        (on_volume(2, 7, Some(1)), false),
        (on_volume(2, 7, None), false),
    ];
    for (kept, one_volume) in kepts {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        if one_volume {
            assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
        } else {
            assert!(matches!(answer, AsideRemoval::CannotTell), "{answer:?}");
        }
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
        assert_eq!(
            backend.calls_of("list") + backend.calls_of("remove_file"),
            0
        );
    }

    let kept = on_volume(2, 7, Some(2));
    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(kept));
    backend.lists(
        "/srv/fixture-media/mount-b/downloads",
        &[("sample.mkv", None)],
    );
    let answer =
        disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert_eq!(backend.paths_of("remove_file"), [aside()]);
}

/// On the source's own volume, a destination whose read carries no file
/// number, or a zero one, cannot be told from the set-aside file by number,
/// so its folder is listed once as off the volume: listed by its name, the
/// set-aside name goes; not listed, the source goes back.
#[test]
fn on_the_sources_volume_an_unnumbered_destination_must_be_listed_in_its_folder() {
    let folder = "/srv/fixture-media/share";
    let dest = "/srv/fixture-media/share/sample.mkv";
    for kept in [on_volume(1, 0, Some(1)), numbered(Some(1), None)] {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        backend.lists(folder, &[("sample.mkv", None)]);
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
        assert_eq!(backend.paths_of("list"), [p(folder)]);
        assert_eq!(backend.paths_of("remove_file"), [aside()]);

        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        backend.lists(folder, &[("other.mkv", None)]);
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(matches!(answer, AsideRemoval::CannotTell), "{answer:?}");
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
}

/// Off the source's known volume, the destination's folder is listed once
/// and must hold its name byte for byte before the set-aside name goes. A
/// name the listing lacks, another spelling of it included, cannot tell
/// whether it is the source's entry or the same name in another case; a
/// listing that fails is unread. Either way the source goes back.
#[test]
fn off_the_sources_volume_the_destination_must_be_listed_in_its_folder() {
    let folder = "/srv/fixture-media/share";
    let dest = "/srv/fixture-media/share/sample.mkv";
    for kept in [on_volume(2, 9, Some(1)), numbered(None, None)] {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        backend.lists(folder, &[("other.mkv", None), ("sample.mkv", None)]);
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
        assert_eq!(backend.paths_of("list"), [p(folder)]);
        assert_eq!(backend.paths_of("remove_file"), [aside()]);

        for listed in [&[("other.mkv", None)][..], &[("SAMPLE.mkv", None)][..]] {
            let (disk, backend) = scripted();
            backend.stat_is(dest, Ok(kept));
            backend.lists(folder, listed);
            let answer = disk.remove_source_beside_blocking(
                &p(SOURCE),
                &source_mark(),
                &p(dest),
                &mark(kept),
            );
            assert!(matches!(answer, AsideRemoval::CannotTell), "{answer:?}");
            assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
            assert_eq!(backend.calls_of("remove_file"), 0);
        }

        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(
            matches!(
                answer,
                AsideRemoval::DestinationUnread(PathReading::Unreachable { .. })
            ),
            "{answer:?}"
        );
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
}

/// A link at the destination that reached the source dangles once the source
/// is aside; it is the source's entry, and the source goes back. A link that
/// still reaches something lets the source go. A link adds one followed read.
#[cfg(unix)]
#[test]
fn a_destination_link_to_the_source_puts_the_source_back() {
    let dest = p("/srv/fixture-media/library/link.mkv");

    let (disk, backend) = scripted();
    backend.stats_in_turn(&dest, &[Ok(link(9)), Err(io::ErrorKind::NotFound)]);
    let answer =
        disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &dest, &mark(link(9)));
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(backend.paths_of("lstat"), [dest.clone()]);
    assert_eq!(backend.paths_of("stat"), [dest.clone()]);
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stats_in_turn(&dest, &[Ok(link(9)), Ok(named(8, 1))]);
    let answer =
        disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &dest, &mark(link(9)));
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert_eq!(backend.paths_of("remove_file"), [aside()]);
}

/// A destination that holds another file, or does not read, once the source
/// is aside removes nothing and puts the source back.
#[test]
fn a_changed_or_unread_destination_puts_the_source_back() {
    let dest = "/srv/fixture-media/library/sample.mkv";

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(9, 1)));
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(
        matches!(answer, AsideRemoval::DestinationChanged),
        "{answer:?}"
    );
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stat_is(dest, Err(io::ErrorKind::TimedOut));
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(
        matches!(answer, AsideRemoval::DestinationUnread(_)),
        "{answer:?}"
    );
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A set-aside rename that is refused leaves the source alone and reads
/// nothing. A set-aside name that is taken, by a file an interrupted attempt
/// left or anything else, is read once and, unless it is an empty file, named
/// in its own field.
#[test]
fn a_refused_set_aside_leaves_the_source_alone_and_names_a_taken_name() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let (disk, backend) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    match answer {
        AsideRemoval::AsideNameTaken { at, error } => {
            assert_eq!(at, aside());
            assert_eq!(error.reason(), reason::DISK_DESTINATION_EXISTS);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
    // A taken name is read once, to tell an empty leftover from a file.
    assert_eq!(backend.paths_of("lstat"), [aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::PermissionDenied);
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(
        backend.calls_of("lstat") + backend.calls_of("remove_file"),
        0
    );
}

/// Where only a claimed rename is offered, a failure that left something at
/// the set-aside name may have moved the source there, or left only its own
/// reservation. The set-aside name is read once, and the source put back
/// when that name holds the source's file: a reservation, an empty file, or
/// nothing there is no source that moved, so nothing is put back and no
/// failed put-back is reported. A nonempty file under another number is put
/// back only when the source then reads missing: the volume numbered the
/// moved file anew.
#[test]
fn a_claimed_set_aside_that_left_a_name_puts_back_only_the_sources_own_file() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let aside = aside();
    let claimed = || {
        let (disk, backend) = scripted();
        backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
        *backend.claim_left.lock().unwrap() = true;
        backend.fails("rename_claimed", SOURCE, io::ErrorKind::TimedOut);
        (disk, backend)
    };

    let reservation = RawStat {
        len: 0,
        ..named(99, 1)
    };
    for at_aside in [None, Some(reservation)] {
        let (disk, backend) = claimed();
        backend.stat_is(SOURCE, Err(io::ErrorKind::TimedOut));
        if let Some(stat) = at_aside {
            backend.stat_is(aside.to_str().unwrap(), Ok(stat));
        }
        let answer = disk.remove_source_beside_blocking(
            &p(SOURCE),
            &source_mark(),
            &p(dest),
            &mark(named(8, 1)),
        );
        assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
        assert_eq!(backend.paths_of("rename_claimed"), [p(SOURCE)]);
        assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
    }

    let (disk, backend) = claimed();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(
        backend.paths_of("rename_claimed"),
        [p(SOURCE), aside.clone()]
    );
    assert_eq!(backend.calls_of("remove_file"), 0);

    // Renumbered at the set-aside name: put back only once the source reads
    // missing, and an empty reservation never, whatever the source reads.
    for (source_read, put_back) in [
        (Err(io::ErrorKind::NotFound), true),
        (Ok(named(7, 1)), false),
        (Err(io::ErrorKind::TimedOut), false),
    ] {
        let (disk, backend) = claimed();
        backend.stat_is(aside.to_str().unwrap(), Ok(named(41, 1)));
        backend.stat_is(SOURCE, source_read);
        let answer = disk.remove_source_beside_blocking(
            &p(SOURCE),
            &source_mark(),
            &p(dest),
            &mark(named(8, 1)),
        );
        assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
        assert_eq!(backend.paths_of("lstat"), [aside.clone(), p(SOURCE)]);
        let renamed: &[PathBuf] = if put_back {
            &[p(SOURCE), aside.clone()]
        } else {
            &[p(SOURCE)]
        };
        assert_eq!(backend.paths_of("rename_claimed"), renamed);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
    let (disk, backend) = claimed();
    backend.stat_is(aside.to_str().unwrap(), Ok(reservation));
    backend.stat_is(SOURCE, Err(io::ErrorKind::NotFound));
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
    assert_eq!(backend.paths_of("rename_claimed"), [p(SOURCE)]);
}

/// A put-back that keeps failing says where the source's file is: at the
/// set-aside name. A removal of the set-aside name that fails reads that
/// name: while the file is still there the source is put back; a name that
/// reads gone, with the source's name also missing, was removed; and with
/// the source's name holding a file again, something put it back and the
/// removal is unconfirmed.
#[test]
fn a_failed_put_back_or_removal_says_where_the_file_is() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let aside = aside();

    let (disk, backend) = scripted();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "rename_no_replace",
        aside.to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    match answer {
        AsideRemoval::PutBackFailed { at, .. } => assert_eq!(at, aside),
        other => panic!("{other:?}"),
    }
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "remove_file",
        aside.to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(matches!(answer, AsideRemoval::NotRemoved(_)), "{answer:?}");
    assert_eq!(
        backend.paths_of("rename_no_replace"),
        [p(SOURCE), aside.clone()]
    );

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "remove_file",
        aside.to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    backend.fails(
        "rename_no_replace",
        aside.to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    match answer {
        AsideRemoval::PutBackFailed { at, .. } => assert_eq!(at, aside),
        other => panic!("{other:?}"),
    }

    for put_back_by_another in [false, true] {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(named(8, 1)));
        if put_back_by_another {
            backend.stats_in_turn(&p(SOURCE), &[Ok(named(7, 1))]);
        }
        backend.fails(
            "remove_file",
            aside.to_str().unwrap(),
            io::ErrorKind::TimedOut,
        );
        let answer = disk.remove_source_beside_blocking(
            &p(SOURCE),
            &source_mark(),
            &p(dest),
            &mark(named(8, 1)),
        );
        match put_back_by_another {
            true => assert!(
                matches!(answer, AsideRemoval::RemovalUnconfirmed(_)),
                "{answer:?}"
            ),
            false => assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}"),
        }
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
        assert_eq!(
            backend.paths_of("lstat"),
            [p(dest), aside.clone(), p(SOURCE)]
        );
    }
}

/// A nonempty file an interrupted cleanup left at the set-aside name, with
/// the source's name missing, is put back by a rename that replaces nothing.
/// An empty file there is a reservation, never a source: it is removed, on a
/// fresh read just before, whether the source's name is missing or taken.
/// Nothing at that name or a folder there moves nothing. A source name taken
/// again keeps both, and the nonempty file is named for the operator.
#[test]
fn a_set_aside_source_left_by_an_interruption_is_put_back() {
    let aside = aside();
    let empty = RawStat {
        len: 0,
        ..named(7, 1)
    };

    let (disk, backend) = scripted();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
    assert!(matches!(answer, PutBackAside::PutBack), "{answer:?}");
    assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
    assert_eq!(backend.paths_of("rename_no_replace"), [aside.clone()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let folder = RawStat {
        kind: EntryKind::Directory,
        ..named(7, 1)
    };
    for at_aside in [None, Some(folder)] {
        let (disk, backend) = scripted();
        if let Some(stat) = at_aside {
            backend.stat_is(aside.to_str().unwrap(), Ok(stat));
        }
        let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
        assert!(matches!(answer, PutBackAside::Nothing), "{answer:?}");
        assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
        assert_eq!(backend.calls_of("rename_no_replace"), 0);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }

    for source_taken in [false, true] {
        let (disk, backend) = scripted();
        backend.stat_is(aside.to_str().unwrap(), Ok(empty));
        if source_taken {
            backend.stat_is(SOURCE, Ok(named(8, 1)));
        }
        let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
        assert!(matches!(answer, PutBackAside::Nothing), "{answer:?}");
        assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
        assert_eq!(backend.paths_of("remove_file"), [aside.clone()]);
        assert_eq!(backend.calls_of("rename_no_replace"), 0);
    }

    let (disk, backend) = scripted();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "rename_no_replace",
        aside.to_str().unwrap(),
        io::ErrorKind::AlreadyExists,
    );
    let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
    match answer {
        PutBackAside::SourceNameTaken { at } => assert_eq!(at, aside),
        other => panic!("{other:?}"),
    }
    for operation in ["remove_file", "rename_claimed", "rename"] {
        assert_eq!(backend.calls_of(operation), 0, "{operation}");
    }
}

/// A set-aside name taken by an empty file while the source is at its own
/// name holds no data: it is removed, the source is set aside after all, and
/// the cleanup goes on. A nonempty file there is never touched: the source
/// stays, and the set-aside name is named.
#[test]
fn an_empty_file_at_the_set_aside_name_is_cleared_and_a_nonempty_one_kept() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let aside = aside();
    let empty = RawStat {
        len: 0,
        ..named(9, 1)
    };

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
    backend.stats_in_turn(&aside, &[Ok(empty)]);
    backend.fails_times("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists, 1);
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert_eq!(
        backend.paths_of("rename_no_replace"),
        [p(SOURCE), p(SOURCE)]
    );
    assert_eq!(
        backend.paths_of("remove_file"),
        [aside.clone(), aside.clone()]
    );

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
    backend.stat_is(aside.to_str().unwrap(), Ok(named(9, 1)));
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(dest),
        &mark(named(8, 1)),
    );
    match answer {
        AsideRemoval::AsideNameTaken { at, .. } => assert_eq!(at, aside),
        other => panic!("{other:?}"),
    }
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A destination that is the source's own entry, so the set-aside source
/// goes back; a put-back that fails `failures` times first.
fn put_back_failing(failures: Option<usize>) -> (Disk, Arc<Scripted>) {
    let (disk, backend) = scripted();
    let at = aside();
    let at = at.to_str().unwrap();
    match failures {
        Some(times) => backend.fails_times(
            "rename_no_replace",
            at,
            io::ErrorKind::PermissionDenied,
            times,
        ),
        None => backend.fails("rename_no_replace", at, io::ErrorKind::PermissionDenied),
    }
    (disk, backend)
}

const SAME_ENTRY: &str = "/srv/fixture-media/mount-b/downloads/sample.mkv";

/// A put-back that fails is retried after a pause that holds no thread: an
/// early retry that succeeds answers what the removal found, after only the
/// pauses it waited.
#[tokio::test(start_paused = true)]
async fn a_put_back_that_succeeds_on_an_early_retry_answers_what_was_found() {
    let (disk, backend) = put_back_failing(Some(3));
    backend.stat_is(aside().to_str().unwrap(), Ok(named(7, 1)));
    let started = tokio::time::Instant::now();
    let answer = disk
        .remove_source_beside(
            &p(SOURCE),
            &source_mark(),
            &p(SAME_ENTRY),
            &mark(named(7, 1)),
        )
        .await;
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(
        backend.paths_of("rename_no_replace"),
        [p(SOURCE), aside(), aside(), aside(), aside()]
    );
    assert_eq!(
        started.elapsed(),
        PUT_BACK_RETRY_PAUSES[..3]
            .iter()
            .sum::<std::time::Duration>()
    );
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A put-back that fails every retry stops after thirty seconds and twelve
/// renames, and leaves the file at its set-aside name untouched. The twin
/// for a thread of its own waits the same thirty seconds on its clock.
#[tokio::test(start_paused = true)]
async fn a_put_back_that_keeps_failing_leaves_the_file_at_its_set_aside_name() {
    let (disk, backend) = put_back_failing(None);
    backend.stat_is(aside().to_str().unwrap(), Ok(named(7, 1)));
    let started = tokio::time::Instant::now();
    let answer = disk
        .remove_source_beside(
            &p(SOURCE),
            &source_mark(),
            &p(SAME_ENTRY),
            &mark(named(7, 1)),
        )
        .await;
    match answer {
        AsideRemoval::PutBackFailed { at, .. } => assert_eq!(at, aside()),
        other => panic!("{other:?}"),
    }
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(30));
    assert_eq!(backend.calls_of("rename_no_replace"), 1 + 12);
    assert_eq!(
        backend.paths_of("rename_no_replace")[1..],
        vec![aside(); 12][..]
    );
    for operation in ["remove_file", "rename", "rename_claimed"] {
        assert_eq!(backend.calls_of(operation), 0, "{operation}");
    }

    let clock = ManualClock::at(at(0));
    let (disk, backend) = owner(&clock);
    backend.stat_is(aside().to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "rename_no_replace",
        aside().to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    let answer = disk.remove_source_beside_blocking(
        &p(SOURCE),
        &source_mark(),
        &p(SAME_ENTRY),
        &mark(named(7, 1)),
    );
    assert!(
        matches!(answer, AsideRemoval::PutBackFailed { .. }),
        "{answer:?}"
    );
    assert_eq!(clock.now(), at(30));
    assert_eq!(backend.calls_of("rename_no_replace"), 1 + 12);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A put-back whose set-aside name stops holding the file while it is
/// retried stops at once: back at the source's name, the file was put back
/// by a rename that lost its reply; anywhere else, it moved, and nothing
/// more is done.
#[tokio::test(start_paused = true)]
async fn a_put_back_stops_once_the_set_aside_name_no_longer_holds_the_file() {
    for at_source in [false, true] {
        let (disk, backend) = put_back_failing(None);
        backend.stats_in_turn(&aside(), &[Ok(named(7, 1)), Ok(named(7, 1))]);
        if at_source {
            backend.stat_is(SOURCE, Ok(named(7, 1)));
        }
        let answer = disk
            .remove_source_beside(
                &p(SOURCE),
                &source_mark(),
                &p(SAME_ENTRY),
                &mark(named(7, 1)),
            )
            .await;
        match at_source {
            true => assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}"),
            false => match answer {
                AsideRemoval::SetAsideMoved { at, .. } => assert_eq!(at, aside()),
                other => panic!("{other:?}"),
            },
        }
        // The first attempt and two retries renamed; the third retry found
        // the name empty.
        assert_eq!(backend.calls_of("rename_no_replace"), 1 + 3);
        assert_eq!(backend.paths_of("lstat").last(), Some(&p(SOURCE)));
        assert_eq!(backend.calls_of("remove_file"), 0);
    }
}

/// A volume that makes up a file number per name reads the set-aside file as
/// another file. A regular file of the source's length there, with the
/// source's name missing, is still renamed back on a retry: it never replaces
/// anything. While the rename keeps failing, the file is reported left at its
/// set-aside name, never as moved elsewhere.
#[tokio::test(start_paused = true)]
async fn a_put_back_retries_a_set_aside_file_its_volume_renumbered() {
    let renumbered = named(977, 1);
    for put_back_on_retry in [true, false] {
        let (disk, backend) = put_back_failing(put_back_on_retry.then_some(1));
        backend.stat_is(aside().to_str().unwrap(), Ok(renumbered));
        let answer = disk
            .remove_source_beside(
                &p(SOURCE),
                &source_mark(),
                &p(SAME_ENTRY),
                &mark(named(7, 1)),
            )
            .await;
        match put_back_on_retry {
            true => {
                assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
                assert_eq!(
                    backend.paths_of("rename_no_replace"),
                    [p(SOURCE), aside(), aside()]
                );
            }
            false => {
                match answer {
                    AsideRemoval::PutBackFailed { at, .. } => assert_eq!(at, aside()),
                    other => panic!("{other:?}"),
                }
                assert_eq!(backend.calls_of("rename_no_replace"), 1 + 12);
            }
        }
        assert_eq!(backend.calls_of("remove_file"), 0);
    }

    // Something of another length at the set-aside name is not the file.
    let (disk, backend) = put_back_failing(None);
    backend.stats_in_turn(
        &aside(),
        &[
            Ok(named(7, 1)),
            Ok(RawStat {
                len: 3,
                ..renumbered
            }),
        ],
    );
    let answer = disk
        .remove_source_beside(
            &p(SOURCE),
            &source_mark(),
            &p(SAME_ENTRY),
            &mark(named(7, 1)),
        )
        .await;
    assert!(
        matches!(answer, AsideRemoval::SetAsideMoved { .. }),
        "{answer:?}"
    );
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// On a real volume: a destination reached through a symlinked folder, or a
/// link to the source, is the source's own entry, and the source survives; a
/// second name of the source's file in another folder lets the source go and
/// keeps the destination.
#[cfg(unix)]
#[test]
fn on_real_files_a_linked_folder_alias_keeps_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let downloads = temp.path().join("downloads");
    let library = temp.path().join("library");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::create_dir_all(&library).unwrap();
    let source = downloads.join("sample.mkv");
    std::fs::write(&source, b"payload").unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&downloads, &alias).unwrap();
    let disk = Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)));
    let read = |path: &Path| disk.fingerprint_blocking(path).unwrap();
    let source_read = read(&source);

    let through_alias = alias.join("sample.mkv");
    let answer = disk.remove_source_beside_blocking(
        &source,
        &source_read,
        &through_alias,
        &read(&through_alias),
    );
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"payload");
    assert!(!aside_path_for(&source).exists());

    let link = library.join("link.mkv");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    let answer = disk.remove_source_beside_blocking(&source, &source_read, &link, &read(&link));
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"payload");

    let second = library.join("second.mkv");
    std::fs::hard_link(&source, &second).unwrap();
    let answer = disk.remove_source_beside_blocking(&source, &source_read, &second, &read(&second));
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert!(!source.exists());
    assert!(!aside_path_for(&source).exists());
    assert_eq!(std::fs::read(&second).unwrap(), b"payload");
}
