//! Removing a move's source once a destination that was already there has
//! been adopted in its place. The paths alone cannot show that the two are
//! separate entries: a linked or second-mounted folder, another spelling of
//! one name, or a link at the destination can make the destination the
//! source's own entry, and removing the source would then remove the only
//! copy. So the filesystem is asked: the source is renamed aside first, and
//! removed only while the destination still holds the file it was adopted as.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{DiskError, EntryKind, Mark, PathReading, Remove, Sameness, reason};

/// The pauses between attempts to put a set-aside source back once the first
/// attempt failed: doubling from a tenth of a second to five seconds, then
/// five seconds each, the last cut short so that together they come to thirty
/// seconds. With the first attempt that makes at most twelve renames.
pub const PUT_BACK_RETRY_PAUSES: [Duration; 11] = [
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
    Duration::from_millis(1_600),
    Duration::from_millis(3_200),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_millis(3_700),
];

/// How [`super::Disk::remove_source_beside_blocking`] ended. Every outcome but
/// [`Self::Removed`] leaves the source's data in place; where, each says.
#[derive(Debug)]
pub enum AsideRemoval {
    /// The source was set aside, the destination still held its file, and
    /// the set-aside name was removed.
    Removed,
    /// Setting the source aside took the destination with it: the two were
    /// one entry, or the destination is a link that reaches the source. The
    /// source was put back and nothing was removed.
    SameEntry,
    /// The destination holds another file than the one it was adopted as.
    /// The source was put back and nothing was removed.
    DestinationChanged,
    /// The destination, or its folder's listing, did not read once the
    /// source was aside, so nothing says whether the two are one entry. The
    /// source was put back and nothing was removed.
    DestinationUnread(PathReading),
    /// The destination read once the source was aside, but what it showed
    /// does not tell whether the two are one entry: its folder's listing
    /// lacks its name as spelled, or it shows the source's file number off
    /// the source's own named volume. The source was put back and nothing
    /// was removed.
    CannotTell,
    /// The destination checked out, but removing the set-aside name failed.
    /// The source was put back.
    NotRemoved(DiskError),
    /// The destination checked out, removing the set-aside name answered a
    /// failure, and the name then read missing, while the source's own name
    /// did not read missing: it holds a file again, as when something put the
    /// source back meanwhile, or it did not read. Nothing more was done, for a
    /// later cleanup to retry.
    RemovalUnconfirmed(DiskError),
    /// The source could not be renamed aside. The source is where it was.
    NotSetAside(DiskError),
    /// The set-aside name `at` is already taken, by a file an interrupted
    /// attempt left there or by anything else, so the source was not set
    /// aside. The source is where it was, and nothing at `at` was touched.
    AsideNameTaken { at: PathBuf, error: DiskError },
    /// Putting the source back failed, and kept failing for thirty seconds
    /// of retries (see [`PUT_BACK_RETRY_PAUSES`]), so its file is at `at`,
    /// the set-aside name in the source's own folder, untouched. It is never
    /// removed there; an import that finds the source missing puts it back.
    PutBackFailed { at: PathBuf, error: DiskError },
    /// Putting the source back failed, and while it was retried the
    /// set-aside name `at` stopped holding the source's file, nor does the
    /// source's own name hold it: something else moved it. Nothing more was
    /// done; where the file is was not found.
    SetAsideMoved { at: PathBuf, error: DiskError },
}

/// A put-back of a set-aside source that failed, with what the removal
/// would have answered had it succeeded.
struct Stuck {
    at: PathBuf,
    error: DiskError,
    then: AsideRemoval,
}

/// One attempt at putting a set-aside source back.
enum Attempt {
    /// The source's name holds its file again.
    PutBack,
    /// Neither the set-aside name nor the source's name holds the file.
    Moved,
    /// The rename failed, or the set-aside name did not read; try again.
    Failed(Option<DiskError>),
}

