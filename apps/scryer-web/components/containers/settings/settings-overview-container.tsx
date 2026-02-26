
import * as React from "react";
import { SettingsOverviewSection } from "@/components/views/settings/settings-overview-section";
import { tlsSettingsQuery } from "@/lib/graphql/queries";
import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import type { AdminSetting } from "@/lib/types/admin-settings";
import { TLS_CERT_PATH_KEY, TLS_KEY_PATH_KEY } from "@/lib/constants/settings";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { useClient } from "urql";
import type { Translate } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

type SettingsOverviewContainerProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export function SettingsOverviewContainer({
  t,
  setGlobalStatus,
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsOverviewContainerProps) {
  const client = useClient();
  const [tlsCertPath, setTlsCertPath] = React.useState("");
  const [tlsKeyPath, setTlsKeyPath] = React.useState("");
  const [tlsSaving, setTlsSaving] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(tlsSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const certRecord = data.serviceSettings.items.find(
          (item: AdminSetting) => item.keyName === TLS_CERT_PATH_KEY,
        );
        const keyRecord = data.serviceSettings.items.find(
          (item: AdminSetting) => item.keyName === TLS_KEY_PATH_KEY,
        );
        setTlsCertPath(getSettingDisplayValue(certRecord));
        setTlsKeyPath(getSettingDisplayValue(keyRecord));
      } catch {
        // TLS settings are optional — silently ignore load failures
      }
    })();
    return () => { cancelled = true; };
  }, [client]);

  const handleTlsSave = React.useCallback(async () => {
    setTlsSaving(true);
    try {
      const { error } = await client.mutation(
        saveAdminSettingsMutation,
        {
          input: {
            scope: "system",
            items: [
              { keyName: TLS_CERT_PATH_KEY, value: tlsCertPath.trim() },
              { keyName: TLS_KEY_PATH_KEY, value: tlsKeyPath.trim() },
            ],
          },
        },
      ).toPromise();
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

  return (
    <SettingsOverviewSection
      t={t}
      availableLanguages={availableLanguages}
      selectedLanguage={selectedLanguage}
      uiLanguage={uiLanguage}
      onSelectLanguage={onSelectLanguage}
      tlsCertPath={tlsCertPath}
      setTlsCertPath={setTlsCertPath}
      tlsKeyPath={tlsKeyPath}
      setTlsKeyPath={setTlsKeyPath}
      tlsSaving={tlsSaving}
      onTlsSave={handleTlsSave}
    />
  );
}
