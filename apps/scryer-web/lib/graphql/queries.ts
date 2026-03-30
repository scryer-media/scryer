const TITLE_CORE_FIELDS = `
    id
    name
    facet
    monitored
    tags
    externalIds {
      source
      value
    }
    year
    overview
    posterUrl
    posterSourceUrl
    bannerUrl
    bannerSourceUrl
    backgroundUrl
    backgroundSourceUrl
    sortTitle
    slug
    imdbId
    runtimeMinutes
    genres
    contentStatus
    language
    firstAired
    network
    studio
    country
    aliases
    metadataLanguage
    metadataFetchedAt
    qualityProfileId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt`;

const INTERSTITIAL_MOVIE_FIELDS = `
      tvdbId
      name
      slug
      year
      contentStatus
      overview
      posterUrl
      language
      runtimeMinutes
      sortTitle
      imdbId
      genres
      studio
      digitalReleaseDate
      associationConfidence
      continuityStatus
      movieForm
      confidence
      signalSummary
      placement
      movieTmdbId
      movieMalId`;

const COLLECTION_EPISODE_FIELDS = `
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
      isRecap
      absoluteNumber
      monitored
      createdAt`;

const TITLE_COLLECTION_FIELDS = `
      id
      titleId
      collectionType
      collectionIndex
      label
      orderedPath
      narrativeOrder
      fileSizeBytes
      firstEpisodeNumber
      lastEpisodeNumber
      interstitialMovie {${INTERSTITIAL_MOVIE_FIELDS}
      }
      specialsMovies {${INTERSTITIAL_MOVIE_FIELDS}
      }
      interstitialSeasonEpisode
      monitored
      createdAt
      episodes {${COLLECTION_EPISODE_FIELDS}
      }`;

const TITLE_MEDIA_FILE_FIELDS = `
      id
      titleId
      episodeId
      filePath
      sizeBytes
      qualityLabel
      scanStatus
      createdAt
      videoCodec
      videoWidth
      videoHeight
      videoBitrateKbps
      videoBitDepth
      videoHdrFormat
      videoFrameRate
      videoProfile
      audioCodec
      audioChannels
      audioBitrateKbps
      audioLanguages
      audioStreams {
        codec
        channels
        language
        bitrateKbps
      }
      subtitleLanguages
      subtitleCodecs
      subtitleStreams {
        codec
        language
        name
        forced
        default
      }
      hasMultiaudio
      durationSeconds
      numChapters
      containerFormat
      sceneName
      releaseGroup
      sourceType
      resolution
      videoCodecParsed
      audioCodecParsed
      acquisitionScore
      scoringLog
      indexerSource
      grabbedReleaseTitle
      grabbedAt
      edition
      originalFilePath
      releaseHash`;

const WANTED_ITEM_FIELDS = `
      id
      titleId
      titleName
      episodeId
      collectionId
      mediaType
      searchPhase
      nextSearchAt
      lastSearchAt
      searchCount
      baselineDate
      status
      grabbedRelease
      currentScore
      createdAt
      updatedAt`;

const DOWNLOAD_QUEUE_ITEM_FIELDS = `
    id
    titleId
    titleName
    facet
    isScryerOrigin
    clientId
    clientName
    clientType
    state
    progressPercent
    sizeBytes
    remainingSeconds
    queuedAt
    lastUpdatedAt
    attentionRequired
    attentionReason
    downloadClientItemId
    importStatus
    importErrorMessage
    importedAt
    trackedState
    trackedStatus
    trackedStatusMessages
    trackedMatchType`;

const SUBTITLE_DOWNLOAD_FIELDS = `
    id
    mediaFileId
    language
    provider
    filePath
    score
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    releaseInfo
    synced
    downloadedAt`;

const PROVIDER_TYPE_FIELDS = `
    providerType
    name
    defaultBaseUrl
    configFields {
      key
      label
      fieldType
      required
      defaultValue
      options { value label }
      helpText
    }`;

const NOTIFICATION_CHANNEL_FIELDS = `
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt`;

const NOTIFICATION_SUBSCRIPTION_FIELDS = `
    id
    channelId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt`;

export const titleDetailQuery = `query TitleDetail($id: String!) {
  title(id: $id) {${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
  }
  titleEvents(titleId: $id, limit: 50, offset: 0) {
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    dataJson
    occurredAt
    createdAt
  }
}`;

