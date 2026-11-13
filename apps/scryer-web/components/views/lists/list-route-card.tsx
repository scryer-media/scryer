import * as React from "react";
import { CatalogAudioLanguageFields } from "@/components/common/catalog-audio-language-fields";
import { TitleTagsPicker } from "@/components/common/title-tags-picker";
import {
  Calendar,
  ChevronDown,
  Eye,
  FolderOpen,
  Folders,
  Hash,
  Library,
  SlidersHorizontal,
  Tag,
  type LucideIcon,
} from "lucide-react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleTagDefinition } from "@/lib/types/title-tags";
import type { ListRoute, ListFilter } from "@/lib/types/lists";
import { ListFacetFilters } from "./list-facet-filters";
import { ListEpisodePolicyFields } from "./list-episode-policy-fields";
import type { Facet, LibraryRecord } from "@/lib/types/titles";
import { listKindLabelKey } from "@/lib/utils/lists";
import {
  catalogAudioLanguageDraftFrom,
  catalogAudioLanguagesForSubmit,
  type CatalogAudioLanguageDraft,
} from "@/lib/utils/catalog-audio-languages";
import { selectorId } from "@/lib/utils/dom-ids";
import { facetById } from "@/lib/facets/registry";
import { facetStyle } from "@/lib/facets/style";

const INHERIT_QUALITY_PROFILE = "__inherit__";
/** Select value standing for "follow the instance default" (a null route value). */
const INSTANCE_DEFAULT_AVAILABILITY_VALUE = "default";

function RouteFieldLabel({
  icon: Icon,
  label,
}: {
  icon: LucideIcon;
  label: string;
}) {
  return (
    <span className="inline-flex items-center gap-2">
      <Icon
        className="size-4 shrink-0 text-[var(--scry-muted3)]"
        aria-hidden="true"
      />
      {label}
    </span>
  );
}

function sameLanguageList(
  left: readonly string[] | null | undefined,
  right: readonly string[] | null | undefined,
): boolean {
  if (left == null || right == null) {
    return (left == null) === (right == null);
  }
  return (
    left.length === right.length &&
    left.every((code, index) => code === right[index])
  );
}

/**
 * The route's audio languages for new titles. The choice is held here so that
 * "choose" stays open before any language is ticked; the route only ever
 * carries what the choice sends: null to inherit, an empty required list to
 * require none, or the chosen languages.
 */
