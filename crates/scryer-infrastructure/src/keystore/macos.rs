use super::KeyStore;

const SERVICE: &str = "media.scryer.app";
const ACCOUNT: &str = "encryption-master-key";

/// Stores the encryption key in the macOS login Keychain via Security.framework.
///
/// The keychain entry is identified by service `media.scryer.app` and account
/// `encryption-master-key`. The binary must be codesigned with a stable identity
/// (self-signed certificate) so that Keychain ACLs survive across upgrades.
pub struct MacOSKeychain;

impl KeyStore for MacOSKeychain {
    fn get_key(&self) -> Result<Option<String>, String> {
        match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
            Ok(bytes) => {
                let key = String::from_utf8(bytes)
                    .map_err(|e| format!("keychain entry is not valid UTF-8: {e}"))?;
                let trimmed = key.trim().to_string();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed))
                }
            }
            Err(e) if e.code() == security_framework_sys::base::errSecItemNotFound => Ok(None),
            Err(e) => Err(format!("macOS Keychain error: {e}")),
        }
    }

    fn set_key(&self, key_base64: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, key_base64.as_bytes())
            .map_err(|e| format!("failed to store key in macOS Keychain: {e}"))
    }

    fn delete_key(&self) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == security_framework_sys::base::errSecItemNotFound => Ok(()),
            Err(e) => Err(format!("failed to delete key from macOS Keychain: {e}")),
        }
    }

    fn name(&self) -> &'static str {
        "macOS Keychain"
    }
}

#[cfg(test)]
mod tests {
    // Keychain tests require an interactive macOS session with a login keychain.
    // They cannot run in CI (headless runners get errSecInteractionNotAllowed).
    // Run manually: cargo nextest run -p scryer-infrastructure keystore::macos --ignored

    use super::*;

    #[test]
    #[ignore = "requires macOS login keychain — run manually"]
    fn keychain_round_trip() {
        let store = MacOSKeychain;
        let test_key = "dGVzdC1rZXktZm9yLWtleWNoYWlu";

        // Clean up any leftover from a previous test run
        let _ = store.delete_key();

        assert!(matches!(store.get_key(), Ok(None)));

        store.set_key(test_key).unwrap();
        assert_eq!(store.get_key().unwrap(), Some(test_key.to_string()));

        // Overwrite
        let new_key = "bmV3LXRlc3Qta2V5LXZhbHVl";
        store.set_key(new_key).unwrap();
        assert_eq!(store.get_key().unwrap(), Some(new_key.to_string()));

        store.delete_key().unwrap();
        assert!(matches!(store.get_key(), Ok(None)));
    }
}
