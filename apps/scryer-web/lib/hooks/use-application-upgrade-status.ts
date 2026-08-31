import { useCallback, useEffect, useRef, useState } from "react";
import { useClient } from "urql";

import { applicationUpgradeStatusQuery } from "@/lib/graphql/queries";
import { notifyBackendRestarting } from "@/lib/graphql/urql-client";
import type {
  ApplicationInstallationKind,
  ApplicationUpgradeManagementOwner,
  ApplicationUpgradeStatus,
  JobRun,
} from "@/lib/types";
import { normalizeApplicationUpgradeProgress } from "@/lib/utils/application-upgrade-progress";
import { normalizeJobRun } from "@/lib/utils/job-runs";

const INSTALLATION_KINDS = new Set<ApplicationInstallationKind>([
  "PORTABLE",
  "DIRECT_MSI",
  "DOCKER",
  "HOMEBREW",
  "WINGET",
  "WINDOWS_SUPERVISED",
  "DISABLED",
  "UNSUPPORTED",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nullableText(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function normalizeInstallationKind(value: unknown): ApplicationInstallationKind {
  return typeof value === "string" && INSTALLATION_KINDS.has(value as ApplicationInstallationKind)
    ? value as ApplicationInstallationKind
    : "UNSUPPORTED";
}

function normalizeManagementOwner(value: unknown): ApplicationUpgradeManagementOwner {
  return value === "IN_APP" ? "IN_APP" : "OPERATOR";
}

function normalizeUpgradeStatus(value: unknown): ApplicationUpgradeStatus | null {
  if (!isRecord(value) || typeof value.currentVersion !== "string") {
    return null;
  }

  return {
    currentVersion: value.currentVersion,
    updateVersion: nullableText(value.updateVersion),
    updateTag: nullableText(value.updateTag),
    updateAvailable: value.updateAvailable === true,
    installationKind: normalizeInstallationKind(value.installationKind),
    managementOwner: normalizeManagementOwner(value.managementOwner),
    eligible: value.eligible === true,
    eligibilityReason: nullableText(value.eligibilityReason) ?? "unsupported_layout",
    activeRun: normalizeJobRun(value.activeRun),
    latestRun: normalizeJobRun(value.latestRun),
  };
}

export function useApplicationUpgradeStatus(enabled = true) {
  const client = useClient();
  const [status, setStatus] = useState<ApplicationUpgradeStatus | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<Error | null>(null);
  const activeRunRef = useRef<JobRun | null>(null);
  const previousPhaseRef = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    if (!enabled) {
      return null;
    }

    setLoading(true);
    try {
      const result = await client
        .query<{ applicationUpgradeStatus?: unknown }>(
          applicationUpgradeStatusQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (result.error) {
        throw result.error;
      }

      const nextStatus = normalizeUpgradeStatus(result.data?.applicationUpgradeStatus);
      const progress = normalizeApplicationUpgradeProgress(nextStatus?.activeRun?.progressJson);
      activeRunRef.current = nextStatus?.activeRun ?? null;
      previousPhaseRef.current = progress?.phase ?? null;
      setStatus(nextStatus);
      setError(null);

      if (progress?.phase === "restarting") {
        notifyBackendRestarting();
      }
      return nextStatus;
    } catch (caught) {
      const nextError = caught instanceof Error ? caught : new Error(String(caught));
      setError(nextError);
      if (
        previousPhaseRef.current === "awaiting_elevation" &&
        activeRunRef.current
      ) {
        notifyBackendRestarting();
      }
      return null;
    } finally {
      setLoading(false);
    }
  }, [client, enabled]);

  useEffect(() => {
    if (!enabled) {
      activeRunRef.current = null;
      previousPhaseRef.current = null;
      setStatus(null);
      setError(null);
      setLoading(false);
      return;
    }
    void refresh();
  }, [enabled, refresh]);

  useEffect(() => {
    if (!enabled || !status?.activeRun) {
      return;
    }
    const interval = window.setInterval(() => {
      void refresh();
    }, 2_000);
    return () => window.clearInterval(interval);
  }, [enabled, refresh, status?.activeRun]);

  return { status, loading, error, refresh };
}
