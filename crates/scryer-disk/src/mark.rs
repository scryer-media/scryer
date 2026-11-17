//! What one read of a file said about which file it is and which version of
//! it, kept opaque. A caller holds a [`Mark`] and asks what it means: is this
//! path still that file, are these two one file, are they on one volume. No
//! number in it is handed out.
//!
//! A mark carries whatever the platform offered: the device and inode on
//! unix, a zero inode included; the volume serial alone on Windows, which
//! tells no two files apart; and length and times everywhere. Each answer
//! says when the platform's facts were not enough to tell.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::backend::RawStat;
use super::reading::EntryKind;

/// Whether two marks are one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sameness {
    Same,
    Different,
    /// The platform gave no file number for one of them, or gave zero for
    /// both, which some network and FUSE filesystems report for every file.
    CannotTell,
}

/// Whether two paths name one directory entry, so that removing the entry
/// one names removes what the other names too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryAlias {
    /// One entry, or two the paths alone cannot tell apart: reached through
    /// another spelling of its folder (a link or a second mount on the way),
    /// under a second spelling of its name, as one file with no other name,
    /// or by a link at the second path that reaches it.
    Same,
    /// Two entries, whatever file they hold: two names of one file in two
    /// places are two entries, and so are two links to one file.
    Distinct,
    /// A read the answer needed failed, or a path names no entry of its own
    /// (it ends in `..`). Nothing is guessed.
    CannotTell,
}

/// How many links a link at the destination is followed through before the
/// answer is given up, as the platforms give up resolving one.
const MOST_HOPS: usize = 40;

impl super::Disk {
    /// Whether `dest` names the directory entry `source` names. `dest_read`
    /// is the caller's read of `dest` itself, links not followed.
    ///
    /// The answer never says two entries where the paths, resolved, are one:
    ///
    /// - Paths that match as written, ignoring case and Unicode form, are
    ///   one entry, read nothing.
    /// - One file that the reads do not show to have another name is one
    ///   entry wherever its two paths are: one name gives one entry.
    /// - Otherwise names that differ, ignoring case and Unicode form, are two
    ///   entries: two names of one file, or two files.
    /// - Names that agree are one entry when their folders are one folder by
    ///   the folders' own file numbers, as two mounts of one volume show it,
    ///   or when the folders resolve to paths that match. The resolution is
    ///   made only when the numbers did not say one folder, so that a
    ///   filesystem that numbers each path anew (CIFS without `serverino`)
    ///   still shows one entry.
    ///
    /// When `dest_read` is a link, it is followed one link at a time, each
    /// step judged by the same rule, so a link to a link that is the source
    /// is the source's entry. A step that is missing ends the chain as
    /// another entry. A source that is itself a link is its own entry.
    ///
    /// A read that fails, other than a step of the chain that is missing,
    /// answers [`EntryAlias::CannotTell`], as does a path ending in `..`.
    ///
    /// Reads: none when the paths match as written. Otherwise one read of
    /// the source, unless `dest_read` already shows another name. Names that
    /// agree add a stat of each folder, and two resolutions when the folders'
    /// numbers did not say one folder. Each link followed adds a read of the
    /// link, a read of what it names, and the same reads again for that.
    pub fn entry_alias_blocking(&self, source: &Path, dest: &Path, dest_read: &Mark) -> EntryAlias {
        let Some(source_entry) = Entry::of(source) else {
            return EntryAlias::CannotTell;
        };
        let mut answer = self.one_entry(&source_entry, dest, dest_read);
        let mut at = dest.to_path_buf();
        let mut read = dest_read.clone();
        for _ in 0..MOST_HOPS {
            if answer != EntryAlias::Distinct || read.kind() != Some(EntryKind::Symlink) {
                return answer;
            }
            let Ok(target) = self.read_link_blocking(&at) else {
                return EntryAlias::CannotTell;
            };
            // A relative target is read from the link's own folder, as the
            // platform resolves it.
            at = match at.parent() {
                Some(folder) if target.is_relative() => folder.join(target),
                _ => target,
            };
            read = match self.fingerprint_blocking(&at) {
                Ok(read) => read,
                Err(super::PathReading::Missing { .. }) => return EntryAlias::Distinct,
                Err(_) => return EntryAlias::CannotTell,
            };
            answer = self.one_entry(&source_entry, &at, &read);
        }
        match answer {
            EntryAlias::Distinct if read.kind() == Some(EntryKind::Symlink) => {
                EntryAlias::CannotTell
            }
            answer => answer,
        }
    }

