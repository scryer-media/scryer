use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use scryer_application::application_upgrade::{
    InstallationAssessment, InstallationEvidence, InstallationOs, classify_installation,
};

#[cfg(windows)]
const SCRYER_REGISTRY_KEY: &str = "Software\\Scryer Media\\Scryer";
static WRITABILITY_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn collect_installation_assessment() -> InstallationAssessment {
    let executable_path = env::current_exe().ok().map(canonical_executable_path);
    let (windows_distribution_owner, windows_legacy_msi_registry_key_exists) =
        windows_registry_evidence();

    let evidence = InstallationEvidence {
        disable_self_upgrade: env::var("SCRYER_DISABLE_SELF_UPGRADE").ok(),
        package: env::var("SCRYER_PACKAGE").ok(),
        executable_dir_writable: executable_dir_writable(executable_path.as_deref()),
        docker_env_present: Path::new("/.dockerenv").exists(),
        os: current_os(),
        windows_session_zero: windows_session_zero(),
        windows_task_scheduler_parent: windows_task_scheduler_parent(),
        windows_executable_under_program_files: executable_under_program_files(
            executable_path.as_deref(),
        ),
        windows_distribution_owner,
        windows_legacy_msi_registry_key_exists,
        tray_supervised: env::var("SCRYER_TRAY_SUPERVISED")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true")),
        executable_path,
    };

    classify_installation(&evidence)
}

/// Resolve the executable through symlinks, keeping the raw path if it cannot.
///
/// Package managers install behind symlink farms: Homebrew links
/// `/usr/local/opt/scryer/bin/scryer` and `/home/linuxbrew/.linuxbrew/bin/scryer`
/// into a Cellar, and only the resolved path shows the layout. Both the
/// classifier and the recorded evidence use the same resolved path so later
/// comparisons against the running executable cannot disagree.
fn canonical_executable_path(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

fn current_os() -> InstallationOs {
    match env::consts::OS {
        "windows" => InstallationOs::Windows,
        "macos" => InstallationOs::Macos,
        "linux" => InstallationOs::Linux,
        _ => InstallationOs::Other,
    }
}

fn executable_dir_writable(executable_path: Option<&Path>) -> bool {
    let Some(directory) = executable_path.and_then(Path::parent) else {
        return false;
    };
    let unique_suffix = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos()),
        WRITABILITY_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let probe_path = directory.join(format!(".scryer-write-probe-{unique_suffix}"));

    let created = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe_path)
        .is_ok();
    if !created {
        return false;
    }

    fs::remove_file(probe_path).is_ok()
}

#[cfg(windows)]
fn windows_session_zero() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
    }

    let mut session_id = u32::MAX;
    // SAFETY: The process ID is valid and `session_id` points to writable memory.
    unsafe { ProcessIdToSessionId(std::process::id(), &mut session_id) != 0 && session_id == 0 }
}

#[cfg(windows)]
fn windows_task_scheduler_parent() -> bool {
    use std::mem::size_of;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut processes = Vec::new();
    let mut entry = ProcessEntry32W {
        dw_size: size_of::<ProcessEntry32W>() as u32,
        cnt_usage: 0,
        th32_process_id: 0,
        th32_default_heap_id: 0,
        th32_module_id: 0,
        cnt_threads: 0,
        th32_parent_process_id: 0,
        pc_pri_class_base: 0,
        dw_flags: 0,
        sz_exe_file: [0; 260],
    };
    // SAFETY: The snapshot handle is valid and `entry` is initialized with its size.
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
    while next != 0 {
        let name_end = entry
            .sz_exe_file
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(entry.sz_exe_file.len());
        processes.push((
            entry.th32_process_id,
            entry.th32_parent_process_id,
            String::from_utf16_lossy(&entry.sz_exe_file[..name_end]),
        ));
        entry.dw_size = size_of::<ProcessEntry32W>() as u32;
        // SAFETY: The snapshot handle and entry buffer remain valid for iteration.
        next = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    // SAFETY: This function owns the snapshot handle returned above.
    unsafe { CloseHandle(snapshot) };

    let current = std::process::id();
    let parent = processes
        .iter()
        .find(|(process_id, _, _)| *process_id == current)
        .map(|(_, parent_process_id, _)| *parent_process_id);
    parent
        .and_then(|parent_process_id| {
            processes
                .iter()
                .find(|(process_id, _, _)| *process_id == parent_process_id)
        })
        .is_some_and(|(_, _, image)| is_windows_task_scheduler_parent_image(image))
}

#[cfg(not(windows))]
fn windows_task_scheduler_parent() -> bool {
    false
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_windows_task_scheduler_parent_image(image: &str) -> bool {
    matches!(
        image
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(image)
            .to_ascii_lowercase()
            .as_str(),
        "taskeng.exe" | "taskhost.exe" | "taskhostw.exe"
    )
}

#[cfg(not(windows))]
fn windows_session_zero() -> bool {
    false
}

#[cfg(windows)]
fn windows_registry_evidence() -> (Option<String>, bool) {
    use std::ptr;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
    };

    let mut key: HKEY = ptr::null_mut();
    let key_path = wide(SCRYER_REGISTRY_KEY);
    // SAFETY: The registry path is nul-terminated and `key` points to writable memory.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return (None, false);
    }

    let owner = registry_string_value(key, "DistributionOwner", REG_SZ);
    // SAFETY: This function owns the registry key returned by `RegOpenKeyExW`.
    unsafe { RegCloseKey(key) };
    (owner, true)
}

