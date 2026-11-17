//! A hard link that must prove itself: what proves, what does not, and that
//! an unproven link removes only the name its own call created. Against the
//! scripted backend first, then on synthetic files in a temporary directory.

use super::*;
use crate::place::{Doubt, LinkAfter, LinkProof};

const SOURCE: &str = "/srv/fixture-media/incoming/sample.mkv";
const DEST: &str = "/srv/fixture-media/Fixture Show/sample.mkv";

fn linking(fallback: Fallback) -> Placement {
    Placement {
        method: Method::HardLink,
        fallback,
        replace: Replace::Never,
        verify: Verify::Quick,
        parents: Parents::MustExist,
    }
}

fn scripted() -> (Disk, Arc<Scripted>) {
    owner(&ManualClock::at(at(0)))
}

/// A stat of a file on device 1 with this file number and length.
fn numbered(inode: u64, len: u64) -> RawStat {
    let mut stat = file(len);
    stat.extra.inode = Some(inode);
    stat
}

/// A stat as a platform that numbers no files gives it.
fn unnumbered(len: u64, modified_secs: u64) -> RawStat {
    let mut stat = file(len);
    stat.extra.device = None;
    stat.extra.inode = None;
    stat.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified_secs));
    stat
}

fn prove(
    disk: &Disk,
    fallback: Fallback,
    seen: &RawStat,
    proof: LinkProof,
) -> Result<Placed, PlaceRefusal> {
    let seen = Mark::of_stat(seen);
    disk.place_proven_blocking(&p(SOURCE), &p(DEST), linking(fallback), &seen, proof)
}

#[test]
fn a_link_whose_new_name_is_the_file_seen_is_proven() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(7, 17)));
    let placed = prove(
        &disk,
        Fallback::None,
        &numbered(7, 17),
        LinkProof::NameAndLength,
    )
    .expect("proven");
    assert_eq!(placed.done, Done::HardLinked);
    assert!(!placed.fell_back());
    assert_eq!(backend.paths_of("lstat"), [p(DEST)]);
    assert_eq!(backend.paths_of("stat"), [p(DEST)]);
    assert_eq!(backend.calls_of("remove_file"), 0);

    // A filesystem that numbers every file zero links truly: zero is
    // compared as given.
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(0, 17)));
    let placed = prove(
        &disk,
        Fallback::None,
        &numbered(0, 17),
        LinkProof::NameAndLength,
    );
    assert_eq!(placed.expect("proven").done, Done::HardLinked);
    assert_eq!(backend.calls_of("remove_file"), 0);

    // Where no file is numbered, length and modification time stand in.
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(unnumbered(17, 40)));
    let placed = prove(
        &disk,
        Fallback::None,
        &unnumbered(17, 40),
        LinkProof::NameAndLength,
    );
    assert_eq!(placed.expect("proven").done, Done::HardLinked);
}