impl Stuck {
    /// What one retry settled, or the hold back to try again, keeping the
    /// latest rename failure.
    fn after(mut self, source: &Path, attempt: Attempt) -> Result<AsideRemoval, Self> {
        match attempt {
            Attempt::PutBack => {
                tracing::info!(
                    source = %source.display(),
                    from = %self.at.display(),
                    "a move's source set aside was put back on a retry"
                );
                Ok(self.then)
            }
            Attempt::Moved => {
                tracing::error!(
                    source = %source.display(),
                    at = %self.at.display(),
                    "a move's source set aside could not be put back, and its file left the set-aside name for somewhere else"
                );
                Ok(AsideRemoval::SetAsideMoved {
                    at: self.at,
                    error: self.error,
                })
            }
            Attempt::Failed(error) => {
                if let Some(error) = error {
                    self.error = error;
                }
                Err(self)
            }
        }
    }

    /// The answer once every put-back retry failed: the file stays at the
    /// set-aside name, untouched.
    fn left_aside(self, source: &Path) -> AsideRemoval {
        tracing::error!(
            source = %source.display(),
            at = %self.at.display(),
            error = %self.error,
            "a move's source set aside could not be put back after thirty seconds of retries; it is left at its set-aside name"
        );
        AsideRemoval::PutBackFailed {
            at: self.at,
            error: self.error,
        }
    }
}

/// Runs `job` on a blocking thread; a panic there carries on here.
async fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(job).await {
        Ok(value) => value,
        Err(join) => std::panic::resume_unwind(join.into_panic()),
    }
}

/// What [`super::Disk::put_back_set_aside_source_blocking`] did.
#[derive(Debug)]
pub enum PutBackAside {
    /// The set-aside name held a regular file and it was renamed back to
    /// the source's name.
    PutBack,
    /// There was nothing to put back: no nonempty regular file at the
    /// set-aside name. An empty one there was removed. Nothing was moved.
    Nothing,
    /// Renaming it back failed, and a read after the failure found the
    /// nonempty file still at `at`.
    Failed { at: PathBuf, error: DiskError },
    /// The set-aside name `at` holds a nonempty file, and the source's name
    /// holds something again, so nothing was moved and nothing removed:
    /// the file at `at` is left for the operator, logged.
    SourceNameTaken { at: PathBuf },
}

/// The one hidden name in `source`'s own folder its file is set aside under.
/// It carries no media extension, so no scanner takes it for media, and it
/// matches no name a cleanup pass removes: not a staging, upgrade, recovery
/// or extraction name, and no `.partial`.
pub fn aside_path_for(source: &Path) -> PathBuf {
    let folder = source.parent().unwrap_or_else(|| Path::new("."));
    let digest = blake3::hash(source.as_os_str().as_encoded_bytes()).to_hex();
    folder.join(format!(".scry-ia-{}", &digest[..9]))
}

/// Whether `path`, as spelled, names a network share (`\\server\share`, or
/// its `\\?\UNC\` or `\\.\UNC\` form). Read from the text alone, no probe: a
/// drive letter mapped to a share is not told apart here. Never on other
/// platforms.
fn on_a_share(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if match prefix.kind() {
                    Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
                    Prefix::DeviceNS(name) => name.eq_ignore_ascii_case("UNC"),
                    _ => false,
                }
        )
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

