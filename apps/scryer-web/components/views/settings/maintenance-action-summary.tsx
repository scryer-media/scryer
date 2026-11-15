import type { ReactNode } from "react";
import { AlertTriangle } from "lucide-react";
import { HelpLabel } from "@/components/common/info-help";
import { Badge } from "@/components/ui/badge";
import { formatAudioLanguageLabels } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceDecidedActions,
  MaintenanceDeclaredActions,
} from "@/lib/types/maintenance-rule-sets";
import type {
  RequestDeclaredActions,
  RequestShaping,
} from "@/lib/types/request-rule-sets";
import {
  declaredActionEntries,
  decidedActionEntries,
} from "@/lib/utils/maintenance-rule-sets";
import {
  requestDeclaredActionEntries,
  requestMonitorTypeLabelKey,
  requestShapingConflicts,
  requestShapingEntries,
  type RequestActionValueFormat,
} from "@/lib/utils/request-rule-sets";
import {
  regoDiagnosticMessage,
  regoSourceLine,
  type RuleValidationDiagnostic,
} from "@/lib/utils/rego-diagnostics";
import { selectorId } from "@/lib/utils/dom-ids";
import { useRuleOutputDescriptors } from "@/lib/hooks/use-rule-builder-schema";
import type { RuleBuilderOutputDescriptor } from "@/lib/utils/rule-builder";
import {
  isRuleOutputDestructive,
  ruleOutputHeadLabel,
  ruleOutputHeadNames,
  sortByRuleOutputOrder,
} from "@/lib/utils/rule-output-vocabulary";

/// One head of a rule family as a summary shows it: the head and the values
/// it carries, already formatted.
type ActionEntry = { head: string; values: readonly string[] };

/// The heads of one rule family as the server describes them. `family` is the
/// builder family the descriptors are read for, and names the data attribute
/// each badge carries, `data-<family>-action-head`.
type ActionHeadVocabulary = {
  family: string;
  descriptors: readonly RuleBuilderOutputDescriptor[];
};

function useActionHeadVocabulary(family: string): ActionHeadVocabulary {
  return { family, descriptors: useRuleOutputDescriptors(family) };
}

