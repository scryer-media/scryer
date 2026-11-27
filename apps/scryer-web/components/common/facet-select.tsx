import * as React from "react";
import { Checkbox as CheckboxPrimitive } from "radix-ui";

import { useTranslate } from "@/lib/context/translate-context";
import { facetById } from "@/lib/facets/registry";
import { useFacets } from "@/lib/facets/use-facets";
import { facetStyle } from "@/lib/facets/style";
import {
  facetSelectOptions,
  isFacetSelected,
  toggleFacetValue,
} from "@/lib/facets/selection";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

const CHIP_CLASS =
  "inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm font-medium leading-none transition-colors outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] disabled:cursor-not-allowed disabled:opacity-50";

const UNCHECKED_CHIP_CLASS =
  "border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-muted2)] hover:border-[var(--scry-border2)] hover:text-[var(--scry-ink2)]";

/**
 * "Applies to" pickers across settings select the same three facets, so they
 * wear the same identity here as everywhere else: the registry's icon and the
 * facet's own colour when on, a plain outline when off. A checked chip carries
 * its colour instead of the generic accent tick, which is what tells the three
 * options apart at a glance.
 */
type FacetSelectProps<T extends string> = {
  /** Option values in the caller's own stored spelling. */
  values: readonly T[];
  selected: readonly T[];
  onChange: (next: T[]) => void;
  /** Prefix for each chip's selector id, e.g. "settings-rule-facet". */
  idPrefix: string;
  disabled?: boolean;
  className?: string;
};

export function FacetSelect<T extends string>({
  values,
  selected,
  onChange,
  idPrefix,
  disabled,
  className,
}: FacetSelectProps<T>) {
  const t = useTranslate();
  // A hidden facet is not offered, unless the saved value already selects it:
  // that chip stays so the selection can be seen and cleared.
  const { values: visibleFacetValues } = useFacets();
  const offered = React.useMemo(
    () => visibleFacetValues(values, selected),
    [selected, values, visibleFacetValues],
  );
  const options = React.useMemo(() => facetSelectOptions(offered), [offered]);

  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      {options.map(({ value, facet }) => {
        const Icon = facet.icon;
        const checked = isFacetSelected(selected, value);
        const style = facetStyle(facet.id);
        return (
          <CheckboxPrimitive.Root
            key={value}
            id={selectorId(idPrefix, value)}
            checked={checked}
            disabled={disabled}
            onCheckedChange={(next) =>
              onChange(toggleFacetValue(selected, value, next === true))
            }
            className={cn(
              CHIP_CLASS,
              checked
                ? "shadow-[inset_0_1px_0_rgba(255,255,255,0.06)]"
                : UNCHECKED_CHIP_CLASS,
            )}
            style={
              checked
                ? {
                    background: style.bg,
                    borderColor: style.border,
                    color: style.text,
                  }
                : undefined
            }
          >
            <Icon
              className={cn("size-4 shrink-0", !checked && "opacity-70")}
              style={{ color: style.dot }}
              aria-hidden
            />
            {t(facet.navLabelKey)}
          </CheckboxPrimitive.Root>
        );
      })}
    </div>
  );
}

type LibrarySelectProps = {
  libraries: readonly { id: string; name: string; facet: string }[];
  selected: readonly string[];
  onChange: (next: string[]) => void;
  /** Prefix for each chip's selector id, e.g. "settings-request-rule-library". */
  idPrefix: string;
  /** Shown when there are no libraries to pick, i.e. the rule applies to all. */
  emptyLabel: React.ReactNode;
  disabled?: boolean;
  className?: string;
};

/**
 * The same chips as {@link FacetSelect}, one per library, each wearing its
 * library's facet icon and colour. Rules scoped to libraries pick with this, so
 * every rule surface scopes with one control.
 */
export function LibrarySelect({
  libraries,
  selected,
  onChange,
  idPrefix,
  emptyLabel,
  disabled,
  className,
}: LibrarySelectProps) {
  if (libraries.length === 0) {
    return <p className="text-xs text-muted-foreground">{emptyLabel}</p>;
  }
  return (
    <div className={cn("flex flex-wrap items-center gap-2", className)}>
      {libraries.map((library) => {
        const facet = facetById(library.facet.trim().toUpperCase());
        const Icon = facet?.icon;
        const checked = selected.includes(library.id);
        const style = facet ? facetStyle(facet.id) : null;
        return (
          <CheckboxPrimitive.Root
            key={library.id}
            id={selectorId(idPrefix, library.id)}
            checked={checked}
            disabled={disabled}
            onCheckedChange={(next) =>
              onChange(
                next === true
                  ? [...selected.filter((id) => id !== library.id), library.id]
                  : selected.filter((id) => id !== library.id),
              )
            }
            className={cn(
              CHIP_CLASS,
              checked
                ? "shadow-[inset_0_1px_0_rgba(255,255,255,0.06)]"
                : UNCHECKED_CHIP_CLASS,
            )}
            style={
              checked && style
                ? {
                    background: style.bg,
                    borderColor: style.border,
                    color: style.text,
                  }
                : undefined
            }
          >
            {Icon ? (
              <Icon
                className={cn("size-4 shrink-0", !checked && "opacity-70")}
                style={{ color: style?.dot }}
                aria-hidden
              />
            ) : null}
            {library.name}
          </CheckboxPrimitive.Root>
        );
      })}
    </div>
  );
}

type FacetTagsProps = {
  values: readonly string[];
  /** Rendered when nothing is selected, i.e. the row applies to everything. */
  emptyLabel: React.ReactNode;
  className?: string;
};

/** Read-only echo of a {@link FacetSelect}, for tables and summaries. */
export function FacetTags({ values, emptyLabel, className }: FacetTagsProps) {
  const t = useTranslate();
  const options = React.useMemo(() => facetSelectOptions(values), [values]);

  if (options.length === 0) {
    return (
      <span
        className={cn(
          "inline-flex items-center rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-xs text-[var(--scry-muted2)]",
          className,
        )}
      >
        {emptyLabel}
      </span>
    );
  }

  return (
    <div className={cn("flex flex-wrap items-center gap-1", className)}>
      {options.map(({ value, facet }) => {
        const Icon = facet.icon;
        const style = facetStyle(facet.id);
        return (
          <span
            key={value}
            className="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium leading-5"
            style={{
              background: style.bg,
              borderColor: style.border,
              color: style.text,
            }}
          >
            <Icon
              className="size-3 shrink-0"
              style={{ color: style.dot }}
              aria-hidden
            />
            {t(facet.navLabelKey)}
          </span>
        );
      })}
    </div>
  );
}
