import * as React from "react";

import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { localizedFacetLabel } from "@/components/views/overview-localization";
import { useTranslate } from "@/lib/context/translate-context";
import { useFacets } from "@/lib/facets/use-facets";
import { cn } from "@/lib/utils";

// eslint-disable-next-line no-restricted-syntax -- orders the libraries the user has; a library is a record, not a facet choice
const FACET_ORDER = ["MOVIE", "SERIES", "ANIME"];

type LibraryOption = { id: string; name: string; facet: string };

type LibrarySingleSelectProps = {
  id: string;
  label?: React.ReactNode;
  placeholder?: string;
  libraries: readonly LibraryOption[];
  value: string;
  onValueChange: (libraryId: string) => void;
  disabled?: boolean;
  className?: string;
};

/**
 * Picks exactly one library, grouped under its facet the way
 * `LibraryMultiSelect` lists them, for forms that act on a single library.
 */
export function LibrarySingleSelect({
  id,
  label,
  placeholder,
  libraries,
  value,
  onValueChange,
  disabled = false,
  className,
}: LibrarySingleSelectProps) {
  const t = useTranslate();
  // A hidden facet's libraries are not offered, unless one is the value.
  const { isVisible } = useFacets();
  const groups = React.useMemo(() => {
    const byFacet = new Map<string, LibraryOption[]>();
    for (const library of libraries) {
      if (!isVisible(library.facet) && library.id !== value) continue;
      const facet = library.facet.trim().toUpperCase();
      const group = byFacet.get(facet);
      if (group) group.push(library);
      else byFacet.set(facet, [library]);
    }
    const rank = (facet: string) => {
      const at = FACET_ORDER.indexOf(facet);
      return at < 0 ? FACET_ORDER.length : at;
    };
    return [...byFacet.entries()]
      .sort(([a], [b]) => rank(a) - rank(b))
      .map(([facet, members]) => ({ facet, libraries: members }));
  }, [isVisible, libraries, value]);
  const showFacetGroups = groups.length > 1;
  const isDisabled = disabled || libraries.length === 0;

  return (
    <div className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      {label ? (
        <label
          htmlFor={id}
          className={cn(
            "block text-sm font-medium text-[var(--scry-ink2)]",
            isDisabled && "opacity-60",
          )}
        >
          {label}
        </label>
      ) : null}
      <Select value={value} onValueChange={onValueChange} disabled={isDisabled}>
        <SelectTrigger id={id} className="w-full">
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent>
          {groups.map((group) => (
            <SelectGroup key={group.facet}>
              {showFacetGroups ? (
                <SelectLabel>{localizedFacetLabel(t, group.facet)}</SelectLabel>
              ) : null}
              {group.libraries.map((library) => (
                <SelectItem key={library.id} value={library.id}>
                  {library.name}
                </SelectItem>
              ))}
            </SelectGroup>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
