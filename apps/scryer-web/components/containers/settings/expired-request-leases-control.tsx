import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { useMaintenanceRuleState } from "@/components/containers/settings/use-maintenance-rule-state";
import { Button } from "@/components/ui/button";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  maintenanceRuleSetQuery,
  maintenanceRuleSetsQuery,
} from "@/lib/graphql/queries";
import { updateMaintenanceRuleMatcherMutation } from "@/lib/graphql/mutations";
import type {
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetRecord,
  MaintenanceRuleState,
} from "@/lib/types/maintenance-rule-sets";
import {
  maintenanceRuleDraftFromDetail,
  ruleStateLabelKey,
  ruleStateOptionsFor,
  updateMaintenanceRuleMatcherInput,
} from "@/lib/utils/maintenance-rule-sets";

/// State and grace period of the built-in rule that deletes titles whose
/// request lease expired. Everything else about it is fixed.
export function ExpiredRequestLeasesControl() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [record, setRecord] = useState<MaintenanceRuleSetRecord | null>(null);
  const [graceDays, setGraceDays] = useState(0);
  const [busyRuleSetId, setBusyRuleSetId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const { data } = await client
      .query(maintenanceRuleSetsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    const shipped =
      ((data?.maintenanceRuleSets ?? []) as MaintenanceRuleSetRecord[]).find(
        (rule) => rule.systemOwned,
      ) ?? null;
    setRecord(shipped);
    if (shipped) setGraceDays(shipped.graceDays);
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const { setRuleState, armingDialog } = useMaintenanceRuleState({
    onChanged: refresh,
    setBusyRuleSetId,
  });

  if (!record) return null;

  const saveGraceDays = async () => {
    setBusyRuleSetId(record.id);
    try {
      const detail = await client
        .query(
          maintenanceRuleSetQuery,
          { id: record.id },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (detail.error) throw detail.error;
      const draft = maintenanceRuleDraftFromDetail(
        detail.data.maintenanceRuleSet as MaintenanceRuleSetDetail,
      );
      const { error } = await client
        .mutation(updateMaintenanceRuleMatcherMutation, {
          input: updateMaintenanceRuleMatcherInput(record.id, {
            ...draft,
            graceDays,
          }),
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.maintenanceRuleUpdated"), {
        level: "SUCCESS",
      });
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
      );
    } finally {
      setBusyRuleSetId(null);
      await refresh();
    }
  };

  const busy = busyRuleSetId !== null;
  return (
    <div
      id="settings-expired-request-leases"
      className="space-y-2 rounded border border-border bg-card px-3 py-2"
    >
      <p className="font-medium text-foreground">{record.name}</p>
      <p className="text-xs text-muted-foreground">
        {t("settings.expiredRequestLeasesHelp")}
      </p>
      <div className="flex flex-wrap items-end gap-3">
        <div className="w-48">
          <SingleSelectField
            id="settings-expired-request-leases-state"
            label={t("settings.maintenanceRuleState")}
            size="sm"
            value={record.state}
            disabled={busy}
            onValueChange={(value) =>
              setRuleState(record, value as MaintenanceRuleState)
            }
            options={ruleStateOptionsFor(record).map((state) => {
              const key = ruleStateLabelKey(state);
              return {
                value: state,
                ariaLabel: state,
                label: key ? t(key) : state,
              };
            })}
          />
        </div>
        <label className="w-32">
          <Label className="mb-2 block">
            {t("settings.maintenanceRuleGraceDays")}
          </Label>
          <Input
            id="settings-expired-request-leases-grace-days"
            {...integerInputProps}
            min={0}
            value={graceDays}
            onChange={(event) =>
              setGraceDays(Number(sanitizeDigits(event.target.value)) || 0)
            }
          />
        </label>
        <Button
          id="settings-expired-request-leases-save"
          type="button"
          size="sm"
          disabled={busy || graceDays === record.graceDays}
          onClick={() => void saveGraceDays()}
        >
          {t("label.save")}
        </Button>
      </div>
      {record.editedSinceEnabled ? (
        <p className="text-xs text-[var(--scry-warning-text)]">
          {t("settings.maintenanceRuleEditedSinceEnabled")}
        </p>
      ) : null}
      {armingDialog}
    </div>
  );
}
