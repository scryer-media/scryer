import { HelpLabel } from "@/components/common/info-help";
import { ScriptScheduleEditor } from "@/components/common/script-schedule-editor";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { MaintenanceRuleSetRecord } from "@/lib/types/maintenance-rule-sets";
import type { ScriptSchedule } from "@/lib/types/scripts";
import {
  describeSchedule,
  documentUiLanguage,
} from "@/lib/utils/cron-description";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";

/// A rule's schedule in words and, when one is planned, its next run in the
/// viewer's local time. Both come from the rule list the page already loads.
export function MaintenanceRuleScheduleSummary({
  record,
  idPrefix,
}: {
  record: Pick<MaintenanceRuleSetRecord, "schedule" | "nextRunAt">;
  idPrefix: string;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  return (
    <div className="space-y-0.5">
      <span id={selectorId(idPrefix, "description")}>
        {describeSchedule(record.schedule, t, documentUiLanguage(), {
          use24HourTimeFormat: dateTimeFormat === "ISO24H" ? true : undefined,
        })}
      </span>
      {record.nextRunAt ? (
        <span
          id={selectorId(idPrefix, "next-run")}
          className="block text-xs text-muted-foreground"
        >
          {t("jobs.nextRunPrefix", {
            value: formatUiDateTime(record.nextRunAt, dateTimeFormat, {
              fallback: record.nextRunAt,
            }),
          })}
        </span>
      ) : null}
    </div>
  );
}

/// The schedule control of a rule: the scheduled-script editor, without the
/// startup option, which rules do not have.
export function MaintenanceRuleScheduleField({
  value,
  onChange,
  idPrefix,
  disabled,
  nextRunAt,
}: {
  value: ScriptSchedule;
  onChange: (value: ScriptSchedule) => void;
  idPrefix: string;
  disabled?: boolean;
  /// The saved rule's next run, shown under the editor. Omitted for a rule
  /// that is not saved yet.
  nextRunAt?: string | null;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  return (
    <div id={idPrefix} className="space-y-2">
      <Label className="block">
        <HelpLabel
          label={t("script.editor.schedule")}
          help={t("settings.maintenanceRuleScheduleHelp")}
        />
      </Label>
      <ScriptScheduleEditor
        idPrefix={idPrefix}
        value={value}
        onChange={onChange}
        disabled={disabled}
        manualHelp={t("settings.maintenanceRuleScheduleManualHelp")}
      />
      {nextRunAt ? (
        <p
          id={selectorId(idPrefix, "next-run")}
          className="text-xs text-muted-foreground"
        >
          {t("jobs.nextRunPrefix", {
            value: formatUiDateTime(nextRunAt, dateTimeFormat, {
              fallback: nextRunAt,
            }),
          })}
        </p>
      ) : null}
    </div>
  );
}
