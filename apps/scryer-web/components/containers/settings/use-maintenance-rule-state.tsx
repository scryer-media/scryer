import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { useCallback, useState } from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { maintenanceCandidatesQuery } from "@/lib/graphql/queries";
import { setMaintenanceRuleStateMutation } from "@/lib/graphql/mutations";
import type {
  MaintenanceCandidate,
  MaintenanceRuleSetRecord,
  MaintenanceRuleState,
} from "@/lib/types/maintenance-rule-sets";
import {
  isNonTerminalCandidateState,
  parseAcknowledgedCandidateCountMismatch,
  ruleStateNeedsConfirmation,
  setMaintenanceRuleStateVariables,
} from "@/lib/utils/maintenance-rule-sets";

export const MAINTENANCE_CANDIDATE_LIMIT = 200;

/// Enabling a Destructive rule in flight. The count is the number the dialog actually
/// showed, and the same number it sends as the acknowledgement, so the server's
/// check compares against what the operator saw rather than a fresher count
/// they never read.
type PendingArming = {
  record: MaintenanceRuleSetRecord;
  candidateCount: number;
  titles: string[];
  acknowledged: boolean;
  loading: boolean;
  /// Set after the server refused the acknowledgement and the dialog re-asked
  /// against the count the server reported.
  countChanged: boolean;
};

