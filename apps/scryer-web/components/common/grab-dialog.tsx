// The grab dialog for the Indexers › Search pane (spec 0002, WP5).
//
// A title-less search row carries no candidate token, so the target is asked
// for at the moment of grabbing: pick a library title and the release is
// tokenised against it (D4) and queued through the existing download
// mutations, or grab it unlinked and it goes straight to a download client
// with no title behind it (D8). Coverage is never asked for — the server
// resolves it from the release name (D11) — and a linked grab uses the
// indexer's own client mapping (D16).
import * as React from "react";
import {
  CircleAlert,
  CircleCheckBig,
  Download,
  HardDriveDownload,
  Search,
  Unlink,
} from "lucide-react";
import { useClient } from "urql";

import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  issueInteractiveReleaseCandidateTokenMutation,
  queueExistingMutation,
  queueReplacementMutation,
  queueUnlinkedReleaseMutation,
} from "@/lib/graphql/mutations";
import {
  catalogSearchTitlesQuery,
  downloadClientsQuery,
} from "@/lib/graphql/queries";
import type { InteractiveSearchKind } from "@/lib/graphql/release-search";
import type { DownloadClientRecord, Release, TitleRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  episodeSubjectIncomplete,
  episodeSubjectInput,
  grabDialogCtaKey,
  grabDialogTitleFacet,
  releaseRejectionCodes,
  titleGapLabel,
  titleHoldsFile,
  titleIsEpisodic,
} from "@/lib/utils/grab-dialog";
import {
  formatReleaseSize,
  indexerSearchRowKey,
  totalReleaseBytes,
} from "@/lib/utils/indexer-search";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";

/** Titles fetched per keystroke; the picker shows the first few of them. */
const TITLE_CANDIDATE_LIMIT = 25;
const VISIBLE_TITLE_CANDIDATES = 5;
const TITLE_SEARCH_DEBOUNCE_MS = 250;

export type GrabDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Releases being grabbed; they all land on one target. */
  releases: Release[];
  /**
   * Search job each row came from, keyed by `indexerSearchRowKey`. A retry
   * mints a second job, so the row — not the pane — knows its search id.
   */
  searchIdByRowKey: ReadonlyMap<string, string>;
  /** Seed for the title picker: the operator's own search query. */
  initialQuery: string;
  /** Search kind, which picks the facet the title picker filters on. */
  kind: InteractiveSearchKind;
  /** Called once every release in the batch was queued. */
  onGrabbed: () => void;
};

