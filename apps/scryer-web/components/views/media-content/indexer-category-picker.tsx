import * as React from "react";
import { createPortal } from "react-dom";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronDown } from "lucide-react";

export type ViewCategoryId = "movie" | "series" | "anime";

export type IndexerCategoryDefinition = {
  code: string;
  labelKey: keyof typeof indexerCategoryLabelMap;
};

export type IndexerCategoryGroupDefinition = {
  labelKey: keyof typeof indexerCategoryGroupLabelMap;
  code: string;
  categories: IndexerCategoryDefinition[];
};

export const indexerCategoryGroupLabelMap = {
  tv: "settings.indexerCategoryTv",
  movies: "settings.indexerCategoryMovies",
  other: "settings.indexerCategoryOther",
  foreign: "settings.indexerCategoryForeign",
  sd: "settings.indexerCategorySd",
  hd: "settings.indexerCategoryHd",
  uhd: "settings.indexerCategoryUhd",
  anime: "settings.indexerCategoryAnime",
  documentary: "settings.indexerCategoryDocumentary",
  sport: "settings.indexerCategorySport",
  bluRay: "settings.indexerCategoryBluRay",
  threeD: "settings.indexerCategoryThreeD",
  misc: "settings.indexerCategoryMisc",
} as const;

export const indexerCategoryLabelMap: Record<string, string> = {
  "5020": "settings.indexerCategoryForeign",
  "5030": "settings.indexerCategorySd",
  "5040": "settings.indexerCategoryHd",
  "5045": "settings.indexerCategoryUhd",
  "5050": "settings.indexerCategoryOther",
  "5060": "settings.indexerCategorySport",
  "5070": "settings.indexerCategoryAnime",
  "5080": "settings.indexerCategoryDocumentary",
  "8010": "settings.indexerCategoryMisc",
  "2010": "settings.indexerCategoryForeign",
  "2020": "settings.indexerCategoryOther",
  "2030": "settings.indexerCategorySd",
  "2040": "settings.indexerCategoryHd",
  "2045": "settings.indexerCategoryUhd",
  "2050": "settings.indexerCategoryBluRay",
  "2060": "settings.indexerCategoryThreeD",
};

export const INDEXER_CATEGORY_DEFINITIONS: Record<string, IndexerCategoryGroupDefinition> = {
  tv: {
    labelKey: "tv",
    code: "5000",
    categories: [
      { code: "5020", labelKey: "settings.indexerCategoryForeign" },
      { code: "5030", labelKey: "settings.indexerCategorySd" },
      { code: "5040", labelKey: "settings.indexerCategoryHd" },
      { code: "5045", labelKey: "settings.indexerCategoryUhd" },
      { code: "5050", labelKey: "settings.indexerCategoryOther" },
      { code: "5060", labelKey: "settings.indexerCategorySport" },
      { code: "5070", labelKey: "settings.indexerCategoryAnime" },
      { code: "5080", labelKey: "settings.indexerCategoryDocumentary" },
    ],
  },
  movies: {
    labelKey: "movies",
    code: "2000",
    categories: [
      { code: "2010", labelKey: "settings.indexerCategoryForeign" },
      { code: "2020", labelKey: "settings.indexerCategoryOther" },
      { code: "2030", labelKey: "settings.indexerCategorySd" },
      { code: "2040", labelKey: "settings.indexerCategoryHd" },
      { code: "2045", labelKey: "settings.indexerCategoryUhd" },
      { code: "2050", labelKey: "settings.indexerCategoryBluRay" },
      { code: "2060", labelKey: "settings.indexerCategoryThreeD" },
    ],
  },
  other: {
    labelKey: "other",
    code: "8000",
    categories: [
      { code: "8010", labelKey: "settings.indexerCategoryMisc" },
    ],
  },
};

export const INDEXER_CATEGORY_GROUPS_BY_SCOPE: Record<
  ViewCategoryId,
  Array<"movies" | "tv" | "other">
> = {
  movie: ["movies", "other"],
  series: ["tv", "other"],
  anime: ["tv", "other"],
};