/// The per-rule Disabled / Dry run / Enabled control, including the reviewed
/// confirmation that enabling a rule which deletes files requires. Every
/// surface that moves a rule's state goes through this, so none of them can
/// skip the confirmation.
export function useMaintenanceRuleState({
  onChanged,
  setBusyRuleSetId,
}: {
  onChanged: () => Promise<void>;
  setBusyRuleSetId: (id: string | null) => void;
}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [pendingArming, setPendingArming] = useState<PendingArming | null>(
    null,
  );

  const applyRuleState = useCallback(
    async (
      record: MaintenanceRuleSetRecord,
      state: MaintenanceRuleState,
      candidateCount?: number,
    ): Promise<
      { ok: true } | { ok: false; message: string; error: unknown }
    > => {
      const { error } = await client
        .mutation(
          setMaintenanceRuleStateMutation,
          setMaintenanceRuleStateVariables(record, state, candidateCount),
        )
        .toPromise();
      if (error) {
        return {
          ok: false,
          message: userFacingGraphQlErrorMessage(
            error,
            t("status.failedToUpdate"),
            t,
          ),
          error,
        };
      }
      await onChanged();
      return { ok: true };
    },
    [client, onChanged, t],
  );

  /// List what the rule could act on once enabled. The dialog shows every
  /// title and sends the count it showed.
  const openDestructiveArming = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      setPendingArming({
        record,
        candidateCount: 0,
        titles: [],
        acknowledged: false,
        loading: true,
        countChanged: false,
      });
      try {
        const { data, error } = await client
          .query(
            maintenanceCandidatesQuery,
            { ruleSetId: record.id, limit: MAINTENANCE_CANDIDATE_LIMIT },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;
        const actionable = (
          (data?.maintenanceCandidates as MaintenanceCandidate[]) ?? []
        ).filter((candidate) => isNonTerminalCandidateState(candidate.state));
        setPendingArming((prev) =>
          prev && prev.record.id === record.id
            ? {
                ...prev,
                candidateCount: actionable.length,
                titles: actionable.map((candidate) =>
                  candidate.subjectKind === "title"
                    ? candidate.titleName
                    : `${candidate.titleName} · ${candidate.subjectLabel}`,
                ),
                loading: false,
              }
            : prev,
        );
      } catch (error) {
        setPendingArming(null);
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
          { level: "ERROR" },
        );
      }
    },
    [client, setGlobalStatus, t],
  );

  const setRuleState = useCallback(
    (record: MaintenanceRuleSetRecord, state: MaintenanceRuleState) => {
      if (state === record.state) return;
      if (ruleStateNeedsConfirmation(record, state)) {
        void openDestructiveArming(record);
        return;
      }
      void (async () => {
        setBusyRuleSetId(record.id);
        const result = await applyRuleState(record, state);
        setBusyRuleSetId(null);
        setGlobalStatus(
          result.ok
            ? t("settings.maintenanceRuleStateUpdated", { name: record.name })
            : result.message,
          { level: result.ok ? "SUCCESS" : "ERROR" },
        );
      })();
    },
    [
      applyRuleState,
      openDestructiveArming,
      setBusyRuleSetId,
      setGlobalStatus,
      t,
    ],
  );

  /// The server rejects a stale acknowledgement and reports the count it holds.
  /// Rather than dropping the operator back to the table, the dialog adopts
  /// that count, clears the acknowledgement, and asks again against the real
  /// number.
  const confirmDestructiveArming = useCallback(async () => {
    if (!pendingArming || !pendingArming.acknowledged) return;
    const { record, candidateCount } = pendingArming;
    setPendingArming((prev) => (prev ? { ...prev, loading: true } : prev));
    const result = await applyRuleState(record, "ENABLED", candidateCount);
    if (result.ok) {
      setPendingArming(null);
      setGlobalStatus(
        t("settings.maintenanceRuleStateUpdated", { name: record.name }),
        { level: "SUCCESS" },
      );
      return;
    }
    const serverCount = parseAcknowledgedCandidateCountMismatch(result.error);
    if (serverCount === null) {
      setPendingArming(null);
      setGlobalStatus(
        userFacingGraphQlErrorMessage(result, t("status.apiError"), t),
        { level: "ERROR" },
      );
      await onChanged();
      return;
    }
    setPendingArming((prev) =>
      prev
        ? {
            ...prev,
            candidateCount: serverCount,
            acknowledged: false,
            loading: false,
            countChanged: true,
          }
        : prev,
    );
  }, [applyRuleState, onChanged, pendingArming, setGlobalStatus, t]);

  const armingDialog = (
    <ConfirmDialog
      open={pendingArming !== null}
      contentId="settings-maintenance-arming-confirm"
      title={t("settings.maintenanceArmDestructiveTitle")}
      description={
        pendingArming
          ? t("settings.maintenanceArmDestructiveIntro", {
              name: pendingArming.record.name,
            })
          : ""
      }
      confirmLabel={t("settings.maintenanceArmDestructiveConfirm")}
      cancelLabel={t("label.cancel")}
      isBusy={pendingArming?.loading ?? false}
      confirmDisabled={!pendingArming?.acknowledged}
      onConfirm={() => void confirmDestructiveArming()}
      onCancel={() => setPendingArming(null)}
    >
      {pendingArming ? (
        <div className="space-y-2 text-xs">
          {pendingArming.loading && pendingArming.candidateCount === 0 ? (
            <p className="text-muted-foreground">
              {t("settings.maintenanceArmDestructiveLoading")}
            </p>
          ) : (
            <>
              {pendingArming.countChanged ? (
                <p
                  id="settings-maintenance-arming-count-changed"
                  className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1.5 text-[var(--scry-warning-text)]"
                >
                  {t("settings.maintenanceArmDestructiveCountChanged", {
                    count: pendingArming.candidateCount,
                  })}
                </p>
              ) : null}
              <p
                id="settings-maintenance-arming-count"
                className="font-semibold"
              >
                {pendingArming.candidateCount === 0
                  ? t("settings.maintenanceArmDestructiveNoCandidates")
                  : t("settings.maintenanceArmDestructiveCount", {
                      count: pendingArming.candidateCount,
                    })}
              </p>
              {pendingArming.titles.length > 0 ? (
                <ul
                  id="settings-maintenance-arming-candidates"
                  className="max-h-48 list-disc space-y-0.5 overflow-y-auto pl-5 text-muted-foreground"
                >
                  {pendingArming.titles.map((title, index) => (
                    <li key={`${index}-${title}`}>{title}</li>
                  ))}
                </ul>
              ) : null}
              <label className="flex items-start gap-2 pt-1">
                <Checkbox
                  id="settings-maintenance-arming-acknowledge"
                  checked={pendingArming.acknowledged}
                  onCheckedChange={(value) =>
                    setPendingArming((prev) =>
                      prev ? { ...prev, acknowledged: value === true } : prev,
                    )
                  }
                />
                <span>
                  {t("settings.maintenanceArmDestructiveAcknowledge", {
                    count: pendingArming.candidateCount,
                  })}
                </span>
              </label>
            </>
          )}
        </div>
      ) : null}
    </ConfirmDialog>
  );

  return { setRuleState, armingDialog };
}
