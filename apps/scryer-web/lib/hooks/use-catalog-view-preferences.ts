import * as React from "react";
import { useClient } from "urql";
import { toast } from "sonner";
import { useUiSettings } from "@/lib/context/ui-settings-context";
import { setMyCatalogViewMutation } from "@/lib/graphql/mutations";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { SetMyCatalogViewInput, UiSettings } from "@/lib/types/settings";
import {
  catalogColumnsInput,
  catalogDeviceClass,
  catalogFacetForView,
  catalogViewModeToProfile,
  savedCatalogViewMode,
  savedCatalogVisibleColumns,
} from "@/lib/utils/catalog-view-preferences";
import {
  readStoredContentViewMode,
  writeStoredContentViewMode,
  type ContentViewMode,
} from "@/components/views/media-content/content-view-mode";
import {
  TITLE_TABLE_COLUMN_KEYS,
  defaultTitleTableVisibleColumnsForView,
  isTitleTableColumnSupportedForView,
  type TitleTableColumnKey,
  type TitleTableVisibleColumns,
} from "@/components/views/media-content/title-table-shared";
import type { ViewId } from "@/components/root/types";

/**
 * The catalog view mode and table columns for one facet on this device class.
 * The profile is authoritative once loaded; until then, and when the profile
 * has nothing saved, the view mode this browser stored applies. Changes are
 * saved to the profile per device class and facet.
 */
export function useCatalogViewPreferences(
  view: ViewId,
  t: (key: string) => string,
) {
  const client = useClient();
  const isMobile = useIsMobile();
  const { uiSettings, uiSettingsLoaded, setUiSettings } = useUiSettings();
  const deviceClass = catalogDeviceClass(isMobile);
  const facet = catalogFacetForView(view);
  const scopeKey = `${deviceClass}:${view}`;

  // Choices made in this session show at once, before the save returns.
  const [localViewModes, setLocalViewModes] = React.useState<
    Record<string, ContentViewMode>
  >({});
  const [localColumns, setLocalColumns] = React.useState<
    Record<string, TitleTableVisibleColumns>
  >({});
  const saveSequenceRef = React.useRef(0);

  const isSupported = React.useCallback(
    (key: TitleTableColumnKey) => isTitleTableColumnSupportedForView(key, view),
    [view],
  );

  const viewMode: ContentViewMode = React.useMemo(() => {
    const local = localViewModes[scopeKey];
    if (local) {
      return local;
    }
    const saved =
      uiSettingsLoaded && facet
        ? savedCatalogViewMode(uiSettings, deviceClass, facet)
        : null;
    return saved ?? readStoredContentViewMode(view);
  }, [deviceClass, facet, localViewModes, scopeKey, uiSettings, uiSettingsLoaded, view]);

  const visibleColumns: TitleTableVisibleColumns = React.useMemo(() => {
    const local = localColumns[scopeKey];
    if (local) {
      return local;
    }
    const defaults = defaultTitleTableVisibleColumnsForView(view);
    if (!uiSettingsLoaded || !facet) {
      return defaults;
    }
    return (
      savedCatalogVisibleColumns(uiSettings, {
        deviceClass,
        facet,
        viewMode,
        columnKeys: TITLE_TABLE_COLUMN_KEYS,
        defaults,
        isSupported,
      }) ?? defaults
    );
  }, [
    deviceClass,
    facet,
    isSupported,
    localColumns,
    scopeKey,
    uiSettings,
    uiSettingsLoaded,
    view,
    viewMode,
  ]);

  const saveToProfile = React.useCallback(
    (input: SetMyCatalogViewInput) => {
      if (!uiSettingsLoaded) {
        return;
      }
      const sequence = ++saveSequenceRef.current;
      void client
        .mutation<{ setMyCatalogView?: UiSettings }, { input: SetMyCatalogViewInput }>(
          setMyCatalogViewMutation,
          { input },
        )
        .toPromise()
        .then((result) => {
          if (result.error || !result.data?.setMyCatalogView) {
            toast.error(t("status.catalogViewSaveFailed"));
            return;
          }
          if (sequence === saveSequenceRef.current) {
            setUiSettings(result.data.setMyCatalogView);
          }
        });
    },
    [client, setUiSettings, t, uiSettingsLoaded],
  );

  const setViewMode = React.useCallback(
    (mode: ContentViewMode) => {
      setLocalViewModes((current) => ({ ...current, [scopeKey]: mode }));
      writeStoredContentViewMode(mode, view);
      if (facet) {
        saveToProfile({ deviceClass, facet, viewMode: catalogViewModeToProfile(mode) });
      }
    },
    [deviceClass, facet, saveToProfile, scopeKey, view],
  );

  const setColumnVisible = React.useCallback(
    (key: TitleTableColumnKey, checked: boolean) => {
      const next = { ...visibleColumns, [key]: checked };
      setLocalColumns((current) => ({ ...current, [scopeKey]: next }));
      if (facet) {
        saveToProfile({
          deviceClass,
          facet,
          columns: catalogColumnsInput(next, TITLE_TABLE_COLUMN_KEYS, isSupported),
        });
      }
    },
    [deviceClass, facet, isSupported, saveToProfile, scopeKey, visibleColumns],
  );

  return { viewMode, setViewMode, visibleColumns, setColumnVisible };
}
