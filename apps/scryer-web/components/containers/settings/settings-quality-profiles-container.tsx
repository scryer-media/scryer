
import { SettingsQualityProfilesSection } from "@/components/views/settings/settings-quality-profiles-section";
import { useQualityProfilesManager } from "@/lib/hooks/use-quality-profiles-manager";
import type { Translate } from "@/components/root/types";
import { QUALITY_PROFILE_INHERIT_VALUE } from "@/lib/constants/settings";

type SettingsQualityProfilesContainerProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
};

export function SettingsQualityProfilesContainer({
  t,
  setGlobalStatus,
}: SettingsQualityProfilesContainerProps) {
  const {
    mediaSettingsLoading,
    initialLoadComplete,
    qualityProfilesSaving,
    qualityProfiles,
    qualityProfileParseError,
    qualityProfileDraft,
    availableSourceAllowlist,
    availableVideoCodecAllowlist,
    availableAudioCodecAllowlist,
    activeQualityProfileTierOptions,
    availableQualityTiers,
    archivalQualityOptions,
    activeSourceAllowlist,
    activeSourceBlocklist,
    activeVideoCodecAllowlist,
    activeVideoCodecBlocklist,
    activeAudioCodecAllowlist,
    activeAudioCodecBlocklist,
    qualityCategoryLabels,
    getQualityProfileCriteria,
    getQualityProfileBoolean,
    loadQualityProfileById,
    moveProfileListToAllowed,
    moveProfileListToDenied,
    addQualityTier,
    removeQualityTier,
    updateQualityProfilesGlobal,
    saveGlobalQualityProfile,
    updateQualityProfileDraft,
    globalQualityProfileId,
    setGlobalQualityProfileId,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    categoryQualityProfileSaving,
    deleteQualityProfile,
    saveCategoryQualityProfile,
    toProfileOptions,
  } = useQualityProfilesManager({ setGlobalStatus, t });

  return (
    <SettingsQualityProfilesSection
      t={t}
      qualityProfiles={qualityProfiles}
      qualityProfileParseError={qualityProfileParseError}
      getQualityProfileCriteria={getQualityProfileCriteria}
      getQualityProfileBoolean={getQualityProfileBoolean}
      loadQualityProfileById={loadQualityProfileById}
      activeQualityProfileTierOptions={activeQualityProfileTierOptions}
      availableQualityTiers={availableQualityTiers}
      updateQualityProfileDraft={updateQualityProfileDraft}
      qualityProfileDraft={qualityProfileDraft}
      availableSourceAllowlist={availableSourceAllowlist}
      availableVideoCodecAllowlist={availableVideoCodecAllowlist}
      availableAudioCodecAllowlist={availableAudioCodecAllowlist}
      activeSourceAllowlist={activeSourceAllowlist}
      activeSourceBlocklist={activeSourceBlocklist}
      activeVideoCodecAllowlist={activeVideoCodecAllowlist}
      activeVideoCodecBlocklist={activeVideoCodecBlocklist}
      activeAudioCodecAllowlist={activeAudioCodecAllowlist}
      activeAudioCodecBlocklist={activeAudioCodecBlocklist}
      qualityCategoryLabels={qualityCategoryLabels}
      moveProfileListToAllowed={moveProfileListToAllowed}
      moveProfileListToDenied={moveProfileListToDenied}
      addQualityTier={addQualityTier}
      removeQualityTier={removeQualityTier}
      qualityProfileInheritValue={QUALITY_PROFILE_INHERIT_VALUE}
      toProfileOptions={toProfileOptions}
      globalQualityProfileId={globalQualityProfileId}
      setGlobalQualityProfileId={setGlobalQualityProfileId}
      categoryQualityProfileOverrides={categoryQualityProfileOverrides}
      setCategoryQualityProfileOverrides={setCategoryQualityProfileOverrides}
    mediaSettingsLoading={mediaSettingsLoading}
    initialLoadComplete={initialLoadComplete}
    qualityProfilesSaving={qualityProfilesSaving}
    updateQualityProfilesGlobal={updateQualityProfilesGlobal}
    saveGlobalQualityProfile={saveGlobalQualityProfile}
    categoryQualityProfileSaving={categoryQualityProfileSaving}
    saveCategoryQualityProfile={saveCategoryQualityProfile}
    archivalQualityOptions={archivalQualityOptions}
    deleteQualityProfile={deleteQualityProfile}
  />
  );
}
