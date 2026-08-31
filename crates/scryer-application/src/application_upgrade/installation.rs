use std::path::PathBuf;

/// Operating-system family observed while collecting installation evidence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InstallationOs {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    Macos,
    /// Linux.
    Linux,
    /// Any other operating system.
    #[default]
    Other,
}

/// Plain startup evidence used to assess whether in-app upgrades are available.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InstallationEvidence {
    /// Raw `SCRYER_DISABLE_SELF_UPGRADE` environment marker, when set.
    pub disable_self_upgrade: Option<String>,
    /// Raw `SCRYER_PACKAGE` environment marker, when set.
    pub package: Option<String>,
    /// Executable path, when it can be resolved.
    pub executable_path: Option<PathBuf>,
    /// Whether the executable directory accepted a create-and-delete probe.
    pub executable_dir_writable: bool,
    /// Whether the Docker sentinel file is present.
    pub docker_env_present: bool,
    /// Operating-system family.
    pub os: InstallationOs,
    /// Whether the Windows process runs in session zero.
    pub windows_session_zero: bool,
    /// Whether Task Scheduler directly launched the Windows process.
    pub windows_task_scheduler_parent: bool,
    /// `DistributionOwner` from the Scryer machine registry key, when present.
    pub windows_distribution_owner: Option<String>,
    /// Whether the executable path is contained by Program Files.
    pub windows_executable_under_program_files: bool,
    /// Whether the legacy Scryer machine registry key exists.
    pub windows_legacy_msi_registry_key_exists: bool,
    /// Whether the Windows tray launched and supervises this process.
    pub tray_supervised: bool,
}

/// Installation layout classification used by the in-app upgrade surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallationKind {
    Portable,
    DirectMsi,
    Docker,
    Homebrew,
    Winget,
    WindowsSupervised,
    Disabled,
    Unsupported,
}

/// Party responsible for managing application upgrades.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagementOwner {
    InApp,
    Operator,
}

/// Stable code explaining upgrade eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EligibilityReason {
    DisabledByOperator,
    ManagedByDocker,
    ManagedByHomebrew,
    WindowsSupervised,
    ManagedByWinget,
    Eligible,
    UnsupportedLayout,
    InstallDirNotWritable,
}

impl EligibilityReason {
    /// Return the stable snake_case API code for this reason.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DisabledByOperator => "disabled_by_operator",
            Self::ManagedByDocker => "managed_by_docker",
            Self::ManagedByHomebrew => "managed_by_homebrew",
            Self::WindowsSupervised => "windows_supervised",
            Self::ManagedByWinget => "managed_by_winget",
            Self::Eligible => "eligible",
            Self::UnsupportedLayout => "unsupported_layout",
            Self::InstallDirNotWritable => "install_dir_not_writable",
        }
    }
}

/// Immutable installation assessment captured during startup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstallationAssessment {
    pub kind: InstallationKind,
    pub owner: ManagementOwner,
    pub eligible: bool,
    pub reason: EligibilityReason,
    /// Whether the Windows tray is responsible for shutting down and relaunching this process.
    pub tray_supervised: bool,
}

impl Default for InstallationAssessment {
    fn default() -> Self {
        Self {
            kind: InstallationKind::Unsupported,
            owner: ManagementOwner::Operator,
            eligible: false,
            reason: EligibilityReason::UnsupportedLayout,
            tray_supervised: false,
        }
    }
}

