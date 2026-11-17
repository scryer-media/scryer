//! What a Windows stat makes of the numbers Win32 hands back, kept apart from
//! the calls so that it builds, and is tested, on every platform.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::reading::EntryKind;

/// FILETIME counts 100 ns intervals from 1601; this many lie before 1970.
pub(crate) const INTERVALS_TO_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
/// `FILE_ATTRIBUTE_DIRECTORY`.
pub(crate) const DIRECTORY: u32 = 0x10;
/// `FILE_ATTRIBUTE_REPARSE_POINT`.
pub(crate) const REPARSE_POINT: u32 = 0x400;
/// The reparse tag bit that marks a name surrogate: a link or a junction.
const NAME_SURROGATE: u32 = 0x2000_0000;
/// `FILE_SUPPORTS_OPEN_BY_FILE_ID`: the filesystem keeps a file's number
/// for the file's life, so it can open a file by it.
const SUPPORTS_OPEN_BY_FILE_ID: u32 = 0x0100_0000;
/// `FILE_SUPPORTS_INTEGRITY_STREAMS`: the mark of ReFS, and of a cluster
/// volume over it, whose 128-bit file numbers the 64-bit index truncates.
const SUPPORTS_INTEGRITY_STREAMS: u32 = 0x0400_0000;

/// The instant a FILETIME of `intervals` names, as std converts one. A time
/// past what the conversion can hold is no time, never a panic.
pub(crate) fn filetime(intervals: u64) -> Option<SystemTime> {
    if intervals >= INTERVALS_TO_UNIX_EPOCH {
        let since = intervals - INTERVALS_TO_UNIX_EPOCH;
        UNIX_EPOCH.checked_add(span(since))
    } else {
        UNIX_EPOCH.checked_sub(span(INTERVALS_TO_UNIX_EPOCH - intervals))
    }
}

fn span(intervals: u64) -> Duration {
    Duration::new(
        intervals / 10_000_000,
        ((intervals % 10_000_000) * 100) as u32,
    )
}

/// Whether a file's attributes and reparse tag name a link or a junction.
pub(crate) fn is_name_surrogate(attributes: u32, tag: u32) -> bool {
    attributes & REPARSE_POINT != 0 && tag & NAME_SURROGATE != 0
}

/// What kind of entry the attributes and reparse tag describe. A reparse
/// point that is not a link or a junction (a deduplicated, compressed or
/// cloud-placeholder file) is the file or folder it reads as.
pub(crate) fn kind(attributes: u32, tag: u32) -> EntryKind {
    if is_name_surrogate(attributes, tag) {
        EntryKind::Symlink
    } else if attributes & DIRECTORY != 0 {
        EntryKind::Directory
    } else {
        EntryKind::File
    }
}

/// Whether a volume whose flags read `flags` keeps file numbers that tell
/// its files apart: it says it can open a file by its number, and it is not
/// ReFS, which it says by supporting integrity streams. FAT and exFAT derive
/// a number from where the entry sits, so a rename changes it, and do not
/// say they keep them.
pub(crate) fn keeps_numbers(flags: u32) -> bool {
    flags & SUPPORTS_OPEN_BY_FILE_ID != 0 && flags & SUPPORTS_INTEGRITY_STREAMS == 0
}

/// What a volume answered about its file numbers, by serial: a few slots a
/// stat reads without a lock. A serial lands in one slot and pushes out the
/// one before it. An answer is remembered. A volume that did not answer
/// leaves the reads without numbers and is not asked again until
/// [`ASK_AGAIN_AFTER`] reads of it have gone by, so one that never answers
/// is asked once in that many reads rather than on every one.
pub(crate) struct VolumeAnswers {
    slots: [AtomicU64; 16],
}

