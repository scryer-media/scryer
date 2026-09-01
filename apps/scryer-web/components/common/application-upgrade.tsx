import * as React from "react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/sonner";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { startApplicationUpgradeMutation } from "@/lib/graphql/queries";
import { useApplicationUpgradeStatus } from "@/lib/hooks/use-application-upgrade-status";
import type {
  ApplicationInstallationKind,
  ApplicationUpgradePhase,
  ApplicationUpgradeStatus,
  JobRun,
  JobRunStatus,
} from "@/lib/types";
import { formatByteCount } from "@/lib/utils/activity-utils";
import { normalizeApplicationUpgradeProgress } from "@/lib/utils/application-upgrade-progress";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { normalizeJobRun } from "@/lib/utils/job-runs";

const INSTALLATION_KIND_KEYS: Record<ApplicationInstallationKind, string> = {
  PORTABLE: "appUpgrade.installationKind.portable",
  DIRECT_MSI: "appUpgrade.installationKind.directMsi",
  DOCKER: "appUpgrade.installationKind.docker",
  HOMEBREW: "appUpgrade.installationKind.homebrew",
  WINGET: "appUpgrade.installationKind.winget",
  WINDOWS_SUPERVISED: "appUpgrade.installationKind.windowsSupervised",
  DISABLED: "appUpgrade.installationKind.disabled",
  UNSUPPORTED: "appUpgrade.installationKind.unsupported",
};

const RUN_STATUS_KEYS: Record<JobRunStatus, string> = {
  QUEUED: "appUpgrade.runStatus.queued",
  DISCOVERING: "appUpgrade.runStatus.discovering",
  RUNNING: "appUpgrade.runStatus.running",
  COMPLETED: "appUpgrade.runStatus.completed",
  WARNING: "appUpgrade.runStatus.warning",
  FAILED: "appUpgrade.runStatus.failed",
};

function phaseLabelKey(phase: ApplicationUpgradePhase | null): string {
  return `appUpgrade.phase.${phase ?? "unknown"}`;
}

function runStatusLabel(run: JobRun, t: ReturnType<typeof useTranslate>): string {
  return t(RUN_STATUS_KEYS[run.status]);
}

function operatorGuidanceKey(kind: ApplicationInstallationKind): string {
  switch (kind) {
    case "WINGET":
      return "winget";
    case "DOCKER":
      return "docker";
    case "HOMEBREW":
      return "homebrew";
    case "DISABLED":
      return "disabled";
    default:
      return "other";
  }
}

export function ApplicationUpgradeAction({
  status,
  className,
  onStatusChanged,
}: {
  status: ApplicationUpgradeStatus;
  className?: string;
  onStatusChanged?: () => void;
}) {
  const client = useClient();
  const t = useTranslate();
  const { registerInteractiveJobRun } = useJobRunToasts();
  const [open, setOpen] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const canStart =
    status.eligible &&
    status.updateAvailable &&
    !status.activeRun &&
    Boolean(status.updateTag?.trim()) &&
    Boolean(status.updateVersion?.trim());

  const startUpgrade = React.useCallback(async () => {
    if (!canStart || !status.updateTag || !status.updateVersion) {
      return;
    }

    setBusy(true);
    try {
      const result = await client
        .mutation<{ startApplicationUpgrade?: { jobRun?: unknown } }>(
          startApplicationUpgradeMutation,
          {
            input: {
              expectedTag: status.updateTag,
              expectedVersion: status.updateVersion,
            },
          },
        )
        .toPromise();
      if (result.error) {
        throw result.error;
      }

      const run = normalizeJobRun(result.data?.startApplicationUpgrade?.jobRun);
      if (!run) {
        throw new Error(t("appUpgrade.startFailed"));
      }

      registerInteractiveJobRun(run, () => onStatusChanged?.());
      setOpen(false);
      onStatusChanged?.();
    } catch (error) {
      toast.error(error instanceof Error && error.message ? error.message : t("appUpgrade.startFailed"));
    } finally {
      setBusy(false);
    }
  }, [canStart, client, onStatusChanged, registerInteractiveJobRun, status.updateTag, status.updateVersion, t]);

  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="primary"
        className={className}
        disabled={!canStart || busy}
        onClick={() => setOpen(true)}
      >
        {t("appUpgrade.upgradeNow")}
      </Button>
      <ConfirmDialog
        open={open}
        title={t("appUpgrade.confirmTitle")}
        description={t("appUpgrade.confirmDescription", {
          version: status.updateVersion ?? t("label.unknown"),
        })}
        confirmLabel={t("appUpgrade.confirmAction")}
        cancelLabel={t("label.cancel")}
        confirmButtonVariant="default"
        isBusy={busy}
        onConfirm={() => void startUpgrade()}
        onCancel={() => {
          if (!busy) {
            setOpen(false);
          }
        }}
      >
        <div className="space-y-2 text-sm text-[var(--scry-muted)]">
          <p>{t("appUpgrade.restartNotice")}</p>
          {status.installationKind === "DIRECT_MSI" ? (
            <p>{t("appUpgrade.uacNotice")}</p>
          ) : null}
        </div>
      </ConfirmDialog>
    </>
  );
}