export function GrabDialog({
  open,
  onOpenChange,
  releases,
  searchIdByRowKey,
  initialQuery,
  kind,
  onGrabbed,
}: GrabDialogProps) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();

  const [titleQuery, setTitleQuery] = React.useState(initialQuery);
  const [candidates, setCandidates] = React.useState<TitleRecord[]>([]);
  const [loadingTitles, setLoadingTitles] = React.useState(false);
  const [selectedTitle, setSelectedTitle] = React.useState<TitleRecord | null>(
    null,
  );
  const [unlinked, setUnlinked] = React.useState(false);
  const [clients, setClients] = React.useState<DownloadClientRecord[]>([]);
  const [clientId, setClientId] = React.useState("");
  const [season, setSeason] = React.useState("");
  const [episode, setEpisode] = React.useState("");
  const [replaceExisting, setReplaceExisting] = React.useState(false);
  const [acknowledged, setAcknowledged] = React.useState(false);
  const [submitting, setSubmitting] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  // Row keys already queued in this opening. A retry after a partial failure
  // only re-submits the releases that did not make it.
  const [queuedRowKeys, setQueuedRowKeys] = React.useState<Set<string>>(
    () => new Set(),
  );

  // Each opening starts from the pane's current query with nothing chosen; the
  // dialog is short-lived and never carries a previous target forward.
  React.useEffect(() => {
    if (!open) {
      return;
    }
    setTitleQuery(initialQuery);
    setSelectedTitle(null);
    setUnlinked(false);
    setClientId("");
    setSeason("");
    setEpisode("");
    setReplaceExisting(false);
    setAcknowledged(false);
    setSubmitting(false);
    setErrorMessage(null);
    setQueuedRowKeys(new Set());
  }, [initialQuery, open]);

  const facet = grabDialogTitleFacet(kind);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    setLoadingTitles(true);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const { data, error } = await client
            .query(catalogSearchTitlesQuery, {
              query: titleQuery.trim() || null,
              facet,
              limit: TITLE_CANDIDATE_LIMIT,
            })
            .toPromise();
          if (error) throw error;
          if (cancelled) {
            return;
          }
          setCandidates((data?.titles?.items ?? []) as TitleRecord[]);
        } catch (error) {
          if (cancelled) {
            return;
          }
          setCandidates([]);
          setErrorMessage(
            userFacingGraphQlErrorMessage(error, t("status.failedToLoad")),
          );
        } finally {
          if (!cancelled) {
            setLoadingTitles(false);
          }
        }
      })();
    }, TITLE_SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [client, facet, open, t, titleQuery]);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const { data, error } = await client
          .query(downloadClientsQuery, {}, { requestPolicy: "cache-first" })
          .toPromise();
        if (error) throw error;
        if (cancelled) {
          return;
        }
        setClients(
          ((data?.downloadClientConfigs ?? []) as DownloadClientRecord[]).filter(
            (record) => record.isEnabled,
          ),
        );
      } catch (error) {
        if (cancelled) {
          return;
        }
        setErrorMessage(
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad")),
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, open, t]);

  const rejectionCodes = React.useMemo(
    () => releaseRejectionCodes(releases),
    [releases],
  );
  const visibleCandidates = candidates.slice(0, VISIBLE_TITLE_CANDIDATES);
  const episodic = !unlinked && titleIsEpisodic(selectedTitle);
  const canReplace =
    !unlinked && selectedTitle != null && titleHoldsFile(selectedTitle);
  const incompleteSubject = episodic && episodeSubjectIncomplete(season, episode);
  const useReplacement = canReplace && replaceExisting;
  const canSubmit =
    !submitting &&
    !incompleteSubject &&
    (rejectionCodes.length === 0 || acknowledged) &&
    (unlinked ? clientId !== "" : selectedTitle !== null);

  const chooseTitle = React.useCallback((title: TitleRecord) => {
    setUnlinked(false);
    setSelectedTitle(title);
    setReplaceExisting(false);
    setSeason("");
    setEpisode("");
  }, []);

  const chooseUnlinked = React.useCallback(() => {
    setUnlinked(true);
    setSelectedTitle(null);
    setReplaceExisting(false);
  }, []);

  const grabLinked = React.useCallback(
    async (
      release: Release,
      searchId: string,
      downloadUrl: string,
      title: TitleRecord,
    ) => {
      const { data, error } = await client
        .mutation(issueInteractiveReleaseCandidateTokenMutation, {
          input: {
            searchId,
            downloadUrl,
            titleId: title.id,
            ...episodeSubjectInput(season, episode),
          },
        })
        .toPromise();
      if (error) throw error;
      const tokenized = data?.issueInteractiveReleaseCandidateToken as
        | Release
        | undefined;
      if (!tokenized?.candidateToken) {
        throw new Error(t("status.releaseMissingCandidateToken"));
      }

      const conflictMessage = t("grabDialog.conflict", { name: release.title });
      const queueDocument = useReplacement
        ? queueReplacementMutation
        : queueExistingMutation;
      const payload = await retryWithReplaceOnConflict(
        {
          titleId: title.id,
          scope: releaseQueueScopeInput(tokenized, { title: true }),
          candidateToken: tokenized.candidateToken,
          sizeBytes: tokenized.sizeBytes ?? release.sizeBytes ?? null,
        },
        async (input) => {
          const { data: queued, error: queueError } = await client
            .mutation(queueDocument, { input })
            .toPromise();
          if (queueError) throw queueError;
          return useReplacement
            ? queued?.queueReplacementRelease
            : queued?.queueExistingTitleDownload;
        },
        conflictMessage,
        confirmReplaceConflict,
      );
      assertNoReplaceConflict(payload, conflictMessage);
      setGlobalStatus(t("status.queueSuccess", { name: release.title }));
    },
    [
      client,
      confirmReplaceConflict,
      episode,
      season,
      setGlobalStatus,
      t,
      useReplacement,
    ],
  );

  const grabUnlinked = React.useCallback(
    async (release: Release, searchId: string, downloadUrl: string) => {
      const { data, error } = await client
        .mutation(queueUnlinkedReleaseMutation, {
          input: {
            searchId,
            downloadUrl,
            downloadClientId: clientId,
          },
        })
        .toPromise();
      if (error) throw error;
      const payload = data?.queueUnlinkedRelease as
        | { clientName: string; sourceTitle: string }
        | undefined;
      setGlobalStatus(
        t("grabDialog.status.unlinked", {
          name: payload?.sourceTitle ?? release.title,
          client: payload?.clientName ?? "",
        }),
      );
    },
    [client, clientId, setGlobalStatus, t],
  );

  const handleGrab = React.useCallback(async () => {
    setErrorMessage(null);
    setSubmitting(true);
    let failures = 0;
    try {
      // Sequential on purpose: each release reports its own outcome, and a
      // conflict prompt can only be answered one release at a time.
      for (const release of releases) {
        const rowKey = indexerSearchRowKey(release);
        if (queuedRowKeys.has(rowKey)) {
          continue;
        }
        const searchId = searchIdByRowKey.get(rowKey);
        // The server locates a release by its download url, falling back to
        // the indexer link for rows that carry no direct download source.
        const downloadUrl = release.downloadUrl ?? release.link;
        if (!searchId || !downloadUrl) {
          failures += 1;
          setErrorMessage(t("grabDialog.error.expired"));
          continue;
        }
        try {
          if (unlinked) {
            await grabUnlinked(release, searchId, downloadUrl);
          } else if (selectedTitle) {
            await grabLinked(release, searchId, downloadUrl, selectedTitle);
          }
          setQueuedRowKeys((current) => new Set(current).add(rowKey));
        } catch (error) {
          failures += 1;
          const reason = userFacingGraphQlErrorMessage(
            error,
            t("status.queueFailed"),
          );
          setErrorMessage(reason);
          setGlobalStatus(
            t("grabDialog.status.failed", { name: release.title, reason }),
          );
        }
      }
    } finally {
      setSubmitting(false);
    }
    // A search job outlives its results by five minutes; past that the grab
    // fails and the dialog stays open so the operator can re-run the search.
    if (failures === 0) {
      onGrabbed();
      onOpenChange(false);
    }
  }, [
    grabLinked,
    grabUnlinked,
    onGrabbed,
    onOpenChange,
    queuedRowKeys,
    releases,
    searchIdByRowKey,
    selectedTitle,
    setGlobalStatus,
    t,
    unlinked,
  ]);

  const multiple = releases.length > 1;

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent
          id="grab-dialog"
          data-ui="grab-dialog"
          className="w-[660px] gap-0 overflow-hidden rounded-[16px] border-[var(--scry-border2)] bg-[var(--scry-card2)] p-0 sm:max-w-[660px]"
        >
          <div className="flex items-start gap-3 border-b border-[var(--scry-border)] px-5 py-4">
            <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text-soft)]">
              <HardDriveDownload className="h-4 w-4" />
            </span>
            <div className="min-w-0">
              <DialogTitle className="text-[17px] font-bold text-[var(--scry-ink3)]">
                {multiple
                  ? t("grabDialog.title.many", { count: releases.length })
                  : t("grabDialog.title.one")}
              </DialogTitle>
              <DialogDescription className="mt-1 text-[12.5px] text-[var(--scry-muted2)]">
                {t("grabDialog.subtitle")}
              </DialogDescription>
            </div>
          </div>

          <div className="max-h-[58vh] space-y-4 overflow-y-auto px-5 py-4">
            <ReleaseSummary releases={releases} />

            <section className="space-y-2">
              <h3 className="text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
                {t("grabDialog.assign.label")}
              </h3>
              <div className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3">
                <Search className="h-3.5 w-3.5 shrink-0 text-[var(--scry-faint)]" />
                <Input
                  id="grab-dialog-title-query"
                  value={titleQuery}
                  onChange={(event) => setTitleQuery(event.target.value)}
                  placeholder={t("grabDialog.assign.placeholder")}
                  aria-label={t("grabDialog.assign.label")}
                  className="h-10 border-0 bg-transparent px-0 shadow-none focus-visible:ring-0"
                />
                <span className="shrink-0 whitespace-nowrap text-[11px] text-[var(--scry-faint)]">
                  {t("grabDialog.assign.count", {
                    shown: visibleCandidates.length,
                    total: candidates.length,
                  })}
                </span>
              </div>

              {visibleCandidates.map((title) => (
                <TitleCandidateRow
                  key={title.id}
                  title={title}
                  selected={selectedTitle?.id === title.id}
                  onChoose={chooseTitle}
                />
              ))}
              {visibleCandidates.length === 0 ? (
                <p
                  id="grab-dialog-candidates-empty"
                  className="px-1 text-[12px] text-[var(--scry-muted3)]"
                >
                  {loadingTitles
                    ? t("grabDialog.assign.loading")
                    : t("grabDialog.assign.empty")}
                </p>
              ) : null}

              <button
                id="grab-dialog-unlinked"
                type="button"
                aria-pressed={unlinked}
                onClick={chooseUnlinked}
                className={cn(
                  "flex w-full items-center gap-2 rounded-[10px] border border-dashed px-3 py-2.5 text-left text-[12.5px] transition",
                  unlinked
                    ? "border-solid border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]"
                    : "border-[var(--scry-border3)] text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)]",
                )}
              >
                <Unlink className="h-3.5 w-3.5 shrink-0" />
                {t("grabDialog.assign.unlinked")}
              </button>
            </section>

            {episodic ? (
              <section className="grid grid-cols-2 gap-3">
                <LabelledField label={t("grabDialog.season")}>
                  <Input
                    id="grab-dialog-season"
                    value={season}
                    inputMode="numeric"
                    onChange={(event) => setSeason(event.target.value)}
                    aria-label={t("grabDialog.season")}
                  />
                </LabelledField>
                <LabelledField label={t("grabDialog.episode")}>
                  <Input
                    id="grab-dialog-episode"
                    value={episode}
                    inputMode="numeric"
                    onChange={(event) => setEpisode(event.target.value)}
                    aria-label={t("grabDialog.episode")}
                  />
                </LabelledField>
                <p
                  id="grab-dialog-episodic-help"
                  className={cn(
                    "col-span-2 text-[11.5px]",
                    incompleteSubject
                      ? "text-[var(--scry-danger-text-soft)]"
                      : "text-[var(--scry-faint)]",
                  )}
                >
                  {incompleteSubject
                    ? t("grabDialog.episodic.incomplete")
                    : t("grabDialog.episodic.help")}
                </p>
              </section>
            ) : null}

            <section className="grid grid-cols-2 gap-3">
              <LabelledField label={t("grabDialog.client")}>
                {unlinked ? (
                  <Select value={clientId} onValueChange={setClientId}>
                    <SelectTrigger
                      id="grab-dialog-client"
                      aria-label={t("grabDialog.client")}
                      className="w-full"
                    >
                      <SelectValue
                        placeholder={t("grabDialog.client.placeholder")}
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {clients.map((record) => (
                        <SelectItem key={record.id} value={record.id}>
                          {record.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : (
                  <p
                    id="grab-dialog-client"
                    className="flex h-9 items-center rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[12.5px] text-[var(--scry-muted2)]"
                  >
                    {t("grabDialog.client.routed")}
                  </p>
                )}
                {unlinked && clients.length === 0 ? (
                  <p className="mt-1 text-[11.5px] text-[var(--scry-danger-text-soft)]">
                    {t("grabDialog.client.none")}
                  </p>
                ) : null}
              </LabelledField>
              <LabelledField label={t("grabDialog.importPath")}>
                <p
                  id="grab-dialog-import-path"
                  className="flex h-9 items-center truncate rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[12.5px] text-[var(--scry-muted2)]"
                >
                  {unlinked || !selectedTitle?.rootFolderPath
                    ? t("grabDialog.importPath.clientDefault")
                    : selectedTitle.rootFolderPath}
                </p>
              </LabelledField>
            </section>

            {canReplace ? (
              <label className="flex items-start gap-2.5 text-[12.5px] text-[var(--scry-ink2)]">
                <Checkbox
                  id="grab-dialog-replace"
                  size="compact"
                  checked={replaceExisting}
                  onCheckedChange={(checked) =>
                    setReplaceExisting(checked === true)
                  }
                />
                {t("grabDialog.option.replace")}
              </label>
            ) : null}

            {rejectionCodes.length > 0 ? (
              <label className="flex items-start gap-2.5 text-[12.5px] text-[var(--scry-warning-text)]">
                <Checkbox
                  id="grab-dialog-acknowledge"
                  size="compact"
                  checked={acknowledged}
                  onCheckedChange={(checked) => setAcknowledged(checked === true)}
                />
                {t("grabDialog.option.acknowledge", {
                  codes: rejectionCodes.join(", "),
                })}
              </label>
            ) : null}

            {errorMessage ? (
              <p
                id="grab-dialog-error"
                className="flex items-start gap-2 rounded-[8px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-[12px] text-[var(--scry-danger-text-soft)]"
              >
                <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                {errorMessage}
              </p>
            ) : null}
          </div>

          <div className="flex flex-wrap items-center gap-3 border-t border-[var(--scry-border)] bg-[var(--scry-surfD)] px-5 py-3">
            <span
              id="grab-dialog-consequence"
              className={cn(
                "flex min-w-0 items-center gap-2 text-[12.5px]",
                unlinked
                  ? "text-[var(--scry-warning-text)]"
                  : selectedTitle
                    ? "text-[var(--scry-success-text-soft)]"
                    : "text-[var(--scry-muted3)]",
              )}
            >
              <span
                className={cn(
                  "h-1.5 w-1.5 shrink-0 rounded-full",
                  unlinked
                    ? "bg-[var(--scry-warning-solid)]"
                    : selectedTitle
                      ? "bg-[var(--scry-success-solid)]"
                      : "bg-[var(--scry-faint3)]",
                )}
              />
              <span className="truncate">
                {unlinked
                  ? t("grabDialog.footer.unlinked")
                  : selectedTitle
                    ? t("grabDialog.footer.linked", { name: selectedTitle.name })
                    : t("grabDialog.footer.pickTitle")}
              </span>
            </span>
            <div className="min-w-2 flex-1" />
            <Button
              id="grab-dialog-cancel"
              type="button"
              variant="outline"
              size="sm"
              onClick={() => onOpenChange(false)}
            >
              {t("label.cancel")}
            </Button>
            <Button
              id="grab-dialog-submit"
              type="button"
              variant="success"
              size="sm"
              disabled={!canSubmit}
              onClick={() => {
                void handleGrab();
              }}
            >
              <Download className="h-3.5 w-3.5" />
              {t(grabDialogCtaKey(unlinked, releases.length))}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
      {replaceConflictDialog}
    </>
  );
}

function LabelledField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="min-w-0">
      <div className="mb-1.5 text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
        {label}
      </div>
      {children}
    </div>
  );
}

function ReleaseSummary({ releases }: { releases: Release[] }) {
  const t = useTranslate();
  const single = releases.length === 1 ? releases[0] : null;
  return (
    <div
      id="grab-dialog-release-summary"
      className="flex items-center gap-3 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2.5"
    >
      <span className="shrink-0 rounded-[5px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] px-1.5 py-px text-[9.5px] font-extrabold tracking-[0.04em] text-[var(--scry-text4)]">
        {single ? (single.source ?? "—") : t("grabDialog.summary.mix")}
      </span>
      <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-[var(--scry-ink3)]">
        {single
          ? single.title
          : t("grabDialog.summary.mixed", { count: releases.length })}
      </span>
      <span className="shrink-0 text-[12.5px] tabular-nums text-[var(--scry-muted2)]">
        {formatReleaseSize(totalReleaseBytes(releases))}
      </span>
    </div>
  );
}

function TitleCandidateRow({
  title,
  selected,
  onChoose,
}: {
  title: TitleRecord;
  selected: boolean;
  onChoose: (title: TitleRecord) => void;
}) {
  const t = useTranslate();
  const gap = titleGapLabel(title);
  return (
    <button
      id={selectorId("grab-dialog-candidate", title.id)}
      data-ui="grab-dialog-candidate"
      type="button"
      aria-pressed={selected}
      onClick={() => onChoose(title)}
      className={cn(
        "flex w-full items-center gap-3 rounded-[10px] border px-3 py-2.5 text-left transition",
        selected
          ? "border-[var(--scry-accent)] bg-[rgba(var(--scry-accent-rgb),0.09)]"
          : "border-[var(--scry-border2)] hover:bg-[var(--scry-hover)]",
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink3)]">
          {title.name}
          {title.year ? ` (${title.year})` : ""}
        </span>
        <span className="block truncate text-[11.5px] text-[var(--scry-muted3)]">
          {[title.facet, title.libraryName, title.rootFolderPath]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </span>
      <span
        className={cn(
          "shrink-0 text-[11.5px]",
          gap.complete
            ? "text-[var(--scry-success-text-soft)]"
            : "text-[var(--scry-muted2)]",
        )}
      >
        {t(gap.key, gap.params)}
      </span>
      {selected ? (
        <CircleCheckBig className="h-4 w-4 shrink-0 text-[var(--scry-success-text-soft)]" />
      ) : null}
    </button>
  );
}