const KNOWN: u64 = 0b010;
const KEEPS: u64 = 0b001;
const UNANSWERED: u64 = 0b100;
/// Where a slot counts the reads a volume that did not answer has gone
/// unasked.
const SKIPPED_SHIFT: u32 = 8;
const SKIPPED_MASK: u64 = 0xFFFF;
/// How many reads of a volume that did not answer go by before it is asked
/// again.
pub(crate) const ASK_AGAIN_AFTER: u64 = 256;

impl VolumeAnswers {
    pub(crate) const fn new() -> Self {
        Self {
            slots: [const { AtomicU64::new(0) }; 16],
        }
    }

    /// Whether the volume `serial` keeps its file numbers: the remembered
    /// answer, or `ask`'s, which gives the volume's flags or `None` when the
    /// volume did not say. One that did not say is `false`, and is not asked
    /// again until [`ASK_AGAIN_AFTER`] reads of it have gone by.
    pub(crate) fn keeps_numbers(&self, serial: u32, ask: impl FnOnce() -> Option<u32>) -> bool {
        let slot = &self.slots[serial as usize % self.slots.len()];
        let held = slot.load(Ordering::Relaxed);
        if (held >> 32) as u32 == serial {
            if held & KNOWN != 0 {
                return held & KEEPS != 0;
            }
            let skipped = (held >> SKIPPED_SHIFT) & SKIPPED_MASK;
            if held & UNANSWERED != 0 && skipped < ASK_AGAIN_AFTER {
                // Only over what was read: a count lost to another read is
                // harmless, an answer written meanwhile is kept.
                let counted =
                    (u64::from(serial) << 32) | UNANSWERED | ((skipped + 1) << SKIPPED_SHIFT);
                return match slot.compare_exchange(
                    held,
                    counted,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => false,
                    Err(now) => known_answer(now, serial).unwrap_or(false),
                };
            }
        }
        let Some(flags) = ask() else {
            // No answer never replaces one another read wrote meanwhile.
            let unanswered = (u64::from(serial) << 32) | UNANSWERED;
            return match slot.compare_exchange(
                held,
                unanswered,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => false,
                Err(now) => known_answer(now, serial).unwrap_or(false),
            };
        };
        let keeps = keeps_numbers(flags);
        slot.store(
            (u64::from(serial) << 32) | KNOWN | u64::from(keeps),
            Ordering::Relaxed,
        );
        keeps
    }
}

