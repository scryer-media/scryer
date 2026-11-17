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

/// A destination that is the source's own entry is gone once the source is
/// aside, however the paths spell it: one folder under two mounts, another
/// case or Unicode form of one name, a share that numbers each path anew.
/// The source goes back and nothing is removed: two renames, one read.
#[test]
fn a_destination_that_is_the_source_entry_puts_the_source_back() {
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
        let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(kept));
        assert!(
            matches!(answer, AsideRemoval::SameEntry),
            "{dest}: {answer:?}"
        );
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
        assert_eq!(backend.paths_of("lstat"), [p(dest)]);
        assert_eq!(backend.calls_of("stat"), 0);
        assert_eq!(backend.calls_of("remove_file"), 0, "{dest}");
    }
}

/// A destination that still holds its file once the source is aside is
/// another entry: a second name of the source's own file elsewhere, another
/// file, or a file on a share that numbers each path anew or not at all.
/// The set-aside name is removed, never the source's name: one rename, one
/// read, one removal.
#[test]
fn a_destination_that_stays_lets_the_set_aside_source_go() {
    let dests = [
        ("/srv/fixture-media/library/second-name.mkv", named(7, 2)),
        ("/srv/fixture-media/library/sample.mkv", named(8, 1)),
        ("/srv/fixture-media/share/sample.mkv", named(60, 1)),
        ("/srv/fixture-media/share/sample.mkv", numbered(None, None)),
    ];
    for (dest, kept) in dests {
        let (disk, backend) = scripted();
        backend.stat_is(dest, Ok(kept));
        let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(kept));
        assert!(
            matches!(answer, AsideRemoval::Removed),
            "{dest}: {answer:?}"
        );
        assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
        assert_eq!(backend.paths_of("lstat"), [p(dest)]);
        assert_eq!(backend.paths_of("remove_file"), [aside()]);
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
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &dest, &mark(link(9)));
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(backend.paths_of("lstat"), [dest.clone()]);
    assert_eq!(backend.paths_of("stat"), [dest.clone()]);
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stats_in_turn(&dest, &[Ok(link(9)), Ok(named(8, 1))]);
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &dest, &mark(link(9)));
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
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(
        matches!(answer, AsideRemoval::DestinationChanged),
        "{answer:?}"
    );
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stat_is(dest, Err(io::ErrorKind::TimedOut));
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(
        matches!(answer, AsideRemoval::DestinationUnread(_)),
        "{answer:?}"
    );
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside()]);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A set-aside rename that is refused, a name left at the set-aside name by
/// an earlier attempt included, leaves the source alone and reads nothing.
#[test]
fn a_refused_set_aside_leaves_the_source_alone() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let (disk, backend) = scripted();
    backend.fails("rename_no_replace", SOURCE, io::ErrorKind::AlreadyExists);
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE)]);
    assert_eq!(
        backend.calls_of("lstat") + backend.calls_of("remove_file"),
        0
    );
}

/// Where only a claimed rename is offered, a failure that left something at
/// the set-aside name may have moved the source there: the source is read,
/// and put back only when it is not where it was.
#[test]
fn a_claimed_set_aside_that_left_a_name_puts_back_only_a_moved_source() {
    let dest = "/srv/fixture-media/library/sample.mkv";
    let aside = aside();

    let (disk, backend) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    *backend.claim_left.lock().unwrap() = true;
    backend.fails("rename_claimed", SOURCE, io::ErrorKind::TimedOut);
    backend.stat_is(SOURCE, Ok(named(7, 1)));
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(backend.paths_of("rename_claimed"), [p(SOURCE)]);
    assert_eq!(backend.paths_of("lstat"), [p(SOURCE)]);

    let (disk, backend) = scripted();
    backend.always_fails("rename_no_replace", io::ErrorKind::Unsupported);
    *backend.claim_left.lock().unwrap() = true;
    backend.fails("rename_claimed", SOURCE, io::ErrorKind::TimedOut);
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(matches!(answer, AsideRemoval::NotSetAside(_)), "{answer:?}");
    assert_eq!(
        backend.paths_of("rename_claimed"),
        [p(SOURCE), aside.clone()]
    );
    assert_eq!(backend.calls_of("remove_file"), 0);
}

/// A put-back that fails says where the source's file is: at the set-aside
/// name. A removal of the set-aside name that fails puts the source back.
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
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    match answer {
        AsideRemoval::PutBackFailed { at, .. } => assert_eq!(at, aside),
        other => panic!("{other:?}"),
    }
    assert_eq!(backend.calls_of("remove_file"), 0);

    let (disk, backend) = scripted();
    backend.stat_is(dest, Ok(named(8, 1)));
    backend.fails(
        "remove_file",
        aside.to_str().unwrap(),
        io::ErrorKind::PermissionDenied,
    );
    let answer = disk.remove_source_beside_blocking(&p(SOURCE), &p(dest), &mark(named(8, 1)));
    assert!(matches!(answer, AsideRemoval::NotRemoved(_)), "{answer:?}");
    assert_eq!(backend.paths_of("rename_no_replace"), [p(SOURCE), aside]);
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

    let through_alias = alias.join("sample.mkv");
    let answer = disk.remove_source_beside_blocking(&source, &through_alias, &read(&through_alias));
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"payload");
    assert!(!aside_path_for(&source).exists());

    let link = library.join("link.mkv");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    let answer = disk.remove_source_beside_blocking(&source, &link, &read(&link));
    assert!(matches!(answer, AsideRemoval::SameEntry), "{answer:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"payload");

    let second = library.join("second.mkv");
    std::fs::hard_link(&source, &second).unwrap();
    let answer = disk.remove_source_beside_blocking(&source, &second, &read(&second));
    assert!(matches!(answer, AsideRemoval::Removed), "{answer:?}");
    assert!(!source.exists());
    assert!(!aside_path_for(&source).exists());
    assert_eq!(std::fs::read(&second).unwrap(), b"payload");
}