function ListRouteAudioLanguageFields({
  route,
  library,
  idPrefix,
  disabled,
  onChange,
}: {
  route: ListRoute;
  /** The route's library, whose languages "inherit" stands for when known. */
  library: LibraryRecord | null;
  idPrefix: string;
  disabled?: boolean;
  onChange: (
    patch: Pick<ListRoute, "requiredAudioLanguages" | "preferredAudioLanguages">,
  ) => void;
}) {
  const [draft, setDraft] = React.useState<CatalogAudioLanguageDraft>(() =>
    catalogAudioLanguageDraftFrom(
      route.requiredAudioLanguages,
      route.preferredAudioLanguages,
    ),
  );
  // The draft follows the route: when the route's stored languages are no
  // longer what the draft sends (the route was replaced under the card), it
  // starts again from them. A draft that sends what is stored is kept, so
  // "choose" with nothing ticked stays open.
  const sent = catalogAudioLanguagesForSubmit(draft);
  if (
    !sameLanguageList(sent.required ?? null, route.requiredAudioLanguages) ||
    // An empty preferred list inherits, as null does.
    !sameLanguageList(
      sent.preferred ?? null,
      route.preferredAudioLanguages?.length
        ? route.preferredAudioLanguages
        : null,
    )
  ) {
    setDraft(
      catalogAudioLanguageDraftFrom(
        route.requiredAudioLanguages,
        route.preferredAudioLanguages,
      ),
    );
  }
  return (
    <CatalogAudioLanguageFields
      idPrefix={idPrefix}
      value={draft}
      onChange={(next) => {
        setDraft(next);
        const sent = catalogAudioLanguagesForSubmit(next);
        onChange({
          requiredAudioLanguages: sent.required ?? null,
          preferredAudioLanguages: sent.preferred ?? null,
        });
      }}
      inheritedRequired={library?.inheritedRequiredAudioLanguages ?? null}
      inheritedPreferred={library?.inheritedPreferredAudioLanguages ?? null}
      disabled={Boolean(disabled)}
    />
  );
}

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
  const library =
    kindLibraries.find((entry) => entry.id === route.libraryId) ?? null;
  const roots = library?.roots ?? [];
  const update = (patch: Partial<ListRoute>) =>
    onChange({ ...route, ...patch });

  const monitorOptions =
    kind === "MOVIE"
      ? [
          { value: "MONITORED", label: t("search.monitorType.monitored") },
          { value: "UNMONITORED", label: t("search.monitorType.unmonitored") },
        ]
      : [
          {
            value: "FUTURE_EPISODES",
            label: t("search.monitorType.futureEpisodes"),
          },
          {
            value: "MISSING_AND_FUTURE_EPISODES",
            label: t("search.monitorType.missingAndFutureEpisodes"),
          },
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
      <CollapsibleTrigger
        id={`${idPrefix}-route-${kind.toLowerCase()}-toggle`}
        className="group flex w-full items-center justify-between gap-3 rounded-[12px] p-4 text-left text-[13px] font-semibold text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        disabled={disabled}
      >
        <span className="inline-flex items-center gap-2">
          <FacetIcon
            className="size-4"
            style={{ color: facetStyle(kind).text }}
            aria-hidden="true"
          />
          {t(listKindLabelKey(kind))}
        </span>
        <ChevronDown
          className="size-4 shrink-0 transition-transform group-data-[state=open]:rotate-180"
          aria-hidden="true"
        />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <fieldset className="space-y-4 px-4 pb-4" disabled={disabled}>
          <legend className="sr-only">
            {t("lists.route.heading", { kind: t(listKindLabelKey(kind)) })}
          </legend>
          {kindLibraries.length === 0 ? (
            <p className="text-[12.5px] text-[var(--scry-warning-text)]">
              {t("lists.route.noLibrary")}
            </p>
          ) : null}
          <div className="grid gap-3 sm:grid-cols-2">
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-library`}
              label={
                <RouteFieldLabel
                  icon={Library}
                  label={t("search.addConfigLibrary")}
                />
              }
              value={route.libraryId}
              options={kindLibraries.map((entry) => ({
                value: entry.id,
                label: entry.name,
              }))}
              onValueChange={(libraryId) => {
                const next = kindLibraries.find(
                  (entry) => entry.id === libraryId,
                );
                const root =
                  next?.roots.find((entry) => entry.isDefault) ??
                  next?.roots[0];
                update({ libraryId, rootFolderId: root?.id ?? null });
              }}
              disabled={disabled || kindLibraries.length === 0}
            />
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-quality`}
              label={
                <RouteFieldLabel
                  icon={SlidersHorizontal}
                  label={t("search.addConfigQualityProfile")}
                />
              }
              value={route.qualityProfileId ?? INHERIT_QUALITY_PROFILE}
              options={[
                ...(!requestOnly
                  ? [
                      {
                        value: INHERIT_QUALITY_PROFILE,
                        label: t("search.addConfigInheritLibrary"),
                      },
                    ]
                  : []),
                ...qualityProfiles.map((profile) => ({
                  value: profile.id,
                  label: profile.name,
                })),
              ]}
              onValueChange={(value) =>
                update({
                  qualityProfileId:
                    value === INHERIT_QUALITY_PROFILE ? null : value,
                })
              }
              disabled={disabled}
            />
            {!requestOnly ? (
              <SingleSelectField
                id={`${idPrefix}-route-${kind.toLowerCase()}-root`}
                label={
                  <RouteFieldLabel
                    icon={FolderOpen}
                    label={t("search.addConfigRootFolder")}
                  />
                }
                valueKind="path"
                value={route.rootFolderId ?? ""}
                options={roots.map((root) => ({
                  value: root.id,
                  label: root.path,
                }))}
                onValueChange={(rootFolderId) => update({ rootFolderId })}
                disabled={disabled || roots.length === 0}
              />
            ) : null}
            <SingleSelectField
              id={`${idPrefix}-route-${kind.toLowerCase()}-monitor`}
              label={
                <RouteFieldLabel
                  icon={Eye}
                  label={t("search.addConfigMonitorType")}
                />
              }
              value={route.monitorType}
              options={monitorOptions}
              onValueChange={(monitorType) => update({ monitorType })}
              disabled={disabled}
            />
            {!requestOnly &&
              (kind === "MOVIE" ? (
                <SingleSelectField
                  id={`${idPrefix}-route-${kind.toLowerCase()}-availability`}
                  label={
                    <RouteFieldLabel
                      icon={Calendar}
                      label={t("settings.minAvailabilityLabel")}
                    />
                  }
                  value={
                    route.minAvailability ?? INSTANCE_DEFAULT_AVAILABILITY_VALUE
                  }
                  options={[
                    INSTANCE_DEFAULT_AVAILABILITY_VALUE,
                    "announced",
                    "in_cinemas",
                    "released",
                  ].map((value) => ({
                    value,
                    label: t(`settings.minAvailability.${value}`),
                  }))}
                  onValueChange={(minAvailability) =>
                    update({
                      minAvailability:
                        minAvailability === INSTANCE_DEFAULT_AVAILABILITY_VALUE
                          ? null
                          : minAvailability,
                    })
                  }
                  disabled={disabled}
                />
              ) : (
                <SingleSelectField
                  id={`${idPrefix}-route-${kind.toLowerCase()}-season-folder`}
                  label={
                    <RouteFieldLabel
                      icon={Folders}
                      label={t("search.addConfigSeasonFolder")}
                    />
                  }
                  value={
                    route.useSeasonFolders === false ? "disabled" : "enabled"
                  }
                  options={[
                    {
                      value: "enabled",
                      label: t("search.seasonFolder.enabled"),
                    },
                    {
                      value: "disabled",
                      label: t("search.seasonFolder.disabled"),
                    },
                  ]}
                  onValueChange={(value) =>
                    update({ useSeasonFolders: value === "enabled" })
                  }
                  disabled={disabled}
                />
              ))}
            {!requestOnly && kind === "ANIME" ? (
              <SingleSelectField
                id={`${idPrefix}-route-${kind.toLowerCase()}-numbering`}
                label={
                  <RouteFieldLabel
                    icon={Hash}
                    label={t("settings.releaseNumberingLabel")}
                  />
                }
                value={route.releaseNumbering ?? "AUTO"}
                options={[
                  { value: "AUTO", label: t("settings.releaseNumberingAuto") },
                  {
                    value: "OFFICIAL",
                    label: t("settings.releaseNumberingOfficial"),
                  },
                  {
                    value: "ALTERNATE",
                    label: t("settings.releaseNumberingAlternate"),
                  },
                  { value: "DVD", label: t("settings.releaseNumberingDvd") },
                ]}
                onValueChange={(releaseNumbering) =>
                  update({ releaseNumbering })
                }
                disabled={disabled}
              />
            ) : null}
            {!requestOnly ? (
              <div className="min-w-0 space-y-1.5">
                <label
                  htmlFor={`${idPrefix}-route-${kind.toLowerCase()}-tags-tags-add`}
                  className="block text-sm font-medium text-[var(--scry-ink2)]"
                >
                  <RouteFieldLabel icon={Tag} label={t("settings.titleTags")} />
                </label>
                <TitleTagsPicker
                  layout="compact"
                  idPrefix={`${idPrefix}-route-${kind.toLowerCase()}-tags`}
                  value={route.tags}
                  onChange={(tags) => update({ tags })}
                  definitions={tagDefinitions}
                  loading={tagsLoading}
                  disabled={disabled}
                />
              </div>
            ) : null}
            <ListRouteAudioLanguageFields
              route={route}
              library={library}
              idPrefix={selectorId(idPrefix, "route", kind.toLowerCase())}
              disabled={disabled}
              onChange={update}
            />
          </div>
          <ListFacetFilters
            facet={kind}
            filters={filters}
            onChange={onFiltersChange}
            disabled={disabled}
            idPrefix={`${idPrefix}-${kind.toLowerCase()}`}
          />
          <ListEpisodePolicyFields
            facet={kind}
            library={library}
            filters={filters}
            onChange={onFiltersChange}
            disabled={disabled}
            idPrefix={`${idPrefix}-${kind.toLowerCase()}`}
          />
        </fieldset>
      </CollapsibleContent>
    </Collapsible>
  );
}
