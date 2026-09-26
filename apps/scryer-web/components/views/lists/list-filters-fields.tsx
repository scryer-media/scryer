import { CheckboxField } from "@/components/ui/checkbox";
import {
  decimalInputProps,
  Input,
  integerInputProps,
  sanitizeDecimal,
  sanitizeDigits,
} from "@/components/ui/input";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListFilter } from "@/lib/types/lists";
import {
  EMPTY_LIST_FILTER,
  findListFilter,
  splitListValues,
  withListFilter,
} from "@/lib/utils/lists";

const RATING_SCALES = ["tmdb", "imdb"] as const;

type ListFiltersFieldsProps = {
  filters: ListFilter[];
  disabled?: boolean;
  idPrefix: string;
  onChange: (filters: ListFilter[]) => void;
};

function numberOrNull(raw: string): number | null {
  if (!raw.trim()) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

const FIELD_LABEL = "block text-sm font-medium text-[var(--scry-ink2)]";

/**
 * The filters a public list can apply. Filter kinds this form does not edit
 * are kept untouched so saving never drops them.
 */
export function ListFiltersFields({ filters, disabled, idPrefix, onChange }: ListFiltersFieldsProps) {
  const t = useTranslate();
  const rating = findListFilter(filters, "RATING_AT_LEAST");
  const years = findListFilter(filters, "RELEASE_YEAR");
  const genres = findListFilter(filters, "EXCLUDE_GENRES");
  const languages = findListFilter(filters, "LANGUAGE");
  const releasedOnly = findListFilter(filters, "RELEASED_ONLY");

  const setRating = (scale: string | null, value: number | null) => {
    onChange(
      withListFilter(
        filters,
        "RATING_AT_LEAST",
        value === null ? null : { ...EMPTY_LIST_FILTER, scale: scale ?? RATING_SCALES[0], value },
      ),
    );
  };
  const setYears = (from: number | null, to: number | null) => {
    onChange(
      withListFilter(
        filters,
        "RELEASE_YEAR",
        from === null && to === null ? null : { ...EMPTY_LIST_FILTER, from, to },
      ),
    );
  };
  const setValues = (kind: "EXCLUDE_GENRES" | "LANGUAGE", raw: string) => {
    const values = splitListValues(raw);
    onChange(withListFilter(filters, kind, values.length ? { ...EMPTY_LIST_FILTER, values } : null));
  };

  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1.5">
        <span className={FIELD_LABEL}>{t("lists.filter.rating")}</span>
        <div className="flex gap-2">
          <SingleSelectField
            id={`${idPrefix}-filter-rating-scale`}
            className="w-28 flex-none space-y-0"
            label={t("lists.filter.ratingScale")}
            labelClassName="sr-only"
            value={rating?.scale ?? RATING_SCALES[0]}
            options={RATING_SCALES.map((scale) => ({ value: scale, label: t(`lists.filter.scale.${scale}`) }))}
            onValueChange={(scale) => setRating(scale, rating?.value ?? null)}
            disabled={disabled}
          />
          <Input
            id={`${idPrefix}-filter-rating-value`}
            {...decimalInputProps}
            placeholder={t("lists.filter.ratingPlaceholder")}
            value={rating?.value === null || rating?.value === undefined ? "" : String(rating.value)}
            onChange={(event) =>
              setRating(rating?.scale ?? null, numberOrNull(sanitizeDecimal(event.target.value)))
            }
            disabled={disabled}
          />
        </div>
      </div>
      <div className="space-y-1.5">
        <span className={FIELD_LABEL}>{t("lists.filter.releaseYear")}</span>
        <div className="flex items-center gap-2">
          <Input
            id={`${idPrefix}-filter-year-from`}
            {...integerInputProps}
            aria-label={t("lists.filter.yearFrom")}
            placeholder={t("lists.filter.yearFrom")}
            value={years?.from === null || years?.from === undefined ? "" : String(years.from)}
            onChange={(event) => setYears(numberOrNull(sanitizeDigits(event.target.value)), years?.to ?? null)}
            disabled={disabled}
          />
          <span className="text-[var(--scry-muted)]">–</span>
          <Input
            id={`${idPrefix}-filter-year-to`}
            {...integerInputProps}
            aria-label={t("lists.filter.yearTo")}
            placeholder={t("lists.filter.yearTo")}
            value={years?.to === null || years?.to === undefined ? "" : String(years.to)}
            onChange={(event) => setYears(years?.from ?? null, numberOrNull(sanitizeDigits(event.target.value)))}
            disabled={disabled}
          />
        </div>
      </div>
      <label className="space-y-1.5" htmlFor={`${idPrefix}-filter-genres`}>
        <span className={FIELD_LABEL}>{t("lists.filter.excludeGenres")}</span>
        <Input
          id={`${idPrefix}-filter-genres`}
          placeholder={t("lists.filter.excludeGenresPlaceholder")}
          defaultValue={genres?.values.join(", ") ?? ""}
          onBlur={(event) => setValues("EXCLUDE_GENRES", event.target.value)}
          disabled={disabled}
        />
      </label>
      <label className="space-y-1.5" htmlFor={`${idPrefix}-filter-languages`}>
        <span className={FIELD_LABEL}>{t("lists.filter.languages")}</span>
        <Input
          id={`${idPrefix}-filter-languages`}
          placeholder={t("lists.filter.languagesPlaceholder")}
          defaultValue={languages?.values.join(", ") ?? ""}
          onBlur={(event) => setValues("LANGUAGE", event.target.value)}
          disabled={disabled}
        />
      </label>
      <CheckboxField
        id={`${idPrefix}-filter-released-only`}
        className="sm:col-span-2"
        label={t("lists.filter.releasedOnly")}
        description={t("lists.filter.releasedOnlyHelp")}
        checked={releasedOnly !== null}
        onCheckedChange={(checked) =>
          onChange(withListFilter(filters, "RELEASED_ONLY", checked === true ? EMPTY_LIST_FILTER : null))
        }
        disabled={disabled}
      />
    </div>
  );
}
