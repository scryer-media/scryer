export const TITLE_CORE_FIELDS = `
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
    requiredAudioLanguagesOverride
    effectiveRequiredAudioLanguages
    inheritsRequiredAudioLanguages
    rootFolderId
    rootFolderPath
    monitorType
    useSeasonFolders
    monitorSpecials
    interSeasonMovies
    fillerPolicy
    recapPolicy
    createdAt`;

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
    stringValue
    boolValue
    intValue
    floatValue
    secretStored`;

const SERIES_MOVIE_LINK_FIELDS = `
      id
      seriesTitleId
      placement
      narrativeOrder
      afterSeason
      beforeSeason
      linkedEpisodeId
      associationConfidence
      continuityStatus
      movieForm
      confidence
      signalSummary
      source
      monitored
      createdAt
      updatedAt
      movie {
        id
        title
        sortTitle
      slug
      year
        overview
        posterUrl
        backgroundUrl
        language
        runtimeMinutes
      contentStatus
      imdbId
        tvdbId
        tmdbId
        malId
        anidbId
      genres
      studio
      digitalReleaseDate
        createdAt
        updatedAt
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
      imageUrl
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
      monitored
      createdAt
      episodes {${COLLECTION_EPISODE_FIELDS}
      }`;

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
    episodeId
    titleName
    facet
    isScryerOrigin
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
    queueScope {
      kind
      episodeId
      episodeIds
      collectionId
      seriesMovieLinkId
    }`;

const TITLE_OVERVIEW_FIELDS = `${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
    seriesMovieLinks {${SERIES_MOVIE_LINK_FIELDS}
    }
    mediaFiles {${TITLE_MEDIA_FILE_FIELDS}
    }
    wantedItems {
      items {${WANTED_ITEM_FIELDS}
      }
    }`;

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
    sourceHint
    sourceTitle
    errorMessage
    attemptedAt
    episodeIds`;

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
      options { value label }
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

export const titleDetailQuery = `query TitleDetail($id: ID!) {
  title(id: $id) {${TITLE_CORE_FIELDS}
    collections {${TITLE_COLLECTION_FIELDS}
    }
    seriesMovieLinks {${SERIES_MOVIE_LINK_FIELDS}
    }
  }
  titleHistory: titleHistory(filter: { titleIds: [$id], limit: 50, offset: 0 }) {
    records {${TITLE_EVENT_FIELDS}
    }
  }
}`;

export const titleBySlugQuery = `query TitleBySlug($facet: MediaFacetValue!, $librarySlug: String, $slug: String!) {
  titleBySlug(facet: $facet, librarySlug: $librarySlug, slug: $slug) {
    id
    slug
    libraryId
    librarySlug
  }
}`;

export const titleReleaseBlocklistQuery = `query TitleReleaseBlocklist($titleId: ID!, $limit: Int) {
  titleReleaseBlocklist(titleId: $titleId, limit: $limit) {${TITLE_RELEASE_BLOCKLIST_FIELDS}
  }
}`;

export const titleOverviewNativeQuery = `query TitleOverviewNative($id: ID!, $blocklistLimit: Int) {
  title(id: $id) {${TITLE_OVERVIEW_FIELDS}
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
    records {${TITLE_EVENT_FIELDS}
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

export const titleOverviewDownloadFeedbackQuery = `query TitleOverviewDownloadFeedback($id: ID!) {
  downloadQueueItems: downloadQueue(titleId: $id, includeAllActivity: true, includeImportActivity: true, activityFilter: all) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
  completedDownloadQueueItems: downloadQueue(titleId: $id, includeAllActivity: true, includeHistoryOnly: true, activityFilter: all) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const titleDownloadQueueItemsQuery = `query TitleDownloadQueueItems($id: ID!) {
  title(id: $id) {
    id
    downloadQueueItems {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
  }
}`;

export const deleteTitlePreviewQuery = `query DeleteTitlePreview($titleId: ID!) {
  deleteTitlePreview(titleId: $titleId) {${DELETE_PREVIEW_FIELDS}
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

export const deleteExternalSubtitlePreviewQuery = `query DeleteExternalSubtitlePreview($externalSubtitleId: ID!) {
  deleteExternalSubtitlePreview(externalSubtitleId: $externalSubtitleId) {${DELETE_PREVIEW_FIELDS}
  }
}`;

export const externalSubtitleBlocklistEntriesQuery = `query ExternalSubtitleBlocklistEntries($mediaFileId: ID!) {
  externalSubtitleBlocklistEntries(mediaFileId: $mediaFileId) {${EXTERNAL_SUBTITLE_BLOCKLIST_FIELDS}
  }
}`;

export const searchForTitleQuery = `query SearchReleasesForTitle($titleId: ID!) {
  searchReleases(input: { titleId: $titleId }) {
    source
    title
    link
    downloadUrl
    candidateToken
    queueScope {
      kind
      episodeId
      episodeIds
      collectionId
      seriesMovieLinkId
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
    infoHash
    freeleech
    downloadVolumeFactor
    autoEligible
    autoDecisionCode
    autoDecisionSummary
  }
}`;

export const searchForEpisodeQuery = `query SearchReleasesForEpisode($titleId: ID!, $season: String!, $episode: String!) {
  searchReleases(input: {
    titleId: $titleId,
    season: $season,
    episode: $episode
  }) {
    source
    title
    link
    downloadUrl
    candidateToken
    queueScope {
      kind
      episodeId
      episodeIds
      collectionId
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
    infoHash
    freeleech
    downloadVolumeFactor
    autoEligible
    autoDecisionCode
    autoDecisionSummary
  }
}`;

export const searchForSeriesMovieQuery = `query SearchReleasesForSeriesMovie($titleId: ID!, $seriesMovieLinkId: ID!) {
  searchReleases(input: {
    titleId: $titleId,
    seriesMovieLinkId: $seriesMovieLinkId
  }) {
    source
    title
    link
    downloadUrl
    candidateToken
    queueScope {
      kind
      episodeId
      episodeIds
      collectionId
      seriesMovieLinkId
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
    infoHash
    freeleech
    downloadVolumeFactor
    autoEligible
    autoDecisionCode
    autoDecisionSummary
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
    imdbId
    posterUrl
    posterSourceUrl
    rootFolderId
    rootFolderPath
    qualityTier
    sizeBytes
    episodesOwned
    episodesMonitored
    episodesTotal
    contentStatus
    metadataFetchedAt
    createdAt`;

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
    librarySlug
    monitored
    slug
    posterUrl
    posterSourceUrl
    metadataFetchedAt
    createdAt
    externalIds {
      source
      value
    }`;

export const TITLE_LIST_FIELDS_WITH_EXTERNAL_IDS = `${TITLE_LIST_FIELDS}
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
    roots {
      id
      path
      isDefault
    }
  }
}`;

export const mediaRequestAdminLibrariesQuery = `query MediaRequestAdminLibraries($facet: MediaFacetValue) {
  libraries(facet: $facet, permission: manageTitles) {
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
  libraries(facet: $facet, permission: request) {
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
    }
  }
}`;

export const titlesQuery = `query Titles(
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
${TITLE_LIST_FIELDS}
    }
    limit
    offset
    hasMore
    totalCount
  }
}`;

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

export const titleListEntryQuery = `query TitleListEntry($id: ID!) {
  title(id: $id) {
${TITLE_LIST_FIELDS}
  }
}`;

type ReactiveRefreshVariableValue = string | number | null;

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
    }
  | {
      key: string;
      kind: "titleOverviewNative";
      titleId: string;
      blocklistLimit: number;
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
      kind: "titleOverviewNative";
      titleAlias: string;
      titleAcquisitionDiagnosticsAlias: string;
      titleHistoryAlias: string;
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
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${TITLE_LIST_FIELDS}\n  }`,
        );
        variables[titleIdVariableName] = action.titleId;
        actionPlans.push({ key: action.key, kind: action.kind, titleAlias });
        break;
      }
      case "titleOverviewNative": {
        const titleIdVariableName = `titleOverviewId${index}`;
        const blocklistLimitVariableName = `titleOverviewBlocklistLimit${index}`;
        const titleAlias = `titleOverviewTitleAction${index}`;
        const titleAcquisitionDiagnosticsAlias = `titleOverviewDiagnosticsAction${index}`;
        const titleHistoryAlias = `titleOverviewHistoryAction${index}`;
        const titleReleaseBlocklistAlias = `titleOverviewBlocklistAction${index}`;
        const externalSubtitlesAlias = `titleOverviewExternalSubtitlesAction${index}`;
        const setupStatusAlias = `titleOverviewSetupStatusAction${index}`;

        variableDefinitions.push(`$${titleIdVariableName}: ID!`);
        variableDefinitions.push(`$${blocklistLimitVariableName}: Int`);
        fields.push(
          `  ${titleAlias}: title(id: $${titleIdVariableName}) {\n${TITLE_OVERVIEW_FIELDS}\n  }`,
        );
        fields.push(
          `  ${titleAcquisitionDiagnosticsAlias}: titleAcquisitionDiagnostics(titleId: $${titleIdVariableName}) {\n    recentDecisions {\n      id\n      wantedItemId\n      titleId\n      releaseTitle\n      releaseUrl\n      releaseSizeBytes\n      decisionCode\n      candidateScore\n      currentScore\n      scoreDelta\n      explanationJson\n      createdAt\n    }\n    decisionCounts {\n      code\n      count\n    }\n    wantedStatusCounts {\n      status\n      count\n    }\n    pendingReleaseCounts {\n      status\n      count\n    }\n    mismatchRecoveryEligibleCount\n    latestDecisionAt\n    latestWantedSearchAt\n  }`,
        );
        fields.push(
          `  ${titleHistoryAlias}: titleHistory(filter: { titleIds: [$${titleIdVariableName}], limit: 50, offset: 0 }) {\n    records {\n${TITLE_EVENT_FIELDS}\n    }\n  }`,
        );
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
          titleAcquisitionDiagnosticsAlias,
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
          `  ${downloadQueueItemsAlias}: downloadQueue(titleId: $${titleIdVariableName}, includeAllActivity: true, includeImportActivity: true, activityFilter: all) {\n${DOWNLOAD_QUEUE_ITEM_FIELDS}\n  }`,
        );
        fields.push(
          `  ${completedDownloadQueueItemsAlias}: downloadQueue(titleId: $${titleIdVariableName}, includeAllActivity: true, includeHistoryOnly: true, activityFilter: all) {\n${DOWNLOAD_QUEUE_ITEM_FIELDS}\n  }`,
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

export const auditLogQuery = `query AuditLog($eventTypes: [DomainEventTypeValue!], $titleId: ID, $facet: MediaFacetValue, $beforeSequence: Int, $afterSequence: Int, $limit: Int) {
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

export const usersQuery = `query Users {
  users {
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

export const indexersQuery = `query Indexers($providerType: String) {
  indexers(providerType: $providerType) {
    id
    name
    providerType
    baseUrl
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

export const downloadImportQuery = `query DownloadImport($limit: Int, $offset: Int, $filter: DownloadImportFilterValue) {
  downloadImport(limit: $limit, offset: $offset, filter: $filter) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
  }
}`;

export const downloadHistoryQuery = `query DownloadHistory($limit: Int, $offset: Int, $filters: [DownloadHistoryFilterValue!], $clientIds: [ID!], $scryerSubmittedOnly: Boolean, $sortKey: DownloadHistorySortKeyValue, $sortDirection: SortDirectionValue) {
  downloadHistory(limit: $limit, offset: $offset, filters: $filters, clientIds: $clientIds, scryerSubmittedOnly: $scryerSubmittedOnly, sortKey: $sortKey, sortDirection: $sortDirection) {
    items {${DOWNLOAD_QUEUE_ITEM_FIELDS}
    }
    hasMore
    totalCount
    availableClients {
      clientId
      clientName
      clientType
    }
  }
}`;

export const downloadQueueSubscription = `subscription DownloadQueueStream($includeAllActivity: Boolean, $includeHistoryOnly: Boolean, $includeImportActivity: Boolean, $titleId: ID, $activityFilter: DownloadActivityFilterValue) {
  downloadQueue(includeAllActivity: $includeAllActivity, includeHistoryOnly: $includeHistoryOnly, includeImportActivity: $includeImportActivity, titleId: $titleId, activityFilter: $activityFilter) {${DOWNLOAD_QUEUE_ITEM_FIELDS}
  }
}`;

export const importQueueCountQuery = `query ImportQueueCount {
  downloadImport(limit: 1, offset: 0, filter: all) {
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
    requiredAudioLanguages
    folderTemplate
    seasonFolderTemplate
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
    importMode`;

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
  mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
}`;

export const seriesOverviewSettingsInitQuery = `query SeriesOverviewSettingsInit($scope: ContentScopeValue!) {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  mediaSettings(scope: $scope) {${mediaSettingsFieldSelection}
  }
}`;

export const cutoffUnmetTitlesQuery = `query CutoffUnmetTitles($facet: MediaFacetValue, $libraryIds: [ID!]) {
  cutoffUnmetTitles(facet: $facet, libraryIds: $libraryIds) {
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
  }
}`;

export const downloadClientsInitQuery = `query DownloadClientsInit {
  downloadClientConfigs {${downloadClientFieldSelection}
  }
  downloadClientProviderTypes {${PROVIDER_TYPE_FIELDS}
  }
  runtimeInfo {
    runtimePathStyle
  }
}`;

export const libraryDownloadClientsQuery = `query LibraryDownloadClients {
  downloadClientConfigs {${downloadClientFieldSelection}
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
  manageableLibraries: libraries(permission: manageTitles) {
    id
    facet
    name
    slug
    isDefault
    roots {
      id
      path
      isDefault
    }
  }
  requestableLibraries: libraries(permission: request) {
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
  movieSettings: mediaSettings(scope: movie) {${mediaSettingsFieldSelection}
  }
  seriesSettings: mediaSettings(scope: series) {${mediaSettingsFieldSelection}
  }
  animeSettings: mediaSettings(scope: anime) {${mediaSettingsFieldSelection}
  }
}`;

export const globalSearchRequesterInitQuery = `query GlobalSearchRequesterInit {
  qualityProfileSettings {${qualityProfileSettingsFieldSelection}
  }
  requestableLibraries: libraries(permission: request) {
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
  requestableLibraries: libraries(permission: request) {
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
    syncIntervalSeconds
    batchSize
  }
}`;

export const generalSettingsQuery = `query GeneralSettings {
  generalSettings {
    keepHistoryForever
    historyRetentionDays
    pluginHttpCaBundlePem
    pluginHttpTrustedCertificates {
      fingerprintSha256
      pem
    }
  }
}`;

export const securitySettingsQuery = `query SecuritySettings {
  securitySettings {
    formLoginEnabled
    passwordMinLength
    skipLoginForLocalIps
    mfaRequireConfigStepUp
    mfaRequirePasswordLogin
    totpRequireJellyfinLogin
    effectiveFormLoginEnabled
    envOverrideActive
    envOverrideDescription
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
    envOverrideActive
    mfaRequirePasswordLogin
    mfaRequireConfigStepUp
    totpRequireJellyfinLogin
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

export const externalImportMonitorWarmupStatusQuery = `query ExternalImportMonitorWarmupStatus($sessionId: ID!) {
  externalImportMonitorWarmupStatus(sessionId: $sessionId) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const externalImportMonitorWarmupProgressSubscription = `subscription ExternalImportMonitorWarmupProgress($sessionId: ID!) {
  externalImportMonitorWarmupProgress(sessionId: $sessionId) {${EXTERNAL_IMPORT_MONITOR_WARMUP_PROGRESS_FIELDS}
  }
}`;

export const settingsChangedSubscription = `subscription SettingsChanged {
  settingsChanged
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

export const scryerVersionQuery = `query ScryerVersion {
  scryerVersion
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

export const previewManualImportQuery = `query PreviewManualImport($input: PreviewManualImportInput!) {
  previewManualImport(input: $input) {
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
    availableSeriesMovies {
      seriesMovieLinkId
      movieTitle
      year
      runtimeMinutes
    }
  }
}`;

export const previewManualImportPathQuery = `query PreviewManualImportPath($input: PreviewManualImportPathInput!) {
  previewManualImportPath(input: $input) {
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
    availableSeriesMovies {
      seriesMovieLinkId
      movieTitle
      year
      runtimeMinutes
    }
  }
}`;

export const wantedItemsQuery = `query WantedItems($statuses: [WantedStatusValue!], $mediaTypes: [WantedMediaTypeValue!], $titleId: ID, $libraryIds: [ID!], $titleSearch: String, $latestDecisionCodes: [String!], $limit: Int, $offset: Int) {
  wantedItems(statuses: $statuses, mediaTypes: $mediaTypes, titleId: $titleId, libraryIds: $libraryIds, titleSearch: $titleSearch, latestDecisionCodes: $latestDecisionCodes, limit: $limit, offset: $offset) {
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
      searchPhase
      nextSearchAt
      lastSearchAt
      searchCount
      baselineDate
      status
      grabbedRelease
      currentScore
      latestReleaseDecision {
        decisionCode
        createdAt
      }
      mismatchRecoveryEligible
      createdAt
      updatedAt
    }
    total
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

export const recycledItemsQuery = `query RecycledItems($libraryIds: [ID!]) {
  recycledItems(libraryIds: $libraryIds) {
    items {
      id
      originalPath
      fileName
      sizeBytes
      titleId
      reason
      recycledAt
      mediaRoot
      libraryId
      libraryName
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

export const pendingImportsQuery = `query PendingImports($facet: MediaFacetValue!, $libraryIds: [ID!], $status: PendingImportStatusValue! = pending, $limit: Int = 50, $offset: Int = 0) {
  pendingImports(facet: $facet, libraryIds: $libraryIds, status: $status, limit: $limit, offset: $offset) {
    total
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
      addedAt
      delayUntil
      status
    }
    limit
    offset
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
    records {
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
    year
    overview
    runtimeMinutes
    language
    contentStatus
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
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
    year
    overview
    runtimeMinutes
    language
    contentStatus
    requestedQualityProfileId
    requestedQualityProfileName
    requestedMonitorType
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
