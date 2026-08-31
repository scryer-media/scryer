use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA: &str = "scryer.upgrade.helper-plan.v1";

/// Total budget the helper may spend waiting for the previous process tree to
/// exit and for the installed executables to be released.
pub const APPLICATION_UPGRADE_HELPER_WAIT_BUDGET: Duration = Duration::from_secs(60);

/// Durable instructions consumed by the temporary Windows upgrade helper.
///
/// This deliberately lives in the application crate: the executable only needs to
/// perform the small Windows process and file-system operations described here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperPlan {
    pub schema: String,
    pub mode: ApplicationUpgradeHelperMode,
    pub owner: ApplicationUpgradeHelperOwner,
    pub journal_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msi_path: Option<PathBuf>,
    pub install_dir: PathBuf,
    /// Process ids the helper must observe exiting before it touches the
    /// installation. An empty list is legal: older plans predate this field and
    /// fall back to the file-release probe alone.
    #[serde(default)]
    pub wait_process_ids: Vec<u32>,
    #[serde(default)]
    pub replace: Vec<ApplicationUpgradeHelperReplacement>,
    pub backup_suffix: String,
    pub relaunch: ApplicationUpgradeHelperRelaunch,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_shutdown_program: Option<PathBuf>,
    pub expected_version: String,
    pub expected_tag: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationUpgradeHelperMode {
    /// Swap the executables staged from a portable `.tar.gz` upgrade artifact.
    Portable,
    Msi,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationUpgradeHelperOwner {
    Direct,
    Tray,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperReplacement {
    pub from_staged: PathBuf,
    pub to_install: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationUpgradeHelperRelaunch {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl ApplicationUpgradeHelperPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA {
            return Err(format!(
                "unsupported upgrade helper plan schema '{}'",
                self.schema
            ));
        }
        if self.journal_path.as_os_str().is_empty()
            || self.install_dir.as_os_str().is_empty()
            || self.relaunch.program.as_os_str().is_empty()
            || self.relaunch.cwd.as_os_str().is_empty()
            || self.expected_version.trim().is_empty()
            || self.expected_tag.trim().is_empty()
        {
            return Err(
                "upgrade helper plan contains an empty required path or version".to_string(),
            );
        }
        if !self.backup_suffix.starts_with(".pre-upgrade-")
            || self.backup_suffix.contains('/')
            || self.backup_suffix.contains('\\')
        {
            return Err("upgrade helper plan has an unsafe backup suffix".to_string());
        }
        if self.owner == ApplicationUpgradeHelperOwner::Tray && self.tray_shutdown_program.is_none()
        {
            return Err(
                "tray-owned upgrade helper plan requires a tray shutdown program".to_string(),
            );
        }

        match self.mode {
            ApplicationUpgradeHelperMode::Portable => {
                let staged_dir = self.staged_dir.as_deref().ok_or_else(|| {
                    "portable upgrade helper plan requires staged_dir".to_string()
                })?;
                if self.msi_path.is_some() || self.replace.len() != 2 {
                    return Err(
                        "portable upgrade helper plan requires exactly two replacements and no MSI"
                            .to_string(),
                    );
                }
                let mut names = self
                    .replace
                    .iter()
                    .map(|replacement| {
                        if !replacement.from_staged.starts_with(staged_dir)
                            || replacement.to_install.parent() != Some(self.install_dir.as_path())
                        {
                            return Err(
                                "portable upgrade helper plan has a replacement outside its declared directories"
                                    .to_string(),
                            );
                        }
                        replacement
                            .to_install
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                "portable upgrade helper plan has an invalid replacement name"
                                    .to_string()
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                names.sort_unstable();
                if names != ["scryer-tray.exe", "scryer.exe"] {
                    return Err(
                        "portable upgrade helper plan must replace scryer.exe and scryer-tray.exe"
                            .to_string(),
                    );
                }
            }
            ApplicationUpgradeHelperMode::Msi => {
                if self.staged_dir.is_some() || !self.replace.is_empty() {
                    return Err(
                        "MSI upgrade helper plan must not contain staged replacements".to_string(),
                    );
                }
                if self.msi_path.is_none() {
                    return Err("MSI upgrade helper plan requires msi_path".to_string());
                }
            }
        }
        Ok(())
    }
}

/// The two atomic renames used for a single portable executable replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableReplacementOperations {
    pub retain_backup_from: PathBuf,
    pub retain_backup_to: PathBuf,
    pub install_from: PathBuf,
    pub install_to: PathBuf,
}

pub fn portable_replacement_operations(
    replacement: &ApplicationUpgradeHelperReplacement,
    backup_suffix: &str,
) -> PortableReplacementOperations {
    PortableReplacementOperations {
        retain_backup_from: replacement.to_install.clone(),
        retain_backup_to: PathBuf::from(format!(
            "{}{}",
            replacement.to_install.display(),
            backup_suffix
        )),
        install_from: replacement.from_staged.clone(),
        install_to: replacement.to_install.clone(),
    }
}

/// Rename operations that undo replacements in reverse order.
///
/// `completed` contains replacements for which the staged member was installed.
/// `backup_only` is the replacement whose original executable was moved aside but
/// whose staged member could not be installed.
pub fn portable_replacement_rollback_operations(
    completed: &[PortableReplacementOperations],
    backup_only: Option<&PortableReplacementOperations>,
) -> Vec<(PathBuf, PathBuf)> {
    let mut rollback = Vec::new();
    if let Some(backup_only) = backup_only {
        rollback.push((
            backup_only.retain_backup_to.clone(),
            backup_only.retain_backup_from.clone(),
        ));
    }
    for operation in completed.iter().rev() {
        rollback.push((operation.install_to.clone(), operation.install_from.clone()));
        rollback.push((
            operation.retain_backup_to.clone(),
            operation.retain_backup_from.clone(),
        ));
    }
    rollback
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MsiHelperJournalTransition {
    Restarting,
    RebootRequired,
    HelperError(String),
}

/// Translate the Windows installer/UAC status into the durable journal state.
pub fn msi_exit_code_transition(code: u32) -> MsiHelperJournalTransition {
    match code {
        0 => MsiHelperJournalTransition::Restarting,
        3010 => MsiHelperJournalTransition::RebootRequired,
        1223 => MsiHelperJournalTransition::HelperError("elevation was declined".to_string()),
        code => MsiHelperJournalTransition::HelperError(format!("installer exit code {code}")),
    }
}

/// Whether an installer exit code represents a successful installation.
pub fn msi_install_succeeded(code: u32) -> bool {
    matches!(code, 0 | 3010)
}

/// Whether the helper must re-register the per-user tray Run value.
///
/// Shipped MSIs still carry an unconditional unregister custom action, so a
/// major upgrade performed by them silently drops a user's "start at login"
/// preference. The helper restores it only when it was set before the install,
/// the install succeeded, and the value is gone afterwards.
pub fn should_restore_tray_startup(
    was_registered: bool,
    still_registered: bool,
    install_succeeded: bool,
) -> bool {
    was_registered && !still_registered && install_succeeded
}

/// Whether the helper must probe installed executables for write access.
///
/// `msiexec` owns in-use file semantics for MSI installs, so probing there only
/// produces false negatives; the process wait is the gate instead.
pub fn helper_write_probe_required(mode: ApplicationUpgradeHelperMode) -> bool {
    match mode {
        ApplicationUpgradeHelperMode::Portable => true,
        ApplicationUpgradeHelperMode::Msi => false,
    }
}

/// Verdict for a single failed write probe against an installed executable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteProbeOutcome {
    /// The file is being shared by a process that is still exiting; retry.
    Retry,
    /// The helper will never be allowed to write this file; fail immediately.
    Fatal(&'static str),
}

/// Message reported when the helper lacks write permission on the installation.
pub const WRITE_PROBE_PERMISSION_DENIED: &str = "no write permission for installed executables";

/// Classify a failed write probe so a permanent denial is not retried for a minute.
///
/// Windows surfaces a sharing violation (os error 32) as an error kind that is
/// not `PermissionDenied`, while `ERROR_ACCESS_DENIED` (os error 5) maps to
/// `PermissionDenied`. Only the former is worth retrying.
pub fn classify_write_probe_error(
    kind: std::io::ErrorKind,
    raw_os_error: Option<i32>,
) -> WriteProbeOutcome {
    if kind == std::io::ErrorKind::PermissionDenied || raw_os_error == Some(5) {
        return WriteProbeOutcome::Fatal(WRITE_PROBE_PERMISSION_DENIED);
    }
    WriteProbeOutcome::Retry
}

/// Whether a failure to open a process handle means that process already exited.
///
/// `ERROR_INVALID_PARAMETER` is returned for a process id that no longer exists;
/// `ERROR_ACCESS_DENIED` is returned when the id has been recycled into a
/// process this helper may not touch. Neither can be waited on, and in both
/// cases the process the plan named is gone.
pub fn open_process_failure_means_exited(win32_error: u32) -> bool {
    matches!(win32_error, 5 | 87)
}

/// Remaining slice of the helper wait budget, or `None` once it is exhausted.
pub fn helper_wait_remaining(elapsed: Duration, budget: Duration) -> Option<Duration> {
    budget.checked_sub(elapsed).filter(|left| !left.is_zero())
}

/// Whether a reboot-required journal can be completed on this boot.
pub fn reboot_required_completion_allowed(
    written_at: Option<DateTime<Utc>>,
    boot_time: Option<DateTime<Utc>>,
    expected_version_booted: bool,
    expected_executable_booted: bool,
) -> bool {
    expected_version_booted
        && expected_executable_booted
        && written_at
            .zip(boot_time)
            .is_some_and(|(written_at, boot_time)| boot_time > written_at)
}

pub fn path_is_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn portable_plan() -> ApplicationUpgradeHelperPlan {
        ApplicationUpgradeHelperPlan {
            schema: APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA.to_string(),
            mode: ApplicationUpgradeHelperMode::Portable,
            owner: ApplicationUpgradeHelperOwner::Tray,
            journal_path: PathBuf::from("C:/data/application-upgrade/journal.json"),
            staged_dir: Some(PathBuf::from(
                "C:/data/application-upgrade/staging/extracted",
            )),
            msi_path: None,
            install_dir: PathBuf::from("C:/Program Files/Scryer"),
            wait_process_ids: vec![4242],
            replace: vec![
                ApplicationUpgradeHelperReplacement {
                    from_staged: PathBuf::from(
                        "C:/data/application-upgrade/staging/extracted/scryer.exe",
                    ),
                    to_install: PathBuf::from("C:/Program Files/Scryer/scryer.exe"),
                },
                ApplicationUpgradeHelperReplacement {
                    from_staged: PathBuf::from(
                        "C:/data/application-upgrade/staging/extracted/scryer-tray.exe",
                    ),
                    to_install: PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe"),
                },
            ],
            backup_suffix: ".pre-upgrade-1.2.3".to_string(),
            relaunch: ApplicationUpgradeHelperRelaunch {
                program: PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe"),
                args: vec!["--login-start".to_string()],
                cwd: PathBuf::from("C:/Program Files/Scryer"),
            },
            tray_shutdown_program: Some(PathBuf::from("C:/Program Files/Scryer/scryer-tray.exe")),
            expected_version: "1.2.4".to_string(),
            expected_tag: "v1.2.4".to_string(),
        }
    }

    #[test]
    fn helper_plan_round_trips_and_validates() {
        let plan = portable_plan();
        let decoded: ApplicationUpgradeHelperPlan =
            serde_json::from_slice(&serde_json::to_vec(&plan).expect("encode plan"))
                .expect("decode plan");
        assert_eq!(decoded, plan);
        decoded.validate().expect("valid plan");
    }

    #[test]
    fn helper_plan_rejects_invalid_mode_specific_fields() {
        let mut plan = portable_plan();
        plan.replace.pop();
        assert!(plan.validate().is_err());

        let mut plan = portable_plan();
        plan.backup_suffix = "../escape".to_string();
        assert!(plan.validate().is_err());
    }

    #[test]
    fn msi_helper_plan_requires_only_the_installer_path() {
        let mut plan = portable_plan();
        plan.mode = ApplicationUpgradeHelperMode::Msi;
        plan.owner = ApplicationUpgradeHelperOwner::Direct;
        plan.staged_dir = None;
        plan.msi_path = Some(PathBuf::from(
            "C:/data/application-upgrade/staging/artifact",
        ));
        plan.replace.clear();
        plan.tray_shutdown_program = None;
        plan.relaunch = ApplicationUpgradeHelperRelaunch {
            program: PathBuf::from("C:/Program Files/Scryer/scryer.exe"),
            args: Vec::new(),
            cwd: PathBuf::from("C:/Program Files/Scryer"),
        };
        plan.validate().expect("valid MSI plan");
        let value = serde_json::to_value(plan).expect("encode MSI plan");
        assert!(value.get("staged_dir").is_none());
        assert!(value.get("msi_path").is_some());
    }

    #[test]
    fn helper_plan_without_wait_process_ids_still_parses_and_validates() {
        let plan = portable_plan();
        let mut value = serde_json::to_value(&plan).expect("encode plan");
        value
            .as_object_mut()
            .expect("plan object")
            .remove("wait_process_ids");
        let decoded: ApplicationUpgradeHelperPlan =
            serde_json::from_value(value).expect("decode legacy plan");
        assert!(decoded.wait_process_ids.is_empty());
        decoded.validate().expect("legacy plan stays valid");
    }

    #[test]
    fn write_probe_is_required_only_for_portable_replacements() {
        assert!(helper_write_probe_required(
            ApplicationUpgradeHelperMode::Portable
        ));
        assert!(!helper_write_probe_required(
            ApplicationUpgradeHelperMode::Msi
        ));
    }

    #[test]
    fn access_denied_write_probes_are_fatal_while_sharing_violations_retry() {
        assert_eq!(
            classify_write_probe_error(std::io::ErrorKind::PermissionDenied, Some(5)),
            WriteProbeOutcome::Fatal(WRITE_PROBE_PERMISSION_DENIED)
        );
        // Windows reports a sharing violation (os error 32) through a kind that
        // is not PermissionDenied; that is the only case worth retrying.
        assert_eq!(
            classify_write_probe_error(std::io::ErrorKind::Other, Some(32)),
            WriteProbeOutcome::Retry
        );
        assert_eq!(
            classify_write_probe_error(std::io::ErrorKind::NotFound, Some(2)),
            WriteProbeOutcome::Retry
        );
        // A permission denial without a raw os error is still fatal.
        assert_eq!(
            classify_write_probe_error(std::io::ErrorKind::PermissionDenied, None),
            WriteProbeOutcome::Fatal(WRITE_PROBE_PERMISSION_DENIED)
        );
    }

    #[test]
    fn unopenable_process_ids_count_as_exited_only_for_gone_processes() {
        assert!(open_process_failure_means_exited(87));
        assert!(open_process_failure_means_exited(5));
        assert!(!open_process_failure_means_exited(8));
    }

    #[test]
    fn helper_wait_budget_runs_out_exactly_once() {
        assert_eq!(
            helper_wait_remaining(
                Duration::from_secs(10),
                APPLICATION_UPGRADE_HELPER_WAIT_BUDGET
            ),
            Some(Duration::from_secs(50))
        );
        assert_eq!(
            helper_wait_remaining(
                APPLICATION_UPGRADE_HELPER_WAIT_BUDGET,
                APPLICATION_UPGRADE_HELPER_WAIT_BUDGET
            ),
            None
        );
        assert_eq!(
            helper_wait_remaining(
                Duration::from_secs(61),
                APPLICATION_UPGRADE_HELPER_WAIT_BUDGET
            ),
            None
        );
    }

    #[test]
    fn tray_startup_is_restored_only_after_a_successful_install_erased_it() {
        assert!(msi_install_succeeded(0));
        assert!(msi_install_succeeded(3010));
        assert!(!msi_install_succeeded(1603));

        assert!(should_restore_tray_startup(true, false, true));
        assert!(!should_restore_tray_startup(false, false, true));
        assert!(!should_restore_tray_startup(true, true, true));
        assert!(!should_restore_tray_startup(true, false, false));
    }

    #[test]
    fn msi_exit_codes_map_to_durable_transitions() {
        assert_eq!(
            msi_exit_code_transition(0),
            MsiHelperJournalTransition::Restarting
        );
        assert_eq!(
            msi_exit_code_transition(3010),
            MsiHelperJournalTransition::RebootRequired
        );
        assert_eq!(
            msi_exit_code_transition(1223),
            MsiHelperJournalTransition::HelperError("elevation was declined".to_string())
        );
        assert_eq!(
            msi_exit_code_transition(1603),
            MsiHelperJournalTransition::HelperError("installer exit code 1603".to_string())
        );
    }

    #[test]
    fn replacement_rollback_reverses_completed_members_before_restoring_backups() {
        let plan = portable_plan();
        let operations = plan
            .replace
            .iter()
            .map(|replacement| portable_replacement_operations(replacement, &plan.backup_suffix))
            .collect::<Vec<_>>();
        assert_eq!(
            portable_replacement_rollback_operations(&operations, None),
            vec![
                (
                    operations[1].install_to.clone(),
                    operations[1].install_from.clone()
                ),
                (
                    operations[1].retain_backup_to.clone(),
                    operations[1].retain_backup_from.clone()
                ),
                (
                    operations[0].install_to.clone(),
                    operations[0].install_from.clone()
                ),
                (
                    operations[0].retain_backup_to.clone(),
                    operations[0].retain_backup_from.clone()
                ),
            ]
        );
        assert_eq!(
            portable_replacement_rollback_operations(&operations[..1], Some(&operations[1]))[0],
            (
                operations[1].retain_backup_to.clone(),
                operations[1].retain_backup_from.clone()
            )
        );
    }

    #[test]
    fn reboot_completion_requires_a_boot_after_the_journal_write() {
        let written_at = Utc.with_ymd_and_hms(2026, 8, 24, 12, 0, 0).single();
        let rebooted = Utc.with_ymd_and_hms(2026, 8, 24, 12, 1, 0).single();
        let not_rebooted = Utc.with_ymd_and_hms(2026, 8, 24, 11, 59, 0).single();
        assert!(reboot_required_completion_allowed(
            written_at, rebooted, true, true
        ));
        assert!(!reboot_required_completion_allowed(
            written_at,
            not_rebooted,
            true,
            true
        ));
        assert!(!reboot_required_completion_allowed(
            None, rebooted, true, true
        ));
        assert!(!reboot_required_completion_allowed(
            written_at, rebooted, false, true
        ));
    }
}
