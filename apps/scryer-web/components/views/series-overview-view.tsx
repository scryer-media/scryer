
import * as React from "react";
import { ArrowLeft, CalendarDays, ChevronDown, ChevronRight, Clapperboard, Clock3, ExternalLink, FileInput, Film, FolderOpen, HardDrive, Loader2, Search, Settings2, Star, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import { searchSeriesEpisodeQuery } from "@/lib/graphql/queries";
import { queueExistingMutation } from "@/lib/graphql/mutations";
import {
  QUALITY_PROFILE_PREFIX,
  ROOT_FOLDER_PREFIX,
  SEASON_FOLDER_PREFIX,
  getTagValue,
  setTagValue,
  removeTagByPrefix,
} from "@/lib/utils/title-tags";
import { useClient } from "urql";
import type { Release } from "@/lib/types";
import type { Translate } from "@/components/root/types";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
  TitleCollection,
  TitleDetail,
  TitleEvent,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import { AnimeMetadataPanel } from "@/components/views/anime-metadata-panel";
import type { DownloadQueueItem } from "@/lib/types/download-queue";

function formatDate(iso: string | null | undefined) {
  if (!iso) {
    return "—";
  }
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
}

function formatRuntimeFromMinutes(runtimeMinutes: number | null | undefined) {
  if (!runtimeMinutes || runtimeMinutes <= 0) {
    return null;
  }
  const hours = Math.floor(runtimeMinutes / 60);
  const minutes = runtimeMinutes % 60;
  if (hours === 0) {
    return `${minutes}m`;
  }
  return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
}

function formatFileSize(bytes: number) {
  if (bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

type EpisodePanelTab = "details" | "search" | "blocklist";

function parseSeasonSortValue(collection: TitleCollection) {
  const key = collection.narrativeOrder ?? collection.collectionIndex ?? "";
  const match = key.match(/\d+(\.\d+)?/);
  if (!match) {
    const fallback = `${collection.collectionIndex ?? ""} ${collection.label ?? ""}`;
    const fallbackMatch = fallback.match(/\d+/);
    return fallbackMatch ? Number.parseInt(fallbackMatch[0], 10) : Number.MAX_SAFE_INTEGER;
  }
  return Number.parseFloat(match[0]);
}

function isSpecialsCollection(collection: TitleCollection) {
  return collection.collectionType === "specials" || parseSeasonSortValue(collection) === 0;
}

function seasonHeading(collection: TitleCollection) {
  if (collection.collectionType === "interstitial") {
    return collection.label ?? "Movie";
  }
  if (collection.collectionType === "specials") {
    return collection.label ?? "Specials";
  }
  const indexValue = collection.collectionIndex.trim();
  const normalizedIndex = indexValue.match(/^\d+$/)
    ? indexValue === "0"
      ? "Specials"
      : `Season ${indexValue}`
    : indexValue;
  if (collection.label && collection.label.trim().length > 0) {
    return collection.label;
  }
  return normalizedIndex.length > 0 ? normalizedIndex : "Season";
}

function episodeSortValue(episode: CollectionEpisode) {
  if (!episode.episodeNumber) {
    return Number.MAX_SAFE_INTEGER;
  }
  const match = episode.episodeNumber.match(/\d+/);
  if (!match) {
    return Number.MAX_SAFE_INTEGER;
  }
  return Number.parseInt(match[0], 10);
}

function parseNumberToken(raw: string | null | undefined): number | null {
  const match = raw?.match(/\d+/);
  if (!match) {
    return null;
  }
  const value = Number.parseInt(match[0], 10);
  return Number.isFinite(value) ? value : null;
}

function episodeKey(season: number, episode: number): string {
  return `${season}-${episode}`;
}

function extractEpisodeKeysFromReleaseTitle(raw: string | null | undefined): Set<string> {
  if (!raw) {
    return new Set();
  }
  const title = raw.toUpperCase();
  const keys = new Set<string>();

  const seasonEpisodePattern = /S(\d{1,3})E(\d{1,4})(?:E(\d{1,4}))?/g;
  for (const match of title.matchAll(seasonEpisodePattern)) {
    const season = Number.parseInt(match[1], 10);
    const firstEpisode = Number.parseInt(match[2], 10);
    if (!Number.isFinite(season) || !Number.isFinite(firstEpisode)) {
      continue;
    }
    keys.add(episodeKey(season, firstEpisode));
    if (match[3]) {
      const secondEpisode = Number.parseInt(match[3], 10);
      if (Number.isFinite(secondEpisode)) {
        keys.add(episodeKey(season, secondEpisode));
      }
    }
  }

  const xPattern = /\b(\d{1,3})X(\d{1,4})(?:-(\d{1,4}))?\b/g;
  for (const match of title.matchAll(xPattern)) {
    const season = Number.parseInt(match[1], 10);
    const firstEpisode = Number.parseInt(match[2], 10);
    if (!Number.isFinite(season) || !Number.isFinite(firstEpisode)) {
      continue;
    }
    keys.add(episodeKey(season, firstEpisode));
    if (match[3]) {
      const secondEpisode = Number.parseInt(match[3], 10);
      if (Number.isFinite(secondEpisode)) {
        keys.add(episodeKey(season, secondEpisode));
      }
    }
  }

  return keys;
}

function blocklistEntryMatchesEpisode(
  entry: TitleReleaseBlocklistEntry,
  episode: CollectionEpisode,
  collection: TitleCollection,
): boolean {
  const season = parseNumberToken(episode.seasonNumber) ?? parseNumberToken(collection.collectionIndex);
  const episodeNumber = parseNumberToken(episode.episodeNumber);
  if (season == null || episodeNumber == null) {
    return false;
  }
  const keys = extractEpisodeKeysFromReleaseTitle(entry.sourceTitle);
  return keys.has(episodeKey(season, episodeNumber));
}

/**
 * Sort DB collections: non-specials descending (newest first), specials (season 0) at the end.
 */
function sortDbCollections(collections: TitleCollection[]) {
  return [...collections].sort((left, right) => {
    const leftVal = parseSeasonSortValue(left);
    const rightVal = parseSeasonSortValue(right);
    if (leftVal === 0 && rightVal !== 0) return 1;
    if (rightVal === 0 && leftVal !== 0) return -1;
    if (leftVal !== rightVal) return rightVal - leftVal;
    return right.collectionIndex.localeCompare(left.collectionIndex);
  });
}

/**
 * Find the key of the most recent (highest-numbered, non-specials) season to auto-expand.
 */
function findLatestSeasonKey(collections: TitleCollection[]): string | null {
  if (collections.length === 0) return null;
  const nonSpecials = collections.filter((c) => !isSpecialsCollection(c));
  if (nonSpecials.length === 0) return null;
  const latest = nonSpecials.reduce((best, current) =>
    parseSeasonSortValue(current) > parseSeasonSortValue(best)
      ? current
      : best,
  );
  return `s-${latest.id}`;
}

// ─── title settings ──────────────────────────────────────────────────────────

const INHERIT_VALUE = "__inherit__";

function TitleSettingsPanel({
  t,
  title,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
}: {
  t: Translate;
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  onUpdateTitleTags: (newTags: string[]) => Promise<void>;
}) {
  const currentProfileId = getTagValue(title.tags, QUALITY_PROFILE_PREFIX) ?? INHERIT_VALUE;
  const currentRootFolder = getTagValue(title.tags, ROOT_FOLDER_PREFIX) ?? "";
  const currentSeasonFolder = getTagValue(title.tags, SEASON_FOLDER_PREFIX) ?? "enabled";
  const [rootFolderDraft, setRootFolderDraft] = React.useState(currentRootFolder || defaultRootFolder);
  const [saving, setSaving] = React.useState(false);

  React.useEffect(() => {
    setRootFolderDraft(currentRootFolder || defaultRootFolder);
  }, [currentRootFolder, defaultRootFolder]);

  const handleProfileChange = async (value: string) => {
    setSaving(true);
    try {
      const newTags =
        value === INHERIT_VALUE
          ? removeTagByPrefix(title.tags, QUALITY_PROFILE_PREFIX)
          : setTagValue(title.tags, QUALITY_PROFILE_PREFIX, value);
      await onUpdateTitleTags(newTags);
    } finally {
      setSaving(false);
    }
  };

  const handleRootFolderSave = async () => {
    const trimmed = rootFolderDraft.trim();
    if (!trimmed || trimmed === defaultRootFolder) {
      setSaving(true);
      try {
        await onUpdateTitleTags(removeTagByPrefix(title.tags, ROOT_FOLDER_PREFIX));
      } finally {
        setSaving(false);
      }
      return;
    }
    setSaving(true);
    try {
      await onUpdateTitleTags(setTagValue(title.tags, ROOT_FOLDER_PREFIX, trimmed));
    } finally {
      setSaving(false);
    }
  };

  const handleSeasonFolderChange = async (value: string) => {
    setSaving(true);
    try {
      await onUpdateTitleTags(setTagValue(title.tags, SEASON_FOLDER_PREFIX, value));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="p-4">
      <div className="flex flex-wrap items-end gap-4">
        <div className="min-w-[200px] flex-1">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.qualityProfile")}
          </label>
          <Select
            value={currentProfileId}
            onValueChange={(v) => void handleProfileChange(v)}
            disabled={saving || qualityProfiles.length === 0}
          >
            <SelectTrigger className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={INHERIT_VALUE}>
                {t("title.inheritDefault")}
              </SelectItem>
              {qualityProfiles.map((p) => (
                <SelectItem key={p.id} value={p.id}>
                  {p.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <div className="min-w-[280px] flex-[2]">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("title.rootFolder")}
          </label>
          <div className="flex gap-2">
            <Input
              className="h-9 font-mono text-sm"
              value={rootFolderDraft}
              onChange={(e) => setRootFolderDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void handleRootFolderSave();
              }}
              disabled={saving}
            />
            {rootFolderDraft.trim() !== (currentRootFolder || defaultRootFolder) && (
              <Button
                size="sm"
                className="h-9"
                onClick={() => void handleRootFolderSave()}
                disabled={saving}
              >
                {t("settings.saveButton")}
              </Button>
            )}
          </div>
        </div>

        <div className="min-w-[160px]">
          <label className="mb-1 block text-xs font-medium text-muted-foreground">
            {t("search.addConfigSeasonFolder")}
          </label>
          <Select
            value={currentSeasonFolder}
            onValueChange={(v) => void handleSeasonFolderChange(v)}
            disabled={saving}
          >
            <SelectTrigger className="h-9 w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
              <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
    </div>
  );
}

// ─── main view ───────────────────────────────────────────────────────────────

type Props = {
  t: Translate;
  loading: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleEvent[];
  episodesByCollection: Record<string, CollectionEpisode[]>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  setGlobalStatus: (status: string) => void;
  onTitleChanged?: () => Promise<void>;
  onBackToList?: () => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => Promise<void> | void;
  qualityProfiles?: { id: string; name: string }[];
  defaultRootFolder?: string;
  onUpdateTitleTags?: (newTags: string[]) => Promise<void>;
  completedDownloads?: DownloadQueueItem[];
  onOpenManualImport?: (item: DownloadQueueItem) => void;
};

export function SeriesOverviewView({
  t,
  loading,
  title,
  collections,
  events,
  episodesByCollection,
  mediaFilesByEpisode,
  releaseBlocklistEntries,
  setGlobalStatus,
  onTitleChanged,
  onBackToList,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  onAutoSearchEpisode,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
  completedDownloads,
  onOpenManualImport,
}: Props) {
  const client = useClient();
  const sortedCollections = React.useMemo(
    () => sortDbCollections(collections),
    [collections],
  );

  const latestKey = React.useMemo(
    () => findLatestSeasonKey(sortedCollections),
    [sortedCollections],
  );

  const [expandedKeys, setExpandedKeys] = React.useState<Set<string>>(new Set());

  // Initialize expanded state when data arrives
  const initializedRef = React.useRef(false);
  React.useEffect(() => {
    if (initializedRef.current) return;
    if (latestKey) {
      initializedRef.current = true;
      setExpandedKeys(new Set([latestKey]));
    }
  }, [latestKey]);

  const toggleKey = React.useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const [expandedEpisodeRows, setExpandedEpisodeRows] = React.useState<Set<string>>(new Set());
  const [episodeActiveTab, setEpisodeActiveTab] = React.useState<Record<string, EpisodePanelTab>>({});
  const [searchResultsByEpisode, setSearchResultsByEpisode] = React.useState<Record<string, Release[]>>({});
  const [searchLoadingByEpisode, setSearchLoadingByEpisode] = React.useState<Record<string, boolean>>({});
  const [autoSearchLoadingByEpisode, setAutoSearchLoadingByEpisode] = React.useState<
    Record<string, boolean>
  >({});

  const handleRunEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!title) return;
      const episodeId = episode.id;
      setSearchLoadingByEpisode((prev) => ({ ...prev, [episodeId]: true }));

      const tvdbId =
        title.externalIds
          ?.find((eid) => eid.source.toLowerCase() === "tvdb")
          ?.value?.trim() ?? "";
      const collection = collections.find((c) => c.id === episode.collectionId);
      const seasonNum = episode.seasonNumber?.trim().replace(/\D+/g, "")
        || collection?.collectionIndex?.trim().replace(/\D+/g, "")
        || "1";
      const episodeNum = episode.episodeNumber?.trim().replace(/\D+/g, "") || "1";

      client.query(searchSeriesEpisodeQuery, {
        title: title.name,
        season: seasonNum,
        episode: episodeNum,
        tvdbId,
        category: title.facet,
        limit: 25,
      }).toPromise()
        .then(({ data, error: queryError }) => {
          if (queryError) throw queryError;
          setSearchResultsByEpisode((prev) => ({
            ...prev,
            [episodeId]: data.searchIndexersEpisode ?? [],
          }));
        })
        .catch(() => {
          setSearchResultsByEpisode((prev) => ({
            ...prev,
            [episodeId]: [],
          }));
        })
        .finally(() => {
          setSearchLoadingByEpisode((prev) => {
            const next = { ...prev };
            delete next[episodeId];
            return next;
          });
        });
    },
    [client, title, collections],
  );

  const handleToggleEpisodeSearch = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = expandedEpisodeRows.has(episodeId);
      const currentTab = episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "search") {
        setExpandedEpisodeRows((prev) => {
          const next = new Set(prev);
          next.delete(episodeId);
          return next;
        });
      } else {
        setExpandedEpisodeRows((prev) => new Set(prev).add(episodeId));
        setEpisodeActiveTab((prev) => ({ ...prev, [episodeId]: "search" }));
        if (!Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episodeId)) {
          handleRunEpisodeSearch(episode);
        }
      }
    },
    [expandedEpisodeRows, episodeActiveTab, handleRunEpisodeSearch, searchResultsByEpisode],
  );

  const handleToggleEpisodeDetails = React.useCallback(
    (episode: CollectionEpisode) => {
      const episodeId = episode.id;
      const isOpen = expandedEpisodeRows.has(episodeId);
      const currentTab = episodeActiveTab[episodeId] ?? "details";

      if (isOpen && currentTab === "details") {
        setExpandedEpisodeRows((prev) => {
          const next = new Set(prev);
          next.delete(episodeId);
          return next;
        });
      } else {
        setExpandedEpisodeRows((prev) => new Set(prev).add(episodeId));
        setEpisodeActiveTab((prev) => ({ ...prev, [episodeId]: "details" }));
      }
    },
    [expandedEpisodeRows, episodeActiveTab],
  );

  const handleEpisodeTabChange = React.useCallback(
    (episodeId: string, tab: EpisodePanelTab, episode: CollectionEpisode) => {
      setEpisodeActiveTab((prev) => ({ ...prev, [episodeId]: tab }));
      if (tab === "search" && !Object.prototype.hasOwnProperty.call(searchResultsByEpisode, episodeId)) {
        handleRunEpisodeSearch(episode);
      }
    },
    [handleRunEpisodeSearch, searchResultsByEpisode],
  );

  const handleQueueFromEpisodeSearch = React.useCallback(
    (release: Release) => {
      if (!title) return Promise.resolve();
      if (release.qualityProfileDecision && release.qualityProfileDecision.allowed === false) {
        const reason = release.qualityProfileDecision.blockCodes.join(", ") || "unknown";
        setGlobalStatus(t("status.qualityProfileBlocked", { reason }));
        return Promise.resolve();
      }

      const sourceHint = release.downloadUrl || release.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return Promise.resolve();
      }

      return client.mutation(queueExistingMutation, {
        input: {
          titleId: title.id,
          sourceHint,
          sourceTitle: release.title,
        },
      }).toPromise()
        .then(async ({ error: mutationError }) => {
          if (mutationError) throw mutationError;
          const queuedMessage = t("status.queuedLatest", { name: title.name });
          setGlobalStatus(queuedMessage);
          await onTitleChanged?.();
        })
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        });
    },
    [onTitleChanged, client, setGlobalStatus, t, title],
  );

  const handleAutoSearchEpisode = React.useCallback(
    (episode: CollectionEpisode) => {
      if (!onAutoSearchEpisode) return;
      const episodeId = episode.id;
      setAutoSearchLoadingByEpisode((prev) => ({ ...prev, [episodeId]: true }));
      Promise.resolve(onAutoSearchEpisode(episode))
        .catch((error: unknown) => {
          setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        })
        .finally(() => {
          setAutoSearchLoadingByEpisode((prev) => {
            const next = { ...prev };
            delete next[episodeId];
            return next;
          });
        });
    },
    [onAutoSearchEpisode, setGlobalStatus, t],
  );

  if (loading) {
    return (
      <div className="space-y-4">
        <div className="h-8 w-48 animate-pulse rounded bg-muted" />
        <div className="h-32 animate-pulse rounded-lg bg-muted" />
        <div className="h-48 animate-pulse rounded-lg bg-muted" />
      </div>
    );
  }

  if (!title) {
    return (
      <div className="space-y-4">
        <button
          type="button"
          className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
          onClick={() => onBackToList?.()}
        >
          <ArrowLeft className="h-4 w-4" /> Back to {t("nav.series")}
        </button>
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">Title not found.</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const runtime = formatRuntimeFromMinutes(title.runtimeMinutes);

  return (
    <div className="space-y-4">
      <button
        type="button"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        onClick={() => onBackToList?.()}
      >
        <ArrowLeft className="h-4 w-4" /> Back to {t("nav.series")}
      </button>

      <Card>
        <CardContent className="p-4">
          <div className="flex gap-5">
            <div className="shrink-0">
              {title.posterUrl ? (
                <img
                  src={title.posterUrl}
                  alt={title.name}
                  className="h-auto w-[180px] rounded-lg object-cover shadow-lg block"
                />
              ) : (
                <div className="flex h-[270px] w-[180px] items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60">
                  No Poster
                </div>
              )}
            </div>

            <div className="min-w-0 flex-1">
              <h1 className="text-2xl font-bold text-foreground">
                {title.name}
                {title.year ? (
                  <span className="ml-2 text-lg font-normal text-muted-foreground">
                    ({title.year})
                  </span>
                ) : null}
              </h1>

              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span
                  className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${
                    title.monitored
                      ? "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300"
                      : "bg-accent text-muted-foreground"
                  }`}
                >
                  {title.monitored ? "Monitored" : "Unmonitored"}
                </span>
                <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                  {title.facet}
                </span>
                {title.contentStatus ? (
                  <span className="inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium capitalize text-muted-foreground">
                    {title.contentStatus}
                  </span>
                ) : null}
                {runtime ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Clock3 className="h-3.5 w-3.5" />
                    {runtime}
                  </span>
                ) : null}
                {title.network ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Clapperboard className="h-3.5 w-3.5" />
                    {title.network}
                  </span>
                ) : null}
                {title.facet === "anime" ? (
                  <>
                    {(() => { const e = title.externalIds.find((e) => e.source === "mal"); return e ? (
                      <a href={`https://myanimelist.net/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.malLink")}
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anilist"); return e ? (
                      <a href={`https://anilist.co/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.anilistLink")}
                      </a>
                    ) : null; })()}
                    {(() => { const e = title.externalIds.find((e) => e.source === "anidb"); return e ? (
                      <a href={`https://anidb.net/anime/${e.value}`} target="_blank" rel="noopener noreferrer" className="inline-flex items-center gap-1 text-xs text-primary hover:underline">
                        <ExternalLink className="h-3 w-3" />
                        {t("anime.anidbLink")}
                      </a>
                    ) : null; })()}
                  </>
                ) : null}
              </div>

              {title.genres.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {title.genres.map((genre) => (
                    <span
                      key={genre}
                      className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                    >
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              {title.overview ? (
                <p className="mt-4 text-sm leading-relaxed text-muted-foreground">
                  {title.overview}
                </p>
              ) : null}

              <p className="mt-2 text-right text-xs text-muted-foreground/60">
                Added {formatDate(title.createdAt)}
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      {title.facet === "anime" ? (
        <AnimeMetadataPanel
          tags={title.tags}
          episodesByCollection={episodesByCollection}
          t={t}
        />
      ) : null}

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="flex items-center gap-2 text-base">
              <FolderOpen className="h-4 w-4" />
              Seasons and Episodes
            </CardTitle>
            {onOpenManualImport && completedDownloads && completedDownloads.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => onOpenManualImport(completedDownloads[0])}
              >
                <FileInput className="mr-1.5 h-4 w-4" />
                Manual Import
              </Button>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {sortedCollections.length > 0 ? (
            sortedCollections.map((collection) => {
              const key = `s-${collection.id}`;
              const sortedEpisodes = [
                ...(episodesByCollection[collection.id] ?? []),
              ].sort((left, right) => episodeSortValue(right) - episodeSortValue(left));

              return (
                <SeasonSection
                  key={key}
                  collection={collection}
                  episodes={sortedEpisodes}
                  facet={title.facet}
                  expanded={expandedKeys.has(key)}
                  onToggle={() => toggleKey(key)}
                  expandedEpisodeRows={expandedEpisodeRows}
                  episodeActiveTab={episodeActiveTab}
                  mediaFilesByEpisode={mediaFilesByEpisode}
                  releaseBlocklistEntries={releaseBlocklistEntries}
                  searchResultsByEpisode={searchResultsByEpisode}
                  searchLoadingByEpisode={searchLoadingByEpisode}
                  autoSearchLoadingByEpisode={autoSearchLoadingByEpisode}
                  onToggleEpisodeSearch={handleToggleEpisodeSearch}
                  onToggleEpisodeDetails={handleToggleEpisodeDetails}
                  onEpisodeTabChange={handleEpisodeTabChange}
                  onRunEpisodeSearch={handleRunEpisodeSearch}
                  onQueueFromEpisodeSearch={handleQueueFromEpisodeSearch}
                  onAutoSearchEpisode={handleAutoSearchEpisode}
                  onSetCollectionMonitored={onSetCollectionMonitored}
                  onSetEpisodeMonitored={onSetEpisodeMonitored}
                  t={t}
                />
              );
            })
          ) : (
            <p className="text-sm text-muted-foreground">
              No seasons are tracked for this show yet.
            </p>
          )}
        </CardContent>
      </Card>

      {onUpdateTitleTags && qualityProfiles && defaultRootFolder ? (
        <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
          <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
            <span className="inline-flex items-center gap-2">
              <Settings2 className="h-4 w-4" />
              {t("title.settings")}
            </span>
          </summary>
          <div className="border-t border-border">
            <TitleSettingsPanel
              t={t}
              title={title}
              qualityProfiles={qualityProfiles}
              defaultRootFolder={defaultRootFolder}
              onUpdateTitleTags={onUpdateTitleTags}
            />
          </div>
        </details>
      ) : null}

      {events.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Recent Activity</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-2">
              {events.map((event) => (
                <div key={event.id} className="flex items-start gap-3 text-sm">
                  <span className="shrink-0 text-xs text-muted-foreground/60">
                    {formatDate(event.occurredAt)}
                  </span>
                  <span className="capitalize text-muted-foreground">
                    {event.eventType.replace(/_/g, " ")}
                  </span>
                  <span className="text-muted-foreground">{event.message}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

function SeasonSection({
  collection,
  episodes,
  expanded,
  facet,
  onToggle,
  expandedEpisodeRows,
  episodeActiveTab,
  mediaFilesByEpisode,
  releaseBlocklistEntries,
  searchResultsByEpisode,
  searchLoadingByEpisode,
  onToggleEpisodeSearch,
  onToggleEpisodeDetails,
  onEpisodeTabChange,
  onRunEpisodeSearch,
  onQueueFromEpisodeSearch,
  autoSearchLoadingByEpisode,
  onAutoSearchEpisode,
  onSetCollectionMonitored,
  onSetEpisodeMonitored,
  t,
}: {
  collection: TitleCollection;
  facet: string;
  episodes: CollectionEpisode[];
  expanded: boolean;
  onToggle: () => void;
  expandedEpisodeRows: Set<string>;
  episodeActiveTab: Record<string, EpisodePanelTab>;
  mediaFilesByEpisode: Record<string, EpisodeMediaFile[]>;
  releaseBlocklistEntries: TitleReleaseBlocklistEntry[];
  searchResultsByEpisode: Record<string, Release[]>;
  searchLoadingByEpisode: Record<string, boolean>;
  autoSearchLoadingByEpisode: Record<string, boolean>;
  onToggleEpisodeSearch: (episode: CollectionEpisode) => void;
  onToggleEpisodeDetails: (episode: CollectionEpisode) => void;
  onEpisodeTabChange: (episodeId: string, tab: EpisodePanelTab, episode: CollectionEpisode) => void;
  onRunEpisodeSearch: (episode: CollectionEpisode) => void;
  onQueueFromEpisodeSearch: (release: Release) => Promise<void> | void;
  onAutoSearchEpisode?: (episode: CollectionEpisode) => void;
  onSetCollectionMonitored?: (collectionId: string, monitored: boolean) => Promise<void>;
  onSetEpisodeMonitored?: (episodeId: string, monitored: boolean) => Promise<void>;
  t: Translate;
}) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const [seasonToggling, setSeasonToggling] = React.useState(false);
  const [episodeToggling, setEpisodeToggling] = React.useState<Set<string>>(new Set());

  const seasonCheckedState: boolean | "indeterminate" = React.useMemo(() => {
    if (episodes.length === 0) return collection.monitored;
    const monitoredCount = episodes.filter((ep) => ep.monitored).length;
    if (monitoredCount === 0) return false;
    if (monitoredCount === episodes.length) return true;
    return "indeterminate";
  }, [episodes, collection.monitored]);

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-background/40">
      <div
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onToggle();
          }
        }}
        className="flex w-full cursor-pointer flex-wrap items-center justify-between gap-3 bg-card/60 px-4 py-2 text-left transition hover:bg-accent/80"
      >
        <div className="flex items-center gap-2">
          {seasonToggling ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
          ) : (
            <Checkbox
              checked={seasonCheckedState}
              aria-label={t("title.seasonMonitored")}
              className="size-6 [&_svg]:size-4"
              onCheckedChange={() => {
                if (!onSetCollectionMonitored) return;
                setSeasonToggling(true);
                const nextMonitored = seasonCheckedState !== true;
                onSetCollectionMonitored(collection.id, nextMonitored)
                  .finally(() => setSeasonToggling(false));
              }}
              onClick={(e) => e.stopPropagation()}
            />
          )}
          <Chevron className="h-4 w-4 shrink-0 text-muted-foreground" />
          <div>
            <p className="text-sm font-semibold text-foreground">
              {seasonHeading(collection)}
            </p>
            {collection.firstEpisodeNumber || collection.lastEpisodeNumber ? (
              <p className="text-xs text-muted-foreground">
                Episodes {collection.firstEpisodeNumber ?? "?"} - {collection.lastEpisodeNumber ?? "?"}
              </p>
            ) : null}
          </div>
        </div>
        <span className="text-xs text-muted-foreground">
          {collection.collectionType === "interstitial" ? (
            <span className="inline-flex items-center gap-1">
              <Film className="h-3 w-3" />
              Movie
            </span>
          ) : isSpecialsCollection(collection) ? (
            <span className="inline-flex items-center gap-1">
              <Star className="h-3 w-3" />
              {episodes.length} special{episodes.length === 1 ? "" : "s"}
            </span>
          ) : (
            <>
              {episodes.length} episode
              {episodes.length === 1 ? "" : "s"}
            </>
          )}
        </span>
      </div>

      {expanded ? (
        collection.collectionType === "interstitial" ? (
          <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
            Canon movie installment. Positioned in narrative viewing order.
          </div>
        ) : episodes.length === 0 ? (
          <div className="border-t border-border px-4 py-3 text-sm text-muted-foreground">
            No episode records for this season.
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-10 text-center" />
                <TableHead className="w-28 text-center">Episode</TableHead>
                <TableHead>Title</TableHead>
                <TableHead className="w-40">Air Date</TableHead>
                <TableHead className="w-28 text-center">{t("episode.quality")}</TableHead>
                <TableHead className="w-20 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {episodes.map((episode) => {
                const isPanelOpen = expandedEpisodeRows.has(episode.id);
                const activeTab = episodeActiveTab[episode.id] ?? "details";
                const episodeResults = searchResultsByEpisode[episode.id] ?? [];
                const episodeLoading = searchLoadingByEpisode[episode.id] === true;
                const autoSearching = autoSearchLoadingByEpisode[episode.id] === true;
                const episodeFiles = mediaFilesByEpisode[episode.id] ?? [];

                return (
                  <React.Fragment key={episode.id}>
                    <TableRow className={`cv-auto-row-sm${episode.monitored ? "" : " opacity-50"}`}>
                      <TableCell className="px-2 text-center align-middle">
                        {episodeToggling.has(episode.id) ? (
                          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                        ) : (
                          <Checkbox
                            checked={episode.monitored}
                            aria-label={t("title.episodeMonitored")}
                            className="size-6 [&_svg]:size-4"
                            onCheckedChange={() => {
                              if (!onSetEpisodeMonitored) return;
                              setEpisodeToggling((prev) => new Set(prev).add(episode.id));
                              onSetEpisodeMonitored(episode.id, !episode.monitored)
                                .finally(() => {
                                  setEpisodeToggling((prev) => {
                                    const next = new Set(prev);
                                    next.delete(episode.id);
                                    return next;
                                  });
                                });
                            }}
                          />
                        )}
                      </TableCell>
                      <TableCell className="text-center align-middle font-mono text-sm text-card-foreground">
                        <div className="flex flex-col items-center gap-0.5">
                          <span>{episode.episodeNumber ?? episode.episodeLabel ?? "—"}</span>
                          {episode.absoluteNumber && facet === "anime" ? (
                            <span className="text-[10px] text-muted-foreground">
                              #{episode.absoluteNumber}
                            </span>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell
                        className="align-middle text-sm text-card-foreground cursor-pointer hover:text-foreground"
                        onClick={() => onToggleEpisodeDetails(episode)}
                      >
                        <div className="flex items-center gap-1.5">
                          <span>{episode.title || episode.episodeLabel || "—"}</span>
                          {episode.episodeType === "special" ? (
                            <span className="rounded border border-indigo-500/30 bg-indigo-500/15 px-1.5 py-0.5 text-[10px] font-medium text-indigo-700 dark:text-indigo-300">
                              {t("episode.special")}
                            </span>
                          ) : episode.episodeType === "ova" ? (
                            <span className="rounded border border-violet-500/30 bg-violet-500/15 px-1.5 py-0.5 text-[10px] font-medium text-violet-700 dark:text-violet-300">
                              {t("episode.ova")}
                            </span>
                          ) : episode.episodeType === "ona" ? (
                            <span className="rounded border border-emerald-500/30 bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                              {t("episode.ona")}
                            </span>
                          ) : episode.episodeType === "alternate" ? (
                            <span className="rounded border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium text-sky-700 dark:text-sky-300">
                              {t("episode.alternate")}
                            </span>
                          ) : null}
                          {episode.isFiller ? (
                            <span className="rounded border border-orange-500/30 bg-orange-500/15 px-1.5 py-0.5 text-[10px] font-medium text-orange-700 dark:text-orange-300">
                              {t("episode.filler")}
                            </span>
                          ) : null}
                          {episode.hasMultiAudio ? (
                            <span className="rounded border border-purple-500/30 bg-purple-500/15 px-1.5 py-0.5 text-[10px] font-medium text-purple-700 dark:text-purple-300">
                              {t("episode.multiAudio")}
                            </span>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        <span className="inline-flex items-center gap-1">
                          <CalendarDays className="h-3.5 w-3.5" />
                          {formatDate(episode.airDate)}
                        </span>
                      </TableCell>
                      <TableCell className="text-center">
                        {episodeFiles.length > 0 && episodeFiles[0].qualityLabel ? (
                          <span className="rounded border border-emerald-500/40 dark:border-emerald-500/30 bg-emerald-500/20 dark:bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                            {episodeFiles[0].qualityLabel}
                          </span>
                        ) : episode.monitored ? (
                          <span className="rounded border border-amber-500/30 bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-300">
                            {t("episode.missing")}
                          </span>
                        ) : null}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="inline-flex items-center justify-end gap-2">
                          {onAutoSearchEpisode ? (
                            <HoverCard openDelay={3000} closeDelay={75}>
                              <HoverCardTrigger asChild>
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  aria-label={t("label.search")}
                                  onClick={() => onAutoSearchEpisode?.(episode)}
                                  disabled={autoSearching}
                                >
                                  {autoSearching ? (
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                  ) : (
                                    <Zap className="h-4 w-4" />
                                  )}
                                </Button>
                              </HoverCardTrigger>
                              <HoverCardContent>
                                <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                  {t("help.autoSearchTooltip")}
                                </p>
                              </HoverCardContent>
                            </HoverCard>
                          ) : null}
                          <HoverCard openDelay={3000} closeDelay={75}>
                            <HoverCardTrigger asChild>
                              <Button
                                variant="ghost"
                                size="sm"
                                aria-label={t("label.search")}
                                onClick={() => onToggleEpisodeSearch(episode)}
                              >
                                <Search className="h-4 w-4" />
                              </Button>
                            </HoverCardTrigger>
                            <HoverCardContent>
                              <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                {t("help.interactiveSearchTooltip")}
                              </p>
                            </HoverCardContent>
                          </HoverCard>
                        </div>
                      </TableCell>
                    </TableRow>
                    {isPanelOpen ? (
                      <TableRow>
                        <TableCell colSpan={6} className="border-t border-border bg-background/40 p-0">
                          <div className="px-4 py-3">
                            <Tabs
                              value={activeTab}
                              onValueChange={(val) => onEpisodeTabChange(episode.id, val as EpisodePanelTab, episode)}
                            >
                              <TabsList>
                                <TabsTrigger value="details">{t("episode.details")}</TabsTrigger>
                                <TabsTrigger value="search">{t("episode.search")}</TabsTrigger>
                                <TabsTrigger value="blocklist">Blocklist</TabsTrigger>
                              </TabsList>
                              <TabsContent value="details">
                                <EpisodeDetailsPanel episode={episode} mediaFiles={episodeFiles} t={t} />
                              </TabsContent>
                              <TabsContent value="search">
                                <div className="mb-2 flex items-center justify-end">
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => onRunEpisodeSearch(episode)}
                                    disabled={episodeLoading}
                                    aria-label={t("label.search")}
                                  >
                                    <Search className="h-4 w-4" />
                                    <span className="ml-1">
                                      {episodeLoading ? t("label.searching") : t("label.refresh")}
                                    </span>
                                  </Button>
                                </div>
                                {episodeLoading ? (
                                  <div className="flex items-center gap-3 py-3">
                                    <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                                    <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                                  </div>
                                ) : episodeResults.length === 0 ? (
                                  <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
                                ) : (
                                  <SearchResultBuckets
                                    results={episodeResults}
                                    onQueue={onQueueFromEpisodeSearch}
                                    t={t}
                                  />
                                )}
                              </TabsContent>
                              <TabsContent value="blocklist">
                                <EpisodeBlocklistPanel entries={releaseBlocklistEntries.filter((entry) =>
                                  blocklistEntryMatchesEpisode(entry, episode, collection),
                                )} />
                              </TabsContent>
                            </Tabs>
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </React.Fragment>
                );
              })}
            </TableBody>
          </Table>
        )
      ) : null}
    </div>
  );
}

function EpisodeDetailsPanel({
  episode,
  mediaFiles,
  t,
}: {
  episode: CollectionEpisode;
  mediaFiles: EpisodeMediaFile[];
  t: Translate;
}) {
  return (
    <div className="space-y-3">
      {episode.overview ? (
        <div>
          <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.overview")}</p>
          <p className="text-sm leading-relaxed text-muted-foreground">{episode.overview}</p>
        </div>
      ) : null}
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.fileOnDisk")}</p>
        {mediaFiles.length > 0 ? (
          <div className="space-y-2">
            {mediaFiles.map((file) => (
              <div key={file.id} className="flex flex-wrap items-center gap-3 rounded bg-card/60 px-3 py-2 text-sm">
                <HardDrive className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
                <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{file.filePath}</span>
                {file.qualityLabel ? (
                  <span className="rounded border border-emerald-500/40 dark:border-emerald-500/30 bg-emerald-500/20 dark:bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                    {file.qualityLabel}
                  </span>
                ) : null}
                <span className="text-xs text-muted-foreground/60">{formatFileSize(Number(file.sizeBytes))}</span>
                <span className="text-xs text-muted-foreground/60">{formatDate(file.createdAt)}</span>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm italic text-muted-foreground/60">{t("episode.noFile")}</p>
        )}
      </div>
    </div>
  );
}

function EpisodeBlocklistPanel({
  entries,
}: {
  entries: TitleReleaseBlocklistEntry[];
}) {
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No blocked releases recorded for this episode.
      </p>
    );
  }

  return (
    <div className="space-y-2">
      {entries.map((entry, index) => (
        <div
          key={`${entry.sourceHint ?? "hint"}-${entry.sourceTitle ?? "title"}-${entry.attemptedAt}-${index}`}
          className="rounded-lg border border-border bg-background/35 p-3"
        >
          <p className="break-words text-sm text-card-foreground">
            {entry.sourceTitle || "Untitled release"}
          </p>
          {entry.sourceHint ? (
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground/60">
              {entry.sourceHint}
            </p>
          ) : null}
          <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
            <span className="text-muted-foreground/60">{formatDate(entry.attemptedAt)}</span>
            {entry.errorMessage ? (
              <span className="rounded bg-red-950/40 px-2 py-0.5 text-red-200">
                {entry.errorMessage}
              </span>
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}
