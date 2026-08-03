
import * as React from "react";
import { FileVideo, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
  quality: string | null;
  parsedSeason: number | null;
  parsedEpisodes: number[];
  suggestedEpisodeId: string | null;
  suggestedEpisodeLabel: string | null;
};

type AvailableEpisode = {
  id: string;
  titleId: string;
  collectionId: string | null;
  episodeType: string;
  episodeNumber: string | null;
  seasonNumber: string | null;
  episodeLabel: string | null;
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
  const season = ep.seasonNumber?.replace(/\D/g, "") ?? "?";
  const epNum = ep.episodeNumber?.replace(/\D/g, "") ?? "?";
  const tag = `S${season.padStart(2, "0")}E${epNum.padStart(2, "0")}`;
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
  return groups;
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

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId: string;
  titleName: string;
  clientId?: string;
  clientType: string;
  downloadClientItemId: string;
  onImportComplete?: () => void;
};

export function ManualImportDialog({
  open,
  onOpenChange,
  titleId,
  titleName,
  clientId,
  clientType,
  downloadClientItemId,
  onImportComplete,
}: Props) {
  const client = useClient();
  const navigate = useNavigate();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [archivePluginRequired, setArchivePluginRequired] = React.useState(false);
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

  // Load preview when dialog opens
  React.useEffect(() => {
    if (!open) {
      setFiles([]);
      setEpisodes([]);
      setSeriesMovies([]);
      setMappings({});
      setSelectionId(null);
      setError(null);
      setArchivePluginRequired(false);
      return;
    }

    setLoading(true);
    setError(null);
    setArchivePluginRequired(false);
    client.mutation(beginManualImportSelectionMutation, {
      input: {
        clientId,
        clientType,
        downloadClientItemId,
        titleId,
      },
    }).toPromise()
      .then(({ data, error: queryError }) => {
        if (queryError) throw queryError;
        const preview = data.beginManualImportSelection;
        setSelectionId(preview.selectionId);
        setFiles(preview.files);
        setEpisodes(preview.availableEpisodes);
        setSeriesMovies(preview.availableSeriesMovies ?? []);
        // Initialize mappings from suggested matches
        const initial: Record<string, string> = {};
        for (const file of preview.files) {
          initial[file.candidateId] = file.suggestedEpisodeId
            ? episodeTargetValue(file.suggestedEpisodeId)
            : UNASSIGNED;
        }
        setMappings(initial);
      })
      .catch((err: unknown) => {
        setImportError(err, "Failed to load preview");
      })
      .finally(() => setLoading(false));
  }, [open, clientId, clientType, downloadClientItemId, titleId, client, setImportError]);

  const groupedEpisodes = React.useMemo(() => groupEpisodesBySeason(episodes), [episodes]);

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
      onImportComplete?.();
      onOpenChange(false);
    } catch (err: unknown) {
      setImportError(err, "Import failed");
    } finally {
      setImporting(false);
    }
  }, [
    client,
    mappings,
    onImportComplete,
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
            <span className="text-sm text-muted-foreground">Scanning files...</span>
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
              <p
                id="activity-manual-import-empty"
                className="py-8 text-center text-sm text-muted-foreground"
              >
                No video files found in the download.
              </p>
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
                        <div className="flex items-center gap-2">
                          <FileVideo className="h-4 w-4 shrink-0 text-muted-foreground/60" />
                          <span className="max-w-[280px] truncate font-[var(--font-code)] text-xs text-card-foreground" title={file.fileName}>
                            {file.fileName}
                          </span>
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
                        <Select
                          value={mappings[file.candidateId] ?? UNASSIGNED}
                          onValueChange={(value) =>
                            setMappings((prev) => ({ ...prev, [file.candidateId]: value }))
                          }
                        >
                          <SelectTrigger
                            id={selectorId("activity-manual-import-assign", file.candidateId)}
                            className="h-8 w-full text-xs"
                          >
                            <SelectValue placeholder="Select target..." />
                          </SelectTrigger>
                          <SelectContent className="max-h-[300px]">
                            <SelectItem
                              id={selectorId("activity-manual-import-skip-option")}
                              value={UNASSIGNED}
                            >
                              <span className="text-muted-foreground/60">Skip (unassigned)</span>
                            </SelectItem>
                            {seriesMovies.length > 0 && (
                              <>
                                <div className="px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                                  Series movies
                                </div>
                                {seriesMovies.map((movie) => (
                                  <SelectItem
                                    key={movie.seriesMovieLinkId}
                                    id={selectorId(
                                      "activity-manual-import-series-movie-option",
                                      movie.seriesMovieLinkId,
                                    )}
                                    value={seriesMovieTargetValue(movie.seriesMovieLinkId)}
                                  >
                                    {seriesMovieLabel(movie)}
                                  </SelectItem>
                                ))}
                              </>
                            )}
                            {Array.from(groupedEpisodes.entries()).map(([seasonLabel, eps]) => (
                              <React.Fragment key={seasonLabel}>
                                <div className="px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                                  {seasonLabel}
                                </div>
                                {eps.map((ep) => (
                                  <SelectItem
                                    key={ep.id}
                                    id={selectorId("activity-manual-import-episode-option", ep.id)}
                                    value={episodeTargetValue(ep.id)}
                                  >
                                    {episodeLabel(ep)}
                                  </SelectItem>
                                ))}
                              </React.Fragment>
                            ))}
                          </SelectContent>
                        </Select>
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
