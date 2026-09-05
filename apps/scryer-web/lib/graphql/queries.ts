export const DISCOVERY_ITEM_FIELDS = `
    id
    targetKey
    targetKind
    resolved
    resolvedTitleId
    displayTitle
    originalTitle
    sortTitle
    year
    posterUrl
    backgroundUrl
    overview
    contentType
    isAdult
    canonicalTags {
      key
      category
      name
      confidence
      sources
      sourceTagKeys
      isAdult
      isSpoiler
    }
    rating
    externalIds {
      source
      kind
      id
      key
    }
    statusTags
    sourceTags
    sources
    bestSource
    relationTypes
    relationSubtypes
    sourceCount
    edgeCount
    relationCount
    sourceSubjectCount
    rankScore
    matchedSubjectTitles
    matchedSubjectCount
    tmdbCollectionId
    tmdbCollectionName
    ownedInInput
    facetTerms
    contextTerms
    studioSlug
    personIds`;

const DISCOVERY_ITEM_DETAIL_FIELDS = `${DISCOVERY_ITEM_FIELDS}
    externalRatings {
      source
      value
      score
      normalized
      votes
      url
    }`;

const DISCOVERY_HOME_CARD_FIELDS = `
    id
    targetKey
    targetKind
    displayTitle
    originalTitle
    sortTitle
    year
    posterUrl
    contentType
    isAdult
    ownedInInput`;

const DISCOVERY_HOME_HERO_FIELDS = `${DISCOVERY_HOME_CARD_FIELDS}
    backgroundUrl
    overview
    rating
    ratingSources
    externalRatings {
      source
      value
      score
      normalized
      votes
      url
    }
    genreTags {
      key
      category
      name
      confidence
      sources
      sourceTagKeys
      isAdult
      isSpoiler
    }
    matchedSubjectCount`;

export const DISCOVERY_SECTION_FIELDS = `
    sectionId
    sectionType
    title
    surface
    totalCount
    items {
${DISCOVERY_ITEM_FIELDS}
    }`;

const DISCOVERY_HOME_SECTION_FIELDS = `
    sectionId
    sectionType
    title
    surface
    totalCount
    items {
${DISCOVERY_HOME_CARD_FIELDS}
    }`;

export const discoveryHomeCardsQuery = `query DiscoveryHomeCards($input: DiscoveryHomeInput) {
  discoveryHomeCards(input: $input) {
    canViewPersonalized
    status {
      pendingContextChangeCount
    }
    heroItem {
${DISCOVERY_HOME_HERO_FIELDS}
    }
    publicSections {
${DISCOVERY_HOME_SECTION_FIELDS}
    }
    personalizedSections {
${DISCOVERY_HOME_SECTION_FIELDS}
    }
    completeCollection {
${DISCOVERY_HOME_SECTION_FIELDS}
    }
  }
}`;

export const discoveryHomeFilterOptionsQuery = `query DiscoveryHomeFilterOptions($input: DiscoveryHomeFilterOptionsInput) {
  discoveryHomeFilterOptions(input: $input) {
    genres { key name }
    themes { key name }
    studioSlugs
  }
}`;

export const discoveryItemsQuery = `query DiscoveryItems($input: DiscoveryItemsInput) {
  discoveryItems(input: $input) {
    totalCount
    canViewPersonalized
    items {
${DISCOVERY_ITEM_FIELDS}
    }
  }
}`;

export const discoveryItemDetailQuery = `query DiscoveryItemDetail($input: DiscoveryItemDetailInput!) {
  discoveryItemDetail(input: $input) {
${DISCOVERY_ITEM_DETAIL_FIELDS}
  }
}`;

const CATALOG_DISCOVERY_ITEM_FIELDS = `
    id
    targetKey
    targetKind
    resolved
    resolvedTitleId
    displayTitle
    originalTitle
    sortTitle
    year
    posterUrl
    backgroundUrl
    contentType
    isAdult
    canonicalTags {
      key
      category
      name
      confidence
      sources
      sourceTagKeys
      isAdult
      isSpoiler
    }
    externalIds {
      source
      kind
      id
      key
    }
    statusTags
    sourceTags
    rankScore
    ownedInInput`;

export const catalogDiscoveryQuery = `query CatalogDiscovery($input: CatalogDiscoveryInput!) {
  catalogDiscovery(input: $input) {
    canViewPersonalized
    groups {
      id
      kind
      surface
      labelValue
      totalCount
      items {
${CATALOG_DISCOVERY_ITEM_FIELDS}
      }
    }
  }
}`;

const TITLE_CANONICAL_TAG_FIELDS = `
      key
      category
      name
      confidence
      sources
      sourceTagKeys
      isAdult
      isSpoiler`;

const TITLE_RATING_SUMMARY_FIELDS = `
      rating
      ratingSources
      externalRatings {
        source
        value
        score
        normalized
        votes
        url
      }`;

/**
 * Credit kinds the metadata provider uses for on-screen performers. Crew kinds
 * (`director`, `writer`, `creator`, `composer`) are cached server-side but have
 * no UI yet, so the overview asks for exactly these.
 */
export const TITLE_CAST_CREDIT_KINDS = ["actor", "voice_actor"] as const;

/**
 * Credits fetched per title — the server's clamp. The overview splits these
 * into an original-cast rail and a dub rail (anime carries a Japanese and an
 * English voice actor per character), so fetching only one rail's worth would
 * starve whichever cast sorted second.
 */
export const TITLE_CAST_CREDIT_LIMIT = 50;

/**
 * Cast for the overview rails. The server does the kind filtering and
 * billing-order sort; the language split and per-rail cap happen client-side in
 * lib/utils/title-cast.
 */
const TITLE_CAST_CREDIT_FIELDS = `
      kind
      personName
      personOriginalName
      personImageUrl
      character
      language
      billingOrder
      episodeCount`;

const TITLE_CAST_CREDIT_SELECTION = `
    credits(kinds: [${TITLE_CAST_CREDIT_KINDS.map((kind) => `"${kind}"`).join(", ")}], limit: ${TITLE_CAST_CREDIT_LIMIT}) {${TITLE_CAST_CREDIT_FIELDS}
    }`;

export const PROVIDER_CONFIG_VALUE_FIELDS = `
    key
    label
    fieldType
    required
    defaultValue
    valueSource
    role
    hostBinding
    options { value label }
    helpText
    value {
      __typename
      ... on StringConfigValuePayload { stringValue: value }
      ... on BoolConfigValuePayload { boolValue: value }
      ... on IntConfigValuePayload { intValue: value }
      ... on FloatConfigValuePayload { floatValue: value }
      ... on SecretConfigValuePayload { stored }
    }`;

const SERIES_SIDE_PANEL_MOVIE_LINK_FIELDS = `
      id
      narrativeOrder
      afterSeason
      beforeSeason
      linkedEpisodeId
      continuityStatus
      movieForm
      signalSummary
      monitoringOverride
      metadataActive
      monitored
      movie {
        id
        title
        slug
        year
        overview
        posterUrl
        runtimeMinutes
        contentStatus
        imdbId
        tvdbId
        tmdbId
        malId
        anidbId
        ratings {${TITLE_RATING_SUMMARY_FIELDS}
        }
      }`;

const SERIES_SIDE_PANEL_EPISODE_ROW_FIELDS = `
      id
      titleId
      collectionId
      episodeType
      episodeNumber
      seasonNumber
      episodeLabel
      title
      airDate
      durationSeconds
      isFiller
      isRecap
      absoluteNumber
      monitored
      mediaAvailability {
        state
        primaryQualityLabel
      }
      createdAt`;

const MOVIE_SIDE_PANEL_COLLECTION_FIELDS = `
      id
      titleId
      collectionType
      collectionIndex
      label
      orderedPath
      createdAt`;

const SERIES_SIDE_PANEL_COLLECTION_FIELDS = `
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
      episodesOwned
      episodesMonitored
      episodesTotal
      episodeRecordsTotal
      createdAt`;

const TITLE_MEDIA_FILE_FIELDS = `
      id
      titleId
      episodeId
      seriesMovieLinkIds
      role
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
      libraryId
      libraryName
      librarySlug
      episodeId
      collectionId
      mediaType
      lastSearchAt
      status
      grabbedRelease
      sourceProvider
      currentScore
      convergenceState
      indexersCovered
      indexersRouted
      recencyLane
      mismatchRecoveryEligible
      createdAt
      updatedAt`;

const DOWNLOAD_QUEUE_ITEM_FIELDS = `
    id
    titleId
    episodeId
    titleName
    facet
    isScryerOrigin
    sourceProvider
    clientId
    clientName
    clientType
    state
    displayState
    progressPercent
    importTransferPhase
    importTransferBytes
    importTransferTotalBytes
    importTransferStartedAt
    importTransferUpdatedAt
    sizeBytes
    remainingSeconds
    queuedAt
    lastUpdatedAt
    attentionRequired
    attentionReason
    downloadClientItemId
    downloadId
    importStatus
    importErrorCode
    importErrorMessage
    importedAt
    deleteStatus
    deleteErrorMessage
    trackedState
    trackedStatus
    trackedStatusMessages
    trackedMatchType
    seedingState
    seedRatio
    seedRatioGoal
    seedTimeSeconds
    seedTimeGoalSeconds
    isPrivate
    queueScope {
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
    }`;

const MOVIE_SIDE_PANEL_TITLE_FIELDS = `
    id
    name
    facet
    libraryId
    libraryName
    librarySlug
    monitored
    tags
    playbackLinks {
      connectionId
      displayName
      provider
      href
    }
    externalIds {
      source
      value
    }
    year
    overview
    posterUrl
    posterSourceUrl
    backgroundUrl
    backgroundSourceUrl
    sortTitle
    slug
    imdbId
    runtimeMinutes
    canonicalTags {${TITLE_CANONICAL_TAG_FIELDS}
    }
    contentStatus
    language
    firstAired
    network
    studio
    country
    aliases
    metadataLanguage
    metadataLanguageOverride
    effectiveMetadataLanguage
    inheritsMetadataLanguage
    metadataFetchedAt
    qualityProfileId
    requiredAudioLanguagesOverride
    effectiveRequiredAudioLanguages
    inheritsRequiredAudioLanguages
    rootFolderId
    rootFolderPath
    monitorType
    useSeasonFolders
    useSeasonFoldersOverride
    effectiveUseSeasonFolders
    inheritsUseSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt
    collections {${MOVIE_SIDE_PANEL_COLLECTION_FIELDS}
    }
    wantedItems {
      items {${WANTED_ITEM_FIELDS}
      }
    }
    ratings {${TITLE_RATING_SUMMARY_FIELDS}
    }${TITLE_CAST_CREDIT_SELECTION}
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }`;

const SERIES_SIDE_PANEL_TITLE_FIELDS = `
    id
    name
    facet
    libraryId
    librarySlug
    monitored
    playbackLinks {
      connectionId
      displayName
      provider
      href
    }
    externalIds {
      source
      value
    }
    year
    overview
    posterUrl
    posterSourceUrl
    backgroundUrl
    slug
    canonicalTags {${TITLE_CANONICAL_TAG_FIELDS}
    }
    contentStatus
    metadataLanguage
    metadataLanguageOverride
    effectiveMetadataLanguage
    inheritsMetadataLanguage
    network
    metadataFetchedAt
    qualityProfileId
    qualityTier
    effectiveRequiredAudioLanguages
    inheritsRequiredAudioLanguages
    rootFolderId
    useSeasonFolders
    useSeasonFoldersOverride
    effectiveUseSeasonFolders
    inheritsUseSeasonFolders
    fillerPolicy
    recapPolicy
    effectiveFillerPolicy
    effectiveRecapPolicy
    createdAt
    collections {${SERIES_SIDE_PANEL_COLLECTION_FIELDS}
    }
    seriesMovieLinks {${SERIES_SIDE_PANEL_MOVIE_LINK_FIELDS}
    }
    ratings {${TITLE_RATING_SUMMARY_FIELDS}
    }${TITLE_CAST_CREDIT_SELECTION}`;

