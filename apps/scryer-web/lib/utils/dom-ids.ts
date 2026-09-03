export function selectorToken(value: string | number): string {
  const normalized = String(value)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)+/g, "");

  return normalized || "item";
}

type MetadataSearchSelectorInput = {
  name: string;
  imdbId?: string | null;
  smgId?: string | number | null;
  tvdbId?: string | number | null;
};

export function metadataSearchSelectorParts(
  result: MetadataSearchSelectorInput,
): string[] {
  const smgId = String(result.smgId ?? "").trim();
  if (smgId) {
    return ["smg", smgId];
  }

  const imdbId = result.imdbId?.trim();
  if (imdbId) {
    return ["imdb", imdbId];
  }

  const tvdbId = String(result.tvdbId ?? "").trim();
  if (tvdbId) {
    return ["tvdb", tvdbId];
  }

  return ["name", result.name];
}

export function globalSearchMetadataResultId(
  facet: string,
  result: MetadataSearchSelectorInput,
): string {
  return selectorId(
    "global-search-metadata-result",
    facet,
    ...metadataSearchSelectorParts(result),
  );
}

export function titleOverviewRowId(titleId: string): string {
  return selectorId("title-overview-row", titleId);
}

export function wantedItemRowId(wantedItemId: string): string {
  return selectorId("wanted-item-row", wantedItemId);
}

export function wantedItemSearchNowId(wantedItemId: string): string {
  return selectorId("wanted-item-search-now", wantedItemId);
}

export function titleOverviewSearchButtonId(titleId: string): string {
  return selectorId("title-overview-search", titleId);
}

export function titleOverviewInteractiveSearchButtonId(
  titleId: string,
): string {
  return selectorId("title-overview-interactive-search", titleId);
}

export function titleOverviewInteractiveSearchPanelId(titleId: string): string {
  return selectorId("title-overview-interactive-search-panel", titleId);
}

export function titleOverviewOpenButtonId(titleId: string): string {
  return selectorId("title-overview-open", titleId);
}

export function titleOverviewDeleteButtonId(titleId: string): string {
  return selectorId("title-overview-delete", titleId);
}

export function titleOverviewViewModeId(view: string, mode: string): string {
  return selectorId("title-overview-view-mode", view, mode);
}

function normalizedEpisodeSelectorKey(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  const season = Number.parseInt(String(seasonNumber ?? "").trim(), 10);
  const episode = Number.parseInt(String(episodeNumber ?? "").trim(), 10);
  if (Number.isFinite(season) && season > 0 && Number.isFinite(episode) && episode > 0) {
    return selectorId(facet, `s${String(season).padStart(2, "0")}e${String(episode).padStart(2, "0")}`);
  }

  const absolute = Number.parseInt(String(absoluteNumber ?? "").trim(), 10);
  if (Number.isFinite(absolute) && absolute > 0) {
    return selectorId(facet, `abs${String(absolute).padStart(3, "0")}`);
  }

  return selectorId(facet, "episode");
}

function normalizedSeasonSelectorKey(
  seasonNumber: string | number | null | undefined,
): string {
  const season = Number.parseInt(String(seasonNumber ?? "").trim(), 10);
  if (Number.isFinite(season) && season >= 0) {
    return `s${String(season).padStart(2, "0")}`;
  }

  return selectorId("season", seasonNumber ?? "");
}

export function seriesOverviewEpisodeRowId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
}

export function seriesOverviewEpisodeAutoSearchId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode-auto-search",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
}

export function seriesOverviewEpisodeInteractiveSearchId(
  facet: string,
  seasonNumber: string | number | null | undefined,
  episodeNumber: string | number | null | undefined,
  absoluteNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-episode-interactive-search",
    normalizedEpisodeSelectorKey(facet, seasonNumber, episodeNumber, absoluteNumber),
  );
}

export function seriesOverviewSeriesMovieRowId(seriesMovieLinkId: string): string {
  return selectorId("series-overview-series-movie", seriesMovieLinkId);
}

export function seriesOverviewSeriesMovieInteractiveSearchId(seriesMovieLinkId: string): string {
  return selectorId("series-overview-series-movie-interactive-search", seriesMovieLinkId);
}

export function seriesOverviewSeriesMovieAutoSearchId(seriesMovieLinkId: string): string {
  return selectorId("series-overview-series-movie-auto-search", seriesMovieLinkId);
}

export function seriesOverviewSeasonMonitorId(collectionId: string): string {
  return selectorId("series-overview-season-monitor", collectionId);
}

export function seriesOverviewSeasonToggleId(
  seasonNumber: string | number | null | undefined,
): string {
  return selectorId(
    "series-overview-season-toggle",
    normalizedSeasonSelectorKey(seasonNumber),
  );
}