#[test]
fn a_proof_of_the_name_alone_reads_it_once() {
    let (disk, backend) = scripted();
    // The same file at another length: only the fuller proof would look.
    backend.stat_is(DEST, Ok(numbered(7, 9)));
    let placed = prove(&disk, Fallback::None, &numbered(7, 17), LinkProof::Name);
    assert_eq!(placed.expect("proven").done, Done::HardLinked);
    assert_eq!(backend.paths_of("lstat"), [p(DEST)], "one probe");
    assert_eq!(backend.calls_of("stat"), 0);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[test]
fn an_unproven_link_is_removed_and_says_what_did_not_prove() {
    type Script = fn(&Scripted);
    type Check = fn(&Doubt) -> bool;
    let cases: [(&str, Script, RawStat, Check); 6] = [
        (
            "another file",
            |backend| backend.stat_is(DEST, Ok(numbered(8, 17))),
            numbered(7, 17),
            |doubt| matches!(doubt, Doubt::OtherFile { .. }),
        ),
        (
            "zero against a number",
            |backend| backend.stat_is(DEST, Ok(numbered(0, 17))),
            numbered(7, 17),
            |doubt| matches!(doubt, Doubt::OtherFile { .. }),
        ),
        (
            "another modification time where nothing is numbered",
            |backend| backend.stat_is(DEST, Ok(unnumbered(17, 41))),
            unnumbered(17, 40),
            |doubt| matches!(doubt, Doubt::OtherFile { .. }),
        ),
        (
            "another length",
            |backend| backend.stat_is(DEST, Ok(numbered(7, 9))),
            numbered(7, 17),
            |doubt| matches!(doubt, Doubt::OtherLength(read) if read.size() == Some(9)),
        ),
        (
            "a length that cannot be read",
            |backend| {
                backend.stat_is(DEST, Ok(numbered(7, 17)));
                backend.fails("stat", DEST, io::ErrorKind::NotConnected);
            },
            numbered(7, 17),
            |doubt| matches!(doubt, Doubt::OtherLength(read) if read.size().is_none()),
        ),
        (
            "a name that cannot be read",
            |backend| backend.fails("lstat", DEST, io::ErrorKind::PermissionDenied),
            numbered(7, 17),
            |doubt| {
                matches!(doubt, Doubt::Unread(error)
                    if error.reason() == reason::DISK_PERMISSION_DENIED)
            },
        ),
    ];
    for (case, script, seen, expected) in cases {
        let (disk, backend) = scripted();
        script(&backend);
        let refusal =
            prove(&disk, Fallback::None, &seen, LinkProof::NameAndLength).expect_err("not proven");
        assert_eq!(
            refusal.refused,
            Refused::CannotLink(CannotLink::Unverifiable),
            "{case}"
        );
        assert_eq!(
            refusal.refused.reason(),
            reason::DISK_LINK_UNVERIFIABLE,
            "{case}"
        );
        let unproven = refusal.unproven.as_ref().expect("says why");
        assert_eq!(unproven.after, LinkAfter::Removed, "{case}");
        let doubt = &unproven.doubt;
        assert!(expected(doubt), "{case}: {doubt:?}");
        assert_eq!(
            backend.paths_of("remove_file"),
            [p(DEST)],
            "{case}: only the name it made"
        );
        assert_eq!(backend.calls_of("open"), 0, "{case}: no copy was asked for");
        assert_eq!(backend.calls_of("remove_dir"), 0, "{case}");
        assert_eq!(backend.calls_of("remove_tree"), 0, "{case}");
    }
}

#[test]
fn a_name_that_was_there_before_the_call_is_refused_and_never_removed() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    backend.fails("hard_link", DEST, io::ErrorKind::AlreadyExists);
    for fallback in [Fallback::None, Fallback::Copy] {
        let refusal = prove(&disk, fallback, &numbered(7, 17), LinkProof::NameAndLength)
            .expect_err("refused");
        assert_eq!(refusal.refused, Refused::DestinationExists);
        assert!(refusal.unproven.is_none());
    }
    assert_eq!(backend.calls_of("remove_file"), 0);
    assert_eq!(backend.calls_of("lstat"), 0, "nothing of its own to prove");
    assert_eq!(backend.calls_of("open"), 0);
}

#[test]
fn a_link_that_was_not_made_is_never_removed() {
    for kind in [
        io::ErrorKind::CrossesDevices,
        io::ErrorKind::Unsupported,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
    ] {
        let (disk, backend) = scripted();
        backend.fails("hard_link", DEST, kind);
        let refusal = prove(
            &disk,
            Fallback::None,
            &numbered(7, 17),
            LinkProof::NameAndLength,
        )
        .expect_err("refused");
        assert!(refusal.unproven.is_none(), "{kind:?}");
        assert_eq!(backend.calls_of("remove_file"), 0, "{kind:?}");
        assert_eq!(backend.calls_of("lstat"), 0, "{kind:?}");
    }
}

