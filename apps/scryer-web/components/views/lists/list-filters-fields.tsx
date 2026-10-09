import { CheckboxField } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { SEARCH_LANGUAGES } from "@/lib/constants/audio-languages";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListFilter } from "@/lib/types/lists";
import {
  EMPTY_LIST_FILTER,
  findListFilter,
  withListFilter,
} from "@/lib/utils/lists";

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
  const years = findListFilter(filters, "RELEASE_YEAR");
  const languages = findListFilter(filters, "LANGUAGE");
  const releasedOnly = findListFilter(filters, "RELEASED_ONLY");

  const setYears = (from: number | null, to: number | null) => {
    onChange(
      withListFilter(
        filters,
        "RELEASE_YEAR",
        from === null && to === null ? null : { ...EMPTY_LIST_FILTER, from, to },
      ),
    );
  };

  return (
    <div className="grid gap-3 sm:grid-cols-2">
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
      <div className="space-y-1.5">
        <span className={FIELD_LABEL}>{t("lists.filter.languages")}</span>
        <SubtitleLanguagePicker
          languageOptions={SEARCH_LANGUAGES}
          ariaLabel={t("lists.filter.languages")}
          value={languages?.values ?? []}
          onChange={(values) => onChange(withListFilter(filters, "LANGUAGE", values.length ? { ...EMPTY_LIST_FILTER, values } : null))}
          disabled={disabled}
          triggerId={`${idPrefix}-filter-languages`}
          modal
        />
      </div>
      <CheckboxField
        id={`${idPrefix}-filter-released-only`}
        className="sm:col-span-2"
        label={t("lists.filter.releasedOnly")}
        checked={releasedOnly !== null}
        onCheckedChange={(checked) =>
          onChange(withListFilter(filters, "RELEASED_ONLY", checked === true ? EMPTY_LIST_FILTER : null))
        }
        disabled={disabled}
      />
    </div>
  );
}
