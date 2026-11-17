use crate::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSourceSignature {
    pub scheme: String,
    pub value: String,
}

#[cfg(windows)]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "windows_last_write_100ns_v1";
#[cfg(unix)]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "unix_mtime_nsec_v1";
#[cfg(all(not(unix), not(windows)))]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "system_time_nsec_v1";

pub fn file_source_signature_from_metadata(
    metadata: &std::fs::Metadata,
) -> AppResult<FileSourceSignature> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value: metadata.last_write_time().to_string(),
        })
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value: format!("{}:{}", metadata.mtime(), metadata.mtime_nsec()),
        })
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        let modified = metadata.modified().map_err(|error| {
            crate::AppError::Repository(format!("failed to read media file modified time: {error}"))
        })?;
        let value = match modified.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
            Err(error) => {
                let duration = error.duration();
                format!("-{}:{}", duration.as_secs(), duration.subsec_nanos())
            }
        };

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value,
        })
    }
}

/// The signature a file modified at `modified` carries in this platform's
/// scheme, spelled exactly as [`file_source_signature_from_metadata`] spells
/// it, so a signature taken from a mark and one read from metadata compare
/// equal. `None` for a time the scheme cannot spell.
pub fn file_source_signature_from_modified(
    modified: std::time::SystemTime,
) -> Option<FileSourceSignature> {
    use std::time::UNIX_EPOCH;

    // Nanoseconds from the epoch, negative before it.
    let nanos: i128 = match modified.duration_since(UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).ok()?,
        Err(before) => -i128::try_from(before.duration().as_nanos()).ok()?,
    };

    #[cfg(windows)]
    let value = {
        // 100-nanosecond intervals since 1601-01-01, the FILETIME epoch.
        const FILETIME_EPOCH_TO_UNIX_TICKS: i128 = 116_444_736_000_000_000;
        let ticks = nanos.div_euclid(100) + FILETIME_EPOCH_TO_UNIX_TICKS;
        u64::try_from(ticks).ok()?.to_string()
    };

    // Seconds round down and the nanoseconds count forward from them, as
    // `st_mtime` and `st_mtime_nsec` do for a time before the epoch.
    #[cfg(unix)]
    let value = {
        let secs = i64::try_from(nanos.div_euclid(1_000_000_000)).ok()?;
        format!("{}:{}", secs, nanos.rem_euclid(1_000_000_000))
    };

    #[cfg(all(not(unix), not(windows)))]
    let value = {
        let magnitude = nanos.unsigned_abs();
        let sign = if nanos < 0 { "-" } else { "" };
        format!(
            "{sign}{}:{}",
            magnitude / 1_000_000_000,
            magnitude % 1_000_000_000
        )
    };

    Some(FileSourceSignature {
        scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn signatures_at(
        modified: std::time::SystemTime,
    ) -> (FileSourceSignature, FileSourceSignature) {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_modified(modified).expect("set modified");
        let metadata = file.as_file().metadata().expect("metadata");
        let from_metadata = file_source_signature_from_metadata(&metadata).expect("signature");
        let from_modified =
            file_source_signature_from_modified(metadata.modified().expect("modified"))
                .expect("signature from modified time");
        (from_metadata, from_modified)
    }

    #[test]
    fn a_time_after_the_epoch_is_spelled_as_metadata_spells_it() {
        let (from_metadata, from_modified) =
            signatures_at(UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789));
        assert_eq!(from_modified, from_metadata);
    }

    #[test]
    fn a_time_before_the_epoch_is_spelled_as_metadata_spells_it() {
        for before in [Duration::new(5, 0), Duration::new(5, 250_000_000)] {
            let (from_metadata, from_modified) = signatures_at(UNIX_EPOCH - before);
            assert_eq!(from_modified, from_metadata, "{before:?} before the epoch");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_time_before_the_epoch_rounds_its_seconds_down() {
        let spelled = |before| {
            file_source_signature_from_modified(UNIX_EPOCH - before)
                .expect("signature")
                .value
        };
        assert_eq!(spelled(Duration::new(5, 0)), "-5:0");
        assert_eq!(spelled(Duration::new(5, 250_000_000)), "-6:750000000");
    }

    #[cfg(windows)]
    #[test]
    fn a_time_is_spelled_in_filetime_ticks() {
        let spelled = |modified| {
            file_source_signature_from_modified(modified)
                .expect("signature")
                .value
        };
        assert_eq!(spelled(UNIX_EPOCH), "116444736000000000");
        assert_eq!(
            spelled(UNIX_EPOCH + Duration::new(1, 500)),
            "116444736010000005"
        );
        assert_eq!(
            spelled(UNIX_EPOCH - Duration::from_secs(1)),
            "116444735990000000"
        );
    }
}