/// Classify an installation from startup evidence using upgrade-safety precedence.
pub fn classify_installation(evidence: &InstallationEvidence) -> InstallationAssessment {
    if env_marker_enabled(evidence.disable_self_upgrade.as_deref()) {
        return operator_assessment(
            InstallationKind::Disabled,
            EligibilityReason::DisabledByOperator,
        );
    }

    if package_is(evidence.package.as_deref(), "docker") || evidence.docker_env_present {
        return operator_assessment(InstallationKind::Docker, EligibilityReason::ManagedByDocker);
    }

    if package_is(evidence.package.as_deref(), "homebrew") || is_homebrew_layout(evidence) {
        return operator_assessment(
            InstallationKind::Homebrew,
            EligibilityReason::ManagedByHomebrew,
        );
    }

    if evidence.os == InstallationOs::Windows
        && (evidence.windows_session_zero || evidence.windows_task_scheduler_parent)
    {
        return operator_assessment(
            InstallationKind::WindowsSupervised,
            EligibilityReason::WindowsSupervised,
        );
    }

    if evidence.os == InstallationOs::Windows
        && package_is(evidence.windows_distribution_owner.as_deref(), "winget")
    {
        return operator_assessment(InstallationKind::Winget, EligibilityReason::ManagedByWinget);
    }

    if evidence.os == InstallationOs::Windows
        && (package_is(evidence.windows_distribution_owner.as_deref(), "msi")
            || (evidence.windows_distribution_owner.is_none()
                && evidence.windows_legacy_msi_registry_key_exists
                && evidence.windows_executable_under_program_files))
    {
        return in_app_assessment(InstallationKind::DirectMsi, evidence.tray_supervised);
    }

    if evidence.os == InstallationOs::Windows
        && evidence
            .windows_distribution_owner
            .as_deref()
            .is_some_and(|owner| {
                !owner.trim().is_empty()
                    && !package_is(Some(owner), "winget")
                    && !package_is(Some(owner), "msi")
            })
    {
        return operator_assessment(
            InstallationKind::Unsupported,
            EligibilityReason::UnsupportedLayout,
        );
    }

    if evidence.executable_dir_writable {
        return in_app_assessment(InstallationKind::Portable, evidence.tray_supervised);
    }

    let reason = if evidence.executable_path.is_some() {
        EligibilityReason::InstallDirNotWritable
    } else {
        EligibilityReason::UnsupportedLayout
    };
    operator_assessment(InstallationKind::Unsupported, reason)
}

fn env_marker_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

