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
/// and no other name is the set-aside file seen through another view, though
/// its read names another volume or none: the source goes back, with no
/// listing. With another name it is a second link, judged as any other
/// destination off the source's volume.
#[test]
fn a_destination_with_the_sources_number_and_no_other_name_is_its_entry_on_any_volume() {
    let dest = "/srv/fixture-media/mount-b/downloads/sample.mkv";
    for kept in [on_volume(2, 7, Some(1)), on_volume(2, 7, None)] {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        let answer =
            disk.remove_source_beside_blocking(&p(SOURCE), &source_mark(), &p(dest), &mark(kept));
        assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
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

/// Off the source's known volume, the destination's folder is listed once
/// and must hold its name byte for byte before the set-aside name goes. A
/// name the listing lacks, another spelling of it included, is the source's
/// entry; a listing that fails is unread. Either way the source goes back.
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
            assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
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
/// left or anything else, is named in its own field.
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
    assert_eq!(
        backend.calls_of("lstat") + backend.calls_of("remove_file"),
        0
    );

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
/// only when that name holds the source's file: a reservation, an empty
/// file, or nothing there is no source that moved, so nothing is put back
/// and no failed put-back is reported.
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
}

/// A put-back that fails says where the source's file is: at the set-aside
/// name. A removal of the set-aside name that fails reads that name: while
/// the file is still there the source is put back, and a name that reads
/// gone was removed after all, the destination having held the file.
#[test]
fn a_failed_put_back_or_removal_says_where_the_file_is() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let aside = aside();

    let (disk, backend) = scripted();
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

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
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
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
    assert_eq!(backend.paths_of("lstat"), [p(dest), aside]);
}

/// An import that finds its source missing puts back a regular file an
/// interrupted cleanup left at the set-aside name, by a rename that never
/// replaces anything. Nothing at that name, a folder there, or a source name
/// taken again moves nothing, and nothing is removed.
#[test]
fn a_set_aside_source_left_by_an_interruption_is_put_back() {
    let aside = aside();

    let (disk, backend) = scripted();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
    assert!(matches!(answer, PutBackAside::PutBack), "{answer:?}");
    assert_eq!(backend.paths_of("lstat"), [aside.clone()]);
    assert_eq!(backend.paths_of("rename_no_replace"), [aside.clone()]);

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
    }

    let (disk, backend) = scripted();
    backend.stat_is(aside.to_str().unwrap(), Ok(named(7, 1)));
    backend.fails(
        "rename_no_replace",
        aside.to_str().unwrap(),
        io::ErrorKind::AlreadyExists,
    );
    let answer = disk.put_back_set_aside_source_blocking(&p(SOURCE));
    assert!(matches!(answer, PutBackAside::Nothing), "{answer:?}");
    for operation in ["remove_file", "rename_claimed", "rename"] {
        assert_eq!(backend.calls_of(operation), 0, "{operation}");
    }
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
