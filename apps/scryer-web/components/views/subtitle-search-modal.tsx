import * as React from "react";
import { useClient } from "urql";
import { Search, ArrowDownToLine, Hash, CircleAlert } from "lucide-react";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Badge } from "@/components/ui/badge";
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
  type SubtitleSearchPayload,
  type SubtitleSearchResult,
  type SubtitleSearchStatus,
  downloadSubtitleMutation,
  searchSubtitlesMutation,
} from "@/lib/graphql/mutations";
import { externalSubtitleBlocklistEntriesQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { ExternalSubtitleSection } from "@/components/common/external-subtitle-section";
import type {
  ExternalSubtitleBlocklistEntryRecord,
  ExternalSubtitleRecord,
} from "@/lib/types/subtitles";
import { SUBTITLE_LANGUAGES, type SubtitleLanguage } from "@/lib/constants/subtitle-languages";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mediaFileId: string;
  libraryId: string | null;
  filePath: string;
  downloads: ExternalSubtitleRecord[];
  onChanged: () => void | Promise<void>;
};

export function SubtitleSearchModal({
  open,
  onOpenChange,
  mediaFileId,
  libraryId,
  filePath,
  downloads,
  onChanged,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const { user } = useAuth();
  const [language, setLanguage] = React.useState("");
  const [availableLanguages, setAvailableLanguages] = React.useState<string[]>([]);
  const [status, setStatus] = React.useState<SubtitleSearchStatus | null>(null);
  const [results, setResults] = React.useState<SubtitleSearchResult[]>([]);
  const [hasSearched, setHasSearched] = React.useState(false);
  const [searching, setSearching] = React.useState(false);
  const [downloadingId, setDownloadingId] = React.useState<string | null>(null);
  const [blocklistEntries, setBlacklistEntries] = React.useState<
    ExternalSubtitleBlocklistEntryRecord[]
  >([]);
  // Read inside `runSearch` without making it depend on the state, which would
  // re-fire the open effect every time a search lands.
  const availableLanguagesRef = React.useRef<string[]>([]);
  React.useEffect(() => {
    availableLanguagesRef.current = availableLanguages;
  }, [availableLanguages]);

  const loadBlocklistEntries = React.useCallback(async () => {
    const { data, error } = await client
      .query(
        externalSubtitleBlocklistEntriesQuery,
        { mediaFileId },
        { requestPolicy: "network-only" },
      )
      .toPromise();
    if (error) {
      throw error;
    }
    setBlacklistEntries(
      (data?.externalSubtitleBlocklistEntries ?? []) as ExternalSubtitleBlocklistEntryRecord[],
    );
  }, [client, mediaFileId]);

  const runSearch = React.useCallback(
    async (
      nextLanguage: string | null,
      options?: {
        announceNoResults?: boolean;
      },
    ) => {
      setSearching(true);
      setHasSearched(true);
      setResults([]);
      try {
        const trimmed = nextLanguage?.trim() ?? "";
        const { data, error } = await client
          .mutation(searchSubtitlesMutation, {
            input: {
              mediaFileId,
              ...(trimmed ? { language: trimmed } : {}),
            },
          })
          .toPromise();
        if (error) throw error;
        const payload = data?.searchSubtitles as SubtitleSearchPayload | undefined;
        if (!payload) {
          throw new Error(t("status.apiError"));
        }
        // The server is the single source of truth for what was searched and
        // what can be searched next; the modal does no provider probing.
        setStatus(payload.status);
        setLanguage(payload.language);
        setAvailableLanguages(payload.availableLanguages);
        const sorted = [...payload.results].sort(
          (a: SubtitleSearchResult, b: SubtitleSearchResult) => b.score - a.score,
        );
        setResults(sorted);
        if (
          (options?.announceNoResults ?? true) &&
          payload.status === "READY" &&
          sorted.length === 0
        ) {
          setGlobalStatus(t("subtitle.noResults"));
        }
      } catch (error) {
        // A failed search (a provider error surfaces as a GraphQL error, not a
        // status) must leave the controls usable so the user can retry without
        // reopening the modal: reaching the provider stage means the server
        // considered this file searchable, so treat it as READY and make sure
        // the picker holds something to search with.
        setStatus((current) => current ?? "READY");
        setLanguage((current) => current || availableLanguagesRef.current[0] || "eng");
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
          { level: "ERROR" },
        );
      } finally {
        setSearching(false);
      }
    },
    [client, mediaFileId, setGlobalStatus, t],
  );

  React.useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    setResults([]);
    setHasSearched(false);
    setStatus(null);
    setAvailableLanguages([]);
    availableLanguagesRef.current = [];
    // One search on open, with no language: the server answers with the
    // language it searched and the languages it can serve next.
    void runSearch(null, { announceNoResults: false });
    void loadBlocklistEntries().catch((error: unknown) => {
      if (!cancelled) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
          { level: "ERROR" },
        );
      }
    });

    return () => {
      cancelled = true;
    };
  }, [loadBlocklistEntries, open, runSearch, setGlobalStatus, t]);

  const handleSearch = React.useCallback(async () => {
    await runSearch(language);
  }, [language, runSearch]);

  const handleDownload = React.useCallback(
    async (result: SubtitleSearchResult) => {
      setDownloadingId(result.providerFileId);
      try {
        const { error } = await client
          .mutation(downloadSubtitleMutation, {
            input: {
              mediaFileId,
              provider: result.provider,
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
        await onChanged();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
          { level: "ERROR" },
        );
      } finally {
        setDownloadingId(null);
      }
    },
    [client, mediaFileId, onChanged, setGlobalStatus, t],
  );

  // Only READY means the server will answer a search; every other status is a
  // message with the controls disabled.
  const canSearchSubtitles = status === "READY";
  const canOpenSubtitleSettings = hasAppPermission(
    user,
    APP_PERMISSIONS.manageCatalogSettings,
  );

  // Every language stays selectable — the configured ones simply sort first.
  const languageOptions = React.useMemo<SubtitleLanguage[]>(() => {
    if (availableLanguages.length === 0) {
      return SUBTITLE_LANGUAGES;
    }
    const rank = new Map(availableLanguages.map((code, index) => [code, index]));
    return [...SUBTITLE_LANGUAGES].sort((a, b) => {
      const left = rank.get(a.code) ?? Number.MAX_SAFE_INTEGER;
      const right = rank.get(b.code) ?? Number.MAX_SAFE_INTEGER;
      return left - right;
    });
  }, [availableLanguages]);

  const statusMessage =
    status === "NO_PROVIDERS"
      ? {
          title: t("subtitle.providersRequiredTitle"),
          body: t("subtitle.providersRequiredBody"),
        }
      : status === "PROVIDER_UNAVAILABLE"
        ? {
            title: t("subtitle.providerUnavailableTitle"),
            body: t("subtitle.providerUnavailableBody"),
          }
        : status === "DISABLED"
          ? {
              title: t("subtitle.subtitlesDisabledTitle"),
              body: t("subtitle.subtitlesDisabledBody"),
            }
          : null;

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent
          id="subtitle-search-dialog"
          className="flex h-[min(92vh,58rem)] !w-[calc(100vw-1.5rem)] !max-w-[calc(100vw-1.5rem)] flex-col overflow-hidden sm:!w-[min(98vw,96rem)] sm:!max-w-[min(98vw,96rem)]"
        >
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Search className="h-4 w-4" />
              {t("subtitle.manualSearch")}
            </DialogTitle>
            <p className="truncate font-[var(--font-code)] text-xs text-muted-foreground">
              {filePath}
            </p>
          </DialogHeader>

          <div className="flex flex-col gap-3">
            {statusMessage ? (
              <div
                role="alert"
                className="flex items-start gap-3 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]"
              >
                <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]" />
                <div className="space-y-1">
                  <p className="font-medium">{statusMessage.title}</p>
                  <p className="text-xs text-[var(--scry-warning-text)] opacity-80">
                    {statusMessage.body}
                  </p>
                  {canOpenSubtitleSettings ? (
                    <Button
                      id="subtitle-search-open-settings"
                      asChild
                      size="sm"
                      variant="outline"
                      className="border-[var(--scry-warning-border)] bg-background/80"
                    >
                      <Link
                        to="/settings/subtitles"
                        onClick={() => onOpenChange(false)}
                      >
                        {t("subtitle.providersRequiredAction")}
                      </Link>
                    </Button>
                  ) : null}
                </div>
              </div>
            ) : null}
            <div className="flex items-center gap-2">
              <div id="subtitle-search-language-picker" className="min-w-0 flex-1">
                <SubtitleLanguagePicker
                  value={language ? [language] : []}
                  languageOptions={languageOptions}
                  onChange={(codes) => setLanguage(codes[0] ?? "")}
                  singleSelect
                  modal
                  compact
                  disabled={!canSearchSubtitles}
                  triggerId="subtitle-search-language-trigger"
                  panelId="subtitle-search-language-panel"
                  searchInputId="subtitle-search-language-input"
                  optionIdPrefix="subtitle-search-language-option"
                />
              </div>
              <Button
                id="subtitle-search-submit"
                onClick={handleSearch}
                disabled={searching || !language.trim() || !canSearchSubtitles}
              >
                {searching ? (
                  <LoadingMark className="mr-1 h-4 w-4" />
                ) : (
                  <Search className="mr-1 h-4 w-4" />
                )}
                {searching ? t("subtitle.searching") : t("subtitle.search")}
              </Button>
            </div>
            <div className="grid gap-4 lg:grid-cols-2">
              <ExternalSubtitleSection
                downloads={downloads}
                libraryId={libraryId}
                allowBlocklist
                onChanged={async () => {
                  await Promise.all([onChanged(), loadBlocklistEntries()]);
                }}
              />
              <div className="space-y-2">
                <p className="text-xs font-medium text-muted-foreground">
                  {t("subtitle.blocklist")}
                </p>
                {blocklistEntries.length === 0 ? (
                  <p className="text-xs text-muted-foreground/70">
                    {t("subtitle.noResults")}
                  </p>
                ) : (
                  <div className="space-y-2">
                    {blocklistEntries.map((entry) => (
                      <div
                        key={entry.id}
                        className="rounded-lg border border-border/60 bg-background/50 px-3 py-2"
                      >
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge tone="info" className="px-1.5 text-[10px] uppercase tracking-wide">
                            {entry.language}
                          </Badge>
                          <span className="rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                            {entry.provider}
                          </span>
                          <span className="text-[11px] text-muted-foreground">
                            {formatUiDateTime(entry.createdAt, dateTimeFormat)}
                          </span>
                        </div>
                        <p className="mt-1 break-all font-[var(--font-code)] text-[11px] text-muted-foreground">
                          {entry.providerFileId}
                        </p>
                        {entry.reason ? (
                          <p className="mt-1 text-[11px] text-muted-foreground">
                            {entry.reason}
                          </p>
                        ) : null}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-auto rounded-md border border-border/70 bg-background/30">
            {results.length > 0 ? (
              <Table className="w-full min-w-[760px] table-fixed">
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[58%]">
                      {t("subtitle.releaseInfo")}
                    </TableHead>
                    <TableHead className="w-20 text-center">
                      {t("subtitle.score")}
                    </TableHead>
                    <TableHead className="w-32 text-center">
                      {t("subtitle.flags")}
                    </TableHead>
                    <TableHead className="w-28">
                      {t("subtitle.provider")}
                    </TableHead>
                    <TableHead className="w-36 text-right" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {results.map((r) => (
                    <TableRow
                      key={r.providerFileId}
                      id={selectorId("subtitle-search-result-row", r.providerFileId)}
                      data-ui="subtitle-search-result-row"
                      data-subtitle-release-info={r.releaseInfo}
                    >
                      <TableCell className="min-w-0">
                        <span className="block break-words text-xs leading-relaxed">
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
                          {r.scorePercent}%
                          {r.hashMatched ? (
                            <Hash className="h-3 w-3 text-[var(--scry-success-text-soft)]" />
                          ) : null}
                        </span>
                      </TableCell>
                      <TableCell className="text-center">
                        <div className="flex justify-center gap-1">
                          {r.hearingImpaired ? (
                            <Badge tone="warning" className="px-1.5 text-[10px]">
                              {t("subtitle.hearingImpaired")}
                            </Badge>
                          ) : null}
                          {r.forced ? (
                            <Badge tone="info" className="px-1.5 text-[10px]">
                              {t("subtitle.forced")}
                            </Badge>
                          ) : null}
                          {r.aiTranslated ? (
                            <Badge tone="negative" className="px-1.5 text-[10px]">
                              {t("subtitle.aiTranslated")}
                            </Badge>
                          ) : null}
                          {r.machineTranslated ? (
                            <Badge tone="negative" className="px-1.5 text-[10px]">
                              {t("subtitle.machineTranslated")}
                            </Badge>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell className="whitespace-nowrap text-xs text-muted-foreground">
                        {r.provider}
                      </TableCell>
                      <TableCell className="whitespace-nowrap text-right">
                        <IconButton
                          id={selectorId("subtitle-search-download", r.providerFileId)}
                          label={downloadingId === r.providerFileId ? t("subtitle.downloading") : t("subtitle.download")}
                          tone="install"
                          disabled={downloadingId === r.providerFileId}
                          onClick={() => void handleDownload(r)}
                        >
                          {downloadingId === r.providerFileId ? (
                            <LoadingMark className="h-4 w-4" />
                          ) : (
                            <ArrowDownToLine className="h-4 w-4" />
                          )}
                        </IconButton>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : hasSearched && !searching && status === "READY" ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                {t("subtitle.noResults")}
              </p>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