export function sortCategoryCodes(values: string[]): string[] {
  return [...values].sort((left, right) => {
    const leftNumber = Number.parseInt(left, 10);
    const rightNumber = Number.parseInt(right, 10);

    if (Number.isNaN(leftNumber) || Number.isNaN(rightNumber)) {
      if (Number.isNaN(leftNumber) && Number.isNaN(rightNumber)) {
        return left.localeCompare(right, undefined, { numeric: true, sensitivity: "base" });
      }
      return Number.isNaN(leftNumber) ? 1 : -1;
    }

    return leftNumber === rightNumber
      ? left.localeCompare(right, undefined, { numeric: true, sensitivity: "base" })
      : leftNumber - rightNumber;
  });
}

export function normalizeCategoryCodes(values: string[]): string[] {
  const seen = new Set<string>();
  const next: string[] = [];
  for (const value of values) {
    const normalized = value.trim();
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    next.push(normalized);
  }
  return next;
}

function formatTagsInput(tags: string[]): string {
  return tags.join(", ");
}

export function formatCategoryCodeList(codes: string[]): string {
  return formatTagsInput(sortCategoryCodes(normalizeCategoryCodes(codes)));
}

export function getSortedCategoryCodesByScope(scope: ViewCategoryId, values: string[]) {
  const normalized = normalizeCategoryCodes(values);
  const scopeGroups = INDEXER_CATEGORY_GROUPS_BY_SCOPE[scope]
    .map((groupKey) => {
      const group = INDEXER_CATEGORY_DEFINITIONS[groupKey];
      return [group.code, ...group.categories.map((category) => category.code)];
    })
    .flat();
  const orderedKnownCodes = scopeGroups.filter((code) => normalized.includes(code));
  const unknownCodes = normalized.filter((code) => !scopeGroups.includes(code));
  return [...orderedKnownCodes, ...unknownCodes];
}

type IndexerCategoryPickerProps = {
  value: string[];
  scope: ViewCategoryId;
  disabled: boolean;
  categoriesLabel?: string;
  onChange: (categories: string[]) => void;
};

