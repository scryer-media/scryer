
import * as React from "react";
import { Check, ChevronsUpDown, FileVideo, Loader2, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  MediaInfoBadges,
  type MediaInfoFile,
} from "@/components/common/media-info-badges";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { hasGraphQlErrorCode } from "@/lib/graphql/error-message";
import {
  beginManualImportSelectionMutation,
  queueManualImportMutation,
} from "@/lib/graphql/mutations";
import { selectorId } from "@/lib/utils/dom-ids";
import { compareManualImportSeasonLabels } from "@/lib/utils/manual-import-actions";
import { type ManualImportVideoFacts } from "@/lib/utils/manual-import-video-facts";
import { buildViewPath } from "@/lib/utils/routing";
import { useNavigate } from "react-router";
import { useClient } from "urql";

const ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_CODE = "ARCHIVE_EXTRACTION_PLUGIN_REQUIRED";
const ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_MESSAGE = [
  "This import is blocked because the download contains archive files.",
  "Install, update, or enable the Archive Extraction plugin, then re-import.",
].join(" ");

type FilePreview = {
  candidateId: string;
  fileName: string;
  sizeBytes: number;
  videoFacts: ManualImportVideoFacts | null;
  quality: string | null;
  parsedSeason: number | null;
  parsedEpisodes: number[];
  suggestedEpisodeId: string | null;
  suggestedEpisodeLabel: string | null;
  suggestedSeriesMovieLinkId: string | null;
};

function mediaInfoFileForManualImport(facts: ManualImportVideoFacts): MediaInfoFile {
  return {
    scanStatus: "scanned",
    videoCodec: facts.videoCodec,
    videoWidth: facts.videoWidth,
    videoHeight: facts.videoHeight,
    videoBitrateKbps: null,
    videoBitDepth: null,
    videoHdrFormat: null,
    videoFrameRate: null,
    videoProfile: null,
    audioCodec: facts.audioCodec,
    audioChannels: null,
    audioBitrateKbps: null,
    audioLanguages: [],
    audioStreams: facts.audioCodec
      ? [{ codec: facts.audioCodec, channels: null, language: null, bitrateKbps: null }]
      : [],
    subtitleLanguages: [],
    subtitleCodecs: [],
    subtitleStreams: [],
    hasMultiaudio: false,
    durationSeconds: facts.durationSeconds,
    numChapters: null,
    containerFormat: facts.containerFormat,
  };
}

type AvailableEpisode = {
  id: string;
  titleId: string;
  collectionId: string | null;
  episodeType: string;
  episodeNumber: string | null;
  seasonNumber: string | null;
  absoluteNumber: string | null;
  title: string | null;
  monitored: boolean;
};

type AvailableSeriesMovie = {
  seriesMovieLinkId: string;
  movieTitle: string;
  year: number | null;
  runtimeMinutes: number | null;
};

type ManualImportFileMapping = {
  candidateId: string;
  episodeId?: string;
  seriesMovieLinkId?: string;
};

