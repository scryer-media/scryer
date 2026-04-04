
import { SettingsQualityProfilesSection } from "@/components/views/settings/settings-quality-profiles-section";
import { useQualityProfilesManager } from "@/lib/hooks/use-quality-profiles-manager";
import { QUALITY_PROFILE_INHERIT_VALUE } from "@/lib/constants/settings";

export function SettingsQualityProfilesContainer() {
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
    saveGlobalScoringPersona,
    updateQualityProfileDraft,
    globalQualityProfileId,
    setGlobalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    categoryPersonaSelections,
    categoryQualityProfileSaving,
    deleteQualityProfile,
    saveCategoryQualityProfile,
    saveCategoryScoringPersona,
    toProfileOptions,
  } = useQualityProfilesManager();

  return (
    <SettingsQualityProfilesSection
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
      globalScoringPersona={globalScoringPersona}
      categoryQualityProfileOverrides={categoryQualityProfileOverrides}
      setCategoryQualityProfileOverrides={setCategoryQualityProfileOverrides}
      categoryPersonaSelections={categoryPersonaSelections}
    mediaSettingsLoading={mediaSettingsLoading}
    initialLoadComplete={initialLoadComplete}
    qualityProfilesSaving={qualityProfilesSaving}
    updateQualityProfilesGlobal={updateQualityProfilesGlobal}
    saveGlobalQualityProfile={saveGlobalQualityProfile}
    saveGlobalScoringPersona={saveGlobalScoringPersona}
    categoryQualityProfileSaving={categoryQualityProfileSaving}
    saveCategoryQualityProfile={saveCategoryQualityProfile}
    saveCategoryScoringPersona={saveCategoryScoringPersona}
    archivalQualityOptions={archivalQualityOptions}
    deleteQualityProfile={deleteQualityProfile}
  />
  );
}
