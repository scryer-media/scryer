import { TitleTagsPicker } from "@/components/common/title-tags-picker";
import { ChevronDown } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleTagDefinition } from "@/lib/types/title-tags";
import type { ListRoute, ListFilter } from "@/lib/types/lists";
import { ListFacetFilters } from "./list-facet-filters";
import type { Facet, LibraryRecord } from "@/lib/types/titles";
import { listKindLabelKey } from "@/lib/utils/lists";
import { facetById } from "@/lib/facets/registry";
import { facetStyle } from "@/lib/facets/style";

const INHERIT_QUALITY_PROFILE = "__inherit__";

type ListRouteCardProps = {
  filters: ListFilter[];
  onFiltersChange: (filters: ListFilter[]) => void;
  kind: Facet;
  route: ListRoute;
  libraries: readonly LibraryRecord[];
  qualityProfiles: ReadonlyArray<{ id: string; name: string }>;
  tagDefinitions: readonly TitleTagDefinition[];
  tagsLoading: boolean;
  disabled?: boolean;
  requestOnly?: boolean;
  idPrefix: string;
  onChange: (route: ListRoute) => void;
};

/** Where one kind of list item lands: library, profile, folder and the add options. */
export function ListRouteCard({
  kind,
  route,
  libraries,
  qualityProfiles,
  tagDefinitions,
  tagsLoading,
  disabled,
  requestOnly = false,
  idPrefix,
  onChange,
  filters,
  onFiltersChange,
}: ListRouteCardProps) {
  const t = useTranslate();
  const FacetIcon = facetById(kind)!.icon;
  const kindLibraries = libraries.filter((library) => library.facet === kind);
  const library = kindLibraries.find((entry) => entry.id === route.libraryId) ?? null;
  const roots = library?.roots ?? [];
  const update = (patch: Partial<ListRoute>) => onChange({ ...route, ...patch });

  const monitorOptions =
    kind === "MOVIE"
      ? [
          { value: "MONITORED", label: t("search.monitorType.monitored") },
          { value: "UNMONITORED", label: t("search.monitorType.unmonitored") },
        ]
      : [
          { value: "FUTURE_EPISODES", label: t("search.monitorType.futureEpisodes") },
          { value: "MISSING_AND_FUTURE_EPISODES", label: t("search.monitorType.missingAndFutureEpisodes") },
          { value: "ALL_EPISODES", label: t("search.monitorType.allEpisodes") },
          { value: "NONE", label: t("search.monitorType.none") },
        ];

  return (
    <Collapsible
      id={`${idPrefix}-route-${kind.toLowerCase()}`}
      className="rounded-[12px] border"
      style={{ borderColor: facetStyle(kind).dot }}
      disabled={disabled}
    >
      <CollapsibleTrigger className="group flex w-full items-center justify-between gap-3 rounded-[12px] p-4 text-left text-[13px] font-semibold text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" disabled={disabled}>
        <span className="inline-flex items-center gap-2">
          <FacetIcon className="size-4" style={{ color: facetStyle(kind).text }} aria-hidden="true" />
          {t(listKindLabelKey(kind))}
        </span>
        <ChevronDown className="size-4 shrink-0 transition-transform group-data-[state=open]:rotate-180" aria-hidden="true" />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <fieldset className="space-y-4 px-4 pb-4" disabled={disabled}>
          <legend className="sr-only">{t("lists.route.heading", { kind: t(listKindLabelKey(kind)) })}</legend>
          {kindLibraries.length === 0 ? (
            <p className="text-[12.5px] text-[var(--scry-warning-text)]">{t("lists.route.noLibrary")}</p>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-2">
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-library`}
              label={t("search.addConfigLibrary")}
              value={route.libraryId}
              options={kindLibraries.map((entry) => ({ value: entry.id, label: entry.name }))}
              onValueChange={(libraryId) => {
                const next = kindLibraries.find((entry) => entry.id === libraryId);
                const root = next?.roots.find((entry) => entry.isDefault) ?? next?.roots[0];
                update({ libraryId, rootFolderId: root?.id ?? null });
              }}
              disabled={disabled || kindLibraries.length === 0}
            />
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-quality`}
              label={t("search.addConfigQualityProfile")}
              value={route.qualityProfileId ?? INHERIT_QUALITY_PROFILE}
              options={[
                ...(!requestOnly ? [{ value: INHERIT_QUALITY_PROFILE, label: t("search.addConfigInheritLibrary") }] : []),
                ...qualityProfiles.map((profile) => ({ value: profile.id, label: profile.name })),
              ]}
              onValueChange={(value) =>
                update({ qualityProfileId: value === INHERIT_QUALITY_PROFILE ? null : value })
              }
              disabled={disabled}
            />
            {!requestOnly ? <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-root`}
              label={t("search.addConfigRootFolder")}
              valueKind="path"
              value={route.rootFolderId ?? ""}
              options={roots.map((root) => ({ value: root.id, label: root.path }))}
              onValueChange={(rootFolderId) => update({ rootFolderId })}
              disabled={disabled || roots.length === 0}
            /> : null}
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-monitor`}
              label={t("search.addConfigMonitorType")}
              value={route.monitorType}
              options={monitorOptions}
              onValueChange={(monitorType) => update({ monitorType })}
              disabled={disabled}
            />
            {!requestOnly && (kind === "MOVIE" ? (
              <SingleSelectField
                id={`${idPrefix}-route-${kind.toLowerCase()}-availability`}
                label={t("settings.minAvailabilityLabel")}
                value={route.minAvailability ?? "announced"}
                options={["announced", "in_cinemas", "released"].map((value) => ({
                  value,
                  label: t(`settings.minAvailability.${value}`),
                }))}
                onValueChange={(minAvailability) => update({ minAvailability })}
                disabled={disabled}
              />
            ) : (
              <SingleSelectField
                id={`${idPrefix}-route-${kind.toLowerCase()}-season-folder`}
                label={t("search.addConfigSeasonFolder")}
                value={route.useSeasonFolders === false ? "disabled" : "enabled"}
                options={[
                  { value: "enabled", label: t("search.seasonFolder.enabled") },
                  { value: "disabled", label: t("search.seasonFolder.disabled") },
                ]}
                onValueChange={(value) => update({ useSeasonFolders: value === "enabled" })}
                disabled={disabled}
              />
            ))}
            {!requestOnly && kind === "ANIME" ? (
              <SingleSelectField
                id={`${idPrefix}-route-${kind.toLowerCase()}-numbering`}
                label={t("settings.releaseNumberingLabel")}
                value={route.releaseNumbering ?? "AUTO"}
                options={[
                  { value: "AUTO", label: t("settings.releaseNumberingAuto") },
                  { value: "OFFICIAL", label: t("settings.releaseNumberingOfficial") },
                  { value: "ALTERNATE", label: t("settings.releaseNumberingAlternate") },
                  { value: "DVD", label: t("settings.releaseNumberingDvd") },
                ]}
                onValueChange={(releaseNumbering) => update({ releaseNumbering })}
                disabled={disabled}
              />
            ) : null}
          </div>
          {!requestOnly ? <TitleTagsPicker
            idPrefix={`${idPrefix}-route-${kind.toLowerCase()}-tags`}
            value={route.tags}
            onChange={(tags) => update({ tags })}
            definitions={tagDefinitions}
            loading={tagsLoading}
            disabled={disabled}
          /> : null}
          <ListFacetFilters facet={kind} filters={filters} onChange={onFiltersChange} disabled={disabled} idPrefix={`${idPrefix}-${kind.toLowerCase()}`} />
        </fieldset>
      </CollapsibleContent>
    </Collapsible>
  );
}
