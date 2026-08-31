use std::path::Path;

#[cfg(windows)]
use scryer_application::application_upgrade::{
    APPLICATION_UPGRADE_HELPER_WAIT_BUDGET, ApplicationUpgradeHelperMode,
    ApplicationUpgradeHelperOwner, WriteProbeOutcome, classify_write_probe_error,
    helper_wait_remaining, helper_write_probe_required, msi_install_succeeded,
    open_process_failure_means_exited, should_restore_tray_startup,
};
use scryer_application::application_upgrade::{
    ApplicationUpgradeHelperPlan, MsiHelperJournalTransition, msi_exit_code_transition,
};

pub fn maybe_run_upgrade_helper() -> Result<bool, String> {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--upgrade-helper")) {
        return Ok(false);
    }
    let plan_path = args
        .next()
        .ok_or_else(|| "--upgrade-helper requires a plan path".to_string())?;
    if args.next().is_some() {
        return Err("--upgrade-helper accepts exactly one plan path".to_string());
    }
    let plan = read_plan(Path::new(&plan_path))?;
    #[cfg(windows)]
    {
        run_windows_helper(&plan)?;
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let _ = plan;
        Err("--upgrade-helper is only available on Windows".to_string())
    }
}

fn read_plan(path: &Path) -> Result<ApplicationUpgradeHelperPlan, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read application upgrade helper plan: {error}"))?;
    let plan: ApplicationUpgradeHelperPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid application upgrade helper plan: {error}"))?;
    plan.validate()?;
    Ok(plan)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn msi_launch_error_transition(win32_error: u32) -> MsiHelperJournalTransition {
    if win32_error == 1223 {
        msi_exit_code_transition(win32_error)
    } else {
        MsiHelperJournalTransition::HelperError(format!(
            "failed to launch elevated installer (win32 error {win32_error})"
        ))
    }
}

#[cfg(windows)]
fn run_windows_helper(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    let outcome = (|| {
        stop_owner(plan)?;
        let started = std::time::Instant::now();
        wait_for_process_exit(&plan.wait_process_ids, started)?;
        // msiexec owns in-use file semantics for MSI installs; probing there in
        // an unelevated helper only ever reports ERROR_ACCESS_DENIED against
        // Program Files and would burn the whole budget before the UAC prompt.
        if helper_write_probe_required(plan.mode) {
            wait_for_file_release(&installed_executables(plan), started)?;
        }
        match plan.mode {
            ApplicationUpgradeHelperMode::Portable => apply_portable_replacements(plan),
            ApplicationUpgradeHelperMode::Msi => run_msi_installer(plan),
        }
    })();

    if let Err(error) = &outcome {
        write_helper_error(plan, error.clone());
    }
    if let Err(error) = relaunch_owner(plan) {
        let message = format!("failed to relaunch application after upgrade helper: {error}");
        write_helper_error(plan, message.clone());
        return Err(message);
    }
    outcome
}

#[cfg(windows)]
fn stop_owner(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    if plan.owner != ApplicationUpgradeHelperOwner::Tray {
        return Ok(());
    }
    let program = plan
        .tray_shutdown_program
        .as_ref()
        .ok_or_else(|| "tray-owned helper plan has no shutdown program".to_string())?;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let status = std::process::Command::new(program)
        .arg("--shutdown")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| format!("failed to invoke tray shutdown: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tray shutdown exited with {status}"))
    }
}

#[cfg(windows)]
fn installed_executables(plan: &ApplicationUpgradeHelperPlan) -> Vec<std::path::PathBuf> {
    plan.replace
        .iter()
        .map(|replacement| replacement.to_install.clone())
        .collect()
}

