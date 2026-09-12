import { facetById, type FacetDefinition } from "./registry.ts";

/**
 * Facet values as rule sets and post-processing scripts persist them. Delay
 * profiles persist the upper-case domain spelling instead (`FACET_OPTIONS` in
 * `lib/utils/delay-profiles`), so these helpers match case-insensitively and
 * echo back whichever spelling the caller passed in: one picker serves every
 * screen without rewriting stored values.
 */
export const LOWERCASE_FACET_IDS = ["movie", "series", "anime"] as const;

export type FacetSelectOption<T extends string> = {
  /** The caller's own spelling of the value, echoed back on change. */
  value: T;
  facet: FacetDefinition;
};

export function facetSelectOptions<T extends string>(
  values: readonly T[],
): FacetSelectOption<T>[] {
  return values.flatMap((value) => {
    const facet = facetById(value.trim().toUpperCase());
    return facet ? [{ value, facet }] : [];
  });
}

export function isFacetSelected<T extends string>(
  selected: readonly T[],
  value: T,
): boolean {
  return selected.some((entry) => entry.toLowerCase() === value.toLowerCase());
}

export function toggleFacetValue<T extends string>(
  selected: readonly T[],
  value: T,
  checked: boolean,
): T[] {
  const without = selected.filter(
    (entry) => entry.toLowerCase() !== value.toLowerCase(),
  );
  return checked ? [...without, value] : without;
}
