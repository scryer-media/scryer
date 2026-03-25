import * as React from "react";
import { useClient } from "urql";
import { Search, Download, Loader2, Hash } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
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
import {
  searchSubtitlesMutation,
  downloadSubtitleMutation,
  type SubtitleSearchResult,
} from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mediaFileId: string;
  filePath: string;
  onDownloaded: () => void;
};

export function SubtitleSearchModal({
  open,
  onOpenChange,
  mediaFileId,
  filePath,
  onDownloaded,
}: Props) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [language, setLanguage] = React.useState("eng");
  const [results, setResults] = React.useState<SubtitleSearchResult[]>([]);
  const [searching, setSearching] = React.useState(false);
  const [downloadingId, setDownloadingId] = React.useState<string | null>(null);

  const handleSearch = React.useCallback(async () => {
    setSearching(true);
    setResults([]);
    try {
      const { data, error } = await client
        .mutation(searchSubtitlesMutation, {
          input: { mediaFileId, language: language.trim() },
        })
        .toPromise();
      if (error) throw error;
      const sorted = [...(data?.searchSubtitles ?? [])].sort(
        (a: SubtitleSearchResult, b: SubtitleSearchResult) => b.score - a.score,
      );
      setResults(sorted);
      if (sorted.length === 0) {
        setGlobalStatus(t("subtitle.noResults"));
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.apiError"),
      );
    } finally {
      setSearching(false);
    }
  }, [client, mediaFileId, language, setGlobalStatus, t]);

  const handleDownload = React.useCallback(
    async (result: SubtitleSearchResult) => {
      setDownloadingId(result.providerFileId);
      try {
        const { error } = await client
          .mutation(downloadSubtitleMutation, {
            input: {
              mediaFileId,
              providerFileId: result.providerFileId,
              language: result.language,
              forced: result.forced,
              hearingImpaired: result.hearingImpaired,
              score: result.score,
              releaseInfo: result.releaseInfo,
              uploader: result.uploader,
              aiTranslated: result.aiTranslated,
              machineTranslated: result.machineTranslated,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("subtitle.download") + " \u2714");
        onDownloaded();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        setDownloadingId(null);
      }
    },
    [client, mediaFileId, setGlobalStatus, t, onDownloaded],
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl max-h-[80vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Search className="h-4 w-4" />
            {t("subtitle.manualSearch")}
          </DialogTitle>
          <p className="truncate font-mono text-xs text-muted-foreground">
            {filePath}
          </p>
        </DialogHeader>

        <div className="flex items-center gap-2">
          <Input
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
            placeholder={t("subtitle.selectLanguage")}
            className="max-w-[200px]"
          />
          <Button onClick={handleSearch} disabled={searching || !language.trim()}>
            {searching ? (
              <Loader2 className="mr-1 h-4 w-4 animate-spin" />
            ) : (
              <Search className="mr-1 h-4 w-4" />
            )}
            {searching ? t("subtitle.searching") : t("subtitle.search")}
          </Button>
        </div>

        <div className="flex-1 overflow-auto">
          {results.length > 0 ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("subtitle.releaseInfo")}</TableHead>
                  <TableHead className="text-center">
                    {t("subtitle.score")}
                  </TableHead>
                  <TableHead className="text-center">Flags</TableHead>
                  <TableHead>{t("subtitle.provider")}</TableHead>
                  <TableHead className="text-right" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {results.map((r) => (
                  <TableRow key={r.providerFileId}>
                    <TableCell className="max-w-[300px]">
                      <span className="block truncate text-xs">
                        {r.releaseInfo || "—"}
                      </span>
                      {r.uploader ? (
                        <span className="text-[10px] text-muted-foreground">
                          {r.uploader}
                        </span>
                      ) : null}
                    </TableCell>
                    <TableCell className="text-center">
                      <span className="inline-flex items-center gap-1 text-xs font-medium">
                        {r.score}
                        {r.hashMatched ? (
                          <Hash className="h-3 w-3 text-emerald-400" />
                        ) : null}
                      </span>
                    </TableCell>
                    <TableCell className="text-center">
                      <div className="flex justify-center gap-1">
                        {r.hearingImpaired ? (
                          <span className="rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] text-amber-300">
                            {t("subtitle.hearingImpaired")}
                          </span>
                        ) : null}
                        {r.forced ? (
                          <span className="rounded bg-purple-500/20 px-1.5 py-0.5 text-[10px] text-purple-300">
                            {t("subtitle.forced")}
                          </span>
                        ) : null}
                        {r.aiTranslated ? (
                          <span className="rounded bg-red-500/20 px-1.5 py-0.5 text-[10px] text-red-300">
                            {t("subtitle.aiTranslated")}
                          </span>
                        ) : null}
                        {r.machineTranslated ? (
                          <span className="rounded bg-red-500/20 px-1.5 py-0.5 text-[10px] text-red-300">
                            {t("subtitle.machineTranslated")}
                          </span>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground">
                      {r.provider}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={downloadingId === r.providerFileId}
                        onClick={() => void handleDownload(r)}
                      >
                        {downloadingId === r.providerFileId ? (
                          <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                        ) : (
                          <Download className="mr-1 h-3 w-3" />
                        )}
                        {downloadingId === r.providerFileId
                          ? t("subtitle.downloading")
                          : t("subtitle.download")}
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          ) : !searching ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {t("subtitle.noResults")}
            </p>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
