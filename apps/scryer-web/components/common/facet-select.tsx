import * as React from "react";
import { Checkbox as CheckboxPrimitive } from "radix-ui";

import { useTranslate } from "@/lib/context/translate-context";
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
  const options = React.useMemo(() => facetSelectOptions(values), [values]);

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
                : "border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-muted2)] hover:border-[var(--scry-border2)] hover:text-[var(--scry-ink2)]",
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
