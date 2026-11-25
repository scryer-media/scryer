//! Stored versions compare as the text comparisons they replace did, over
//! every spelling rows hold, except that a zero file number on both sides is
//! unknown instead of the same file.

use crate::{Sameness, StoredVersion};

/// The comparisons as they were written before, kept here as the reference.
mod before {
    pub fn file_key(version: &str) -> Option<String> {
        if version.starts_with("symlink:") {
            return None;
        }
        let mut parts = version.split(':');
        Some(format!("{}:{}", parts.next()?, parts.next()?))
    }
    pub fn same_file(left: &str, right: &str) -> bool {
        left.split(':').take(2).eq(right.split(':').take(2))
    }
    pub fn same_content(left: &str, right: &str) -> bool {
        left.split(':').take(5).eq(right.split(':').take(5))
    }
}

const SEEDED: &[&str] = &[
    // Unix versions: one file, its ctime changed, its content changed, its
    // length changed, another file, another device.
    "64769:1234:5000:1700000000:120:1700000000:120",
    "64769:1234:5000:1700000000:120:1700000999:5",
    "64769:1234:5000:1700000500:0:1700000500:0",
    "64769:1234:5001:1700000000:120:1700000000:120",
    "64769:9876:5000:1700000000:120:1700000000:120",
    "64770:1234:5000:1700000000:120:1700000000:120",
    // A filesystem that numbers every file zero.
    "64771:0:5000:1700000000:120:1700000000:120",
    "64771:0:5000:1700000000:120:1700000444:9",
    "64771:0:7000:1700000300:0:1700000300:0",
    "64772:0:5000:1700000000:120:1700000000:120",
    // Symlinks, one with a target that reads like a zero number.
    "symlink:/srv/fixture-media/Fixture Show/a.mkv:Ok(SystemTime { tv_sec: 1, tv_nsec: 2 })",
    "symlink:/srv/fixture-media/Fixture Show/b.mkv:Ok(SystemTime { tv_sec: 1, tv_nsec: 2 })",
    "symlink:0:Ok(SystemTime { tv_sec: 1, tv_nsec: 2 })",
    // Windows-style: length, scheme and value, no file number.
    "5000:windows_last_write_100ns_v1:133000000000000000",
    "5000:windows_last_write_100ns_v1:133000000000000001",
    // Short and garbled text.
    "",
    "64769",
    "64769:",
    "64769:1234",
    "0:0",
    ":::::",
    "not a version",
];

fn zero_numbered(left: &str, right: &str) -> bool {
    let number = |text: &str| match text.starts_with("symlink:") {
        true => None,
        false => text.split(':').nth(1).map(str::to_owned),
    };
    number(left).as_deref() == Some("0") && number(right).as_deref() == Some("0")
}

#[test]
fn stored_versions_compare_as_before_except_a_zero_file_number() {
    let mut zero_cases = Vec::new();
    for left in SEEDED {
        let version = StoredVersion::new(left);
        assert_eq!(version.file_key(), before::file_key(left), "{left:?}");
        for right in SEEDED {
            let other = StoredVersion::new(right);
            for (now, then) in [
                (version.same_file(other), before::same_file(left, right)),
                (
                    version.same_content(other),
                    before::same_content(left, right),
                ),
            ] {
                if then && zero_numbered(left, right) {
                    assert_eq!(now, Sameness::CannotTell, "{left:?} {right:?}");
                    zero_cases.push((*left, *right));
                } else {
                    let expected = match then {
                        true => Sameness::Same,
                        false => Sameness::Different,
                    };
                    assert_eq!(now, expected, "{left:?} {right:?}");
                }
            }
        }
    }
    // The only answers that changed: two versions on one device whose file
    // number is zero, and "0:0" against itself, each once for the file and,
    // where length and modification time match too, once for the content.
    zero_cases.sort_unstable();
    zero_cases.dedup();
    assert_eq!(
        zero_cases,
        [
            ("0:0", "0:0"),
            (
                "64771:0:5000:1700000000:120:1700000000:120",
                "64771:0:5000:1700000000:120:1700000000:120"
            ),
            (
                "64771:0:5000:1700000000:120:1700000000:120",
                "64771:0:5000:1700000000:120:1700000444:9"
            ),
            (
                "64771:0:5000:1700000000:120:1700000000:120",
                "64771:0:7000:1700000300:0:1700000300:0"
            ),
            (
                "64771:0:5000:1700000000:120:1700000444:9",
                "64771:0:5000:1700000000:120:1700000000:120"
            ),
            (
                "64771:0:5000:1700000000:120:1700000444:9",
                "64771:0:5000:1700000000:120:1700000444:9"
            ),
            (
                "64771:0:5000:1700000000:120:1700000444:9",
                "64771:0:7000:1700000300:0:1700000300:0"
            ),
            (
                "64771:0:7000:1700000300:0:1700000300:0",
                "64771:0:5000:1700000000:120:1700000000:120"
            ),
            (
                "64771:0:7000:1700000300:0:1700000300:0",
                "64771:0:5000:1700000000:120:1700000444:9"
            ),
            (
                "64771:0:7000:1700000300:0:1700000300:0",
                "64771:0:7000:1700000300:0:1700000300:0"
            ),
            (
                "64772:0:5000:1700000000:120:1700000000:120",
                "64772:0:5000:1700000000:120:1700000000:120"
            ),
        ]
    );
}
