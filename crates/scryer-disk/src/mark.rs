//! What one read of a file said about which file it is and which version of
//! it, kept opaque. A caller holds a [`Mark`] and asks what it means: is this
//! path still that file, are these two one file, are they on one volume. No
//! number in it is handed out.
//!
//! A mark carries whatever the platform offered: the device and inode on
//! unix, a zero inode included; the volume serial and file index on Windows,
//! the index only on a volume that keeps it for a file's life; and length
//! and times everywhere. Each answer says when the platform's facts were not
//! enough to tell.

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

/// Whether two paths name one directory entry, as far as one cheap look
/// tells. Only [`super::Disk::remove_source_beside_blocking`] settles it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryAlias {
    /// The paths are one as written, or both read as one file that the read
    /// does not show to have another name.
    Same,
    /// Nothing here shows one entry. They may still be one, through a linked
    /// or mounted folder, another spelling, or a link at the second path.
    Distinct,
    /// The read of the source failed. Nothing is guessed.
    CannotTell,
}

impl super::Disk {
    /// Whether `dest` plainly names the directory entry `source` names: a
    /// cheap early refusal, never a proof that the two are separate. `dest_read`
    /// is the caller's read of `dest` itself, links not followed.
    ///
    /// Paths that are one as written are one entry and read nothing. Another
    /// spelling of the same name is not judged here: on a volume that tells
    /// case apart it is another file. A destination the read shows to have
    /// another name is not judged further. Otherwise the source is read once,
    /// links not followed, and one file is one entry.
    pub fn entry_alias_blocking(&self, source: &Path, dest: &Path, dest_read: &Mark) -> EntryAlias {
        if source == dest {
            return EntryAlias::Same;
        }
        if dest_read.has_other_names() {
            return EntryAlias::Distinct;
        }
        match self.fingerprint_blocking(source) {
            Ok(read) => Self::entry_alias_of(source, &read, dest, dest_read),
            Err(_) => EntryAlias::CannotTell,
        }
    }

    /// [`Self::entry_alias_blocking`] for a caller that already holds
    /// `source_read`, its read of `source` itself, links not followed. Reads
    /// nothing, so it never answers [`EntryAlias::CannotTell`].
    pub fn entry_alias_of(
        source: &Path,
        source_read: &Mark,
        dest: &Path,
        dest_read: &Mark,
    ) -> EntryAlias {
        if source == dest {
            return EntryAlias::Same;
        }
        if dest_read.has_other_names() {
            return EntryAlias::Distinct;
        }
        match source_read.same_file(dest_read) {
            Sameness::Same => EntryAlias::Same,
            Sameness::Different | Sameness::CannotTell => EntryAlias::Distinct,
        }
    }
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

/// What a new hard link's name must show to be the file it was made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkLane {
    /// One volume and one file number: it is the link.
    Identity,
    /// No usable file number on either side, and the source's length and
    /// modification time: its content must match the source's.
    Content,
    /// It is not the link.
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Volume {
    Unix(u64),
    Windows(u64),
    Unknown,
}

/// `text` as [`Mark::to_stored`] writes it now, where an earlier mark stored
/// a Windows volume serial of zero as a volume.
fn zero_serial_as_unknown(text: &str) -> String {
    if text == format!("{WINDOWS_LANE}0") {
        return UNKNOWN_LANE.to_owned();
    }
    let mut fields: Vec<&str> = text.split(':').collect();
    if fields.first() == Some(&STORED_TAG) && fields.get(6) == Some(&"w0") {
        fields[6] = "-";
    }
    fields.join(":")
}

impl Volume {
    /// A Windows volume by its serial. A serial of zero names none: it is
    /// what a volume that has none reads as, and a stat that reads it keeps
    /// no volume either.
    fn windows(serial: u64) -> Self {
        match serial {
            0 => Self::Unknown,
            serial => Self::Windows(serial),
        }
    }
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
    /// Whether the disk service read this mark itself, by its own stat of a
    /// name. A mark rebuilt from stored text or from a volume number is not,
    /// and neither is one built from std metadata, whether a caller held it
    /// or the service took it from a handle it opened: on Windows std's
    /// metadata names no volume or file. None of those ever proves a link.
    fresh: bool,
}

const STORED_TAG: &str = "mark1";
const UNIX_LANE: &str = "unix-dev:";
const WINDOWS_LANE: &str = "windows-volume:";
pub(crate) const UNKNOWN_LANE: &str = "unknown-volume";

impl Mark {
    pub(crate) fn of_stat(stat: &RawStat) -> Self {
        let (volume, file) = match (stat.extra.device, stat.extra.serial) {
            (Some(device), _) => (Volume::Unix(device), stat.extra.inode),
            (None, Some(serial)) => (
                Volume::Windows(serial),
                stat.extra.inode.filter(|_| !stat.extra.unnumbered),
            ),
            (None, None) => (Volume::Unknown, None),
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
            fresh: true,
        }
    }