export const titleReleaseBlocklistQuery = `query TitleReleaseBlocklist($titleId: String!, $limit: Int) {
  titleReleaseBlocklist(titleId: $titleId, limit: $limit) {
    sourceHint
    sourceTitle
    errorMessage
    attemptedAt
  }
}`;

export const titleOverviewInitQuery = `query TitleOverviewInit($id: String!, $blocklistLimit: Int) {
  title(id: $id) {${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
    wantedItems {${WANTED_ITEM_FIELDS}
    }
    downloadQueueItems {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
  }
  titleEvents(titleId: $id, limit: 50, offset: 0) {
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    dataJson
    occurredAt
    createdAt
  }
  titleReleaseBlocklist(titleId: $id, limit: $blocklistLimit) {
    sourceHint
    sourceTitle
    errorMessage
    attemptedAt
  }
  subtitleDownloads(titleId: $id) {${SUBTITLE_DOWNLOAD_FIELDS}
  }
}`;

export const searchQuery = `query SearchIndexers($query: String!, $imdbId: String, $tvdbId: String, $category: String, $limit: Int) {
  searchReleases(input: {
    query: $query,
    imdbId: $imdbId,
    tvdbId: $tvdbId,
    category: $category,
    limit: $limit
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchSeriesEpisodeQuery = `query SearchIndexersEpisode($title: String!, $season: String!, $episode: String!, $imdbId: String, $tvdbId: String, $anidbId: String, $category: String, $absoluteEpisode: Int) {
  searchReleases(input: {
    query: $title,
    season: $season,
    episode: $episode,
    imdbId: $imdbId,
    tvdbId: $tvdbId,
    anidbId: $anidbId,
    category: $category,
    absoluteEpisode: $absoluteEpisode
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchForTitleQuery = `query SearchIndexersForTitle($titleId: String!) {
  searchReleases(input: { titleId: $titleId }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const searchForEpisodeQuery = `query SearchIndexersForEpisode($titleId: String!, $season: String!, $episode: String!) {
  searchReleases(input: {
    titleId: $titleId,
    season: $season,
    episode: $episode
  }) {
    source
    title
    link
    downloadUrl
    sourceKind
    sizeBytes
    publishedAt
    thumbsUp
    thumbsDown
    parsedRelease {
      rawTitle
      normalizedTitle
      releaseGroup
      quality
      source
      videoCodec
      videoEncoding
      audio
      isDualAudio
      isAtmos
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
      isAiEnhanced
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
        source
        ruleSetName
      }
    }
    seeders
    peers
    infoHash
    freeleech
    downloadVolumeFactor
  }
}`;

export const titlesQuery = `query Titles($facet: MediaFacetValue, $query: String) {
  titles(facet: $facet, query: $query) {
    id
    name
    facet
    monitored
    tags
    imdbId
    posterUrl
    posterSourceUrl
    qualityTier
    sizeBytes
    contentStatus
    externalIds {
      source
      value
    }
  }
}`;

export const mediaRenamePreviewQuery = `query MediaRenamePreview($input: MediaRenamePreviewInput!) {
  mediaRenamePreview(input: $input) {
    facet
    titleId
    template
    collisionPolicy
    missingMetadataPolicy
    fingerprint
    total
    renamable
    noop
    conflicts
    errors
    items {
      collectionId
      currentPath
      proposedPath
      normalizedFilename
      collision
      reasonCode
      writeAction
      sourceSizeBytes
      sourceMtimeUnixMs
    }
  }
}`;

export const activityQuery = `query Activity($limit: Int, $offset: Int) {
  activityEvents(limit: $limit, offset: $offset) {
    id
    kind
    severity
    channels
    message
    actorUserId
    titleId
    occurredAt
  }
}`;

export const activitySubscriptionQuery = `subscription ActivityStream {
  activityEvents {
    id
    kind
    severity
    channels
    actorUserId
    titleId
    facet
    message
    occurredAt
  }
}`;

export const usersQuery = `query Users {
  users {
    id
    username
    entitlements
  }
}`;

export const indexersQuery = `query Indexers($providerType: String) {
  indexers(providerType: $providerType) {
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
    lastQueryAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const indexerProviderTypesQuery = `query IndexerProviderTypes {
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const downloadClientProviderTypesQuery = `query DownloadClientProviderTypes {
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const downloadClientsQuery = `query DownloadClients {
  downloadClientConfigs {
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

export const downloadQueueQuery = `query DownloadQueue($includeAllActivity: Boolean, $includeHistoryOnly: Boolean) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const downloadHistoryQuery = `query DownloadHistory($limit: Int, $offset: Int) {
  downloadHistory(limit: $limit, offset: $offset) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
  }
}`;

export const downloadQueueSubscription = `subscription DownloadQueueStream($includeAllActivity: Boolean, $includeHistoryOnly: Boolean) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

const downloadClientFieldSelection = `
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
    updatedAt`;

const indexerFieldSelection = `
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
    updatedAt`;

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

// Batched query for quality profiles page: 5 requests → 1
export const qualityProfilesInitQuery = `query QualityProfilesInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
}`;

export const movieOverviewSettingsInitQuery = `query MovieOverviewSettingsInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
}`;

export const seriesOverviewSettingsInitQuery = `query SeriesOverviewSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const wantedCutoffInitQuery = `query WantedCutoffInit {
  titles {
    id
    name
    monitored
    facet
    posterUrl
    qualityTier
  }
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
}`;

export const downloadClientsInitQuery = `query DownloadClientsInit {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const indexersInitQuery = `query IndexersInit($providerType: String) {
  indexers(providerType: $providerType) {${indexerFieldSelection}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const setupWizardProviderTypesInitQuery = `query SetupWizardProviderTypesInit {
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const rootFoldersQuery = `query RootFolders($facet: MediaFacetValue!) {
  rootFolders(facet: $facet) { path isDefault }
}`;

export const mediaSettingsInitQuery = `query MediaSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const globalSearchInitQuery = `query GlobalSearchInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  movieSettings: mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
  seriesSettings: mediaSettings(scope: series) {${mediaSettingsFieldSelection}
  }
  animeSettings: mediaSettings(scope: anime) {${mediaSettingsFieldSelection}
  }
}`;

// Batched query for routing page bootstrap.
export const routingPageInitQuery = `query RoutingPageInit($scopeId: ContentScopeValue!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  indexers {${indexerFieldSelection}
  }
  downloadClientRouting(scope: $scopeId) {${downloadClientRoutingFieldSelection}
  }
  indexerRouting(scope: $scopeId) {${indexerRoutingFieldSelection}
  }
}`;

// TLS settings query
export const tlsSettingsQuery = `query TlsSettings {
  serviceSettings {${serviceSettingsFieldSelection}
  }
}`;

// Acquisition settings query
export const acquisitionSettingsQuery = `query AcquisitionSettings {
  acquisitionSettings {
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

export const delayProfilesQuery = `query DelayProfiles {
  delayProfiles {
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

export const libraryPathsQuery = `query LibraryPaths {
  libraryPaths {${libraryPathsFieldSelection}
  }
}`;

export const subtitleSettingsQuery = `query SubtitleSettings {
  subtitleSettings {
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

// Batched query for download client routing: 2 requests → 1
export const downloadClientRoutingInitQuery = `query DownloadClientRoutingInit($scopeId: ContentScopeValue!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientRouting(scope: $scopeId) {${downloadClientRoutingFieldSelection}
  }
}`;

// Batched query for indexer routing: 2 requests → 1
export const indexerRoutingInitQuery = `query IndexerRoutingInit($scopeId: ContentScopeValue!) {
  indexers {${indexerFieldSelection}
  }
  indexerRouting(scope: $scopeId) {${indexerRoutingFieldSelection}
  }
}`;

export const meQuery = `query Me {
  me {
    id
    username
    entitlements
  }
}`;

export const importHistoryQuery = `query ImportHistory($limit: Int) {
  importHistory(limit: $limit) {
    id
    sourceSystem
    sourceRef
    sourceTitle
    importType
    status
    errorMessage
    decision
    skipReason
    titleId
    sourcePath
    destPath
    startedAt
    finishedAt
    createdAt
  }
}`;

export const importHistoryChangedSubscription = `subscription ImportHistoryChanged {
  importHistoryChanged
}`;

export const settingsChangedSubscription = `subscription SettingsChanged {
  settingsChanged
}`;

export const systemHealthQuery = `query SystemHealth {
  systemHealth {
    serviceReady
    dbPath
    totalTitles
    monitoredTitles
    totalUsers
    titlesMovie
    titlesTv
    titlesAnime
    titlesOther
    recentEvents
    recentEventPreview
    dbMigrationVersion
    dbPendingMigrations
    smgCertExpiresAt
    smgCertDaysRemaining
    indexerStats {
      indexerId
      indexerName
      queriesLast24H
      successfulLast24H
      failedLast24H
      lastQueryAt
      apiCurrent
      apiMax
      grabCurrent
      grabMax
    }
  }
}`;

export const serviceLogsQuery = `query ServiceLogs($limit: Int) {
  serviceLogs(limit: $limit) {
    generatedAt
    lines
    count
  }
}`;

export const serviceLogLinesSubscription = `subscription ServiceLogLines {
  serviceLogLines
}`;

export const previewManualImportQuery = `query PreviewManualImport($downloadClientItemId: String!, $titleId: String!) {
  previewManualImport(downloadClientItemId: $downloadClientItemId, titleId: $titleId) {
    files {
      filePath
      fileName
      sizeBytes
      quality
      parsedSeason
      parsedEpisodes
      suggestedEpisodeId
      suggestedEpisodeLabel
    }
    availableEpisodes {
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      monitored
    }
  }
}`;

export const wantedItemsQuery = `query WantedItems($status: WantedStatusValue, $mediaType: WantedMediaTypeValue, $titleId: String, $limit: Int, $offset: Int) {
  wantedItems(status: $status, mediaType: $mediaType, titleId: $titleId, limit: $limit, offset: $offset) {
    items {
      id
      titleId
      titleName
      episodeId
      mediaType
      searchPhase
      nextSearchAt
      lastSearchAt
      searchCount
      baselineDate
      status
      grabbedRelease
      currentScore
      createdAt
      updatedAt
    }
    total
  }
}`;

export const releaseDecisionsQuery = `query ReleaseDecisions($wantedItemId: String!, $limit: Int) {
  wantedItem(id: $wantedItemId) {
    id
    releaseDecisions(limit: $limit) {
      id
      wantedItemId
      titleId
      releaseTitle
      releaseUrl
      releaseSizeBytes
      decisionCode
      candidateScore
      currentScore
      scoreDelta
      explanationJson
      createdAt
    }
  }
}`;

export const pluginsQuery = `query Plugins {
  plugins {
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

export const recycledItemsQuery = `query RecycledItems($limit: Int, $offset: Int) {
  recycledItems(limit: $limit, offset: $offset) {
    items {
      id
      originalPath
      fileName
      sizeBytes
      titleId
      reason
      recycledAt
      mediaRoot
    }
    totalCount
  }
}`;

export const ruleSetsQuery = `query RuleSets {
  ruleSets {
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

export const convenienceSettingsQuery = `query ConvenienceSettings {
  convenienceSettings {
    requiredAudio { scope languages ruleSetId }
  }
}`;

// ── Community Rule Packs ──────────────────────────────────────────────

export const rulePackRegistryQuery = `query RulePackRegistry {
  rulePackRegistry {
    id
    name
    description
    author
    version
  }
}`;

export const rulePackTemplatesQuery = `query RulePackTemplates($packId: String!) {
  rulePackTemplates(packId: $packId) {
    id
    title
    description
    category
    regoSource
    appliedFacets
  }
}`;

// ── Notifications ─────────────────────────────────────────────────────

export const notificationChannelsQuery = `query NotificationChannels {
  notificationChannels {${NOTIFICATION_CHANNEL_FIELDS}
  }
}`;

export const notificationSubscriptionsQuery = `query NotificationSubscriptions {
  notificationSubscriptions {${NOTIFICATION_SUBSCRIPTION_FIELDS}
  }
}`;

export const notificationProviderTypesQuery = `query NotificationProviderTypes {
  notificationProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const notificationEventTypesQuery = `query NotificationEventTypes {
  notificationEventTypes
}`;

export const notificationsInitQuery = `query NotificationsInit {
  notificationChannels {${NOTIFICATION_CHANNEL_FIELDS}
  }
  notificationSubscriptions {${NOTIFICATION_SUBSCRIPTION_FIELDS}
  }
  notificationProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  notificationEventTypes
}`;

// ── Metadata Gateway (proxied through backend) ────────────────────────

const METADATA_SEARCH_FIELDS = `
    tvdbId
    name
    imdbId
    slug
    type
    year
    status
    overview
    popularity
    posterUrl
    language
    runtimeMinutes
    sortTitle`;

export const searchMetadataQuery = `query SearchMetadata($query: String!, $type: String!, $limit: Int, $language: String) {
  searchMetadata(query: $query, type: $type, limit: $limit, language: $language) {${METADATA_SEARCH_FIELDS}
  }
}`;

export const searchMetadataMultiQuery = `query SearchMetadataMulti($query: String!, $limit: Int, $language: String) {
  searchMetadataMulti(query: $query, limit: $limit, language: $language) {
    movies {${METADATA_SEARCH_FIELDS}
    }
    series {${METADATA_SEARCH_FIELDS}
    }
    anime {${METADATA_SEARCH_FIELDS}
    }
  }
}`;

export const metadataMovieQuery = `query MetadataMovie($tvdbId: Int!, $language: String) {
  metadataMovie(tvdbId: $tvdbId, language: $language) {
    tvdbId
    name
    slug
    year
    status
    overview
    posterUrl
    language
    runtimeMinutes
    sortTitle
    imdbId
    genres
    studio
    tmdbReleaseDate
  }
}`;

export const metadataSeriesQuery = `query MetadataSeries($id: String!, $includeEpisodes: Boolean, $language: String) {
  metadataSeries(id: $id, includeEpisodes: $includeEpisodes, language: $language) {
    tvdbId
    name
    sortName
    slug
    year
    status
    firstAired
    overview
    network
    runtimeMinutes
    posterUrl
    country
    genres
    aliases
    seasons {
      tvdbId
      number
      label
      episodeType
    }
    episodes {
      tvdbId
      episodeNumber
      seasonNumber
      name
      aired
      runtimeMinutes
      isFiller
    }
  }
}`;

export const pendingReleasesQuery = `query PendingReleases {
  pendingReleases {
    id
    wantedItemId
    titleId
    releaseTitle
    releaseUrl
    releaseSizeBytes
    releaseScore
    scoringLogJson
    indexerSource
    addedAt
    delayUntil
    status
  }
}`;

export const calendarEpisodesQuery = `query CalendarEpisodes($startDate: String!, $endDate: String!) {
  calendarEpisodes(startDate: $startDate, endDate: $endDate) {
    id
    titleId
    titleName
    titleFacet
    seasonNumber
    episodeNumber
    episodeTitle
    airDate
    monitored
  }
}`;

// ── Setup Wizard ──────────────────────────────────────────────────────

export const setupStatusQuery = `query SetupStatus {
  setupStatus {
    setupComplete
    hasDownloadClients
    hasIndexers
  }
}`;

export const browsePathQuery = `query BrowsePath($path: String!) {
  browsePath(path: $path) {
    name
    path
  }
}`;

export const postProcessingScriptsQuery = `query PostProcessingScripts {
  postProcessingScripts {
    id
    name
    description
    scriptType
    scriptContent
    appliedFacets
    executionMode
    timeoutSecs
    priority
    enabled
    debug
    createdAt
    updatedAt
  }
}`;

export const postProcessingScriptRunsQuery = `query PostProcessingScriptRuns($scriptId: String!, $limit: Int) {
  postProcessingScriptRuns(scriptId: $scriptId, limit: $limit) {
    id
    scriptId
    scriptName
    titleId
    titleName
    facet
    filePath
    status
    exitCode
    stdoutTail
    stderrTail
    durationMs
    startedAt
    completedAt
  }
}`;

export const subtitleDownloadsQuery = `query SubtitleDownloads($titleId: String!) {
  subtitleDownloads(titleId: $titleId) {${SUBTITLE_DOWNLOAD_FIELDS}
  }
}`;

export const titleHistoryQuery = `query TitleHistory($filter: TitleHistoryFilterInput!) {
  titleHistory(filter: $filter) {
    records {
      id
      titleId
      episodeId
      collectionId
      eventType
      sourceTitle
      quality
      downloadId
      dataJson
      occurredAt
      createdAt
    }
    totalCount
  }
}`;

export const episodeHistoryQuery = `query EpisodeHistory($episodeId: String!, $limit: Int) {
  episodeHistory(episodeId: $episodeId, limit: $limit) {
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    dataJson
    occurredAt
    createdAt
  }
}`;
