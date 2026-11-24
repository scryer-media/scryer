//! File versions as location rows store them, read without changing the
//! text. Rows written before keep their exact bytes and compare as they did,
//! except that a zero file number on both sides is unknown, never the same
//! file.
//!
//! On unix a version is `dev:ino:len:mtime:mtime_nsec:ctime:ctime_nsec`; a
//! symlink's is `symlink:<target>:<modified>`, and other platforms write
//! `len:<scheme>:<value>`. Only the leading `:`-separated fields are
//! compared: on unix the device and file number say which file, and the
//! length and modification time with them say its content is as recorded. A
//! change time is not compared, since removing another link to the file
//! changes it.
//!
//! The other spellings are split the same way, so they compare less than
//! they hold. A symlink's leading fields are a prefix of its target and the
//! link's own time (a target or time holding `:` ends them early), never the
//! file it names, so a symlink's version says nothing about that file's
//! content. The other platforms' spelling holds no file number at all.

use crate::Sameness;

/// The fields that name the file.
const FILE_FIELDS: usize = 2;
/// The fields that name the file and its content.
const CONTENT_FIELDS: usize = 5;

/// One stored version, borrowed. No probe.
#[derive(Clone, Copy, Debug)]
pub struct StoredVersion<'a>(&'a str);

impl<'a> StoredVersion<'a> {
    pub fn new(text: &'a str) -> Self {
        Self(text)
    }

    /// Whether both versions name one file: the same device and file
    /// number. A zero number on both sides is [`Sameness::CannotTell`]:
    /// filesystems that do not number files report zero for every file.
    pub fn same_file(self, other: Self) -> Sameness {
        self.compare(other, FILE_FIELDS)
    }

    /// Whether both versions name one file with the content recorded: the
    /// same device, file number, length and modification time. A zero file
    /// number on both sides is [`Sameness::CannotTell`].
    pub fn same_content(self, other: Self) -> Sameness {
        self.compare(other, CONTENT_FIELDS)
    }

    /// The device and file number as one key, for finding versions that
    /// name one file; `None` for a symlink's version and for text with fewer
    /// fields. A zero number is kept: a caller that groups by the key to find
    /// two names of one file treats every such name as possibly one file.
    pub fn file_key(self) -> Option<String> {
        if self.0.starts_with("symlink:") {
            return None;
        }
        let mut fields = self.0.split(':');
        Some(format!("{}:{}", fields.next()?, fields.next()?))
    }

    fn compare(self, other: Self, fields: usize) -> Sameness {
        if !self
            .0
            .split(':')
            .take(fields)
            .eq(other.0.split(':').take(fields))
        {
            return Sameness::Different;
        }
        match (self.file_number(), other.file_number()) {
            (Some("0"), Some("0")) => Sameness::CannotTell,
            _ => Sameness::Same,
        }
    }

    /// The file number field; `None` for a symlink's version, whose second
    /// field is its target.
    fn file_number(self) -> Option<&'a str> {
        if self.0.starts_with("symlink:") {
            return None;
        }
        self.0.split(':').nth(1)
    }
}

/// What a restore records of the name it restores to, as text compared
/// whole: `file:<len>:<secs>:<nanos>` for a file (`unknown` for a time not
/// read), and `symlink`, `non-file` or `missing` otherwise. No probe.
pub fn restore_target_state(reading: crate::PathReading) -> String {
    match reading.kind() {
        Some(crate::EntryKind::Symlink) => "symlink".to_string(),
        Some(crate::EntryKind::File) => {
            let modified = reading
                .modified()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            format!("file:{}:{modified}", reading.size().unwrap_or(0))
        }
        Some(_) => "non-file".to_string(),
        None => "missing".to_string(),
    }
}
