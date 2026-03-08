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

export const addTitleMutation = `mutation AddTitle($input: AddTitleInput!) {
  addTitle(input: $input) {
    title { id name facet minAvailability }
    downloadJobId
  }
}`;

export const addTitleAndQueueMutation = `mutation AddTitleAndQueue($input: AddTitleInput!) {
  addTitleAndQueueDownload(input: $input) {
    title { id name facet }
    downloadJobId
  }
}`;

export const scanMovieLibraryMutation = `mutation ScanMovieLibrary {
  scanMovieLibrary {
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

export const saveAdminSettingsMutation = `mutation SaveAdminSettings($input: AdminSettingsUpdateInput!) {
  saveAdminSettings(input: $input) {
    scope
    scopeId
    items {
      category
      scope
      keyName
      dataType
      defaultValueJson
      effectiveValueJson
      valueJson
      source
      hasOverride
      isSensitive
      validationJson
      scopeId
      updatedByUserId
      createdAt
      updatedAt
    }
  }
}`;

export const deleteQualityProfileMutation = `mutation DeleteQualityProfile($input: DeleteQualityProfileInput!) {
  deleteQualityProfile(input: $input) {
    scope
    scopeId
    items {
      category
      scope
      keyName
      dataType
      defaultValueJson
      effectiveValueJson
      valueJson
      source
      hasOverride
      isSensitive
      validationJson
      scopeId
      updatedByUserId
      createdAt
      updatedAt
    }
  }
}`;

export const queueExistingMutation = `mutation QueueExisting($input: QueueDownloadInput!) {
  queueExistingTitleDownload(input: $input)
}`;

export const queueManualImportMutation = `mutation QueueManualImport($input: QueueManualImportInput!) {
  queueManualImport(input: $input)
}`;

export const pauseDownloadMutation = `mutation PauseDownload($input: PauseDownloadInput!) {
  pauseDownload(input: $input)
}`;

export const resumeDownloadMutation = `mutation ResumeDownload($input: ResumeDownloadInput!) {
  resumeDownload(input: $input)
}`;

export const deleteDownloadMutation = `mutation DeleteDownload($input: DeleteDownloadInput!) {
  deleteDownload(input: $input)
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

export const updateTitleMutation = `mutation UpdateTitle($input: UpdateTitleInput!) {
  updateTitle(input: $input) { id name facet tags monitored }
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