/// The answer a slot holds for `serial`, if it holds one.
fn known_answer(held: u64, serial: u32) -> Option<bool> {
    ((held >> 32) as u32 == serial && held & KNOWN != 0).then_some(held & KEEPS != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_filetime_converts_and_one_out_of_range_reads_as_no_time() {
        assert_eq!(filetime(INTERVALS_TO_UNIX_EPOCH), Some(UNIX_EPOCH));
        assert_eq!(
            filetime(INTERVALS_TO_UNIX_EPOCH - 15),
            Some(UNIX_EPOCH - Duration::from_nanos(1_500))
        );
        assert_eq!(
            filetime(INTERVALS_TO_UNIX_EPOCH + 10_000_000),
            Some(UNIX_EPOCH + Duration::from_secs(1))
        );
        // The high bit set: a FILETIME no clock writes, which reads as no
        // time or a far one, never a panic.
        for intervals in [1 << 63, u64::MAX, u64::MAX - 1] {
            let _ = filetime(intervals);
        }
        assert_eq!(
            filetime(0),
            UNIX_EPOCH.checked_sub(span(INTERVALS_TO_UNIX_EPOCH))
        );
    }

    #[test]
    fn a_volume_keeps_numbers_only_by_id_and_not_as_refs() {
        assert!(keeps_numbers(SUPPORTS_OPEN_BY_FILE_ID));
        assert!(!keeps_numbers(0));
        assert!(!keeps_numbers(
            SUPPORTS_OPEN_BY_FILE_ID | SUPPORTS_INTEGRITY_STREAMS
        ));
    }

    #[test]
    fn a_volume_is_asked_once_it_answers_and_now_and_then_until_it_does() {
        let answers = VolumeAnswers::new();
        let asked = Cell::new(0);
        let ask = |flags: Option<u32>| {
            let asked = &asked;
            move || {
                asked.set(asked.get() + 1);
                flags
            }
        };
        assert!(answers.keeps_numbers(7, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        assert!(answers.keeps_numbers(7, ask(None)));
        assert_eq!(asked.get(), 1);
        // A volume that did not answer gives its reads no numbers, and is
        // not asked again until ASK_AGAIN_AFTER reads have gone by; once it
        // answers, the answer is kept.
        assert!(!answers.keeps_numbers(9, ask(None)));
        assert_eq!(asked.get(), 2);
        for _ in 0..ASK_AGAIN_AFTER {
            assert!(!answers.keeps_numbers(9, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        }
        assert_eq!(asked.get(), 2);
        assert!(!answers.keeps_numbers(9, ask(None)));
        assert_eq!(asked.get(), 3);
        for _ in 0..ASK_AGAIN_AFTER {
            assert!(!answers.keeps_numbers(9, ask(None)));
        }
        assert!(answers.keeps_numbers(9, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        assert!(answers.keeps_numbers(9, ask(None)));
        assert_eq!(asked.get(), 4);
        // A volume that answered it keeps none is remembered so.
        assert!(!answers.keeps_numbers(11, ask(Some(0))));
        assert!(!answers.keeps_numbers(11, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        assert_eq!(asked.get(), 5);
        // Another serial in the same slot pushes the first one out.
        assert!(!answers.keeps_numbers(7 + 16, ask(Some(0))));
        assert!(answers.keeps_numbers(7, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        assert_eq!(asked.get(), 7);
        // A volume that did not answer is pushed out the same way, and
        // asked when next seen.
        assert!(!answers.keeps_numbers(13, ask(None)));
        assert!(!answers.keeps_numbers(13 + 16, ask(Some(0))));
        assert!(answers.keeps_numbers(13, ask(Some(SUPPORTS_OPEN_BY_FILE_ID))));
        assert_eq!(asked.get(), 10);
    }

    /// An answer another read writes while this one asks is never replaced
    /// by this one's silence, and is what this read gives.
    #[test]
    fn a_volume_that_did_not_answer_never_replaces_an_answer_written_meanwhile() {
        let answers = VolumeAnswers::new();
        let slot = 21_u32;
        let others_answer = || {
            assert!(answers.keeps_numbers(slot, || Some(SUPPORTS_OPEN_BY_FILE_ID)));
        };
        assert!(answers.keeps_numbers(slot, || {
            others_answer();
            None
        }));
        assert!(answers.keeps_numbers(slot, || None));
    }

    #[test]
    fn a_reparse_tag_names_a_link_only_when_it_is_a_name_surrogate() {
        assert_eq!(kind(0, 0), EntryKind::File);
        assert_eq!(kind(DIRECTORY, 0), EntryKind::Directory);
        // IO_REPARSE_TAG_SYMLINK and IO_REPARSE_TAG_MOUNT_POINT.
        for tag in [0xA000_000C, 0xA000_0003] {
            assert!(is_name_surrogate(REPARSE_POINT, tag), "{tag:#x}");
            assert_eq!(kind(REPARSE_POINT, tag), EntryKind::Symlink, "{tag:#x}");
            assert_eq!(kind(REPARSE_POINT | DIRECTORY, tag), EntryKind::Symlink);
            assert!(!is_name_surrogate(0, tag), "{tag:#x}");
        }
        // IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_DEDUP, IO_REPARSE_TAG_WOF.
        for tag in [0x9000_001A, 0x8000_0013, 0x8000_0017] {
            assert!(!is_name_surrogate(REPARSE_POINT, tag), "{tag:#x}");
            assert_eq!(kind(REPARSE_POINT, tag), EntryKind::File, "{tag:#x}");
        }
    }
}
