import { AlertTriangle } from "lucide-react";
import { HelpLabel } from "@/components/common/info-help";
import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceDecidedActions,
  MaintenanceDeclaredActions,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  MAINTENANCE_ACTION_HEAD_RULE_NAMES,
  actionHeadLabelKey,
  declaredActionEntries,
  decidedActionEntries,
  isDestructiveActionHead,
  type MaintenanceActionEntry,
} from "@/lib/utils/maintenance-rule-sets";
import {
  regoDiagnosticMessage,
  regoSourceLine,
  type RuleValidationDiagnostic,
} from "@/lib/utils/rego-diagnostics";
import { selectorId } from "@/lib/utils/dom-ids";

/// One action as a badge: the head's label, then the values it carries.
function ActionEntryBadge({
  entry,
  id,
}: {
  entry: MaintenanceActionEntry;
  id?: string;
}) {
  const t = useTranslate();
  const key = actionHeadLabelKey(entry.head);
  const label = key ? t(key) : entry.head;
  return (
    <Badge
      id={id}
      data-maintenance-action-head={entry.head}
      tone={isDestructiveActionHead(entry.head) ? "negative" : "neutral"}
      className="max-w-full text-[11px]"
    >
      <span className="truncate">
        {entry.values.length > 0
          ? `${label}: ${entry.values.join(", ")}`
          : label}
      </span>
    </Badge>
  );
}

function ActionEntryList({
  entries,
  idPrefix,
}: {
  entries: MaintenanceActionEntry[];
  idPrefix: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {entries.map((entry) => (
        <ActionEntryBadge
          key={entry.head}
          entry={entry}
          id={selectorId(idPrefix, entry.head)}
        />
      ))}
    </div>
  );
}

/// What a rule's source declares it can do, in the order the server applies
/// it. Rendered on the rule list and under the editor.
export function MaintenanceDeclaredActionsSummary({
  declared,
  idPrefix,
}: {
  declared: MaintenanceDeclaredActions;
  idPrefix: string;
}) {
  const t = useTranslate();
  const entries = declaredActionEntries(declared);
  if (entries.length === 0) {
    return (
      <span
        id={selectorId(idPrefix, "none")}
        className="text-xs text-muted-foreground"
      >
        {t("settings.maintenanceDeclaredActionsNone")}
      </span>
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-1">
      {declared.destructive ? (
        <Badge
          id={selectorId(idPrefix, "destructive")}
          tone="negative"
          className="text-[11px]"
        >
          <AlertTriangle className="mr-1 h-3 w-3" aria-hidden />
          {t("settings.maintenanceTemplateDestructiveBadge")}
        </Badge>
      ) : null}
      <ActionEntryList entries={entries} idPrefix={idPrefix} />
    </div>
  );
}

/// What a rule decided for one preview row or candidate.
export function MaintenanceDecidedActionsSummary({
  actions,
  idPrefix,
}: {
  actions: MaintenanceDecidedActions;
  idPrefix: string;
}) {
  const t = useTranslate();
  const entries = decidedActionEntries(actions);
  if (entries.length === 0) {
    return (
      <span className="text-xs text-muted-foreground">
        {t("settings.maintenanceDecidedActionsNone")}
      </span>
    );
  }
  return <ActionEntryList entries={entries} idPrefix={idPrefix} />;
}

/// The panel under the editor: what the source declares, and validation
/// warnings, which never block saving, kept apart from errors.
export function MaintenanceRuleSourceFindings({
  source,
  declared,
  warnings,
  checked,
}: {
  source: string;
  /// Null when the source could not be parsed, undefined before validation.
  declared: MaintenanceDeclaredActions | null | undefined;
  warnings: readonly RuleValidationDiagnostic[];
  /// Validation has answered for the current source.
  checked: boolean;
}) {
  const t = useTranslate();
  if (!checked) return null;
  const order = MAINTENANCE_ACTION_HEAD_ORDER.map(
    (head) => MAINTENANCE_ACTION_HEAD_RULE_NAMES[head],
  ).join(", ");
  return (
    <div
      id="settings-maintenance-rule-source-findings"
      className="space-y-2 rounded border border-border bg-muted/30 px-3 py-2"
    >
      <div className="space-y-1">
        <p className="text-xs font-medium">
          <HelpLabel
            label={t("settings.maintenanceDeclaredActionsTitle")}
            help={t("settings.maintenanceDeclaredActionsHelp", { order })}
          />
        </p>
        {declared ? (
          <MaintenanceDeclaredActionsSummary
            declared={declared}
            idPrefix="settings-maintenance-rule-declared-action"
          />
        ) : (
          <p
            id="settings-maintenance-rule-declared-actions-unavailable"
            className="text-xs text-muted-foreground"
          >
            {t("settings.maintenanceDeclaredActionsUnavailable")}
          </p>
        )}
      </div>
      {warnings.length > 0 ? (
        <div className="space-y-1">
          <p className="text-xs font-medium text-[var(--scry-warning-text)]">
            {t("settings.maintenanceRuleWarningsTitle")}
          </p>
          <ul
            id="settings-maintenance-rule-warnings"
            className="space-y-1 text-xs"
          >
            {warnings.map((warning, index) => {
              const line = warning.line
                ? regoSourceLine(source, warning.line)
                : null;
              return (
                <li
                  key={`${warning.code}-${index}`}
                  id={selectorId(
                    "settings-maintenance-rule-warning",
                    `${warning.code}-${index}`,
                  )}
                  data-diagnostic-code={warning.code}
                  className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-[var(--scry-warning-text)]"
                >
                  {line && line > 0 ? (
                    <span className="mr-1 font-mono">
                      {t("settings.maintenanceRuleWarningLocation", {
                        line,
                        column: warning.column ?? 1,
                      })}
                    </span>
                  ) : null}
                  {regoDiagnosticMessage(warning, t)}
                </li>
              );
            })}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
