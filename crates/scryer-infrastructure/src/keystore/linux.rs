use super::KeyStore;
use std::path::PathBuf;

const DOCKER_SECRET_PATH: &str = "/run/secrets/scryer_encryption_key";

/// Reads the encryption key from a Docker secret mounted at
/// `/run/secrets/scryer_encryption_key`.
///
/// Docker secrets are tmpfs-backed, never written to disk, and not visible
/// via `docker inspect` or `/proc/PID/environ`.
pub struct DockerSecret;

impl KeyStore for DockerSecret {
    fn get_key(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(DOCKER_SECRET_PATH) {
            Ok(contents) => {
                let trimmed = contents.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!(
                "failed to read Docker secret at {DOCKER_SECRET_PATH}: {e}"
            )),
        }
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
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => {
                let trimmed = contents.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!(
                "failed to read key file at {}: {e}",
                self.path.display()
            )),
        }
    }

    fn set_key(&self, key_base64: &str) -> Result<(), String> {
        use std::io::Write;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
        }

        // Open with 0600 from the start to avoid a TOCTOU race where the file
        // is briefly world-readable before permissions are narrowed.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)
                .map_err(|e| {
                    format!("failed to create key file at {}: {e}", self.path.display())
                })?;
            file.write_all(key_base64.as_bytes())
                .map_err(|e| format!("failed to write key file at {}: {e}", self.path.display()))?;
        }

        #[cfg(not(unix))]
        {
            std::fs::write(&self.path, key_base64)
                .map_err(|e| format!("failed to write key file at {}: {e}", self.path.display()))?;
        }

        Ok(())
    }

    fn delete_key(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "failed to delete key file at {}: {e}",
                self.path.display()
            )),
        }
    }

    fn name(&self) -> &'static str {
        "key file"
    }
}

#[cfg(test)]
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
