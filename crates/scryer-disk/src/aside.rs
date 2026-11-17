//! Removing a move's source once a destination that was already there has
//! been adopted in its place. The paths alone cannot show that the two are
//! separate entries: a linked or second-mounted folder, another spelling of
//! one name, or a link at the destination can make the destination the
//! source's own entry, and removing the source would then remove the only
//! copy. So the filesystem is asked: the source is renamed aside first, and
//! removed only while the destination still holds the file it was adopted as.

use std::path::{Path, PathBuf};

use super::{DiskError, EntryKind, Mark, PathReading, Remove};

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
    /// The destination did not read once the source was aside, so nothing
    /// says whether the two are one entry. The source was put back and
    /// nothing was removed.
    DestinationUnread(PathReading),
    /// The destination checked out, but removing the set-aside name failed.
    /// The source was put back.
    NotRemoved(DiskError),
    /// The source could not be renamed aside, a name left at the set-aside
    /// name by an earlier attempt included. The source is where it was.
    NotSetAside(DiskError),
    /// Putting the source back failed, so its file is at `at`, the
    /// set-aside name in the source's own folder.
    PutBackFailed { at: PathBuf, error: DiskError },
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
    /// `kept_mark` is the read of `kept`, links not followed, taken when it
    /// was adopted. The caller has just proved `source` is still the file it
    /// read.
    ///
    /// 1. The source is renamed, never replacing anything, to
    ///    [`aside_path_for`]. A refused rename leaves it where it was.
    /// 2. `kept` is read, links not followed. A link is also read followed,
    ///    so a link that reached the source shows it no longer does.
    /// 3. While `kept` still holds the file `kept_mark` marks, the set-aside
    ///    name is removed. Otherwise the source is renamed back, never
    ///    replacing anything.
    ///
    /// Probes: one rename and one read of `kept`, a second read when `kept`
    /// is a link, then one removal; a put-back adds one rename.
    pub fn remove_source_beside_blocking(
        &self,
        source: &Path,
        kept: &Path,
        kept_mark: &Mark,
    ) -> AsideRemoval {
        let aside = aside_path_for(source);
        if let Err(error) = self.move_exclusive_blocking(source, &aside) {
            return self.after_refused_set_aside(source, aside, error);
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

        match self.remove_blocking(&aside, Remove::File) {
            Ok(_) => AsideRemoval::Removed,
            Err(error) => self.put_back(source, aside, AsideRemoval::NotRemoved(error)),
        }
    }

    /// After a set-aside rename failed. A failure that left something at the
    /// set-aside name may have moved the source there and lost the reply, so
    /// the source is read once and, when it is not where it was, put back.
    fn after_refused_set_aside(
        &self,
        source: &Path,
        aside: PathBuf,
        error: DiskError,
    ) -> AsideRemoval {
        if error.left().is_none() {
            return AsideRemoval::NotSetAside(error);
        }
        if self.fingerprint_blocking(source).is_ok() {
            return AsideRemoval::NotSetAside(error);
        }
        self.put_back(source, aside, AsideRemoval::NotSetAside(error))
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
