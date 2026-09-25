import * as React from "react";
import {
  CircleAlert,
  CircleCheckBig,
  Download,
  HardDriveDownload,
  Search,
} from "lucide-react";
import { useClient } from "urql";

import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { grabSubjects, rankGrabSuggestions, groupGrabRouting, canSubmitGrab, grabClientKey, selectedGrabClientKey, pendingGrabRows, type GrabClient, type GrabGroup, type GrabRoutingRow } from "@/lib/utils/indexer-grab";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
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
  queueIndexerSearchAssignmentMutation,
  queueUnlinkedReleaseMutation,
} from "@/lib/graphql/mutations";
import {
  catalogSearchTitlesQuery,
  indexerGrabClientsQuery,
  downloadClientCategoriesQuery,
} from "@/lib/graphql/queries";
import type { Release, TitleRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  episodeSubjectIncomplete,
  episodeSubjectInput,
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
  /** Called once every release in the batch was queued. */
  onGrabbed: () => void;
};

export function GrabDialog({
  open,
  onOpenChange,
  releases,
  searchIdByRowKey,
  onGrabbed,
}: GrabDialogProps) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();

  const [titleQuery, setTitleQuery] = React.useState("");
  const [candidates, setCandidates] = React.useState<TitleRecord[]>([]);
  const [loadingTitles, setLoadingTitles] = React.useState(false);
  const [selectedTitle, setSelectedTitle] = React.useState<TitleRecord | null>(
    null,
  );
  const [groups, setGroups] = React.useState<GrabGroup[]>([]);
  const [loadingRouting, setLoadingRouting] = React.useState(false);
  const [frozenAction, setFrozenAction] = React.useState<boolean | null>(null);
  const subjects = React.useMemo(() => grabSubjects(releases), [releases]);
  const [season, setSeason] = React.useState("");
  const [episode, setEpisode] = React.useState("");
  const [replaceExisting, setReplaceExisting] = React.useState(false);
  const [acknowledged, setAcknowledged] = React.useState(false);
  const [submitting, setSubmitting] = React.useState(false);
  const submissionInFlight = React.useRef(false);
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
    setTitleQuery("");
    setSelectedTitle(null);
    setGroups([]);
    setFrozenAction(null);
    setSeason("");
    setEpisode("");
    setReplaceExisting(false);
    setAcknowledged(false);
    setSubmitting(false);
    setErrorMessage(null);
    setQueuedRowKeys(new Set());
  }, [open]);


  React.useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    setLoadingTitles(true);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const queries = titleQuery.trim() ? [titleQuery.trim()] : [...new Set(subjects.map((subject) => subject.name))];
          const found: TitleRecord[] = [];
          for (const query of queries) {
            const { data, error } = await client.query(catalogSearchTitlesQuery, {
              query, facet: null, limit: TITLE_CANDIDATE_LIMIT,
            }).toPromise();
            if (cancelled) return;
            if (error) throw error;
            found.push(...(data?.titles?.items ?? []) as TitleRecord[]);
          }
          setCandidates(titleQuery.trim() ? found : rankGrabSuggestions(found, subjects));
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
  }, [client, subjects, open, t, titleQuery]);

  React.useEffect(() => {
    if (!open || frozenAction !== null) return;
    let cancelled = false;
    setLoadingRouting(true);
    void (async () => {
      const rows: GrabRoutingRow[] = [];
      for (const release of releases) {
        const rowKey = indexerSearchRowKey(release);
        const row: GrabRoutingRow = { rowKey, plain: [], assigned: [] };
        const load = async (titleId: string | null): Promise<GrabClient[]> => {
          const searchId = searchIdByRowKey.get(rowKey);
          const downloadUrl = release.downloadUrl ?? release.link;
          if (!searchId || !downloadUrl) throw new Error(t("grabDialog.error.expired"));
          const { data, error } = await client.query(indexerGrabClientsQuery, { searchId, downloadUrl, titleId }, { requestPolicy: "network-only" }).toPromise();
          if (error) throw error;
          return data?.indexerGrabClients ?? [];
        };
        try { row.plain = await load(null); }
        catch (error) { row.plainError = userFacingGraphQlErrorMessage(error, t("status.failedToLoad")); }
        if (selectedTitle) {
          try { row.assigned = await load(selectedTitle.id); }
          catch (error) { row.assignedError = userFacingGraphQlErrorMessage(error, t("status.failedToLoad")); }
        }
        if (cancelled) return;
        rows.push(row);
      }
      if (!cancelled && !submissionInFlight.current) {
        setGroups((current) => groupGrabRouting(rows).map((group) => {
          const previous = current.find((item) => item.id === group.id);
          return previous ? { ...group, clientId: previous.clientId, category: previous.category } : group;
        }));
        setLoadingRouting(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client, open, releases, searchIdByRowKey, selectedTitle, frozenAction, t]);

  const rejectionCodes = React.useMemo(
    () => releaseRejectionCodes(releases),
    [releases],
  );
  const visibleCandidates = titleQuery.trim() ? candidates : candidates.slice(0, VISIBLE_TITLE_CANDIDATES);
  const episodic = titleIsEpisodic(selectedTitle);
  const canReplace =
    selectedTitle != null && titleHoldsFile(selectedTitle);
  const incompleteSubject = episodic && episodeSubjectIncomplete(season, episode);
  const useReplacement = canReplace && replaceExisting;
  const locked = submitting || frozenAction !== null;
  const routingReady = !loadingRouting && groups.length > 0;
  const grabGate = { groups, rejectionCount: rejectionCodes.length, acknowledged };
  const canGrab = canSubmitGrab({ ...grabGate, assign: false, ready: !submitting && routingReady && frozenAction !== true });
  const canAssign = canSubmitGrab({
    ...grabGate,
    assign: true,
    ready: !submitting && routingReady && frozenAction !== false && selectedTitle !== null && !incompleteSubject,
  });

  const chooseTitle = React.useCallback((title: TitleRecord) => {
    setSelectedTitle((current) => current?.id === title.id ? null : title);
    setReplaceExisting(false);
    setSeason("");
    setEpisode("");
  }, []);

  const selectionFor = React.useCallback((release: Release) => {
    const group = groups.find((group) => group.rows.some((row) => row.rowKey === indexerSearchRowKey(release)));
    if (!group?.clientId) throw new Error(t("grabDialog.client.none"));
    return { clientId: group.clientId, category: group.category };
  }, [groups, t]);

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
      const routing = selectionFor(release);
      const payload = await retryWithReplaceOnConflict(
        {
          titleId: title.id,
          scope: releaseQueueScopeInput(tokenized, { title: true }),
          candidateToken: tokenized.candidateToken,
          sizeBytes: tokenized.sizeBytes ?? release.sizeBytes ?? null,
        },
        async (input) => {
          const { data: queued, error: queueError } = await client
            .mutation(queueIndexerSearchAssignmentMutation, { input, routing, replacement: useReplacement })
            .toPromise();
          if (queueError) throw queueError;
          return queued?.queueIndexerSearchAssignment;
        },
        conflictMessage,
        confirmReplaceConflict,
      );
      assertNoReplaceConflict(payload, conflictMessage);
      if (!payload?.jobId) throw new Error(t("status.queueFailed"));
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
      selectionFor,
    ],
  );

  const grabUnlinked = React.useCallback(
    async (release: Release, searchId: string, downloadUrl: string) => {
      const { data, error } = await client
        .mutation(queueUnlinkedReleaseMutation, {
          input: {
            searchId,
            downloadUrl,
            downloadClientId: selectionFor(release).clientId,
            category: selectionFor(release).category,
          },
        })
        .toPromise();
      if (error) throw error;
      const payload = data?.queueUnlinkedRelease as
        | { downloadId: string; clientName: string; sourceTitle: string }
        | undefined;
      if (!payload?.downloadId) throw new Error(t("status.queueFailed"));
      setGlobalStatus(
        t("grabDialog.status.unlinked", {
          name: payload?.sourceTitle ?? release.title,
          client: payload?.clientName ?? "",
        }),
      );
    },
    [client, selectionFor, setGlobalStatus, t],
  );

  const handleGrab = React.useCallback(async (assign: boolean) => {
    if (submissionInFlight.current || (assign ? !canAssign : !canGrab)) return;
    submissionInFlight.current = true;
    setErrorMessage(null);
    setSubmitting(true);
    setFrozenAction(assign);
    let successes = queuedRowKeys.size;
    let failures = 0;
    try {
      // Sequential on purpose: each release reports its own outcome, and a
      // conflict prompt can only be answered one release at a time.
      for (const release of pendingGrabRows(releases, queuedRowKeys, indexerSearchRowKey)) {
        const rowKey = indexerSearchRowKey(release);
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
          if (!assign) {
            await grabUnlinked(release, searchId, downloadUrl);
          } else if (selectedTitle) {
            await grabLinked(release, searchId, downloadUrl, selectedTitle);
          }
          successes += 1;
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
      submissionInFlight.current = false;
      setSubmitting(false);
      if (successes === 0) setFrozenAction(null);
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
    canGrab,
    canAssign,
  ]);

  const multiple = releases.length > 1;

  return (
    <>
      <Dialog open={open} onOpenChange={(next) => { if (!submitting) onOpenChange(next); }}>
        <DialogContent
          id="grab-dialog"
          data-ui="grab-dialog"
          aria-describedby={undefined}
          className="flex max-h-[calc(100dvh-2rem)] w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden rounded-[16px] border-[var(--scry-border2)] bg-[var(--scry-card2)] p-0 sm:max-w-[660px]"
        >
          <div className="flex shrink-0 items-start gap-3 border-b border-[var(--scry-border)] px-5 py-4">
            <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text-soft)]">
              <HardDriveDownload className="h-4 w-4" />
            </span>
            <div className="min-w-0">
              <DialogTitle className="text-[17px] font-bold text-[var(--scry-ink3)]">
                {multiple
                  ? t("grabDialog.title.many", { count: releases.length })
                  : t("grabDialog.title.one")}
              </DialogTitle>

            </div>
          </div>

          <fieldset disabled={locked} className="max-h-[62vh] min-h-0 min-w-0 space-y-4 overflow-y-auto px-4 py-4 sm:px-5">
            <ReleaseSummary releases={releases} />
            {loadingRouting ? <p className="text-xs text-[var(--scry-muted2)]">{t("grabDialog.routing.loading")}</p> : null}
            {groups.map((group, index) => (
              <section key={group.id} className="space-y-2 rounded-lg border border-[var(--scry-border2)] p-3">
                {groups.length > 1 ? <p className="text-xs text-[var(--scry-muted2)]">{t(group.rows.length === 1 ? "grabDialog.routing.single" : "grabDialog.routing.group", { count: group.rows.length })}</p> : null}
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <LabelledField label={t("grabDialog.client")}>
                    <Select value={selectedGrabClientKey(group)} disabled={locked || loadingRouting || group.clients.some((item) => item.mapped)} onValueChange={(key) => setGroups((current) => current.map((item) => {
                      const choice = item.clients.find((client) => grabClientKey(client) === key);
                      return item.id === group.id && choice ? { ...item, clientId: choice.id, category: choice.category ?? "" } : item;
                    }))}>
                      <SelectTrigger id={`grab-dialog-client-${index}`} aria-label={t("grabDialog.client")} className="w-full"><SelectValue placeholder={t("grabDialog.client.placeholder")} /></SelectTrigger>
                      <SelectContent>{group.clients.map((item) => <SelectItem key={grabClientKey(item)} value={grabClientKey(item)}>{group.clients.some((other) => other !== item && other.id === item.id) && item.category ? `${item.name} · ${item.category}` : item.name}</SelectItem>)}</SelectContent>
                    </Select>
                  </LabelledField>
                  <CategoryField clientId={group.clientId} category={group.category} disabled={locked} index={index} onChange={(category) => setGroups((current) => current.map((item) => item.id === group.id ? { ...item, category } : item))} />
                </div>
                {[...new Set(group.rows.flatMap((row) => [row.plainError, row.assignedError]).filter(Boolean))].map((error) => <p key={error} role="alert" className="text-xs text-[var(--scry-danger-text-soft)]">{error}</p>)}
              </section>
            ))}


            <section className="space-y-2">
              <h3 className="text-[10.5px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
                {t(titleQuery.trim() ? "grabDialog.assign.label" : "grabDialog.assign.suggested")}
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
                {incompleteSubject ? <p role="alert" className="col-span-2 text-xs text-[var(--scry-danger-text-soft)]">{t("grabDialog.episodic.incomplete")}</p> : null}
              </section>
            ) : null}

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
          </fieldset>

          <div className="flex shrink-0 flex-wrap items-center gap-2 border-t border-[var(--scry-border)] bg-[var(--scry-surfD)] px-5 py-3">
            {queuedRowKeys.size > 0 ? <span className="text-xs text-[var(--scry-muted2)]">{t("grabDialog.footer.partial", { count: queuedRowKeys.size })}</span> : null}
            <div className="flex-1" />
            <Button id="grab-dialog-cancel" type="button" variant="outline" size="sm" disabled={submitting} onClick={() => onOpenChange(false)}>{t("label.cancel")}</Button>
            <Button id="grab-dialog-grab" type="button" variant="outline" size="sm" disabled={!canGrab} onClick={() => { void handleGrab(false); }}>{t("grabDialog.cta.grab")}</Button>
            <Button id="grab-dialog-submit" type="button" variant="success" size="sm" disabled={!canAssign} onClick={() => { void handleGrab(true); }}>
              <Download className="h-3.5 w-3.5" />{t("grabDialog.cta.assign")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
      {replaceConflictDialog}
    </>
  );
}

