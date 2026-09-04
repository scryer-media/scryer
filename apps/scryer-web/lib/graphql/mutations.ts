import {
  BACKUP_INFO_FIELDS,
  JOB_RUN_FIELDS,
  LOCATION_OPERATION_FIELDS,
  MAINTENANCE_EXCLUSION_FIELDS,
  MAINTENANCE_RULE_SET_DETAIL_FIELDS,
  MAINTENANCE_RULE_SET_FIELDS,
  MEDIA_SERVER_CONNECTION_FIELDS,
  PROVIDER_CONFIG_VALUE_FIELDS,
  RELEASE_SEARCH_RESULT_FIELDS,
  SEEDING_PROFILE_FIELDS,
  SUBTITLE_PROVIDER_CONFIG_FIELDS,
  SUBTITLE_SETTINGS_FIELDS,
  TITLE_MUTATION_RESULT_FIELDS,
} from "./queries.ts";

const AUTH_USER_FIELDS = `
      id
      username
      hasPassword
      hasMfa
      hasPasskey
      accountKind
      appPermissions
      libraryPermissions {
        libraryId
        permissions
      }`;

const LOGIN_PAYLOAD_FIELDS = `
    token
    user {${AUTH_USER_FIELDS}
    }
    expiresAt
    mfaVerifiedUntil
    securityActionVerifiedUntil
    mfaEnrollmentRequired
    passwordChangeRequired
    persistSession`;

