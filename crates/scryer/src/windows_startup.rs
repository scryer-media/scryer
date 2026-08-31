//! Per-user Windows startup registration shared by the tray and the upgrade helper.
//!
//! The tray owns the "start Scryer when I sign in" preference, and the temporary
//! upgrade helper has to observe and restore it around an MSI major upgrade.
//! Both binaries include this module, so each uses only part of it.
#![allow(dead_code)]

use std::path::Path;
use std::ptr;

use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

pub(crate) const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
pub(crate) const RUN_VALUE: &str = "ScryerMedia.Scryer";

/// Point the per-user Run value at `executable`, launching it in login mode.
pub(crate) fn register_startup(executable: &Path) -> Result<(), String> {
    let mut key: HKEY = ptr::null_mut();
    let key_path = wide(RUN_KEY);
    let mut disposition = 0;
    // SAFETY: The registry path and output pointers are valid for the call.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            ptr::null(),
            0,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            &mut disposition,
        )
    };
    if status != 0 {
        return Err(format!(
            "failed to open Windows startup registry key: error {status}"
        ));
    }
    let value_name = wide(RUN_VALUE);
    let command = wide(&format!("\"{}\" --login-start", executable.display()));
    // SAFETY: The registry key is open and command contains a terminating UTF-16 nul.
    let status = unsafe {
        RegSetValueExW(
            key,
            value_name.as_ptr(),
            0,
            REG_SZ,
            command.as_ptr().cast(),
            (command.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    // SAFETY: This function owns the registry handle returned above.
    unsafe { RegCloseKey(key) };
    if status != 0 {
        return Err(format!("failed to enable Scryer startup: error {status}"));
    }
    Ok(())
}

/// Remove the per-user Run value, tolerating a key or value that is already gone.
pub(crate) fn unregister_startup() -> Result<(), String> {
    let mut key: HKEY = ptr::null_mut();
    let key_path = wide(RUN_KEY);
    // SAFETY: The registry path and output key pointer are valid for the call.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return Ok(());
    }
    let value_name = wide(RUN_VALUE);
    // SAFETY: The key is open and the value name is nul-terminated.
    let status = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
    // SAFETY: This function owns the registry handle returned above.
    unsafe { RegCloseKey(key) };
    if status != 0 && status != 2 {
        return Err(format!("failed to disable Scryer startup: error {status}"));
    }
    Ok(())
}

/// Whether the per-user Run value currently exists.
pub(crate) fn startup_enabled() -> Result<bool, String> {
    let mut key: HKEY = ptr::null_mut();
    let key_path = wide(RUN_KEY);
    // SAFETY: The registry path and output key pointer are valid for the call.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return Ok(false);
    }
    let value_name = wide(RUN_VALUE);
    // SAFETY: The key is open and we only query the value's metadata.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    // SAFETY: This function owns the registry handle returned above.
    unsafe { RegCloseKey(key) };
    Ok(status == 0)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