impl super::Disk {
    /// Removes `source` once the filesystem shows that `kept`, a destination
    /// that already held the file and was adopted, is a separate entry.
    /// `source_mark` is the caller's read of `source` that it has just
    /// proved is still the file it imported; nothing reads it again.
    /// `kept_mark` is the read of `kept`, links not followed, taken when it
    /// was adopted.
    ///
    /// 1. The source is renamed, never replacing anything, to
    ///    [`aside_path_for`]. A refused rename leaves it where it was.
    /// 2. `kept` is read, links not followed. A link is also read followed,
    ///    so a link that reached the source shows it no longer does.
    /// 3. Where `kept` now shows the source's own nonzero file number and
    ///    length and no other name, on the source's own named volume, it is
    ///    the set-aside file: the source goes back. The same number and
    ///    length on another volume, or one not named, cannot tell: a second
    ///    view of one folder can name another volume, and two volumes can
    ///    number a file alike. The source goes back either way.
    /// 4. Unless `kept` and the source both carry nonzero file numbers on
    ///    one known volume, and on Windows `kept`'s path names no network
    ///    share, `kept`'s folder is listed once and must hold `kept`'s name,
    ///    byte for byte.
    ///    A second view of a folder can keep answering a renamed name from
    ///    its cache; the listing is the folder's own word. A name the listing
    ///    lacks as spelled cannot tell, since the folder may hold it in
    ///    another case or normalization; a listing that fails is unread. The
    ///    source goes back either way.
    /// 5. While `kept` still holds the file `kept_mark` marks, the set-aside
    ///    name is removed. Otherwise the source is renamed back, never
    ///    replacing anything.
    ///
    /// What stays unproven: on a second cached view of one folder, a name
    /// whose file number that view makes up per name reads as another file
    /// for as long as the view's cache holds it, and where both carry
    /// numbers on one known volume reached by a drive letter, no listing is
    /// taken: a drive letter mapped to a share is not told from a local one
    /// without another probe. A destination that is a second hard link of the
    /// source's file elsewhere is kept and the source goes: the library
    /// entry for the source's name is gone, the data is not.
    ///
    /// Probes, for one adopted destination: one rename and one read of
    /// `kept`, a second read when `kept` is a link, then one removal. Unless
    /// both carry numbers on one known volume off a named share, one listing
    /// of `kept`'s folder besides. A put-back adds one rename. A put-back that
    /// fails is retried, each retry one read of the set-aside name and one
    /// rename, plus one read of the source's name when the set-aside name no
    /// longer marks the file; see [`PUT_BACK_RETRY_PAUSES`].
    ///
    /// This twin pauses between retries on the calling thread, for up to
    /// thirty seconds: for a thread of its own, such as a worker process.
    /// Everywhere else use [`Self::remove_source_beside`].
    pub fn remove_source_beside_blocking(
        &self,
        source: &Path,
        source_mark: &Mark,
        kept: &Path,
        kept_mark: &Mark,
    ) -> AsideRemoval {
        let mut stuck = match self.remove_source_beside_once(source, source_mark, kept, kept_mark) {
            Ok(answer) => return answer,
            Err(stuck) => *stuck,
        };
        for pause in PUT_BACK_RETRY_PAUSES {
            self.inner.clock.pause(pause);
            let attempt = self.put_back_attempt(source, source_mark, &stuck.at);
            stuck = match stuck.after(source, attempt) {
                Ok(answer) => return answer,
                Err(stuck) => stuck,
            };
        }
        stuck.left_aside(source)
    }

    /// [`Self::remove_source_beside_blocking`] from the async runtime: each
    /// filesystem step runs on a blocking thread, and the pauses between
    /// put-back retries hold no thread.
    pub async fn remove_source_beside(
        &self,
        source: &Path,
        source_mark: &Mark,
        kept: &Path,
        kept_mark: &Mark,
    ) -> AsideRemoval {
        let once = {
            let (disk, source, source_mark, kept, kept_mark) = (
                self.clone(),
                source.to_path_buf(),
                source_mark.clone(),
                kept.to_path_buf(),
                kept_mark.clone(),
            );
            blocking(move || {
                disk.remove_source_beside_once(&source, &source_mark, &kept, &kept_mark)
            })
            .await
        };
        let mut stuck = match once {
            Ok(answer) => return answer,
            Err(stuck) => *stuck,
        };
        for pause in PUT_BACK_RETRY_PAUSES {
            tokio::time::sleep(pause).await;
            let (disk, owned_source, owned_mark, at) = (
                self.clone(),
                source.to_path_buf(),
                source_mark.clone(),
                stuck.at.clone(),
            );
            let attempt =
                blocking(move || disk.put_back_attempt(&owned_source, &owned_mark, &at)).await;
            stuck = match stuck.after(source, attempt) {
                Ok(answer) => return answer,
                Err(stuck) => stuck,
            };
        }
        stuck.left_aside(source)
    }