    /// Whether `other` names `entry`'s directory entry. `other_read` is a
    /// read of `other` itself, links not followed.
    fn one_entry(&self, entry: &Entry<'_>, other: &Path, other_read: &Mark) -> EntryAlias {
        let (Some(other_folder), Some(other_name)) = (folder_of(other), other.file_name()) else {
            return EntryAlias::CannotTell;
        };
        let paths_match = super::place::paths_are_same_file;
        if paths_match(entry.path, other) {
            return EntryAlias::Same;
        }
        if !other_read.has_other_names() {
            let Some(read) = entry.read(self) else {
                return EntryAlias::CannotTell;
            };
            if read.same_file(other_read) == Sameness::Same {
                return EntryAlias::Same;
            }
        }
        if !paths_match(Path::new(entry.name), Path::new(other_name)) {
            return EntryAlias::Distinct;
        }
        let Some(folder_read) = entry.folder_read(self) else {
            return EntryAlias::CannotTell;
        };
        let Ok(other_folder_read) = self.fingerprint_followed_blocking(other_folder) else {
            return EntryAlias::CannotTell;
        };
        if folder_read.same_file(&other_folder_read) == Sameness::Same {
            return EntryAlias::Same;
        }
        let (Some(resolved), Ok(other_resolved)) = (
            entry.resolved_folder(self),
            self.canonicalize_blocking(other_folder),
        ) else {
            return EntryAlias::CannotTell;
        };
        match paths_match(&resolved.join(entry.name), &other_resolved.join(other_name)) {
            true => EntryAlias::Same,
            false => EntryAlias::Distinct,
        }
    }
}

/// The source's side of the question, each read made once and only when
/// asked; `None` is a read that failed.
struct Entry<'a> {
    path: &'a Path,
    name: &'a std::ffi::OsStr,
    folder: &'a Path,
    read: std::cell::OnceCell<Option<Mark>>,
    folder_read: std::cell::OnceCell<Option<Mark>>,
    resolved_folder: std::cell::OnceCell<Option<std::path::PathBuf>>,
}

impl<'a> Entry<'a> {
    fn of(path: &'a Path) -> Option<Self> {
        Some(Self {
            path,
            name: path.file_name()?,
            folder: folder_of(path)?,
            read: std::cell::OnceCell::new(),
            folder_read: std::cell::OnceCell::new(),
            resolved_folder: std::cell::OnceCell::new(),
        })
    }

    fn read(&self, disk: &super::Disk) -> Option<&Mark> {
        self.read
            .get_or_init(|| disk.fingerprint_blocking(self.path).ok())
            .as_ref()
    }

    fn folder_read(&self, disk: &super::Disk) -> Option<&Mark> {
        self.folder_read
            .get_or_init(|| disk.fingerprint_followed_blocking(self.folder).ok())
            .as_ref()
    }

    fn resolved_folder(&self, disk: &super::Disk) -> Option<&Path> {
        self.resolved_folder
            .get_or_init(|| disk.canonicalize_blocking(self.folder).ok())
            .as_deref()
    }
}

/// The folder a path's entry is in; `.` for a bare name.
fn folder_of(path: &Path) -> Option<&Path> {
    path.file_name()?;
    Some(
        path.parent()
            .filter(|folder| !folder.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )
}

/// What became of a marked file, from the strongest finding down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drift {
    /// The same file, and nothing read about it differs.
    Same,
    /// Nothing read about it differs, but the platform could not tell
    /// whether it is the same file.
    SameFacts,
    /// Its length, modification time and owner held; its change time or
    /// creation time did not.
    Touched,
    /// Its length and modification time held; its owner did not.
    Reowned,
    /// Its length or modification time differs.
    Rewritten,
    /// A different file.
    Replaced,
    /// One of the marks names a volume only, so there is nothing to compare.
    CannotTell,
}

impl Drift {
    /// Nothing read about the file differs. `false` when that is not shown.
    pub fn is_untouched(self) -> bool {
        matches!(self, Self::Same | Self::SameFacts)
    }

    /// Its length and modification time held, and it was not shown to be
    /// another file. `false` when that is not shown.
    pub fn is_unwritten(self) -> bool {
        self.is_unwritten_by_its_owner() || self == Self::Reowned
    }

