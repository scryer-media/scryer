import { regoCodeAt } from "./rego-lexical.ts";
export { regoCodeAt } from "./rego-lexical.ts";
import releaseContract from "../contracts/rule-input-contract.json" with { type: "json" };
import requestContract from "../contracts/request-input-contract.json" with { type: "json" };
import maintenanceContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import catalog from "../contracts/rego-assistance.json" with { type: "json" };
import type { RuleBuilderOutputDescriptor } from "./rule-builder.ts";
import {
  isRuleOutputInVariant,
  ruleOutputHeadHelp,
  ruleOutputHeadLabel,
  ruleOutputSnippet,
} from "./rule-output-vocabulary.ts";

export type RegoFamily = "release" | "request" | "maintenance";
export type RegoField = {
  path: string;
  type: string;
  descKey: string;
  availabilityKey?: string;
};
export const regoCatalog = catalog;

export type RegoHelper = {
  name: string;
  args: string[];
  returns: string;
  descKey: string;
};

/// Helpers the host writes into one kind of rule's package rather than
/// registering with the engine, so they exist only for that family.
const FAMILY_HELPERS: Record<RegoFamily, RegoHelper[]> = {
  release: [],
  request: [],
  maintenance: [
    {
      name: "days_since",
      args: ["ts"],
      returns: "number",
      descKey: "settings.regoHelper.daysSince",
    },
  ],
};

/// Every helper an author can call in a rule of this family.
export function regoHelpers(family: RegoFamily): RegoHelper[] {
  return [...catalog.helpers, ...FAMILY_HELPERS[family]];
}
/// The builder family whose served output descriptors describe a Rego
/// family's heads.
export const REGO_FAMILY_OUTPUTS: Record<RegoFamily, string> = {
  release: "scoring",
  request: "request",
  maintenance: "maintenance",
};

/// One head as the editor offers and explains it, built from the head's served
/// descriptor and its translation tags.
export type RegoHeadAssist = {
  /// The Rego head name, which is the completion label.
  head: string;
  /// The translated label, or the head when the tag has no translation.
  detail: string;
  /// The translated help; null when it has none, and then left out.
  documentation: string | null;
  /// The rule the completion inserts, in the editor's snippet syntax.
  snippet: string;
};

/// The heads a rule of this variant can define. Empty while the descriptors
/// are loading or after the read failed, so the editor then offers no heads
/// and works as it does for a family without head assistance.
export function regoHeadAssists(
  descriptors: readonly RuleBuilderOutputDescriptor[],
  t: (key: string) => string,
  variant: string | null | undefined,
): RegoHeadAssist[] {
  return descriptors
    .filter((descriptor) => isRuleOutputInVariant(descriptor, variant))
    .map((descriptor) => ({
      head: descriptor.head,
      detail: ruleOutputHeadLabel(t, descriptors, descriptor.id),
      documentation: ruleOutputHeadHelp(t, descriptor),
      snippet: ruleOutputSnippet(descriptor),
    }));
}

export const regoContracts = {
  release: releaseContract,
  request: requestContract,
  maintenance: maintenanceContract,
};

export function regoFields(family: RegoFamily): RegoField[] {
  const fields = new Map<string, RegoField>();
  for (const section of regoContracts[family].sections) {
    const parts = section.path.split(".");
    for (let i = 1; i < parts.length; i++) {
      const path = parts.slice(0, i + 1).join(".");
      if (!fields.has(path))
        fields.set(path, {
          path,
          type: path === "input.file" ? "object | null" : "object",
          descKey: section.titleKey,
          ...(family === "release" && path.startsWith("input.file")
            ? { availabilityKey: "settings.regoFileAvailability" }
            : {}),
        });
    }
    for (const field of section.fields) {
      const path = `${section.path}.${field.field}`;
      fields.set(path, {
        path,
        type: field.type,
        descKey: field.descKey,
        ...(family === "release" && path.startsWith("input.file")
          ? { availabilityKey: "settings.regoFileAvailability" }
          : {}),
        ...(path === "input.release.extra"
          ? { availabilityKey: "settings.regoExtraAvailability" }
          : {}),
      });
    }
  }
  return [...fields.values()];
}

/**
 * One bracketed index in a Rego path: `[0]`, `[_]`, `[ i ]` or `[ ]`. The
 * trailing whitespace belongs to the optional index so blank brackets split
 * their whitespace only one way; a run of `[ ]` would otherwise backtrack
 * exponentially while the user types.
 */
export const REGO_INDEX_SOURCE = String.raw`\[\s*(?:(?:\d+|[A-Za-z_]\w*)\s*)?\]`;

const REGO_INDEX_GLOBAL = new RegExp(REGO_INDEX_SOURCE, "g");
const REGO_COMPLETION_PATH = new RegExp(
  String.raw`(?:\b(?:input|scryer|object|strings|array|regex)\b(?:\.[A-Za-z_]\w*|${REGO_INDEX_SOURCE})*\.)?(?:[A-Za-z_]\w*)?$`,
);

export function normalizeRegoPath(path: string): string {
  return path.replace(REGO_INDEX_GLOBAL, "[]");
}

export function regoCompletionContext(source: string, position: number) {
  if (!regoCodeAt(source, position)) return null;
  const before = source.slice(0, position);
  const match = before.match(REGO_COMPLETION_PATH);
  if (!match) return null;
  const text = match[0];
  const dot = text.lastIndexOf(".");
  // Don't offer root names for an unsupported path (for example an alias).
  if (/[.\w\]]$/.test(before.slice(0, before.length - text.length)))
    return null;
  return {
    from: position - (text.length - dot - 1),
    parent: dot < 0 ? "" : normalizeRegoPath(text.slice(0, dot)),
    prefix: text.slice(dot + 1),
  };
}

export function regoFieldCompletions(
  family: RegoFamily,
  parent: string,
): RegoField[] {
  return regoFields(family).filter(
    ({ path }) =>
      path.slice(0, path.lastIndexOf(".")) === parent &&
      !path.slice(path.lastIndexOf(".") + 1).includes("[]"),
  );
}