fn package_is(value: Option<&str>, expected: &str) -> bool {
    value.is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

fn is_homebrew_layout(evidence: &InstallationEvidence) -> bool {
    if evidence.os == InstallationOs::Windows {
        return false;
    }

    evidence.executable_path.as_ref().is_some_and(|path| {
        let path = path.to_string_lossy();
        // `/usr/local/opt` is the Intel-macOS keg link root and
        // `/home/linuxbrew/.linuxbrew` is the Linuxbrew prefix; both reach the
        // Cellar through symlinks that a canonicalized path may not show.
        path.contains("/Cellar/")
            || path.starts_with("/opt/homebrew/")
            || path.starts_with("/usr/local/Cellar/")
            || path.starts_with("/usr/local/opt/")
            || path.starts_with("/home/linuxbrew/.linuxbrew/")
    })
}

fn in_app_assessment(kind: InstallationKind, tray_supervised: bool) -> InstallationAssessment {
    InstallationAssessment {
        kind,
        owner: ManagementOwner::InApp,
        eligible: true,
        reason: EligibilityReason::Eligible,
        tray_supervised,
    }
}

fn operator_assessment(
    kind: InstallationKind,
    reason: EligibilityReason,
) -> InstallationAssessment {
    InstallationAssessment {
        kind,
        owner: ManagementOwner::Operator,
        eligible: false,
        reason,
        tray_supervised: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> InstallationEvidence {
        InstallationEvidence {
            executable_path: Some(PathBuf::from("/opt/scryer/scryer")),
            executable_dir_writable: true,
            os: InstallationOs::Linux,
            ..Default::default()
        }
    }

    fn assert_assessment(
        evidence: InstallationEvidence,
        kind: InstallationKind,
        owner: ManagementOwner,
        eligible: bool,
        reason: EligibilityReason,
    ) {
        assert_eq!(
            classify_installation(&evidence),
            InstallationAssessment {
                kind,
                owner,
                eligible,
                reason,
                tray_supervised: evidence.tray_supervised && eligible,
            }
        );
    }

    #[test]
    fn classifies_every_installation_kind() {
        let portable = evidence();
        assert_assessment(
            portable,
            InstallationKind::Portable,
            ManagementOwner::InApp,
            true,
            EligibilityReason::Eligible,
        );

        let mut direct_msi = evidence();
        direct_msi.os = InstallationOs::Windows;
        direct_msi.windows_distribution_owner = Some("msi".to_string());
        assert_assessment(
            direct_msi,
            InstallationKind::DirectMsi,
            ManagementOwner::InApp,
            true,
            EligibilityReason::Eligible,
        );

        let mut docker = evidence();
        docker.package = Some("docker".to_string());
        assert_assessment(
            docker,
            InstallationKind::Docker,
            ManagementOwner::Operator,
            false,
            EligibilityReason::ManagedByDocker,
        );

        let mut homebrew = evidence();
        homebrew.package = Some("homebrew".to_string());
        assert_assessment(
            homebrew,
            InstallationKind::Homebrew,
            ManagementOwner::Operator,
            false,
            EligibilityReason::ManagedByHomebrew,
        );

        let mut winget = evidence();
        winget.os = InstallationOs::Windows;
        winget.windows_distribution_owner = Some("winget".to_string());
        assert_assessment(
            winget,
            InstallationKind::Winget,
            ManagementOwner::Operator,
            false,
            EligibilityReason::ManagedByWinget,
        );

        let mut supervised = evidence();
        supervised.os = InstallationOs::Windows;
        supervised.windows_session_zero = true;
        assert_assessment(
            supervised,
            InstallationKind::WindowsSupervised,
            ManagementOwner::Operator,
            false,
            EligibilityReason::WindowsSupervised,
        );

        let mut task_scheduler = evidence();
        task_scheduler.os = InstallationOs::Windows;
        task_scheduler.windows_task_scheduler_parent = true;
        assert_assessment(
            task_scheduler,
            InstallationKind::WindowsSupervised,
            ManagementOwner::Operator,
            false,
            EligibilityReason::WindowsSupervised,
        );

        let mut disabled = evidence();
        disabled.disable_self_upgrade = Some("true".to_string());
        assert_assessment(
            disabled,
            InstallationKind::Disabled,
            ManagementOwner::Operator,
            false,
            EligibilityReason::DisabledByOperator,
        );

        let mut unsupported = evidence();
        unsupported.executable_dir_writable = false;
        unsupported.executable_path = None;
        assert_assessment(
            unsupported,
            InstallationKind::Unsupported,
            ManagementOwner::Operator,
            false,
            EligibilityReason::UnsupportedLayout,
        );
    }

    #[test]
    fn classifies_whitespace_padded_windows_distribution_owner() {
        let mut direct_msi = evidence();
        direct_msi.os = InstallationOs::Windows;
        direct_msi.windows_distribution_owner = Some("  msi  ".to_string());
        assert_assessment(
            direct_msi,
            InstallationKind::DirectMsi,
            ManagementOwner::InApp,
            true,
            EligibilityReason::Eligible,
        );
    }

    #[test]
    fn carries_tray_supervision_into_an_eligible_assessment() {
        let mut evidence = evidence();
        evidence.os = InstallationOs::Windows;
        evidence.tray_supervised = true;
        let assessment = classify_installation(&evidence);
        assert_eq!(assessment.kind, InstallationKind::Portable);
        assert!(assessment.eligible);
        assert!(assessment.tray_supervised);
    }

    #[test]
    fn reports_install_directory_not_writable_when_that_is_the_only_disqualifier() {
        let mut evidence = evidence();
        evidence.executable_dir_writable = false;
        assert_assessment(
            evidence,
            InstallationKind::Unsupported,
            ManagementOwner::Operator,
            false,
            EligibilityReason::InstallDirNotWritable,
        );
    }

    #[test]
    fn managed_evidence_precedes_writable_portable_layout() {
        let mut disabled = evidence();
        disabled.disable_self_upgrade = Some("1".to_string());
        disabled.package = Some("docker".to_string());
        assert_eq!(
            classify_installation(&disabled).kind,
            InstallationKind::Disabled
        );

        let mut docker = evidence();
        docker.docker_env_present = true;
        assert_eq!(
            classify_installation(&docker).kind,
            InstallationKind::Docker
        );

        let mut docker_before_homebrew = evidence();
        docker_before_homebrew.package = Some("homebrew".to_string());
        docker_before_homebrew.docker_env_present = true;
        assert_eq!(
            classify_installation(&docker_before_homebrew).kind,
            InstallationKind::Docker
        );

        let mut homebrew = evidence();
        homebrew.executable_path = Some(PathBuf::from("/usr/local/Cellar/scryer/bin/scryer"));
        assert_eq!(
            classify_installation(&homebrew).kind,
            InstallationKind::Homebrew
        );

        let mut homebrew_before_session_zero = evidence();
        homebrew_before_session_zero.os = InstallationOs::Windows;
        homebrew_before_session_zero.package = Some("homebrew".to_string());
        homebrew_before_session_zero.windows_session_zero = true;
        assert_eq!(
            classify_installation(&homebrew_before_session_zero).kind,
            InstallationKind::Homebrew
        );
    }

    #[test]
    fn detects_every_homebrew_prefix_layout() {
        for path in [
            "/opt/homebrew/Cellar/scryer/0.18.22/bin/scryer",
            "/opt/homebrew/bin/scryer",
            "/usr/local/Cellar/scryer/0.18.22/bin/scryer",
            "/usr/local/opt/scryer/bin/scryer",
            "/home/linuxbrew/.linuxbrew/bin/scryer",
            "/home/linuxbrew/.linuxbrew/Cellar/scryer/0.18.22/bin/scryer",
        ] {
            let mut evidence = evidence();
            evidence.executable_path = Some(PathBuf::from(path));
            assert_eq!(
                classify_installation(&evidence).kind,
                InstallationKind::Homebrew,
                "{path} must classify as Homebrew"
            );
        }

        for path in ["/opt/scryer/scryer", "/usr/local/bin/scryer"] {
            let mut evidence = evidence();
            evidence.executable_path = Some(PathBuf::from(path));
            assert_eq!(
                classify_installation(&evidence).kind,
                InstallationKind::Portable,
                "{path} must not classify as Homebrew"
            );
        }

        // Windows paths never take the Homebrew branch.
        let mut windows = evidence();
        windows.os = InstallationOs::Windows;
        windows.executable_path = Some(PathBuf::from("C:/usr/local/opt/scryer/scryer.exe"));
        assert_eq!(
            classify_installation(&windows).kind,
            InstallationKind::Portable
        );
    }

    #[test]
    fn session_zero_precedes_windows_distribution_evidence() {
        let mut evidence = evidence();
        evidence.os = InstallationOs::Windows;
        evidence.windows_session_zero = true;
        evidence.windows_distribution_owner = Some("msi".to_string());
        assert_eq!(
            classify_installation(&evidence).kind,
            InstallationKind::WindowsSupervised
        );
    }

    #[test]
    fn winget_owner_precedes_legacy_msi_evidence() {
        let mut evidence = evidence();
        evidence.os = InstallationOs::Windows;
        evidence.windows_distribution_owner = Some("winget".to_string());
        evidence.windows_legacy_msi_registry_key_exists = true;
        evidence.windows_executable_under_program_files = true;
        assert_eq!(
            classify_installation(&evidence).kind,
            InstallationKind::Winget
        );
    }

    #[test]
    fn detects_legacy_msi_only_when_key_and_program_files_evidence_are_present() {
        let mut evidence = evidence();
        evidence.os = InstallationOs::Windows;
        evidence.windows_legacy_msi_registry_key_exists = true;
        evidence.windows_executable_under_program_files = true;
        assert_eq!(
            classify_installation(&evidence).kind,
            InstallationKind::DirectMsi
        );

        let mut missing_program_files = evidence.clone();
        missing_program_files.windows_executable_under_program_files = false;
        assert_eq!(
            classify_installation(&missing_program_files).kind,
            InstallationKind::Portable
        );

        let mut explicit_owner = evidence;
        explicit_owner.windows_distribution_owner = Some("other".to_string());
        assert_eq!(
            classify_installation(&explicit_owner).kind,
            InstallationKind::Unsupported
        );
    }

    #[test]
    fn unknown_distribution_owner_is_operator_managed() {
        let mut evidence = evidence();
        evidence.os = InstallationOs::Windows;
        evidence.windows_distribution_owner = Some("future-msi-channel".to_string());
        assert_assessment(
            evidence,
            InstallationKind::Unsupported,
            ManagementOwner::Operator,
            false,
            EligibilityReason::UnsupportedLayout,
        );
    }

    #[test]
    fn tray_supervision_does_not_change_a_portable_assessment() {
        let mut evidence = evidence();
        evidence.tray_supervised = true;
        assert_eq!(
            classify_installation(&evidence).kind,
            InstallationKind::Portable
        );
    }
}