function CategoryField({ clientId, category, disabled, index, onChange }: {
  clientId: string; category: string; disabled: boolean; index: number; onChange: (category: string) => void;
}) {
  const client = useClient();
  const t = useTranslate();
  const [choices, setChoices] = React.useState<string[]>([]);
  React.useEffect(() => {
    let cancelled = false;
    setChoices([]);
    if (!clientId) return;
    void (async () => {
      const { data, error } = await client.query(downloadClientCategoriesQuery, { clientId }, { requestPolicy: "network-only" }).toPromise();
      if (cancelled) return;
      if (!error && data?.downloadClientCategories?.supported) setChoices(data.downloadClientCategories.categories);
    })().catch(() => { if (!cancelled) setChoices([]); });
    return () => { cancelled = true; };
  }, [client, clientId]);
  const listId = `grab-dialog-categories-${index}`;
  return <LabelledField label={t("grabDialog.category")}>
    <Input id={`grab-dialog-category-${index}`} list={listId} value={category} disabled={disabled || !clientId} onChange={(event) => onChange(event.target.value)} placeholder={t("grabDialog.category.default")} aria-label={t("grabDialog.category")} />
    <datalist id={listId}>{choices.map((choice) => <option key={choice} value={choice} />)}</datalist>
    <button type="button" disabled={disabled || !clientId} onClick={() => onChange("")} className="mt-1 text-xs text-[var(--scry-accent)]">{t("grabDialog.category.default")}</button>
  </LabelledField>;
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
      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-2 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2.5"
    >
      <span className="min-w-0 justify-self-start [overflow-wrap:anywhere] rounded-[5px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] px-1.5 py-px text-[9.5px] font-extrabold tracking-[0.04em] text-[var(--scry-text4)]">
        {single ? (single.source ?? "—") : t("grabDialog.summary.mix")}
      </span>
      <span className="col-span-2 row-start-2 min-w-0 whitespace-normal [overflow-wrap:anywhere] text-[13px] font-semibold text-[var(--scry-ink3)]">
        {single
          ? single.title
          : t("grabDialog.summary.mixed", { count: releases.length })}
      </span>
      <span className="col-start-2 row-start-1 text-[12.5px] tabular-nums text-[var(--scry-muted2)]">
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
      <TitlePosterSlot src={title.posterUrl} alt={title.name} emptyLabel={title.name} fallbackTitle={title.name} fallbackShowText={false} className="h-16 w-11 shrink-0 rounded object-cover" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink3)]">
          {title.name}
          {title.year ? ` (${title.year})` : ""}
        </span>
        <span className="block truncate text-[11.5px] text-[var(--scry-muted3)]">
          {[title.facet, title.libraryName]
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
