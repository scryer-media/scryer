mod engine;
mod helper_plan;
mod installation;
pub mod manifest;
mod restart;

pub use engine::{
    ApplicationUpgradeJobAccepted, ApplicationUpgradeJobRequest, ApplicationUpgradeJournal,
    ApplicationUpgradeProgress, application_upgrade_helper_update_journal, phases,
};
pub use helper_plan::{
    APPLICATION_UPGRADE_HELPER_PLAN_SCHEMA, APPLICATION_UPGRADE_HELPER_WAIT_BUDGET,
    ApplicationUpgradeHelperMode, ApplicationUpgradeHelperOwner, ApplicationUpgradeHelperPlan,
    ApplicationUpgradeHelperRelaunch, ApplicationUpgradeHelperReplacement,
    MsiHelperJournalTransition, PortableReplacementOperations, WRITE_PROBE_PERMISSION_DENIED,
    WriteProbeOutcome, classify_write_probe_error, helper_wait_remaining,
    helper_write_probe_required, msi_exit_code_transition, msi_install_succeeded,
    open_process_failure_means_exited, path_is_within, portable_replacement_operations,
    portable_replacement_rollback_operations, reboot_required_completion_allowed,
    should_restore_tray_startup,
};
pub use installation::{
    EligibilityReason, InstallationAssessment, InstallationEvidence, InstallationKind,
    InstallationOs, ManagementOwner, classify_installation,
};
pub use restart::ApplicationUpgradeRestartHandle;