#[test]
fn a_removal_that_fails_is_the_refusal_and_still_says_what_did_not_prove() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    backend.fails("remove_file", DEST, io::ErrorKind::PermissionDenied);
    // The copy must not run over a link that is still there.
    let refusal = prove(
        &disk,
        Fallback::Copy,
        &numbered(7, 17),
        LinkProof::NameAndLength,
    )
    .expect_err("the removal failed");
    assert_eq!(refusal.refused, Refused::Denied);
    let unproven = refusal.unproven.as_ref().expect("says why");
    assert_eq!(unproven.after, LinkAfter::NotRemoved);
    let doubt = &unproven.doubt;
    assert!(matches!(doubt, Doubt::OtherFile { .. }), "{doubt:?}");
    assert_eq!(backend.calls_of("open"), 0, "no copy over the link");

    // A name already gone was changed by something else: that is a failure.
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    backend.fails("remove_file", DEST, io::ErrorKind::NotFound);
    let refusal = prove(
        &disk,
        Fallback::None,
        &numbered(7, 17),
        LinkProof::NameAndLength,
    )
    .expect_err("the name was already gone");
    assert_eq!(refusal.refused, Refused::Failed);
    let unproven = refusal.unproven.as_ref().expect("says why");
    assert_eq!(unproven.after, LinkAfter::NotRemoved);
    assert_eq!(refusal.to_string(), format!("{DEST} is already gone"));
}

#[test]
fn an_unproven_link_falls_back_to_a_copy_once_it_is_removed() {
    let (disk, backend) = scripted();
    backend.stat_is(SOURCE, Ok(numbered(7, 17)));
    backend.holds(SOURCE, b"synthetic payload");
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    *backend.staged_contents.lock().unwrap() = Some(b"synthetic payload".to_vec());
    *backend.staged_stat.lock().unwrap() = Some(Ok(file(17)));
    let placed = prove(
        &disk,
        Fallback::Copy,
        &numbered(7, 17),
        LinkProof::NameAndLength,
    )
    .expect("copied");
    assert_eq!(placed.done, Done::Copied);
    assert!(placed.fell_back());
    let removed = backend.paths_of("remove_file");
    assert_eq!(removed, [p(DEST)], "the link, before the copy");
}

#[test]
fn a_plain_link_is_not_proven_and_reads_nothing() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    let placed = disk
        .place_blocking(&p(SOURCE), &p(DEST), linking(Fallback::None))
        .expect("linked");
    assert_eq!(placed.done, Done::HardLinked);
    assert_eq!(backend.calls_of("lstat") + backend.calls_of("stat"), 0);
    assert_eq!(backend.calls_of("remove_file"), 0);
}

#[tokio::test]
async fn the_async_form_proves_like_the_blocking_one() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    let seen = Mark::of_stat(&numbered(7, 17));
    let placing = disk.place_proven(SOURCE, DEST, linking(Fallback::None), seen, LinkProof::Name);
    let refusal = super::place::bounded(placing)
        .await
        .expect_err("not proven");
    assert_eq!(
        refusal.refused,
        Refused::CannotLink(CannotLink::Unverifiable)
    );
    assert_eq!(backend.paths_of("remove_file"), [p(DEST)]);
}

mod on_real_files {
    use super::*;

    fn disk() -> Disk {
        Disk::with_seams(Arc::new(SystemBackend), ManualClock::at(at(0)))
    }