    /// Renames the set-aside file back while the set-aside name still holds
    /// what `source_mark` marks. Where it does not, the source's name is read:
    /// a rename that took effect though it answered a failure left the file
    /// there. A regular file of the source's length at the set-aside name,
    /// with the source's name missing, is renamed back too: a volume that
    /// makes up a file number per name reads the set-aside file as another,
    /// and the rename never replaces anything. The file has moved only when
    /// the set-aside name is missing or holds something of another kind or
    /// length, and the source's name does not hold it. One read of the
    /// set-aside name and one rename, or two reads, or two reads and one
    /// rename.
    fn put_back_attempt(&self, source: &Path, source_mark: &Mark, aside: &Path) -> Attempt {
        let still_aside = match self.fingerprint_blocking(aside) {
            Ok(read) if source_mark.still_marks(&read) => return self.rename_back(aside, source),
            Ok(read) => {
                read.kind() == Some(EntryKind::File)
                    && source_mark.len().is_some_and(|len| read.is_len(len))
            }
            Err(PathReading::Missing { .. }) => false,
            Err(_) => return Attempt::Failed(None),
        };
        match self.fingerprint_blocking(source) {
            Ok(read) if source_mark.still_marks(&read) => Attempt::PutBack,
            Err(PathReading::Missing { .. }) if still_aside => self.rename_back(aside, source),
            _ if still_aside => Attempt::Failed(None),
            _ => Attempt::Moved,
        }
    }

    /// One rename of the set-aside file back to the source's name, never
    /// replacing anything.
    fn rename_back(&self, aside: &Path, source: &Path) -> Attempt {
        match self.move_exclusive_blocking(aside, source) {
            Ok(_) => Attempt::PutBack,
            Err(error) => Attempt::Failed(Some(error)),
        }
    }

    fn remove_source_beside_once(
        &self,
        source: &Path,
        source_mark: &Mark,
        kept: &Path,
        kept_mark: &Mark,
    ) -> Result<AsideRemoval, Box<Stuck>> {
        let aside = aside_path_for(source);
        if let Err(error) = self.move_exclusive_blocking(source, &aside) {
            match self.clear_reservation_and_set_aside(source, &aside, &error) {
                Some(Ok(())) => {}
                Some(Err(again)) => {
                    return self.after_refused_set_aside(source, source_mark, aside, again);
                }
                None => return self.after_refused_set_aside(source, source_mark, aside, error),
            }
        }

        let kept_now = match self.fingerprint_blocking(kept) {
            Ok(now) => now,
            Err(PathReading::Missing { .. }) => {
                return self.put_back(source, aside, AsideRemoval::SameEntry);
            }
            Err(reading) => {
                return self.put_back(source, aside, AsideRemoval::DestinationUnread(reading));
            }
        };
        if !kept_now.has_other_names() {
            match kept_now.numbered_as(source_mark) {
                Sameness::Same => return self.put_back(source, aside, AsideRemoval::SameEntry),
                Sameness::CannotTell => {
                    return self.put_back(source, aside, AsideRemoval::CannotTell);
                }
                Sameness::Different => {}
            }
        }
        if !kept_mark.still_marks(&kept_now) {
            return self.put_back(source, aside, AsideRemoval::DestinationChanged);
        }
        if kept_mark.kind() == Some(EntryKind::Symlink) {
            match self.fingerprint_followed_blocking(kept) {
                Ok(_) => {}
                Err(PathReading::Missing { .. }) => {
                    return self.put_back(source, aside, AsideRemoval::SameEntry);
                }
                Err(reading) => {
                    return self.put_back(source, aside, AsideRemoval::DestinationUnread(reading));
                }
            }
        }
        if (!kept_now.numbered_on_one_volume(source_mark) || on_a_share(kept) || on_a_share(source))
            && let Some(answer) = self.unlisted(kept)
        {
            return self.put_back(source, aside, answer);
        }

        match self.remove_blocking(&aside, Remove::File) {
            Ok(_) => Ok(AsideRemoval::Removed),
            Err(error) => match self.fingerprint_blocking(&aside) {
                // The removal may have taken effect though it answered a
                // failure, or something may have put the source back: the
                // source's own name tells which. Missing there too, the
                // removal took effect.
                Err(PathReading::Missing { .. }) => match self.fingerprint_blocking(source) {
                    Err(PathReading::Missing { .. }) => {
                        tracing::info!(
                            source = %source.display(),
                            at = %aside.display(),
                            error = %error,
                            "removing a move's set-aside source answered a failure, but neither name holds it any more; it was removed"
                        );
                        Ok(AsideRemoval::Removed)
                    }
                    _ => Ok(AsideRemoval::RemovalUnconfirmed(error)),
                },
                _ => self.put_back(source, aside, AsideRemoval::NotRemoved(error)),
            },
        }
    }

