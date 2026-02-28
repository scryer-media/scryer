
import { memo } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SettingsOverviewContainer } from "@/components/containers/settings/settings-overview-container";
import { SettingsUsersContainer } from "@/components/containers/settings/settings-users-container";
import { SettingsIndexersContainer } from "@/components/containers/settings/settings-indexers-container";
import { SettingsDownloadClientsContainer } from "@/components/containers/settings/settings-download-clients-container";
import { SettingsQualityProfilesContainer } from "@/components/containers/settings/settings-quality-profiles-container";
import { SettingsAcquisitionContainer } from "@/components/containers/settings/settings-acquisition-container";
import { SettingsProfileContainer } from "@/components/containers/settings/settings-profile-container";
import { SettingsRulesContainer } from "@/components/containers/settings/settings-rules-container";
import type { SettingsSection, Translate } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

type SettingsContainerProps = {
  settingsSection: SettingsSection;
  t: Translate;
  setGlobalStatus: (status: string) => void;
  userId?: string;
  username?: string;
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export const SettingsContainer = memo(function SettingsContainer({
  settingsSection,
  t,
  setGlobalStatus,
  userId,
  username,
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsContainerProps) {
  return (
    <Card className="bg-card border-border">
      <CardHeader>
        <CardTitle>
          {t("settings.sectionTitle", {
            section:
              settingsSection === "profile"
                ? t("settings.profile")
                : settingsSection === "general"
                  ? t("settings.general")
                : settingsSection === "users"
                  ? t("settings.users")
                : settingsSection === "indexers"
                  ? t("settings.indexers")
                : settingsSection === "downloadClients"
                  ? t("settings.downloadClients")
                : settingsSection === "acquisition"
                  ? t("settings.acquisition")
                : settingsSection === "rules"
                  ? t("settings.rules")
                    : t("settings.qualityProfiles"),
          })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {settingsSection === "profile" ? (
          <SettingsProfileContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
            userId={userId}
            username={username}
          />
        ) : settingsSection === "general" ? (
          <SettingsOverviewContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
            availableLanguages={availableLanguages}
            selectedLanguage={selectedLanguage}
            uiLanguage={uiLanguage}
            onSelectLanguage={onSelectLanguage}
          />
        ) : settingsSection === "users" ? (
          <SettingsUsersContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        ) : settingsSection === "indexers" ? (
          <SettingsIndexersContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        ) : settingsSection === "downloadClients" ? (
          <SettingsDownloadClientsContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        ) : settingsSection === "acquisition" ? (
          <SettingsAcquisitionContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        ) : settingsSection === "rules" ? (
          <SettingsRulesContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        ) : (
          <SettingsQualityProfilesContainer
            t={t}
            setGlobalStatus={setGlobalStatus}
          />
        )}
      </CardContent>
    </Card>
  );
});
