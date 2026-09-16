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
  type SubtitleSearchResult,
  downloadSubtitleMutation,
  searchSubtitlesMutation,
} from "@/lib/graphql/mutations";
import {
  externalSubtitleBlocklistEntriesQuery,
  subtitleSettingsInitQuery,
} from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { ExternalSubtitleSection } from "@/components/common/external-subtitle-section";
import type {
  ExternalSubtitleBlocklistEntryRecord,
  ExternalSubtitleRecord,
} from "@/lib/types/subtitles";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import { LoadingMark } from "@/components/common/loading-mark";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  mediaFileId: string;
  filePath: string;
  downloads: ExternalSubtitleRecord[];
  onChanged: () => void | Promise<void>;
};

export function SubtitleSearchModal({
  open,
  onOpenChange,
  mediaFileId,
  filePath,
  downloads,
  onChanged,
}: Props) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [language, setLanguage] = React.useState("eng");
  const [results, setResults] = React.useState<SubtitleSearchResult[]>([]);
  const [hasSearched, setHasSearched] = React.useState(false);
  const [searching, setSearching] = React.useState(false);
  const [hasEnabledProviders, setHasEnabledProviders] = React.useState<boolean | null>(null);
  const [hasEnabledOpenSubtitlesProvider, setHasEnabledOpenSubtitlesProvider] =
    React.useState<boolean | null>(null);
  const [hasEnabledNonOpenSubtitlesProvider, setHasEnabledNonOpenSubtitlesProvider] =
    React.useState<boolean | null>(null);
  const [hasOpenSubtitlesApiKey, setHasOpenSubtitlesApiKey] = React.useState<boolean | null>(null);
  const [downloadingId, setDownloadingId] = React.useState<string | null>(null);
  const [blocklistEntries, setBlacklistEntries] = React.useState<
    ExternalSubtitleBlocklistEntryRecord[]
  >([]);

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
      nextLanguage: string,
      options?: {
        announceNoResults?: boolean;
      },
    ) => {
      setSearching(true);
      setHasSearched(true);
      setResults([]);
      try {
        const { data, error } = await client
          .mutation(searchSubtitlesMutation, {
            input: { mediaFileId, language: nextLanguage.trim() },
          })
          .toPromise();
        if (error) throw error;
        const sorted = [...(data?.searchSubtitles ?? [])].sort(
          (a: SubtitleSearchResult, b: SubtitleSearchResult) => b.score - a.score,
        );
        setResults(sorted);
        if ((options?.announceNoResults ?? true) && sorted.length === 0) {
          setGlobalStatus(t("subtitle.noResults"));
        }
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
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
    setHasEnabledProviders(null);
    setHasEnabledOpenSubtitlesProvider(null);
    setHasEnabledNonOpenSubtitlesProvider(null);
    setHasOpenSubtitlesApiKey(null);
    void Promise.all([
      client.query(subtitleSettingsInitQuery, {}, { requestPolicy: "network-only" }).toPromise(),
      client
        .query(
          externalSubtitleBlocklistEntriesQuery,
          { mediaFileId },
          { requestPolicy: "network-only" },
        )
        .toPromise(),
    ])
      .then(([settingsResult, blocklistResult]) => {
        if (cancelled) {
          return;
        }
        if (settingsResult.error) {
          throw settingsResult.error;
        }
        if (blocklistResult.error) {
          throw blocklistResult.error;
        }
        const preferredLanguage =
          settingsResult.data?.subtitleSettings?.languages?.[0]?.code ?? "eng";
        const enabledProviders = (
          settingsResult.data?.subtitleProviderConfigs ?? []
        ).filter((provider: { isEnabled: boolean }) => provider.isEnabled);
        const availableHostBindings = new Set<string>(
          (settingsResult.data?.subtitleProviderTypes ?? []).flatMap(
            (providerType: { availableHostBindings?: string[] | null }) =>
              providerType.availableHostBindings ?? [],
          ),
        );
        const hasOpenSubtitlesProvider = enabledProviders.some(
          (provider: { providerType: string }) =>
            provider.providerType.trim().toLowerCase() === "opensubtitles",
        );
        const hasNonOpenSubtitlesProvider = enabledProviders.some(
          (provider: { providerType: string }) =>
            provider.providerType.trim().toLowerCase() !== "opensubtitles",
        );
        setHasEnabledProviders(enabledProviders.length > 0);
        setHasEnabledOpenSubtitlesProvider(hasOpenSubtitlesProvider);
        setHasEnabledNonOpenSubtitlesProvider(hasNonOpenSubtitlesProvider);
        setHasOpenSubtitlesApiKey(
          availableHostBindings.has("smg.opensubtitles_api_key"),
        );
        setLanguage(preferredLanguage);
        setBlacklistEntries(
          (blocklistResult.data?.externalSubtitleBlocklistEntries ?? []) as ExternalSubtitleBlocklistEntryRecord[],
        );
        const hasApiKey = availableHostBindings.has("smg.opensubtitles_api_key");
        const canAutoSearch =
          enabledProviders.length > 0 &&
          (hasNonOpenSubtitlesProvider || !hasOpenSubtitlesProvider || hasApiKey);
        if (canAutoSearch) {
          void runSearch(preferredLanguage, { announceNoResults: false });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.apiError"),
          );
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, mediaFileId, open, runSearch, setGlobalStatus, t]);

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
        );
      } finally {
        setDownloadingId(null);
      }
    },
    [client, mediaFileId, onChanged, setGlobalStatus, t],
  );

  const canSearchSubtitles =
    hasEnabledProviders === true &&
    (hasEnabledNonOpenSubtitlesProvider === true ||
      hasEnabledOpenSubtitlesProvider !== true ||
      hasOpenSubtitlesApiKey === true);

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
            {hasEnabledProviders === false ? (
              <div
                role="alert"
                className="flex items-start gap-3 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]"
              >
                <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]" />
                <div className="space-y-1">
                  <p className="font-medium">
                    {t("subtitle.providersRequiredTitle")}
                  </p>
                  <p className="text-xs text-[var(--scry-warning-text)] opacity-80">
                    {t("subtitle.providersRequiredBody")}
                  </p>
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
                </div>
              </div>
            ) : hasEnabledOpenSubtitlesProvider === true &&
              hasOpenSubtitlesApiKey === false ? (
              <div
                role="alert"
                className="flex items-start gap-3 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]"
              >
                <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]" />
                <div className="space-y-1">
                  <p className="font-medium">
                    {t("subtitle.apiKeyRequiredTitle")}
                  </p>
                  <p className="text-xs text-[var(--scry-warning-text)] opacity-80">
                    {t("subtitle.apiKeyRequiredBody")}
                  </p>
                </div>
              </div>
            ) : null}
            <div className="flex items-center gap-2">
              <div id="subtitle-search-language-picker" className="min-w-0 flex-1">
                <SubtitleLanguagePicker
                  value={language ? [language] : []}
                  onChange={(codes) => setLanguage(codes[0] ?? "")}
                  singleSelect
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
            ) : hasSearched && !searching ? (
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
