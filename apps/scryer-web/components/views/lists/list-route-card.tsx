import { TitleTagsPicker } from "@/components/common/title-tags-picker";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleTagDefinition } from "@/lib/types/title-tags";
import type { ListRoute } from "@/lib/types/lists";
import type { Facet, LibraryRecord } from "@/lib/types/titles";
import { listKindLabelKey } from "@/lib/utils/lists";

const INHERIT_QUALITY_PROFILE = "__inherit__";

type ListRouteCardProps = {
  kind: Facet;
  route: ListRoute;
  libraries: readonly LibraryRecord[];
  qualityProfiles: ReadonlyArray<{ id: string; name: string }>;
  tagDefinitions: readonly TitleTagDefinition[];
  tagsLoading: boolean;
  disabled?: boolean;
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
  idPrefix,
  onChange,
}: ListRouteCardProps) {
  const t = useTranslate();
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
    <fieldset
      id={`${idPrefix}-route-${kind.toLowerCase()}`}
      className="space-y-4 rounded-[12px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-4"
      disabled={disabled}
    >
      <legend className="px-1 text-[13px] font-semibold text-[var(--scry-ink2)]">
        {t("lists.route.heading", { kind: t(listKindLabelKey(kind)) })}
      </legend>
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
            { value: INHERIT_QUALITY_PROFILE, label: t("search.addConfigInheritLibrary") },
            ...qualityProfiles.map((profile) => ({ value: profile.id, label: profile.name })),
          ]}
          onValueChange={(value) =>
            update({ qualityProfileId: value === INHERIT_QUALITY_PROFILE ? null : value })
          }
          disabled={disabled}
        />
        <SingleSelectField
          id={`${idPrefix}-route-${kind.toLowerCase()}-root`}
          label={t("search.addConfigRootFolder")}
          valueKind="path"
          value={route.rootFolderId ?? ""}
          options={roots.map((root) => ({ value: root.id, label: root.path }))}
          onValueChange={(rootFolderId) => update({ rootFolderId })}
          disabled={disabled || roots.length === 0}
        />
        <SingleSelectField
          id={`${idPrefix}-route-${kind.toLowerCase()}-monitor`}
          label={t("search.addConfigMonitorType")}
          value={route.monitorType}
          options={monitorOptions}
          onValueChange={(monitorType) => update({ monitorType })}
          disabled={disabled}
        />
        {kind === "MOVIE" ? (
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
        )}
        {kind === "ANIME" ? (
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
      <TitleTagsPicker
        idPrefix={`${idPrefix}-route-${kind.toLowerCase()}-tags`}
        value={route.tags}
        onChange={(tags) => update({ tags })}
        definitions={tagDefinitions}
        loading={tagsLoading}
        disabled={disabled}
      />
    </fieldset>
  );
}
