import { userFacingGraphQlErrorMessage } from "../../lib/graphql/error-message.ts";

import * as React from "react";
import { ManualDiscSelectionControl } from "./manual-disc-selection";
import type { ManualDiscSelection } from "@/lib/utils/manual-import-video-facts";
import { FileVideo } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
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
import { type ManualImportVideoFacts } from "@/lib/utils/manual-import-video-facts";
import {
  EpisodeTargetSelect,
  type EpisodeTargetPickerLabels,
} from "@/components/common/episode-target-picker";
import {
  EPISODE_TARGET_PREFIX,
  SERIES_MOVIE_TARGET_PREFIX,
  UNASSIGNED_TARGET,
  episodeTargetIds,
  episodeTargetLabel,
  episodeTargetValue,
  groupEpisodesBySeason,
  seriesMovieTargetLabel,
  seriesMovieTargetValue,
} from "@/lib/utils/episode-target";
import { buildViewPath } from "@/lib/utils/routing";
import { useNavigate } from "react-router";
import { useClient } from "urql";
import { LoadingMark } from "@/components/common/loading-mark";

const ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_CODE =
  "ARCHIVE_EXTRACTION_PLUGIN_REQUIRED";
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
  suggestedEpisodeIds: string[];
  suggestedEpisodeLabel: string | null;
  suggestedSeriesMovieLinkId: string | null;
};

function mediaInfoFileForManualImport(
  facts: ManualImportVideoFacts,
): MediaInfoFile {
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
      ? [
          {
            codec: facts.audioCodec,
            channels: null,
            language: null,
            bitrateKbps: null,
          },
        ]
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
  episodeIds?: string[];
  seriesMovieLinkId?: string;
  discSelection?: import("@/lib/utils/manual-import-video-facts").ManualDiscSelection;
};

