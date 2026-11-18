//! A hard link that must prove itself: what proves, what does not, and that
//! an unproven link removes only the name its own call created. Against the
//! scripted backend first, then on synthetic files in a temporary directory.

use super::*;
use crate::place::{Doubt, LinkAfter, LinkProof, LinkProven};

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
    assert_eq!(placed.proven_by, Some(LinkProven::Identity));
    assert!(!placed.fell_back());
    assert_eq!(backend.paths_of("lstat"), [p(DEST)]);
    assert_eq!(backend.paths_of("stat"), [p(DEST)]);
    assert_eq!(backend.calls_of("remove_file"), 0);
    assert_eq!(backend.calls_of("open"), 0, "file numbers read no content");

    // A filesystem that numbers every file zero, and one that numbers none,
    // prove by content: one length, and the first and last MiB alike.
    for (linked, seen) in [
        (numbered(0, 17), numbered(0, 17)),
        (unnumbered(17, 40), unnumbered(17, 40)),
    ] {
        let (disk, backend) = scripted();
        backend.stat_is(
            DEST,
            Ok(RawStat {
                modified: seen.modified,
                ..linked
            }),
        );
        backend.holds(SOURCE, b"seventeen bytes..");
        backend.holds(DEST, b"seventeen bytes..");
        let placed = prove(&disk, Fallback::None, &seen, LinkProof::NameAndLength).expect("proven");
        assert_eq!(placed.done, Done::HardLinked);
        assert_eq!(placed.proven_by, Some(LinkProven::Content));
        assert_eq!(backend.paths_of("open"), [p(SOURCE), p(DEST)]);
        assert_eq!(backend.calls_of("remove_file"), 0);
    }

    // Where one read has a number and the other none, the volume numbers its
    // files and nothing proves: the content is not asked, and the name this
    // call made is removed.
    for (linked, seen) in [
        (numbered(7, 17), unnumbered(17, 0)),
        (unnumbered(17, 0), numbered(7, 17)),
    ] {
        let (disk, backend) = scripted();
        backend.stat_is(
            DEST,
            Ok(RawStat {
                modified: seen.modified,
                ..linked
            }),
        );
        backend.holds(SOURCE, b"seventeen bytes..");
        backend.holds(DEST, b"seventeen bytes..");
        let refusal =
            prove(&disk, Fallback::None, &seen, LinkProof::NameAndLength).expect_err("unproven");
        assert_eq!(
            refusal.refused,
            Refused::CannotLink(CannotLink::Unverifiable)
        );
        let doubt = &refusal.unproven.as_ref().expect("says why").doubt;
        assert!(matches!(doubt, Doubt::OtherFile { .. }), "{doubt:?}");
        assert_eq!(backend.calls_of("open"), 0, "no content is read");
        assert_eq!(backend.paths_of("remove_file"), [p(DEST)]);
    }
}

/// Only the service's own read proves: the same read stored and read back,
/// or naming its volume alone, does not, on any lane, and the name the call
/// made is removed without its content being read.
#[test]
fn a_stored_or_volume_only_mark_never_proves_a_link() {
    for stat in [numbered(7, 17), numbered(0, 17), unnumbered(17, 40)] {
        let read = Mark::of_stat(&stat);
        let stored = Mark::from_stored(&read.to_stored()).expect("stored");
        for seen in [read.as_stored(), stored, read.volume()] {
            let (disk, backend) = scripted();
            backend.stat_is(DEST, Ok(stat));
            backend.holds(SOURCE, b"seventeen bytes..");
            backend.holds(DEST, b"seventeen bytes..");
            let refusal = disk
                .place_proven_blocking(
                    &p(SOURCE),
                    &p(DEST),
                    linking(Fallback::None),
                    &seen,
                    LinkProof::Name,
                )
                .expect_err("never proven");
            assert_eq!(
                refusal.refused,
                Refused::CannotLink(CannotLink::Unverifiable),
                "{seen:?}"
            );
            let doubt = &refusal.unproven.as_ref().expect("says why").doubt;
            assert!(matches!(doubt, Doubt::OtherFile { .. }), "{doubt:?}");
            assert_eq!(backend.calls_of("open"), 0, "no content is read");
            assert_eq!(backend.paths_of("remove_file"), [p(DEST)]);
        }
    }
}

