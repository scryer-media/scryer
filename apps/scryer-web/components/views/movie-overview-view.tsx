
import * as React from "react";
import { ArrowLeft, FolderOpen, Loader2, Search, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  QUALITY_PROFILE_PREFIX,
  ROOT_FOLDER_PREFIX,
  getTagValue,
  setTagValue,
  removeTagByPrefix,
} from "@/lib/utils/title-tags";
import { useTranslate } from "@/lib/context/translate-context";
import type { Translate } from "@/components/root/types";
import type { Release } from "@/lib/types";
import type {
  MediaRenamePlan,
  TitleReleaseBlocklistEntry,
  TitleDetail,
  TitleCollection,
  TitleEvent,
  TitleMediaFile,
} from "@/components/containers/movie-overview-container";
import { MediaInfoBadges } from "@/components/common/media-info-badges";

const imdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/imdb.svg`;

// ─── helpers ────────────────────────────────────────────────────────────────

function formatDate(iso: string) {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  } catch {
    return iso;
  }
}

function formatDateTime(iso: string) {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function formatRuntime(minutes: number | null | undefined) {
  if (!minutes || minutes <= 0) return null;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

function prettifyTagValue(raw: string) {
  const trimmed = raw.trim();
  if (!trimmed) return trimmed;
  if (trimmed.toLowerCase() === "4k") return "4K";
  return trimmed;
}

function resolveMonitorTypeLabel(t: Translate, value: string) {
  switch (value) {
    case "monitored":
      return t("search.monitorType.monitored");
    case "unmonitored":
      return t("search.monitorType.unmonitored");
    case "futureEpisodes":
      return t("search.monitorType.futureEpisodes");
    case "missingAndFutureEpisodes":
      return t("search.monitorType.missingAndFutureEpisodes");
    case "allEpisodes":
      return t("search.monitorType.allEpisodes");
    case "none":
      return t("search.monitorType.none");
    default:
      return value;
  }
}

function formatTitleTag(t: Translate, tag: string) {
  const qualityPrefix = "scryer:quality-profile:";
  const monitorPrefix = "scryer:monitor-type:";
  const seasonFolderPrefix = "scryer:season-folder:";

  if (tag.startsWith(qualityPrefix)) {
    const value = prettifyTagValue(tag.slice(qualityPrefix.length));
    return {
      label: `${t("settings.qualityProfileSection")}: ${value}`,
      className: "bg-indigo-500/20 text-indigo-200",
    };
  }

  if (tag.startsWith(monitorPrefix)) {
    const value = tag.slice(monitorPrefix.length).trim();
    return {
      label: `${t("search.addConfigMonitorType")}: ${resolveMonitorTypeLabel(t, value)}`,
      className: "bg-sky-500/20 text-sky-200",
    };
  }

  if (tag.startsWith(seasonFolderPrefix)) {
    const value = tag.slice(seasonFolderPrefix.length).trim();
    const translatedValue =
      value === "enabled"
        ? t("search.seasonFolder.enabled")
        : value === "disabled"
          ? t("search.seasonFolder.disabled")
          : value;
    return {
      label: `${t("search.addConfigSeasonFolder")}: ${translatedValue}`,
      className: "bg-emerald-500/20 text-emerald-700 dark:text-emerald-200",
    };
  }

  return {
    label: tag,
    className: "bg-accent text-muted-foreground",
  };
}

const MONITOR_TYPE_TAG_PREFIX = "scryer:monitor-type:";

// ─── title settings ──────────────────────────────────────────────────────────

const INHERIT_VALUE = "__inherit__";

function TitleSettingsPanel({
  title,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
}: {
  title: TitleDetail;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  onUpdateTitleTags: (newTags: string[]) => Promise<void>;
}) {
  const t = useTranslate();
  const currentProfileId = getTagValue(title.tags, QUALITY_PROFILE_PREFIX) ?? INHERIT_VALUE;
  const currentRootFolder = getTagValue(title.tags, ROOT_FOLDER_PREFIX) ?? "";
  const [rootFolderDraft, setRootFolderDraft] = React.useState(currentRootFolder || defaultRootFolder);
  const [saving, setSaving] = React.useState(false);

  // Sync draft when title changes externally
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
      // Reset to default — remove tag
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

  return (
    <div className="p-4">
      <div className="flex flex-wrap items-end gap-4">
        {/* Quality Profile */}
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

        {/* Root Folder */}
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
      </div>
    </div>
  );
}

// ─── main view ────────────────────────────────────────────────────────────────

type Props = {
  loading: boolean;
  title: TitleDetail | null;
  collections: TitleCollection[];
  events: TitleEvent[];
  searchResults: Release[];
  searching: boolean;
  renamePlan: MediaRenamePlan | null;
  renamePreviewing: boolean;
  renameApplying: boolean;
  onSearch: () => void;
  onQueue: (r: Release) => void;
  onScanLibrary: () => void;
  onPreviewRename: () => void;
  onApplyRename: () => void;
  onBackToList?: () => void;
  qualityProfiles: { id: string; name: string }[];
  defaultRootFolder: string;
  onUpdateTitleTags: (newTags: string[]) => Promise<void>;
  blocklistEntries: TitleReleaseBlocklistEntry[];
  mediaFiles: TitleMediaFile[];
};

export function MovieOverviewView({
  loading,
  title,
  collections,
  events,
  searchResults,
  searching,
  renamePlan,
  renamePreviewing,
  renameApplying,
  onSearch,
  onQueue,
  onScanLibrary,
  onPreviewRename,
  onApplyRename,
  onBackToList,
  qualityProfiles,
  defaultRootFolder,
  onUpdateTitleTags,
  blocklistEntries,
  mediaFiles,
}: Props) {
  const t = useTranslate();
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
          <ArrowLeft className="h-4 w-4" /> Back to Movies
        </button>
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">Title not found.</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const imdbId = title.imdbId ?? title.externalIds.find((e) => e.source === "imdb")?.value;

  const posterUrl = title.posterUrl;
  const overview = title.overview;
  const genres = title.genres ?? [];
  const runtime = formatRuntime(title.runtimeMinutes);
  const year = title.year;
  const studio = title.studio;

  return (
    <div className="space-y-4">
      {/* back nav */}
      <button
        type="button"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        onClick={() => onBackToList?.()}
      >
        <ArrowLeft className="h-4 w-4" /> Back to Movies
      </button>

      {/* title header with poster */}
      <Card>
        <CardContent className="p-4">
          <div className="flex gap-5">
            {/* poster */}
            <div className="shrink-0">
              {posterUrl ? (
                <img
                  src={posterUrl}
                  alt={title.name}
                  className="h-auto w-[180px] rounded-lg object-cover shadow-lg block"
                />
              ) : (
                <div className="flex h-[270px] w-[180px] items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60">
                  No Poster
                </div>
              )}
            </div>

            {/* title info */}
            <div className="min-w-0 flex-1">
              <h1 className="text-2xl font-bold text-foreground">
                {title.name}
                {year ? <span className="ml-2 text-lg font-normal text-muted-foreground">({year})</span> : null}
              </h1>

              {/* meta chips */}
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${title.monitored ? "bg-emerald-500/20 text-emerald-700 dark:text-emerald-300" : "bg-accent text-muted-foreground"}`}>
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
                  <span className="text-xs text-muted-foreground">{runtime}</span>
                ) : null}
                {studio ? (
                  <span className="text-xs text-muted-foreground">{studio}</span>
                ) : null}
                {title.tags
                  .filter((tag) => !tag.startsWith(MONITOR_TYPE_TAG_PREFIX))
                  .map((tag) => {
                  const formattedTag = formatTitleTag(t, tag);
                  return (
                    <span
                      key={tag}
                      className={`inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium ${formattedTag.className}`}
                    >
                      {formattedTag.label}
                    </span>
                  );
                })}
              </div>

              {/* genres */}
              {genres.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {genres.map((genre) => (
                    <span key={genre} className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                      {genre}
                    </span>
                  ))}
                </div>
              ) : null}

              {overview ? (
                <p className="mt-4 text-sm leading-relaxed text-muted-foreground">{overview}</p>
              ) : null}

              {/* external IDs */}
              <div className="mt-3 flex flex-wrap gap-3 text-sm">
                {imdbId ? (
                  <a
                    href={
                      imdbId.startsWith("tt")
                        ? `https://www.imdb.com/title/${imdbId}`
                        : `https://www.imdb.com/find?q=${encodeURIComponent(imdbId)}&s=tt`
                    }
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex h-12 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-base hover:bg-muted"
                    aria-label="Open on IMDb"
                  >
                    <img src={imdbLogoUrl} alt="IMDb" className="h-8 w-8" />
                    <span className="text-muted-foreground">IMDb</span>
                  </a>
                ) : null}
                {title.externalIds
                  .filter((e) => e.source !== "imdb" && e.source !== "tvdb")
                  .map((e) => (
                    <div key={e.source}>
                      <span className="text-muted-foreground capitalize">{e.source} </span>
                      <span className="font-mono text-card-foreground">{e.value}</span>
                    </div>
                  ))}
              </div>

              <p className="mt-2 text-right text-xs text-muted-foreground/60">Added {formatDate(title.createdAt)}</p>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* files on disk */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between gap-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <FolderOpen className="h-4 w-4" />
              Files on Disk
            </CardTitle>
            <Button
              size="sm"
              variant="secondary"
              onClick={onPreviewRename}
              disabled={renamePreviewing || collections.length === 0}
            >
              {renamePreviewing ? t("rename.previewing") : t("rename.previewButton")}
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {collections.length === 0 ? (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">No files tracked. Run a library scan to detect files on disk.</p>
              <Button size="sm" onClick={onScanLibrary}>
                {t("settings.libraryScanButton")}
              </Button>
            </div>
          ) : (
            <div className="space-y-2">
              {collections.map((col) => {
                const qualityHint = col.label ?? col.collectionIndex ?? null;
                const mediaFile = mediaFiles.find((f) => f.filePath === col.orderedPath) ?? null;
                return (
                  <div key={col.id} className="rounded-lg border border-border p-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 space-y-1.5">
                        {col.orderedPath ? (
                          <p className="truncate font-mono text-xs text-muted-foreground">{col.orderedPath}</p>
                        ) : (
                          <p className="text-sm text-muted-foreground">Path not recorded</p>
                        )}
                        <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                          <span className="capitalize">{col.collectionType}</span>
                          {qualityHint ? (
                            <span className="rounded bg-accent px-1 py-0.5 text-card-foreground">{qualityHint}</span>
                          ) : null}
                          <span className="text-muted-foreground/60">Added {formatDate(col.createdAt)}</span>
                          {mediaFile?.acquisitionScore != null ? (
                            <span className="text-muted-foreground/60" title={mediaFile.scoringLog ?? undefined}>
                              {t("mediaFile.score", { score: mediaFile.acquisitionScore })}
                            </span>
                          ) : null}
                        </div>
                        {mediaFile ? <MediaInfoBadges file={mediaFile} /> : null}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {renamePlan ? (
            <div className="mt-5 space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2 text-sm text-muted-foreground">
                <div>
                  {t("rename.planSummary", {
                    total: renamePlan.total,
                    renamable: renamePlan.renamable,
                    noop: renamePlan.noop,
                    conflicts: renamePlan.conflicts,
                    errors: renamePlan.errors,
                  })}
                </div>
                <code className="rounded bg-card px-2 py-1 text-xs text-muted-foreground">
                  {renamePlan.fingerprint.slice(0, 16)}
                </code>
              </div>
              <div className="max-h-72 overflow-auto rounded-lg border border-border">
                <table className="min-w-full text-sm">
                  <thead className="bg-card/70 text-muted-foreground">
                    <tr>
                      <th className="px-3 py-2 text-left font-medium">Current</th>
                      <th className="px-3 py-2 text-left font-medium">Proposed</th>
                      <th className="px-3 py-2 text-left font-medium">Action</th>
                      <th className="px-3 py-2 text-left font-medium">Reason</th>
                    </tr>
                  </thead>
                  <tbody>
                    {renamePlan.items.map((item) => (
                      <tr key={`${item.collectionId ?? "none"}-${item.currentPath ?? ""}`} className="border-t border-border">
                        <td className="px-3 py-2 align-top font-mono text-xs text-muted-foreground">
                          {item.currentPath || "—"}
                        </td>
                        <td className="px-3 py-2 align-top font-mono text-xs text-muted-foreground">
                          {item.proposedPath ?? "—"}
                        </td>
                        <td className="px-3 py-2 align-top text-xs text-card-foreground">{item.writeAction}</td>
                        <td className="px-3 py-2 align-top text-xs text-muted-foreground">{item.reasonCode}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="flex justify-end">
                <Button
                  size="sm"
                  onClick={onApplyRename}
                  disabled={renameApplying || renamePlan.renamable === 0}
                >
                  {renameApplying ? t("rename.applying") : t("rename.applyButton")}
                </Button>
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>

      {/* indexer search */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between gap-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <Search className="h-4 w-4" />
              Search Indexers
            </CardTitle>
            <Button size="sm" onClick={onSearch} disabled={searching}>
              {searching ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Searching&hellip;
                </>
              ) : "Search Now"}
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {searchResults.length === 0 && !searching ? (
            <p className="text-sm text-muted-foreground">
              Click &ldquo;Search Now&rdquo; to query your configured indexers for releases of <span className="text-foreground">{title.name}</span>.
            </p>
          ) : searching ? (
            <div className="flex flex-col items-center gap-4 py-8">
              <Loader2 className="h-8 w-8 animate-spin text-emerald-500" />
              <p className="text-sm text-muted-foreground">Searching indexers for releases&hellip;</p>
              <div className="w-full space-y-2">
                {[1, 2, 3].map((n) => (
                  <div key={n} className="h-12 animate-pulse rounded-lg bg-muted" style={{ animationDelay: `${n * 150}ms` }} />
                ))}
              </div>
            </div>
          ) : (
            <SearchResultBuckets
              results={searchResults}
              onQueue={onQueue}
            />
          )}
        </CardContent>
      </Card>

      <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
          <span className="inline-flex items-center gap-2">
            Blocked Releases
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {blocklistEntries.length}
            </span>
          </span>
        </summary>
        <div className="border-t border-border p-4">
          {blocklistEntries.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No blocked releases recorded for this movie.
            </p>
          ) : (
            <div className="space-y-2">
              {blocklistEntries.map((entry) => (
                <div
                  key={`${entry.sourceHint ?? ""}-${entry.attemptedAt}-${entry.sourceTitle ?? ""}`}
                  className="rounded-lg border border-border bg-background/30 p-3"
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
                    <span className="text-muted-foreground/60">{formatDateTime(entry.attemptedAt)}</span>
                    {entry.errorMessage ? (
                      <span className="rounded bg-red-950/40 px-2 py-0.5 text-red-200">
                        {entry.errorMessage}
                      </span>
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </details>

      {/* title settings */}
      <details className="rounded-xl border border-border bg-card text-card-foreground overflow-hidden">
        <summary className="cursor-pointer select-none px-4 py-3 text-sm font-medium text-card-foreground">
          <span className="inline-flex items-center gap-2">
            <Settings2 className="h-4 w-4" />
            {t("title.settings")}
          </span>
        </summary>
        <div className="border-t border-border">
          <TitleSettingsPanel
            title={title}
            qualityProfiles={qualityProfiles}
            defaultRootFolder={defaultRootFolder}
            onUpdateTitleTags={onUpdateTitleTags}
          />
        </div>
      </details>

      {/* recent activity */}
      {events.length > 0 ? (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Recent Activity</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-2">
              {events.map((ev) => (
                <div key={ev.id} className="flex items-start gap-3 text-sm">
                  <span className="shrink-0 text-xs text-muted-foreground/60">{formatDate(ev.occurredAt)}</span>
                  <span className="capitalize text-muted-foreground">{ev.eventType.replace(/_/g, " ")}</span>
                  <span className="text-muted-foreground">{ev.message}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}
