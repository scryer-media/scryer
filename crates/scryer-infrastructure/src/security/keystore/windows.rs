use super::KeyStore;
use std::{path::Path, sync::OnceLock};

use keyring_core::{Entry, Error as KeyringError};
use windows_native_keyring_store::Store;

const DEFAULT_SERVICE: &str = "scryer";
const DESKTOP_SERVICE: &str = "ScryerMedia.Scryer.Desktop.v1";
const ACCOUNT: &str = "encryption-master-key";

/// Stores the encryption key in Windows Credential Manager via the keyring core/store crates.
///
/// The credential is tied to the current user account and persists across reboots.
/// Works under NSSM service accounts (Credential Manager is per-user).
pub struct WindowsCredentialManager {
    service: &'static str,
}

impl WindowsCredentialManager {
    pub fn for_data_dir(data_dir: Option<&Path>) -> Self {
        let service = data_dir
            .filter(|path| is_desktop_profile(path))
            .map(|_| DESKTOP_SERVICE)
            .unwrap_or(DEFAULT_SERVICE);
        Self { service }
    }

    fn entry(&self) -> Result<Entry, String> {
        static STORE_INIT: OnceLock<Result<(), String>> = OnceLock::new();

        STORE_INIT
            .get_or_init(|| {
                let store = Store::new().map_err(|e| {
                    format!("failed to initialize Windows Credential Manager store: {e}")
                })?;
                keyring_core::set_default_store(store);
                Ok(())
            })
            .as_ref()
            .map_err(Clone::clone)?;

        Entry::new(self.service, ACCOUNT)
            .map_err(|e| format!("failed to create credential entry: {e}"))
    }
}

fn is_desktop_profile(path: &Path) -> bool {
    let mut components = path.components().rev();
    let Some(profile_name) = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
    else {
        return false;
    };
    let Some(publisher_name) = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
    else {
        return false;
    };

    profile_name.eq_ignore_ascii_case("Scryer")
        && publisher_name.eq_ignore_ascii_case("ScryerMedia")
}

impl KeyStore for WindowsCredentialManager {
    fn get_key(&self) -> Result<Option<String>, String> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(password) => {
                let trimmed = password.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(format!("Windows Credential Manager error: {e}")),
        }
    }

    fn set_key(&self, key_base64: &str) -> Result<(), String> {
        let entry = self.entry()?;
        entry
            .set_password(key_base64)
            .map_err(|e| format!("failed to store key in Windows Credential Manager: {e}"))
    }

    fn delete_key(&self) -> Result<(), String> {
        let entry = self.entry()?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(format!(
                "failed to delete key from Windows Credential Manager: {e}"
            )),
        }
    }

    fn name(&self) -> &'static str {
        "Windows Credential Manager"
    }
}

#[cfg(test)]
mod tests {
    // Credential Manager tests require a Windows user session.
    // Run manually: cargo nextest run -p scryer-infrastructure keystore::windows --ignored

    use super::*;

    #[test]
    #[ignore = "requires Windows user session — run manually"]
    fn credential_manager_round_trip() {
        let store = WindowsCredentialManager::for_data_dir(None);
        let test_key = "dGVzdC1rZXktZm9yLWNyZWRtZ3I=";
        let original = store.get_key().unwrap();

        store.set_key(test_key).unwrap();
        assert_eq!(store.get_key().unwrap(), Some(test_key.to_string()));

        match original {
            Some(previous) => {
                store.set_key(&previous).unwrap();
                assert_eq!(store.get_key().unwrap(), Some(previous));
            }
            None => {
                store.delete_key().unwrap();
                assert!(matches!(store.get_key(), Ok(None)));
            }
        }
    }

    #[test]
    fn desktop_profile_uses_dedicated_credential_namespace() {
        let profile = Path::new(r"C:\Users\example\AppData\Local\ScryerMedia\Scryer");
        let desktop_store = WindowsCredentialManager::for_data_dir(Some(profile));
        let legacy_store = WindowsCredentialManager::for_data_dir(Some(Path::new(
            r"C:\Users\example\AppData\Roaming\scryer",
        )));

        assert_eq!(desktop_store.service, DESKTOP_SERVICE);
        assert_eq!(legacy_store.service, DEFAULT_SERVICE);
    }
}