/// Wait for every process the plan named to exit, within the shared budget.
///
/// This is the real gate: once the backend (and, for tray-owned plans, the tray
/// it supervises) is gone, the installation is free for either msiexec or the
/// portable replacement to touch.
#[cfg(windows)]
fn wait_for_process_exit(process_ids: &[u32], started: std::time::Instant) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    for process_id in process_ids {
        let Some(remaining) =
            helper_wait_remaining(started.elapsed(), APPLICATION_UPGRADE_HELPER_WAIT_BUDGET)
        else {
            return Err(format!(
                "timed out waiting for process {process_id} to exit before upgrading"
            ));
        };
        // SAFETY: OpenProcess takes plain integers and returns either a handle
        // this function owns or null.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, *process_id) };
        if handle.is_null() {
            // SAFETY: GetLastError reads the thread-local error set by OpenProcess.
            let error = unsafe { GetLastError() };
            if open_process_failure_means_exited(error) {
                continue;
            }
            return Err(format!(
                "failed to wait for process {process_id} (win32 error {error})"
            ));
        }
        let milliseconds = u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: The handle was returned by OpenProcess and is closed below.
        let status = unsafe { WaitForSingleObject(handle, milliseconds) };
        // SAFETY: This function owns the handle returned by OpenProcess.
        unsafe { CloseHandle(handle) };
        if status == WAIT_TIMEOUT {
            return Err(format!(
                "timed out waiting for process {process_id} to exit before upgrading"
            ));
        }
        if status != WAIT_OBJECT_0 {
            // SAFETY: GetLastError reads the thread-local error set by the wait.
            let error = unsafe { GetLastError() };
            return Err(format!(
                "failed to wait for process {process_id} (wait status {status}, win32 error {error})"
            ));
        }
    }
    Ok(())
}

