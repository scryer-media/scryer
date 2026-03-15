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
    bannerUrl
    backgroundUrl
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
    interstitialMovie {
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
    }
    specialsMovies {
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
    }
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
    bannerUrl
    backgroundUrl
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
    interstitialMovie {
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
    }
    specialsMovies {
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
    }
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

export function buildCollectionEpisodesBatchQuery(collectionIds: string[]): {
  query: string;
  variables: Record<string, string>;
} {
  if (collectionIds.length === 0) {
    return { query: "query Empty { __typename }", variables: {} };
  }

  const variables: Record<string, string> = {};
  const parts: string[] = [];
  for (let i = 0; i < collectionIds.length; i++) {
    variables[`cid${i}`] = collectionIds[i];
    parts.push(
      `  c${i}: collectionEpisodes(collectionId: $cid${i}) {${COLLECTION_EPISODE_FIELDS}\n  }`,
    );
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
      }
    }
  }
}`;

export const searchSeriesEpisodeQuery = `query SearchIndexersEpisode($title: String!, $season: String!, $episode: String!, $imdbId: String, $tvdbId: String, $category: String, $absoluteEpisode: Int, $limit: Int) {
  searchIndexersEpisode(title: $title, season: $season, episode: $episode, imdbId: $imdbId, tvdbId: $tvdbId, category: $category, absoluteEpisode: $absoluteEpisode, limit: $limit) {
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
      }
    }
  }
}`;

export const searchSeasonQuery = `query SearchIndexersSeason($title: String!, $season: String!, $imdbId: String, $tvdbId: String, $category: String, $limit: Int) {
  searchIndexersSeason(title: $title, season: $season, imdbId: $imdbId, tvdbId: $tvdbId, category: $category, limit: $limit) {
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
    tags
    imdbId
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
    interstitialMovie {
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
    }
    specialsMovies {
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
    }
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
    releaseHash
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
    lastQueryAt
    configJson
    createdAt
    updatedAt
  }
}`;

export const indexerProviderTypesQuery = `query IndexerProviderTypes {
  indexerProviderTypes {
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
    }
  }
}`;

export const downloadClientProviderTypesQuery = `query DownloadClientProviderTypes {
  downloadClientProviderTypes {
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
    }
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

export const adminSettingsQuery = `query AdminSettings($scope: String, $scopeId: String, $category: String, $keyNames: [String!]) {
  adminSettings(scope: $scope, scopeId: $scopeId, category: $category, keyNames: $keyNames) {
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
    configJson
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

export const rootFoldersQuery = `query RootFolders($facet: String!) {
  rootFolders(facet: $facet) { path isDefault }
}`;

// Batched query for media settings with current-scope key filtering.
export const mediaSettingsInitQuery = `query MediaSettingsInit(
  $scopeId: String!
  $facet: String!
  $mediaKeyNames: [String!]
  $systemKeyNames: [String!]
  $categoryKeyNames: [String!]
) {
  systemSettings: adminSettings(scope: "system", category: "media", keyNames: $systemKeyNames) {${adminSettingsFieldSelection}
  }
  mediaSettings: adminSettings(scope: "system", category: "media", keyNames: $mediaKeyNames) {${adminSettingsFieldSelection}
  }
  categorySettings: adminSettings(scope: "system", scopeId: $scopeId, category: "media", keyNames: $categoryKeyNames) {${adminSettingsFieldSelection}
  }
  rootFolders(facet: $facet) { path isDefault }
}`;

export const globalSearchInitQuery = `query GlobalSearchInit(
  $systemKeyNames: [String!]
  $movieKeyNames: [String!]
  $seriesKeyNames: [String!]
  $animeKeyNames: [String!]
) {
  systemSettings: adminSettings(scope: "system", category: "media", keyNames: $systemKeyNames) {${adminSettingsFieldSelection}
  }
  movieSettings: adminSettings(scope: "system", scopeId: "movie", category: "media", keyNames: $movieKeyNames) {${adminSettingsFieldSelection}
  }
  seriesSettings: adminSettings(scope: "system", scopeId: "series", category: "media", keyNames: $seriesKeyNames) {${adminSettingsFieldSelection}
  }
  animeSettings: adminSettings(scope: "system", scopeId: "anime", category: "media", keyNames: $animeKeyNames) {${adminSettingsFieldSelection}
  }
  movieRootFolders: rootFolders(facet: "movie") { path isDefault }
  seriesRootFolders: rootFolders(facet: "tv") { path isDefault }
  animeRootFolders: rootFolders(facet: "anime") { path isDefault }
}`;

// Batched query for routing page bootstrap.
export const routingPageInitQuery = `query RoutingPageInit($scopeId: String!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  indexers {${indexerFieldSelection}
  }
  categorySettings: adminSettings(scope: "system", scopeId: $scopeId, category: "media") {${adminSettingsFieldSelection}
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

export const postProcessingSettingsQuery = `query PostProcessingSettings {
  postProcessingSettings: adminSettings(scope: "system", category: "post_processing") {${adminSettingsFieldSelection}
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

export const importHistoryChangedSubscription = `subscription ImportHistoryChanged {
  importHistoryChanged
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

export const ruleSetsQuery = `query RuleSets {
  ruleSets {
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

// ── Notifications ─────────────────────────────────────────────────────

export const notificationChannelsQuery = `query NotificationChannels {
  notificationChannels {
    id
    name
    channelType
    configJson
    isEnabled
    createdAt
    updatedAt
  }
}`;

export const notificationSubscriptionsQuery = `query NotificationSubscriptions {
  notificationSubscriptions {
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

export const notificationProviderTypesQuery = `query NotificationProviderTypes {
  notificationProviderTypes {
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
    }
  }
}`;

export const notificationEventTypesQuery = `query NotificationEventTypes {
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
