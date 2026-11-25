import {
  ERROR_REASON_KEYS,
  type ErrorTranslate,
} from "../graphql/error-message.ts";
import type {
  ExternalRuleImportFact,
  ExternalRuleImportItem,
} from "../types/external-import.ts";

/** Translator diagnostic codes on Maintainerr import items, and their copy. */
export const MAINTAINERR_DIAGNOSTIC_KEYS: Readonly<Record<string, string>> = {
  "maintainerr.group.invalid": "settings.maintainerrImportCode.groupInvalid",
  "maintainerr.group.rules_disabled":
    "settings.maintainerrImportCode.groupRulesDisabled",
  "maintainerr.group.rules_empty":
    "settings.maintainerrImportCode.groupRulesEmpty",
  "maintainerr.group.rules_too_many":
    "settings.maintainerrImportCode.groupRulesTooMany",
  "maintainerr.group.media_type_unsupported":
    "settings.maintainerrImportCode.groupMediaTypeUnsupported",
  "maintainerr.rule.invalid": "settings.maintainerrImportCode.ruleInvalid",
  "maintainerr.rule.second_value_missing":
    "settings.maintainerrImportCode.ruleSecondValueMissing",
  "maintainerr.rule.username_missing":
    "settings.maintainerrImportCode.ruleUsernameMissing",
  "maintainerr.application.unknown":
    "settings.maintainerrImportCode.applicationUnknown",
  "maintainerr.property.unknown":
    "settings.maintainerrImportCode.propertyUnknown",
  "maintainerr.property.unmapped":
    "settings.maintainerrImportCode.propertyUnmapped",
  "maintainerr.property.collection_unsupported":
    "settings.maintainerrImportCode.propertyCollectionUnsupported",
  "maintainerr.property.level_unsupported":
    "settings.maintainerrImportCode.propertyLevelUnsupported",
  "maintainerr.comparator.unsupported":
    "settings.maintainerrImportCode.comparatorUnsupported",
  "maintainerr.comparator.date_equality_unsupported":
    "settings.maintainerrImportCode.comparatorDateEqualityUnsupported",
  "maintainerr.comparator.partial_list_widens":
    "settings.maintainerrImportCode.comparatorPartialListWidens",
  "maintainerr.comparator.absence_widens":
    "settings.maintainerrImportCode.comparatorAbsenceWidens",
  "maintainerr.comparator.date_drift_widens":
    "settings.maintainerrImportCode.comparatorDateDriftWidens",
  "maintainerr.username.unknown_widens":
    "settings.maintainerrImportCode.usernameUnknownWidens",
  "maintainerr.scope.all_libraries":
    "settings.maintainerrImportCode.scopeAllLibraries",
  "maintainerr.custom_value.invalid":
    "settings.maintainerrImportCode.customValueInvalid",
  "maintainerr.action.missing": "settings.maintainerrImportCode.actionMissing",
  "maintainerr.action.unsupported":
    "settings.maintainerrImportCode.actionUnsupported",
  "maintainerr.quality_profile.unresolved":
    "settings.maintainerrImportCode.qualityProfileUnresolved",
  "maintainerr.quality_profile.not_in_scryer":
    "settings.maintainerrImportCode.qualityProfileNotInScryer",
  "maintainerr.source.too_large":
    "settings.maintainerrImportCode.sourceTooLarge",
  "maintainerr.source.invalid": "settings.maintainerrImportCode.sourceInvalid",
  "maintainerr.username.unknown":
    "settings.maintainerrImportCode.usernameUnknown",
  "maintainerr.fact.approximate":
    "settings.maintainerrImportCode.factApproximate",
  "maintainerr.action.approximate":
    "settings.maintainerrImportCode.actionApproximate",
};

/** One translator finding on a Maintainerr import item. */
export type MaintainerrDiagnostic = {
  code: string;
  severity: "ERROR" | "WARNING";
  /** Zero-based position of the Maintainerr rule in its group, when one. */
  ruleIndex: number | null;
  message: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function diagnosticOf(fact: ExternalRuleImportFact): MaintainerrDiagnostic[] {
  if (!isRecord(fact.details)) return [];
  const { severity, ruleIndex } = fact.details;
  if (severity !== "ERROR" && severity !== "WARNING") return [];
  return [
    {
      code: fact.code,
      severity,
      ruleIndex:
        typeof ruleIndex === "number" &&
        Number.isSafeInteger(ruleIndex) &&
        ruleIndex >= 0
          ? ruleIndex
          : null,
      message: fact.message,
    },
  ];
}

/**
 * Every translator finding on an item, errors first, each in report order.
 * A fact that only restates the item's own reason carries no severity and is
 * left to the item's reason line.
 */
export function maintainerrDiagnostics(
  item: ExternalRuleImportItem,
): MaintainerrDiagnostic[] {
  const diagnostics = item.facts.flatMap(diagnosticOf);
  return [
    ...diagnostics.filter((diagnostic) => diagnostic.severity === "ERROR"),
    ...diagnostics.filter((diagnostic) => diagnostic.severity === "WARNING"),
  ];
}

/**
 * Display copy for a diagnostic. The code decides the copy; the server
 * sentence is shown only for a code this client does not know.
 */
export function maintainerrDiagnosticMessage(
  diagnostic: Pick<MaintainerrDiagnostic, "code" | "message">,
  translate: ErrorTranslate,
): string {
  const key =
    MAINTAINERR_DIAGNOSTIC_KEYS[diagnostic.code] ??
    ERROR_REASON_KEYS[diagnostic.code];
  if (key) return translate(key);
  return diagnostic.message.trim() || diagnostic.code;
}