/// Where only the content can prove a link, a name with other content is
/// another writer's file and is left alone; content that cannot be read
/// proves nothing, and the name this call made is removed.
#[test]
fn a_link_proven_by_content_keeps_a_name_with_other_content() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(0, 17)));
    backend.holds(SOURCE, b"seventeen bytes..");
    backend.holds(DEST, b"seventeen other..");
    for fallback in [Fallback::None, Fallback::Copy] {
        let refusal = prove(&disk, fallback, &numbered(0, 17), LinkProof::NameAndLength)
            .expect_err("other content");
        assert_eq!(refusal.refused, Refused::DestinationExists);
        let unproven = refusal.unproven.as_ref().expect("says why");
        assert!(matches!(unproven.doubt, Doubt::OtherContent { .. }));
        assert_eq!(unproven.after, LinkAfter::LeftAlone);
    }
    assert_eq!(backend.calls_of("remove_file"), 0);
    assert_eq!(backend.calls_of("stat"), 0, "nothing resolved");

    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(0, 17)));
    backend.holds(SOURCE, b"seventeen bytes..");
    let refusal = prove(&disk, Fallback::None, &numbered(0, 17), LinkProof::Name)
        .expect_err("unread content");
    assert_eq!(
        refusal.refused,
        Refused::CannotLink(CannotLink::Unverifiable)
    );
    let unproven = refusal.unproven.as_ref().expect("says why");
    assert!(
        matches!(unproven.doubt, Doubt::Unread(_)),
        "{:?}",
        unproven.doubt
    );
    assert_eq!(unproven.after, LinkAfter::Removed);
    assert_eq!(backend.paths_of("remove_file"), [p(DEST)]);

    // A length that differs on the content lane is the length's answer:
    // nothing is read.
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(0, 9)));
    let refusal = prove(&disk, Fallback::None, &numbered(0, 17), LinkProof::Name)
        .expect_err("another writer's");
    assert_eq!(refusal.refused, Refused::DestinationExists);
    assert_eq!(backend.calls_of("open"), 0);
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
    let cases: [(&str, Script, RawStat, Check); 5] = [
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
    assert_eq!(refusal.refused, Refused::Denied(Side::Destination));
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
    assert_eq!(refusal.refused, Refused::DestinationMissing);
    let unproven = refusal.unproven.as_ref().expect("says why");
    assert_eq!(unproven.after, LinkAfter::AlreadyGone);
    assert_eq!(refusal.to_string(), format!("{DEST} is already gone"));
}

/// A link to a regular file never reads as anything else, so such a name is
/// another writer's: it is refused, said to be left alone, and not removed.
#[test]
fn a_new_name_that_is_no_regular_file_is_left_alone() {
    for fallback in [Fallback::None, Fallback::Copy] {
        let (disk, backend) = scripted();
        backend.stat_is(DEST, Ok(dir()));
        let refusal = prove(&disk, fallback, &numbered(7, 17), LinkProof::NameAndLength)
            .expect_err("not the link");
        assert_eq!(refusal.refused, Refused::DestinationExists);
        let unproven = refusal.unproven.as_ref().expect("says why");
        assert_eq!(unproven.after, LinkAfter::LeftAlone);
        assert!(matches!(
            &unproven.doubt,
            Doubt::OtherFile { linked } if linked.kind() == Some(EntryKind::Directory)
        ));
        let removals = ["remove_file", "remove_dir", "remove_tree"];
        assert_eq!(removals.map(|verb| backend.calls_of(verb)), [0, 0, 0]);
        assert_eq!(backend.calls_of("open"), 0, "no copy over it");
    }
}

/// A regular file at the new name that is not the link is removed only when
/// its length and modification time are the source's, which is how a true
/// link reads where files are numbered by path. With another length or
/// another modification time it is another writer's file and stays.
#[test]
fn a_regular_file_that_is_not_the_link_goes_only_with_the_sources_facts() {
    fn timed(inode: u64, len: u64, modified_secs: u64) -> RawStat {
        let mut stat = numbered(inode, len);
        stat.modified = unnumbered(len, modified_secs).modified;
        stat
    }
    // (what the new name reads as, what the source was seen as, removed)
    let cases = [
        (timed(8, 17, 40), timed(7, 17, 40), true),
        (unnumbered(17, 40), timed(7, 17, 40), true),
        (timed(8, 9, 40), timed(7, 17, 40), false),
        (timed(8, 17, 41), timed(7, 17, 40), false),
        (unnumbered(17, 41), unnumbered(17, 40), false),
        (unnumbered(9, 40), unnumbered(17, 40), false),
    ];
    for (index, (linked, seen, removed)) in cases.into_iter().enumerate() {
        for fallback in [Fallback::None, Fallback::Copy] {
            let (disk, backend) = scripted();
            backend.stat_is(DEST, Ok(linked));
            backend.fails("open", SOURCE, io::ErrorKind::PermissionDenied);
            let refusal =
                prove(&disk, fallback, &seen, LinkProof::NameAndLength).expect_err("not the link");
            let unproven = refusal.unproven.as_ref().expect("says why");
            // A name that reads with no number against a numbered source is
            // not the link either: no content is asked.
            assert!(
                matches!(unproven.doubt, Doubt::OtherFile { .. }),
                "case {index}: {:?}",
                unproven.doubt
            );
            let after = match removed {
                true => LinkAfter::Removed,
                false => LinkAfter::LeftAlone,
            };
            assert_eq!(unproven.after, after, "case {index}");
            let removals = backend.paths_of("remove_file");
            let of_the_name = removals.iter().filter(|path| **path == p(DEST)).count();
            assert_eq!(of_the_name, usize::from(removed), "case {index}");
            if !removed {
                assert_eq!(refusal.refused, Refused::DestinationExists, "case {index}");
                assert_eq!(backend.calls_of("open"), 0, "case {index}: no copy over it");
            }
        }
    }
}