export const TITLE_MUTATION_RESULT_FIELDS = `
    id
    name
    facet
    libraryId
    libraryName
    librarySlug
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
    backgroundUrl
    backgroundSourceUrl
    sortTitle
    slug
    imdbId
    runtimeMinutes
    canonicalTags {${TITLE_CANONICAL_TAG_FIELDS}
    }
    contentStatus
    language
    firstAired
    network
    studio
    country
    aliases
    metadataLanguage
    metadataLanguageOverride
    effectiveMetadataLanguage
    inheritsMetadataLanguage
    metadataFetchedAt
    qualityProfileId
    requiredAudioLanguagesOverride
    effectiveRequiredAudioLanguages
    inheritsRequiredAudioLanguages
    rootFolderId
    rootFolderPath
    monitorType
    useSeasonFolders
    useSeasonFoldersOverride
    effectiveUseSeasonFolders
    inheritsUseSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt`;

const TITLE_EVENT_FIELDS = `
    id
    titleId
    episodeId
    collectionId
    eventType
    sourceTitle
    quality
    downloadId
    clientId
    clientName
    failureReason
    blocklistReason
    dataJson
    occurredAt
    createdAt`;

const TITLE_RELEASE_BLOCKLIST_FIELDS = `
    id
    releaseName
    errorMessage
    attemptedAt`;

const EXTERNAL_SUBTITLE_FIELDS = `
    id
    mediaFileId
    titleId
    episodeId
    sourceKind
    language
    provider
    providerFileId
    filePath
    score
    scorePercent
    hearingImpaired
    forced
    aiTranslated
    machineTranslated
    uploader
    releaseInfo
    synced
    downloadedAt`;

const EXTERNAL_SUBTITLE_BLOCKLIST_FIELDS = `
    id
    mediaFileId
    provider
    providerFileId
    language
    reason
    createdAt`;

const IMPORT_HISTORY_FIELDS = `
    id
    sourceSystem
    sourceRef
    sourceTitle
    facet
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
    createdAt`;

const PROVIDER_TYPE_FIELDS = `
    providerType
    name
    defaultBaseUrl
    availableHostBindings
    recommendedFacets
    supportedEvents
    supportsTest
    configFields {
      key
      label
      fieldType
      required
      defaultValue
      valueSource
      role
      hostBinding
      options { value label configOverrides { key value } }
      helpText
    }`;

export const SUBTITLE_SETTINGS_FIELDS = `
    enabled
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
    syncMaxOffsetSeconds`;

export const SUBTITLE_PROVIDER_CONFIG_FIELDS = `
    id
    name
    providerType
    hasConfig
    storedSecretKeys
    enabledFacets
    isEnabled
    lastHealthStatus
    lastError
    lastErrorAt
    disabledUntil
    createdAt
    updatedAt`;

const NOTIFICATION_CHANNEL_FIELDS = `
    id
    name
    channelType
    mediaServerConnectionId
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    createdAt
    updatedAt`;

export const MEDIA_SERVER_CONNECTION_FIELDS = `
    id
    provider
    displayName
    baseUrl
    externalUrl
    enabled
    loginEnabled
    linkingEnabled
    autoAddEnabled
    defaultAppPermissions
    defaultLibraryGrants {
      libraryId
      permissions
    }
    machineIdPresent
    apiKeyPresent
    embyServerIdPresent
    embyConnectEnabled
    pathMappings {
      sourcePath
      destinationPath
    }
    createdAt
    updatedAt`;

const NOTIFICATION_SUBSCRIPTION_FIELDS = `
    id
    channelId
    targetKind
    targetId
    eventType
    scope
    scopeId
    isEnabled
    createdAt
    updatedAt`;

const NOTIFICATION_TARGET_FIELDS = `
    id
    targetKind
    name
    providerType
    mediaServerProvider
    mediaServerConnectionId
    isEnabled`;

export const BACKUP_INFO_FIELDS = `
    filename
    sizeBytes
    createdAt
    formatVersion
    sourceEngine
    sourceMigrationKey
    trigger
    encrypted
    rowCounts {
      table
      rowCount
    }
    status
    errorMessage`;

const DELETE_PREVIEW_FIELDS = `
    fingerprint
    totalFileCount
    mediaCount
    subtitleCount
    imageCount
    otherCount
    directoryCount
    requiresTypedConfirmation
    typedConfirmationPrompt
    targetLabel
    samplePaths`;

export const titleBySlugQuery = `query TitleBySlug($facet: MediaFacetValue!, $librarySlug: String, $slug: String!) {
  titleBySlug(facet: $facet, librarySlug: $librarySlug, slug: $slug) {
    id
    slug
    libraryId
    librarySlug
  }
}`;

export const titleRouteTargetQuery = `query TitleRouteTarget($id: ID!) {
  title(id: $id) {
    id
    facet
    slug
    libraryId
    librarySlug
  }
}`;

export const titleReleaseBlocklistQuery = `query TitleReleaseBlocklist($titleId: ID!, $limit: Int) {
  titleReleaseBlocklist(titleId: $titleId, limit: $limit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
}`;

export const movieSidePanelTitleQuery = `query MovieSidePanelTitle($id: ID!) {
  title(id: $id) {${MOVIE_SIDE_PANEL_TITLE_FIELDS}
  }
}`;

export const movieSidePanelOverviewQuery = `query MovieSidePanelOverview($id: ID!, $blocklistLimit: Int) {
  title(id: $id) {${MOVIE_SIDE_PANEL_TITLE_FIELDS}
  }
  titleAcquisitionDiagnostics(titleId: $id) {
    recentDecisions {
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
    decisionCounts {
      code
      count
    }
    wantedStatusCounts {
      status
      count
    }
    pendingReleaseCounts {
      status
      count
    }
    mismatchRecoveryEligibleCount
    latestDecisionAt
    latestWantedSearchAt
  }
  titleHistory: titleHistory(filter: { titleIds: [$id], limit: 50, offset: 0 }) {
    items {${TITLE_EVENT_FIELDS}
    }
  }
  titleReleaseBlocklist(titleId: $id, limit: $blocklistLimit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
  externalSubtitles(titleId: $id) {${EXTERNAL_SUBTITLE_FIELDS}
  }
  setupStatus {
    hasDownloadClients
  }
}`;

export const seriesSidePanelOverviewQuery = `query SeriesSidePanelOverview($id: ID!, $blocklistLimit: Int) {
  title(id: $id) {${SERIES_SIDE_PANEL_TITLE_FIELDS}
  }
  titleReleaseBlocklist(titleId: $id, limit: $blocklistLimit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
  externalSubtitles(titleId: $id) {${EXTERNAL_SUBTITLE_FIELDS}
  }
  setupStatus {
    hasDownloadClients
  }
}`;

export const seriesCollectionEpisodesQuery = `query SeriesCollectionEpisodes($id: ID!) {
  collectionById(id: $id) {
    id
    episodes {${SERIES_SIDE_PANEL_EPISODE_ROW_FIELDS}
    }
  }
}`;

export const episodeCollectionRefQuery = `query EpisodeCollectionRef($titleId: ID!, $episodeId: ID!) {
  episode(titleId: $titleId, episodeId: $episodeId) {
    id
    collectionId
  }
}`;