function OperatorGuidance({ status }: { status: ApplicationUpgradeStatus }) {
  const t = useTranslate();
  const guidance = operatorGuidanceKey(status.installationKind);

  if (status.managementOwner !== "OPERATOR") {
    return null;
  }

  const commandKey = guidance === "winget"
    ? "appUpgrade.guidance.wingetCommand"
    : guidance === "homebrew"
      ? "appUpgrade.guidance.homebrewCommand"
      : null;

  return (
    <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3 text-sm">
      <p className="font-medium text-[var(--scry-ink2)]">{t("appUpgrade.operatorGuidance")}</p>
      <p className="mt-1 text-[var(--scry-muted3)]">
        {t(`appUpgrade.guidance.${guidance}`)}
      </p>
      {commandKey ? (
        <code data-code-font className="mt-2 inline-block rounded bg-[var(--scry-soft)] px-2 py-1 text-xs text-[var(--scry-ink2)]">
          {t(commandKey)}
        </code>
      ) : null}
      {guidance === "disabled" ? (
        <code data-code-font className="mt-2 inline-block rounded bg-[var(--scry-soft)] px-2 py-1 text-xs text-[var(--scry-ink2)]">
          {t("appUpgrade.guidance.disabledVariable")}
        </code>
      ) : null}
    </div>
  );
}

export function ApplicationUpgradeSection() {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { status, loading, refresh } = useApplicationUpgradeStatus();
  const activeRun = status?.activeRun ?? null;
  const latestRun = status?.latestRun ?? null;
  const activeProgress = normalizeApplicationUpgradeProgress(activeRun?.progressJson);
  const latestProgress = normalizeApplicationUpgradeProgress(latestRun?.progressJson);
  const rebootRequired =
    activeProgress?.phase === "reboot_required" || latestProgress?.phase === "reboot_required";

  return (
    <section className="overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]">
      <div className="flex flex-col gap-3 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-[15px] font-semibold text-[var(--scry-ink2)]">{t("appUpgrade.title")}</h2>
          <p className="mt-1 text-sm text-[var(--scry-muted3)]">{t("appUpgrade.subtitle")}</p>
        </div>
        {status?.eligible ? (
          <ApplicationUpgradeAction
            status={status}
            className="w-full sm:w-auto"
            onStatusChanged={() => void refresh()}
          />
        ) : null}
      </div>
      <div className="space-y-4 p-4 sm:p-5">
        {loading && !status ? <p className="text-sm text-[var(--scry-muted3)]">{t("appUpgrade.loading")}</p> : null}
        {status ? (
          <>
            <dl className="grid gap-3 sm:grid-cols-3">
              <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3">
                <dt className="text-xs uppercase tracking-[0.12em] text-[var(--scry-muted3)]">{t("appUpgrade.currentVersion")}</dt>
                <dd className="mt-1 font-semibold text-[var(--scry-ink2)]">{status.currentVersion || t("label.unknown")}</dd>
              </div>
              <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3">
                <dt className="text-xs uppercase tracking-[0.12em] text-[var(--scry-muted3)]">{t("appUpgrade.availableVersion")}</dt>
                <dd className="mt-1 font-semibold text-[var(--scry-ink2)]">{status.updateVersion ?? t("appUpgrade.noUpdate")}</dd>
              </div>
              <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3">
                <dt className="text-xs uppercase tracking-[0.12em] text-[var(--scry-muted3)]">{t("appUpgrade.installationKind")}</dt>
                <dd className="mt-1 font-semibold text-[var(--scry-ink2)]">{t(INSTALLATION_KIND_KEYS[status.installationKind])}</dd>
              </div>
            </dl>

            {rebootRequired ? (
              <div className="rounded-[12px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm text-[var(--scry-warning-text)]">
                {t("appUpgrade.rebootRequired")}
              </div>
            ) : null}

            {activeRun ? (
              <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3 text-sm">
                <p className="font-medium text-[var(--scry-ink2)]">{t("appUpgrade.activeRun")}</p>
                <p className="mt-1 text-[var(--scry-muted3)]">
                  {t("appUpgrade.runStatusLabel", { status: runStatusLabel(activeRun, t) })}
                  {" · "}
                  {t(phaseLabelKey(activeProgress?.phase ?? null))}
                </p>
                {activeProgress?.phase === "downloading" && activeProgress.downloadedBytes !== null ? (
                  <p className="mt-1 text-[var(--scry-muted3)]">
                    {activeProgress.totalBytes !== null
                      ? t("appUpgrade.downloadProgress", {
                        downloaded: formatByteCount(activeProgress.downloadedBytes),
                        total: formatByteCount(activeProgress.totalBytes),
                      })
                      : t("appUpgrade.downloadedBytes", {
                        downloaded: formatByteCount(activeProgress.downloadedBytes),
                      })}
                  </p>
                ) : null}
              </div>
            ) : null}

            {latestRun ? (
              <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3 text-sm">
                <p className="font-medium text-[var(--scry-ink2)]">{t("appUpgrade.latestRun")}</p>
                <p className="mt-1 text-[var(--scry-muted3)]">
                  {t("appUpgrade.runStatusLabel", { status: runStatusLabel(latestRun, t) })}
                  {` · ${formatUiDateTime(latestRun.completedAt ?? latestRun.startedAt, dateTimeFormat)}`}
                </p>
                {latestRun.errorText ?? latestProgress?.error ?? latestRun.summaryText ? (
                  <p className="mt-1 text-[var(--scry-muted3)]">
                    {latestRun.errorText ?? latestProgress?.error ?? latestRun.summaryText}
                  </p>
                ) : null}
              </div>
            ) : null}

            <OperatorGuidance status={status} />
          </>
        ) : null}
      </div>
    </section>
  );
}
