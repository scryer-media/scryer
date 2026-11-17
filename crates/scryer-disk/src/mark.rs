//! What one read of a file said about which file it is and which version of
//! it, kept opaque. A caller holds a [`Mark`] and asks what it means: is this
//! path still that file, are these two one file, are they on one volume. No
//! number in it is handed out.
//!
//! A mark carries whatever the platform offered: the device and inode on
//! unix, a zero inode included; the volume serial alone on Windows, which
//! tells no two files apart; and length and times everywhere. Each answer
//! says when the platform's facts were not enough to tell.

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

    /// Whether both marks are one file. No probe.
    pub(crate) fn same_file(&self, other: &Self) -> Sameness {
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
                let modified = facts.modified.map(since_epoch);
                format!("len={} modified={modified:?}", facts.len)
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