    /// Whether `kept`'s folder lists `kept`'s name byte for byte: `None` when
    /// it does, else why the source goes back. A name not listed as spelled
    /// cannot tell: the folder may list it in another case or normalization.
    fn unlisted(&self, kept: &Path) -> Option<AsideRemoval> {
        let (Some(folder), Some(name)) = (kept.parent(), kept.file_name()) else {
            return Some(AsideRemoval::CannotTell);
        };
        match self.list_blocking(folder) {
            Ok(listed) => match listed
                .iter()
                .any(|entry| entry.path.file_name() == Some(name))
            {
                true => None,
                false => Some(AsideRemoval::CannotTell),
            },
            Err(error) => Some(AsideRemoval::DestinationUnread(PathReading::Unreachable {
                observed_at: self.inner.clock.now(),
                cause: error.reason(),
            })),
        }
    }

    /// After a set-aside rename refused because the set-aside name is taken:
    /// when an empty regular file holds that name, while the source is still
    /// at its own name, it is a reservation an interrupted set-aside left, or
    /// an empty source set aside earlier. It holds no data, so it is removed
    /// and the set-aside is tried once more. `None` where that does not
    /// apply, or the removal failed; a nonempty file there is never touched.
    /// One read and one removal of the set-aside name, and one rename, on
    /// that failure alone.
    fn clear_reservation_and_set_aside(
        &self,
        source: &Path,
        aside: &Path,
        error: &DiskError,
    ) -> Option<Result<(), DiskError>> {
        if error.left().is_some() || error.reason() != reason::DISK_DESTINATION_EXISTS {
            return None;
        }
        match self.fingerprint_blocking(aside) {
            Ok(read) if read.kind() == Some(EntryKind::File) && read.is_len(0) => {}
            _ => return None,
        }
        self.remove_blocking(aside, Remove::File).ok()?;
        tracing::info!(
            at = %aside.display(),
            "an empty file left at a move's set-aside name was removed"
        );
        Some(self.move_exclusive_blocking(source, aside).map(|_| ()))
    }

    /// After a set-aside rename failed. A taken set-aside name is named. A
    /// failure that left something at the set-aside name may have moved the
    /// source there and lost the reply, or left only a reservation of its
    /// own: the set-aside name is read once, and the source is put back when
    /// that name holds the source's file. Where it holds another nonempty
    /// regular file, the source is read once too: a source that reads
    /// missing went to the set-aside name under a number its volume made up
    /// for the new name, and is put back. An empty file there is a
    /// reservation and is never put back as the source. A put-back never
    /// replaces anything.
    fn after_refused_set_aside(
        &self,
        source: &Path,
        source_mark: &Mark,
        aside: PathBuf,
        error: DiskError,
    ) -> Result<AsideRemoval, Box<Stuck>> {
        if error.left().is_none() {
            if error.reason() == reason::DISK_DESTINATION_EXISTS {
                tracing::warn!(
                    source = %source.display(),
                    at = %aside.display(),
                    "a move's set-aside name holds a file while the source is at its own name; both are kept"
                );
                return Ok(AsideRemoval::AsideNameTaken { at: aside, error });
            }
            return Ok(AsideRemoval::NotSetAside(error));
        }
        let at_aside = match self.fingerprint_blocking(&aside) {
            Ok(read) if read.kind() == Some(EntryKind::File) && !read.is_len(0) => read,
            _ => return Ok(AsideRemoval::NotSetAside(error)),
        };
        if source_mark.still_marks(&at_aside)
            || matches!(
                self.fingerprint_blocking(source),
                Err(PathReading::Missing { .. })
            )
        {
            return self.put_back(source, aside, AsideRemoval::NotSetAside(error));
        }
        Ok(AsideRemoval::NotSetAside(error))
    }

