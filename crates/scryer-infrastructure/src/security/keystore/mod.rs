//! Platform-native keystore backends for the encryption master key.
//!
//! Each platform compiles only its own backend — no dead code from other platforms.
//! The priority chain in [`platform_keystores`] returns backends in descending
//! priority order; callers iterate and use the first one that returns a key.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::PathBuf;
use std::sync::{
    Once,
    atomic::{AtomicBool, Ordering},
};

const DISABLE_PLATFORM_KEYSTORE_ENV: &str = "SCRYER_DISABLE_PLATFORM_KEYSTORE";
static DISABLE_PLATFORM_KEYSTORE_FOR_PROCESS: AtomicBool = AtomicBool::new(false);

/// A backend that can store and retrieve the encryption master key.
pub trait KeyStore: Send + Sync {
    /// Retrieve the base64-encoded encryption key, if stored.
    fn get_key(&self) -> Result<Option<String>, String>;

    /// Store the base64-encoded encryption key.
    fn set_key(&self, key_base64: &str) -> Result<(), String>;

    /// Delete the stored key.
    fn delete_key(&self) -> Result<(), String>;

    /// Human-readable name for log messages (e.g. "macOS Keychain").
    fn name(&self) -> &'static str;
}

#[doc(hidden)]
pub fn disable_platform_keystore_for_tests() {
    DISABLE_PLATFORM_KEYSTORE_FOR_PROCESS.store(true, Ordering::SeqCst);

    static SET_DISABLE_ENV: Once = Once::new();
    SET_DISABLE_ENV.call_once(|| {
        // Test helpers call this before constructing app services. Setting the
        // env flag makes the platform-keystore block inherited by any child
        // process spawned from the test.
        unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, "1") };
    });
}

/// Returns platform-native keystores in priority order.
///
/// `data_dir` is the application data directory (resolved by the binary crate).
/// Linux uses it for the `KeyFile` backend; Windows uses the dedicated desktop
/// profile path to select its isolated Credential Manager namespace.
#[allow(clippy::vec_init_then_push)] // conditional cfg pushes can't use vec![]
pub fn platform_keystores(data_dir: Option<PathBuf>) -> Vec<Box<dyn KeyStore>> {
    if platform_keystore_disabled() {
        return Vec::new();
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    let _ = &data_dir;

    let mut stores: Vec<Box<dyn KeyStore>> = Vec::new();

    #[cfg(target_os = "macos")]
    stores.push(Box::new(macos::MacOSKeychain));

    #[cfg(target_os = "windows")]
    stores.push(Box::new(windows::WindowsCredentialManager::for_data_dir(
        data_dir.as_deref(),
    )));

    #[cfg(target_os = "linux")]
    {
        stores.push(Box::new(linux::DockerSecret));
        if let Some(dir) = data_dir {
            stores.push(Box::new(linux::KeyFile::new(dir)));
        }
    }

    stores
}

fn platform_keystore_disabled() -> bool {
    if DISABLE_PLATFORM_KEYSTORE_FOR_PROCESS.load(Ordering::SeqCst) {
        return true;
    }

    if cfg!(test) {
        return true;
    }

    if running_under_rust_test_harness() {
        return true;
    }

    platform_keystore_disabled_by_env()
}

fn platform_keystore_disabled_by_env() -> bool {
    std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn running_under_rust_test_harness() -> bool {
    // Cargo exposes this to integration tests and benches, and child processes
    // inherit it. Treat that process tree as non-interactive for keystore use.
    if std::env::var_os("CARGO_TARGET_TMPDIR").is_some() {
        return true;
    }

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(parent) = exe.parent() else {
        return false;
    };
    if parent.file_name().and_then(|name| name.to_str()) != Some("deps") {
        return false;
    }

    exe.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(has_rust_test_binary_hash_suffix)
}

fn has_rust_test_binary_hash_suffix(stem: &str) -> bool {
    stem.rsplit_once('-').is_some_and(|(_, suffix)| {
        suffix.len() >= 8 && suffix.chars().all(|ch| ch.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn platform_keystore_flag_defaults_to_enabled_when_env_is_absent() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).ok();
        unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) };

        assert!(!platform_keystore_disabled_by_env());

        match original {
            Some(value) => unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, value) },
            None => unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) },
        }
    }

    #[test]
    fn platform_keystore_flag_disables_stores() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).ok();
        unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, "1") };

        assert!(platform_keystore_disabled_by_env());
        assert!(platform_keystores(None).is_empty());

        match original {
            Some(value) => unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, value) },
            None => unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) },
        }
    }

    #[test]
    fn test_helper_sets_inheritable_disable_flag() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).ok();
        unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) };

        disable_platform_keystore_for_tests();

        assert_eq!(
            std::env::var(DISABLE_PLATFORM_KEYSTORE_ENV).as_deref(),
            Ok("1")
        );

        match original {
            Some(value) => unsafe { std::env::set_var(DISABLE_PLATFORM_KEYSTORE_ENV, value) },
            None => unsafe { std::env::remove_var(DISABLE_PLATFORM_KEYSTORE_ENV) },
        }
    }

    #[test]
    fn platform_keystore_is_disabled_in_test_binaries() {
        assert!(platform_keystore_disabled());
        assert!(platform_keystores(None).is_empty());
    }

    #[test]
    fn detects_rust_test_binary_hash_suffix() {
        assert!(has_rust_test_binary_hash_suffix(
            "integration_graphql-a1b2c3d4e5f6"
        ));
        assert!(!has_rust_test_binary_hash_suffix("scryer"));
        assert!(!has_rust_test_binary_hash_suffix("not-a-test"));
    }
}
