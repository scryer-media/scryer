import * as React from "react";
import * as ToggleGroupPrimitive from "@radix-ui/react-toggle-group";
import { Input, decimalInputProps } from "@/components/ui/input";
import { CheckboxField } from "@/components/ui/checkbox";
import { Button } from "@/components/ui/button";
import { MultiSelectOptionList } from "@/components/ui/multi-select-dropdown";
import { selectContentClassName, selectTriggerClassName } from "@/components/ui/select";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ChevronDown } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { useCanonicalVocabulary } from "@/lib/hooks/use-canonical-vocabulary";
import type { ListFilter } from "@/lib/types/lists";
import type { Facet } from "@/lib/types/titles";
import { EMPTY_LIST_FILTER } from "@/lib/utils/lists";
import { ratingSourceInfo } from "@/lib/utils/title-ratings";
import {
  listRatingInputText,
  parseListRatingInput,
} from "@/lib/utils/list-rating-input";

const SOURCES = [
  "imdb",
  "tomatoes",
  "audience",
  "metacritic",
  "mcuser",
  "letterboxd",
  "tmdb",
  "tvdb",
  "trakt",
  "mal",
  "anilist",
  "anidb",
  "mdblist",
];
const SCALES: Record<string, number> = {
  tomatoes: 100,
  audience: 100,
  metacritic: 100,
  letterboxd: 5,
  anilist: 100,
  mdblist: 100,
};