#[cfg(not(windows))]
fn windows_registry_evidence() -> (Option<String>, bool) {
    (None, false)
}

#[cfg(windows)]
fn registry_string_value(
    key: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
    expected_type: u32,
) -> Option<String> {
    use std::ptr;
    use windows_sys::Win32::System::Registry::RegQueryValueExW;

    let name = wide(name);
    let mut value_type = 0_u32;
    let mut byte_len = 0_u32;
    // SAFETY: The key and value name are valid; output pointers are writable.
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status != 0 || value_type != expected_type || byte_len == 0 {
        return None;
    }

    let mut value = vec![0_u16; (byte_len as usize).div_ceil(2)];
    // SAFETY: The buffer is allocated for the reported byte length and all pointers are valid.
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            value.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != 0 || value_type != expected_type {
        return None;
    }

    let terminator = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    Some(String::from_utf16_lossy(&value[..terminator]))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn executable_under_program_files(executable_path: Option<&Path>) -> bool {
    let Some(executable_path) = executable_path else {
        return false;
    };
    let executable = executable_path.to_string_lossy().to_ascii_lowercase();

    ["ProgramFiles", "ProgramW6432"].into_iter().any(|name| {
        env::var_os(name).is_some_and(|program_files| {
            let program_files = PathBuf::from(program_files);
            let program_files = program_files.to_string_lossy().to_ascii_lowercase();
            let program_files = program_files.trim_end_matches(['\\', '/']);
            executable == program_files
                || executable
                    .strip_prefix(program_files)
                    .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
        })
    })
}

#[cfg(not(windows))]
fn executable_under_program_files(_executable_path: Option<&Path>) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_task_scheduler_parent_images() {
        for image in [
            "taskeng.exe",
            "TASKHOST.EXE",
            "C:\\Windows\\System32\\taskhostw.exe",
        ] {
            assert!(is_windows_task_scheduler_parent_image(image));
        }
        for image in ["explorer.exe", "scryer-tray.exe", "taskhost-helper.exe"] {
            assert!(!is_windows_task_scheduler_parent_image(image));
        }
    }

    #[test]
    fn canonicalization_falls_back_to_the_raw_path() {
        let missing = PathBuf::from("/this/path/does/not/exist/scryer");
        assert_eq!(canonical_executable_path(missing.clone()), missing);
    }

    #[cfg(unix)]
    #[test]
    fn canonicalization_resolves_a_symlinked_executable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let real_dir = temp.path().join("Cellar/scryer/0.18.22/bin");
        fs::create_dir_all(&real_dir).expect("create cellar directory");
        let real_path = real_dir.join("scryer");
        fs::write(&real_path, b"binary").expect("write executable");
        let link_dir = temp.path().join("opt/scryer/bin");
        fs::create_dir_all(&link_dir).expect("create link directory");
        let link_path = link_dir.join("scryer");
        std::os::unix::fs::symlink(&real_path, &link_path).expect("link executable");

        assert_eq!(
            canonical_executable_path(link_path),
            fs::canonicalize(&real_path).expect("canonical real path")
        );
    }
}
