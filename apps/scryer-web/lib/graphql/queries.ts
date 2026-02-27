export const titleDetailQuery = `query TitleDetail($id: String!) {
  title(id: $id) {
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
    createdAt
  }
  titleCollections(titleId: $id) {
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
    monitored
    createdAt
  }
  titleEvents(titleId: $id, limit: 10, offset: 0) {
    id
    eventType
    actorUserId
    titleId
    message
    occurredAt
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
  title(id: $id) {
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
    createdAt
  }
  titleCollections(titleId: $id) {
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
    monitored
    createdAt
  }
  titleEvents(titleId: $id, limit: 10, offset: 0) {
    id
    eventType
    actorUserId
    titleId
    message
    occurredAt
  }
  titleReleaseBlocklist(titleId: $id, limit: $blocklistLimit) {
    sourceHint
    sourceTitle
    errorMessage
    attemptedAt
  }
}`;

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

export function buildCollectionEpisodesBatchQuery(
  collectionIds: string[],
): { query: string; variables: Record<string, string> } {
  if (collectionIds.length === 0) {
    return { query: "query Empty { __typename }", variables: {} };
  }

  const variables: Record<string, string> = {};
  const parts: string[] = [];
  for (let i = 0; i < collectionIds.length; i++) {
    variables[`cid${i}`] = collectionIds[i];
    parts.push(`  c${i}: collectionEpisodes(collectionId: $cid${i}) {${COLLECTION_EPISODE_FIELDS}\n  }`);
  }

  const varDecls = collectionIds.map((_, i) => `$cid${i}: String!`).join(", ");
  const query = `query CollectionEpisodesBatch(${varDecls}) {\n${parts.join("\n")}\n}`;
  return { query, variables };
}

export const searchQuery = `query SearchIndexers($query: String!, $imdbId: String, $tvdbId: String, $category: String, $limit: Int) {
  searchIndexers(query: $query, imdbId: $imdbId, tvdbId: $tvdbId, category: $category, limit: $limit) {
    source
    title
    link
    downloadUrl
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
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
      }
    }
  }
}`;

export const searchSeriesEpisodeQuery = `query SearchIndexersEpisode($title: String!, $season: String!, $episode: String!, $imdbId: String, $tvdbId: String, $category: String, $limit: Int) {
  searchIndexersEpisode(title: $title, season: $season, episode: $episode, imdbId: $imdbId, tvdbId: $tvdbId, category: $category, limit: $limit) {
    source
    title
    link
    downloadUrl
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
      isDolbyVision
      detectedHdr
      parseConfidence
      isProperUpload
      isRemux
      isBdDisk
    }
    qualityProfileDecision {
      allowed
      blockCodes
      releaseScore
      preferenceScore
      scoringLog {
        code
        delta
      }
    }
  }
}`;

export const titlesQuery = `query Titles($facet: String, $query: String) {
  titles(facet: $facet, query: $query) {
    id
    name
    facet
    monitored
    posterUrl
    qualityTier
    sizeBytes
    externalIds {
      source
      value
    }
  }
}`;

export const titleCollectionsQuery = `query TitleCollections($titleId: String!) {
  titleCollections(titleId: $titleId) {
    id
    collectionType
    collectionIndex
    label
    orderedPath
    narrativeOrder
    fileSizeBytes
    firstEpisodeNumber
    lastEpisodeNumber
  }
}`;

export const collectionEpisodesQuery = `query CollectionEpisodes($collectionId: String!) {
  collectionEpisodes(collectionId: $collectionId) {
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
    monitored
    createdAt
  }
}`;

export const titleMediaFilesQuery = `query TitleMediaFiles($titleId: String!) {
  titleMediaFiles(titleId: $titleId) {
    id
    titleId
    episodeId
    filePath
    sizeBytes
    qualityLabel
    scanStatus
    createdAt
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
    createdAt
    updatedAt
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

export const adminSettingsQuery = `query AdminSettings($scope: String, $scopeId: String, $category: String) {
  adminSettings(scope: $scope, scopeId: $scopeId, category: $category) {
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

export const downloadQueueQuery = `query DownloadQueue($includeAllActivity: Boolean, $includeHistoryOnly: Boolean) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly) {
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
  }
}`;

export const downloadQueueSubscription = `subscription DownloadQueueStream($includeAllActivity: Boolean, $includeHistoryOnly: Boolean) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly) {
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
  }
}`;

// Shared field selections for batched queries
const adminSettingsFieldSelection = `
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
    createdAt
    updatedAt`;

// Batched query for quality profiles page: 5 requests → 1
export const qualityProfilesInitQuery = `query QualityProfilesInit {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  systemSettings: adminSettings(scope: "system") {${adminSettingsFieldSelection}
  }
  movieSettings: adminSettings(scope: "system", scopeId: "movie", category: "media") {${adminSettingsFieldSelection}
  }
  seriesSettings: adminSettings(scope: "system", scopeId: "series", category: "media") {${adminSettingsFieldSelection}
  }
  animeSettings: adminSettings(scope: "system", scopeId: "anime", category: "media") {${adminSettingsFieldSelection}
  }
}`;

// Batched query for media settings: 5 requests → 1
export const mediaSettingsInitQuery = `query MediaSettingsInit {
  mediaSettings: adminSettings(scope: "media", category: "media") {${adminSettingsFieldSelection}
  }
  systemSettings: adminSettings(scope: "system", category: "media") {${adminSettingsFieldSelection}
  }
  movieSettings: adminSettings(scope: "system", scopeId: "movie", category: "media") {${adminSettingsFieldSelection}
  }
  seriesSettings: adminSettings(scope: "system", scopeId: "series", category: "media") {${adminSettingsFieldSelection}
  }
  animeSettings: adminSettings(scope: "system", scopeId: "anime", category: "media") {${adminSettingsFieldSelection}
  }
}`;

// TLS settings query
export const tlsSettingsQuery = `query TlsSettings {
  serviceSettings: adminSettings(scope: "system", category: "service") {${adminSettingsFieldSelection}
  }
}`;

// Acquisition settings query
export const acquisitionSettingsQuery = `query AcquisitionSettings {
  acquisitionSettings: adminSettings(scope: "system", category: "acquisition") {${adminSettingsFieldSelection}
  }
}`;

// Batched query for download client routing: 2 requests → 1
export const downloadClientRoutingInitQuery = `query DownloadClientRoutingInit($scopeId: String!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  categorySettings: adminSettings(scope: "system", scopeId: $scopeId, category: "media") {${adminSettingsFieldSelection}
  }
}`;

// Batched query for indexer routing: 2 requests → 1
export const indexerRoutingInitQuery = `query IndexerRoutingInit($scopeId: String!) {
  indexers {${indexerFieldSelection}
  }
  categorySettings: adminSettings(scope: "system", scopeId: $scopeId, category: "media") {${adminSettingsFieldSelection}
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
  }
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

export const wantedItemsQuery = `query WantedItems($status: String, $mediaType: String, $titleId: String, $limit: Int, $offset: Int) {
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

export const releaseDecisionsQuery = `query ReleaseDecisions($wantedItemId: String, $titleId: String, $limit: Int) {
  releaseDecisions(wantedItemId: $wantedItemId, titleId: $titleId, limit: $limit) {
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
}`;
