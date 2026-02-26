
import * as React from "react";
import { CheckCircle2, FileVideo, Loader2, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
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
import { previewManualImportQuery } from "@/lib/graphql/queries";
import { executeManualImportMutation } from "@/lib/graphql/mutations";
import { useClient } from "urql";

type FilePreview = {
  filePath: string;
  fileName: string;
  sizeBytes: string;
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

type ImportResult = {
  filePath: string;
  episodeId: string;
  success: boolean;
  destPath: string | null;
  errorMessage: string | null;
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

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titleId: string;
  titleName: string;
  downloadClientItemId: string;
  onImportComplete?: () => void;
};

export function ManualImportDialog({
  open,
  onOpenChange,
  titleId,
  titleName,
  downloadClientItemId,
  onImportComplete,
}: Props) {
  const client = useClient();
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [files, setFiles] = React.useState<FilePreview[]>([]);
  const [episodes, setEpisodes] = React.useState<AvailableEpisode[]>([]);
  const [mappings, setMappings] = React.useState<Record<string, string>>({});
  const [importing, setImporting] = React.useState(false);
  const [results, setResults] = React.useState<ImportResult[] | null>(null);

  // Load preview when dialog opens
  React.useEffect(() => {
    if (!open) {
      setFiles([]);
      setEpisodes([]);
      setMappings({});
      setResults(null);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    client.query(previewManualImportQuery, {
      downloadClientItemId,
      titleId,
    }).toPromise()
      .then(({ data, error: queryError }) => {
        if (queryError) throw queryError;
        const preview = data.previewManualImport;
        setFiles(preview.files);
        setEpisodes(preview.availableEpisodes);
        // Initialize mappings from suggested matches
        const initial: Record<string, string> = {};
        for (const file of preview.files) {
          initial[file.filePath] = file.suggestedEpisodeId ?? UNASSIGNED;
        }
        setMappings(initial);
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : "Failed to load preview");
      })
      .finally(() => setLoading(false));
  }, [open, downloadClientItemId, titleId, client]);

  const groupedEpisodes = React.useMemo(() => groupEpisodesBySeason(episodes), [episodes]);

  const assignedCount = React.useMemo(
    () => Object.values(mappings).filter((v) => v !== UNASSIGNED).length,
    [mappings],
  );

  const handleImport = React.useCallback(async () => {
    const fileMappings = Object.entries(mappings)
      .filter(([, episodeId]) => episodeId !== UNASSIGNED)
      .map(([filePath, episodeId]) => ({ filePath, episodeId }));

    if (fileMappings.length === 0) return;

    setImporting(true);
    try {
      const { data, error: mutationError } = await client.mutation(executeManualImportMutation, {
        input: { titleId, files: fileMappings },
      }).toPromise();
      if (mutationError) throw mutationError;
      setResults(data.executeManualImport);
      onImportComplete?.();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setImporting(false);
    }
  }, [mappings, titleId, client, onImportComplete]);

  const successCount = results?.filter((r) => r.success).length ?? 0;
  const failCount = results?.filter((r) => !r.success).length ?? 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-4xl max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Manual Import</DialogTitle>
          <DialogDescription>
            Match files to episodes for {titleName}
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex items-center justify-center gap-3 py-12">
            <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
            <span className="text-sm text-muted-foreground">Scanning files...</span>
          </div>
        ) : error && files.length === 0 ? (
          <div className="py-8 text-center text-sm text-red-400">{error}</div>
        ) : results ? (
          <div className="space-y-3">
            <div className="flex items-center gap-3 text-sm">
              {successCount > 0 && (
                <span className="flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
                  <CheckCircle2 className="h-4 w-4" />
                  {successCount} imported
                </span>
              )}
              {failCount > 0 && (
                <span className="flex items-center gap-1 text-red-400">
                  <XCircle className="h-4 w-4" />
                  {failCount} failed
                </span>
              )}
            </div>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>File</TableHead>
                  <TableHead className="w-20 text-center">Status</TableHead>
                  <TableHead>Detail</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {results.map((result) => {
                  const fileName = result.filePath.split("/").pop() ?? result.filePath;
                  return (
                    <TableRow key={result.filePath}>
                      <TableCell className="max-w-[300px] truncate font-mono text-xs">
                        {fileName}
                      </TableCell>
                      <TableCell className="text-center">
                        {result.success ? (
                          <CheckCircle2 className="mx-auto h-4 w-4 text-emerald-600 dark:text-emerald-400" />
                        ) : (
                          <XCircle className="mx-auto h-4 w-4 text-red-400" />
                        )}
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {result.success
                          ? result.destPath?.split("/").pop() ?? "Imported"
                          : result.errorMessage ?? "Unknown error"}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>
        ) : (
          <>
            {files.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                No video files found in the download.
              </p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>File</TableHead>
                    <TableHead className="w-24 text-right">Size</TableHead>
                    <TableHead className="w-24 text-center">Quality</TableHead>
                    <TableHead className="w-[280px]">Episode</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {files.map((file) => (
                    <TableRow key={file.filePath}>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <FileVideo className="h-4 w-4 shrink-0 text-muted-foreground/60" />
                          <span className="max-w-[280px] truncate font-mono text-xs text-card-foreground" title={file.fileName}>
                            {file.fileName}
                          </span>
                        </div>
                      </TableCell>
                      <TableCell className="text-right text-xs text-muted-foreground">
                        {formatFileSize(Number(file.sizeBytes))}
                      </TableCell>
                      <TableCell className="text-center">
                        {file.quality ? (
                          <span className="rounded border border-blue-500/30 bg-blue-500/15 px-1.5 py-0.5 text-[10px] font-medium text-blue-300">
                            {file.quality}
                          </span>
                        ) : (
                          <span className="text-xs text-muted-foreground/60">—</span>
                        )}
                      </TableCell>
                      <TableCell>
                        <Select
                          value={mappings[file.filePath] ?? UNASSIGNED}
                          onValueChange={(value) =>
                            setMappings((prev) => ({ ...prev, [file.filePath]: value }))
                          }
                        >
                          <SelectTrigger className="h-8 w-full text-xs">
                            <SelectValue placeholder="Select episode..." />
                          </SelectTrigger>
                          <SelectContent className="max-h-[300px]">
                            <SelectItem value={UNASSIGNED}>
                              <span className="text-muted-foreground/60">Skip (unassigned)</span>
                            </SelectItem>
                            {Array.from(groupedEpisodes.entries()).map(([seasonLabel, eps]) => (
                              <React.Fragment key={seasonLabel}>
                                <div className="px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/60">
                                  {seasonLabel}
                                </div>
                                {eps.map((ep) => (
                                  <SelectItem key={ep.id} value={ep.id}>
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
              <p className="text-sm text-red-400">{error}</p>
            )}
          </>
        )}

        <DialogFooter>
          {results ? (
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Close
            </Button>
          ) : (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)} disabled={importing}>
                Cancel
              </Button>
              <Button
                onClick={() => void handleImport()}
                disabled={importing || assignedCount === 0 || loading}
              >
                {importing ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Importing...
                  </>
                ) : (
                  `Import ${assignedCount} file${assignedCount === 1 ? "" : "s"}`
                )}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
