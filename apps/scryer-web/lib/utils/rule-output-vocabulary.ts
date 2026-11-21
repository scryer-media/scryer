import {
  translatedOr,
  type RuleBuilderOutputDescriptor,
} from "./rule-builder.ts";

/// Everything a summary, reference table or editor says about a rule
/// family's heads comes from the descriptors the server serves, keyed by the
/// head's id (`DELETE_FILES`) as the API names it. A head the descriptors do
/// not list, including every head while they are still loading, is shown by
/// its id and treated as neither ordered nor destructive.

type Descriptors = readonly RuleBuilderOutputDescriptor[];

export function ruleOutputDescriptor(
  descriptors: Descriptors,
  id: string,
): RuleBuilderOutputDescriptor | undefined {
  return descriptors.find((descriptor) => descriptor.id === id);
}

/// A head's label in the UI language: its translation tag, then its Rego
/// head name, then the id itself.
export function ruleOutputHeadLabel(
  t: (key: string) => string,
  descriptors: Descriptors,
  id: string,
): string {
  const descriptor = ruleOutputDescriptor(descriptors, id);
  return descriptor
    ? translatedOr(t, descriptor.labelKey, descriptor.head)
    : id;
}

/// A head's help in the UI language, or null when it has none translated.
export function ruleOutputHeadHelp(
  t: (key: string) => string,
  descriptor: Pick<RuleBuilderOutputDescriptor, "helpKey">,
): string | null {
  if (!descriptor.helpKey) return null;
  return translatedOr(t, descriptor.helpKey, "") || null;
}

export function isRuleOutputDestructive(
  descriptors: Descriptors,
  id: string,
): boolean {
  return ruleOutputDescriptor(descriptors, id)?.destructive === true;
}

/// Items in the order the descriptors list their heads, which is the order the
/// server applies them in. Heads it does not list go last, in the order given.
export function sortByRuleOutputOrder<T>(
  descriptors: Descriptors,
  items: readonly T[],
  idOf: (item: T) => string,
): T[] {
  const rank = new Map(
    descriptors.map((descriptor, at) => [descriptor.id, at]),
  );
  const last = descriptors.length;
  return items
    .map((item, at) => ({ item, at, rank: rank.get(idOf(item)) ?? last }))
    .sort((left, right) => left.rank - right.rank || left.at - right.at)
    .map(({ item }) => item);
}

/// Whether a head is offered for a rule of this variant. A null variant, or a
/// descriptor without variants, offers every head.
export function isRuleOutputInVariant(
  descriptor: Pick<RuleBuilderOutputDescriptor, "variants">,
  variant: string | null | undefined,
): boolean {
  return (
    !variant || !descriptor.variants || descriptor.variants.includes(variant)
  );
}

/// The Rego head names in apply order, for help text that lists them.
export function ruleOutputHeadNames(descriptors: Descriptors): string {
  return descriptors.map((descriptor) => descriptor.head).join(", ");
}

/// The example literal for a descriptor's value in reference text.
function sampleValue(descriptor: RuleBuilderOutputDescriptor): string {
  if (descriptor.valueKind === "NUMBER") return String(descriptor.min ?? 0);
  return '"..."';
}

/// How a head is written in source, from its template: the helper becomes
/// `{ ... }` and each value an example literal.
export function ruleOutputSyntax(
  descriptor: RuleBuilderOutputDescriptor,
): string {
  const sample = sampleValue(descriptor);
  return descriptor.template.replace(
    /\{(head|helper|value|items|item|part\.[\w]+)\}/g,
    (_, token: string) => {
      if (token === "head") return descriptor.head;
      if (token === "helper") return "{ ... }";
      if (token.startsWith("part.")) return token.slice(5);
      return sample;
    },
  );
}

/// A completion snippet for a head, from its template, in the editor's snippet
/// syntax. Each value is a tab stop and the helper becomes a body holding one
/// placeholder condition, so the inserted rule is valid Rego as it stands.
export function ruleOutputSnippet(
  descriptor: RuleBuilderOutputDescriptor,
): string {
  let stop = 0;
  const field = (placeholder: string) => `\${${++stop}:${placeholder}}`;
  return descriptor.template.replace(
    /\{(head|helper|value|items|item|part\.[\w]+)\}/g,
    (_, token: string) => {
      if (token === "head") return descriptor.head;
      if (token === "helper") return `{\n\t${field("true")}\n}`;
      if (token.startsWith("part.")) return field(token.slice(5));
      if (descriptor.valueKind === "NUMBER")
        return field(String(descriptor.min ?? 0));
      return `"${field("")}"`;
    },
  );
}

/// The ids among `known` that a shipped template's source defines, in apply
/// order. Only for the templates this build ships, whose rule heads start a
/// line; a rule the operator wrote is read by the server's validation instead.
export function templateRuleOutputs(
  source: string,
  descriptors: Descriptors,
): string[] {
  const byHead = new Map(
    descriptors.map((descriptor) => [descriptor.head, descriptor.id]),
  );
  const ids = new Set<string>();
  for (const match of source.matchAll(/^([a-z_]+)\b/gm)) {
    const id = byHead.get(match[1]);
    if (id) ids.add(id);
  }
  return sortByRuleOutputOrder(descriptors, [...ids], (id) => id);
}
