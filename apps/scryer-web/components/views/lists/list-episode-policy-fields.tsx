import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListFilter } from "@/lib/types/lists";
import type { Facet } from "@/lib/types/titles";
import { EMPTY_LIST_FILTER } from "@/lib/utils/lists";

const INHERIT = "__inherit__";

export function ListEpisodePolicyFields({
  facet, filters, onChange, disabled, idPrefix,
}: {
  facet: Facet;
  filters: ListFilter[];
  onChange: (filters: ListFilter[]) => void;
  disabled?: boolean;
  idPrefix: string;
}) {
  const t = useTranslate();
  if (facet === "MOVIE") return null;
  const policies: Array<{
    kind: ListFilter["kind"];
    label: string;
    options: Array<{ value: string; label: string }>;
  }> = [{
    kind: "MONITOR_SPECIALS",
    label: "settings.monitorSpecialsLabel",
    options: [
      { value: "true", label: "search.seasonFolder.enabled" },
      { value: "false", label: "search.seasonFolder.disabled" },
    ],
  }];
  if (facet === "ANIME") policies.push({
    kind: "FILLER_POLICY",
    label: "settings.fillerPolicyLabel",
    options: [
      { value: "DOWNLOAD_ALL", label: "settings.fillerPolicyDownloadAll" },
      { value: "SKIP_FILLER", label: "settings.fillerPolicySkipFiller" },
    ],
  }, {
    kind: "RECAP_POLICY",
    label: "settings.recapPolicyLabel",
    options: [
      { value: "DOWNLOAD_ALL", label: "settings.recapPolicyDownloadAll" },
      { value: "SKIP_RECAP", label: "settings.recapPolicySkipRecap" },
    ],
  });
  return <div className="grid gap-3 sm:col-span-2 sm:grid-cols-2">
    {policies.map(({ kind, label, options }) => <SingleSelectField
      key={kind}
      id={`${idPrefix}-${kind.toLowerCase()}`}
      label={t(label)}
      value={filters.find((filter) => filter.kind === kind && filter.facet === facet)?.values[0] ?? INHERIT}
      options={[
        { value: INHERIT, label: t("search.addConfigInheritLibrary") },
        ...options.map((option) => ({ ...option, label: t(option.label) })),
      ]}
      onValueChange={(value) => onChange([
        ...filters.filter((filter) => filter.kind !== kind || filter.facet !== facet),
        ...(value === INHERIT ? [] : [{ ...EMPTY_LIST_FILTER, kind, facet, values: [value] }]),
      ])}
      disabled={disabled}
    />)}
  </div>;
}
