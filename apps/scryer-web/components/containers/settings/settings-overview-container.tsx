
import * as React from "react";
import { SettingsOverviewSection } from "@/components/views/settings/settings-overview-section";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { tlsSettingsQuery } from "@/lib/graphql/queries";
import {
  rehydrateAllMetadataMutation,
  updateServiceSettingsMutation,
} from "@/lib/graphql/mutations";
import type { ServiceSettings } from "@/lib/types/settings";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

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
  const [tlsCertPath, setTlsCertPath] = React.useState("");
  const [tlsKeyPath, setTlsKeyPath] = React.useState("");
  const [tlsSaving, setTlsSaving] = React.useState(false);
  const [pendingLanguage, setPendingLanguage] = React.useState<string | null>(null);
  const [rehydrating, setRehydrating] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(tlsSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const serviceSettings = data.serviceSettings as ServiceSettings | undefined;
        setTlsCertPath(serviceSettings?.tlsCertPath ?? "");
        setTlsKeyPath(serviceSettings?.tlsKeyPath ?? "");
      } catch {
        // TLS settings are optional — silently ignore load failures
      }
    })();
    return () => { cancelled = true; };
  }, [client]);

  const handleTlsSave = React.useCallback(async () => {
    setTlsSaving(true);
    try {
      const { error } = await client
        .mutation(updateServiceSettingsMutation, {
          input: {
            tlsCertPath: tlsCertPath.trim(),
            tlsKeyPath: tlsKeyPath.trim(),
          },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.tlsSaved"));
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setTlsSaving(false);
    }
  }, [client, setGlobalStatus, t, tlsCertPath, tlsKeyPath]);

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
        { language: pendingLanguage },
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

  return (
    <>
      <SettingsOverviewSection
        availableLanguages={availableLanguages}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={handleLanguageSelect}
        tlsCertPath={tlsCertPath}
        setTlsCertPath={setTlsCertPath}
        tlsKeyPath={tlsKeyPath}
        setTlsKeyPath={setTlsKeyPath}
        tlsSaving={tlsSaving}
        onTlsSave={handleTlsSave}
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
