
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SettingsIndexersSection } from "@/components/views/settings/settings-indexers-section";
import { SettingsDownloadClientsSection } from "@/components/views/settings/settings-download-clients-section";
import { SettingsOverviewSection } from "@/components/views/settings/settings-overview-section";
import { SettingsQualityProfilesSection } from "@/components/views/settings/settings-quality-profiles-section";
import { SettingsUsersSection } from "@/components/views/settings/settings-users-section";
import type { LocaleCode } from "@/lib/i18n";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

export function SettingsView({
  state,
}: {
  state: Record<string, unknown>;
}) {
  const {
    t,
    settingsSection,
    settingsUsers,
    settingsIndexers,
    settingsDownloadClients,
    downloadClientDraft,
    setDownloadClientDraft,
    mutatingDownloadClientId,
    isTestingDownloadClientConnection,
    editingDownloadClientId,
    resetDownloadClientDraft,
    submitDownloadClient,
    testDownloadClientConnection,
    editDownloadClient,
    toggleDownloadClientEnabled,
    deleteDownloadClient,
    ALL_ENTITLEMENTS,
    humanizeEntitlement,
    newUsername,
    setNewUsername,
    newPassword,
    setNewPassword,
    newEntitlements,
    toggleNewEntitlement,
    createUser,
    userPasswordDrafts,
    userEntitlementDrafts,
    updateUserPasswordDraft,
    toggleUserEntitlement,
    mutatingUserId,
    setUserPassword,
    setUserEntitlements,
    deleteUser,
    qualityProfiles,
    qualityProfileParseError,
    getQualityProfileCriteria,
    getQualityProfileBoolean,
    loadQualityProfileById,
    activeQualityProfileTierOptions,
    availableQualityTiers,
    updateQualityProfileDraft,
    qualityProfileDraft,
    availableSourceAllowlist,
    availableVideoCodecAllowlist,
    availableAudioCodecAllowlist,
    activeSourceAllowlist,
    activeSourceBlocklist,
    activeVideoCodecAllowlist,
    activeVideoCodecBlocklist,
    activeAudioCodecAllowlist,
    activeAudioCodecBlocklist,
    qualityCategoryLabels,
    moveProfileListToAllowed,
    moveProfileListToDenied,
    addQualityTier,
    removeQualityTier,
    qualityProfileInheritValue,
    toProfileOptions,
    globalQualityProfileId,
    setGlobalQualityProfileId,
    categoryQualityProfileOverrides,
    setCategoryQualityProfileOverrides,
    mediaSettingsLoading,
    qualityProfilesSaving,
    updateQualityProfilesGlobal,
    categoryQualityProfileSaving,
    saveCategoryQualityProfile,
    saveGlobalQualityProfile,
    deleteQualityProfile,
    settingsIndexerFilter,
    setSettingsIndexerFilter,
    indexerDraft,
    setIndexerDraft,
    mutatingIndexerId,
    editingIndexerId,
    resetIndexerDraft,
    submitIndexer,
    editIndexer,
    toggleIndexerEnabled,
    deleteIndexer,
    archivalQualityOptions,
    initialLoadComplete,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } = state as Record<string, any>;

  return (
    <Card className="bg-card border-border">
      <CardHeader>
        <CardTitle>
          {t("settings.sectionTitle", {
            section:
              settingsSection === "general"
                ? t("settings.general")
                : settingsSection === "users"
                  ? t("settings.users")
                  : settingsSection === "indexers"
              ? t("settings.indexers")
              : settingsSection === "downloadClients"
                ? t("settings.downloadClients")
                : t("settings.qualityProfiles"),
          })}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {settingsSection === "general" ? (
          <SettingsOverviewSection
            t={t as Translate}
            availableLanguages={[]}
            selectedLanguage={null}
            uiLanguage={"eng" as LocaleCode}
            onSelectLanguage={() => undefined}
          />
        ) : settingsSection === "users" ? (
          <SettingsUsersSection
            t={t as Translate}
            settingsUsers={settingsUsers}
            ALL_ENTITLEMENTS={ALL_ENTITLEMENTS}
            humanizeEntitlement={humanizeEntitlement}
            newUsername={newUsername}
            setNewUsername={setNewUsername}
            newPassword={newPassword}
            setNewPassword={setNewPassword}
            newEntitlements={newEntitlements}
            toggleNewEntitlement={toggleNewEntitlement}
            createUser={createUser}
            userPasswordDrafts={userPasswordDrafts}
            userEntitlementDrafts={userEntitlementDrafts}
            updateUserPasswordDraft={updateUserPasswordDraft}
            toggleUserEntitlement={toggleUserEntitlement}
            mutatingUserId={mutatingUserId}
            setUserPassword={setUserPassword}
            setUserEntitlements={setUserEntitlements}
            deleteUser={deleteUser}
          />
        ) : settingsSection === "qualityProfiles" ? (
          <SettingsQualityProfilesSection
            t={t as Translate}
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
            qualityProfileInheritValue={qualityProfileInheritValue}
            toProfileOptions={toProfileOptions}
            globalQualityProfileId={globalQualityProfileId}
            setGlobalQualityProfileId={setGlobalQualityProfileId}
            categoryQualityProfileOverrides={categoryQualityProfileOverrides}
            setCategoryQualityProfileOverrides={setCategoryQualityProfileOverrides}
            mediaSettingsLoading={mediaSettingsLoading}
            qualityProfilesSaving={qualityProfilesSaving}
            updateQualityProfilesGlobal={updateQualityProfilesGlobal}
            saveGlobalQualityProfile={saveGlobalQualityProfile}
            categoryQualityProfileSaving={categoryQualityProfileSaving}
            saveCategoryQualityProfile={saveCategoryQualityProfile}
            archivalQualityOptions={archivalQualityOptions}
            initialLoadComplete={initialLoadComplete ?? true}
            deleteQualityProfile={deleteQualityProfile as (profileId: string) => Promise<void>}
          />
        ) : settingsSection === "downloadClients" ? (
          <SettingsDownloadClientsSection
            t={t as Translate}
            editingDownloadClientId={editingDownloadClientId}
            downloadClientDraft={downloadClientDraft}
            setDownloadClientDraft={setDownloadClientDraft}
            submitDownloadClient={submitDownloadClient}
            testDownloadClientConnection={testDownloadClientConnection}
            isTestingDownloadClientConnection={isTestingDownloadClientConnection}
            mutatingDownloadClientId={mutatingDownloadClientId}
            resetDownloadClientDraft={resetDownloadClientDraft}
            settingsDownloadClients={settingsDownloadClients}
            editDownloadClient={editDownloadClient}
            toggleDownloadClientEnabled={toggleDownloadClientEnabled}
            deleteDownloadClient={deleteDownloadClient}
          />
        ) : (
          <SettingsIndexersSection
            t={t as Translate}
            editingIndexerId={editingIndexerId}
            indexerDraft={indexerDraft}
            setIndexerDraft={setIndexerDraft}
            submitIndexer={submitIndexer}
            mutatingIndexerId={mutatingIndexerId}
            resetIndexerDraft={resetIndexerDraft}
            settingsIndexerFilter={settingsIndexerFilter}
            setSettingsIndexerFilter={setSettingsIndexerFilter}
            settingsIndexers={settingsIndexers}
            editIndexer={editIndexer}
            toggleIndexerEnabled={toggleIndexerEnabled}
            deleteIndexer={deleteIndexer}
            providerTypes={[]}
          />
        )}
      </CardContent>
    </Card>
  );
}