export const IndexerCategoryPicker = React.memo(function IndexerCategoryPicker({
  value,
  scope,
  disabled,
  onChange,
  categoriesLabel,
}: IndexerCategoryPickerProps) {
  const t = useTranslate();
  const pickerRef = React.useRef<HTMLDivElement>(null);
  const floatingPanelRef = React.useRef<HTMLDivElement>(null);
  const [isOpen, setIsOpen] = React.useState(false);
  const [draftCategories, setDraftCategories] = React.useState<string[]>(() =>
    getSortedCategoryCodesByScope(scope, value),
  );
  const [pickerRect, setPickerRect] = React.useState<DOMRect | null>(null);

  React.useEffect(() => {
    if (!isOpen) {
      setDraftCategories(getSortedCategoryCodesByScope(scope, value));
      return;
    }
    if (pickerRef.current && typeof window !== "undefined") {
      setPickerRect(pickerRef.current.getBoundingClientRect());
    }
  }, [isOpen, scope, value]);

  React.useEffect(() => {
    if (!isOpen) {
      return;
    }
    const handlePointerDown = (event: MouseEvent) => {
      if (!pickerRef.current) {
        return;
      }
      if (
        !pickerRef.current?.contains(event.target as Node) &&
        !floatingPanelRef.current?.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };

    const handleScrollOrResize = () => {
      if (!pickerRef.current || typeof window === "undefined") {
        return;
      }
      setPickerRect(pickerRef.current.getBoundingClientRect());
    };

    document.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("scroll", handleScrollOrResize, true);
    window.addEventListener("resize", handleScrollOrResize, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("scroll", handleScrollOrResize, true);
      window.removeEventListener("resize", handleScrollOrResize, true);
    };
  }, [isOpen]);

  const selectedSet = React.useMemo(
    () => new Set<string>(normalizeCategoryCodes(draftCategories)),
    [draftCategories],
  );

  const toggleCategory = (code: string) => {
    setDraftCategories((previous) => {
      const current = new Set(previous.map((entry) => entry.trim()));
      if (current.has(code)) {
        current.delete(code);
      } else {
        current.add(code);
      }
      const sorted = getSortedCategoryCodesByScope(scope, Array.from(current));
      onChange(sorted);
      return sorted;
    });
  };

  const toggleCategoryHeader = (groupKey: "movies" | "tv" | "other") => {
    const groupCode = INDEXER_CATEGORY_DEFINITIONS[groupKey].code;
    setDraftCategories((previous) => {
      const next = new Set(previous.map((entry) => entry.trim()));
      if (next.has(groupCode)) {
        next.delete(groupCode);
      } else {
        next.add(groupCode);
      }
      const sorted = getSortedCategoryCodesByScope(scope, Array.from(next));
      onChange(sorted);
      return sorted;
    });
  };

  const floatingPanel = isOpen && pickerRect && !disabled
    ? createPortal(
        <div
          ref={floatingPanelRef}
          className="z-50 max-h-80 overflow-y-auto rounded-xl border border-border bg-popover p-2 shadow-lg"
          style={{
            position: "fixed",
            top: pickerRect.bottom + 4,
            left: pickerRect.left,
            width: Math.max(260, Math.round(pickerRect.width)),
          }}
        >
          {INDEXER_CATEGORY_GROUPS_BY_SCOPE[scope].map((groupKey) => {
            const group = INDEXER_CATEGORY_DEFINITIONS[groupKey];
            return (
              <div key={groupKey} className="mb-2 last:mb-0">
                <label className="mb-1 flex items-center justify-between rounded-md px-2 py-1 text-sm capitalize text-foreground hover:bg-accent/60">
                  <span className="flex items-center gap-2">
                    <Checkbox
                      checked={selectedSet.has(group.code)}
                      onCheckedChange={() => toggleCategoryHeader(groupKey)}
                      aria-label={`${t("indexerCategory.labelCategory")}: ${t(group.labelKey)} ${group.code}`}
                      disabled={disabled}
                    />
                    {t(group.labelKey)}
                  </span>
                  <span className="text-xs font-mono text-muted-foreground">{group.code}</span>
                </label>
                <div className="space-y-1 pl-3">
                  {group.categories.map((category) => (
                    <label
                      key={category.code}
                      className="flex items-center justify-between gap-3 rounded-md px-2 py-1 text-sm capitalize text-foreground hover:bg-accent/60"
                    >
                      <span className="flex items-center gap-2 text-foreground">
                        <Checkbox
                          checked={selectedSet.has(category.code)}
                          onCheckedChange={() => toggleCategory(category.code)}
                          aria-label={`${t("indexerCategory.labelCategory")}: ${t(category.labelKey)} ${category.code}`}
                          disabled={disabled}
                        />
                        {t(category.labelKey)}
                      </span>
                      <span className="text-xs font-mono text-muted-foreground">{category.code}</span>
                    </label>
                  ))}
                </div>
              </div>
            );
          })}
        </div>,
        document.body,
      )
    : null;

  return (
    <div ref={pickerRef} className="relative inline-block w-full">
      <Button
        type="button"
        variant="secondary"
        className="h-auto w-full justify-between gap-2 border border-input bg-field px-3 py-2 text-sm"
        onClick={() => setIsOpen((previous) => !previous)}
        disabled={disabled}
        aria-label={categoriesLabel || t("settings.indexerRoutingCategories")}
      >
        <span
          className={`truncate text-left ${formatCategoryCodeList(draftCategories) ? "text-card-foreground" : "text-muted-foreground"}`}
        >
          {formatCategoryCodeList(draftCategories) || t("settings.indexerRoutingCategoriesPlaceholder")}
        </span>
        <ChevronDown className={`h-4 w-4 transition-transform ${isOpen ? "rotate-180" : ""}`} />
      </Button>
      {floatingPanel}
    </div>
  );
});
