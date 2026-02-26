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
    createdAt
    updatedAt
  }
}`;

export const deleteIndexerMutation = `mutation DeleteIndexer($input: DeleteIndexerConfigInput!) {
  deleteIndexerConfig(input: $input)
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
    title { id name facet }
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
  setCollectionMonitored(input: $input) { id monitored }
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
