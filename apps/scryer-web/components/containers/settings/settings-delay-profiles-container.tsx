import { SettingsDelayProfilesSection } from "@/components/views/settings/settings-delay-profiles-section";
import { useDelayProfilesManager } from "@/lib/hooks/use-delay-profiles-manager";

export function SettingsDelayProfilesContainer() {
  const manager = useDelayProfilesManager();

  return (
    <SettingsDelayProfilesSection
      loading={manager.loading}
      saving={manager.saving}
      profiles={manager.profiles}
      parseError={manager.parseError}
      draft={manager.draft}
      setDraft={manager.setDraft}
      saveProfile={manager.saveProfile}
      deleteProfile={manager.deleteProfile}
      loadProfileById={manager.loadProfileById}
      resetDraft={manager.resetDraft}
    />
  );
}
