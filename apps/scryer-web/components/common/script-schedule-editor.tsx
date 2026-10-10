import { useEffect, useState, type ReactNode } from "react";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import { useClient } from "urql";

import { ScriptChoiceGroup } from "@/components/common/script-choice-group";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { validateScriptScheduleQuery } from "@/lib/graphql/queries";
import { cn } from "@/lib/utils";
import {
  SCHEDULE_WEEKDAYS,
  type ScheduleWeekdayValue,
  type ScriptSchedule,
  type ScriptScheduleKindValue,
  type ScriptScheduleValidation,
} from "@/lib/types/scripts";
import {
  describeCronExpression,
  documentUiLanguage,
  isCronDescriptionLocaleLoaded,
  loadCronDescriptionLocale,
} from "@/lib/utils/cron-description";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  intervalToSeconds,
  orderWeekdays,
  scheduleForKind,
  splitIntervalSeconds,
  toScriptScheduleInput,
  type IntervalUnit,
} from "@/lib/utils/script-schedule";

const SCHEDULE_KINDS: readonly ScriptScheduleKindValue[] = ["MANUAL", "INTERVAL", "DAILY", "WEEKLY", "CRON"];
const INTERVAL_UNITS: readonly IntervalUnit[] = ["minutes", "hours", "days"];
const VALIDATION_DEBOUNCE_MS = 400;
const NEXT_RUNS_SHOWN = 3;

// The height of a choice group, so a text field beside one is the same size.
const SCHEDULE_INPUT_HEIGHT_CLASS = "h-10.5";

/** A labelled text field laid out like a choice group, so the two line up side by side. */
function ScheduleField({
  label,
  htmlFor,
  className,
  children,
}: {
  label: string;
  htmlFor: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={cn("min-w-0 space-y-1.5", className)}>
      <div className="flex h-5 items-center">
        <Label htmlFor={htmlFor} className="block">
          {label}
        </Label>
      </div>
      {children}
    </div>
  );
}

type CronValidationState = {
  expression: string;
  result: ScriptScheduleValidation | null;
  failure: string | null;
};