    fn mark(path: &Path) -> Mark {
        Mark::of(&std::fs::symlink_metadata(path).expect("fixture metadata"))
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

    struct Fixture {
        dir: tempfile::TempDir,
        source: PathBuf,
        dest: PathBuf,
        neighbour: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("invented-source.mkv");
        let dest = dir.path().join("invented-dest.mkv");
        let neighbour = dir.path().join("invented-neighbour.mkv");
        std::fs::write(&source, b"synthetic payload").expect("source");
        std::fs::write(&neighbour, b"unrelated").expect("neighbour");
        Fixture {
            dir,
            source,
            dest,
            neighbour,
        }
    }

    impl Fixture {
        fn untouched(&self) {
            assert_eq!(
                std::fs::read(&self.source).expect("source"),
                b"synthetic payload"
            );
            assert_eq!(
                std::fs::read(&self.neighbour).expect("neighbour"),
                b"unrelated"
            );
        }
    }

    #[test]
    fn a_true_link_is_proven_and_kept() {
        let at = fixture();
        let seen = mark(&at.source);
        let placed = disk()
            .place_proven_blocking(
                &at.source,
                &at.dest,
                linking(Fallback::None),
                &seen,
                LinkProof::NameAndLength,
            )
            .expect("proven");
        assert_eq!(placed.done, Done::HardLinked);
        assert_eq!(std::fs::read(&at.dest).expect("dest"), b"synthetic payload");
        at.untouched();
    }

    #[test]
    fn a_link_to_another_file_than_the_one_seen_is_removed() {
        let at = fixture();
        // The caller saw another file under the source's name.
        let seen = mark(&at.neighbour);
        let refusal = disk()
            .place_proven_blocking(
                &at.source,
                &at.dest,
                linking(Fallback::None),
                &seen,
                LinkProof::NameAndLength,
            )
            .expect_err("not proven");
        assert_eq!(
            refusal.refused,
            Refused::CannotLink(CannotLink::Unverifiable)
        );
        let doubt = refusal.unproven.expect("says why").doubt;
        assert!(matches!(doubt, Doubt::OtherFile { .. }), "{doubt:?}");
        assert_eq!(
            names(at.dir.path()),
            ["invented-neighbour.mkv", "invented-source.mkv"]
        );
        at.untouched();
    }

    /// The file numbers match, so only the resolved length can tell.
    #[cfg(unix)]
    #[test]
    fn a_link_whose_length_is_not_the_one_seen_is_removed() {
        let at = fixture();
        let seen = mark(&at.source);
        std::fs::write(&at.source, b"synthetic payload, grown").expect("grow the source");
        let refusal = disk()
            .place_proven_blocking(
                &at.source,
                &at.dest,
                linking(Fallback::None),
                &seen,
                LinkProof::NameAndLength,
            )
            .expect_err("not proven");
        assert_eq!(
            refusal.refused,
            Refused::CannotLink(CannotLink::Unverifiable)
        );
        let doubt = refusal.unproven.expect("says why").doubt;
        assert!(matches!(doubt, Doubt::OtherLength(_)), "{doubt:?}");
        assert_eq!(
            names(at.dir.path()),
            ["invented-neighbour.mkv", "invented-source.mkv"]
        );
        assert_eq!(
            std::fs::read(&at.source).expect("source"),
            b"synthetic payload, grown"
        );
        assert_eq!(
            std::fs::read(&at.neighbour).expect("neighbour"),
            b"unrelated"
        );

        // The proof of the name alone does not ask for the length.
        let placed = disk()
            .place_proven_blocking(
                &at.source,
                &at.dest,
                linking(Fallback::None),
                &seen,
                LinkProof::Name,
            )
            .expect("proven by name");
        assert_eq!(placed.done, Done::HardLinked);
    }

    #[test]
    fn a_destination_that_was_already_there_is_refused_and_kept() {
        let at = fixture();
        std::fs::write(&at.dest, b"already here").expect("occupant");
        // Even a mark that could never prove removes nothing it did not make.
        for seen in [mark(&at.source), mark(&at.neighbour)] {
            for fallback in [Fallback::None, Fallback::Copy] {
                let refusal = disk()
                    .place_proven_blocking(
                        &at.source,
                        &at.dest,
                        linking(fallback),
                        &seen,
                        LinkProof::NameAndLength,
                    )
                    .expect_err("refused");
                assert_eq!(refusal.refused, Refused::DestinationExists);
                assert!(refusal.unproven.is_none());
                assert_eq!(std::fs::read(&at.dest).expect("occupant"), b"already here");
            }
        }
        at.untouched();
    }

    #[test]
    fn an_unproven_link_is_removed_before_its_fallback_copy_lands() {
        let at = fixture();
        let seen = mark(&at.neighbour);
        let placed = disk()
            .place_proven_blocking(
                &at.source,
                &at.dest,
                linking(Fallback::Copy),
                &seen,
                LinkProof::NameAndLength,
            )
            .expect("copied");
        assert_eq!(placed.done, Done::Copied);
        assert_eq!(std::fs::read(&at.dest).expect("dest"), b"synthetic payload");
        // A copy, not a second name for the source.
        assert_eq!(
            mark(&at.dest).same_file(&mark(&at.source)) == Sameness::Same,
            false
        );
        assert_eq!(
            names(at.dir.path()),
            [
                "invented-dest.mkv",
                "invented-neighbour.mkv",
                "invented-source.mkv"
            ]
        );
        at.untouched();
    }
}
