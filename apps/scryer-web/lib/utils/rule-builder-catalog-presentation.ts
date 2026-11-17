import type { RuleBuilderOptionValue } from "./rule-builder.ts";

/// How the builder presents a picker catalog it knows. A served output names
/// its catalog; a name missing here, or one the page cannot supply, is typed
/// in as free text within the output's own limits instead.
export type RuleBuilderCatalogPresentation = {
  placeholderKey: string;
  /// Shown in place of an empty picker.
  emptyKey?: string;
  /// How the picker sets out each value.
  display?: "text" | "path";
};

export const RULE_BUILDER_CATALOGS: Readonly<
  Record<string, RuleBuilderCatalogPresentation>
> = {
  qualityProfiles: {
    placeholderKey: "settings.ruleBuilderQualityProfilePlaceholder",
  },
  // Id catalogs a condition picks from, labelled by name.
  qualityProfileIds: {
    placeholderKey: "settings.ruleBuilderQualityProfilePlaceholder",
  },
  users: { placeholderKey: "settings.ruleBuilderUserPlaceholder" },
  lists: { placeholderKey: "settings.ruleBuilderListPlaceholder" },
  libraries: { placeholderKey: "settings.lifecycleBuilderLibraryPlaceholder" },
  roots: {
    placeholderKey: "settings.lifecycleBuilderRootPlaceholder",
    display: "path",
  },
  titleTags: {
    placeholderKey: "settings.ruleBuilderTagsPlaceholder",
    emptyKey: "settings.ruleBuilderNoTags",
  },
  monitorTypes: { placeholderKey: "settings.requestBuilderMonitorPlaceholder" },
  audioLanguages: {
    placeholderKey: "settings.requestBuilderLanguagesPlaceholder",
  },
};

/// The stored values a loaded catalog does not list. Nothing is reported while
/// the catalog is loading, or when it came back empty (which a failed load
/// also leaves), so a value is only called missing against a real list. The
/// values stay in the rule either way.
export function ruleBuilderCatalogMisses(
  names: readonly RuleBuilderOptionValue[] | null | undefined,
  values: readonly string[],
): string[] {
  if (!names || names.length === 0) return [];
  const known = new Set(
    names.map((name) => (typeof name === "string" ? name : name.value)),
  );
  return values.filter((value) => value !== "" && !known.has(value));
}
