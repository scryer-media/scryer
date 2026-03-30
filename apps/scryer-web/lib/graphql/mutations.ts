export const loginMutation = `mutation Login($input: LoginInput!) {
  login(input: $input) {
    token
    user {
      id
      username
      entitlements
    }
    expiresAt
  }
}`;

export const createUserMutation = `mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    username
    entitlements
  }
}`;

export const setUserPasswordMutation = `mutation SetUserPassword($input: SetUserPasswordInput!) {
  setUserPassword(input: $input) {
    id
    username
    entitlements
  }
}`;

export const setUserEntitlementsMutation = `mutation SetUserEntitlements($input: SetUserEntitlementsInput!) {
  setUserEntitlements(input: $input) {
    id
    username
    entitlements
  }
}`;

export const deleteUserMutation = `mutation DeleteUser($input: DeleteUserInput!) {
  deleteUser(input: $input)
}`;

export const deleteTitleMutation = `mutation DeleteTitle($input: DeleteTitleInput!) {
  deleteTitle(input: $input)
}`;

export const createIndexerMutation = `mutation CreateIndexer($input: CreateIndexerConfigInput!) {
  createIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const updateIndexerMutation = `mutation UpdateIndexer($input: UpdateIndexerConfigInput!) {
  updateIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    hasApiKey
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const deleteIndexerMutation = `mutation DeleteIndexer($input: DeleteIndexerConfigInput!) {
  deleteIndexerConfig(input: $input)
}`;

export const testIndexerConnectionMutation = `mutation TestIndexerConnection($input: TestIndexerConnectionInput!) {
  testIndexerConnection(input: $input)
}`;

export const createDownloadClientMutation = `mutation CreateDownloadClient($input: CreateDownloadClientConfigInput!) {
  createDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    configJson
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const updateDownloadClientMutation = `mutation UpdateDownloadClient($input: UpdateDownloadClientConfigInput!) {
  updateDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    configJson
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const testDownloadClientConnectionMutation = `mutation TestDownloadClientConnection($input: TestDownloadClientConnectionInput!) {
  testDownloadClientConnection(input: $input)
}`;

export const deleteDownloadClientMutation = `mutation DeleteDownloadClient($input: DeleteDownloadClientConfigInput!) {
  deleteDownloadClientConfig(input: $input)
}`;

export const reorderDownloadClientsMutation = `mutation ReorderDownloadClients($input: ReorderDownloadClientConfigsInput!) {
  reorderDownloadClientConfigs(input: $input)
}`;

export const addTitleMutation = `mutation AddTitle($input: AddTitleInput!) {
  addTitle(input: $input) {
    title { id name facet minAvailability }
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const addTitleAndQueueMutation = `mutation AddTitleAndQueue($input: AddTitleInput!) {
  addTitleAndQueueDownload(input: $input) {
    title { id name facet }
    downloadJobId
    queuedDownload {
      jobId
      titleId
      titleName
      sourceTitle
      sourceKind
    }
  }
}`;

export const deleteMediaFileMutation = `mutation DeleteMediaFile($input: DeleteMediaFileInput!) {
  deleteMediaFile(input: $input)
}`;

export const scanLibraryMutation = `mutation ScanLibrary($facet: MediaFacetValue!) {
  scanLibrary(facet: $facet) {
    scanned
    matched
    imported
    skipped
    unmatched
  }
}`;

export const scanTitleLibraryMutation = `mutation ScanTitleLibrary($input: TitleIdInput!) {
  scanTitleLibrary(input: $input) {
    scanned
    matched
    imported
    skipped
    unmatched
  }
}`;

export const applyMediaRenameMutation = `mutation ApplyMediaRename($input: MediaRenameApplyInput!) {
  applyMediaRename(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const applyMediaRenameBulkMutation = `mutation ApplyMediaRenameBulk($input: MediaRenameBulkApplyInput!) {
  applyMediaRenameBulk(input: $input) {
    planFingerprint
    total
    applied
    skipped
    failed
    items {
      collectionId
      currentPath
      proposedPath
      finalPath
      writeAction
      status
      reasonCode
      errorMessage
    }
  }
}`;

export const updateSubtitleSettingsMutation = `mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
  updateSubtitleSettings(input: $input) {
    enabled
    hasOpenSubtitlesApiKey
    openSubtitlesUsername
    hasOpenSubtitlesPassword
    languages {
      code
      hearingImpaired
      forced
    }
    autoDownloadOnImport
    minimumScoreSeries
    minimumScoreMovie
    searchIntervalHours
    includeAiTranslated
    includeMachineTranslated
    syncEnabled
    syncThresholdSeries
    syncThresholdMovie
    syncMaxOffsetSeconds
  }
}`;

export const updateAcquisitionSettingsMutation = `mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
  updateAcquisitionSettings(input: $input) {
    enabled
    upgradeCooldownHours
    sameTierMinDelta
    crossTierMinDelta
    forcedUpgradeDeltaBypass
    pollIntervalSeconds
    syncIntervalSeconds
    batchSize
  }
}`;

export const upsertDelayProfileMutation = `mutation UpsertDelayProfile($input: DelayProfileInput!) {
  upsertDelayProfile(input: $input) {
    id
    name
    usenetDelayMinutes
    torrentDelayMinutes
    preferredProtocol
    minAgeMinutes
    bypassScoreThreshold
    appliesToFacets
    tags
    priority
    enabled
  }
}`;

export const deleteDelayProfileMutation = `mutation DeleteDelayProfile($input: DeleteDelayProfileInput!) {
  deleteDelayProfile(input: $input) {
    id
  }
}`;

const qualityProfileCriteriaFields = `
      qualityTiers
      archivalQuality
      allowUnknownQuality
      sourceAllowlist
      sourceBlocklist
      videoCodecAllowlist
      videoCodecBlocklist
      audioCodecAllowlist
      audioCodecBlocklist
      dolbyVisionAllowed
      detectedHdrAllowed
      preferRemux
      allowBdDisk
      allowUpgrades
      requiredAudioLanguages
      scoringPersona
      scoringOverrides {
        allowX265Non4K
        blockDvWithoutFallback
        preferCompactEncodes
        preferLosslessAudio
        blockUpscaled
      }
      cutoffTier
      minScoreToGrab
      facetPersonaOverrides {
        scope
        persona
      }`;

const qualityProfileSettingsFieldSelection = `
    globalProfileId
    profiles {
      id
      name
      criteria {${qualityProfileCriteriaFields}
      }
    }
    categorySelections {
      scope
      overrideProfileId
      effectiveProfileId
      inheritsGlobal
    }`;

const downloadClientRoutingFieldSelection = `
    clientId
    enabled
    category
    recentQueuePriority
    olderQueuePriority
    removeCompleted
    removeFailed`;

const indexerRoutingFieldSelection = `
    indexerId
    enabled
    categories
    priority`;

const mediaSettingsFieldSelection = `
    scope
    libraryPath
    rootFolders {
      path
      isDefault
    }
    renameTemplate
    renameCollisionPolicy
    renameMissingMetadataPolicy
    fillerPolicy
    recapPolicy
    monitorSpecials
    interSeasonMovies
    monitorFillerMovies
    nfoWriteOnImport
    plexmatchWriteOnImport`;

const libraryPathsFieldSelection = `
    moviePath
    seriesPath
    animePath`;

const serviceSettingsFieldSelection = `
    tlsCertPath
    tlsKeyPath`;

export const saveQualityProfileSettingsMutation = `mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
  saveQualityProfileSettings(input: $input) {${qualityProfileSettingsFieldSelection}
  }
}`;

export const deleteQualityProfileMutation = `mutation DeleteQualityProfile($input: DeleteQualityProfileInput!) {
  deleteQualityProfile(input: $input) {
${qualityProfileSettingsFieldSelection}
  }
}`;

export const updateDownloadClientRoutingMutation = `mutation UpdateDownloadClientRouting($input: UpdateDownloadClientRoutingInput!) {
  updateDownloadClientRouting(input: $input) {${downloadClientRoutingFieldSelection}
  }
}`;

export const updateIndexerRoutingMutation = `mutation UpdateIndexerRouting($input: UpdateIndexerRoutingInput!) {
  updateIndexerRouting(input: $input) {${indexerRoutingFieldSelection}
  }
}`;

export const updateMediaSettingsMutation = `mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
  updateMediaSettings(input: $input) {${mediaSettingsFieldSelection}
  }
}`;

export const updateLibraryPathsMutation = `mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
  updateLibraryPaths(input: $input) {${libraryPathsFieldSelection}
  }
}`;

export const updateServiceSettingsMutation = `mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
  updateServiceSettings(input: $input) {${serviceSettingsFieldSelection}
  }
}`;

export const queueExistingMutation = `mutation QueueExisting($input: QueueDownloadInput!) {
  queueExistingTitleDownload(input: $input) {
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
  }
}`;

export const queueManualImportMutation = `mutation QueueManualImport($input: QueueManualImportInput!) {
  queueManualImport(input: $input) {
    kind
    downloadClientItemId
    importId
    removed
    queueItem {
      id
      titleId
      titleName
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const pauseDownloadMutation = `mutation PauseDownload($input: PauseDownloadInput!) {
  pauseDownload(input: $input) {
    kind
    downloadClientItemId
    removed
    queueItem {
      id
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const resumeDownloadMutation = `mutation ResumeDownload($input: ResumeDownloadInput!) {
  resumeDownload(input: $input) {
    kind
    downloadClientItemId
    removed
    queueItem {
      id
      clientType
      downloadClientItemId
      state
    }
  }
}`;

export const deleteDownloadMutation = `mutation DeleteDownload($input: DeleteDownloadInput!) {
  deleteDownload(input: $input) {
    kind
    downloadClientItemId
    removed
    clientType
  }
}`;

export const setCollectionMonitoredMutation = `mutation SetCollectionMonitored($input: SetCollectionMonitoredInput!) {
  setCollectionMonitored(input: $input) {
    id
    monitored
    episodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      overview
      airDate
      durationSeconds
      hasMultiAudio
      hasSubtitle
      isFiller
      absoluteNumber
      monitored
      createdAt
    }
  }
}`;

export const setEpisodeMonitoredMutation = `mutation SetEpisodeMonitored($input: SetEpisodeMonitoredInput!) {
  setEpisodeMonitored(input: $input) { id monitored }
}`;

export const setTitleMonitoredMutation = `mutation SetTitleMonitored($input: SetTitleMonitoredInput!) {
  setTitleMonitored(input: $input) { id monitored }
}`;

export const updateTitleMutation = `mutation UpdateTitle($input: UpdateTitleInput!) {
  updateTitle(input: $input) {
    id
    name
    facet
    tags
    monitored
    qualityProfileId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
  }
}`;

export const triggerImportMutation = `mutation TriggerImport($input: TriggerImportInput!) {
  triggerImport(input: $input) {
    importId
    decision
    skipReason
    titleId
    sourcePath
    destPath
    fileSizeBytes
    linkType
    errorMessage
  }
}`;

export const executeManualImportMutation = `mutation ExecuteManualImport($input: ExecuteManualImportInput!) {
  executeManualImport(input: $input) {
    filePath
    episodeId
    success
    destPath
    errorMessage
  }
}`;

export const triggerWantedSearchMutation = `mutation TriggerWantedSearch($input: WantedItemIdInput!) {
  triggerWantedSearch(input: $input)
}`;

export const triggerTitleWantedSearchMutation = `mutation TriggerTitleWantedSearch($input: TitleIdInput!) {
  triggerTitleWantedSearch(input: $input)
}`;

export const triggerSeasonWantedSearchMutation = `mutation TriggerSeasonWantedSearch($input: SeasonSearchInput!) {
  triggerSeasonWantedSearch(input: $input)
}`;

export const pauseWantedItemMutation = `mutation PauseWantedItem($input: WantedItemIdInput!) {
  pauseWantedItem(input: $input)
}`;

export const resumeWantedItemMutation = `mutation ResumeWantedItem($input: WantedItemIdInput!) {
  resumeWantedItem(input: $input)
}`;

export const resetWantedItemMutation = `mutation ResetWantedItem($input: WantedItemIdInput!) {
  resetWantedItem(input: $input)
}`;

// ── RSS Sync ─────────────────────────────────────────────────────────────

export const triggerRssSyncMutation = `mutation TriggerRssSync {
  triggerRssSync {
    releasesFetched
    releasesMatched
    releasesGrabbed
    releasesHeld
  }
}`;

// ── Pending Releases ─────────────────────────────────────────────────────

export const forceGrabPendingReleaseMutation = `mutation ForceGrabPendingRelease($input: PendingReleaseActionInput!) {
  forceGrabPendingRelease(input: $input)
}`;

export const dismissPendingReleaseMutation = `mutation DismissPendingRelease($input: PendingReleaseActionInput!) {
  dismissPendingRelease(input: $input)
}`;

// ── Plugins ──────────────────────────────────────────────────────────────

export const refreshPluginRegistryMutation = `mutation RefreshPluginRegistry {
  refreshPluginRegistry {
    id
    name
    description
    version
    pluginType
    providerType
    author
    official
    builtin
    sourceUrl
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
  }
}`;

export const installPluginMutation = `mutation InstallPlugin($input: InstallPluginInput!) {
  installPlugin(input: $input) {
    id
    pluginId
    name
    version
    pluginType
    providerType
    isEnabled
    isBuiltin
    installedAt
    updatedAt
  }
}`;

export const uninstallPluginMutation = `mutation UninstallPlugin($input: UninstallPluginInput!) {
  uninstallPlugin(input: $input)
}`;

export const togglePluginMutation = `mutation TogglePlugin($input: TogglePluginInput!) {
  togglePlugin(input: $input) {
    id
    pluginId
    name
    version
    pluginType
    providerType
    isEnabled
    isBuiltin
    installedAt
    updatedAt
  }
}`;

export const upgradePluginMutation = `mutation UpgradePlugin($input: UpgradePluginInput!) {
  upgradePlugin(input: $input) {
    id
    pluginId
    name
    version
    pluginType
    providerType
    isEnabled
    isBuiltin
    installedAt
    updatedAt
  }
}`;

// ── Recycle Bin ─────────────────────────────────────────────────────────

export const restoreRecycledItemMutation = `mutation RestoreRecycledItem($id: String!) {
  restoreRecycledItem(id: $id)
}`;

export const deleteRecycledItemMutation = `mutation DeleteRecycledItem($id: String!) {
  deleteRecycledItem(id: $id)
}`;

export const emptyRecycleBinMutation = `mutation EmptyRecycleBin {
  emptyRecycleBin
}`;

// ── Notifications ────────────────────────────────────────────────────────

export const createNotificationChannelMutation = `mutation CreateNotificationChannel($input: CreateNotificationChannelInput!) {
  createNotificationChannel(input: $input) {
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationChannelMutation = `mutation UpdateNotificationChannel($input: UpdateNotificationChannelInput!) {
  updateNotificationChannel(input: $input) {
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationChannelMutation = `mutation DeleteNotificationChannel($id: String!) {
  deleteNotificationChannel(id: $id)
}`;

export const testNotificationChannelMutation = `mutation TestNotificationChannel($id: String!) {
  testNotificationChannel(id: $id)
}`;

export const createNotificationSubscriptionMutation = `mutation CreateNotificationSubscription($input: CreateNotificationSubscriptionInput!) {
  createNotificationSubscription(input: $input) {
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const updateNotificationSubscriptionMutation = `mutation UpdateNotificationSubscription($input: UpdateNotificationSubscriptionInput!) {
  updateNotificationSubscription(input: $input) {
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationSubscriptionMutation = `mutation DeleteNotificationSubscription($id: String!) {
  deleteNotificationSubscription(id: $id)
}`;

// ── Rule Sets ────────────────────────────────────────────────────────────

export const createRuleSetMutation = `mutation CreateRuleSet($input: CreateRuleSetInput!) {
  createRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const updateRuleSetMutation = `mutation UpdateRuleSet($input: UpdateRuleSetInput!) {
  updateRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const deleteRuleSetMutation = `mutation DeleteRuleSet($id: String!) {
  deleteRuleSet(id: $id)
}`;

export const toggleRuleSetMutation = `mutation ToggleRuleSet($input: ToggleRuleSetInput!) {
  toggleRuleSet(input: $input) {
    id
    name
    description
    regoSource
    enabled
    priority
    appliedFacets
    isManaged
    managedKey
    createdAt
    updatedAt
  }
}`;

export const validateRuleSetMutation = `mutation ValidateRuleSet($input: ValidateRuleSetInput!) {
  validateRuleSet(input: $input) {
    valid
    errors
  }
}`;

// ── Convenience Rules ──────────────────────────────────────────────────

export const setConvenienceRequiredAudioMutation = `mutation SetConvenienceRequiredAudio($input: SetConvenienceRequiredAudioInput!) {
  setConvenienceRequiredAudio(input: $input)
}`;

export const setTitleRequiredAudioMutation = `mutation SetTitleRequiredAudio($input: SetTitleRequiredAudioInput!) {
  setTitleRequiredAudio(input: $input)
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const completeSetupMutation = `mutation CompleteSetup {
  completeSetup
}`;

// ── External Import (Sonarr/Radarr) ──────────────────────────────────

export const previewExternalImportMutation = `mutation PreviewExternalImport($input: PreviewExternalImportInput!) {
  previewExternalImport(input: $input) {
    sonarrConnected
    radarrConnected
    sonarrVersion
    radarrVersion
    rootFolders { source path }
    downloadClients {
      sources name implementation scryerClientType
      host port useSsl urlBase username apiKey
      dedupKey supported
    }
    indexers {
      sources name implementation scryerProviderType
      baseUrl apiKey dedupKey supported
    }
  }
}`;

export const executeExternalImportMutation = `mutation ExecuteExternalImport($input: ExecuteExternalImportInput!) {
  executeExternalImport(input: $input) {
    mediaPathsSaved
    downloadClientsCreated
    indexersCreated
    pluginsInstalled
    errors
  }
}`;

export const rehydrateAllMetadataMutation = `mutation RehydrateAllMetadata($language: String!) {
  rehydrateAllMetadata(language: $language)
}`;

const ppScriptFields = `
    id name description scriptType scriptContent appliedFacets
    executionMode timeoutSecs priority enabled debug createdAt updatedAt
`;

export const createPostProcessingScriptMutation = `mutation CreatePostProcessingScript($input: CreatePostProcessingScriptInput!) {
  createPostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const updatePostProcessingScriptMutation = `mutation UpdatePostProcessingScript($input: UpdatePostProcessingScriptInput!) {
  updatePostProcessingScript(input: $input) {${ppScriptFields}}
}`;

export const deletePostProcessingScriptMutation = `mutation DeletePostProcessingScript($id: String!) {
  deletePostProcessingScript(id: $id)
}`;

export const togglePostProcessingScriptMutation = `mutation TogglePostProcessingScript($id: String!) {
  togglePostProcessingScript(id: $id) {${ppScriptFields}}
}`;

// Input type companion — keep in sync with ExecuteExternalImportInput on the backend.
export type DownloadClientApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

// ── Subtitle mutations ──────────────────────────────────────────────────────

export const searchSubtitlesMutation = `mutation SearchSubtitles($input: SearchSubtitlesInput!) {
  searchSubtitles(input: $input) {
    provider
    providerFileId
    language
    releaseInfo
    score
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    downloadCount
    hashMatched
  }
}`;

export const downloadSubtitleMutation = `mutation DownloadSubtitle($input: DownloadSubtitleInput!) {
  downloadSubtitle(input: $input)
}`;

export const blacklistSubtitleMutation = `mutation BlacklistSubtitle($input: BlacklistSubtitleInput!) {
  blacklistSubtitle(input: $input)
}`;

// ── Import retry mutations ────────────────────────────────────────────────

export const retryImportMutation = `mutation RetryImport($input: RetryImportInput!) {
  retryImport(input: $input) {
    importId
    decision
    skipReason
    titleId
    sourcePath
    destPath
    errorMessage
  }
}`;

export const ignoreTrackedDownloadMutation = `mutation IgnoreTrackedDownload($input: IgnoreTrackedDownloadInput!) {
  ignoreTrackedDownload(input: $input) {
    kind
    downloadClientItemId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export const assignTrackedDownloadTitleMutation = `mutation AssignTrackedDownloadTitle($input: AssignTrackedDownloadTitleInput!) {
  assignTrackedDownloadTitle(input: $input) {
    kind
    downloadClientItemId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      facet
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export type SubtitleSearchResult = {
  provider: string;
  providerFileId: string;
  language: string;
  releaseInfo: string | null;
  score: number;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  downloadCount: number | null;
  hashMatched: boolean;
};
