//! Removing a move's source once a destination that was already there has
//! been adopted in its place. The paths alone cannot show that the two are
//! separate entries: a linked or second-mounted folder, another spelling of
//! one name, or a link at the destination can make the destination the
//! source's own entry, and removing the source would then remove the only
//! copy. So the filesystem is asked: the source is renamed aside first, and
//! removed only while the destination still holds the file it was adopted as.

use std::path::{Path, PathBuf};

use super::{DiskError, EntryKind, Mark, PathReading, Remove, Sameness, reason};

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
    /// failure, and the name then read missing: it may have been removed, or
    /// another import may have put the source back meanwhile. Nothing more
    /// was done; where the file is was not read.
    RemovalUnconfirmed(DiskError),
    /// The source could not be renamed aside. The source is where it was.
    NotSetAside(DiskError),
    /// The set-aside name `at` is already taken, by a file an interrupted
    /// attempt left there or by anything else, so the source was not set
    /// aside. The source is where it was, and nothing at `at` was touched.
    AsideNameTaken { at: PathBuf, error: DiskError },
    /// Putting the source back failed, so its file is at `at`, the
    /// set-aside name in the source's own folder.
    PutBackFailed { at: PathBuf, error: DiskError },
}

/// What [`super::Disk::put_back_set_aside_source_blocking`] did.
#[derive(Debug)]
pub enum PutBackAside {
    /// The set-aside name held a regular file and it was renamed back to
    /// the source's name.
    PutBack,
    /// There was nothing to put back: no regular file at the set-aside name,
    /// or the source's name is taken. Nothing was moved.
    Nothing,
    /// Renaming it back failed; it is still at `at`.
    Failed { at: PathBuf, error: DiskError },
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
    /// 4. Where `kept` and the source are not on one known volume, `kept`'s
    ///    folder is listed once and must hold `kept`'s name, byte for byte.
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
    /// for as long as the view's cache holds it, and on one known volume no
    /// listing is taken. A destination that is a second hard link of the
    /// source's file elsewhere is kept and the source goes: the library
    /// entry for the source's name is gone, the data is not.
    ///
    /// Probes, for one adopted destination: one rename and one read of
    /// `kept`, a second read when `kept` is a link, then one removal. Where
    /// the two are not on one known volume, one listing of `kept`'s folder
    /// besides. A put-back adds one rename.
    pub fn remove_source_beside_blocking(
        &self,
        source: &Path,
        source_mark: &Mark,
        kept: &Path,
        kept_mark: &Mark,
    ) -> AsideRemoval {
        let aside = aside_path_for(source);
        if let Err(error) = self.move_exclusive_blocking(source, &aside) {
            return self.after_refused_set_aside(source, source_mark, aside, error);
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
        if kept_now.same_volume(source_mark) != Sameness::Same
            && let Some(answer) = self.unlisted(kept)
        {
            return self.put_back(source, aside, answer);
        }

        match self.remove_blocking(&aside, Remove::File) {
            Ok(_) => AsideRemoval::Removed,
            Err(error) => match self.fingerprint_blocking(&aside) {
                // The removal may have taken effect though it answered a
                // failure, or another import may have put the source back:
                // gone from the set-aside name is no proof of either.
                Err(PathReading::Missing { .. }) => AsideRemoval::RemovalUnconfirmed(error),
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
    ) -> AsideRemoval {
        if error.left().is_none() {
            if error.reason() == reason::DISK_DESTINATION_EXISTS {
                return AsideRemoval::AsideNameTaken { at: aside, error };
            }
            return AsideRemoval::NotSetAside(error);
        }
        let at_aside = match self.fingerprint_blocking(&aside) {
            Ok(read) if read.kind() == Some(EntryKind::File) && !read.is_len(0) => read,
            _ => return AsideRemoval::NotSetAside(error),
        };
        if source_mark.still_marks(&at_aside)
            || matches!(
                self.fingerprint_blocking(source),
                Err(PathReading::Missing { .. })
            )
        {
            return self.put_back(source, aside, AsideRemoval::NotSetAside(error));
        }
        AsideRemoval::NotSetAside(error)
    }

    /// Puts back a source an interrupted removal left at its set-aside name,
    /// for an import that finds the source missing. Only a nonempty regular
    /// file at the set-aside name is moved, and only by a rename that never
    /// replaces anything: a source name that is taken again keeps what is
    /// there, and an empty file is a reservation, never the source. Nothing
    /// is removed. The caller keeps this from running while a removal of the
    /// same source is in flight.
    ///
    /// Probes: one read of the set-aside name, and one rename when it holds
    /// a regular file.
    pub fn put_back_set_aside_source_blocking(&self, source: &Path) -> PutBackAside {
        let aside = aside_path_for(source);
        match self.fingerprint_blocking(&aside) {
            Ok(read) if read.kind() == Some(EntryKind::File) && !read.is_len(0) => {}
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
                PutBackAside::Nothing
            }
            Err(error) => PutBackAside::Failed { at: aside, error },
        }
    }

    /// Renames the set-aside source back, never replacing anything, and
    /// answers `then`; or [`AsideRemoval::PutBackFailed`].
    fn put_back(&self, source: &Path, aside: PathBuf, then: AsideRemoval) -> AsideRemoval {
        match self.move_exclusive_blocking(&aside, source) {
            Ok(_) => then,
            Err(error) => {
                tracing::error!(
                    source = %source.display(),
                    at = %aside.display(),
                    error = %error,
                    "a move's source set aside could not be put back; it is left at its set-aside name"
                );
                AsideRemoval::PutBackFailed { at: aside, error }
            }
        }
    }
}