/// One action as a badge: the head's label, then the values it carries.
function ActionEntryBadge({
  entry,
  vocabulary,
  id,
}: {
  entry: ActionEntry;
  vocabulary: ActionHeadVocabulary;
  id?: string;
}) {
  const t = useTranslate();
  const label = ruleOutputHeadLabel(t, vocabulary.descriptors, entry.head);
  return (
    <Badge
      id={id}
      {...{ [`data-${vocabulary.family}-action-head`]: entry.head }}
      tone={
        isRuleOutputDestructive(vocabulary.descriptors, entry.head)
          ? "negative"
          : "neutral"
      }
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
  vocabulary,
  idPrefix,
}: {
  entries: readonly ActionEntry[];
  vocabulary: ActionHeadVocabulary;
  idPrefix: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {entries.map((entry) => (
        <ActionEntryBadge
          key={entry.head}
          entry={entry}
          vocabulary={vocabulary}
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
  const vocabulary = useActionHeadVocabulary("maintenance");
  const entries = sortByRuleOutputOrder(
    vocabulary.descriptors,
    declaredActionEntries(declared),
    (entry) => entry.head,
  );
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
      <ActionEntryList
        entries={entries}
        vocabulary={vocabulary}
        idPrefix={idPrefix}
      />
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
  const vocabulary = useActionHeadVocabulary("maintenance");
  const entries = decidedActionEntries(actions);
  if (entries.length === 0) {
    return (
      <span className="text-xs text-muted-foreground">
        {t("settings.maintenanceDecidedActionsNone")}
      </span>
    );
  }
  return (
    <ActionEntryList
      entries={entries}
      vocabulary={vocabulary}
      idPrefix={idPrefix}
    />
  );
}

/// The keys and ids one rule family's findings panel uses.
type SourceFindingsVocabulary = {
  /// Every id in the panel hangs off it: `<id>-source-findings`,
  /// `<id>-declared-actions-unavailable`, `<id>-warnings`, `<id>-warning-*`.
  idPrefix: string;
  titleKey: string;
  helpKey: string;
  unavailableKey: string;
  warningsTitleKey: string;
  warningLocationKey: string;
};

/// The panel under a rule editor: what the source declares, and validation
/// warnings, which never block saving, kept apart from errors.
function RuleSourceFindings({
  vocabulary,
  source,
  declared,
  order,
  warnings,
  checked,
}: {
  vocabulary: SourceFindingsVocabulary;
  source: string;
  /// Null when the source could not be read.
  declared: ReactNode | null;
  /// The head order the help names.
  order: string;
  warnings: readonly RuleValidationDiagnostic[];
  checked: boolean;
}) {
  const t = useTranslate();
  if (!checked) return null;
  const prefix = vocabulary.idPrefix;
  return (
    <div
      id={`${prefix}-source-findings`}
      className="space-y-2 rounded border border-border bg-muted/30 px-3 py-2"
    >
      <div className="space-y-1">
        <p className="text-xs font-medium">
          <HelpLabel
            label={t(vocabulary.titleKey)}
            help={t(vocabulary.helpKey, { order })}
          />
        </p>
        {declared ?? (
          <p
            id={`${prefix}-declared-actions-unavailable`}
            className="text-xs text-muted-foreground"
          >
            {t(vocabulary.unavailableKey)}
          </p>
        )}
      </div>
      {warnings.length > 0 ? (
        <div className="space-y-1">
          <p className="text-xs font-medium text-[var(--scry-warning-text)]">
            {t(vocabulary.warningsTitleKey)}
          </p>
          <ul id={`${prefix}-warnings`} className="space-y-1 text-xs">
            {warnings.map((warning, index) => {
              const line = warning.line
                ? regoSourceLine(source, warning.line)
                : null;
              return (
                <li
                  key={`${warning.code}-${index}`}
                  id={selectorId(
                    `${prefix}-warning`,
                    `${warning.code}-${index}`,
                  )}
                  data-diagnostic-code={warning.code}
                  className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1 text-[var(--scry-warning-text)]"
                >
                  {line && line > 0 ? (
                    <span className="mr-1 font-mono">
                      {t(vocabulary.warningLocationKey, {
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

/// The maintenance editor's findings panel.
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
  const descriptors = useRuleOutputDescriptors("maintenance");
  return (
    <RuleSourceFindings
      vocabulary={{
        idPrefix: "settings-maintenance-rule",
        titleKey: "settings.maintenanceDeclaredActionsTitle",
        helpKey: "settings.maintenanceDeclaredActionsHelp",
        unavailableKey: "settings.maintenanceDeclaredActionsUnavailable",
        warningsTitleKey: "settings.maintenanceRuleWarningsTitle",
        warningLocationKey: "settings.maintenanceRuleWarningLocation",
      }}
      source={source}
      declared={
        declared ? (
          <MaintenanceDeclaredActionsSummary
            declared={declared}
            idPrefix="settings-maintenance-rule-declared-action"
          />
        ) : null
      }
      order={ruleOutputHeadNames(descriptors)}
      warnings={warnings}
      checked={checked}
    />
  );
}

// ── Request rules ─────────────────────────────────────────────────────

/// How request summaries format leases, monitoring and languages.
export function useRequestActionValueFormat(): RequestActionValueFormat {
  const t = useTranslate();
  const original = t("title.originalAudioLanguagePerTitle");
  return {
    leaseDays: (days) =>
      days === 0
        ? t("requests.leaseForever")
        : t("requests.shapingLeaseDays", { count: days }),
    monitor: (value) => {
      const key = requestMonitorTypeLabelKey(value);
      return key ? t(key) : value;
    },
    languages: (codes) => formatAudioLanguageLabels(codes, original),
  };
}

/// What a request rule's source declares it can do, in head order.
export function RequestDeclaredActionsSummary({
  declared,
  idPrefix,
}: {
  declared: RequestDeclaredActions;
  idPrefix: string;
}) {
  const t = useTranslate();
  const vocabulary = useActionHeadVocabulary("request");
  const entries = sortByRuleOutputOrder(
    vocabulary.descriptors,
    requestDeclaredActionEntries(declared, useRequestActionValueFormat()),
    (entry) => entry.head,
  );
  if (entries.length === 0) {
    return (
      <span
        id={selectorId(idPrefix, "none")}
        className="text-xs text-muted-foreground"
      >
        {t("settings.requestDeclaredActionsNone")}
      </span>
    );
  }
  return (
    <ActionEntryList
      entries={entries}
      vocabulary={vocabulary}
      idPrefix={idPrefix}
    />
  );
}

/// The request editor's findings panel.
export function RequestRuleSourceFindings({
  source,
  declared,
  warnings,
  checked,
}: {
  source: string;
  declared: RequestDeclaredActions | null | undefined;
  warnings: readonly RuleValidationDiagnostic[];
  checked: boolean;
}) {
  const descriptors = useRuleOutputDescriptors("request");
  return (
    <RuleSourceFindings
      vocabulary={{
        idPrefix: "settings-request-rule",
        titleKey: "settings.requestDeclaredActionsTitle",
        helpKey: "settings.requestDeclaredActionsHelp",
        unavailableKey: "settings.requestDeclaredActionsUnavailable",
        warningsTitleKey: "settings.requestRuleWarningsTitle",
        warningLocationKey: "settings.requestRuleWarningLocation",
      }}
      source={source}
      declared={
        declared ? (
          <RequestDeclaredActionsSummary
            declared={declared}
            idPrefix="settings-request-rule-declared-action"
          />
        ) : null
      }
      order={ruleOutputHeadNames(descriptors)}
      warnings={warnings}
      checked={checked}
    />
  );
}

/// What an approval would be changed to, and each head two rules disagreed
/// on, shown by its head through i18n. Renders nothing when there is neither.
export function RequestShapingSummary({
  shaping,
  idPrefix,
}: {
  shaping: RequestShaping | null | undefined;
  idPrefix: string;
}) {
  const t = useTranslate();
  const descriptors = useRuleOutputDescriptors("request");
  const entries = sortByRuleOutputOrder(
    descriptors,
    requestShapingEntries(shaping, useRequestActionValueFormat()),
    (entry) => entry.head,
  );
  const conflicts = sortByRuleOutputOrder(
    descriptors,
    requestShapingConflicts(shaping),
    (head) => head,
  );
  if (entries.length === 0 && conflicts.length === 0) return null;
  const headLabel = (head: string) => ruleOutputHeadLabel(t, descriptors, head);
  return (
    <div id={idPrefix} className="space-y-1">
      <p className="text-xs font-medium">{t("requests.shapingTitle")}</p>
      {entries.length > 0 ? (
        <ul className="space-y-0.5 text-xs">
          {entries.map((entry, index) => (
            <li
              key={`${entry.head}-${index}`}
              id={selectorId(idPrefix, entry.head, index)}
              data-request-shaping-head={entry.head}
            >
              <span className="font-medium">{headLabel(entry.head)}</span>:{" "}
              {entry.value}
              {entry.ruleSetName ? (
                <span className="text-muted-foreground">
                  {" "}
                  ({t("requests.shapingSetBy", { rule: entry.ruleSetName })})
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      {conflicts.map((head) => (
        <p
          key={head}
          id={selectorId(idPrefix, "conflict", head)}
          data-request-shaping-conflict={head}
          className="text-xs text-[var(--scry-warning-text)]"
        >
          {t("requests.shapingConflict", { head: headLabel(head) })}
        </p>
      ))}
    </div>
  );
}