    /// [`Self::is_unwritten`], and its owner held too.
    pub fn is_unwritten_by_its_owner(self) -> bool {
        matches!(self, Self::Same | Self::SameFacts | Self::Touched)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Volume {
    Unix(u64),
    Windows(u64),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Facts {
    kind: EntryKind,
    len: u64,
    modified: Option<SystemTime>,
    changed: Option<(i64, i64)>,
    created: Option<SystemTime>,
    owner: Option<u32>,
    /// How many names the file has, where the read said; never stored.
    links: Option<u64>,
}

/// One read of a file, opaque. See the module text.
#[derive(Clone, Debug)]
pub struct Mark {
    volume: Volume,
    file: Option<u64>,
    /// Absent for a mark that names a volume only.
    facts: Option<Facts>,
}

const STORED_TAG: &str = "mark1";
const UNIX_LANE: &str = "unix-dev:";
const WINDOWS_LANE: &str = "windows-volume:";
pub(crate) const UNKNOWN_LANE: &str = "unknown-volume";

impl Mark {
    pub(crate) fn of_stat(stat: &RawStat) -> Self {
        let (volume, file) = match stat.extra.device {
            Some(device) => (Volume::Unix(device), stat.extra.inode),
            None => (Volume::Unknown, None),
        };
        Self {
            volume,
            file,
            facts: Some(Facts {
                kind: stat.kind,
                len: stat.len,
                modified: stat.modified,
                changed: stat.extra.changed,
                created: stat.extra.created,
                owner: stat.extra.owner,
                links: stat.links,
            }),
        }
    }

    /// The mark of the volume the platform numbers `number`, a unix device
    /// or a Windows volume serial. No probe. Only for the import worker,
    /// whose payload carries that number until it carries a mark instead.
    #[doc(hidden)]
    pub fn on_volume(number: u64) -> Self {
        Self {
            volume: if cfg!(unix) {
                Volume::Unix(number)
            } else if cfg!(windows) {
                Volume::Windows(number)
            } else {
                Volume::Unknown
            },
            file: None,
            facts: None,
        }
    }

    /// The mark of metadata a caller already holds from an open handle. No
    /// probe.
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        Self::of_stat(&RawStat::of(metadata))
    }

    /// Whether the read named the volume the path is on.
    pub fn names_volume(&self) -> bool {
        self.volume != Volume::Unknown
    }

    /// Whether both marks are one file. A volume the read did not name, a
    /// missing file number, or a zero number on both sides is
    /// [`Sameness::CannotTell`], never [`Sameness::Same`]. No probe.
    pub fn same_file(&self, other: &Self) -> Sameness {
        match (self.volume, other.volume) {
            (Volume::Unknown, _) | (_, Volume::Unknown) => return Sameness::CannotTell,
            (left, right) if left != right => return Sameness::Different,
            _ => {}
        }
        match (self.file, other.file) {
            (None, _) | (_, None) | (Some(0), Some(0)) => Sameness::CannotTell,
            (Some(left), Some(right)) if left == right => Sameness::Same,
            _ => Sameness::Different,
        }
    }

    /// Whether this mark, read of a name just hard-linked, is the file `seen`
    /// was read as before the link. Where the platform numbers files, the
    /// device and the number must match, and a zero number is compared as
    /// given: a filesystem that numbers every file zero links truly. Where
    /// it gives none, the length and modification time must match. `seen`
    /// must be a fresh read: a stored or volume-only mark never proves. No
    /// probe.
    pub(crate) fn is_link_of(&self, seen: &Self) -> bool {
        let (Some(linked), Some(before)) = (self.facts, seen.facts) else {
            return false;
        };
        if linked.kind != EntryKind::File {
            return false;
        }
        match (self.volume, seen.volume) {
            (Volume::Unix(here), Volume::Unix(there)) => {
                here == there && self.file.is_some() && self.file == seen.file
            }
            (Volume::Unknown, Volume::Unknown) => {
                linked.len == before.len && linked.modified == before.modified
            }
            _ => false,
        }
    }

    /// Whether this mark and `seen` were both read in full and differ in
    /// length or modification time. A hard link shares both with its source,
    /// so a name that differs so is somebody else's file. No probe.
    pub(crate) fn differs_in_content_facts(&self, seen: &Self) -> bool {
        matches!(
            (self.facts, seen.facts),
            (Some(here), Some(there))
                if here.len != there.len || here.modified != there.modified
        )
    }

    /// The length read; `None` for a mark that names a volume only.
    pub(crate) fn len(&self) -> Option<u64> {
        self.facts.map(|facts| facts.len)
    }

    /// Whether both marks are on one volume, so they share its space. A
    /// rename between them can still be refused: two bind mounts of one
    /// device do not rename across. No probe.
    pub fn same_volume(&self, other: &Self) -> Sameness {
        match (self.volume, other.volume) {
            (Volume::Unknown, _) | (_, Volume::Unknown) => Sameness::CannotTell,
            (left, right) if left == right => Sameness::Same,
            _ => Sameness::Different,
        }
    }

    /// What differs between this mark and a later one of the same path. No
    /// probe. What kind of entry it is does not enter into it: one file
    /// cannot change kind, and where the platform cannot tell files apart, a
    /// caller that cares checks [`Self::kind`] itself.
    pub fn compare(&self, later: &Self) -> Drift {
        let (Some(before), Some(after)) = (self.facts, later.facts) else {
            return Drift::CannotTell;
        };
        let same = self.same_file(later);
        if same == Sameness::Different {
            return Drift::Replaced;
        }
        if before.len != after.len || before.modified != after.modified {
            Drift::Rewritten
        } else if before.owner != after.owner {
            Drift::Reowned
        } else if before.changed != after.changed || before.created != after.created {
            Drift::Touched
        } else if same == Sameness::Same {
            Drift::Same
        } else {
            Drift::SameFacts
        }
    }

    /// This mark without the link count it was read with, which is never
    /// stored.
    #[cfg(test)]
    pub(crate) fn as_stored(&self) -> Self {
        let mut mark = self.clone();
        if let Some(facts) = &mut mark.facts {
            facts.links = None;
        }
        mark
    }

    /// Whether the read said the file has a name besides this one.
    pub(crate) fn has_other_names(&self) -> bool {
        self.facts
            .is_some_and(|facts| facts.links.is_some_and(|links| links > 1))
    }

    /// What was at the path; `None` for a mark that names a volume only.
    pub fn kind(&self) -> Option<EntryKind> {
        self.facts.map(|facts| facts.kind)
    }

    /// Whether the file was exactly `len` bytes long when read.
    pub fn is_len(&self, len: u64) -> bool {
        self.facts.is_some_and(|facts| facts.len == len)
    }

    /// Whether the file was longer than `limit` bytes when read.
    pub fn exceeds(&self, limit: u64) -> bool {
        self.facts.is_some_and(|facts| facts.len > limit)
    }

    /// A mark of the volume alone, for a caller that keeps where something
    /// is without keeping which file it was.
    pub fn volume(&self) -> Self {
        Self {
            volume: self.volume,
            file: None,
            facts: None,
        }
    }

    /// The copy lane for this mark's volume.
    pub(crate) fn lane(&self) -> String {
        match self.volume {
            Volume::Unix(device) => format!("{UNIX_LANE}{device}"),
            Volume::Windows(serial) => format!("{WINDOWS_LANE}{serial}"),
            Volume::Unknown => UNKNOWN_LANE.to_owned(),
        }
    }

    /// The mark in a few words for a log line: its device and file number
    /// where the platform gave them, its length and modification time
    /// otherwise. For a person; nothing reads it back.
    pub fn label(&self) -> String {
        match (self.volume, self.file, self.facts) {
            (Volume::Unix(device), Some(file), _) => format!("dev={device} ino={file}"),
            (_, _, Some(facts)) => {
                let modified = facts.modified.map(since_epoch).map_or_else(
                    || "unknown".to_owned(),
                    |(seconds, nanos)| {
                        let sign = if seconds < 0 || nanos < 0 { "-" } else { "" };
                        let (seconds, nanos) = (seconds.unsigned_abs(), nanos.unsigned_abs());
                        format!("{sign}{seconds}.{nanos:09}")
                    },
                );
                format!("len={} modified={modified}", facts.len)
            }
            _ => self.lane(),
        }
    }

    /// The mark as text, for a row, a journal or a message. Opaque: only
    /// [`Self::from_stored`] reads it. A volume-only mark is stored as its
    /// lane text.
    pub fn to_stored(&self) -> String {
        let Some(facts) = self.facts else {
            return self.lane();
        };
        let kind = match facts.kind {
            EntryKind::File => 'f',
            EntryKind::Directory => 'd',
            EntryKind::Symlink => 'l',
            EntryKind::Other => 'o',
        };
        let volume = match self.volume {
            Volume::Unix(device) => format!("u{device}"),
            Volume::Windows(serial) => format!("w{serial}"),
            Volume::Unknown => "-".to_owned(),
        };
        let number = |value: Option<u64>| value.map_or("-".to_owned(), |value| value.to_string());
        let pair = |value: Option<(i64, i64)>| {
            value.map_or("-".to_owned(), |(seconds, nanos)| {
                format!("{seconds}.{nanos}")
            })
        };
        [
            STORED_TAG.to_owned(),
            kind.to_string(),
            facts.len.to_string(),
            pair(facts.modified.map(since_epoch)),
            pair(facts.changed),
            pair(facts.created.map(since_epoch)),
            volume,
            number(self.file),
            number(facts.owner.map(u64::from)),
        ]
        .join(":")
    }

    /// The mark [`Self::to_stored`] wrote. `None` for text it did not write,
    /// another spelling of a number it writes included.
    pub fn from_stored(text: &str) -> Option<Self> {
        Self::parsed(text).filter(|mark| mark.to_stored() == text)
    }

    fn parsed(text: &str) -> Option<Self> {
        let volume_only = |volume| Self {
            volume,
            file: None,
            facts: None,
        };
        if text == UNKNOWN_LANE {
            return Some(volume_only(Volume::Unknown));
        }
        if let Some(device) = text.strip_prefix(UNIX_LANE) {
            return Some(volume_only(Volume::Unix(device.parse().ok()?)));
        }
        if let Some(serial) = text.strip_prefix(WINDOWS_LANE) {
            return Some(volume_only(Volume::Windows(serial.parse().ok()?)));
        }
        let fields: Vec<&str> = text.split(':').collect();
        let [
            STORED_TAG,
            kind,
            len,
            modified,
            changed,
            created,
            volume,
            file,
            owner,
        ] = fields[..]
        else {
            return None;
        };
        fn number<T: std::str::FromStr>(text: &str) -> Option<Option<T>> {
            if text == "-" {
                Some(None)
            } else {
                text.parse().ok().map(Some)
            }
        }
        fn pair(text: &str) -> Option<Option<(i64, i64)>> {
            if text == "-" {
                return Some(None);
            }
            let (seconds, nanos) = text.split_once('.')?;
            let nanos = nanos
                .parse()
                .ok()
                .filter(|nanos: &i64| nanos.abs() < 1_000_000_000)?;
            Some(Some((seconds.parse().ok()?, nanos)))
        }
        let time = |text| -> Option<Option<SystemTime>> {
            match pair(text)? {
                None => Some(None),
                Some(at) => from_epoch(at).map(Some),
            }
        };
        let volume = match volume.split_at_checked(1)? {
            ("-", "") => Volume::Unknown,
            ("u", device) => Volume::Unix(device.parse().ok()?),
            // No Windows mark carries a file index.
            ("w", serial) if file == "-" => Volume::Windows(serial.parse().ok()?),
            _ => return None,
        };
        Some(Self {
            volume,
            file: number(file)?,
            facts: Some(Facts {
                kind: match kind {
                    "f" => EntryKind::File,
                    "d" => EntryKind::Directory,
                    "l" => EntryKind::Symlink,
                    "o" => EntryKind::Other,
                    _ => return None,
                },
                len: len.parse().ok()?,
                modified: time(modified)?,
                // A change time's nanoseconds are a count, never signed.
                changed: pair(changed)?.filter(|(_, nanos)| *nanos >= 0),
                created: time(created)?,
                owner: number(owner)?,
                links: None,
            }),
        })
    }
}

/// Seconds and nanoseconds from the epoch, the seconds negative before it.
fn since_epoch(at: SystemTime) -> (i64, i64) {
    match at.duration_since(UNIX_EPOCH) {
        Ok(after) => (
            i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
            i64::from(after.subsec_nanos()),
        ),
        Err(before) => {
            let before = before.duration();
            (
                -i64::try_from(before.as_secs()).unwrap_or(i64::MAX),
                -i64::from(before.subsec_nanos()),
            )
        }
    }
}

fn from_epoch((seconds, nanos): (i64, i64)) -> Option<SystemTime> {
    let span = Duration::new(
        seconds.unsigned_abs(),
        u32::try_from(nanos.unsigned_abs()).ok()?,
    );
    if seconds < 0 || nanos < 0 {
        UNIX_EPOCH.checked_sub(span)
    } else {
        UNIX_EPOCH.checked_add(span)
    }
}