export const loginMutation = `mutation Login($input: LoginInput!) {
  login(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const completeRequiredPasswordChangeMutation = `mutation CompleteRequiredPasswordChange($input: CompleteRequiredPasswordChangeInput!) {
  completeRequiredPasswordChange(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const webauthnRegisterStartMutation = `mutation WebauthnRegisterStart {
  webauthnRegisterStart {
    challengeId
    optionsJson
    expiresAt
  }
}`;

export const webauthnRegisterCompleteMutation = `mutation WebauthnRegisterComplete($input: WebauthnRegisterCompleteInput!) {
  webauthnRegisterComplete(input: $input) {
    id
    friendlyName
    createdAt
    lastUsedAt
  }
}`;

export const webauthnAuthenticateStartMutation = `mutation WebauthnAuthenticateStart($username: String) {
  webauthnAuthenticateStart(username: $username) {
    challengeId
    optionsJson
    expiresAt
  }
}`;

export const webauthnAuthenticateCompleteMutation = `mutation WebauthnAuthenticateComplete($input: WebauthnCompleteInput!) {
  webauthnAuthenticateComplete(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const loginVerificationPasskeyStartMutation = `mutation LoginVerificationPasskeyStart($challengeId: ID!) {
  loginVerificationPasskeyStart(challengeId: $challengeId) {
    challengeId
    optionsJson
    expiresAt
  }
}`;

export const loginVerificationPasskeyCompleteMutation = `mutation LoginVerificationPasskeyComplete($input: LoginVerificationPasskeyCompleteInput!) {
  loginVerificationPasskeyComplete(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const loginVerificationTotpCompleteMutation = `mutation LoginVerificationTotpComplete($input: LoginVerificationTotpCompleteInput!) {
  loginVerificationTotpComplete(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const webauthnLoginEnrollmentStartMutation = `mutation WebauthnLoginEnrollmentStart {
  webauthnLoginEnrollmentStart {
    challengeId
    optionsJson
    expiresAt
  }
}`;

export const webauthnLoginEnrollmentCompleteMutation = `mutation WebauthnLoginEnrollmentComplete($input: WebauthnRegisterCompleteInput!) {
  webauthnLoginEnrollmentComplete(input: $input) {
    passkey {
      id
      friendlyName
      createdAt
      lastUsedAt
    }
    login {
${LOGIN_PAYLOAD_FIELDS}
    }
  }
}`;

export const deleteMyPasskeyMutation = `mutation DeleteMyPasskey($id: ID!) {
  deleteMyPasskey(id: $id) {
    id
  }
}`;

export const revokeMyOauthAppMutation = `mutation RevokeMyOauthApp($grantId: ID!) {
  revokeMyOauthApp(grantId: $grantId) {
    grantId
    revoked
  }
}`;

export const createMyApiKeyMutation = `mutation CreateMyApiKey($input: CreateMyApiKeyInput!) {
  createMyApiKey(input: $input) {
    apiKey
    key { id label actor expiresAt revokedAt lastUsedAt createdAt provisioningSource }
  }
}`;

export const revokeMyApiKeyMutation = `mutation RevokeMyApiKey($id: ID!) {
  revokeMyApiKey(id: $id) { id revoked }
}`;

export const totpEnrollmentStartMutation = `mutation TotpEnrollmentStart {
  totpEnrollmentStart {
    challengeId
    otpauthUrl
    secretBase32
    expiresAt
  }
}`;

export const totpEnrollmentCompleteMutation = `mutation TotpEnrollmentComplete($input: TotpEnrollmentCompleteInput!) {
  totpEnrollmentComplete(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
  }
}`;

export const completeLoginMfaEnrollmentMutation = `mutation CompleteLoginMfaEnrollment($input: TotpEnrollmentCompleteInput!) {
  completeLoginMfaEnrollment(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
    login {
${LOGIN_PAYLOAD_FIELDS}
    }
  }
}`;

export const mfaVerifyStepUpMutation = `mutation MfaVerifyStepUp($input: TotpVerifyInput!) {
  mfaVerifyStepUp(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const accountSecurityPasswordVerifyMutation = `mutation AccountSecurityPasswordVerify($currentPassword: String!) {
  accountSecurityPasswordVerify(currentPassword: $currentPassword) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const accountSecurityPasskeyStartMutation = `mutation AccountSecurityPasskeyStart {
  accountSecurityPasskeyStart {
    challengeId
    optionsJson
    expiresAt
  }
}`;

export const accountSecurityPasskeyCompleteMutation = `mutation AccountSecurityPasskeyComplete($input: WebauthnCompleteInput!) {
  accountSecurityPasskeyComplete(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const totpDisableMutation = `mutation TotpDisable($input: TotpVerifyInput!) {
  totpDisable(input: $input) {
    enabled
    createdAt
    lastUsedAt
    recoveryCodesRemaining
  }
}`;

export const totpRegenerateRecoveryCodesMutation = `mutation TotpRegenerateRecoveryCodes($input: TotpVerifyInput!) {
  totpRegenerateRecoveryCodes(input: $input) {
    status {
      enabled
      createdAt
      lastUsedAt
      recoveryCodesRemaining
    }
    recoveryCodes
  }
}`;

export const createUserMutation = `mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserPasswordMutation = `mutation SetUserPassword($input: SetUserPasswordInput!) {
  setUserPassword(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserAppPermissionsMutation = `mutation SetUserAppPermissions($input: SetUserAppPermissionsInput!) {
  setUserAppPermissions(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const setUserLibraryPermissionsMutation = `mutation SetUserLibraryPermissions($input: SetUserLibraryPermissionsInput!) {
  setUserLibraryPermissions(input: $input) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const deleteUserMutation = `mutation DeleteUser($id: ID!) {
  deleteUser(id: $id) {
    id
  }
}`;

export const setUserLoginEnabledMutation = `mutation SetUserLoginEnabled($input: SetUserLoginEnabledInput!) {
  setUserLoginEnabled(input: $input) {
    id
    username
    loginEnabled
    isDefaultAdmin
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const resetUserMfaMutation = `mutation ResetUserMfa($id: ID!) {
  resetUserMfa(id: $id) {
    id
    username
    hasPassword
    hasMfa
    hasPasskey
    accountKind
    appPermissions
    libraryPermissions {
      libraryId
      permissions
    }
  }
}`;

export const deleteTitleMutation = `mutation DeleteTitle($input: DeleteTitleInput!) {
  deleteTitle(input: $input) {
    id
  }
}`;

export const deleteTitlesMutation = `mutation DeleteTitles($input: DeleteTitlesInput!) {
  deleteTitles(input: $input) {
    acceptedTitleIds
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const renameTitlesMutation = `mutation RenameTitles($input: RenameTitlesInput!) {
  renameTitles(input: $input) {
    acceptedTitleIds
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const createIndexerMutation = `mutation CreateIndexer($input: CreateIndexerConfigInput!) {
  createIndexerConfig(input: $input) {
    id
    name
    providerType
    baseUrl
    proxyConfigId
    downloadClientId
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    isManaged
    managedParentConfigId
    supportsManagedChildrenSync
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorMessage
    lastErrorAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
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
    proxyConfigId
    downloadClientId
    hasApiKey
    storedSecretKeys
    rateLimitSeconds
    rateLimitBurst
    disabledUntil
    isEnabled
    isManaged
    managedParentConfigId
    supportsManagedChildrenSync
    enableInteractiveSearch
    enableAutoSearch
    lastHealthStatus
    lastErrorMessage
    lastErrorAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    createdAt
    updatedAt
  }
}`;

const PROXY_CONFIG_FIELDS = `
    id
    name
    providerType
    protocol
    baseUrl
    requestTimeoutSeconds
    hasCredentials
    remoteDns
    hasPrivateKey
    peerPublicKey
    hasPresharedKey
    tunnelPublicKey
    tunnelAddresses
    tunnelDnsServers
    tunnelMtu
    tunnelKeepaliveSeconds
    hostKeyFingerprint
    hostKeyPinnedAt
    isEnabled
    lastHealthStatus
    lastErrorMessage
    lastErrorAt
    createdAt
    updatedAt`;

export const createProxyConfigMutation = `mutation CreateProxyConfig($input: CreateProxyConfigInput!) {
  createProxyConfig(input: $input) {${PROXY_CONFIG_FIELDS}
  }
}`;

export const updateProxyConfigMutation = `mutation UpdateProxyConfig($input: UpdateProxyConfigInput!) {
  updateProxyConfig(input: $input) {${PROXY_CONFIG_FIELDS}
  }
}`;

export const deleteProxyConfigMutation = `mutation DeleteProxyConfig($id: ID!) {
  deleteProxyConfig(id: $id) {
      id
}
}`;

export const testProxyConfigMutation = `mutation TestProxyConfig($id: ID!) {
  testProxyConfig(id: $id) {
    ok
    status
    message
    durationMs
  }
}`;

/**
 * Trust-on-first-use reset: forgetting the pinned host key makes the next
 * connection pin whatever the server offers.
 */
export const resetProxyHostKeyMutation = `mutation ResetProxyHostKey($id: ID!) {
  resetProxyHostKey(id: $id) {${PROXY_CONFIG_FIELDS}
  }
}`;

export const deleteIndexerMutation = `mutation DeleteIndexer($id: ID!) {
  deleteIndexerConfig(id: $id) {
    id
  }
}`;

export const syncIndexerConfigMutation = `mutation SyncIndexerConfig($id: ID!) {
  syncIndexerConfig(id: $id) {
    parentConfigId
    createdIds
    updatedIds
    deletedIds
  }
}`;

export const testIndexerConnectionMutation = `mutation TestIndexerConnection($input: TestIndexerConnectionInput!) {
  testIndexerConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const createDownloadClientMutation = `mutation CreateDownloadClient($input: CreateDownloadClientConfigInput!) {
  createDownloadClientConfig(input: $input) {
    id
    name
    clientType
    baseUrl
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    proxyConfigId
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
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    proxyConfigId
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const testDownloadClientConnectionMutation = `mutation TestDownloadClientConnection($input: TestDownloadClientConnectionInput!) {
  testDownloadClientConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const deleteDownloadClientMutation = `mutation DeleteDownloadClient($id: ID!) {
  deleteDownloadClientConfig(id: $id) {
    id
    clearedIndexerMappingCount
  }
}`;

export const reorderDownloadClientsMutation = `mutation ReorderDownloadClients($input: ReorderDownloadClientConfigsInput!) {
  reorderDownloadClientConfigs(input: $input) {
    ids
  }
}`;

export const addTitleMutation = `mutation AddTitle($input: AddTitleInput!) {
  addTitle(input: $input) {
      title {${TITLE_MUTATION_RESULT_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
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
    title {${TITLE_MUTATION_RESULT_FIELDS}
    }
    metadataHydrationState
    reusedExistingTitle
    reusedQueuedDownload
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

export const submitMediaRequestMutation = `mutation SubmitMediaRequest($input: SubmitMediaRequestInput!) {
  submitMediaRequest(input: $input) {
      requestId
}
}`;

export const approveMediaRequestMutation = `mutation ApproveMediaRequest($input: ApproveMediaRequestInput!) {
  approveMediaRequest(input: $input) {
    titleId
    wantedSearch {
      queuedCount
      skippedInProgressCount
    }
    searchError
  }
}`;

export const dismissMediaRequestMutation = `mutation DismissMediaRequest($requestId: ID!) {
  dismissMediaRequest(requestId: $requestId) {
      requestId
}
}`;

export const updateMyMediaRequestMutation = `mutation UpdateMyMediaRequest($input: UpdateMediaRequestInput!) {
  updateMyMediaRequest(input: $input) {
    id
    libraryId
    facet
    status
    identityFingerprint
    title
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
    updatedAt
  }
}`;

export const cancelMyMediaRequestMutation = `mutation CancelMyMediaRequest($requestId: ID!) {
  cancelMyMediaRequest(requestId: $requestId) {
      requestId
}
}`;

export const deleteMediaFileMutation = `mutation DeleteMediaFile($input: DeleteMediaFileInput!) {
  deleteMediaFile(input: $input) {
    id
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const deleteEpisodeFilesMutation = `mutation DeleteEpisodeFiles($input: DeleteEpisodeFilesInput!) {
  deleteEpisodeFiles(input: $input) {
    acceptedFileIds
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const scanLibraryMutation = `mutation ScanLibrary($input: ScanLibraryInput!) {
  scanLibrary(input: $input) {
    sessionId
    facet
    mode
    status
    startedAt
    updatedAt
  }
}`;

const LIBRARY_FIELDS = `
    id
    facet
    name
    slug
    isDefault
    isBootstrapDefaultRootSet
    roots {
      id
      path
      isDefault
    }`;

export const createLibraryMutation = `mutation CreateLibrary($input: CreateLibraryInput!) {
  createLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const updateLibraryMutation = `mutation UpdateLibrary($input: UpdateLibraryInput!) {
  updateLibrary(input: $input) {${LIBRARY_FIELDS}
  }
}`;

export const deleteLibraryMutation = `mutation DeleteLibrary($id: ID!) {
  deleteLibrary(id: $id) {
    id
  }
}`;

export const cancelLibraryScanMutation = `mutation CancelLibraryScan($sessionId: ID!) {
  cancelLibraryScan(sessionId: $sessionId) {
    sessionId
    accepted
  }
}`;

export const scanTitleLibraryMutation = `mutation ScanTitleLibrary($titleId: ID!) {
  scanTitleLibrary(titleId: $titleId) {
    scanned
    matched
    imported
    skipped
    unmatched
  }
}`;

export const resolvePendingImportMutation = `mutation ResolvePendingImport($input: ResolvePendingImportInput!) {
  resolvePendingImport(input: $input) {
    created
    metadataHydrationState
    title {
      id
      libraryId
      name
      facet
      monitored
      slug
    }
  }
}`;

export const bindPendingImportMutation = `mutation BindPendingImport($input: BindPendingImportInput!) {
  bindPendingImport(input: $input) {
    created
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      monitored
    }
  }
}`;

export const ignorePendingImportMutation = `mutation IgnorePendingImport($pendingImportId: ID!) {
  ignorePendingImport(pendingImportId: $pendingImportId) {
    id
    status
  }
}`;

export const triggerJobMutation = `mutation TriggerJob($jobKey: JobKeyValue!) {
  triggerJob(jobKey: $jobKey) {
${JOB_RUN_FIELDS}
  }
}`;

export const updateSubtitleSettingsMutation = `mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
  updateSubtitleSettings(input: $input) {${SUBTITLE_SETTINGS_FIELDS}
  }
}`;

export const createSubtitleProviderConfigMutation = `mutation CreateSubtitleProviderConfig($input: CreateSubtitleProviderConfigInput!) {
  createSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const updateSubtitleProviderConfigMutation = `mutation UpdateSubtitleProviderConfig($input: UpdateSubtitleProviderConfigInput!) {
  updateSubtitleProviderConfig(input: $input) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const deleteSubtitleProviderConfigMutation = `mutation DeleteSubtitleProviderConfig($id: ID!) {
  deleteSubtitleProviderConfig(id: $id) {
    id
  }
}`;

export const testSubtitleProviderConnectionMutation = `mutation TestSubtitleProviderConnection($input: TestSubtitleProviderConnectionInput!) {
  testSubtitleProviderConnection(input: $input) {
    status
    message
    retryAfterSeconds
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
    longTailBackfillMaxScopesPerCycle
    longTailReconvergeDays
  }
}`;

export const updateGeneralSettingsMutation = `mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
  updateGeneralSettings(input: $input) {
    experimentalFeaturesEnabled
    personalizedDiscoveryEnabled
    keepHistoryForever
    historyRetentionDays
    imageCacheMaxSizeMb
    effectiveImageCacheMaxSizeBytes
    effectiveImageCacheMaxSizeMb
    imageCacheMaxSizeEnvOverrideActive
    pluginHttpCaBundlePem
    pluginHttpTrustedCertificates {
      fingerprintSha256
      pem
    }
  }
}`;

export const setMyUiSettingsMutation = `mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
  setMyUiSettings(input: $input) {
    theme
    dateTimeFormat
    highlightColor
    secondaryColor
    highContrastMode
    reduceMotion
    hideSponsorButton
    density
    sidebarMode
    defaultLandingView
    tableColumns {
      facet
      tableViewMode
      columnId
      columnOrder
      visible
    }
  }
}`;

export const clearTitleImageCacheMutation = `mutation ClearTitleImageCache {
  clearTitleImageCache {
      requestedAt
}
}`;

export const createBackupMutation = `mutation CreateBackup($input: CreateBackupInput!) {
  createBackup(input: $input) {${BACKUP_INFO_FIELDS}
  }
}`;

export const prepareBackupDownloadMutation = `mutation PrepareBackupDownload($input: PrepareBackupDownloadInput!) {
  prepareBackupDownload(input: $input) {
    downloadUrl
    downloadAuthorizationToken
    expiresAt
  }
}`;

export const deleteBackupMutation = `mutation DeleteBackup($input: DeleteBackupInput!) {
  deleteBackup(input: $input) {
    filename
    deleted
  }
}`;

const AUTO_BACKUP_SETTINGS_FIELDS = `
    enabled
    dailyTimeLocal
    autoBackupKeyPresent
    autoBackupDisabledMissingKeyNotice
    nextRunAt`;

const BACKUP_SETTINGS_FIELDS = `
    customBackupPath
    defaultBackupPath
    effectiveBackupPath`;

export const updateAutoBackupSettingsMutation = `mutation UpdateAutoBackupSettings($input: UpdateAutoBackupSettingsInput!) {
  updateAutoBackupSettings(input: $input) {${AUTO_BACKUP_SETTINGS_FIELDS}
  }
}`;

export const updateBackupSettingsMutation = `mutation UpdateBackupSettings($input: UpdateBackupSettingsInput!) {
  updateBackupSettings(input: $input) {${BACKUP_SETTINGS_FIELDS}
  }
}`;

export const acknowledgeAutoBackupDisabledMissingKeyNoticeMutation = `mutation AcknowledgeAutoBackupDisabledMissingKeyNotice {
  acknowledgeAutoBackupDisabledMissingKeyNotice {${AUTO_BACKUP_SETTINGS_FIELDS}
  }
}`;

export const updateSecuritySettingsMutation = `mutation UpdateSecuritySettings($input: UpdateSecuritySettingsInput!) {
  updateSecuritySettings(input: $input) {
    formLoginEnabled
    passwordMinLength
    skipLoginForLocalIps
    apiKeysRestrictToSystemSettingsUsers
    mfaRequireConfigStepUp
    mfaRequirePasswordLogin
    mfaRequireJellyfinLogin
    mfaRequireEmbyLogin
    effectiveFormLoginEnabled
    envOverrideActive
    envOverrideDescription
  }
}`;

const OAUTH_CLIENT_REGISTRATION_FIELDS = `
    clientId
    displayName
    redirectUris
    enabled
    source
    kind`;

export const createOAuthClientRegistrationMutation = `mutation CreateOAuthClientRegistration($input: CreateOAuthClientRegistrationInput!) {
  createOAuthClientRegistration: createOauthClientRegistration(input: $input) {${OAUTH_CLIENT_REGISTRATION_FIELDS}
  }
}`;

export const updateOAuthClientRegistrationMutation = `mutation UpdateOAuthClientRegistration($clientId: String!, $input: UpdateOAuthClientRegistrationInput!) {
  updateOauthClientRegistration(clientId: $clientId, input: $input) {${OAUTH_CLIENT_REGISTRATION_FIELDS}
  }
}`;

export const deleteOAuthClientRegistrationMutation = `mutation DeleteOAuthClientRegistration($clientId: String!) {
  deleteOauthClientRegistration(clientId: $clientId) {
    clientId
    deleted
  }
}`;

const LINKED_ACCOUNT_FIELDS = `
    id
    userId
    provider
    connectionId
    externalUserId
    username
    displayName
    avatarUrl
    status
    verifiedAt
    lastLoginAt
    createdAt
    updatedAt`;

export const createMediaServerConnectionMutation = `mutation CreateMediaServerConnection($input: CreateMediaServerConnectionInput!) {
  createMediaServerConnection(input: $input) {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
}`;

export const updateMediaServerConnectionMutation = `mutation UpdateMediaServerConnection($input: UpdateMediaServerConnectionInput!) {
  updateMediaServerConnection(input: $input) {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
}`;

export const deleteMediaServerConnectionMutation = `mutation DeleteMediaServerConnection($id: ID!) {
  deleteMediaServerConnection(id: $id) {
    id
  }
}`;

export const testMediaServerConnectionMutation = `mutation TestMediaServerConnection($input: TestMediaServerConnectionInput!) {
  testMediaServerConnection(input: $input) {
    status
    message
    retryAfterSeconds
  }
}`;

export const discoverPlexMediaServersMutation = `mutation DiscoverPlexMediaServers($plexAuthToken: String!) {
  discoverPlexMediaServers(plexAuthToken: $plexAuthToken) {
    id
    name
  }
}`;

export const discoverEmbyConnectServersMutation = `mutation DiscoverEmbyConnectServers($input: DiscoverEmbyConnectServersInput!) {
  discoverEmbyConnectServers(input: $input) {
    serverId
    name
    userType
    localAddress
    remoteAddress
    localApiBaseUrl
    remoteApiBaseUrl
    localStatus
    remoteStatus
    suggestedBaseUrl
  }
}`;

export const testEmbyConnectMutation = `mutation TestEmbyConnect($input: TestEmbyConnectInput!) {
  testEmbyConnect(input: $input) {
    status
    message
  }
}`;

export const createExternalAccountInviteMutation = `mutation CreateExternalAccountInvite($input: CreateExternalAccountInviteInput!) {
  createExternalAccountInvite(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const linkPlexAccountMutation = `mutation LinkPlexAccount($input: LinkPlexAccountInput!) {
  linkPlexAccount(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const linkJellyfinAccountMutation = `mutation LinkJellyfinAccount($input: LinkJellyfinAccountInput!) {
  linkJellyfinAccount(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const linkEmbyAccountMutation = `mutation LinkEmbyAccount($input: LinkEmbyAccountInput!) {
  linkEmbyAccount(input: $input) {${LINKED_ACCOUNT_FIELDS}
  }
}`;

export const unlinkExternalAccountMutation = `mutation UnlinkExternalAccount($linkedAccountId: ID!) {
  unlinkExternalAccount(linkedAccountId: $linkedAccountId) {
    linkedAccountId
  }
}`;

export const loginWithPlexMutation = `mutation LoginWithPlex($input: LoginWithPlexInput!) {
  loginWithPlex(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const loginWithJellyfinMutation = `mutation LoginWithJellyfin($input: LoginWithJellyfinInput!) {
  loginWithJellyfin(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const loginWithEmbyMutation = `mutation LoginWithEmby($input: LoginWithEmbyInput!) {
  loginWithEmby(input: $input) {
${LOGIN_PAYLOAD_FIELDS}
  }
}`;

export const upsertDelayProfileMutation = `mutation UpsertDelayProfile($input: DelayProfileInput!) {
  upsertDelayProfile(input: $input) {
    id
    name
    usenetDelayMinutes
    torrentDelayMinutes
    enableUsenet
    enableTorrent
    preferredProtocol
    minAgeMinutes
    bypassScoreThreshold
    bypassIfHighestQuality
    appliesToFacets
    tags
    priority
    enabled
  }
}`;

export const deleteDelayProfileMutation = `mutation DeleteDelayProfile($id: ID!) {
  deleteDelayProfile(id: $id) {
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
      scoringOverrides {
        allowX265Non4K
        blockDvWithoutFallback
        preferCompactEncodes
        preferLosslessAudio
        blockUpscaled
      }
      cutoffTier
      minScoreToGrab`;

const qualityProfileSettingsFieldSelection = `
    globalProfileId
    globalScoringPersona
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
    }
    categoryPersonaSelections {
      scope
      overridePersona
      effectivePersona
      inheritsGlobal
    }`;

const downloadClientRoutingFieldSelection = `
    clientId
    enabled
    category
    recentQueuePriority
    olderQueuePriority
    removeCompleted
    removeFailed
    seedingProfileId`;

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
    requiredAudioLanguages
    useSeasonFolders
    folderTemplate
    seasonFolderTemplate
    specialsFolderTemplate
    renameEnabled
    renameTemplate
    renameCollisionPolicy
    renameMissingMetadataPolicy
    fillerPolicy
    recapPolicy
    monitorSpecials
    interSeasonMovies
    monitorFillerMovies
    nfoWriteOnImport
    plexmatchWriteOnImport
    importMode
    setPermissionsLinux
    fileChmod
    folderChmod
    chownGroup`;

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

export const deleteQualityProfileMutation = `mutation DeleteQualityProfile($id: ID!) {
  deleteQualityProfile(id: $id) {
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
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        __typename
        ... on EpisodeScopePayload {
          episodeId
        }
        ... on EpisodeSetScopePayload {
          episodeIds
        }
        ... on SeriesMovieScopePayload {
          seriesMovieLinkId
        }
        ... on CollectionScopePayload {
          collectionId
        }
        ... on TitleScopePayload {
          wholeTitle
        }
        ... on OrphanScopePayload {
          orphaned
        }
      }
    }
  }
}`;

export const queueReplacementMutation = `mutation QueueReplacement($input: QueueDownloadInput!) {
  queueReplacementRelease(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        __typename
        ... on EpisodeScopePayload {
          episodeId
        }
        ... on EpisodeSetScopePayload {
          episodeIds
        }
        ... on SeriesMovieScopePayload {
          seriesMovieLinkId
        }
        ... on CollectionScopePayload {
          collectionId
        }
        ... on TitleScopePayload {
          wholeTitle
        }
        ... on OrphanScopePayload {
          orphaned
        }
      }
    }
  }
}`;

export const triggerTitleMismatchRecoverySearchMutation = `mutation TriggerTitleMismatchRecoverySearch($titleId: ID!) {
  triggerTitleMismatchRecoverySearch(titleId: $titleId) {
    titleId
    queuedCount
  }
}`;

export const queueBestReleaseMutation = `mutation QueueBestRelease($input: QueueBestReleaseInput!) {
  queueBestRelease(input: $input) {
    status
    jobId
    titleId
    titleName
    sourceTitle
    sourceKind
    conflict {
      titleId
      titleName
      downloadClientId
      downloadClientType
      downloadClientItemId
      sourceTitle
      sourceKind
      state
      replaceable
      scope {
        __typename
        ... on EpisodeScopePayload {
          episodeId
        }
        ... on EpisodeSetScopePayload {
          episodeIds
        }
        ... on SeriesMovieScopePayload {
          seriesMovieLinkId
        }
        ... on CollectionScopePayload {
          collectionId
        }
        ... on TitleScopePayload {
          wholeTitle
        }
        ... on OrphanScopePayload {
          orphaned
        }
      }
    }
  }
}`;

export const queueManualImportMutation = `mutation QueueManualImport($input: QueueManualImportInput!) {
  queueManualImport(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    importId
    removed
  }
}`;

export const beginManualImportSelectionMutation = `mutation BeginManualImportSelection($input: BeginManualImportSelectionInput!) {
  beginManualImportSelection(input: $input) {
    selectionId
    archiveExtractionNeeded
    files {
      candidateId
      fileName
      sizeBytes
      videoFacts {
        containerFormat
        videoCodec
        audioCodec
        videoWidth
        videoHeight
        durationSeconds
      }
      quality
      parsedSeason
      parsedEpisodes
      suggestedEpisodeId
      suggestedEpisodeLabel
      suggestedSeriesMovieLinkId
    }
    availableEpisodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      absoluteNumber
      title
      monitored
    }
    availableSeriesMovies {
      seriesMovieLinkId
      movieTitle
      year
      runtimeMinutes
    }
  }
}`;

export const pauseDownloadMutation = `mutation PauseDownload($input: PauseDownloadInput!) {
  pauseDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
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
    clientId
    clientType
    removed
    queueItem {
      id
      clientId
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
    clientId
    commandId
    removed
    clientType
    queueItem {
      id
      clientId
      clientType
      downloadClientItemId
      state
      deleteStatus
      deleteErrorMessage
    }
  }
}`;

export function buildIgnoreTrackedDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: IgnoreTrackedDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: ignoreTrackedDownload(input: $input${index}) { kind }`,
  ).join("\n");

  return `mutation IgnoreTrackedDownloads(${variables}) {
${fields}
}`;
}

export function buildDeleteDownloadBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteDownloadInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: deleteDownload(input: $input${index}) { kind removed commandId }`,
  ).join("\n");

  return `mutation DeleteDownloads(${variables}) {
${fields}
}`;
}

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

export const setSeriesMovieMonitoredMutation = `mutation SetSeriesMovieMonitored($input: SetSeriesMovieMonitoredInput!) {
  setSeriesMovieMonitored(input: $input) { id monitored }
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
    rootFolderId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
  }
}`;

export function buildSetTitleMonitoredBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: SetTitleMonitoredInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: setTitleMonitored(input: $input${index}) { id monitored }`,
  ).join("\n");

  return `mutation SetTitleMonitoredBatch(${variables}) {
${fields}
}`;
}

export function buildUpdateTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: UpdateTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: updateTitle(input: $input${index}) { id }`,
  ).join("\n");

  return `mutation UpdateTitleBatch(${variables}) {
${fields}
}`;
}

export function buildDeleteTitleBatchMutation(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$input${index}: DeleteTitleInput!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) => `item${index}: deleteTitle(input: $input${index}) {
      id
  }`,
  ).join("\n");

  return `mutation DeleteTitleBatch(${variables}) {
${fields}
}`;
}

export const fixTitleMatchMutation = `mutation FixTitleMatch($input: FixTitleMatchInput!) {
  fixTitleMatch(input: $input) {
    hydrated
    warnings
    libraryScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    title {
      id
      name
      facet
      externalIds {
        source
        value
      }
      imdbId
      slug
      metadataFetchedAt
    }
  }
}`;

// Folder-match correction. Ownership of a folder another title holds is never
// taken silently: the caller must send SWAP or TAKE_OVER for that case.
export const applyTitleFolderChangeMutation = `mutation ApplyTitleFolderChange($input: ApplyTitleFolderChangeInput!) {
  applyTitleFolderChange(input: $input) {
    outcome
    title {
      id
      name
      folderPath
    }
    previousFolderPath
    detachedMediaFileCount
    scan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    swappedTitle {
      id
      name
      folderPath
    }
    swappedTitleScan {
      scanned
      matched
      imported
      skipped
      unmatched
    }
    displacedTitle {
      id
      name
      previousFolderPath
      repairReasonCode
    }
  }
}`;

export const setPrimaryMovieFileMutation = `mutation SetPrimaryMovieFile($input: SetPrimaryMovieFileInput!) {
  setPrimaryMovieFile(input: $input) {
    id
  }
}`;

// One server-side interactive search job replaces the retired
// per-item trigger mutations; progress is polled via acquisitionSearchJobQuery.
export const triggerAcquisitionSearchMutation = `mutation TriggerAcquisitionSearch($input: TriggerAcquisitionSearchInput!) {
  triggerAcquisitionSearch(input: $input) {
    id
    state
    total
    processed
    grabbedCount
    failedCount
    currentTitle
    startedAt
    finishedAt
  }
}`;

export const cancelAcquisitionSearchMutation = `mutation CancelAcquisitionSearch($id: ID!) {
  cancelAcquisitionSearch(id: $id) {
    id
    accepted
  }
}`;

// Hotfix 0.17.1: starts the server-side interactive release-search job; the
// snapshot is then polled via interactiveReleaseSearchQuery.
export const startInteractiveReleaseSearchMutation = `mutation StartInteractiveReleaseSearch($input: SearchReleasesInput!) {
  startInteractiveReleaseSearch(input: $input) {
    id
    state
    results {${RELEASE_SEARCH_RESULT_FIELDS}
    }
    indexers {
      indexerId
      name
      priority
      status
      resultCount
      elapsedMs
      failureReason
    }
    startedAt
    completedAt
  }
}`;

export const cancelInteractiveReleaseSearchMutation = `mutation CancelInteractiveReleaseSearch($id: ID!) {
  cancelInteractiveReleaseSearch(id: $id) {
    id
    accepted
  }
}`;

// Spec 0002 D4: a title-less search row carries no candidate token, so the grab
// dialog mints one against the title the operator picked and then queues it
// with the existing queue mutations. The payload is the same release row, now
// carrying `candidateToken` and the server-resolved `queueScope` (D11).
export const issueInteractiveReleaseCandidateTokenMutation = `mutation IssueInteractiveReleaseCandidateToken($input: IssueInteractiveReleaseCandidateTokenInput!) {
  issueInteractiveReleaseCandidateToken(input: $input) {${RELEASE_SEARCH_RESULT_FIELDS}
  }
}`;

// Spec 0002 D8: grabs a search row with no catalog title behind it. The client's
// own routing category applies, so there is no category input here.
export const queueUnlinkedReleaseMutation = `mutation QueueUnlinkedRelease($input: QueueUnlinkedReleaseInput!) {
  queueUnlinkedRelease(input: $input) {
    downloadId
    clientName
    sourceTitle
  }
}`;

export const pauseWantedItemMutation = `mutation PauseWantedItem($id: ID!) {
  pauseWantedItem(id: $id) {
    id
  }
}`;

export const resumeWantedItemMutation = `mutation ResumeWantedItem($id: ID!) {
  resumeWantedItem(id: $id) {
    id
  }
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

export const forceGrabPendingReleaseMutation = `mutation ForceGrabPendingRelease($id: ID!) {
  forceGrabPendingRelease(id: $id) {
    id
    grabbed
  }
}`;

export const dismissPendingReleaseMutation = `mutation DismissPendingRelease($id: ID!) {
  dismissPendingRelease(id: $id) {
    id
  }
}`;

// ── Plugins ──────────────────────────────────────────────────────────────

export const refreshPluginCatalogMutation = `mutation RefreshPluginCatalog {
  refreshPluginCatalog {
    id
    name
    description
    version
    latestVersion
    pluginType
    providerType
    author
    official
    publisher
    supportTier
    status
    docsUrl
    sourceRepo
    builtin
    sourceUrl
    sourceKind
    blockedReason
    bytes
    isInstalled
    isEnabled
    installedVersion
    updateAvailable
    installInProgress
    defaultBaseUrl
  }
}`;

export const beginInstallPluginMutation = `mutation BeginInstallPlugin($pluginId: ID!) {
  beginInstallPlugin(pluginId: $pluginId) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const uninstallPluginMutation = `mutation UninstallPlugin($pluginId: ID!) {
  uninstallPlugin(pluginId: $pluginId) {
    pluginId
  }
}`;

export const togglePluginMutation = `mutation TogglePlugin($input: TogglePluginInput!) {
  togglePlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const beginUpgradePluginMutation = `mutation BeginUpgradePlugin($pluginId: ID!) {
  beginUpgradePlugin(pluginId: $pluginId) {
    pluginId
    operationKind
    state
    label
    stepIndex
    stepCount
    message
    error
  }
}`;

export const inspectManualPluginRepoMutation = `mutation InspectManualPluginRepo($input: ManualPluginRepoInput!) {
  inspectManualPluginRepo(input: $input) {
    githubRepoUrl
    plugin {
      id
      name
      description
      version
      latestVersion
      pluginType
      providerType
      author
      official
      publisher
      supportTier
      status
      docsUrl
      sourceRepo
      builtin
      sourceUrl
      sourceKind
      blockedReason
      bytes
      isInstalled
      isEnabled
      installedVersion
      updateAvailable
      installInProgress
      defaultBaseUrl
    }
  }
}`;

export const installManualPluginMutation = `mutation InstallManualPlugin($input: ManualPluginRepoInput!) {
  installManualPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

export const installUploadedPluginMutation = `mutation InstallUploadedPlugin($input: ManualPluginUploadInput!) {
  installUploadedPlugin(input: $input) {
    id
    pluginId
    name
    description
    version
    sdkVersion
    sdkConstraint
    pluginType
    providerType
    isEnabled
    isBuiltin
    sourceKind
    sourceUrl
    publisher
    supportTier
    docsUrl
    sourceRepo
    manifestUrl
    wasmDigest
    artifactDigest
    installedAt
    updatedAt
  }
}`;

// ── Recycle Bin ─────────────────────────────────────────────────────────

export const restoreRecycledItemMutation = `mutation RestoreRecycledItem($id: ID!) {
  restoreRecycledItem(id: $id) {
    id
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const restoreRecycledItemsMutation = `mutation RestoreRecycledItems($input: RestoreRecycledItemsInput!) {
  restoreRecycledItems(input: $input) {
    ids
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const deleteRecycledItemMutation = `mutation DeleteRecycledItem($id: ID!) {
  deleteRecycledItem(id: $id) {
    id
    deleted
  }
}`;

export const deleteRecycledItemsMutation = `mutation DeleteRecycledItems($input: DeleteRecycledItemsInput!) {
  deleteRecycledItems(input: $input) {
    ids
    jobRun {
      id
      jobKey
      displayName
      category
      section
      status
      triggerSource
      startedAt
      completedAt
      summaryJson
      summaryText
      errorText
      progressJson
    }
  }
}`;

export const emptyRecycleBinMutation = `mutation EmptyRecycleBin($libraryIds: [ID!]) {
  emptyRecycleBin(libraryIds: $libraryIds) {
    purgedCount
  }
}`;

export const updateRecycleBinSettingsMutation = `mutation UpdateRecycleBinSettings($input: UpdateRecycleBinSettingsInput!) {
  updateRecycleBinSettings(input: $input) {
    enabled
  }
}`;

export const updateVerificationSettingsMutation = `mutation UpdateVerificationSettings($input: UpdateVerificationSettingsInput!) {
  updateVerificationSettings(input: $input) {
    depth
  }
}`;

export const updatePluginAutoUpdateSettingsMutation = `mutation UpdatePluginAutoUpdateSettings($input: UpdatePluginAutoUpdateSettingsInput!) {
  updatePluginAutoUpdateSettings(input: $input) {
    enabled
  }
}`;

// ── Notifications ────────────────────────────────────────────────────────

export const createNotificationChannelMutation = `mutation CreateNotificationChannel($input: CreateNotificationChannelInput!) {
  createNotificationChannel(input: $input) {
    id
    name
    channelType
    mediaServerConnectionId
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
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
    mediaServerConnectionId
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationChannelMutation = `mutation DeleteNotificationChannel($id: ID!) {
  deleteNotificationChannel(id: $id) {
    id
  }
}`;

export const testNotificationChannelMutation = `mutation TestNotificationChannel($id: ID!) {
  testNotificationChannel(id: $id) {
    id
    status
    message
    retryAfterSeconds
  }
}`;

export const createNotificationSubscriptionMutation = `mutation CreateNotificationSubscription($input: CreateNotificationSubscriptionInput!) {
  createNotificationSubscription(input: $input) {
    id
    channelId
    targetKind
    targetId
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
    targetKind
    targetId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const deleteNotificationSubscriptionMutation = `mutation DeleteNotificationSubscription($id: ID!) {
  deleteNotificationSubscription(id: $id) {
    id
  }
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
    managedTagFilter
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
    managedTagFilter
    createdAt
    updatedAt
  }
}`;

export const deleteRuleSetMutation = `mutation DeleteRuleSet($id: ID!) {
  deleteRuleSet(id: $id) {
    id
  }
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
    managedTagFilter
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

// ── Maintenance Rules ─────────────────────────────────────────────────
//
// Maintenance rule sets are saved disabled and nothing evaluates or executes
// them yet. Preview is the only mutation here that runs a matcher, and it is
// read-only: it reports what a rule would select, and changes nothing.

export const createMaintenanceRuleSetMutation = `mutation CreateMaintenanceRuleSet($input: CreateMaintenanceRuleSetInput!) {
  createMaintenanceRuleSet(input: $input) {${MAINTENANCE_RULE_SET_DETAIL_FIELDS}
  }
}`;

export const updateMaintenanceRuleMatcherMutation = `mutation UpdateMaintenanceRuleMatcher($input: UpdateMaintenanceRuleMatcherInput!) {
  updateMaintenanceRuleMatcher(input: $input) {${MAINTENANCE_RULE_SET_DETAIL_FIELDS}
  }
}`;

export const updateMaintenanceRuleMetadataMutation = `mutation UpdateMaintenanceRuleMetadata($input: UpdateMaintenanceRuleMetadataInput!) {
  updateMaintenanceRuleMetadata(input: $input) {${MAINTENANCE_RULE_SET_FIELDS}
  }
}`;

export const deleteMaintenanceRuleSetMutation = `mutation DeleteMaintenanceRuleSet($id: ID!) {
  deleteMaintenanceRuleSet(id: $id) {
    id
  }
}`;

export const validateMaintenanceRuleMutation = `mutation ValidateMaintenanceRule($input: ValidateMaintenanceRuleInput!) {
  validateMaintenanceRule(input: $input) {
    valid
    errors
  }
}`;

export const previewMaintenanceRuleMutation = `mutation PreviewMaintenanceRule($input: PreviewMaintenanceRuleInput!) {
  previewMaintenanceRule(input: $input) {
    ruleSetId
    matcherContentHash
    evaluatedAt
    titles {
      titleId
      titleName
      facet
      libraryId
      outcome
      reasonCodes
      error
    }
  }
}`;

/// Mode and arming both return the whole rule set, but the caller refetches the
/// list afterwards rather than patching state from the payload, so these
/// selections stay at the identity the caller needs to correlate the response.
export const setMaintenanceRuleModeMutation = `mutation SetMaintenanceRuleMode($input: SetMaintenanceRuleModeInput!) {
  setMaintenanceRuleMode(input: $input) {
    id
    evaluationMode
    enabled
  }
}`;

/// Arming to `DESTRUCTIVE` must acknowledge the rule's current non-terminal
/// candidate count. When it no longer matches, the server rejects the call with
/// the real count in the message and the dialog re-asks against that number.
export const setMaintenanceRuleArmingMutation = `mutation SetMaintenanceRuleArming($input: SetMaintenanceRuleArmingInput!) {
  setMaintenanceRuleArming(input: $input) {
    id
    effectArming
  }
}`;

export const setMaintenanceInstanceGatesMutation = `mutation SetMaintenanceInstanceGates($input: SetMaintenanceInstanceGatesInput!) {
  setMaintenanceInstanceGates(input: $input) {
    evaluationEnabled
    resultDisplayEnabled
    presentationEffectsEnabled
    reversibleEffectsEnabled
    destructiveEffectsEnabled
  }
}`;

export const excludeMaintenanceSubjectMutation = `mutation ExcludeMaintenanceSubject($input: ExcludeMaintenanceSubjectInput!) {
  excludeMaintenanceSubject(input: $input) {${MAINTENANCE_EXCLUSION_FIELDS}
  }
}`;

export const removeMaintenanceExclusionMutation = `mutation RemoveMaintenanceExclusion($id: ID!) {
  removeMaintenanceExclusion(id: $id) {
    id
  }
}`;

export const runMaintenanceEvaluationNowMutation = `mutation RunMaintenanceEvaluationNow($ruleSetId: ID) {
  runMaintenanceEvaluationNow(ruleSetId: $ruleSetId) {
    started
    message
  }
}`;

export const runMaintenanceActionHandlerNowMutation = `mutation RunMaintenanceActionHandlerNow {
  runMaintenanceActionHandlerNow {
    started
    message
  }
}`;

export const setTitleRequiredAudioMutation = `mutation SetTitleRequiredAudio($input: SetTitleRequiredAudioInput!) {
  setTitleRequiredAudio(input: $input) {
    titleId
    facet
    languages
    updated
  }
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const completeSetupMutation = `mutation CompleteSetup {
  completeSetup {
    completed
  }
}`;

// ── External Import (Sonarr/Radarr) ──────────────────────────────────

const EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS = `
    sessionId
    status
    phase
    startedAt
    updatedAt
    overallTotalKnown
    overallProgress { total completed failed }
    moviesTotalKnown
    moviesProgress { total completed failed }
    seriesTotalKnown
    seriesProgress { total completed failed }
    episodeFetchTotalKnown
    episodeFetchExpectedTotal
    episodeFetchExpectedMonitoredTotal
    episodeFetchProgress { total completed failed }
    snapshotBuildTotalKnown
    snapshotBuildProgress { total completed failed }
    matchedMovieCount
    matchedSeriesCount
    unmatchedMovieCount
    unmatchedSeriesCount
    ambiguousMovieCount
    ambiguousSeriesCount
    errorMessage
`;

// Lightweight per-instance connection probe for the Connect step (fired on
// blur). Does NOT start a warmup — the wizard starts that separately on success.
export const validateExternalImportConnectionMutation = `mutation ValidateExternalImportConnection($input: ValidateExternalImportConnectionInput!) {
  validateExternalImportConnection(input: $input) {
    kind
    baseUrl
    connected
    version
    error
  }
}`;

// Per-instance warmup. Runs concurrently across distinct instances; returns a
// progress snapshot whose sessionId the wizard tracks per instance.
export const startExternalImportArrSourceWarmupMutation = `mutation StartExternalImportArrSourceWarmup($input: StartExternalImportArrSourceWarmupInput!) {
  startExternalImportArrSourceWarmup(input: $input) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const startExternalImportProwlarrWarmupMutation = `mutation StartExternalImportProwlarrWarmup($input: StartExternalImportProwlarrWarmupInput!) {
  startExternalImportProwlarrWarmup(input: $input) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const cancelExternalImportArrSourceWarmupMutation = `mutation CancelExternalImportArrSourceWarmup($sessionId: ID!) {
  cancelExternalImportArrSourceWarmup(sessionId: $sessionId) {
    sessionId
    canceled
  }
}`;

export const previewExternalImportMutation = `mutation PreviewExternalImport($input: PreviewExternalImportInput!) {
  previewExternalImport(input: $input) {
    prowlarrConnected
    prowlarrVersion
    prowlarrError
    arrSources {
      sessionId
      sourceKey
      kind
      baseUrl
      connected
      version
      status
      error
    }
    rootFolders {
      sourceWarmupSessionId
      sourceKey
      kind
      arrRootPath
    }
    downloadClients {
      sourceKeys name implementation scryerClientType
      host port useSsl urlBase username apiKeyPresent
      dedupKey supported requiresPasswordOverride
    }
    indexers {
      sourceKeys name implementation scryerProviderType
      baseUrl apiKeyPresent dedupKey supported
      childCount childNames requiresApiKeyOverride apiKeyHelpUrl
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

export const finalizeExternalImportMutation = `mutation FinalizeExternalImport($input: FinalizeExternalImportInput!) {
  finalizeExternalImport(input: $input) {
    monitorWarmupSessionId
  }
}`;

// Sensitive draft (API keys / passwords) — stored server-side, encrypted,
// owner-scoped singleton. Requires ManageSystemSettings + config step-up.
export const saveExternalImportSetupSecretDraftMutation = `mutation SaveExternalImportSetupSecretDraft($input: SaveExternalImportSetupSecretDraftInput!) {
  saveExternalImportSetupSecretDraft(input: $input) {
    overwroteAnotherUserDraft
    updatedAt
  }
}`;

export const clearExternalImportSetupSecretDraftMutation = `mutation ClearExternalImportSetupSecretDraft {
  clearExternalImportSetupSecretDraft {
      cleared
}
}`;

export const rehydrateAllMetadataMutation = `mutation RehydrateAllMetadata($input: RehydrateAllMetadataInput!) {
  rehydrateAllMetadata(input: $input) {
    language
    titlesCleared
  }
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

export const deletePostProcessingScriptMutation = `mutation DeletePostProcessingScript($id: ID!) {
  deletePostProcessingScript(id: $id) {
    id
  }
}`;

export const togglePostProcessingScriptMutation = `mutation TogglePostProcessingScript($id: ID!, $inlineShellAcknowledged: Boolean) {
  togglePostProcessingScript(id: $id, inlineShellAcknowledged: $inlineShellAcknowledged) {${ppScriptFields}}
}`;

// Input type companions — keep in sync with the multi-instance external-import
// inputs in crates/scryer-interface-media-types/src/lib.rs.
export type DownloadClientApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

export type DownloadClientPasswordOverride = {
  dedupKey: string;
  password: string;
};

export type IndexerApiKeyOverride = {
  dedupKey: string;
  apiKey: string;
};

export type ExternalArrSourceKind = "SONARR" | "RADARR";
export type ExternalImportConnectionKind = "SONARR" | "RADARR" | "PROWLARR";

export type ExternalImportConnectionInput = {
  baseUrl: string;
  apiKey: string;
};

export type ValidateExternalImportConnectionInput = {
  kind: ExternalImportConnectionKind;
  connection: ExternalImportConnectionInput;
};

export type StartExternalImportArrSourceWarmupInput = {
  kind: ExternalArrSourceKind;
  connection: ExternalImportConnectionInput;
};

export type StartExternalImportProwlarrWarmupInput = {
  connection: ExternalImportConnectionInput;
};

export type PreviewExternalImportInput = {
  sourceWarmupSessionIds: string[];
  prowlarrWarmupSessionId?: string | null;
  prowlarr?: ExternalImportConnectionInput | null;
};

export type ExecuteExternalImportInput = {
  sourceWarmupSessionIds: string[];
  prowlarr?: ExternalImportConnectionInput | null;
  selectedDownloadClientDedupKeys: string[];
  selectedIndexerDedupKeys: string[];
  downloadClientApiKeyOverrides: DownloadClientApiKeyOverride[];
  downloadClientPasswordOverrides: DownloadClientPasswordOverride[];
  indexerApiKeyOverrides: IndexerApiKeyOverride[];
};

// `sourceWarmupSessionId`/`sourceKey`/`kind` are null for a manually-added root
// (one no Sonarr/Radarr instance reported); such a root only registers its
// Scryer-host path on the target library and carries no monitored status.
export type ExternalImportSourceLibraryMappingInput = {
  sourceWarmupSessionId?: string | null;
  sourceKey?: string | null;
  kind?: ExternalArrSourceKind | null;
  arrRootPath: string;
  scryerRootPath: string;
  libraryId: string;
  facet: "MOVIE" | "SERIES" | "ANIME";
};

export type FinalizeExternalImportInput = {
  sourceWarmupSessionIds: string[];
  mappings: ExternalImportSourceLibraryMappingInput[];
};

export type ExternalImportAggregateWarmupProgressInput = {
  sourceWarmupSessionIds: string[];
};

export type ExternalImportSetupInstanceApiKeyInput = {
  instanceId: string;
  kind: ExternalImportConnectionKind;
  apiKey: string;
};

export type SaveExternalImportSetupSecretDraftInput = {
  instanceApiKeys: ExternalImportSetupInstanceApiKeyInput[];
  downloadClientApiKeyOverrides: DownloadClientApiKeyOverride[];
  downloadClientPasswordOverrides: DownloadClientPasswordOverride[];
  indexerApiKeyOverrides: IndexerApiKeyOverride[];
};

// ── Subtitle mutations ──────────────────────────────────────────────────────

export const searchSubtitlesMutation = `mutation SearchSubtitles($input: SearchSubtitlesInput!) {
  searchSubtitles(input: $input) {
    provider
    providerFileId
    language
    releaseInfo
    score
    scorePercent
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
  downloadSubtitle(input: $input) {
    mediaFileId
    providerFileId
    downloaded
  }
}`;

export const deleteExternalSubtitleMutation = `mutation DeleteExternalSubtitle($input: DeleteExternalSubtitleInput!) {
  deleteExternalSubtitle(input: $input) {
    id
    deleted
  }
}`;

export const blocklistExternalSubtitleMutation = `mutation BlocklistExternalSubtitle($input: BlocklistExternalSubtitleInput!) {
  blocklistExternalSubtitle(input: $input) {
    id
    blocklisted
  }
}`;

export const clearTitleReleaseBlocklistEntryMutation = `mutation ClearTitleReleaseBlocklistEntry($id: ID!) {
  clearTitleReleaseBlocklistEntry(id: $id) {
    id
  }
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

export const cancelActiveImportMutation = `mutation CancelActiveImport($streamId: ID!) {
  cancelActiveImport(streamId: $streamId)
}`;

export const ignoreTrackedDownloadMutation = `mutation IgnoreTrackedDownload($input: IgnoreTrackedDownloadInput!) {
  ignoreTrackedDownload(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
      clientType
      downloadClientItemId
      state
      trackedState
      trackedStatus
    }
  }
}`;

export const markTrackedDownloadFailedMutation = `mutation MarkTrackedDownloadFailed($input: MarkTrackedDownloadFailedInput!) {
  markTrackedDownloadFailed(input: $input) {
    kind
    downloadClientItemId
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      clientId
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
    clientId
    clientType
    removed
    queueItem {
      id
      titleId
      titleName
      facet
      clientId
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
  scorePercent: number;
  hearingImpaired: boolean;
  forced: boolean;
  aiTranslated: boolean;
  machineTranslated: boolean;
  uploader: string | null;
  downloadCount: number | null;
  hashMatched: boolean;
};

export const setIndexerDownloadClientMappingMutation = `mutation SetIndexerDownloadClientMapping($input: SetIndexerDownloadClientMappingInput!) {
  setIndexerDownloadClientMapping(input: $input) {
    downloadClientId
  }
}`;

export const setIndexerSeedingProfileMutation = `mutation SetIndexerSeedingProfile($input: SetIndexerSeedingProfileInput!) {
  setIndexerSeedingProfile(input: $input) {
    id
    seedingProfileId
  }
}`;

export const createSeedingProfileMutation = `mutation CreateSeedingProfile($input: CreateSeedingProfileInput!) {
  createSeedingProfile(input: $input) {${SEEDING_PROFILE_FIELDS}
  }
}`;

export const updateSeedingProfileMutation = `mutation UpdateSeedingProfile($input: UpdateSeedingProfileInput!) {
  updateSeedingProfile(input: $input) {${SEEDING_PROFILE_FIELDS}
  }
}`;

export const deleteSeedingProfileMutation = `mutation DeleteSeedingProfile($id: ID!) {
  deleteSeedingProfile(id: $id) {
    id
  }
}`;

export const setDefaultSeedingProfileMutation = `mutation SetDefaultSeedingProfile($input: SetDefaultSeedingProfileInput!) {
  setDefaultSeedingProfile(input: $input) {
    seedingProfileId
    minimumSeedersFloor
  }
}`;

export const setMinimumSeedersFloorMutation = `mutation SetMinimumSeedersFloor($input: SetMinimumSeedersFloorInput!) {
  setMinimumSeedersFloor(input: $input) {
    seedingProfileId
    minimumSeedersFloor
  }
}`;

export const startLocationOperationMutation = `mutation StartLocationOperation($input: StartLocationOperationInput!) {
  startLocationOperation(input: $input) {
    planFingerprint
    operation {${LOCATION_OPERATION_FIELDS}
    }
  }
}`;

export const cancelLocationOperationMutation = `mutation CancelLocationOperation($id: ID!) {
  cancelLocationOperation(id: $id) {
    id
    cancelRequested
  }
}`;

export const resumeLocationOperationMutation = `mutation ResumeLocationOperation($id: ID!) {
  resumeLocationOperation(id: $id) {
    id
    resumed
    detail
  }
}`;