    /// Puts back a source an interrupted removal left at its set-aside name,
    /// for an import that finds the source missing. Only a nonempty regular
    /// file at the set-aside name is moved, and only by a rename that never
    /// replaces anything. The caller keeps this from running while a removal
    /// of the same source is in flight.
    ///
    /// An empty file there is a reservation a set-aside left, never a source:
    /// a rename that reserves its name first can leave it behind when the
    /// source vanished before the rename. It holds no data, so it is removed,
    /// on a fresh read that says it is a regular empty file immediately
    /// before the removal, whether the source's name is missing or taken.
    ///
    /// A source name that is taken again keeps what is there, and a nonempty
    /// file at the set-aside name is never removed and is named for the
    /// operator.
    ///
    /// Probes: one read of the set-aside name, then one rename when it holds
    /// a nonempty regular file, or one removal when it holds an empty one. A
    /// rename that fails reads the set-aside name once more.
    pub fn put_back_set_aside_source_blocking(&self, source: &Path) -> PutBackAside {
        let aside = aside_path_for(source);
        match self.fingerprint_blocking(&aside) {
            Ok(read) if read.kind() == Some(EntryKind::File) && read.is_len(0) => {
                match self.remove_blocking(&aside, Remove::File) {
                    Ok(_) => tracing::info!(
                        at = %aside.display(),
                        "an empty file left at a move's set-aside name was removed"
                    ),
                    Err(error) => tracing::warn!(
                        at = %aside.display(),
                        error = %error,
                        "an empty file left at a move's set-aside name could not be removed"
                    ),
                }
                return PutBackAside::Nothing;
            }
            Ok(read) if read.kind() == Some(EntryKind::File) => {}
            _ => return PutBackAside::Nothing,
        }
        match self.move_exclusive_blocking(&aside, source) {
            Ok(_) => {
                tracing::warn!(
                    source = %source.display(),
                    from = %aside.display(),
                    "an import source an interrupted cleanup set aside was put back"
                );
                PutBackAside::PutBack
            }
            Err(error) if error.reason() == reason::DISK_DESTINATION_EXISTS => {
                tracing::warn!(
                    source = %source.display(),
                    at = %aside.display(),
                    "an import source an interrupted cleanup set aside is at its set-aside name, and the source's name is taken again; both are kept"
                );
                PutBackAside::SourceNameTaken { at: aside }
            }
            // The file is named as still there only on a fresh read that
            // finds it: a rename that failed because it went elsewhere
            // leaves nothing here to wait on.
            Err(error) => match self.fingerprint_blocking(&aside) {
                Ok(read) if read.kind() == Some(EntryKind::File) && !read.is_len(0) => {
                    PutBackAside::Failed { at: aside, error }
                }
                _ => PutBackAside::Nothing,
            },
        }
    }

    /// Renames the set-aside source back, never replacing anything, and
    /// answers `then`; or what failed, for the caller to retry.
    fn put_back(
        &self,
        source: &Path,
        aside: PathBuf,
        then: AsideRemoval,
    ) -> Result<AsideRemoval, Box<Stuck>> {
        match self.move_exclusive_blocking(&aside, source) {
            Ok(_) => Ok(then),
            Err(error) => {
                tracing::warn!(
                    source = %source.display(),
                    at = %aside.display(),
                    error = %error,
                    "a move's source set aside could not be put back; retrying"
                );
                Err(Box::new(Stuck {
                    at: aside,
                    error,
                    then,
                }))
            }
        }
    }
}

#[cfg(all(test, windows))]
mod share_spellings {
    use super::on_a_share;
    use std::path::Path;

    #[test]
    fn every_unc_spelling_is_a_share_and_a_drive_is_not() {
        for share in [
            r"\\server\share\Show\episode.mkv",
            r"\\?\UNC\server\share\Show\episode.mkv",
            r"\\.\UNC\server\share\Show\episode.mkv",
            r"\\.\unc\server\share\Show\episode.mkv",
        ] {
            assert!(on_a_share(Path::new(share)), "{share}");
        }
        for local in [
            r"C:\Library\Show\episode.mkv",
            r"\\?\C:\Library\Show\episode.mkv",
            r"\\.\C:\Library\Show\episode.mkv",
            r"\\.\PhysicalDrive0",
            r"Library\Show\episode.mkv",
        ] {
            assert!(!on_a_share(Path::new(local)), "{local}");
        }
    }
}
