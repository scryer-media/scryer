import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { useMaintenanceRuleState } from "@/components/containers/settings/use-maintenance-rule-state";
import { HelpLabel } from "@/components/common/info-help";
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
import {
  updateMaintenanceRuleMatcherMutation,
  updateMaintenanceRuleMetadataMutation,
} from "@/lib/graphql/mutations";
import type {
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetRecord,
  MaintenanceRuleState,
} from "@/lib/types/maintenance-rule-sets";
import type { ScriptSchedule } from "@/lib/types/scripts";
import { useRuleBuilderSchema } from "@/lib/hooks/use-rule-builder-schema";
import { MAINTENANCE_RULE_BUILDER_BASE } from "@/lib/utils/maintenance-rule-builder-schema";
import {
  maintenanceRuleBuilder,
  maintenanceRuleDraftFromDetail,
  maintenanceRuleUpdatePlan,
  ruleStateLabelKey,
  ruleStateOptionsFor,
} from "@/lib/utils/maintenance-rule-sets";
import { toScriptScheduleInput } from "@/lib/utils/script-schedule";
import { MaintenanceRuleScheduleField } from "@/components/views/settings/maintenance-rule-schedule";

const sameSchedule = (left: ScriptSchedule, right: ScriptSchedule) =>
  JSON.stringify(toScriptScheduleInput(left)) ===
  JSON.stringify(toScriptScheduleInput(right));

/// State, grace period and schedule of the built-in rule that deletes titles
/// whose request lease expired. Everything else about it is fixed.
export function ExpiredRequestLeasesControl() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [record, setRecord] = useState<MaintenanceRuleSetRecord | null>(null);
  const [graceDays, setGraceDays] = useState(0);
  const [schedule, setSchedule] = useState<ScriptSchedule | null>(null);
  const [busyRuleSetId, setBusyRuleSetId] = useState<string | null>(null);
  const { whenReady: whenSchemaReady } = useRuleBuilderSchema(
    MAINTENANCE_RULE_BUILDER_BASE,
  );

  const refresh = useCallback(async () => {
    const { data } = await client
      .query(maintenanceRuleSetsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    const shipped =
      ((data?.maintenanceRuleSets ?? []) as MaintenanceRuleSetRecord[]).find(
        (rule) => rule.systemOwned,
      ) ?? null;
    setRecord(shipped);
    if (shipped) {
      setGraceDays(shipped.graceDays);
      setSchedule(shipped.schedule);
    }
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const { setRuleState, armingDialog } = useMaintenanceRuleState({
    onChanged: refresh,
    setBusyRuleSetId,
  });

  if (!record || !schedule) return null;

  const scheduleChanged = !sameSchedule(schedule, record.schedule);

  /// The grace period is part of the matcher revision and the schedule is a
  /// rule setting, so each goes on its own mutation and only when changed.
  const saveSettings = async () => {
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
      // Without the served schema the stored builder model is left as it is.
      const binding = maintenanceRuleBuilder(await whenSchemaReady());
      const draft = maintenanceRuleDraftFromDetail(
        detail.data.maintenanceRuleSet as MaintenanceRuleSetDetail,
        binding,
      );
      const plan = maintenanceRuleUpdatePlan(
        record.id,
        draft,
        { ...draft, graceDays, schedule },
        binding,
      );
      if (plan.matcher) {
        const { error } = await client
          .mutation(updateMaintenanceRuleMatcherMutation, {
            input: plan.matcher,
          })
          .toPromise();
        if (error) throw error;
      }
      if (plan.metadata) {
        const { error } = await client
          .mutation(updateMaintenanceRuleMetadataMutation, {
            input: plan.metadata,
          })
          .toPromise();
        if (error) throw error;
      }
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
      <p className="font-medium text-foreground">
        <HelpLabel
          label={record.name}
          help={t("settings.expiredRequestLeasesHelp")}
        />
      </p>
      <MaintenanceRuleScheduleField
        key={record.updatedAt}
        idPrefix="settings-expired-request-leases-schedule"
        value={schedule}
        onChange={setSchedule}
        disabled={busy}
        nextRunAt={scheduleChanged ? null : record.nextRunAt}
      />
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
          disabled={
            busy || (graceDays === record.graceDays && !scheduleChanged)
          }
          onClick={() => void saveSettings()}
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