function CronExpressionField({
  expression,
  onChange,
  disabled,
}: {
  expression: string;
  onChange: (expression: string) => void;
  disabled?: boolean;
}) {
  const t = useTranslate();
  const client = useClient();
  const dateTimeFormat = useUiDateTimeFormat();
  const uiLanguage = documentUiLanguage();
  const [, setLoadedLanguage] = useState<string | null>(null);
  const [validation, setValidation] = useState<CronValidationState | null>(null);

  useEffect(() => {
    if (isCronDescriptionLocaleLoaded(uiLanguage)) return;
    let cancelled = false;
    void loadCronDescriptionLocale(uiLanguage).then(() => {
      if (!cancelled) setLoadedLanguage(uiLanguage);
    });
    return () => {
      cancelled = true;
    };
  }, [uiLanguage]);

  const trimmed = expression.trim();
  useEffect(() => {
    if (!trimmed) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      const schedule = toScriptScheduleInput({
        kind: "CRON",
        everySeconds: null,
        timeLocal: null,
        days: null,
        expression: trimmed,
      });
      void client
        .query(validateScriptScheduleQuery, { schedule }, { requestPolicy: "network-only" })
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled) return;
          if (error) {
            setValidation({ expression: trimmed, result: null, failure: error.message });
            return;
          }
          const payload = data?.validateScriptSchedule as ScriptScheduleValidation | undefined;
          setValidation({
            expression: trimmed,
            result: payload
              ? {
                  valid: payload.valid === true,
                  error: payload.error ?? null,
                  description: payload.description ?? null,
                  nextRuns: Array.isArray(payload.nextRuns) ? payload.nextRuns : [],
                }
              : null,
            failure: null,
          });
        });
    }, VALIDATION_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [client, trimmed]);

  const description = describeCronExpression(expression, uiLanguage, {
    use24HourTimeFormat: dateTimeFormat === "ISO24H" ? true : undefined,
  });
  const current = trimmed && validation?.expression === trimmed ? validation : null;

  return (
    <div className="space-y-2">
      <label className="block">
        <Label className="mb-2 block">{t("script.schedule.cronExpression")}</Label>
        <Input
          id="script-schedule-cron"
          value={expression}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
          className="font-[var(--font-code)]"
          placeholder="0 3 * * *"
          autoComplete="off"
          spellCheck={false}
        />
      </label>
      <p
        id="script-schedule-cron-description"
        className="text-xs text-[var(--scry-ink2)]"
        aria-live="polite"
      >
        {description ?? (trimmed ? t("script.schedule.cronUnreadable") : t("script.schedule.cronHelp"))}
      </p>
      {current ? (
        <div id="script-schedule-validation" className="space-y-1 text-xs" aria-live="polite">
          {current.failure ? (
            <p className="text-[var(--scry-danger-text-soft)]">{current.failure}</p>
          ) : current.result && !current.result.valid ? (
            <p className="text-[var(--scry-danger-text-soft)]">
              {current.result.error ?? t("script.schedule.invalid")}
            </p>
          ) : current.result ? (
            <>
              <p className="text-muted-foreground">{t("script.schedule.nextRuns")}</p>
              <ul className="space-y-0.5">
                {current.result.nextRuns.slice(0, NEXT_RUNS_SHOWN).map((run, index) => (
                  <li
                    key={run}
                    id={`script-schedule-next-run-${index}`}
                    className="text-[var(--scry-ink2)]"
                  >
                    {formatUiDateTime(run, dateTimeFormat, { fallback: run })}
                  </li>
                ))}
              </ul>
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function IntervalFields({
  everySeconds,
  onChange,
  disabled,
}: {
  everySeconds: number | null;
  onChange: (everySeconds: number) => void;
  disabled?: boolean;
}) {
  const t = useTranslate();
  const [initial] = useState(() => splitIntervalSeconds(everySeconds));
  const [unit, setUnit] = useState<IntervalUnit>(initial.unit);
  const [amountText, setAmountText] = useState(String(initial.amount));

  const commit = (text: string, nextUnit: IntervalUnit) => {
    const amount = Number(text);
    if (text && amount >= 1) {
      onChange(intervalToSeconds(amount, nextUnit));
    }
  };

  return (
    <div className="space-y-3">
      {/* Two equal columns sized by the unit group, so the amount field matches it. */}
      <div className="inline-grid max-w-full grid-cols-2 gap-x-4">
        <ScheduleField label={t("script.schedule.every")} htmlFor="script-schedule-interval-amount">
          {/* Takes the column's width without widening it. */}
          <div className="w-0 min-w-full">
            <Input
              id="script-schedule-interval-amount"
              {...integerInputProps}
              className={SCHEDULE_INPUT_HEIGHT_CLASS}
              value={amountText}
              disabled={disabled}
              onChange={(event) => {
                const text = sanitizeDigits(event.target.value);
                setAmountText(text);
                commit(text, unit);
              }}
              onBlur={() => {
                if (!amountText || Number(amountText) < 1) {
                  setAmountText("1");
                  commit("1", unit);
                }
              }}
            />
          </div>
        </ScheduleField>
        <ScriptChoiceGroup
          id="script-schedule-interval-unit"
          label={t("script.schedule.unit")}
          value={unit}
          disabled={disabled}
          onValueChange={(next) => {
            const nextUnit = next as IntervalUnit;
            setUnit(nextUnit);
            commit(amountText || "1", nextUnit);
          }}
          options={INTERVAL_UNITS.map((candidate) => ({
            value: candidate,
            dataValue: candidate.toUpperCase(),
            label: t(`script.schedule.unit.${candidate}`),
          }))}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t("script.schedule.minimumInterval")}</p>
    </div>
  );
}

function TimeField({
  value,
  onChange,
  disabled,
}: {
  value: string | null;
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const t = useTranslate();
  return (
    <ScheduleField label={t("script.schedule.time")} htmlFor="script-schedule-time" className="w-48">
      <Input
        id="script-schedule-time"
        type="time"
        className={SCHEDULE_INPUT_HEIGHT_CLASS}
        value={value ?? ""}
        disabled={disabled}
        onChange={(event) => {
          if (event.target.value) onChange(event.target.value);
        }}
      />
    </ScheduleField>
  );
}

function WeekdayToggles({
  days,
  onChange,
  disabled,
}: {
  days: ScheduleWeekdayValue[];
  onChange: (days: ScheduleWeekdayValue[]) => void;
  disabled?: boolean;
}) {
  const t = useTranslate();
  const labelId = "script-schedule-days-label";
  return (
    <div className="min-w-0 space-y-1.5">
      <div className="flex h-5 items-center">
        <Label id={labelId} className="block">
          {t("script.schedule.days")}
        </Label>
      </div>
      <ToggleGroupPrimitive.Root
        id="script-schedule-days"
        type="multiple"
        aria-labelledby={labelId}
        // Sized to its days, like the single-choice groups beside it.
        className="inline-flex max-w-full flex-wrap gap-1 rounded-md border border-border p-1"
        value={days}
        disabled={disabled}
        onValueChange={(next) => {
          // A weekly schedule always keeps at least one day.
          if (next.length === 0) return;
          onChange(orderWeekdays(next as ScheduleWeekdayValue[]));
        }}
      >
        {SCHEDULE_WEEKDAYS.map((day) => (
          <ToggleGroupPrimitive.Item
            key={day}
            id={`script-schedule-day-${day.toLowerCase()}`}
            value={day}
            data-value={day}
            asChild
          >
            <Button type="button" size="sm" variant={days.includes(day) ? "default" : "ghost"}>
              {t(`script.schedule.day.${day.toLowerCase()}`)}
            </Button>
          </ToggleGroupPrimitive.Item>
        ))}
      </ToggleGroupPrimitive.Root>
    </div>
  );
}

/**
 * Edits when a scheduled script runs. Controlled: every change reports the
 * whole schedule, with only the fields its kind uses.
 */
export function ScriptScheduleEditor({
  value,
  onChange,
  runOnStartup,
  onRunOnStartupChange,
  disabled,
}: {
  value: ScriptSchedule;
  onChange: (value: ScriptSchedule) => void;
  runOnStartup: boolean;
  onRunOnStartupChange: (runOnStartup: boolean) => void;
  disabled?: boolean;
}) {
  const t = useTranslate();

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end gap-4">
        <ScriptChoiceGroup
          id="script-schedule-kind"
          label={t("script.schedule.kind")}
          value={value.kind}
          disabled={disabled}
          onValueChange={(kind) => onChange(scheduleForKind(kind as ScriptScheduleKindValue, value))}
          options={SCHEDULE_KINDS.map((kind) => ({
            value: kind,
            label: t(`script.schedule.kind.${kind.toLowerCase()}`),
          }))}
        />
        <label className="flex h-10 items-center gap-2">
          <Checkbox
            id="script-run-on-startup"
            checked={runOnStartup}
            disabled={disabled}
            onCheckedChange={(checked) => onRunOnStartupChange(checked === true)}
          />
          <span className="text-sm">{t("script.schedule.runOnStartup")}</span>
        </label>
      </div>

      {value.kind === "MANUAL" ? (
        <p className="text-xs text-muted-foreground">{t("script.schedule.manualHelp")}</p>
      ) : null}
      {value.kind === "INTERVAL" ? (
        <IntervalFields
          everySeconds={value.everySeconds}
          disabled={disabled}
          onChange={(everySeconds) => onChange({ ...value, everySeconds })}
        />
      ) : null}
      {value.kind === "DAILY" ? (
        <TimeField
          value={value.timeLocal}
          disabled={disabled}
          onChange={(timeLocal) => onChange({ ...value, timeLocal })}
        />
      ) : null}
      {value.kind === "WEEKLY" ? (
        <div className="flex flex-wrap gap-x-4 gap-y-3">
          <WeekdayToggles
            days={value.days ?? []}
            disabled={disabled}
            onChange={(days) => onChange({ ...value, days })}
          />
          <TimeField
            value={value.timeLocal}
            disabled={disabled}
            onChange={(timeLocal) => onChange({ ...value, timeLocal })}
          />
        </div>
      ) : null}
      {value.kind === "CRON" ? (
        <CronExpressionField
          expression={value.expression ?? ""}
          disabled={disabled}
          onChange={(expression) => onChange({ ...value, expression })}
        />
      ) : null}
    </div>
  );
}