function formatFileSize(bytes: number) {
  if (bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function episodeLabel(ep: AvailableEpisode): string {
  const season = ep.seasonNumber?.replace(/\D/g, "") ?? "";
  const episode = ep.episodeNumber?.replace(/\D/g, "") ?? "";
  const seasonTag = season ? season.padStart(2, "0") : "??";
  const episodeTag = episode ? episode.padStart(2, "0") : "??";
  const tag = `S${seasonTag}E${episodeTag}`;
  const absolute = ep.absoluteNumber?.trim();
  if (absolute) {
    return `${tag} (${absolute})${ep.title ? ` ${ep.title}` : ""}`;
  }
  return ep.title ? `${tag} - ${ep.title}` : tag;
}

function seriesMovieLabel(movie: AvailableSeriesMovie): string {
  const year = movie.year ? ` (${movie.year})` : "";
  const runtime = movie.runtimeMinutes ? ` • ${movie.runtimeMinutes} min` : "";
  return `${movie.movieTitle}${year}${runtime}`;
}

function groupEpisodesBySeason(episodes: AvailableEpisode[]): Map<string, AvailableEpisode[]> {
  const groups = new Map<string, AvailableEpisode[]>();
  for (const ep of episodes) {
    const season = ep.seasonNumber?.replace(/\D/g, "") ?? "0";
    const key = `Season ${season.padStart(2, "0")}`;
    const group = groups.get(key) ?? [];
    group.push(ep);
    groups.set(key, group);
  }
  // Sort episodes within each season
  for (const [key, group] of groups) {
    groups.set(
      key,
      group.sort((a, b) => {
        const aNum = Number.parseInt(a.episodeNumber?.replace(/\D/g, "") ?? "0", 10);
        const bNum = Number.parseInt(b.episodeNumber?.replace(/\D/g, "") ?? "0", 10);
        return aNum - bNum;
      }),
    );
  }
  return new Map(
    Array.from(groups.entries()).sort(([left], [right]) =>
      compareManualImportSeasonLabels(left, right),
    ),
  );
}

const UNASSIGNED = "__unassigned__";
const EPISODE_TARGET_PREFIX = "episode:";
const SERIES_MOVIE_TARGET_PREFIX = "series-movie:";

function episodeTargetValue(episodeId: string): string {
  return `${EPISODE_TARGET_PREFIX}${episodeId}`;
}

function seriesMovieTargetValue(seriesMovieLinkId: string): string {
  return `${SERIES_MOVIE_TARGET_PREFIX}${seriesMovieLinkId}`;
}

type ManualImportTargetSelectProps = {
  candidateId: string;
  value: string;
  groupedEpisodes: ReadonlyMap<string, AvailableEpisode[]>;
  seriesMovies: readonly AvailableSeriesMovie[];
  targetLabels: ReadonlyMap<string, string>;
  onChange: (candidateId: string, value: string) => void;
};

type ManualImportTargetListRow =
  | { kind: "group"; key: string; label: string }
  | { kind: "option"; key: string; label: string; value: string };

function buildManualImportTargetRows(
  groupedEpisodes: ReadonlyMap<string, AvailableEpisode[]>,
  seriesMovies: readonly AvailableSeriesMovie[],
  query: string,
): ManualImportTargetListRow[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const matches = (label: string, groupLabel = "") =>
    normalizedQuery.length === 0 ||
    `${groupLabel} ${label}`.toLocaleLowerCase().includes(normalizedQuery);
  const rows: ManualImportTargetListRow[] = [];

  const unassignedLabel = "Skip (unassigned)";
  if (matches(unassignedLabel)) {
    rows.push({
      kind: "option",
      key: UNASSIGNED,
      label: unassignedLabel,
      value: UNASSIGNED,
    });
  }

  const matchingSeriesMovies = seriesMovies.filter((movie) =>
    matches(seriesMovieLabel(movie), "Series movies"),
  );
  if (matchingSeriesMovies.length > 0) {
    rows.push({ kind: "group", key: "group:series-movies", label: "Series movies" });
    matchingSeriesMovies.forEach((movie) => {
      const value = seriesMovieTargetValue(movie.seriesMovieLinkId);
      rows.push({ kind: "option", key: value, label: seriesMovieLabel(movie), value });
    });
  }

  groupedEpisodes.forEach((episodesInSeason, seasonLabel) => {
    const matchingEpisodes = episodesInSeason.filter((episode) =>
      matches(episodeLabel(episode), seasonLabel),
    );
    if (matchingEpisodes.length === 0) {
      return;
    }
    rows.push({ kind: "group", key: `group:${seasonLabel}`, label: seasonLabel });
    matchingEpisodes.forEach((episode) => {
      const value = episodeTargetValue(episode.id);
      rows.push({ kind: "option", key: value, label: episodeLabel(episode), value });
    });
  });

  return rows;
}

function ManualImportTargetPickerContent({
  value,
  groupedEpisodes,
  seriesMovies,
  onSelect,
}: {
  value: string;
  groupedEpisodes: ReadonlyMap<string, AvailableEpisode[]>;
  seriesMovies: readonly AvailableSeriesMovie[];
  onSelect: (value: string) => void;
}) {
  const [query, setQuery] = React.useState("");
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const rows = React.useMemo(
    () => buildManualImportTargetRows(groupedEpisodes, seriesMovies, query),
    [groupedEpisodes, query, seriesMovies],
  );
  React.useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [query]);

  return (
    <PopoverContent
      align="start"
      className="z-[90] w-[var(--radix-popover-trigger-width)] min-w-[280px] p-2"
    >
      <div className="relative mb-2">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter episodes..."
          aria-label="Filter import targets"
          className="h-8 pl-8 text-xs"
        />
      </div>
      {rows.length === 0 ? (
        <div className="px-2 py-6 text-center text-xs text-muted-foreground">
          No matching targets.
        </div>
      ) : (
        <div
          ref={scrollRef}
          role="listbox"
          aria-label="Import target"
          className="h-[280px] overflow-y-auto overscroll-contain"
        >
          <div className="w-full">
            {rows.map((row) => (
              <div key={row.key}>
                {row.kind === "group" ? (
                    <div className="px-2 pt-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                      {row.label}
                    </div>
                  ) : (
                    <button
                      type="button"
                      role="option"
                      aria-selected={row.value === value}
                      id={
                        row.value === UNASSIGNED
                          ? selectorId("activity-manual-import-skip-option")
                          : row.value.startsWith(SERIES_MOVIE_TARGET_PREFIX)
                            ? selectorId(
                                "activity-manual-import-series-movie-option",
                                row.value.slice(SERIES_MOVIE_TARGET_PREFIX.length),
                              )
                            : selectorId(
                                "activity-manual-import-episode-option",
                                row.value.slice(EPISODE_TARGET_PREFIX.length),
                              )
                      }
                      className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left text-xs text-foreground hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
                      onClick={() => onSelect(row.value)}
                    >
                      <Check
                        className={`h-3.5 w-3.5 shrink-0 ${
                          row.value === value ? "opacity-100" : "opacity-0"
                        }`}
                      />
                      <span className="truncate">{row.label}</span>
                    </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </PopoverContent>
  );
}

const ManualImportTargetSelect = React.memo(function ManualImportTargetSelect({
  candidateId,
  value,
  groupedEpisodes,
  seriesMovies,
  targetLabels,
  onChange,
}: ManualImportTargetSelectProps) {
  const [open, setOpen] = React.useState(false);

  return (
    <Popover modal open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          id={selectorId("activity-manual-import-assign", candidateId)}
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="h-8 w-full justify-between gap-2 px-3 text-xs font-normal"
        >
          <span className="truncate">
            {targetLabels.get(value) ?? "Select target..."}
          </span>
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 opacity-60" />
        </Button>
      </PopoverTrigger>
      {open ? (
        <ManualImportTargetPickerContent
          value={value}
          groupedEpisodes={groupedEpisodes}
          seriesMovies={seriesMovies}
          onSelect={(nextValue) => {
            onChange(candidateId, nextValue);
            setOpen(false);
          }}
        />
      ) : null}
    </Popover>
  );
});

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId: string;
  titleName: string;
  clientId: string;
  clientType: string;
  downloadClientItemId: string;
  onImportQueued?: () => void;
};

export function ManualImportDialog({
  open,
  onOpenChange,
  titleId,
  titleName,
  clientId,
  clientType,
  downloadClientItemId,
  onImportQueued,
}: Props) {
  const client = useClient();
  const navigate = useNavigate();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [loading, setLoading] = React.useState(false);
  const [extractingArchives, setExtractingArchives] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [archivePluginRequired, setArchivePluginRequired] = React.useState(false);
  const [archiveExtractionNeeded, setArchiveExtractionNeeded] = React.useState(false);
  const [files, setFiles] = React.useState<FilePreview[]>([]);
  const [episodes, setEpisodes] = React.useState<AvailableEpisode[]>([]);
  const [seriesMovies, setSeriesMovies] = React.useState<AvailableSeriesMovie[]>([]);
  const [mappings, setMappings] = React.useState<Record<string, string>>({});
  const [selectionId, setSelectionId] = React.useState<string | null>(null);
  const [importing, setImporting] = React.useState(false);

  const setImportError = React.useCallback((err: unknown, fallback: string) => {
    if (hasGraphQlErrorCode(err, ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_CODE)) {
      setArchivePluginRequired(true);
      setError(ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_MESSAGE);
      return;
    }

    setArchivePluginRequired(false);
    setError(err instanceof Error ? err.message : fallback);
  }, []);

  const openArchivePluginSettings = React.useCallback(() => {
    onOpenChange(false);
    navigate(buildViewPath("settings", "downloadClients"));
  }, [navigate, onOpenChange]);

  const loadPreview = React.useCallback((extractArchives = false) => {
    setLoading(true);
    setExtractingArchives(extractArchives);
    setError(null);
    setArchivePluginRequired(false);
    return client.mutation(beginManualImportSelectionMutation, {
      input: {
        clientId,
        clientType,
        downloadClientItemId,
        titleId,
        extractArchives,
      },
    }).toPromise()
      .then(({ data, error: queryError }) => {
        if (queryError) throw queryError;
        const preview = data.beginManualImportSelection;
        setSelectionId(preview.selectionId);
        setFiles(preview.files);
        setEpisodes(preview.availableEpisodes);
        setSeriesMovies(preview.availableSeriesMovies ?? []);
        setArchiveExtractionNeeded(Boolean(preview.archiveExtractionNeeded));
        // Initialize mappings from suggested matches
        const initial: Record<string, string> = {};
        for (const file of preview.files) {
          initial[file.candidateId] = file.suggestedEpisodeId
            ? episodeTargetValue(file.suggestedEpisodeId)
            : file.suggestedSeriesMovieLinkId
              ? seriesMovieTargetValue(file.suggestedSeriesMovieLinkId)
              : UNASSIGNED;
        }
        setMappings(initial);
      })
      .catch((err: unknown) => {
        setArchiveExtractionNeeded(false);
        setImportError(err, "Failed to load preview");
      })
      .finally(() => {
        setLoading(false);
        setExtractingArchives(false);
      });
  }, [client, clientId, clientType, downloadClientItemId, setImportError, titleId]);

  // Load preview when dialog opens.
  React.useEffect(() => {
    if (!open) {
      setFiles([]);
      setEpisodes([]);
      setSeriesMovies([]);
      setMappings({});
      setSelectionId(null);
      setError(null);
      setArchivePluginRequired(false);
      setArchiveExtractionNeeded(false);
      setExtractingArchives(false);
      return;
    }

    void loadPreview();
  }, [loadPreview, open]);

  const groupedEpisodes = React.useMemo(() => groupEpisodesBySeason(episodes), [episodes]);
  const targetLabels = React.useMemo(() => {
    const labels = new Map<string, string>([[UNASSIGNED, "Skip (unassigned)"]]);
    seriesMovies.forEach((movie) => {
      labels.set(
        seriesMovieTargetValue(movie.seriesMovieLinkId),
        seriesMovieLabel(movie),
      );
    });
    episodes.forEach((episode) => {
      labels.set(episodeTargetValue(episode.id), episodeLabel(episode));
    });
    return labels;
  }, [episodes, seriesMovies]);
  const handleMappingChange = React.useCallback((candidateId: string, value: string) => {
    setMappings((previous) => {
      if (previous[candidateId] === value) {
        return previous;
      }
      return { ...previous, [candidateId]: value };
    });
  }, []);

  const assignedCount = React.useMemo(
    () => Object.values(mappings).filter((v) => v !== UNASSIGNED).length,
    [mappings],
  );

  const handleImport = React.useCallback(async () => {
    const fileMappings = Object.entries(mappings)
      .filter(([, target]) => target !== UNASSIGNED)
      .flatMap<ManualImportFileMapping>(([candidateId, target]) => {
        if (target.startsWith(EPISODE_TARGET_PREFIX)) {
          return [{
            candidateId,
            episodeId: target.slice(EPISODE_TARGET_PREFIX.length),
          }];
        }
        if (target.startsWith(SERIES_MOVIE_TARGET_PREFIX)) {
          return [{
            candidateId,
            seriesMovieLinkId: target.slice(SERIES_MOVIE_TARGET_PREFIX.length),
          }];
        }
        return [];
      });

    if (fileMappings.length === 0 || !selectionId) return;

    setImporting(true);
    setArchivePluginRequired(false);
    try {
      const { error: mutationError } = await client.mutation(queueManualImportMutation, {
        input: {
          selectionId,
          files: fileMappings,
        },
      }).toPromise();
      if (mutationError) throw mutationError;
      setGlobalStatus(t("queue.manualImportQueued"));
      onImportQueued?.();
      onOpenChange(false);
    } catch (err: unknown) {
      setImportError(err, "Import failed");
    } finally {
      setImporting(false);
    }
  }, [
    client,
    mappings,
    onImportQueued,
    onOpenChange,
    setImportError,
    setGlobalStatus,
    t,
    selectionId,
  ]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="activity-manual-import-dialog"
        className="sm:max-w-4xl max-h-[85vh] overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>Manual Import</DialogTitle>
          <DialogDescription>
            Match files to episodes or series movies for {titleName}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div
            id="activity-manual-import-loading"
            className="flex items-center justify-center gap-3 py-12"
          >
            <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-accent-text)]" />
            <span className="text-sm text-muted-foreground">
              {extractingArchives
                ? "Extracting archives. This can take a while..."
                : "Scanning files..."}
            </span>
          </div>
        ) : error && files.length === 0 ? (
          <div
            id="activity-manual-import-error"
            className="py-8 text-center text-sm text-[var(--scry-danger-text-soft)]"
          >
            <p>{error}</p>
            {archivePluginRequired && (
              <Button
                id="activity-manual-import-open-archive-plugin-settings"
                variant="outline"
                className="mt-4"
                onClick={openArchivePluginSettings}
              >
                Open Download Clients settings
              </Button>
            )}
          </div>
        ) : (
          <>
            {files.length === 0 ? (
              archiveExtractionNeeded ? (
                <div
                  id="activity-manual-import-archive-extraction-needed"
                  className="flex flex-col items-center gap-3 py-8 text-center"
                >
                  <p className="text-sm text-muted-foreground">
                    This download contains archives. Extract them before mapping media files.
                  </p>
                  <Button
                    id="activity-manual-import-extract-archives"
                    onClick={() => void loadPreview(true)}
                  >
                    Extract archives &amp; preview
                  </Button>
                </div>
              ) : (
                <p
                  id="activity-manual-import-empty"
                  className="py-8 text-center text-sm text-muted-foreground"
                >
                  The video files in this download do not have a recognized extension.
                </p>
              )
            ) : (
              <Table id="activity-manual-import-table">
                <TableHeader>
                  <TableRow>
                    <TableHead>File</TableHead>
                    <TableHead className="w-24 text-right">Size</TableHead>
                    <TableHead className="w-24 text-center">Quality</TableHead>
                    <TableHead className="w-[280px]">Target</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {files.map((file) => (
                    <TableRow
                      key={file.candidateId}
                      id={selectorId("activity-manual-import-file-row", file.candidateId)}
                    >
                      <TableCell>
                        <div className="flex items-start gap-2">
                          <FileVideo className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground/60" />
                          <div className="min-w-0">
                            <span className="block max-w-[280px] truncate font-[var(--font-code)] text-xs text-card-foreground" title={file.fileName}>
                              {file.fileName}
                            </span>
                            {file.videoFacts ? (
                              <div className="mt-1">
                                <MediaInfoBadges
                                  file={mediaInfoFileForManualImport(file.videoFacts)}
                                  includeContainer
                                />
                              </div>
                            ) : null}
                          </div>
                        </div>
                      </TableCell>
                      <TableCell className="text-right font-[var(--font-code)] text-xs text-muted-foreground">
                        {formatFileSize(file.sizeBytes)}
                      </TableCell>
                      <TableCell className="text-center">
                        {file.quality ? (
                          <Badge tone="info" className="px-1.5 text-[10px]">
                            {file.quality}
                          </Badge>
                        ) : (
                          <span className="text-xs text-muted-foreground/60">—</span>
                        )}
                      </TableCell>
                      <TableCell>
                        <ManualImportTargetSelect
                          candidateId={file.candidateId}
                          value={mappings[file.candidateId] ?? UNASSIGNED}
                          groupedEpisodes={groupedEpisodes}
                          seriesMovies={seriesMovies}
                          targetLabels={targetLabels}
                          onChange={handleMappingChange}
                        />
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
            {error && (
              <div
                id="activity-manual-import-error"
                className="text-sm text-[var(--scry-danger-text-soft)]"
              >
                <p>{error}</p>
                {archivePluginRequired && (
                  <Button
                    id="activity-manual-import-open-archive-plugin-settings"
                    variant="outline"
                    className="mt-3"
                    onClick={openArchivePluginSettings}
                  >
                    Open Download Clients settings
                  </Button>
                )}
              </div>
            )}
          </>
        )}

        <DialogFooter>
          <Button
            id="activity-manual-import-cancel"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={importing}
          >
            Cancel
          </Button>
          <Button
            id="activity-manual-import-queue"
            onClick={() => void handleImport()}
            disabled={importing || assignedCount === 0 || loading}
          >
            {importing ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Queueing...
              </>
            ) : (
              `Queue ${assignedCount} file${assignedCount === 1 ? "" : "s"}`
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
