#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct MajorMinor {
    major: u64,
    minor: u64,
}

impl MajorMinor {
    pub(crate) const fn new(major: u64, minor: u64) -> Self {
        Self { major, minor }
    }
}

pub(crate) fn parse_major_minor(version: &str) -> Option<MajorMinor> {
    let normalized = version.trim().trim_start_matches('v');
    let mut parts = normalized.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts
        .next()?
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse::<u64>()
        .ok()?;
    Some(MajorMinor::new(major, minor))
}

pub(crate) fn is_upgrade_from_line_to_at_least(
    previous_version: Option<&str>,
    current_version: &str,
    previous_line: MajorMinor,
    target: MajorMinor,
) -> bool {
    previous_version.and_then(parse_major_minor) == Some(previous_line)
        && parse_major_minor(current_version).is_some_and(|current| current >= target)
}

pub(crate) fn is_upgrade_from_before_to_at_least(
    previous_version: Option<&str>,
    current_version: &str,
    target: MajorMinor,
) -> bool {
    previous_version
        .and_then(parse_major_minor)
        .is_some_and(|previous| previous < target)
        && parse_major_minor(current_version).is_some_and(|current| current >= target)
}

/// Whether `version` is an older release than `major.minor.patch`. A version
/// that cannot be read is not older than anything.
pub(crate) fn is_release_before(version: &str, major: u64, minor: u64, patch: u64) -> bool {
    let normalized = version.trim().trim_start_matches('v');
    let mut parts = normalized.splitn(3, '.');
    let mut number = || {
        parts
            .next()?
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .parse::<u64>()
            .ok()
    };
    match (number(), number(), number()) {
        (Some(found_major), Some(found_minor), Some(found_patch)) => {
            (found_major, found_minor, found_patch) < (major, minor, patch)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_is_older_only_when_all_three_numbers_read_and_compare_lower() {
        for version in [
            "0.21.13",
            "v0.21.13",
            "0.20.99",
            "0.21.9-rc.1",
            "0.21.13+build",
        ] {
            assert!(is_release_before(version, 0, 21, 14), "{version}");
        }
        for version in [
            "0.21.14", "0.21.15", "0.22.0", "1.0.0", "0.21", "", "nightly",
        ] {
            assert!(!is_release_before(version, 0, 21, 14), "{version}");
        }
    }

    #[test]
    fn parses_major_minor_from_common_version_forms() {
        assert_eq!(parse_major_minor("0.16.0"), Some(MajorMinor::new(0, 16)));
        assert_eq!(parse_major_minor("v0.16.1"), Some(MajorMinor::new(0, 16)));
        assert_eq!(
            parse_major_minor("0.16-beta.1"),
            Some(MajorMinor::new(0, 16))
        );
        assert_eq!(parse_major_minor("0"), None);
        assert_eq!(parse_major_minor("garbage"), None);
    }

    #[test]
    fn detects_exact_source_line_upgrade_to_target_or_later() {
        assert!(is_upgrade_from_line_to_at_least(
            Some("0.15.9"),
            "0.16.0",
            MajorMinor::new(0, 15),
            MajorMinor::new(0, 16),
        ));
        assert!(!is_upgrade_from_line_to_at_least(
            Some("0.14.9"),
            "0.16.0",
            MajorMinor::new(0, 15),
            MajorMinor::new(0, 16),
        ));
        assert!(!is_upgrade_from_line_to_at_least(
            Some("0.15.9"),
            "0.15.10",
            MajorMinor::new(0, 15),
            MajorMinor::new(0, 16),
        ));
    }

    #[test]
    fn detects_upgrade_from_before_target_to_target_or_later() {
        assert!(is_upgrade_from_before_to_at_least(
            Some("0.14.9"),
            "0.16.0",
            MajorMinor::new(0, 16),
        ));
        assert!(is_upgrade_from_before_to_at_least(
            Some("0.15.9"),
            "1.0.0",
            MajorMinor::new(0, 16),
        ));
        assert!(!is_upgrade_from_before_to_at_least(
            Some("0.16.0"),
            "0.16.1",
            MajorMinor::new(0, 16),
        ));
        assert!(!is_upgrade_from_before_to_at_least(
            None,
            "0.16.0",
            MajorMinor::new(0, 16),
        ));
    }
}
