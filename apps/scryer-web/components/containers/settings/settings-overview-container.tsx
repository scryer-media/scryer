import * as React from "react";
import { SettingsOverviewSection } from "@/components/views/settings/settings-overview-section";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  generalSettingsQuery,
  verificationSettingsQuery,
} from "@/lib/graphql/queries";
import {
  clearTitleImageCacheMutation,
  rehydrateAllMetadataMutation,
  setMyUiSettingsMutation,
  updateGeneralSettingsMutation,
  updateVerificationSettingsMutation,
} from "@/lib/graphql/mutations";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  uiSettingsInputFromSettings,
  useUiSettings,
} from "@/lib/context/ui-settings-context";
import { useRefreshInstanceFeatures } from "@/lib/context/instance-features-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import type {
  GeneralSettings,
  GeneralSettingsUpdate,
  UiDateTimeFormat,
  UiSettings,
  VerificationDepth,
} from "@/lib/types/settings";

const DEFAULT_GENERAL_SETTINGS: GeneralSettings = {
  experimentalFeaturesEnabled: false,
  personalizedDiscoveryEnabled: true,
  srrdbFilenameRecoveryEnabled: false,
  keepHistoryForever: false,
  historyRetentionDays: 180,
  imageCacheMaxSizeMb: 256,
  effectiveImageCacheMaxSizeBytes: 256 * 1024 * 1024,
  effectiveImageCacheMaxSizeMb: 256,
  imageCacheMaxSizeEnvOverrideActive: false,
  pluginHttpCaBundlePem: "",
  pluginHttpTrustedCertificates: [],
};

type SettingsOverviewContainerProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export function SettingsOverviewContainer({
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const {
    uiSettings,
    uiSettingsLoaded,
    uiSettingsLoading,
    setUiSettings,
  } = useUiSettings();
  const refreshInstanceFeatures = useRefreshInstanceFeatures();
  const [pendingLanguage, setPendingLanguage] = React.useState<string | null>(null);
  const [rehydrating, setRehydrating] = React.useState(false);
  const [uiSettingsSaving, setUiSettingsSaving] = React.useState(false);
  const [generalSettings, setGeneralSettings] = React.useState<GeneralSettings>(
    DEFAULT_GENERAL_SETTINGS,
  );
  const [generalLoading, setGeneralLoading] = React.useState(true);
  const [generalSaving, setGeneralSaving] = React.useState(false);
  const [imageCacheClearing, setImageCacheClearing] = React.useState(false);
  // FR-040–FR-047: the import-copy verification depth. Its own query and
  // mutation, so it loads and saves independently of the general settings blob.
  const [verificationDepth, setVerificationDepth] = React.useState<VerificationDepth>("FULL");
  const [verificationLoading, setVerificationLoading] = React.useState(true);
  const [verificationSaving, setVerificationSaving] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(generalSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        setGeneralSettings({
          ...DEFAULT_GENERAL_SETTINGS,
          ...data?.generalSettings,
        });
      } catch {
        if (!cancelled) {
          setGeneralSettings(DEFAULT_GENERAL_SETTINGS);
        }
      } finally {
        if (!cancelled) setGeneralLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client
          .query<{ verificationSettings: { depth: VerificationDepth } }>(
            verificationSettingsQuery,
            {},
          )
          .toPromise();
        if (error) throw error;
        if (cancelled) return;
        setVerificationDepth(data?.verificationSettings.depth ?? "FULL");
      } catch {
        // The default is the safe one: a failed read must never make the
        // control claim the weaker guarantee is in force.
        if (!cancelled) setVerificationDepth("FULL");
      } finally {
        if (!cancelled) setVerificationLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  const handleVerificationDepthChange = React.useCallback(
    async (depth: VerificationDepth) => {
      if (verificationLoading || verificationSaving || depth === verificationDepth) {
        return;
      }
      const previous = verificationDepth;
      setVerificationDepth(depth);
      setVerificationSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ updateVerificationSettings: { depth: VerificationDepth } }>(
            updateVerificationSettingsMutation,
            { input: { depth } },
          )
          .toPromise();
        if (error) throw error;
        setVerificationDepth(data?.updateVerificationSettings.depth ?? depth);
        setGlobalStatus(t("settings.verificationDepthSaved"));
      } catch (error) {
        setVerificationDepth(previous);
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setVerificationSaving(false);
      }
    },
    [client, setGlobalStatus, t, verificationDepth, verificationLoading, verificationSaving],
  );

  const handleLanguageSelect = React.useCallback((code: string) => {
    if (code === uiLanguage) return;
    setPendingLanguage(code);
  }, [uiLanguage]);

  const handleConfirmLanguageChange = React.useCallback(async () => {
    if (!pendingLanguage) return;
    setRehydrating(true);
    try {
      // Change UI language immediately
      onSelectLanguage(pendingLanguage);

      // Trigger backend metadata rehydration
      const { error } = await client.mutation(
        rehydrateAllMetadataMutation,
        { input: { language: pendingLanguage } },
      ).toPromise();

      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("settings.metadataRehydrationStarted"));
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setRehydrating(false);
      setPendingLanguage(null);
    }
  }, [client, onSelectLanguage, pendingLanguage, setGlobalStatus, t]);

  const pendingLanguageLabel = pendingLanguage
    ? availableLanguages.find((l) => l.code === pendingLanguage)?.label ?? pendingLanguage
    : "";

  const handleDateTimeFormatChange = React.useCallback(
    async (dateTimeFormat: UiDateTimeFormat) => {
      if (
        !uiSettingsLoaded ||
        uiSettingsLoading ||
        uiSettingsSaving ||
        dateTimeFormat === uiSettings.dateTimeFormat
      ) {
        return;
      }

      const nextSettings: UiSettings = {
        ...uiSettings,
        dateTimeFormat,
      };
      setUiSettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ setMyUiSettings: UiSettings }>(setMyUiSettingsMutation, {
            input: uiSettingsInputFromSettings(nextSettings),
          })
          .toPromise();
        if (error) throw error;
        setUiSettings(data?.setMyUiSettings ?? nextSettings);
        setGlobalStatus(t("settings.uiSaved"));
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setUiSettingsSaving(false);
      }
    },
    [
      client,
      setGlobalStatus,
      setUiSettings,
      t,
      uiSettings,
      uiSettingsLoaded,
      uiSettingsLoading,
      uiSettingsSaving,
    ],
  );

  const handleSaveGeneralSettings = React.useCallback(async (update: GeneralSettingsUpdate) => {
    if (update.historyRetentionDays !== undefined && update.historyRetentionDays < 1) {
      setGlobalStatus(t("settings.historyRetentionValidation"));
      return;
    }
    if (update.imageCacheMaxSizeMb !== undefined && update.imageCacheMaxSizeMb < 1) {
      setGlobalStatus(t("settings.imageCacheMaxSizeValidation"));
      return;
    }

    setGeneralSaving(true);
    try {
      const { data, error } = await client
        .mutation(updateGeneralSettingsMutation, {
          input: update,
        })
        .toPromise();
      if (error) throw error;
      setGeneralSettings({
        ...DEFAULT_GENERAL_SETTINGS,
        ...data?.updateGeneralSettings,
      });
      setGlobalStatus(t("settings.generalSaved"));
      // The instance-wide switches are read app-wide through their own
      // actor-only query, so a save has to push the new value into the
      // provider for the gated surfaces to react without a reload.
      if (
        update.experimentalFeaturesEnabled !== undefined ||
        update.personalizedDiscoveryEnabled !== undefined
      ) {
        await refreshInstanceFeatures();
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setGeneralSaving(false);
    }
  }, [client, refreshInstanceFeatures, setGlobalStatus, t]);

  const handleClearImageCache = React.useCallback(async () => {
    if (imageCacheClearing) return;
    setImageCacheClearing(true);
    try {
      const { error } = await client.mutation(clearTitleImageCacheMutation, {}).toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.imageCacheClearQueued"));
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setImageCacheClearing(false);
    }
  }, [client, imageCacheClearing, setGlobalStatus, t]);

  return (
    <>
      <SettingsOverviewSection
        availableLanguages={availableLanguages}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={handleLanguageSelect}
        dateTimeFormat={uiSettings.dateTimeFormat}
        dateTimeFormatLoading={uiSettingsLoading || !uiSettingsLoaded}
        dateTimeFormatSaving={uiSettingsSaving}
        onDateTimeFormatChange={handleDateTimeFormatChange}
        generalSettings={generalSettings}
        onGeneralSettingsChange={setGeneralSettings}
        generalLoading={generalLoading}
        generalSaving={generalSaving}
        imageCacheClearing={imageCacheClearing}
        onGeneralSettingsCommit={handleSaveGeneralSettings}
        onClearImageCache={handleClearImageCache}
        verificationDepth={verificationDepth}
        verificationLoading={verificationLoading}
        verificationSaving={verificationSaving}
        onVerificationDepthChange={handleVerificationDepthChange}
      />
      <ConfirmDialog
        open={pendingLanguage !== null}
        title={t("settings.languageChangeTitle")}
        description={t("settings.languageChangeWarning", { language: pendingLanguageLabel })}
        confirmLabel={t("settings.languageChangeConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={rehydrating}
        onConfirm={handleConfirmLanguageChange}
        onCancel={() => setPendingLanguage(null)}
      />
    </>
  );
}