export const titleOverviewDownloadFeedbackQuery = `query TitleOverviewDownloadFeedback($id: ID!) {
  downloadQueueItems: downloadQueue(titleId: $id, includeAllActivity: true, includeImportActivity: true, activityFilter: ALL) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
  completedDownloadQueueItems: downloadQueue(titleId: $id, includeAllActivity: true, includeHistoryOnly: true, activityFilter: ALL) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const titleDownloadQueueItemsQuery = `query TitleDownloadQueueItems($id: ID!) {
  title(id: $id) {
    id
    downloadQueueItems {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
  }
}`;

export const titleMediaFilesQuery = `query TitleMediaFiles($id: ID!) {
  title(id: $id) {
    id
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
  }
}`;

export const movieEntityDetailQuery = `query MovieEntityDetail($titleId: ID!, $movieId: ID!) {
  title(id: $titleId) {
    id
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
  }
  movieEntity(titleId: $titleId, id: $movieId) {
    id
    credits {${TITLE_CAST_CREDIT_FIELDS}
    }
  }
}`;

export const episodeSidePanelDetailQuery = `query EpisodeSidePanelDetail($titleId: ID!, $episodeId: ID!) {
  episode(titleId: $titleId, episodeId: $episodeId) {
    id
    overview
    imageUrl
    playbackLinks {
      connectionId
      displayName
      provider
      href
    }
    mediaAvailability {
      state
      primaryQualityLabel
    }
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
  }
}`;

export const deleteTitlePreviewQuery = `query DeleteTitlePreview($titleId: ID!) {
  deleteTitlePreview(titleId: $titleId) {${DELETE_PREVIEW_FIELDS}
  }
}`;

// Folder-match correction: describes what claiming a candidate folder would do.
// Read-only — nothing is changed and no file is ever moved.
export const changeTitleFolderPreviewQuery = `query ChangeTitleFolderPreview($input: ChangeTitleFolderPreviewInput!) {
  changeTitleFolderPreview(input: $input) {
    title {
      id
      name
      folderPath
    }
    facet
    libraryId
    libraryName
    currentRootId
    currentRootPath
    selectedFolderPath
    selectedRootId
    selectedRootPath
    ownership
    currentOwner {
      id
      name
      folderPath
    }
    currentFolderTrackedMediaCount
    selectedFolderTrackedMediaCount
    filesWillMove
    noOp
    availableResolutions
  }
}`;

export const deleteTitlesPreviewQuery = `query DeleteTitlesPreview($input: DeleteTitlesPreviewInput!) {
  deleteTitlesPreview(input: $input) {
    preview {${DELETE_PREVIEW_FIELDS}
    }
    items {
      titleId
      error
      preview {${DELETE_PREVIEW_FIELDS}
      }
    }
    failedCount
  }
}`;

export function buildDeleteTitlePreviewBatchQuery(count: number): string {
  const variables = Array.from(
    { length: count },
    (_, index) => `$titleId${index}: ID!`,
  ).join(", ");
  const fields = Array.from(
    { length: count },
    (_, index) =>
      `item${index}: deleteTitlePreview(titleId: $titleId${index}) {${DELETE_PREVIEW_FIELDS}
  }`,
  ).join("\n");

  return `query DeleteTitlePreviewBatch(${variables}) {
${fields}
}`;
}

export const deleteMediaFilePreviewQuery = `query DeleteMediaFilePreview($fileId: ID!) {
  deleteMediaFilePreview(fileId: $fileId) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const deleteEpisodeFilesPreviewQuery = `query DeleteEpisodeFilesPreview($input: DeleteEpisodeFilesPreviewInput!) {
  deleteEpisodeFilesPreview(input: $input) {
    preview {${DELETE_PREVIEW_FIELDS}
    }
    items {
      fileId
      episodeId
      error
    }
    fileCount
    failedCount
  }
}`;

export const deleteExternalSubtitlePreviewQuery = `query DeleteExternalSubtitlePreview($externalSubtitleId: ID!) {
  deleteExternalSubtitlePreview(externalSubtitleId: $externalSubtitleId) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const externalSubtitleBlocklistEntriesQuery = `query ExternalSubtitleBlocklistEntries($mediaFileId: ID!) {
  externalSubtitleBlocklistEntries(mediaFileId: $mediaFileId) {${EXTERNAL_SUBTITLE_BLOCKLIST_FIELDS}
  }
}`;

export const RELEASE_SEARCH_RESULT_FIELDS = `
    source
    indexerId
    title
    link
    downloadUrl
    candidateToken
    queueScope {
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
    grabs
    infoHash
    freeleech
    downloadVolumeFactor
    autoEligible
    autoDecisionCode
    autoDecisionSummary`;

export const searchForTitleQuery = `query SearchReleasesForTitle($titleId: ID!) {
  searchReleases(input: { titleId: $titleId }) {${RELEASE_SEARCH_RESULT_FIELDS}
  }
}`;

export const searchForEpisodeQuery = `query SearchReleasesForEpisode($titleId: ID!, $season: String!, $episode: String!) {
  searchReleases(input: {
    titleId: $titleId,
    season: $season,
    episode: $episode
  }) {${RELEASE_SEARCH_RESULT_FIELDS}
  }
}`;

export const searchForSeriesMovieQuery = `query SearchReleasesForSeriesMovie($titleId: ID!, $seriesMovieLinkId: ID!) {
  searchReleases(input: {
    titleId: $titleId,
    seriesMovieLinkId: $seriesMovieLinkId
  }) {${RELEASE_SEARCH_RESULT_FIELDS}
  }
}`;

// Hotfix 0.17.1: server-side interactive release-search job; results stream in
// as each indexer completes and are polled via this query.
export const interactiveReleaseSearchQuery = `query InteractiveReleaseSearch($id: ID!) {
  interactiveReleaseSearch(id: $id) {
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

export const TITLE_LIST_FIELDS = `
    id
    name
    facet
    libraryId
    libraryName
    librarySlug
    monitored
    tags
    slug
    year
    posterUrl
    posterSourceUrl
    backgroundUrl
    backgroundSourceUrl
    qualityTier
    currentQualityTier
    sizeBytes
    episodesOwned
    episodesMonitored
    episodesTotal
    contentStatus
    metadataLanguage
    metadataLanguageOverride
    effectiveMetadataLanguage
    inheritsMetadataLanguage
    metadataFetchedAt
    createdAt`;

export type TitleCatalogTitleProjection = {
  library?: boolean;
  quality?: boolean;
  size?: boolean;
  episodes?: boolean;
  runtime?: boolean;
  root?: boolean;
  ratings?: boolean;
  movieMedia?: boolean;
  popularity?: boolean;
};

export type TitleCatalogQueryBuildOptions = {
  includePageMetadata?: boolean;
};

const TITLE_CATALOG_BASE_FIELDS = `
    id
    name
    facet
    libraryId
    monitored
    tags
    slug
    year
    posterUrl
    posterSourceUrl
    backgroundUrl
    backgroundSourceUrl
    contentStatus
    metadataLanguage
    metadataLanguageOverride
    effectiveMetadataLanguage
    inheritsMetadataLanguage
    metadataFetchedAt
    qualityProfileId
    rootFolderId
    monitorType
    useSeasonFolders
    useSeasonFoldersOverride
    effectiveUseSeasonFolders
    inheritsUseSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt`;

function titleCatalogListFields(
  projection: TitleCatalogTitleProjection = {},
) {
  const fields = [TITLE_CATALOG_BASE_FIELDS];
  if (projection.library) {
    fields.push(`
    libraryName
    librarySlug`);
  }
  if (projection.quality) {
    fields.push(`
    qualityTier
    currentQualityTier`);
  }
  if (projection.size) {
    fields.push(`
    sizeBytes`);
  }
  if (projection.episodes) {
    fields.push(`
    episodesOwned
    episodesMonitored
    episodesTotal`);
  }
  if (projection.runtime) {
    fields.push(`
    runtimeMinutes`);
  }
  if (projection.root) {
    fields.push(`
    rootFolderPath`);
  }
  if (projection.popularity) {
    fields.push(`
    popularity`);
  }
  if (projection.movieMedia) {
    fields.push(`
    mediaResolution
    mediaHdr
    mediaAudioCodec`);
  }
  if (projection.ratings) {
    fields.push(`
    ratings {
      rating
      ratingSources
      externalRatings {
        source
        value
        score
        normalized
        votes
        url
      }
    }`);
  }
  return fields.join("");
}

export const TITLE_COMMAND_PALETTE_FIELDS = `
    id
    name
    facet
    libraryId
    librarySlug
    slug
    sortTitle
    year
    posterUrl
    posterSourceUrl
    metadataFetchedAt
    createdAt`;

export const TITLE_CATALOG_SEARCH_FIELDS = `
    id
    name
    facet
    libraryId
    libraryName
    librarySlug
    monitored
    tags
    slug
    sortTitle
    year
    posterUrl
    posterSourceUrl
    backgroundUrl
    backgroundSourceUrl
    rootFolderId
    rootFolderPath
    qualityTier
    sizeBytes
    episodesOwned
    episodesMonitored
    episodesTotal
    contentStatus
    metadataFetchedAt
    createdAt
    externalIds {
      source
      value
    }`;

export const TITLE_LIST_FIELDS_WITH_EXTERNAL_IDS = `${TITLE_LIST_FIELDS}
    imdbId
    externalIds {
      source
      value
    }`;

export const librariesQuery = `query Libraries($facet: MediaFacetValue, $permission: LibraryPermissionValue) {
  libraries(facet: $facet, permission: $permission) {
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
    }
  }
}`;

export const mediaRequestAdminLibrariesQuery = `query MediaRequestAdminLibraries($facet: MediaFacetValue) {
  libraries(facet: $facet, permission: MANAGE_TITLES) {
    id
    facet
    name
    slug
    isDefault
    qualityProfileId
    requestQualityProfileIds
    requestQualityProfileDefaultId
    roots {
      id
      path
      isDefault
    }
  }
}`;

export const mediaRequestRequesterLibrariesQuery = `query MediaRequestRequesterLibraries($facet: MediaFacetValue) {
  libraries(facet: $facet, permission: REQUEST) {
    id
    facet
    name
    slug
    isDefault
    requestQualityProfileIds
    requestQualityProfileDefaultId
    roots {
      id
      path
      isDefault
    }
  }
}`;

export const librarySettingsQuery = `query LibrarySettings($libraryId: ID!) {
  librarySettings(libraryId: $libraryId) {
    requiredAudioLanguagesOverride
    requiredAudioLanguages
    metadataLanguageOverride
    metadataLanguage
    useSeasonFoldersOverride
    useSeasonFolders
    qualityProfileIdOverride
    qualityProfileId
    requestQualityProfileIdsOverride
    requestQualityProfileIds
    requestQualityProfileDefaultId
    scoringPersonaOverride
    scoringPersona
    fillerPolicyOverride
    fillerPolicy
    recapPolicyOverride
    recapPolicy
    monitorSpecialsOverride
    monitorSpecials
    interSeasonMoviesOverride
    interSeasonMovies
    monitorFillerMoviesOverride
    monitorFillerMovies
    nfoWriteOnImportOverride
    nfoWriteOnImport
    plexmatchWriteOnImportOverride
    plexmatchWriteOnImport
    importModeOverride
    importMode
    setPermissionsLinuxOverride
    setPermissionsLinux
    fileChmodOverride
    fileChmod
    folderChmodOverride
    folderChmod
    chownGroupOverride
    chownGroup
    indexerRoutingOverride {
      indexerId
      enabled
      categories
      priority
    }
    downloadClientRoutingOverride {
      clientId
      enabled
      category
      recentQueuePriority
      olderQueuePriority
      removeCompleted
      removeFailed
      seedingProfileId
    }
  }
}`;

export const titleCatalogFilterOptionsQuery = `query TitleCatalogFilterOptions(
  $facet: MediaFacetValue,
  $libraryIds: [ID!],
  $rootFolderIds: [ID!]
) {
  titleCatalogFilterOptions(
    facet: $facet,
    libraryIds: $libraryIds,
    rootFolderIds: $rootFolderIds
  ) {
    genres { key name }
    themes { key name }
    minimumYear
    maximumYear
  }
}`;

export function buildTitlesQuery(
  projection: TitleCatalogTitleProjection = {},
  options: TitleCatalogQueryBuildOptions = {},
) {
  const includePageMetadata = options.includePageMetadata ?? true;
  const pageMetadataFields = includePageMetadata
    ? `
    hasMore
    totalCount
    managedBytes
    filterCounts {
      all
      monitored
      unmonitored
      continuing
      ended
    }`
    : "";
  return `query Titles(
  $facet: MediaFacetValue,
  $libraryIds: [ID!],
  $query: String,
  $filter: TitleCatalogFilterInput,
  $sort: TitleCatalogSortInput,
  $limit: Int,
  $offset: Int
) {
  titles(
    facet: $facet,
    libraryIds: $libraryIds,
    query: $query,
    filter: $filter,
    sort: $sort,
    limit: $limit,
    offset: $offset
  ) {
    items {
${titleCatalogListFields(projection)}
    }
${pageMetadataFields}
  }
}`;
}

export const titlesQuery = buildTitlesQuery();

export const catalogSearchTitlesQuery = `query CatalogSearchTitles($facet: MediaFacetValue, $libraryIds: [ID!], $query: String, $limit: Int = 25) {
  titles(facet: $facet, libraryIds: $libraryIds, query: $query, limit: $limit) {
    items {
${TITLE_CATALOG_SEARCH_FIELDS}
    }
  }
}`;

export const commandPaletteTitlesQuery = `query CommandPaletteTitles($facet: MediaFacetValue, $libraryIds: [ID!], $query: String, $limit: Int = 25) {
  titles(facet: $facet, libraryIds: $libraryIds, query: $query, limit: $limit) {
    items {
${TITLE_COMMAND_PALETTE_FIELDS}
    }
  }
}`;

export const titlesByExternalIdsQuery = `query TitlesByExternalIds($source: String!, $values: [String!]!) {
  titlesByExternalIds(source: $source, values: $values) {
${TITLE_LIST_FIELDS_WITH_EXTERNAL_IDS}
  }
}`;

export const titleAutocompleteSelectionQuery = `query TitleAutocompleteSelection($id: ID!) {
  title(id: $id) {
${TITLE_LIST_FIELDS}
  }
}`;

export const titleMoreLikeThisQuery = `query TitleMoreLikeThis($id: ID!, $limit: Int = 12) {
  title(id: $id) {
    id
    moreLikeThis(limit: $limit) {
${DISCOVERY_ITEM_DETAIL_FIELDS}
    }
  }
}`;

type ReactiveRefreshVariableValue = string | number | null;

export type TitleSidePanelOverviewProjection = "MOVIE" | "SERIES";

export type ReactiveRefreshQueryActionInput =
  | {
      key: string;
      kind: "catalogTitles";
      facet?: string | null;
    }
  | {
      key: string;
      kind: "catalogTitle";
      titleId: string;
      projection?: TitleCatalogTitleProjection;
    }
  | {
      key: string;
      kind: "titleSidePanelOverview";
      titleId: string;
      blocklistLimit: number;
      projection: TitleSidePanelOverviewProjection;
    }
  | {
      key: string;
      kind: "titleOverviewDownloadFeedback";
      titleId: string;
    }
  | {
      key: string;
      kind: "importHistory";
      limit?: number | null;
    };

export type ReactiveRefreshQueryActionPlan =
  | {
      key: string;
      kind: "catalogTitles";
      titlesAlias: string;
    }
  | {
      key: string;
      kind: "catalogTitle";
      titleAlias: string;
    }
  | {
      key: string;
      kind: "titleSidePanelOverview";
      titleAlias: string;
      titleHistoryAlias?: string;
      titleReleaseBlocklistAlias: string;
      externalSubtitlesAlias: string;
      setupStatusAlias: string;
    }
  | {
      key: string;
      kind: "titleOverviewDownloadFeedback";
      downloadQueueItemsAlias: string;
      completedDownloadQueueItemsAlias: string;
    }
  | {
      key: string;
      kind: "importHistory";
      importHistoryAlias: string;
    };

export function buildReactiveRefreshQuery(
  actions: ReactiveRefreshQueryActionInput[],
) {
  const variableDefinitions: string[] = [];
  const fields: string[] = [];
  const variables: Record<string, ReactiveRefreshVariableValue> = {};
  const actionPlans: ReactiveRefreshQueryActionPlan[] = [];

  actions.forEach((action, index) => {
    switch (action.kind) {
      case "catalogTitles": {
        const titlesAlias = `catalogTitlesAction${index}`;
        const facetVariableName = `catalogTitlesFacet${index}`;
        variableDefinitions.push(`$${facetVariableName}: MediaFacetValue`);
        fields.push(
          `  ${titlesAlias}: titles(facet: $${facetVariableName}) {\n    items {\n${TITLE_LIST_FIELDS}\n    }\n  }`,
        );
        variables[facetVariableName] = action.facet ?? null;
        actionPlans.push({ key: action.key, kind: action.kind, titlesAlias });
        break;
      }
      case "catalogTitle": {
        const titleAlias = `catalogTitleAction${index}`;
        const titleIdVariableName = `catalogTitleId${index}`;
        variableDefinitions.push(`$${titleIdVariableName}: ID!`);
        const titleFields = titleCatalogListFields(action.projection);
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${titleFields}\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        actionPlans.push({ key: action.key, kind: action.kind, titleAlias });
        break;
      }
      case "titleSidePanelOverview": {
        const titleIdVariableName = `titleSidePanelOverviewId${index}`;
        const blocklistLimitVariableName = `titleSidePanelOverviewBlocklistLimit${index}`;
        const titleAlias = `titleSidePanelOverviewTitleAction${index}`;
        const titleHistoryAlias =
          action.projection === "MOVIE"
            ? `titleSidePanelOverviewHistoryAction${index}`
            : undefined;
        const titleReleaseBlocklistAlias = `titleSidePanelOverviewBlocklistAction${index}`;
        const externalSubtitlesAlias = `titleSidePanelOverviewExternalSubtitlesAction${index}`;
        const setupStatusAlias = `titleSidePanelOverviewSetupStatusAction${index}`;

        variableDefinitions.push(`$${titleIdVariableName}: ID!`);
        variableDefinitions.push(`$${blocklistLimitVariableName}: Int`);
        const titleFields =
          action.projection === "SERIES"
            ? SERIES_SIDE_PANEL_TITLE_FIELDS
            : MOVIE_SIDE_PANEL_TITLE_FIELDS;
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${titleFields}\n  }`,
        );
        if (titleHistoryAlias) {
          fields.push(
            `  ${titleHistoryAlias}: titleHistory(filter: { titleIds: [$${titleIdVariableName}], limit: 50, offset: 0 }) {\n    items {\n${TITLE_EVENT_FIELDS}\n    }\n  }`,
          );
        }
        fields.push(
          `  ${titleReleaseBlocklistAlias}: titleReleaseBlocklist(titleId: $${titleIdVariableName}, limit: $${blocklistLimitVariableName}) {\n${TITLE_RELEASE_BLOCKLIST_FIELDS}\n  }`,
        );
        fields.push(
          `  ${externalSubtitlesAlias}: externalSubtitles(titleId: $${titleIdVariableName}) {\n${EXTERNAL_SUBTITLE_FIELDS}\n  }`,
        );
        fields.push(
          `  ${setupStatusAlias}: setupStatus {\n    hasDownloadClients\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        variables[blocklistLimitVariableName] = action.blocklistLimit;
        actionPlans.push({
          key: action.key,
          kind: action.kind,
          titleAlias,
          titleHistoryAlias,
          titleReleaseBlocklistAlias,
          externalSubtitlesAlias,
          setupStatusAlias,
        });
        break;
      }
      case "titleOverviewDownloadFeedback": {
        const titleIdVariableName = `titleOverviewDownloadFeedbackId${index}`;
        const downloadQueueItemsAlias = `titleOverviewDownloadQueueAction${index}`;
        const completedDownloadQueueItemsAlias = `titleOverviewCompletedDownloadQueueAction${index}`;

        variableDefinitions.push(`$${titleIdVariableName}: ID!`);
        fields.push(
          `  ${downloadQueueItemsAlias}: downloadQueue(titleId: $${titleIdVariableName}, includeAllActivity: true, includeImportActivity: true, activityFilter: ALL) {\n${DOWNLOAD_QUEUE_ITEM_FIELDS}\n  }`,
        );
        fields.push(
          `  ${completedDownloadQueueItemsAlias}: downloadQueue(titleId: $${titleIdVariableName}, includeAllActivity: true, includeHistoryOnly: true, activityFilter: ALL) {\n${DOWNLOAD_QUEUE_ITEM_FIELDS}\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        actionPlans.push({
          key: action.key,
          kind: action.kind,
          downloadQueueItemsAlias,
          completedDownloadQueueItemsAlias,
        });
        break;
      }
      case "importHistory": {
        const importHistoryAlias = `importHistoryAction${index}`;
        const limitVariableName = `importHistoryLimit${index}`;
        variableDefinitions.push(`$${limitVariableName}: Int`);
        fields.push(
          `  ${importHistoryAlias}: importHistory(limit: $${limitVariableName}) {\n${IMPORT_HISTORY_FIELDS}\n  }`,
        );
        variables[limitVariableName] = action.limit ?? null;
        actionPlans.push({
          key: action.key,
          kind: action.kind,
          importHistoryAlias,
        });
        break;
      }
      default: {
        const exhaustiveCheck: never = action;
        throw new Error(
          `unsupported reactive refresh action: ${exhaustiveCheck}`,
        );
      }
    }
  });

  if (fields.length === 0) {
    throw new Error("reactive refresh query requires at least one action");
  }

  const signature = variableDefinitions.length
    ? `(${variableDefinitions.join(", ")})`
    : "";

  return {
    query: `query ReactiveRefresh${signature} {\n${fields.join("\n")}\n}`,
    variables,
    actionPlans,
  };
}

export const mediaRenamePreviewBulkQuery = `query MediaRenamePreviewBulk($input: MediaRenamePreviewBulkInput!) {
  mediaRenamePreviewBulk(input: $input) {
    titleId
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
      writeAction
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
      seriesMovieLinkIds
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
    actorKind
    actorUserId
    actorDisplayName
    titleId
    episodeIds
    occurredAt
  }
}`;

export const activitySubscriptionQuery = `subscription ActivityStream {
  activityEvents {
    id
    kind
    severity
    channels
    actorKind
    actorUserId
    actorDisplayName
    titleId
    facet
    message
    occurredAt
  }
}`;

export const auditLogQuery = `query AuditLog($eventTypes: [DomainEventTypeValue!], $titleId: ID, $facet: MediaFacetValue, $beforeSequence: Long, $afterSequence: Long, $limit: Int) {
  auditLog(
    eventTypes: $eventTypes
    titleId: $titleId
    facet: $facet
    beforeSequence: $beforeSequence
    afterSequence: $afterSequence
    limit: $limit
  ) {
    sequence
    eventId
    occurredAt
    actorKind
    actorUserId
    actorDisplayName
    titleId
    facet
    eventType
    streamKind
    streamId
    payloadJson
  }
}`;

export const LIBRARY_SCAN_PROGRESS_FIELDS = `
  sessionId
  facet
  libraryId
  mode
  status
  startedAt
  updatedAt
  foundTitles
  titleMatchTotalKnown
  titleMatchProgress {
    total
    completed
    failed
  }
  hydrationTotalKnown
  hydrationProgress {
    total
    completed
    failed
  }
  mediaAnalysisTotalKnown
  mediaAnalysisProgress {
    total
    completed
    failed
  }
  summary {
    scanned
    matched
    imported
    skipped
    unmatched
  }
`;

export const activeLibraryScansQuery = `query ActiveLibraryScans {
  activeLibraryScans {
${LIBRARY_SCAN_PROGRESS_FIELDS}
  }
}`;

export const libraryScanStateSubscriptionQuery = `subscription LibraryScanState {
  libraryScanState {
${LIBRARY_SCAN_PROGRESS_FIELDS}
  }
}`;

export const jobsQuery = `query Jobs {
  jobs {
    key
    displayName
    description
    category
    section
    manualTriggerAllowed
    usesLibraryScanProgress
    schedule {
      kind
      description
      intervalSeconds
      initialDelaySeconds
      nextRunAt
    }
  }
}`;

export const JOB_RUN_FIELDS = `
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
  libraryScanProgress {
${LIBRARY_SCAN_PROGRESS_FIELDS}
  }
`;

export const activeJobRunsQuery = `query ActiveJobRuns {
  activeJobRuns {
${JOB_RUN_FIELDS}
  }
}`;

export const jobRunsQuery = `query JobRuns($jobKey: JobKeyValue!, $limit: Int) {
  jobRuns(jobKey: $jobKey, limit: $limit) {
${JOB_RUN_FIELDS}
  }
}`;

export const recentJobRunsQuery = `query RecentJobRuns($limit: Int) {
  recentJobRuns(limit: $limit) {
${JOB_RUN_FIELDS}
  }
}`;

export const jobRunEventsSubscription = `subscription JobRunEvents {
  jobRunEvents {
${JOB_RUN_FIELDS}
  }
}`;

export const acquisitionSearchJobQuery = `query AcquisitionSearchJob($id: ID!) {
  acquisitionSearchJob(id: $id) {
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

export const usersQuery = `query Users {
  users {
    id
    username
    loginEnabled
    isDefaultAdmin
    hasPassword
    passwordChangeRequired
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

export const indexersQuery = `query Indexers($providerType: String) {
  indexers(providerType: $providerType) {
    id
    name
    providerType
    baseUrl
    proxyConfigId
    downloadClientId
    seedingProfileId
    hasProwlarrSeedCriteria
    prowlarrMinimumSeeders
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
    lastQueryAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    createdAt
    updatedAt
  }
}`;

export const indexerProviderTypesQuery = `query IndexerProviderTypes {
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

const proxyConfigFieldSelection = `
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

export const proxyConfigsQuery = `query ProxyConfigs {
  proxyConfigs {${proxyConfigFieldSelection}
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
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    storedSecretKeys
    isEnabled
    status
    lastError
    lastSeenAt
    createdAt
    updatedAt
  }
}`;

export const mediaServerConnectionsQuery = `query MediaServerConnections($provider: MediaServerProviderValue) {
  mediaServerConnections(provider: $provider) {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
  runtimeInfo {
    runtimePathStyle
  }
}`;

export const jellyfinServerUsersQuery = `query JellyfinServerUsers($connectionId: ID!, $search: String) {
  jellyfinServerUsers(connectionId: $connectionId, search: $search) {
    id
    username
    displayName
    avatarUrl
  }
}`;

export const mediaServerUsersQuery = `query MediaServerUsers($search: String) {
  mediaServerUsers(search: $search) {
    connectionId
    connectionName
    provider
    status
    errorMessage
    users {
      id
      username
      displayName
      avatarUrl
    }
  }
}`;

export const downloadQueueQuery = `query DownloadQueue($includeAllActivity: Boolean, $includeHistoryOnly: Boolean, $includeImportActivity: Boolean, $titleId: ID, $activityFilter: DownloadActivityFilterValue) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly, includeImportActivity: $includeImportActivity, titleId: $titleId, activityFilter: $activityFilter) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const downloadQueuePageQuery = `query DownloadQueuePage($limit: Int = 50, $offset: Int = 0, $filters: [DownloadActivityFilterValue!], $clientIds: [ID!], $scryerSubmittedOnly: Boolean = true, $titleId: ID, $sortKey: DownloadQueueSortKeyValue = STATUS, $sortDirection: SortDirectionValue = ASC) {
  downloadQueuePage(limit: $limit, offset: $offset, filters: $filters, clientIds: $clientIds, scryerSubmittedOnly: $scryerSubmittedOnly, titleId: $titleId, sortKey: $sortKey, sortDirection: $sortDirection) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
    availableClients {
      clientId
      clientName
      clientType
    }
    revision
    updatedAt
    ready
    stale
  }
}`;

export const downloadImportQuery = `query DownloadImport($limit: Int, $offset: Int, $filter: DownloadImportFilterValue) {
  downloadImport(limit: $limit, offset: $offset, filter: $filter) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
  }
}`;

export const downloadQueueSubscription = `subscription DownloadQueueStream($includeAllActivity: Boolean, $includeHistoryOnly: Boolean, $includeImportActivity: Boolean, $titleId: ID, $activityFilter: DownloadActivityFilterValue) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly, includeImportActivity: $includeImportActivity, titleId: $titleId, activityFilter: $activityFilter) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const downloadQueueSyncSubscription = `subscription DownloadQueueSync {
  downloadQueueSync {
    revision
    updatedAt
  }
}`;

export const activeImportStreamsQuery = `query ActiveImportStreams {
  activeImportStreams {
    id
    importId
    libraryId
    facet
    sourcePath
    destinationPath
    phase
    bytes
    totalBytes
    queuedAt
    startedAt
    updatedAt
    cancellable
    cancellationRequested
  }
}`;

export const activeImportStreamsSyncSubscription = `subscription ActiveImportStreamsSync {
  activeImportStreamsSync {
    revision
    updatedAt
  }
}`;

export const importQueueCountQuery = `query ImportQueueCount {
  downloadImport(limit: 1, offset: 0, filter: ATTENTION) {
    totalCount
  }
}`;

const downloadClientFieldSelection = `
    id
    name
    clientType
    baseUrl
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
    proxyConfigId
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
    proxyConfigId
    downloadClientId
    seedingProfileId
    hasProwlarrSeedCriteria
    prowlarrMinimumSeeders
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
    lastQueryAt
    config {${PROVIDER_CONFIG_VALUE_FIELDS}
    }
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

// Batched query for quality profiles page: 5 requests → 1
export const qualityProfilesInitQuery = `query QualityProfilesInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
}`;

export const qualityProfileOptionsQuery = `query QualityProfileOptions {
  qualityProfileSettings {
    profiles {
      id
      name
    }
  }
}`;

export const movieOverviewSettingsInitQuery = `query MovieOverviewSettingsInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: MOVIE) {${mediaSettingsFieldSelection}
  }
}`;

export const seriesOverviewSettingsInitQuery = `query SeriesOverviewSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const cutoffUnmetTitlesPageQuery = `query CutoffUnmetTitlesPage($facet: MediaFacetValue, $libraryIds: [ID!], $limit: Int!, $offset: Int!) {
  cutoffUnmetTitlesPage(facet: $facet, libraryIds: $libraryIds, limit: $limit, offset: $offset) {
    items {
      titleId
      titleName
      titleSlug
      titleFacet
      libraryId
      libraryName
      librarySlug
      episodeId
      seasonNumber
      episodeNumber
      currentTier
      targetTier
      convergenceState
      indexersCovered
      indexersRouted
    }
    totalCount
    hasMore
  }
}`;

export const downloadClientsInitQuery = `query DownloadClientsInit {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  proxyConfigs {${proxyConfigFieldSelection}
  }
  runtimeInfo {
    runtimePathStyle
  }
}`;

export const libraryDownloadClientsQuery = `query LibraryDownloadClients {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
}`;

/**
 * The settings page's refresh. It reads the same full selection the init query
 * does, so a save does not blank the columns the list renders (base URL,
 * status, the assigned proxy); the slim `DownloadClients` document stays for
 * callers that only need a name for an id.
 */
export const settingsDownloadClientsQuery = `query SettingsDownloadClients {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
}`;

export const indexersInitQuery = `query IndexersInit($providerType: String) {
  indexers(providerType: $providerType) {${indexerFieldSelection}
  }
  proxyConfigs {${proxyConfigFieldSelection}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const setupWizardProviderTypesInitQuery = `query SetupWizardProviderTypesInit {
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  indexerProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  runtimeInfo {
    runtimePathStyle
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
  runtimeInfo {
    runtimePathStyle
  }
}`;

export const globalSearchInitQuery = `query GlobalSearchInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  manageableLibraries: libraries(permission: MANAGE_TITLES) {
    id
    facet
    name
    slug
    isDefault
    qualityProfileId
    roots {
      id
      path
      isDefault
    }
  }
  requestableLibraries: libraries(permission: REQUEST) {
    id
    facet
    name
    slug
    isDefault
    requestQualityProfileIds
    requestQualityProfileDefaultId
    roots {
      id
      path
      isDefault
    }
  }
  movieSettings: mediaSettings(scope: MOVIE) {${mediaSettingsFieldSelection}
  }
  seriesSettings: mediaSettings(scope: SERIES) {${mediaSettingsFieldSelection}
  }
  animeSettings: mediaSettings(scope: ANIME) {${mediaSettingsFieldSelection}
  }
}`;

export const globalSearchRequesterInitQuery = `query GlobalSearchRequesterInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  requestableLibraries: libraries(permission: REQUEST) {
    id
    facet
    name
    slug
    isDefault
    requestQualityProfileIds
    requestQualityProfileDefaultId
    roots {
      id
      path
      isDefault
    }
  }
}`;

export const requestableLibrariesQuery = `query RequestableLibraries {
  requestableLibraries: libraries(permission: REQUEST) {
    id
    facet
    name
    slug
    isDefault
    requestQualityProfileIds
    requestQualityProfileDefaultId
    roots {
      id
      path
      isDefault
    }
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
    longTailBackfillMaxScopesPerCycle
    longTailReconvergeDays
  }
}`;

export const generalSettingsQuery = `query GeneralSettings {
  generalSettings {
    experimentalFeaturesEnabled
    personalizedDiscoveryEnabled
    srrdbFilenameRecoveryEnabled
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

export const myUiSettingsQuery = `query MyUiSettings {
  myUiSettings {
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

export const securitySettingsQuery = `query SecuritySettings {
  securitySettings {
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

export const oauthClientRegistrationsQuery = `query OAuthClientRegistrations {
  oauthClientRegistrations {
    clientId
    displayName
    redirectUris
    enabled
    source
    kind
  }
}`;

export const externalAuthRuntimeSettingsQuery = `query ExternalAuthRuntimeSettings {
  externalAuthRuntimeSettings {
    loginProviders
    linkingProviders
    connections {
      id
      provider
      displayName
      loginEnabled
      linkingEnabled
      embyConnectEnabled
    }
  }
}`;

export const linkedAccountsQuery = `query LinkedAccounts($userId: ID) {
  linkedAccounts(userId: $userId) {
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
    updatedAt
  }
}`;

export const externalAccountInvitesQuery = `query ExternalAccountInvites {
  externalAccountInvites {
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
    updatedAt
  }
}`;

export const autoBackupSettingsQuery = `query AutoBackupSettings {
  autoBackupSettings {
    enabled
    dailyTimeLocal
    autoBackupKeyPresent
    autoBackupDisabledMissingKeyNotice
    nextRunAt
  }
}`;

export const backupSettingsQuery = `query BackupSettings {
  backupSettings {
    customBackupPath
    defaultBackupPath
    effectiveBackupPath
  }
}`;

export const delayProfilesQuery = `query DelayProfiles {
  delayProfiles {
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

export const SEEDING_PROFILE_FIELDS = `
    id
    name
    ratio
    seedTimeMinutes
    seasonPackMode
    seasonPackRatio
    seasonPackSeedTimeMinutes
    honorTrackerMinimums
    goalMetAction
    neverRemove
    minimumSeeders
    postImportTracking`;

export const seedingProfilesQuery = `query SeedingProfiles {
  seedingProfiles {${SEEDING_PROFILE_FIELDS}
  }
}`;

export const defaultSeedingProfileQuery = `query DefaultSeedingProfile {
  defaultSeedingProfile {
    seedingProfileId
    minimumSeedersFloor
  }
}`;

export const libraryPathsQuery = `query LibraryPaths {
  libraryPaths {${libraryPathsFieldSelection}
  }
}`;

export const subtitleSettingsQuery = `query SubtitleSettings {
  subtitleSettings {${SUBTITLE_SETTINGS_FIELDS}
  }
}`;

export const subtitleProviderTypesQuery = `query SubtitleProviderTypes {
  subtitleProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
}`;

export const subtitleProviderConfigsQuery = `query SubtitleProviderConfigs($providerType: String) {
  subtitleProviderConfigs(providerType: $providerType) {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

export const subtitleSettingsInitQuery = `query SubtitleSettingsInit {
  subtitleSettings {${SUBTITLE_SETTINGS_FIELDS}
  }
  subtitleProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  subtitleProviderConfigs {${SUBTITLE_PROVIDER_CONFIG_FIELDS}
  }
}`;

// Batched query for download client routing: 2 requests → 1
export const downloadClientRoutingInitQuery = `query DownloadClientRoutingInit($scopeId: ContentScopeValue!) {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientRouting(scope: $scopeId) {${downloadClientRoutingFieldSelection}
  }
}`;

export const downloadClientRoutingQuery = `query DownloadClientRouting($scopeId: ContentScopeValue!) {
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

export const authRuntimeStateQuery = `query AuthRuntimeState {
  authRuntimeState {
    effectiveFormLoginEnabled
    skipLoginForLocalIps
    passkeyEnabled
    defaultPersistSession
    envOverrideActive
    mfaRequirePasswordLogin
    mfaRequireConfigStepUp
    mfaRequireJellyfinLogin
  }
}`;

export const oauthAuthorizationClientQuery = `query OAuthAuthorizationClient($clientId: String!, $redirectUri: String!, $scope: String) {
  oauthAuthorizationClient(clientId: $clientId, redirectUri: $redirectUri, scope: $scope) {
    clientId
    displayName
    scope
  }
}`;

export const myPasskeysQuery = `query MyPasskeys {
  myPasskeys {
    id
    friendlyName
    createdAt
    lastUsedAt
  }
}`;

export const myOauthAppsQuery = `query MyOauthApps {
  myOauthApps {
    grantId
    clientId
    clientName
    authorizedAt
    lastUsedAt
  }
}`;

export const myApiKeysQuery = `query MyApiKeys {
  canCreateMyApiKeys
  myApiKeys {
    id label actor expiresAt revokedAt lastUsedAt createdAt provisioningSource
  }
}`;

export const myTotpQuery = `query MyTotp {
  myTotp {
    enabled
    createdAt
    lastUsedAt
    recoveryCodesRemaining
  }
}`;

export const importHistoryQuery = `query ImportHistory($limit: Int) {
  importHistory(limit: $limit) {${IMPORT_HISTORY_FIELDS}
  }
}`;

export const importHistoryChangedSubscription = `subscription ImportHistoryChanged {
  importHistoryChanged
}`;

export const mediaRequestsChangedSubscription = `subscription MediaRequestsChanged {
  mediaRequestsChanged {
    eventId
    eventType
    requestId
    libraryId
  }
}`;

export const indexersChangedSubscription = `subscription IndexersChanged {
  indexersChanged
}`;

export const providerCatalogChangedSubscription = `subscription ProviderCatalogChanged {
  providerCatalogChanged
}`;

export const pluginInstallProgressSubscription = `subscription PluginInstallProgress($pluginId: ID!) {
  pluginInstallProgress(pluginId: $pluginId) {
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

export const externalImportArrSourceWarmupStatusQuery = `query ExternalImportArrSourceWarmupStatus($sessionId: ID!) {
  externalImportArrSourceWarmupStatus(sessionId: $sessionId) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const externalImportWarmupStatusQuery = `query ExternalImportWarmupStatus($sessionId: ID!) {
  externalImportWarmupStatus(sessionId: $sessionId) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

// Aggregated title-fetch progress across every per-instance warmup session,
// used to gate the Summary step.
export const externalImportAggregateWarmupProgressQuery = `query ExternalImportAggregateWarmupProgress($input: ExternalImportAggregateWarmupProgressInput!) {
  externalImportAggregateWarmupProgress(input: $input) {
    status
    titlesTotalKnown
    titlesFetched
    titlesTotal
    errorMessage
  }
}`;

export const externalImportMonitorWarmupProgressSubscription = `subscription ExternalImportMonitorWarmupProgress($sessionId: ID!) {
  externalImportMonitorWarmupProgress(sessionId: $sessionId) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

// Minimal quality-profile list for the import wizard's per-library Quality step.
export const wizardQualityProfilesQuery = `query WizardQualityProfiles {
  qualityProfileSettings {
    globalProfileId
    globalScoringPersona
    profiles { id name }
  }
}`;

// Sensitive import-wizard draft (API keys / passwords) is stored server-side as
// a single owner-scoped, encrypted draft; the rest of the wizard stays in
// sessionStorage. Status never exposes secrets and is readable by anyone.
export const externalImportSetupSecretDraftStatusQuery = `query ExternalImportSetupSecretDraftStatus {
  externalImportSetupSecretDraftStatus {
    hasDraft
    ownedByCurrentUser
    updatedAt
  }
}`;

// Returns null when there is no draft or another user owns it.
export const externalImportSetupSecretDraftQuery = `query ExternalImportSetupSecretDraft {
  externalImportSetupSecretDraft {
    updatedAt
    instanceApiKeys { instanceId kind apiKey }
    downloadClientApiKeyOverrides { dedupKey apiKey }
    downloadClientPasswordOverrides { dedupKey password }
    indexerApiKeyOverrides { dedupKey apiKey }
  }
}`;

export const settingsChangedSubscription = `subscription SettingsChanged {
  settingsChanged
}`;

// Unified reactive-refresh feed. Drives ReactiveRefresh v2 invalidation: a
// single subscription replaces the per-concern "poke" subscriptions. Passing
// `afterSequence` on (re)subscribe replays missed events from the store for
// lossless catch-up.
export const domainEventFeedSubscription = `subscription DomainEventFeed($afterSequence: Long) {
  domainEventFeed(afterSequence: $afterSequence) {
    sequence
    eventId
    occurredAt
    actorKind
    actorUserId
    actorDisplayName
    titleId
    facet
    eventType
    streamKind
    streamId
    payloadJson
  }
}`;

export const systemHealthQuery = `query SystemHealth {
  systemHealth {
    serviceReady
    dbPath
    datastoreEngine
    datastoreMigrationKey
    runtimePathStyle
    totalTitles
    monitoredTitles
    totalUsers
    titlesMovie
    titlesSeries
    titlesAnime
    titlesOther
    recentEvents
    recentEventPreview
    dbMigrationVersion
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

export const smgVersionCompatibilityNoticeQuery = `query SmgVersionCompatibilityNotice {
  smgVersionCompatibilityNotice {
    status
    minimumVersion
    yourVersion
    message
    upgradeDeadline
  }
}`;

export const smgScryerUpdateNoticeQuery = `query SmgScryerUpdateNotice {
  smgScryerUpdateNotice {
    available
    currentVersion
    latestVersion
    latestTag
    releaseUrl
    publishedAt
    checkedAt
  }
}`;

export const applicationUpgradeStatusQuery = `query ApplicationUpgradeStatus {
  applicationUpgradeStatus {
    currentVersion
    updateVersion
    updateTag
    updateAvailable
    installationKind
    managementOwner
    eligible
    eligibilityReason
    activeRun {
${JOB_RUN_FIELDS}
    }
    latestRun {
${JOB_RUN_FIELDS}
    }
  }
}`;

export const startApplicationUpgradeMutation = `mutation StartApplicationUpgrade($input: StartApplicationUpgradeInput!) {
  startApplicationUpgrade(input: $input) {
    jobRun {
${JOB_RUN_FIELDS}
    }
  }
}`;

export const scryerVersionQuery = `query ScryerVersion {
  scryerVersion
}`;

export const instanceFeaturesQuery = `query InstanceFeatures {
  instanceFeatures {
    experimentalFeaturesEnabled
    personalizedDiscoveryEnabled
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

export const wantedNavigationCountsQuery = `query WantedNavigationCounts($libraryIds: [ID!], $titleSearch: String, $cutoffFacet: MediaFacetValue) {
  wantedItems(wantedKind: MISSING, facet: null, libraryIds: $libraryIds, titleSearch: $titleSearch, limit: 1, offset: 0) {
    totalCount
  }
  cutoffUnmetTitlesPage(facet: $cutoffFacet, libraryIds: $libraryIds, limit: 1, offset: 0) {
    totalCount
  }
  pendingReleases(filter: null, limit: 1, offset: 0) {
    totalCount
  }
}`;

export const wantedItemsQuery = `query WantedItems($wantedKind: WantedKindValue!, $facet: MediaFacetValue, $libraryIds: [ID!], $titleSearch: String, $limit: Int, $offset: Int) {
  wantedItems(wantedKind: $wantedKind, facet: $facet, libraryIds: $libraryIds, titleSearch: $titleSearch, limit: $limit, offset: $offset) {
    items {
      id
      titleId
      titleName
      titleSlug
      titleFacet
      libraryId
      libraryName
      librarySlug
      episodeId
      collectionId
      seasonNumber
      episodeNumber
      mediaType
      lastSearchAt
      status
      grabbedRelease
      currentScore
      standbyCount
      latestReleaseDecision {
        decisionCode
        createdAt
      }
      mismatchRecoveryEligible
      convergenceState
      indexersCovered
      indexersRouted
      recencyLane
      createdAt
      updatedAt
    }
    totalCount
    hasMore
  }
}`;

export const releaseDecisionsQuery = `query ReleaseDecisions($wantedItemId: ID!, $limit: Int) {
  wantedItem(id: $wantedItemId) {
    id
    releaseDecisions(limit: $limit) {
      items {
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
      hasMore
    }
  }
}`;

export const pluginsQuery = `query Plugins {
  plugins {
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
  pluginCatalogStatus {
    refreshState
    githubAvailable
    lastCheckedAt
    outageMessage
    blockedActions
    restoreWarnings
    lastError
  }
}`;

export const backupsQuery = `query Backups {
  backups {${BACKUP_INFO_FIELDS}
  }
}`;

export const recycleBinSettingsQuery = `query RecycleBinSettings {
  recycleBinSettings {
    enabled
  }
}`;

/**
 * The verification depth applied to download-client import copies. Location
 * operations force full depth and ignore this.
 */
export const verificationSettingsQuery = `query VerificationSettings {
  verificationSettings {
    depth
  }
}`;

export const pluginAutoUpdateSettingsQuery = `query PluginAutoUpdateSettings {
  pluginAutoUpdateSettings {
    enabled
  }
}`;

export const recycledItemsQuery = `query RecycledItems($libraryIds: [ID!]) {
  recycledItems(libraryIds: $libraryIds) {
    items {
      id
      originalPath
      fileName
      sizeBytes
      titleId
      titleName
      reason
      recycledAt
      scheduledDeletionAt
      mediaRoot
      libraryId
      libraryName
    }
    totalCount
  }
}`;

export const previewRestoreRecycledItemsQuery = `query PreviewRestoreRecycledItems($ids: [ID!]!) {
  previewRestoreRecycledItems(ids: $ids) {
    fingerprint
    items {
      id
      originalPath
      destinationOccupied
    }
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
    managedTagFilter
    createdAt
    updatedAt
  }
}`;

// ── Maintenance Rules ─────────────────────────────────────────────────

export const MAINTENANCE_RULE_SET_FIELDS = `
    id
    name
    description
    enabled
    evaluationMode
    libraryIds
    subjectKind
    currentRevisionNumber
    graceDays
    actionSpec {
      kind
      schemaVersion
      targetQualityProfileId
    }
    createdAt
    updatedAt`;

const maintenanceRuleRevisionFieldSelection = `
    id
    ruleSetId
    revisionNumber
    regoSource
    graceDays
    matcherContentHash
    createdBy
    createdAt`;

/// The list is the only rule-set document that reads `effectArming`: the field
/// is not on the shared selection because every mutation payload also embeds
/// that selection, and arming is refetched from the list after a mutation
/// rather than read back off it.
export const maintenanceRuleSetsQuery = `query MaintenanceRuleSets {
  maintenanceRuleSets {${MAINTENANCE_RULE_SET_FIELDS}
    effectArming
  }
}`;

/// Detail payload shared by the rule-set query and every mutation that returns
/// a rule set with its current revision and action.
export const MAINTENANCE_RULE_SET_DETAIL_FIELDS = `
    ruleSet {${MAINTENANCE_RULE_SET_FIELDS}
    }
    revision {${maintenanceRuleRevisionFieldSelection}
    }
    actionSpec {
      kind
      schemaVersion
      targetQualityProfileId
    }`;

export const maintenanceRuleSetQuery = `query MaintenanceRuleSet($id: ID!) {
  maintenanceRuleSet(id: $id) {${MAINTENANCE_RULE_SET_DETAIL_FIELDS}
  }
}`;

export const maintenanceRuleRevisionsQuery = `query MaintenanceRuleRevisions($ruleSetId: ID!) {
  maintenanceRuleRevisions(ruleSetId: $ruleSetId) {${maintenanceRuleRevisionFieldSelection}
  }
}`;

export const maintenanceActionDescriptorsQuery = `query MaintenanceActionDescriptors {
  maintenanceActionDescriptors {
    kind
    supportedSubjects
    riskClass
    effectClasses
    timingMode
    allowedRepeatModes
    requiresTargetQualityProfile
  }
}`;

/// Candidates. This admin surface always passes `includeShadow: true`: with the
/// result-display gate closed the server otherwise returns nothing, and an
/// operator deciding whether to open that gate has to see what shadow
/// evaluation actually found first.
export const maintenanceCandidatesQuery = `query MaintenanceCandidates($ruleSetId: ID, $states: [MaintenanceCandidateState!], $libraryId: ID, $includeShadow: Boolean, $limit: Int) {
  maintenanceCandidates(ruleSetId: $ruleSetId, states: $states, libraryId: $libraryId, includeShadow: $includeShadow, limit: $limit) {
    id
    ruleSetId
    ruleName
    revisionNumber
    titleId
    titleName
    libraryId
    facet
    state
    stateReason
    reasonCodes
    actionKind
    graceDays
    matchGeneration
    firstMatchedAt
    lastMatchedAt
    dueAt
    heldSince
    updatedAt
  }
}`;

export const maintenanceEvaluationRunsQuery = `query MaintenanceEvaluationRuns($ruleSetId: ID, $limit: Int) {
  maintenanceEvaluationRuns(ruleSetId: $ruleSetId, limit: $limit) {
    id
    ruleSetId
    revisionNumber
    status
    startedAt
    finishedAt
    evaluatedCount
    matchedCount
    noMatchCount
    unknownCount
    errorCount
    durationMs
    error
  }
}`;

export const maintenanceActionRunsQuery = `query MaintenanceActionRuns($ruleSetId: ID, $candidateId: ID, $limit: Int) {
  maintenanceActionRuns(ruleSetId: $ruleSetId, candidateId: $candidateId, limit: $limit) {
    id
    ruleSetId
    candidateId
    titleId
    titleName
    actionKind
    matchGeneration
    attempt
    status
    holdReason
    error
    startedAt
    finishedAt
  }
}`;

/// Reading the gates requires system-settings management. A reader without it
/// gets a GraphQL error rather than a partial answer, so the caller catches it
/// and renders the panel read-locked instead of failing the whole section.
export const maintenanceInstanceGatesQuery = `query MaintenanceInstanceGates {
  maintenanceInstanceGates {
    evaluationEnabled
    resultDisplayEnabled
    presentationEffectsEnabled
    reversibleEffectsEnabled
    destructiveEffectsEnabled
  }
}`;

export const MAINTENANCE_EXCLUSION_FIELDS = `
    id
    ruleSetId
    titleId
    titleName
    reason
    createdBy
    createdAt`;

export const maintenanceExclusionsQuery = `query MaintenanceExclusions($ruleSetId: ID) {
  maintenanceExclusions(ruleSetId: $ruleSetId) {${MAINTENANCE_EXCLUSION_FIELDS}
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

export const notificationTargetsQuery = `query NotificationTargets {
  notificationTargets {${NOTIFICATION_TARGET_FIELDS}
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
  mediaServerConnections {${MEDIA_SERVER_CONNECTION_FIELDS}
  }
  notificationTargets {${NOTIFICATION_TARGET_FIELDS}
  }
  notificationSubscriptions {${NOTIFICATION_SUBSCRIPTION_FIELDS}
  }
  notificationProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  notificationEventTypes
  runtimeInfo {
    runtimePathStyle
  }
}`;

// ── Metadata Gateway (proxied through backend) ────────────────────────

const METADATA_SEARCH_FIELDS = `
    tvdbId
    smgId
    tmdbId
    primarySource
    externalIds {
      source
      value
    }
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

export const searchMetadataQuery = `query SearchMetadata($query: String!, $type: MediaFacetValue!, $limit: Int, $language: String! = "eng", $year: Int) {
  searchMetadata(query: $query, type: $type, limit: $limit, language: $language, year: $year) {${METADATA_SEARCH_FIELDS}
  }
}`;

export const pendingImportTitleSearchQuery = `query PendingImportTitleSearch($pendingImportId: ID!, $query: String!, $limit: Int = 8, $language: String! = "eng", $year: Int) {
  pendingImportTitleSearch(pendingImportId: $pendingImportId, query: $query, limit: $limit, language: $language, year: $year) {${METADATA_SEARCH_FIELDS}
  }
}`;

export const pendingImportCountsQuery = `query PendingImportCounts {
  pendingImportCounts {
    movie
    series
    anime
  }
}`;

export const navigationBadgeCountsQuery = `query NavigationBadgeCounts {
  navigationBadgeCounts {
    pendingImportCounts {
      movie
      series
      anime
    }
    pendingMediaRequestCounts {
      movie
      series
      anime
    }
    activityImportCount
    pluginUpdateCount
  }
}`;

export const pendingImportsQuery = `query PendingImports($facet: MediaFacetValue!, $libraryIds: [ID!], $status: PendingImportStatusValue! = PENDING, $limit: Int = 50, $offset: Int = 0) {
  pendingImports(facet: $facet, libraryIds: $libraryIds, status: $status, limit: $limit, offset: $offset) {
    totalCount
    hasMore
    items {
      id
      libraryId
      facet
      status
      titleId
      titleName
      titleSlug
      displayName
      path
      folderPath
      query
      yearHint
      reason
      searchAttempts {
        query
        resultCount
        topResults
        summary
      }
    }
  }
}`;

export const pendingImportBindingPreviewQuery = `query PendingImportBindingPreview($pendingImportId: ID!) {
  pendingImportBindingPreview(pendingImportId: $pendingImportId) {
    title {
      id
      name
      facet
      monitored
    }
    file {
      filePath
      fileName
      sizeBytes
      parsedSeason
      parsedEpisodes
      parsedAbsoluteNumbers
      suggestedEpisodeIds
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

export const searchMetadataMultiQuery = `query SearchMetadataMulti($query: String!, $limit: Int, $language: String! = "eng") {
  searchMetadataMulti(query: $query, limit: $limit, language: $language) {
    movies {${METADATA_SEARCH_FIELDS}
    }
    series {${METADATA_SEARCH_FIELDS}
    }
    anime {${METADATA_SEARCH_FIELDS}
    }
  }
}`;

export const metadataMovieQuery = `query MetadataMovie($input: MetadataMovieInput!) {
  metadataMovie(input: $input) {
    tvdbId
    smgId
    tmdbId
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
    studio
    tmdbReleaseDate
  }
}`;

export const metadataSeriesQuery = `query MetadataSeries($input: MetadataSeriesInput!) {
  metadataSeries(input: $input) {
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
    aliases
    seasons {
      tvdbId
      number
      label
      episodeType
    }
    animeMovies {
      name
      year
      associationConfidence
      continuityStatus
      placement
      externalIds {
        source
        value
      }
    }
    episodes {
      tvdbId
      episodeNumber
      seasonNumber
      name
      aired
      runtimeMinutes
      isFiller
      imageUrl
    }
  }
}`;

export const pendingReleasesQuery = `query PendingReleases($filter: PendingReleaseFilterInput, $limit: Int, $offset: Int) {
  pendingReleases(filter: $filter, limit: $limit, offset: $offset) {
    items {
      id
      wantedItemId
      titleId
      releaseTitle
      releaseUrl
      releaseSizeBytes
      releaseScore
      scoringLogJson
      indexerSource
      publishedAt
      seeders
      addedAt
      delayUntil
      lastDecisionCode
      role
      status
    }
    hasMore
    totalCount
  }
}`;

export const calendarEpisodesQuery = `query CalendarEpisodes($startDate: Date!, $endDate: Date!, $libraryIds: [ID!]) {
  calendarEpisodes(startDate: $startDate, endDate: $endDate, libraryIds: $libraryIds) {
    id
    titleId
    libraryId
    libraryName
    librarySlug
    titleName
    titleSlug
    titleFacet
    seasonNumber
    episodeNumber
    episodeTitle
    overview
    imageUrl
    airDate
    monitored
    playbackLinks {
      connectionId
      displayName
      provider
      href
    }
    mediaAvailability {
      state
      primaryQualityLabel
    }
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

export const browsePathQuery = `query BrowsePath($path: String!, $includeFiles: Boolean) {
  browsePath(path: $path, includeFiles: $includeFiles) {
    name
    path
    isDirectory
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

export const postProcessingScriptRunsQuery = `query PostProcessingScriptRuns($scriptId: ID!, $limit: Int) {
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

export const externalSubtitlesQuery = `query ExternalSubtitles($titleId: ID!) {
  externalSubtitles(titleId: $titleId) {${EXTERNAL_SUBTITLE_FIELDS}
  }
}`;

export const titleHistoryQuery = `query TitleHistory($filter: TitleHistoryFilterInput!) {
  titleHistory(filter: $filter) {
    items {
      id
      titleId
      titleName
      facet
      episodeId
      episodeIds
      collectionId
      eventType
      actorKind
      actorUserId
      actorDisplayName
      sourceTitle
      displayTitle
      sourceSystem
      sourceRef
      sourceProvider
      sourceHint
      quality
      downloadId
      clientId
      clientName
      importId
      skipReason
      retryRequiresPassword
      failureReason
      blocklistReason
      sourcePath
      destPath
      dataJson
      occurredAt
      createdAt
    }
    totalCount
  }
}`;

export const mediaRequestsQuery = `query MediaRequests($facet: MediaFacetValue, $libraryIds: [ID!], $status: MediaRequestStatusValue) {
  mediaRequests(facet: $facet, libraryIds: $libraryIds, status: $status) {
    id
    libraryId
    facet
    status
    identityFingerprint
    title
    sortTitle
    slug
    posterUrl
    backgroundUrl
    year
    overview
    runtimeMinutes
    language
    contentStatus
    rating
    ratingSources
    externalRatings {
      source
      value
      score
      normalized
      votes
      url
    }
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
    requestedMonitorSelection {
      seasonNumbers
      seriesMovies {
        name
        externalIds {
          source
          value
        }
      }
    }
    resolvedByUserId
    resolvedAt
    createdTitleId
    approvedQualityProfileId
    approvedQualityProfileName
    externalIds {
      source
      value
    }
    requesters {
      userId
      username
      avatarUrl
      requestedAt
    }
    createdByUserId
    createdAt
    updatedAt
  }
}`;

export const myMediaRequestsQuery = `query MyMediaRequests($facet: MediaFacetValue, $libraryIds: [ID!], $status: MediaRequestStatusValue) {
  myMediaRequests(facet: $facet, libraryIds: $libraryIds, status: $status) {
    id
    libraryId
    facet
    status
    identityFingerprint
    title
    sortTitle
    slug
    posterUrl
    backgroundUrl
    year
    overview
    runtimeMinutes
    language
    contentStatus
    rating
    ratingSources
    externalRatings {
      source
      value
      score
      normalized
      votes
      url
    }
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
    requestedMonitorSelection {
      seasonNumbers
      seriesMovies {
        name
        externalIds {
          source
          value
        }
      }
    }
    resolvedByUserId
    resolvedAt
    createdTitleId
    approvedQualityProfileId
    approvedQualityProfileName
    externalIds {
      source
      value
    }
    requesters {
      userId
      username
      avatarUrl
      requestedAt
    }
    createdByUserId
    createdAt
    updatedAt
  }
}`;

// ── Dashboard ──────────────────────────────────────────────────────────────
// Split by refresh cadence rather than by panel: the overview is the slow
// half-page of counters and provider health, while requests, imports and the
// import history each refetch on their own when the operator acts on them.

export const dashboardOverviewQuery = `query DashboardOverview($activityWindowHours: Int!) {
  me {
    id
    username
  }
  navigationBadgeCounts {
    activityImportCount
    pendingMediaRequestCounts {
      movie
      series
      anime
    }
  }
  systemHealth {
    titlesMovie
    titlesSeries
    titlesAnime
    indexerStats {
      indexerId
      indexerName
      queriesLast24H
      failedLast24H
      grabsLast24H
      apiCurrent
      apiMax
    }
  }
  dashboardActivityStats(windowHours: $activityWindowHours) {
    current {
      grabbed
      upgraded
      imported
      importFailed
    }
    previous {
      grabbed
      upgraded
      imported
      importFailed
    }
  }
  indexers {
    id
    name
    providerType
    isEnabled
    lastHealthStatus
    lastErrorMessage
    lastErrorAt
  }
  downloadClientConfigs {
    id
    name
    clientType
    isEnabled
    status
    lastError
    lastSeenAt
  }
  storageRoots {
    path
    libraryId
    libraryName
    facet
    usedBytes
    totalBytes
  }
}`;

export const dashboardPendingRequestsQuery = `query DashboardPendingRequests {
  mediaRequests(status: PENDING) {
    id
    libraryId
    facet
    title
    year
    posterUrl
    requestedQualityProfileId
    requestedMonitorType
    createdAt
    requesters {
      userId
      username
      avatarUrl
    }
  }
  libraries(permission: MANAGE_TITLES) {
    id
    name
    facet
    qualityProfileId
    requestQualityProfileDefaultId
  }
}`;

export const dashboardRecentImportsQuery = `query DashboardRecentImports($limit: Int!) {
  titleHistory(
    filter: { eventTypes: [IMPORTED, FILE_UPGRADED], limit: $limit }
  ) {
    totalCount
    items {
      id
      titleId
      titleName
      posterUrl
      facet
      libraryId
      eventType
      quality
      sizeBytes
      occurredAt
    }
  }
}`;

export const indexerDownloadClientMappingCatalogQuery = `query IndexerDownloadClientMappingCatalog {
  indexerDownloadClientMappingCatalog {
    clients {
      id
      name
      clientType
      isEnabled
      healthStatus
    }
    indexers {
      id
      name
      downloadClientId
      protocolFamilies
      supportsMapping
      compatibleClientIds
    }
    providerCompatibility {
      providerType
      protocolFamilies
      supportsMapping
      compatibleClientIds
    }
  }
}`;

const LOCATION_PLAN_ITEM_FIELDS = `
      kind
      titleId
      mediaFileId
      sourcePath
      destinationPath
      sizeBytes
      sameVolume
      reasonCode
      detail`;

export const locationOperationPreviewQuery = `query LocationOperationPreview($input: LocationOperationPreviewInput!) {
  locationOperationPreview(input: $input) {
    planFingerprint
    operationType
    mode
    sourceLibraryId
    destinationLibraryId
    sourceRootId
    destinationRootId
    selection
    counts {
      itemsTotal
      titlesTotal
      filesTotal
      bytesTotal
      byKind {
        kind
        count
      }
    }
    sections {
      kind
      itemsTotal
      bytesTotal
      complete
      items {${LOCATION_PLAN_ITEM_FIELDS}
      }
    }
    classification {
      titlesTotal
      blocksStart
      groups {
        class
        count
        titles {
          titleId
          class
          sourceLibraryId
          sourceRootId
          sourceFolderPath
          destinationLibraryId
          destinationRootId
          reasonCode
          reason
          destinationIdentityMatch
          mergeTargetTitleId
          mergeTargetTitleName
          sameNamedDestinationTitleId
          sameNamedDestinationTitleName
          ambiguousDestinationTitleIds
          ambiguousDestinationCandidates {
            titleId
            titleName
            sharedIdentities
          }
        }
      }
    }
    freeSpace {
      destinationRequiredBytes
      destinationTotalRequiredBytes
      destinationAvailableBytes
      recycleRequiredBytes
      recycleAvailableBytes
      sameVolumeMove
      recycleOnOtherVolume
      recycleSharesDestinationVolume
      recyclingAvailable
      probed
      sufficient
    }
    verification {
      depth
      files
      bytes
      applies
    }
    confirmation {
      requirement
      typedPhrase
      typedPrompt
    }
    warnings
    blocksStart
    merges {
      sourceTitleId
      destinationTitleId
      destinationTitleName
      sourceLibraryId
      destinationLibraryId
      blocked
      blockedRecords {
        table
        reason
        sourceId
        detail
      }
      mediaFilesRepointed
      roleChanges {
        fileId
        sourceEpisodeId
        destinationEpisodeId
        previousRole
        newRole
        reason
        detail
      }
      historyRowsCarried
      sourceRecordsDropped
    }
  }
}`;

export const LOCATION_OPERATION_FIELDS = `
    id
    operationType
    mode
    state
    initiatedByUserId
    sourceLibraryId
    destinationLibraryId
    sourceRootId
    destinationRootId
    planFingerprint
    verificationDepth
    verificationFallbackCount
    counters {
      titlesTotal
      titlesProcessed
      titlesBlocked
      filesTotal
      filesProcessed
      bytesTotal
      bytesProcessed
      merges
      dedups
      renames
      noOps
      unresolved
    }
    detail
    jobRunId
    workflowOperationId
    cancelRequested
    cancelRequestedAt
    confirmedAt
    startedAt
    createdAt
    updatedAt
    completedAt
    titleCheckpoints {
      titleId
      sequence
      state
      classification
      sourceLibraryId
      sourceRootId
      sourceFolderPath
      destinationLibraryId
      destinationRootId
      destinationFolderPath
      mergedIntoTitleId
      mergedIntoTitleName
      filesTotal
      filesVerified
      bytesTotal
      bytesVerified
      detail
      startedAt
      updatedAt
      completedAt
    }`;

export const locationOperationQuery = `query LocationOperation($id: ID!) {
  locationOperation(id: $id) {${LOCATION_OPERATION_FIELDS}
  }
}`;

/**
 * Which files the operation renamed and deduplicated (FR-091). Read separately
 * from the operation row because the per-file identities live in the stored
 * plan, which the two-second progress poll has no reason to load.
 */
export const locationOperationAssetsQuery = `query LocationOperationAssets($id: ID!) {
  locationOperationAssets(id: $id) {
    operationId
    renamesTotal
    renamesDone
    dedupsTotal
    dedupsDone
    titles {
      titleId
      titleName
      sequence
      settled
      checkpointState
      renames {
        sourcePath
        sourceName
        destinationPath
        destinationName
        provenanceLabel
        mediaFileId
        sizeBytes
        done
      }
      dedups {
        sourcePath
        sourceName
        survivingPath
        survivingName
        done
      }
    }
  }
}`;

/**
 * The plan fields both root-scoped previews share (US4, US5). Narrower than the
 * move dialog's selection: a root-scoped plan has no per-title classification
 * entries to read, and the titles that need naming come from the accounting
 * ledger instead (FR-023).
 */
const LOCATION_ROOT_PLAN_FIELDS = `
    planFingerprint
    operationType
    mode
    sourceLibraryId
    destinationLibraryId
    sourceRootId
    destinationRootId
    counts {
      itemsTotal
      titlesTotal
      filesTotal
      bytesTotal
      byKind {
        kind
        count
      }
    }
    sections {
      kind
      itemsTotal
      bytesTotal
      complete
      items {${LOCATION_PLAN_ITEM_FIELDS}
      }
    }
    classification {
      titlesTotal
      blocksStart
      groups {
        class
        count
      }
    }
    freeSpace {
      destinationRequiredBytes
      destinationTotalRequiredBytes
      destinationAvailableBytes
      recycleRequiredBytes
      recycleAvailableBytes
      sameVolumeMove
      recycleOnOtherVolume
      recycleSharesDestinationVolume
      recyclingAvailable
      probed
      sufficient
    }
    verification {
      depth
      files
      bytes
      applies
    }
    confirmation {
      requirement
      typedPhrase
      typedPrompt
    }
    warnings
    blocksStart`;

/** The every-title ledger, with no exclude affordance to offer (FR-023). */
const LOCATION_TITLE_ACCOUNTING_FIELDS = `
    assignedTotal
    relocating
    catalogOnly
    blocked
    accountsForEveryTitle
    blocksStart
    blockedTitles {
      titleId
      titleName
      reason
      reasonCode
    }`;

/** FR-027's three buckets plus the directory facts cleanup may act on. */
const LOCATION_ROOT_CONTENT_FIELDS = `
    managed {
      class
      total
      bytesTotal
      complete
      entries {
        path
        sizeBytes
        class
        canonicalSidecar
      }
    }
    companions {
      class
      total
      bytesTotal
      complete
      entries {
        path
        sizeBytes
        class
        canonicalSidecar
      }
    }
    unknown {
      class
      total
      bytesTotal
      complete
      entries {
        path
        sizeBytes
        class
        canonicalSidecar
      }
    }
    unknownBytes
    blocksSourceRemoval
    entryCount
    prunableDirectories {
      total
      complete
      paths
    }
    retainedDirectories {
      total
      complete
      paths
    }`;

/** What happens to the old location afterwards (FR-028, FR-031, FR-087). */
const LOCATION_ROOT_RETIREMENT_FIELDS = `
    sourceRootPath
    destinationRootPath
    retireConfigurationAfterRecycling
    recycleAllowlistPaths {
      total
      complete
      paths
    }
    requiresVerificationBeforeSourceRemoval
    emptyDirectoriesOnly
    removableDirectories {
      total
      complete
      paths
    }
    retainedDirectories {
      total
      complete
      paths
    }
    permitsSourceRemoval
    blockers {
      code
      detail
    }`;

/**
 * FR-020's **Change root**, both destinations: replacing one root's path with a
 * new location, or folding it into another root of the same library.
 *
 * One query, because it is one settings action. Name exactly one of
 * `destinationPath` and `destinationRootId`; a path that is already a root of
 * this library resolves to that root and is planned as a fold. One payload,
 * too: `retention` comes back for a path change and `classification` /
 * `defaultTransfer` for a fold, so the branch the server planned is readable
 * off the response.
 */
export const locationRootScopePreviewQuery = `query LocationRootScopePreview($input: LocationRootScopePreviewInput!) {
  locationRootScopePreview(input: $input) {
    plan {${LOCATION_ROOT_PLAN_FIELDS}
    }
    accounting {${LOCATION_TITLE_ACCOUNTING_FIELDS}
    }
    retention {
      rootId
      keepsRootId
      wasLibraryDefault
      remainsLibraryDefault
      retainedRole
      retainedTitleAssignments
    }
    classification {
      movingIntoUnusedFolders
      mergingWithDestinationTitles
      folderNameCollisions
      mediaCollisions
      dedupEligibleFiles
      companionCollisions
      untrackedSourceEntries
      catalogOnly
      blocked
    }
    defaultTransfer {
      sourceWasDefault
      destinationWasDefault
      destinationBecomesDefault
      transfersTheDefault
    }
    content {${LOCATION_ROOT_CONTENT_FIELDS}
    }
    retirement {${LOCATION_ROOT_RETIREMENT_FIELDS}
    }
  }
}`;