export function ListFacetFilters({
  facet,
  filters,
  onChange,
  disabled,
  idPrefix,
}: {
  facet: Facet;
  filters: ListFilter[];
  onChange: (filters: ListFilter[]) => void;
  disabled?: boolean;
  idPrefix: string;
}) {
  const t = useTranslate();
  const { vocabulary, error, loading, retry } = useCanonicalVocabulary();
  const [search, setSearch] = React.useState("");
  const [ratingDrafts, setRatingDrafts] = React.useState<
    Record<string, string>
  >({});
  const ratings = filters.filter(
    (filter) => filter.kind === "RATINGS" && filter.facet === facet,
  );
  const legacyRatings = filters
    .filter((filter) => filter.kind === "RATING_AT_LEAST")
    .map((filter) => ({
      source: filter.scale ?? "tmdb",
      value: filter.value ?? 0,
    }));
  const minimums = [
    ...ratings.flatMap((filter) => filter.minimums ?? []),
    ...legacyRatings,
  ];
  const exclusions = filters.filter(
    (filter) =>
      filter.kind === "EXCLUDE_CANONICAL_TAGS" && filter.facet === facet,
  );
  const resolveLabels = (labels: string[]) => {
    const values: string[] = [];
    const unresolvedLabels: string[] = [];
    for (const label of labels) {
      const matches =
        vocabulary?.entries.filter((entry) =>
          [entry.name, ...entry.aliases].some(
            (name) => name.toLowerCase() === label.trim().toLowerCase(),
          ),
        ) ?? [];
      if (matches.length === 1) values.push(matches[0].key);
      else unresolvedLabels.push(label);
    }
    return { values, unresolvedLabels };
  };
  const converted = resolveLabels([
    ...exclusions.flatMap((filter) => filter.unresolvedLabels ?? []),
    ...filters
      .filter((filter) => filter.kind === "EXCLUDE_GENRES")
      .flatMap((filter) => filter.values),
  ]);
  const keys = [
    ...new Set([
      ...exclusions.flatMap((filter) => filter.values),
      ...converted.values,
    ]),
  ];
  const unresolved = converted.unresolvedLabels;
  const sources =
    facet === "ANIME"
      ? [
          "mal",
          "anilist",
          "anidb",
          ...SOURCES.filter(
            (source) => !["mal", "anilist", "anidb"].includes(source),
          ),
        ]
      : SOURCES;
  const setFacet = (kind: ListFilter["kind"], patch: Partial<ListFilter>) => {
    // Expand alpha filters before editing one facet so the other facets retain their rules.
    const expanded = filters.flatMap((filter): ListFilter[] =>
      filter.kind === "RATING_AT_LEAST"
        ? (["MOVIE", "SERIES", "ANIME"] as Facet[]).map((facet) => ({
            ...EMPTY_LIST_FILTER,
            kind: "RATINGS",
            facet,
            matchAny: false,
            minimums: [
              { source: filter.scale ?? "tmdb", value: filter.value ?? 0 },
            ],
          }))
        : filter.kind === "EXCLUDE_GENRES"
          ? (["MOVIE", "SERIES", "ANIME"] as Facet[]).map((facet) => ({
              ...EMPTY_LIST_FILTER,
              kind: "EXCLUDE_CANONICAL_TAGS",
              facet,
              ...resolveLabels(filter.values),
            }))
          : [filter],
    );
    onChange([
      ...expanded.filter(
        (filter) => !(filter.kind === kind && filter.facet === facet),
      ),
      { ...EMPTY_LIST_FILTER, kind, facet, ...patch },
    ]);
  };
  const setExclusions = (values: string[], unresolvedLabels = unresolved) =>
    setFacet("EXCLUDE_CANONICAL_TAGS", { values, unresolvedLabels });
  const visibleEntries =
    vocabulary?.entries.filter((entry) =>
      [entry.name, ...entry.aliases].some((name) =>
        name.toLowerCase().includes(search.toLowerCase()),
      ),
    ) ?? [];
  const ratingMatch = ratings[0]?.matchAny ? "any" : "all";
  return (
    <div className="grid gap-3 border-t border-[var(--scry-border3)] pt-4 sm:grid-cols-2">
      <div className="min-w-0 space-y-1.5">
      <span id={`${idPrefix}-rating-match-label`} className="block text-sm font-medium text-[var(--scry-ink2)]">{t("lists.filter.match")}</span>
      <ToggleGroupPrimitive.Root type="single"
        className="inline-flex max-w-full flex-wrap rounded-md border border-border p-1"
        aria-labelledby={`${idPrefix}-rating-match-label`}
        value={ratingMatch}
        disabled={disabled}
        onValueChange={(value) => { if (value) setFacet("RATINGS", { matchAny: value === "any", minimums }); }}
      >
        {(["all", "any"] as const).map((value) => (
          <ToggleGroupPrimitive.Item key={value} value={value} asChild>
            <Button type="button" size="sm" variant={value === ratingMatch ? "default" : "ghost"}>
              {t(`lists.filter.${value}`)}
            </Button>
          </ToggleGroupPrimitive.Item>
        ))}
      </ToggleGroupPrimitive.Root>
      </div>
      <div className="min-w-0 space-y-1.5">
      <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
        {t("lists.filter.canonical")}
      </h4>
      {error ? (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]"
        >
          {t("lists.filter.vocabularyError")}{" "}
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={loading || disabled}
            onClick={() => void retry()}
          >
            {t("lists.filter.retry")}
          </Button>
        </div>
      ) : null}
      {keys
        .filter(
          (key) => !vocabulary?.entries.some((entry) => entry.key === key),
        )
        .map((key) => (
          <CheckboxField
            key={key}
            id={`${idPrefix}-${key}`}
            checked
            label={t("lists.filter.unavailable", { label: key })}
            disabled={disabled}
            onCheckedChange={() =>
              setExclusions(keys.filter((value) => value !== key))
            }
          />
        ))}
      {unresolved.map((label) => (
        <CheckboxField
          key={label}
          id={`${idPrefix}-legacy-${label}`}
          checked
          label={t("lists.filter.unresolved", { label })}
          disabled={disabled}
          onCheckedChange={() =>
            setExclusions(
              keys,
              unresolved.filter((value) => value !== label),
            )
          }
        />
      ))}
      {vocabulary ? (
        <Popover modal onOpenChange={(open) => { if (!open) setSearch(""); }}>
          <PopoverTrigger asChild>
            <button
              type="button"
              id={`${idPrefix}-canonical-picker`}
              aria-label={t("lists.filter.canonical")}
              disabled={disabled}
              // As tall as the rating match group beside it.
              className={selectTriggerClassName({ className: "h-10.5 w-full" })}
            >
              <span className={`min-w-0 truncate text-left ${keys.length ? "" : "text-muted-foreground"}`}>
                {keys.length
                  ? keys.map((key) => vocabulary.entries.find((entry) => entry.key === key)?.name ?? key).join(", ")
                  : t("lists.filter.searchCanonical")}
              </span>
              <ChevronDown className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className={selectContentClassName("w-[var(--radix-popover-trigger-width)] space-y-2 p-2")}>
            <Input
              aria-label={t("lists.filter.searchCanonical")}
              placeholder={t("lists.filter.searchCanonical")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              disabled={disabled}
            />
            <MultiSelectOptionList
              groups={[
                {
                  options: visibleEntries.map((entry) => ({
                    value: entry.key,
                    label: entry.name,
                  })),
                },
              ]}
              selectedValues={keys}
              disabled={disabled}
              optionIdPrefix={idPrefix}
              maxHeightClassName="max-h-48"
              onSelectedValuesChange={(values) =>
                setExclusions([
                  ...keys.filter(
                    (key) => !visibleEntries.some((entry) => entry.key === key),
                  ),
                  ...values,
                ])
              }
            />
          </PopoverContent>
        </Popover>
      ) : null}
      </div>
      <h4 className="text-sm font-semibold text-[var(--scry-ink2)] sm:col-span-2">
        {t("lists.filter.ratings")}
      </h4>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,220px),1fr))] gap-2 sm:col-span-2">
        {sources.map((source) => {
          const info = ratingSourceInfo(source);
          return (
            <label key={source} className="flex items-center gap-2 text-sm">
              {info.logoSrc ? (
                <img
                  src={info.logoSrc}
                  alt=""
                  className="size-5 object-contain"
                />
              ) : null}
              <span className="min-w-0 flex-1">{info.label}</span>
              <Input
                {...decimalInputProps}
                className="w-20"
                disabled={disabled}
                aria-label={info.label}
                placeholder={`0–${SCALES[source] ?? 10}`}
                value={listRatingInputText(
                  ratingDrafts[source],
                  minimums.find((minimum) => minimum.source === source)
                    ?.value ?? null,
                )}
                onChange={(event) => {
                  const next = parseListRatingInput(event.target.value);
                  if (!next) return;
                  setRatingDrafts((current) => ({
                    ...current,
                    [source]: next.text,
                  }));
                  setFacet("RATINGS", {
                    matchAny: ratings[0]?.matchAny ?? false,
                    minimums: [
                      ...minimums.filter(
                        (minimum) => minimum.source !== source,
                      ),
                      ...(next.value !== null
                        ? [{ source, value: next.value }]
                        : []),
                    ],
                  });
                }}
              />
            </label>
          );
        })}
      </div>
    </div>
  );
}
