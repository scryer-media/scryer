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

/// The modification time a signature in this platform's scheme records, read
/// back from its text. `None` for another platform's scheme or text this
/// build did not write.
pub fn modified_from_signature(scheme: &str, value: &str) -> Option<std::time::SystemTime> {
    use std::time::{Duration, UNIX_EPOCH};

    if scheme != MEDIA_FILE_SOURCE_SIGNATURE_SCHEME {
        return None;
    }

    #[cfg(windows)]
    {
        // 100-nanosecond intervals since 1601-01-01, the FILETIME epoch.
        const FILETIME_TO_UNIX_SECS: u64 = 11_644_473_600;
        let ticks: u64 = value.trim().parse().ok()?;
        let since_1601 = Duration::new(ticks / 10_000_000, ((ticks % 10_000_000) * 100) as u32);
        let filetime_epoch = UNIX_EPOCH.checked_sub(Duration::from_secs(FILETIME_TO_UNIX_SECS))?;
        filetime_epoch.checked_add(since_1601)
    }

    #[cfg(not(windows))]
    {
        let (secs, nanos) = value.trim().split_once(':')?;
        let nanos: u32 = nanos.parse().ok().filter(|nanos| *nanos < 1_000_000_000)?;
        // A time before the epoch is spelled with a negative seconds count,
        // and the nanoseconds still count forward from it.
        if let Some(before) = secs.strip_prefix('-') {
            let secs: u64 = before.parse().ok()?;
            #[cfg(unix)]
            {
                UNIX_EPOCH
                    .checked_sub(Duration::from_secs(secs))?
                    .checked_add(Duration::from_nanos(u64::from(nanos)))
            }
            #[cfg(not(unix))]
            {
                UNIX_EPOCH.checked_sub(Duration::new(secs, nanos))
            }
        } else {
            let secs: u64 = secs.parse().ok()?;
            UNIX_EPOCH.checked_add(Duration::new(secs, nanos))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_reads_back_as_the_modification_time_it_records() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        let metadata = file.as_file().metadata().expect("metadata");
        let signature = file_source_signature_from_metadata(&metadata).expect("signature");

        assert_eq!(
            modified_from_signature(&signature.scheme, &signature.value),
            Some(metadata.modified().expect("modified"))
        );
    }

    #[test]
    fn another_scheme_or_foreign_text_reads_as_nothing() {
        assert_eq!(modified_from_signature("another_scheme_v1", "1:2"), None);
        assert_eq!(
            modified_from_signature(MEDIA_FILE_SOURCE_SIGNATURE_SCHEME, "not a time"),
            None
        );
    }
}
