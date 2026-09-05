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

export function titleOverviewSelectId(titleId: string): string {
  return selectorId("title-overview-select", titleId);
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

// ── Request rules, leases, decisions and claims ──────────────────────────
//
// One helper per interactive element the request-rules surfaces add, so the
// e2e suite targets a name rather than a class or a position. Fixed ids that
// carry no entity are constants; anything keyed by a rule, request, user,
// template or claim is a function.

export const REQUEST_MEDIA_LEASE_ID = "request-media-lease";
export const REQUEST_MEDIA_LEASE_DAYS_ID = "request-media-lease-days";
export const REQUEST_PREFLIGHT_BANNER_ID = "request-preflight-banner";

export function requestMediaLeaseOptionId(value: string): string {
  return selectorId("request-media-lease-option", value);
}

export function mediaRequestLeaseId(requestId: string): string {
  return selectorId("media-request-lease", requestId);
}

export function mediaRequestDecisionId(requestId: string): string {
  return selectorId("media-request-decision", requestId);
}

export function mediaRequestDecisionPopoverId(requestId: string): string {
  return selectorId("media-request-decision-popover", requestId);
}

export function mediaRequestPolicyTagsId(requestId: string): string {
  return selectorId("media-request-policy-tags", requestId);
}

export function mediaRequestDenyReasonId(requestId: string): string {
  return selectorId("media-request-deny-reason", requestId);
}

export function mediaRequestClaimsToggleId(requestId: string): string {
  return selectorId("media-request-claims-toggle", requestId);
}

export function mediaRequestClaimsPanelId(requestId: string): string {
  return selectorId("media-request-claims-panel", requestId);
}

export const APPROVE_MEDIA_REQUEST_LEASE_ID = "approve-media-request-lease";
export const APPROVE_MEDIA_REQUEST_LEASE_DAYS_ID =
  "approve-media-request-lease-days";
/// Prefix the approve dialog hands `TitleTagsPicker`, which derives its own
/// ids from it: `<prefix>-tags` for the block, `<prefix>-tags-add` for the
/// registry select, and `<prefix>-tag-remove-<label>` for each chip's remove
/// button. The approver picks from the registry rather than typing, so the
/// free-text field these ids used to name no longer exists.
export const APPROVE_MEDIA_REQUEST_TAGS_PREFIX = "approve-media-request";
export const APPROVE_MEDIA_REQUEST_TAGS_ID = "approve-media-request-tags";
export const APPROVE_MEDIA_REQUEST_TAG_ADD_ID = "approve-media-request-tags-add";

export function approveMediaRequestLeaseOptionId(value: string): string {
  return selectorId("approve-media-request-lease-option", value);
}

/// Mirrors `TitleTagsPicker`'s own remove-button id exactly — the picker keeps
/// the label's case and only collapses whitespace, so this cannot go through
/// `selectorId`, which lowercases.
export function approveMediaRequestTagRemoveId(tag: string): string {
  return `${APPROVE_MEDIA_REQUEST_TAGS_PREFIX}-tag-remove-${tag.replace(/\s+/g, "-")}`;
}

export function titleClaimRowId(claimId: string): string {
  return selectorId("title-claim-row", claimId);
}

export function titleClaimExtendId(claimId: string): string {
  return selectorId("title-claim-extend", claimId);
}

export function titleClaimPermanentId(claimId: string): string {
  return selectorId("title-claim-permanent", claimId);
}

export function titleClaimReleaseId(claimId: string): string {
  return selectorId("title-claim-release", claimId);
}

export const TITLE_CLAIM_EXTEND_DATE_ID = "title-claim-extend-date";
export const TITLE_CLAIM_RELEASE_REASON_ID = "title-claim-release-reason";

export function settingsRequestRuleRowId(ruleSetId: string): string {
  return selectorId("settings-request-rule-row", ruleSetId);
}

export function settingsRequestRuleNameId(name: string): string {
  return selectorId("settings-request-rule-name", name);
}

export function settingsRequestRuleModeId(ruleSetId: string): string {
  return selectorId("settings-request-rule-mode", ruleSetId);
}

export function settingsRequestRuleCopyId(ruleSetId: string): string {
  return selectorId("settings-request-rule-copy", ruleSetId);
}

export function settingsRequestRuleEditId(ruleSetId: string): string {
  return selectorId("settings-request-rule-edit", ruleSetId);
}

export function settingsRequestRuleDeleteId(ruleSetId: string): string {
  return selectorId("settings-request-rule-delete", ruleSetId);
}

export function settingsRequestRuleLibraryId(libraryId: string): string {
  return selectorId("settings-request-rule-library", libraryId);
}

export function settingsRequestTemplateId(templateId: string): string {
  return selectorId("settings-request-template", templateId);
}

export function settingsRequestUserId(userId: string): string {
  return selectorId("settings-request-user", userId);
}

export function settingsRequestPreviewTitleResultId(key: string): string {
  return selectorId("settings-request-preview-title-result", key);
}

export function settingsRequestDecisionRowId(index: number): string {
  return selectorId("settings-request-decision-row", index);
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

/**
 * Rows of the Indexers › Search pane. The identity is the one the title-scoped
 * release rows already use, so the same release keeps the same key wherever it
 * is rendered — and a retry that returns it again merges onto its own row.
 */
export function indexerSearchResultRowId(
  release: ReleaseSearchSelectorInput,
): string {
  return selectorId(
    "indexer-search-row",
    ...releaseSearchResultSelectorParts(release),
  );
}

export function indexerSearchResultSelectId(
  release: ReleaseSearchSelectorInput,
): string {
  return selectorId(
    "indexer-search-select",
    ...releaseSearchResultSelectorParts(release),
  );
}

export function indexerSearchResultGrabId(
  release: ReleaseSearchSelectorInput,
): string {
  return selectorId(
    "indexer-search-grab",
    ...releaseSearchResultSelectorParts(release),
  );
}

/** The row's "download to my browser" button (D17). */
export function indexerSearchResultDownloadId(
  release: ReleaseSearchSelectorInput,
): string {
  return selectorId(
    "indexer-search-download",
    ...releaseSearchResultSelectorParts(release),
  );
}

export function indexerSearchResultExpandId(
  release: ReleaseSearchSelectorInput,
): string {
  return selectorId(
    "indexer-search-expand",
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