/// Confirm the installed executables can be written before replacing them.
///
/// A sharing violation means a process is still letting go and is worth
/// retrying; a permission denial never resolves on its own, so it fails fast
/// with a message an operator can act on.
#[cfg(windows)]
fn wait_for_file_release(
    paths: &[std::path::PathBuf],
    started: std::time::Instant,
) -> Result<(), String> {
    loop {
        let mut blocked = None;
        for path in paths {
            if let Err(error) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                match classify_write_probe_error(error.kind(), error.raw_os_error()) {
                    WriteProbeOutcome::Fatal(message) => {
                        return Err(format!("{message}: {}", path.display()));
                    }
                    WriteProbeOutcome::Retry => {
                        blocked = Some(path.clone());
                        break;
                    }
                }
            }
        }
        let Some(blocked) = blocked else {
            return Ok(());
        };
        if helper_wait_remaining(started.elapsed(), APPLICATION_UPGRADE_HELPER_WAIT_BUDGET)
            .is_none()
        {
            return Err(format!(
                "timed out waiting for installed executables to be released: {}",
                blocked.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(windows)]
fn apply_portable_replacements(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use scryer_application::application_upgrade::portable_replacement_operations;

    let mut completed = Vec::new();
    for replacement in &plan.replace {
        let operations = portable_replacement_operations(replacement, &plan.backup_suffix);
        if let Err(error) =
            std::fs::rename(&operations.retain_backup_from, &operations.retain_backup_to)
        {
            rollback_replacements(&completed, None)?;
            return Err(format!(
                "failed to retain installed executable backup: {error}"
            ));
        }
        if let Err(error) = std::fs::rename(&operations.install_from, &operations.install_to) {
            rollback_replacements(&completed, Some(&operations))?;
            return Err(format!("failed to install staged executable: {error}"));
        }
        completed.push(operations);
    }
    Ok(())
}

#[cfg(windows)]
fn rollback_replacements(
    completed: &[scryer_application::application_upgrade::PortableReplacementOperations],
    backup_only: Option<&scryer_application::application_upgrade::PortableReplacementOperations>,
) -> Result<(), String> {
    use scryer_application::application_upgrade::portable_replacement_rollback_operations;

    let mut errors = Vec::new();
    for (from, to) in portable_replacement_rollback_operations(completed, backup_only) {
        if let Err(error) = std::fs::rename(&from, &to) {
            errors.push(format!("{} -> {}: {error}", from.display(), to.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("rollback failed: {}", errors.join("; ")))
    }
}

#[cfg(windows)]
fn run_msi_installer(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use core::mem::size_of;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHELLEXECUTEINFOW_0, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let msi_path = plan
        .msi_path
        .as_ref()
        .ok_or_else(|| "MSI helper plan has no installer path".to_string())?;
    // Shipped MSIs unregister the per-user tray Run value on every removal,
    // including the removal half of a major upgrade, so the preference has to be
    // observed before the installer runs and restored after it.
    let startup_was_registered = crate::windows_startup::startup_enabled().unwrap_or(false);
    let verb = wide("runas");
    let program = wide("msiexec.exe");
    let parameters = wide(&format!(
        "/i \"{}\" /passive /norestart",
        msi_path.display()
    ));
    // SAFETY: Zero is the documented initializer for this Win32 structure; all
    // pointer fields below refer to NUL-terminated buffers that outlive the call.
    let mut execute: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    execute.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    execute.fMask = SEE_MASK_NOCLOSEPROCESS;
    execute.lpVerb = verb.as_ptr();
    execute.lpFile = program.as_ptr();
    execute.lpParameters = parameters.as_ptr();
    execute.lpDirectory = ptr::null();
    execute.nShow = SW_HIDE;
    execute.Anonymous = SHELLEXECUTEINFOW_0 {
        hIcon: ptr::null_mut(),
    };
    // SAFETY: `execute` is fully initialized for ShellExecuteExW and remains valid
    // through the call. The returned process handle is closed below.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        // SAFETY: GetLastError reads the thread-local error set by ShellExecuteExW.
        let transition = msi_launch_error_transition(unsafe { GetLastError() });
        return write_msi_transition(plan, transition);
    }
    // SAFETY: ShellExecuteExW returned a process handle because SEE_MASK_NOCLOSEPROCESS was set.
    unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    let mut exit_code = 0_u32;
    // SAFETY: The process handle is valid until CloseHandle below and exit_code is writable.
    let exit_status = unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) };
    // SAFETY: This helper owns the process handle returned by ShellExecuteExW.
    unsafe { CloseHandle(execute.hProcess) };
    if exit_status == 0 {
        return Err("failed to read MSI installer exit code".to_string());
    }
    restore_tray_startup_after_install(plan, startup_was_registered, exit_code);
    write_msi_transition(plan, msi_exit_code_transition(exit_code))
}

/// Re-register the per-user tray Run value when the installer dropped it.
///
/// Failures here are reported but never fail the upgrade: the application is
/// installed and running, only the "start at login" preference is at stake.
#[cfg(windows)]
fn restore_tray_startup_after_install(
    plan: &ApplicationUpgradeHelperPlan,
    startup_was_registered: bool,
    exit_code: u32,
) {
    let install_succeeded = msi_install_succeeded(exit_code);
    let still_registered = if install_succeeded && startup_was_registered {
        crate::windows_startup::startup_enabled().unwrap_or(true)
    } else {
        true
    };
    if !should_restore_tray_startup(startup_was_registered, still_registered, install_succeeded) {
        return;
    }
    let tray_path = plan.install_dir.join("scryer-tray.exe");
    if let Err(error) = crate::windows_startup::register_startup(&tray_path) {
        eprintln!("failed to restore the Scryer tray startup entry after upgrading: {error}");
    }
}

#[cfg(windows)]
fn write_msi_transition(
    plan: &ApplicationUpgradeHelperPlan,
    transition: scryer_application::application_upgrade::MsiHelperJournalTransition,
) -> Result<(), String> {
    use scryer_application::application_upgrade::{
        application_upgrade_helper_update_journal, phases,
    };

    let (phase, error) = match transition {
        scryer_application::application_upgrade::MsiHelperJournalTransition::Restarting => {
            (phases::RESTARTING, None)
        }
        scryer_application::application_upgrade::MsiHelperJournalTransition::RebootRequired => {
            (phases::REBOOT_REQUIRED, None)
        }
        scryer_application::application_upgrade::MsiHelperJournalTransition::HelperError(error) => {
            (phases::RESTARTING, Some(error))
        }
    };
    application_upgrade_helper_update_journal(&plan.journal_path, phase, error)
        .map_err(|error| format!("failed to update application upgrade journal: {error}"))
}

#[cfg(windows)]
fn write_helper_error(plan: &ApplicationUpgradeHelperPlan, error: String) {
    use scryer_application::application_upgrade::{
        application_upgrade_helper_update_journal, phases,
    };

    if let Err(write_error) = application_upgrade_helper_update_journal(
        &plan.journal_path,
        phases::RESTARTING,
        Some(error),
    ) {
        eprintln!("failed to record application upgrade helper error: {write_error}");
    }
}

#[cfg(windows)]
fn relaunch_owner(plan: &ApplicationUpgradeHelperPlan) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    std::process::Command::new(&plan.relaunch.program)
        .args(&plan.relaunch.args)
        .current_dir(&plan.relaunch.cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to relaunch application owner: {error}"))
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_installer_launch_errors_are_not_reported_as_installer_exit_codes() {
        assert_eq!(
            msi_launch_error_transition(1223),
            MsiHelperJournalTransition::HelperError("elevation was declined".to_string())
        );
        assert_eq!(
            msi_launch_error_transition(2),
            MsiHelperJournalTransition::HelperError(
                "failed to launch elevated installer (win32 error 2)".to_string()
            )
        );
    }
}
