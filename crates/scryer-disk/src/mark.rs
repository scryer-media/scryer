//! What one read of a file said about which file it is and which version of
//! it, kept opaque. A caller holds a [`Mark`] and asks what it means: is this
//! path still that file, are these two one file, does it have other names.
//! No number in it is handed out.
//!
//! A mark carries whatever the platform offered: the device and inode on
//! unix, a zero inode included; the volume serial and file index on Windows
//! when the read asked for them; and length and times everywhere. Each
//! answer says when the platform's facts were not enough to tell.

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

/// Whether a file has names other than the one read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Links {
    Sole,
    Shared,
    /// The read did not carry a link count.
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
    /// Its length and modification time held; its change time, creation
    /// time or owner did not.
    Touched,
    /// Its length or modification time differs.
    Rewritten,
    /// A different file.
    Replaced,
    /// Nothing is at the path now. Only a verb that reads a path says this.
    Gone,
    /// The path could not be read. Only a verb that reads a path says this.
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
    links: Option<u64>,
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
const UNKNOWN_LANE: &str = "unknown-volume";

impl Mark {
    pub(crate) fn of_stat(stat: &RawStat, windows_id: Option<(u64, u64)>) -> Self {
        let (volume, file) = match (windows_id, stat.extra.device) {
            (Some((serial, index)), _) => (Volume::Windows(serial), Some(index)),
            (None, Some(device)) => (Volume::Unix(device), stat.extra.inode),
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
                links: stat.links,
                owner: stat.extra.owner,
            }),
        }
    }

    /// The mark of metadata a caller already holds from an open handle. No
    /// probe. It never carries a Windows file index.
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        Self::of_stat(&RawStat::of(metadata), None)
    }

    /// Whether both marks are one file. No probe.
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

    /// Whether both marks are on one volume, so they share its space and a
    /// name can be moved between them. No probe.
    pub fn same_volume(&self, other: &Self) -> Sameness {
        match (self.volume, other.volume) {
            (Volume::Unknown, _) | (_, Volume::Unknown) => Sameness::CannotTell,
            (left, right) if left == right => Sameness::Same,
            _ => Sameness::Different,
        }
    }

    /// What differs between this mark and a later one of the same path. No
    /// probe; never [`Drift::Gone`] or [`Drift::CannotTell`]. What kind of
    /// entry it is does not enter into it: one file cannot change kind, and
    /// where the platform cannot tell files apart, a caller that cares
    /// checks [`Self::kind`] itself.
    pub fn compare(&self, later: &Self) -> Drift {
        let same = self.same_file(later);
        if same == Sameness::Different {
            return Drift::Replaced;
        }
        let (Some(before), Some(after)) = (self.facts, later.facts) else {
            return Drift::SameFacts;
        };
        if before.len != after.len || before.modified != after.modified {
            Drift::Rewritten
        } else if before.changed != after.changed
            || before.created != after.created
            || before.owner != after.owner
        {
            Drift::Touched
        } else if same == Sameness::Same {
            Drift::Same
        } else {
            Drift::SameFacts
        }
    }

    /// Whether the file had another name when it was read. No probe.
    pub fn other_links(&self) -> Links {
        match self.facts.and_then(|facts| facts.links) {
            Some(count) if count >= 2 => Links::Shared,
            Some(_) => Links::Sole,
            None => Links::CannotTell,
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

    /// The copy lane for this mark's volume: the text
    /// [`super::copy::volume_key`] gives a path on the same volume.
    pub(crate) fn lane(&self) -> String {
        match self.volume {
            Volume::Unix(device) => format!("{UNIX_LANE}{device}"),
            Volume::Windows(serial) => format!("{WINDOWS_LANE}{serial}"),
            Volume::Unknown => UNKNOWN_LANE.to_owned(),
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
            number(facts.links),
            number(facts.owner.map(u64::from)),
        ]
        .join(":")
    }

    /// The mark [`Self::to_stored`] wrote. `None` for text it did not write.
    pub fn from_stored(text: &str) -> Option<Self> {
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
            links,
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
            Some(Some((seconds.parse().ok()?, nanos.parse().ok()?)))
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
            ("w", serial) => Volume::Windows(serial.parse().ok()?),
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
                changed: pair(changed)?,
                created: time(created)?,
                links: number(links)?,
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
