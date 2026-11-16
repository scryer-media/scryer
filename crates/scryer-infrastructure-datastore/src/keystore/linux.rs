use super::KeyStore;
use scryer_application::disk::{Access, Disk, IfPresent, Open, Parents, Remove};
use std::path::{Path, PathBuf};

const DOCKER_SECRET_PATH: &str = "/run/secrets/scryer_encryption_key";

/// The trimmed key at `path`; a missing or blank file holds none.
fn read_key(path: &Path) -> Result<Option<String>, std::io::Error> {
    let contents = Disk::process()
        .read_to_string_blocking(path)
        .if_present()
        .map_err(std::io::Error::from)?;
    Ok(contents
        .map(|contents| contents.trim().to_string())
        .filter(|trimmed| !trimmed.is_empty()))
}

/// Reads the encryption key from a Docker secret mounted at
/// `/run/secrets/scryer_encryption_key`.
///
/// Docker secrets are tmpfs-backed, never written to disk, and not visible
/// via `docker inspect` or `/proc/PID/environ`.
pub struct DockerSecret;

impl KeyStore for DockerSecret {
    fn get_key(&self) -> Result<Option<String>, String> {
        read_key(Path::new(DOCKER_SECRET_PATH))
            .map_err(|e| format!("failed to read Docker secret at {DOCKER_SECRET_PATH}: {e}"))
    }

    fn set_key(&self, _key_base64: &str) -> Result<(), String> {
        Err("Docker secrets are read-only (mounted as tmpfs by the container runtime)".into())
    }

    fn delete_key(&self) -> Result<(), String> {
        Err("Docker secrets are read-only".into())
    }

    fn name(&self) -> &'static str {
        "Docker secret"
    }
}

/// Stores the encryption key as a file in the data directory with `0600` permissions.
///
/// This is the standard approach for headless Linux servers and Synology NAS devices
/// where no OS keychain is available.
pub struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("encryption.key"),
        }
    }
}

impl KeyStore for KeyFile {
    fn get_key(&self) -> Result<Option<String>, String> {
        read_key(&self.path)
            .map_err(|e| format!("failed to read key file at {}: {e}", self.path.display()))
    }

    fn set_key(&self, key_base64: &str) -> Result<(), String> {
        use std::io::Write;

        let disk = Disk::process();
        if let Some(parent) = self.path.parent() {
            disk.create_dir_blocking(parent, Parents::Create)
                .map_err(|e| {
                    format!(
                        "failed to create directory {}: {}",
                        parent.display(),
                        std::io::Error::from(e)
                    )
                })?;
        }

        // Owner-only from the start to avoid a TOCTOU race where the file
        // is briefly world-readable before permissions are narrowed.
        let mut file = disk
            .open_blocking(&self.path, Open::create().with_access(Access::OwnerOnly))
            .map_err(|e| {
                format!(
                    "failed to create key file at {}: {}",
                    self.path.display(),
                    std::io::Error::from(e)
                )
            })?;
        file.write_all(key_base64.as_bytes())
            .map_err(|e| format!("failed to write key file at {}: {e}", self.path.display()))?;
        Ok(())
    }

    fn delete_key(&self) -> Result<(), String> {
        Disk::process()
            .remove_blocking(&self.path, Remove::File)
            .map(drop)
            .map_err(|e| {
                format!(
                    "failed to delete key file at {}: {}",
                    self.path.display(),
                    std::io::Error::from(e)
                )
            })
    }

    fn name(&self) -> &'static str {
        "key file"
    }
}

#[cfg(test)]
// disk-guard: test, example and build-script code may call the filesystem directly.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn docker_secret_missing_returns_none() {
        // /run/secrets/scryer_encryption_key won't exist in test env
        let store = DockerSecret;
        assert!(matches!(store.get_key(), Ok(None)));
    }

    #[test]
    fn key_file_missing_returns_none() {
        let dir = std::env::temp_dir().join("scryer-test-keyfile-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let store = KeyFile::new(dir);
        assert!(matches!(store.get_key(), Ok(None)));
    }

    #[test]
    fn key_file_round_trip() {
        let dir = std::env::temp_dir().join("scryer-test-keyfile-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        let store = KeyFile::new(dir.clone());
        let key = "dGVzdC1rZXktYmFzZTY0LWVuY29kZWQ=";

        store.set_key(key).unwrap();
        assert_eq!(store.get_key().unwrap(), Some(key.to_string()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(dir.join("encryption.key"))
                .unwrap()
                .permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        store.delete_key().unwrap();
        assert!(matches!(store.get_key(), Ok(None)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_empty_returns_none() {
        let dir = std::env::temp_dir().join("scryer-test-keyfile-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("encryption.key"), "  \n  ").unwrap();

        let store = KeyFile::new(dir.clone());
        assert!(matches!(store.get_key(), Ok(None)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_trims_whitespace() {
        let dir = std::env::temp_dir().join("scryer-test-keyfile-trim");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("encryption.key"), "  abc123=\n").unwrap();

        let store = KeyFile::new(dir.clone());
        assert_eq!(store.get_key().unwrap(), Some("abc123=".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