    /// The mark of the volume the platform numbers `number`, a unix device
    /// or a Windows volume serial. A serial of zero names no volume, as a
    /// stat that reads one does. No probe. Only for the import worker, whose
    /// payload carries that number until it carries a mark instead.
    #[doc(hidden)]
    pub fn on_volume(number: u64) -> Self {
        Self {
            volume: if cfg!(unix) {
                Volume::Unix(number)
            } else if cfg!(windows) {
                Volume::windows(number)
            } else {
                Volume::Unknown
            },
            file: None,
            facts: None,
            fresh: false,
        }
    }

    /// The mark of metadata a caller already holds from an open handle. No
    /// probe. The disk service did not read it, so it never proves a link.
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            fresh: false,
            ..Self::of_stat(&RawStat::of(metadata))
        }
    }

    /// Whether the read named the volume the path is on.
    pub fn names_volume(&self) -> bool {
        self.volume != Volume::Unknown
    }

    /// Whether both marks are one file. A volume either read did not name is
    /// [`Sameness::CannotTell`]. Two named volumes that differ are
    /// [`Sameness::Different`] before any file number is looked at, so two
    /// marks without numbers on two volumes are two files. On one volume, a
    /// missing file number or a zero number on both sides is
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

    /// How this mark, read of a name just hard-linked, can be the file
    /// `seen` was read as before the link. Where both reads give a usable
    /// file number on named volumes, one volume and one number are the link.
    /// Only where neither read gives one, a zero number on both sides or
    /// none at all, can the content say, and only for a regular file with
    /// `seen`'s length and modification time. Where one read has a number and
    /// the other has none, or a volume one read does not name, nothing
    /// proves: a volume that numbers its files answers by number. `seen`
    /// and the link must both be the disk service's own reads: a stored
    /// mark, a volume-only mark or one built from std metadata never proves.
    /// No probe.
    pub(crate) fn link_of(&self, seen: &Self) -> LinkLane {
        let (Some(linked), Some(before)) = (self.facts, seen.facts) else {
            return LinkLane::No;
        };
        if !self.fresh || !seen.fresh {
            return LinkLane::No;
        }
        if linked.kind != EntryKind::File {
            return LinkLane::No;
        }
        let usable = |file: Option<u64>| file.filter(|&number| number != 0);
        let numbers = (usable(self.file), usable(seen.file));
        let one_volume = match (self.volume, seen.volume) {
            (Volume::Unix(here), Volume::Unix(there)) => here == there,
            (Volume::Windows(here), Volume::Windows(there)) => here == there,
            // A read that names no volume has no number to compare.
            (Volume::Unknown, _) | (_, Volume::Unknown) => true,
            _ => false,
        };
        let named = self.volume != Volume::Unknown && seen.volume != Volume::Unknown;
        let by_content = match linked.len == before.len && linked.modified == before.modified {
            true => LinkLane::Content,
            false => LinkLane::No,
        };
        match numbers {
            _ if !one_volume => LinkLane::No,
            (Some(here), Some(there)) if named => match here == there {
                true => LinkLane::Identity,
                false => LinkLane::No,
            },
            (None, None) => by_content,
            _ => LinkLane::No,
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
    #[expect(
        clippy::len_without_is_empty,
        reason = "Length is optional file metadata"
    )]
    pub fn len(&self) -> Option<u64> {
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
        mark.fresh = false;
        mark
    }

    /// Whether `now`, a later read of the same path, is still the file this
    /// mark was read of: the same file where the platform numbers files, and
    /// otherwise an entry of the same kind whose length, modification time
    /// and owner held. No probe.
    pub fn still_marks(&self, now: &Self) -> bool {
        match now.same_file(self) {
            Sameness::Same => true,
            Sameness::Different => false,
            Sameness::CannotTell => {
                now.kind().is_some()
                    && now.kind() == self.kind()
                    && self.compare(now).is_unwritten_by_its_owner()
            }
        }
    }

    /// Whether both reads give one nonzero file number and one length, and
    /// so name one file: [`Sameness::Same`] only on one named volume. The same
    /// number and length on another volume, or one not named, cannot tell: a
    /// second view of one folder can name another volume, and two volumes
    /// can each number a file alike. No probe.
    pub(crate) fn numbered_as(&self, other: &Self) -> Sameness {
        let numbered = matches!(
            (self.file, other.file),
            (Some(here), Some(there)) if here != 0 && here == there
        ) && matches!(
            (self.facts, other.facts),
            (Some(here), Some(there)) if here.len == there.len
        );
        match (numbered, self.same_volume(other)) {
            (false, _) => Sameness::Different,
            (true, Sameness::Same) => Sameness::Same,
            (true, _) => Sameness::CannotTell,
        }
    }

    /// Whether both reads give a nonzero file number on one named volume,
    /// so their numbers can tell one file from another. No probe.
    pub(crate) fn numbered_on_one_volume(&self, other: &Self) -> bool {
        self.same_volume(other) == Sameness::Same
            && matches!((self.file, other.file), (Some(here), Some(there)) if here != 0 && there != 0)
    }

    /// The modification time read; `None` where the platform gave none or
    /// for a mark that names a volume only.
    pub fn modified(&self) -> Option<SystemTime> {
        self.facts.and_then(|facts| facts.modified)
    }

    /// Whether the read said the file has a name besides this one.
    pub fn has_other_names(&self) -> bool {
        self.facts
            .is_some_and(|facts| facts.links.is_some_and(|links| links > 1))
    }

    /// Unknown link counts do not prove this is the file's only name.
    pub fn has_one_name(&self) -> bool {
        self.facts.is_some_and(|facts| facts.links == Some(1))
    }

    /// The original version spelling used by persisted location and review rows.
    #[cfg(unix)]
    pub fn legacy_unix_version(&self) -> Option<String> {
        let Volume::Unix(device) = self.volume else {
            return None;
        };
        let file = self.file?;
        let facts = self.facts?;
        let (modified, modified_ns) = since_epoch(facts.modified?);
        // Unix stat rounds seconds down and reports a nonnegative nanos field.
        let (modified, modified_ns) = if modified_ns < 0 {
            (modified.checked_sub(1)?, 1_000_000_000 + modified_ns)
        } else {
            (modified, modified_ns)
        };
        let (changed, changed_ns) = facts.changed?;
        let len = facts.len;
        Some(format!(
            "{device}:{file}:{len}:{modified}:{modified_ns}:{changed}:{changed_ns}"
        ))
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
            fresh: false,
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
            (Volume::Windows(serial), Some(file), _) => format!("volume={serial} index={file}"),
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
    /// another spelling of a number it writes included. A Windows volume
    /// serial of zero, which earlier marks stored as a volume, reads as the
    /// unknown volume it is.
    pub fn from_stored(text: &str) -> Option<Self> {
        Self::parsed(text).filter(|mark| {
            let stored = mark.to_stored();
            stored == text || stored == zero_serial_as_unknown(text)
        })
    }

    fn parsed(text: &str) -> Option<Self> {
        let volume_only = |volume| Self {
            volume,
            file: None,
            facts: None,
            fresh: false,
        };
        if text == UNKNOWN_LANE {
            return Some(volume_only(Volume::Unknown));
        }
        if let Some(device) = text.strip_prefix(UNIX_LANE) {
            return Some(volume_only(Volume::Unix(device.parse().ok()?)));
        }
        if let Some(serial) = text.strip_prefix(WINDOWS_LANE) {
            return Some(volume_only(Volume::windows(serial.parse().ok()?)));
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
            ("w", serial) => Volume::windows(serial.parse().ok()?),
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
            fresh: false,
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