/// The copy that follows a removed link can be refused too, and the refusal
/// still says the link did not prove and is gone.
#[test]
fn a_fallback_copy_that_fails_still_says_the_link_was_removed() {
    let (disk, backend) = scripted();
    backend.stat_is(DEST, Ok(numbered(8, 17)));
    backend.fails("open", SOURCE, io::ErrorKind::PermissionDenied);
    let refusal = prove(
        &disk,
        Fallback::Copy,
        &numbered(7, 17),
        LinkProof::NameAndLength,
    )
    .expect_err("the copy failed");
    assert_eq!(refusal.refused, Refused::Denied(Side::Source));
    let unproven = refusal.unproven.as_ref().expect("says why");
    assert_eq!(unproven.after, LinkAfter::Removed);
    assert!(matches!(unproven.doubt, Doubt::OtherFile { .. }));
    assert_eq!(backend.paths_of("remove_file").first(), Some(&p(DEST)));
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

    /// The disk service's own read, as a caller's seen mark is: on a volume
    /// that numbers its files, it carries the number.
    fn mark(path: &Path) -> Mark {
        disk().fingerprint_blocking(path).expect("fixture metadata")
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
        /// Another file with the source's length and modification time, as a
        /// caller would have seen the source's name before it was replaced
        /// by a file that reads alike.
        fn twin(&self) -> Mark {
            let twin = self.dir.path().join("invented-twin.mkv");
            std::fs::write(&twin, b"synthetic PAYLOAD").expect("twin");
            let modified = std::fs::metadata(&self.source)
                .and_then(|source| source.modified())
                .expect("source time");
            let file = std::fs::File::options()
                .write(true)
                .open(&twin)
                .expect("open twin");
            file.set_modified(modified).expect("twin time");
            drop(file);
            let seen = mark(&twin);
            std::fs::remove_file(&twin).expect("the fixture's own twin");
            seen
        }

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
        // The caller saw another file under the source's name, one that
        // read as the source does now.
        let seen = at.twin();
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

    /// The caller saw a file of another length and time under the source's
    /// name. The name the link call made does not read as that file, so it
    /// is not removed as the link, and nothing is copied over it.
    #[test]
    fn a_new_name_with_another_length_and_time_than_the_one_seen_is_left() {
        for fallback in [Fallback::None, Fallback::Copy] {
            let at = fixture();
            let seen = mark(&at.neighbour);
            let refusal = disk()
                .place_proven_blocking(
                    &at.source,
                    &at.dest,
                    linking(fallback),
                    &seen,
                    LinkProof::NameAndLength,
                )
                .expect_err("not proven");
            assert_eq!(refusal.refused, Refused::DestinationExists);
            let unproven = refusal.unproven.expect("says why");
            assert_eq!(unproven.after, LinkAfter::LeftAlone);
            assert_eq!(std::fs::read(&at.dest).expect("dest"), b"synthetic payload");
            at.untouched();
        }
    }

    #[test]
    fn an_unproven_link_is_removed_before_its_fallback_copy_lands() {
        let at = fixture();
        let seen = at.twin();
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
        // A copy, not a second name for the source: writing to one leaves
        // the other as it was.
        assert_eq!(
            names(at.dir.path()),
            [
                "invented-dest.mkv",
                "invented-neighbour.mkv",
                "invented-source.mkv"
            ]
        );
        std::fs::write(&at.dest, b"rewritten").expect("rewrite the copy");
        at.untouched();
    }
}