function formatFileSize(bytes: number) {
  if (bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

// The target a file's preview suggests, or "unassigned" when it suggests none.
function suggestedTargetValue(file: FilePreview): string {
  if (file.fileName.toLowerCase().endsWith(".iso")) return UNASSIGNED;
  const suggestedEpisodeIds =
    file.suggestedEpisodeIds.length > 0
      ? file.suggestedEpisodeIds
      : file.suggestedEpisodeId
        ? [file.suggestedEpisodeId]
        : [];
  if (suggestedEpisodeIds.length > 0)
    return episodeTargetValue(suggestedEpisodeIds);
  if (file.suggestedSeriesMovieLinkId) {
    return seriesMovieTargetValue(file.suggestedSeriesMovieLinkId);
  }
  return UNASSIGNED;
}

const UNASSIGNED = UNASSIGNED_TARGET;

// The manual-import picker's copy and element ids are unchanged by the move
// to the shared picker.
const MANUAL_IMPORT_TARGET_LABELS: EpisodeTargetPickerLabels = {
  unassigned: "Skip (unassigned)",
  seriesMovies: "Series movies",
  seasonGroup: (groupKey) => groupKey,
  filterPlaceholder: "Filter episodes...",
  filterLabel: "Filter import targets",
  listLabel: "Import target",
  noMatches: "No matching targets.",
  selectPlaceholder: "Select target...",
};

type ManualImportTargetSelectProps = {
  candidateId: string;
  value: string;
  groupedEpisodes: ReadonlyMap<string, AvailableEpisode[]>;
  seriesMovies: readonly AvailableSeriesMovie[];
  targetLabels: ReadonlyMap<string, string>;
  onChange: (candidateId: string, value: string) => void;
};

const ManualImportTargetSelect = React.memo(function ManualImportTargetSelect({
  candidateId,
  onChange,
  ...rest
}: ManualImportTargetSelectProps) {
  const handleChange = React.useCallback(
    (value: string) => onChange(candidateId, value),
    [candidateId, onChange],
  );
  return (
    <EpisodeTargetSelect
      triggerId={selectorId("activity-manual-import-assign", candidateId)}
      idPrefix="activity-manual-import"
      labels={MANUAL_IMPORT_TARGET_LABELS}
      onChange={handleChange}
      {...rest}
    />
  );
});

type Props = {
  facet: string | null;
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
  facet,
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
  const [archivePluginRequired, setArchivePluginRequired] =
    React.useState(false);
  const [archiveExtractionNeeded, setArchiveExtractionNeeded] =
    React.useState(false);
  const [files, setFiles] = React.useState<FilePreview[]>([]);
  const [episodes, setEpisodes] = React.useState<AvailableEpisode[]>([]);
  const [seriesMovies, setSeriesMovies] = React.useState<
    AvailableSeriesMovie[]
  >([]);
  const [mappings, setMappings] = React.useState<Record<string, string>>({});
  const [discSelections, setDiscSelections] = React.useState<
    Record<string, ManualDiscSelection>
  >({});
  const [selectionId, setSelectionId] = React.useState<string | null>(null);
  const [importing, setImporting] = React.useState(false);

  const setImportError = React.useCallback(
    (err: unknown, fallback: string) => {
      if (hasGraphQlErrorCode(err, ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_CODE)) {
        setArchivePluginRequired(true);
        setError(ARCHIVE_EXTRACTION_PLUGIN_REQUIRED_MESSAGE);
        return;
      }

      setArchivePluginRequired(false);
      setError(userFacingGraphQlErrorMessage(err, fallback, t));
    },
    [t],
  );

  const openArchivePluginSettings = React.useCallback(() => {
    onOpenChange(false);
    navigate(buildViewPath("settings", "downloadClients"));
  }, [navigate, onOpenChange]);

  const loadPreview = React.useCallback(
    (extractArchives = false) => {
      setLoading(true);
      setExtractingArchives(extractArchives);
      setError(null);
      setArchivePluginRequired(false);
      return client
        .mutation(beginManualImportSelectionMutation, {
          input: {
            clientId,
            clientType,
            downloadClientItemId,
            titleId,
            extractArchives,
          },
        })
        .toPromise()
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
            initial[file.candidateId] = suggestedTargetValue(file);
          }
          setMappings(initial);
          setDiscSelections({});
        })
        .catch((err: unknown) => {
          setArchiveExtractionNeeded(false);
          setImportError(err, "Failed to load preview");
        })
        .finally(() => {
          setLoading(false);
          setExtractingArchives(false);
        });
    },
    [
      client,
      clientId,
      clientType,
      downloadClientItemId,
      setImportError,
      titleId,
    ],
  );

  // Load preview when dialog opens.
  React.useEffect(() => {
    if (!open) {
      setFiles([]);
      setEpisodes([]);
      setSeriesMovies([]);
      setMappings({});
      setDiscSelections({});
      setSelectionId(null);
      setError(null);
      setArchivePluginRequired(false);
      setArchiveExtractionNeeded(false);
      setExtractingArchives(false);
      return;
    }

    void loadPreview();
  }, [loadPreview, open]);

  const groupedEpisodes = React.useMemo(
    () => groupEpisodesBySeason(episodes),
    [episodes],
  );
  const targetLabels = React.useMemo(() => {
    const labels = new Map<string, string>([[UNASSIGNED, "Skip (unassigned)"]]);
    seriesMovies.forEach((movie) => {
      labels.set(
        seriesMovieTargetValue(movie.seriesMovieLinkId),
        seriesMovieTargetLabel(movie),
      );
    });
    episodes.forEach((episode) => {
      labels.set(
        episodeTargetValue([episode.id]),
        episodeTargetLabel(episode, t),
      );
    });
    return labels;
  }, [episodes, seriesMovies, t]);
  const handleMappingChange = React.useCallback(
    (candidateId: string, value: string) => {
      setMappings((previous) => {
        if (previous[candidateId] === value) {
          return previous;
        }
        return { ...previous, [candidateId]: value };
      });
    },
    [],
  );

  // One click fills every file with what the preview resolved for it — the
  // whole set for a file that holds several episodes. A disc the user already
  // mapped keeps its own selection.
  const applySuggestionsToAll = React.useCallback(() => {
    setMappings((previous) => {
      const next: Record<string, string> = { ...previous };
      for (const file of files) {
        if (discSelections[file.candidateId]) continue;
        next[file.candidateId] = suggestedTargetValue(file);
      }
      return next;
    });
  }, [discSelections, files]);

  const assignedCount = React.useMemo(
    () => Object.values(mappings).filter((v) => v !== UNASSIGNED).length,
    [mappings],
  );

  const handleImport = React.useCallback(async () => {
    const fileMappings = Object.entries(mappings)
      .filter(([, target]) => target !== UNASSIGNED)
      .flatMap<ManualImportFileMapping>(([candidateId, target]) => {
        if (target === "__disc__" && discSelections[candidateId]) {
          return [{ candidateId, discSelection: discSelections[candidateId] }];
        }
        if (target === "__movie__") return [{ candidateId }];
        if (target.startsWith(EPISODE_TARGET_PREFIX)) {
          const episodeIds = episodeTargetIds(target);
          return episodeIds.length > 0 ? [{ candidateId, episodeIds }] : [];
        }
        if (target.startsWith(SERIES_MOVIE_TARGET_PREFIX)) {
          return [
            {
              candidateId,
              seriesMovieLinkId: target.slice(
                SERIES_MOVIE_TARGET_PREFIX.length,
              ),
            },
          ];
        }
        return [];
      });

    if (fileMappings.length === 0 || !selectionId) return;

    setImporting(true);
    setArchivePluginRequired(false);
    try {
      const { error: mutationError } = await client
        .mutation(queueManualImportMutation, {
          input: {
            selectionId,
            files: fileMappings,
          },
        })
        .toPromise();
      if (mutationError) throw mutationError;
      setGlobalStatus(t("queue.manualImportQueued"), { level: "SUCCESS" });
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
    discSelections,
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
        className="sm:max-w-[67.2rem] max-h-[85vh] overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>{t("queue.manualImport")}</DialogTitle>
          <DialogDescription>
            {t("uiText.matchFilesToEpisodesOrSeriesMoviesFor")} {titleName}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div
            id="activity-manual-import-loading"
            className="flex items-center justify-center gap-3 py-12"
          >
            <LoadingMark className="h-5 w-5 text-[var(--scry-accent-text)]" />
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
                {t("uiText.openDownloadClientsSettings")}
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
                    {t(
                      "uiText.thisDownloadContainsArchivesExtractThemBeforeMappingMediaFiles",
                    )}
                  </p>
                  <Button
                    id="activity-manual-import-extract-archives"
                    onClick={() => void loadPreview(true)}
                  >
                    {t("uiText.extractArchivesPreview")}
                  </Button>
                </div>
              ) : (
                <p
                  id="activity-manual-import-empty"
                  className="py-8 text-center text-sm text-muted-foreground"
                >
                  {t(
                    "uiText.theVideoFilesInThisDownloadDoNotHaveARecognizedExtension",
                  )}
                </p>
              )
            ) : (
              <Table id="activity-manual-import-table" className="table-fixed">
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("move.transferFilePath")}</TableHead>
                    <TableHead className="w-24 text-right">
                      {t("title.table.size")}
                    </TableHead>
                    <TableHead className="w-24 text-center">
                      {t("facetSettings.quality")}
                    </TableHead>
                    <TableHead className="w-[280px]">
                      {t("system.auditTarget")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {files.map((file) => (
                    <TableRow
                      key={file.candidateId}
                      id={selectorId(
                        "activity-manual-import-file-row",
                        file.candidateId,
                      )}
                    >
                      <TableCell>
                        <div className="flex items-start gap-2">
                          <FileVideo className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground/60" />
                          <div className="min-w-0">
                            <span className="block break-all font-[var(--font-code)] text-xs text-card-foreground">
                              {file.fileName}
                            </span>
                            {file.videoFacts ? (
                              <div className="mt-1">
                                <MediaInfoBadges
                                  file={mediaInfoFileForManualImport(
                                    file.videoFacts,
                                  )}
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
                          <span className="text-xs text-muted-foreground/60">
                            —
                          </span>
                        )}
                      </TableCell>
                      <TableCell>
                        {file.fileName.toLowerCase().endsWith(".iso") ? (
                          <ManualDiscSelectionControl
                            disc={file.videoFacts?.disc ?? null}
                            report={file.videoFacts?.report}
                            isMovie={facet === "MOVIE"}
                            episodes={episodes.map((episode) => ({
                              id: episode.id,
                              label: episodeTargetLabel(episode, t),
                            }))}
                            value={discSelections[file.candidateId] ?? null}
                            onChange={(selection) => {
                              setDiscSelections((previous) => {
                                const next = { ...previous };
                                if (selection)
                                  next[file.candidateId] = selection;
                                else delete next[file.candidateId];
                                return next;
                              });
                              handleMappingChange(
                                file.candidateId,
                                selection ? "__disc__" : UNASSIGNED,
                              );
                            }}
                          />
                        ) : facet === "MOVIE" ? (
                          <select
                            aria-label={file.fileName}
                            className="rounded border bg-background p-2 text-sm"
                            value={mappings[file.candidateId] ?? UNASSIGNED}
                            onChange={(event) =>
                              handleMappingChange(
                                file.candidateId,
                                event.target.value,
                              )
                            }
                          >
                            <option value={UNASSIGNED}>
                              {t("mediaInfo.discSkip")}
                            </option>
                            <option value="__movie__">
                              {t("mediaInfo.discImportMovie")}
                            </option>
                          </select>
                        ) : (
                          <ManualImportTargetSelect
                            candidateId={file.candidateId}
                            value={mappings[file.candidateId] ?? UNASSIGNED}
                            groupedEpisodes={groupedEpisodes}
                            seriesMovies={seriesMovies}
                            targetLabels={targetLabels}
                            onChange={handleMappingChange}
                          />
                        )}
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
                    {t("uiText.openDownloadClientsSettings")}
                  </Button>
                )}
              </div>
            )}
          </>
        )}

        <DialogFooter>
          {files.length > 0 ? (
            <Button
              id="activity-manual-import-apply-suggestions"
              variant="outline"
              className="mr-auto"
              onClick={applySuggestionsToAll}
              disabled={importing || loading}
            >
              {t("queue.manualImportApplySuggestions")}
            </Button>
          ) : null}
          <Button
            id="activity-manual-import-cancel"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={importing}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="activity-manual-import-queue"
            onClick={() => void handleImport()}
            disabled={importing || assignedCount === 0 || loading}
          >
            {importing ? (
              <>
                <LoadingMark className="mr-2 h-4 w-4" />
                {t("uiText.queueing")}
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
