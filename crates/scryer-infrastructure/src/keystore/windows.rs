use super::KeyStore;

const SERVICE: &str = "scryer";
const ACCOUNT: &str = "encryption-master-key";

/// Stores the encryption key in Windows Credential Manager via the `keyring` crate.
///
/// The credential is tied to the current user account and persists across reboots.
/// Works under NSSM service accounts (Credential Manager is per-user).
pub struct WindowsCredentialManager;

impl WindowsCredentialManager {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|e| format!("failed to create credential entry: {e}"))
    }
}

impl KeyStore for WindowsCredentialManager {
    fn get_key(&self) -> Result<Option<String>, String> {
        let entry = Self::entry()?;
        match entry.get_password() {
            Ok(password) => {
                let trimmed = password.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Windows Credential Manager error: {e}")),
        }
    }

    fn set_key(&self, key_base64: &str) -> Result<(), String> {
        let entry = Self::entry()?;
        entry
            .set_password(key_base64)
            .map_err(|e| format!("failed to store key in Windows Credential Manager: {e}"))
    }

    fn delete_key(&self) -> Result<(), String> {
        let entry = Self::entry()?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
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
        let store = WindowsCredentialManager;
        let test_key = "dGVzdC1rZXktZm9yLWNyZWRtZ3I=";

        let _ = store.delete_key();

        assert!(matches!(store.get_key(), Ok(None)));

        store.set_key(test_key).unwrap();
        assert_eq!(store.get_key().unwrap(), Some(test_key.to_string()));

        store.delete_key().unwrap();
        assert!(matches!(store.get_key(), Ok(None)));
    }
}