export function seriesOverviewSeasonSectionId(collectionId: string): string {
  return selectorId("series-overview-season-section", collectionId);
}

export function seriesOverviewSeasonSearchId(collectionId: string): string {
  return selectorId("series-overview-season-search", collectionId);
}

export function seriesOverviewSeasonSelectId(collectionId: string): string {
  return selectorId("series-overview-season-select", collectionId);
}

export function seriesOverviewEpisodeSelectId(episodeId: string): string {
  return selectorId("series-overview-episode-select", episodeId);
}

export const SERIES_OVERVIEW_DELETE_SELECTED_EPISODES_ID =
  "series-overview-delete-selected-episodes";

export const SERIES_OVERVIEW_CLEAR_EPISODE_SELECTION_ID =
  "series-overview-clear-episode-selection";

export function globalSearchConfigureAddId(
  facet: string,
  result: MetadataSearchSelectorInput,
): string {
  return selectorId(
    "global-search-configure-add",
    facet,
    ...metadataSearchSelectorParts(result),
  );
}

export function globalSearchRequestId(
  facet: string,
  result: MetadataSearchSelectorInput,
): string {
  return selectorId(
    "global-search-request",
    facet,
    ...metadataSearchSelectorParts(result),
  );
}

export function mediaRequestRowId(requestId: string): string {
  return selectorId("media-request-row", requestId);
}

export function mediaRequestStatusId(requestId: string): string {
  return selectorId("media-request-status", requestId);
}

export function mediaRequestApproveId(requestId: string): string {
  return selectorId("media-request-approve", requestId);
}

export function mediaRequestDismissId(requestId: string): string {
  return selectorId("media-request-dismiss", requestId);
}

export function mediaRequestEditId(requestId: string): string {
  return selectorId("media-request-edit", requestId);
}

export function mediaRequestCancelId(requestId: string): string {
  return selectorId("media-request-cancel", requestId);
}

export function mediaRequestProfileOptionId(scope: string, profileId: string): string {
  return selectorId(scope, "media-request-profile-option", profileId);
}

export function mediaRequestMonitorSelectionId(requestId: string): string {
  return selectorId("media-request-monitor-selection", requestId);
}

export function mediaRequestMonitorOptionId(scope: string, monitorType: string): string {
  return selectorId(scope, "media-request-monitor-option", monitorType);
}

type ReleaseSearchSelectorInput = {
  source?: string | null;
  title?: string | null;
  link?: string | null;
  downloadUrl?: string | null;
};

type ReleaseSearchResultIdVariant = "mobile";

function stableShortHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(36).padStart(7, "0").slice(-7);
}

function releaseSearchResultSelectorParts(
  release: ReleaseSearchSelectorInput,
): string[] {
  const source = release.source?.trim() || "unknown";
  const title = release.title?.trim() || "untitled";
  const identity = [
    release.source ?? "",
    release.title ?? "",
    release.link ?? "",
    release.downloadUrl ?? "",
  ].join("|");
  return [source, title, stableShortHash(identity)];
}

export function releaseSearchResultRowId(
  release: ReleaseSearchSelectorInput,
  variant?: ReleaseSearchResultIdVariant,
): string {
  return selectorId(
    "release-search-result-row",
    variant,
    ...releaseSearchResultSelectorParts(release),
  );
}

export function releaseSearchResultQueueId(
  release: ReleaseSearchSelectorInput,
  variant?: ReleaseSearchResultIdVariant,
): string {
  return selectorId(
    "release-search-result-queue",
    variant,
    ...releaseSearchResultSelectorParts(release),
  );
}

/**
 * The line explaining why Queue is unavailable for a result. Separate from the
 * button so a check can assert the reason itself rather than matching its text.
 */
export function releaseSearchResultQueueReasonId(
  release: ReleaseSearchSelectorInput,
  variant?: ReleaseSearchResultIdVariant,
): string {
  return selectorId(
    "release-search-result-queue-reason",
    variant,
    ...releaseSearchResultSelectorParts(release),
  );
}

export function releaseSearchResultQueueAdditionalId(
  release: ReleaseSearchSelectorInput,
  variant?: ReleaseSearchResultIdVariant,
): string {
  return selectorId(
    "release-search-result-queue-additional",
    variant,
    ...releaseSearchResultSelectorParts(release),
  );
}

export function selectorId(
  ...parts: Array<string | number | false | null | undefined>
): string {
  return parts
    .filter(
      (part): part is string | number =>
        part !== false &&
        part !== null &&
        part !== undefined &&
        String(part).trim().length > 0,
    )
    .map((part) => selectorToken(part))
    .join("-");
}
