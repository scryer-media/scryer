import type { JobRun } from "./jobs";

export type ApplicationInstallationKind =
  | "PORTABLE"
  | "DIRECT_MSI"
  | "DOCKER"
  | "HOMEBREW"
  | "WINGET"
  | "WINDOWS_SUPERVISED"
  | "DISABLED"
  | "UNSUPPORTED";

export type ApplicationUpgradeManagementOwner = "IN_APP" | "OPERATOR";

export type ApplicationUpgradeEligibilityReason =
  | "eligible"
  | "managed_by_docker"
  | "managed_by_homebrew"
  | "managed_by_winget"
  | "windows_supervised"
  | "disabled_by_operator"
  | "unsupported_layout"
  | "install_dir_not_writable";

export type ApplicationUpgradeStatus = {
  currentVersion: string;
  updateVersion: string | null;
  updateTag: string | null;
  updateAvailable: boolean;
  installationKind: ApplicationInstallationKind;
  managementOwner: ApplicationUpgradeManagementOwner;
  eligible: boolean;
  eligibilityReason: ApplicationUpgradeEligibilityReason | string;
  activeRun: JobRun | null;
  latestRun: JobRun | null;
};

export type ApplicationUpgradePhase =
  | "checking"
  | "downloading"
  | "verifying"
  | "staging"
  | "applying"
  | "awaiting_elevation"
  | "restarting"
  | "reboot_required"
  | "unknown";

export type ApplicationUpgradeProgress = {
  status: string | null;
  phase: ApplicationUpgradePhase | null;
  downloadedBytes: number | null;
  totalBytes: number | null;
  targetVersion: string | null;
  targetTag: string | null;
  error: string | null;
};
