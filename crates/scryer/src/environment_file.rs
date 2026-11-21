//! Parse configuration completely before applying any of it to a process.

use std::io::Read;
use std::path::Path;

use scryer_application::disk::{Disk, IfPresent, Open};

pub(crate) fn read(path: &Path) -> Result<Vec<(String, String)>, String> {
    read_with_context(path, &std::collections::BTreeMap::new())
}

pub(crate) fn read_with_context(
    path: &Path,
    context: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, String> {
    let file = match Disk::process()
        .open_blocking(path, Open::read())
        .if_present()
    {
        Ok(Some(file)) => file,
        Ok(None) => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "cannot read environment file {}: {}",
                path.display(),
                std::io::Error::from(error).kind()
            ));
        }
    };
    // Restart dotenvy's substitution map at each assignment so earlier
    // assignments retain the same precedence as from_path, without mutating
    // the launcher's process environment. The reader prevents lookahead from
    // consuming the next assignment before we restore that effective context.
    let mut file = std::io::BufReader::new(file);
    let mut effective = context.clone();
    let mut values = Vec::new();
    loop {
        let mut prefix = String::new();
        for (name, value) in &effective {
            let value = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('\n', "\\n");
            prefix.push_str(&format!("{name}=\"{value}\"\n"));
        }
        let mut parser =
            dotenvy::from_read_iter(std::io::Cursor::new(prefix).chain(StatementReader(&mut file)));
        let invalid = || {
            format!(
                "invalid environment file {}; check KEY=value syntax",
                path.display()
            )
        };
        for _ in 0..effective.len() {
            parser.next().ok_or_else(invalid)?.map_err(|_| invalid())?;
        }
        let Some(value) = parser.next() else {
            break;
        };
        let (name, value) = value.map_err(|_| invalid())?;
        if name.contains('\0') || value.contains('\0') {
            return Err(invalid());
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = effective.entry(name.clone()) {
            entry.insert(value.clone());
            values.push((name, value));
        }
    }
    Ok(values)
}

// dotenvy's iterator buffers its reader. Supplying one byte at a time from
// our persistent buffered file leaves it exactly after the parsed statement,
// including multiline quoted values, when the iterator is dropped.
struct StatementReader<'a, R>(&'a mut R);
impl<R: Read> Read for StatementReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = buffer.len().min(1);
        self.0.read(&mut buffer[..length])
    }
}

#[allow(
    dead_code,
    reason = "The shared parser is also compiled by the standalone server"
)]
pub(crate) fn desktop_owned(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "SCRYER_DATA_DIR"
            | "SCRYER_DB_PATH"
            | "SCRYER_DB_URL"
            | "SCRYER_DB_USER"
            | "SCRYER_DB_PASSWORD"
            | "SCRYER_DB_PASSWORD_FILE"
            | "SCRYER_ENCRYPTION_KEY"
            | "SCRYER_ALLOW_EPHEMERAL_ENCRYPTION_KEY"
            | "SCRYER_DISABLE_PLATFORM_KEYSTORE"
            | "SCRYER_BIND"
            | "SCRYER_BASE_PATH"
            | "SCRYER_LOG_FILE"
            | "SCRYER_OPEN_BROWSER"
            | "SCRYER_TRAY_SUPERVISED"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_file_never_returns_partial_configuration_or_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "VALID=example\nINVALID='private-value\n").unwrap();
        let error = read(&path).unwrap_err();
        assert!(!error.contains("private-value"));
    }

    #[test]
    fn missing_file_is_optional_but_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&dir.path().join(".env")).unwrap().is_empty());
        assert!(read(dir.path()).is_err());
    }

    #[test]
    fn only_managed_desktop_configuration_is_reserved() {
        assert!(desktop_owned("scryer_db_url"));
        assert!(desktop_owned("SCRYER_ENCRYPTION_KEY"));
        assert!(desktop_owned("SCRYER_DISABLE_PLATFORM_KEYSTORE"));
        assert!(!desktop_owned("SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS"));
        assert!(!desktop_owned("SCRYER_AUTH_ENABLED"));
    }

    #[test]
    fn later_files_can_expand_previously_parsed_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let context = std::collections::BTreeMap::from([
            ("PROFILE_TEST_HOST".into(), "127.0.0.1".into()),
            ("PROFILE_TEST_LITERAL".into(), "a$b\\c\"d\r".into()),
        ]);
        std::fs::write(
            &path,
            "PROFILE_TEST_HOST=0.0.0.0\nSCRYER_BIND=${PROFILE_TEST_HOST}:8080\nPROFILE_TEST_COPY=${PROFILE_TEST_LITERAL}\n",
        )
        .unwrap();
        let values = read_with_context(&path, &context).unwrap();
        assert_eq!(values[0], ("SCRYER_BIND".into(), "127.0.0.1:8080".into()));
        assert_eq!(values[1].1, "a$b\\c\"d\r");
    }

    #[test]
    fn multiline_values_and_duplicate_assignments_preserve_first_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "PROFILE_TEST_FIRST='line one\nline two'\nPROFILE_TEST_FIRST=discarded\nPROFILE_TEST_SECOND=${PROFILE_TEST_FIRST}\n").unwrap();
        let values = read(&path).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].1, "line one\nline two");
        assert_eq!(values[1].1, "line one\nline two");
    }
}
