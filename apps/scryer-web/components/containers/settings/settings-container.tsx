
import { memo } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SettingsOverviewContainer } from "@/components/containers/settings/settings-overview-container";
import { SettingsUsersContainer } from "@/components/containers/settings/settings-users-container";
import { SettingsIndexersContainer } from "@/components/containers/settings/settings-indexers-container";
import { SettingsDownloadClientsContainer } from "@/components/containers/settings/settings-download-clients-container";
import { SettingsDelayProfilesContainer } from "@/components/containers/settings/settings-delay-profiles-container";
import { SettingsQualityProfilesContainer } from "@/components/containers/settings/settings-quality-profiles-container";
import { SettingsAcquisitionContainer } from "@/components/containers/settings/settings-acquisition-container";
import { SettingsProfileContainer } from "@/components/containers/settings/settings-profile-container";
import { SettingsRulesContainer } from "@/components/containers/settings/settings-rules-container";
import { SettingsPluginsContainer } from "@/components/containers/settings/settings-plugins-container";
import { SettingsNotificationsContainer } from "@/components/containers/settings/settings-notifications-container";
import { SettingsPostProcessingContainer } from "@/components/containers/settings/settings-post-processing-container";
import type { SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";

type SettingsContainerProps = {
  settingsSection: SettingsSection;
  userId?: string;
  username?: string;
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export const SettingsContainer = memo(function SettingsContainer({
  settingsSection,
  userId,
  username,
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsContainerProps) {
  const t = useTranslate();
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
                : settingsSection === "plugins"
                  ? t("settings.plugins")
                : settingsSection === "notifications"
                  ? t("settings.notifications")
                : settingsSection === "post-processing"
                  ? t("settings.postProcessing")
                : settingsSection === "delayProfiles"
                  ? t("settings.delayProfiles")
                    : t("settings.qualityProfiles"),
          })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {settingsSection === "profile" ? (
          <SettingsProfileContainer
            userId={userId}
            username={username}
          />
        ) : settingsSection === "general" ? (
          <SettingsOverviewContainer
            availableLanguages={availableLanguages}
            selectedLanguage={selectedLanguage}
            uiLanguage={uiLanguage}
            onSelectLanguage={onSelectLanguage}
          />
        ) : settingsSection === "users" ? (
          <SettingsUsersContainer />
        ) : settingsSection === "indexers" ? (
          <SettingsIndexersContainer />
        ) : settingsSection === "downloadClients" ? (
          <SettingsDownloadClientsContainer />
        ) : settingsSection === "acquisition" ? (
          <SettingsAcquisitionContainer />
        ) : settingsSection === "rules" ? (
          <SettingsRulesContainer />
        ) : settingsSection === "plugins" ? (
          <SettingsPluginsContainer />
        ) : settingsSection === "notifications" ? (
          <SettingsNotificationsContainer />
        ) : settingsSection === "post-processing" ? (
          <SettingsPostProcessingContainer />
        ) : settingsSection === "delayProfiles" ? (
          <SettingsDelayProfilesContainer />
        ) : (
          <SettingsQualityProfilesContainer />
        )}
      </CardContent>
    </Card>
  );
});
