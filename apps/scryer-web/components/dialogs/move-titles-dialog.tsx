import * as React from "react";
import { useClient } from "urql";
import { useNavigate } from "react-router";
import {
  ArrowRight,
  CircleCheck,
  HardDrive,
  Loader2,
  ShieldCheck,
  TriangleAlert,
  X,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { startLocationOperationMutation } from "@/lib/graphql/mutations";
import { locationOperationPreviewQuery } from "@/lib/graphql/queries";
import {
  blockingTitles,
  classBlocksStart,
  classificationLabelKey,
  classifiedTitlePlacement,
  destinationLibraryDisabledReasonKey,
  offersModeSelection,
  orderedClassificationGroups,
  orderedPlanKindCounts,
  orderedPlanSections,
  planKindLabelKey,
  previewCanStart,
  recognizeStartRefusal,
  refusalMessageKey,
  refusalNeedsFreshPreview,
  remainingSelection,
  toCount,
  typedConfirmationSatisfied,
  type ClassifiedTitlePlacement,
  type LocationClassifiedTitle,
  type LocationOperationPreview,
  type LocationPlanItem,
  type TitleLocationClass,
} from "@/lib/location-operations";
import { formatByteCount } from "@/lib/utils/activity-utils";
import type { LibraryRootRecord } from "@/lib/types/titles";
import { cn } from "@/lib/utils";

/** A title the move workflow was opened for. */
export type MoveTitleRef = {
  id: string;
  name: string;
  libraryId: string;
  libraryName?: string | null;
  rootFolderId?: string | null;
  rootFolderPath?: string | null;
};

/** A library the destination controls may offer. */
export type MoveDestinationLibrary = {
  id: string;
  name: string;
  roots: LibraryRootRecord[];
};

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titles: MoveTitleRef[];
  libraries: MoveDestinationLibrary[];
  /** Root the destination control opens on, when the caller already picked one. */
  initialRootId?: string | null;
  /** Fires with the accepted operation id after a successful confirm. */
  onStarted?: (operationId: string) => void;
};

/** Only `MOVE_WITH_SCRYER` executes in this phase; the rest are announced. */
const MOVE_MODES = [
  "MOVE_WITH_SCRYER",
  "FILES_ALREADY_THERE",
] as const;

type MoveMode = (typeof MOVE_MODES)[number];

function sortRoots(roots: LibraryRootRecord[]): LibraryRootRecord[] {
  return [...roots].sort((left, right) => {
    if (left.isDefault !== right.isDefault) {
      return left.isDefault ? -1 : 1;
    }
    return left.path.localeCompare(right.path);
  });
}

/**
 * Move workflow for FR-010–FR-017 and FR-086: pick a destination, read the
 * whole plan (every classification group, including the empty and the
 * no-op ones), deselect what blocks the start, then confirm the fingerprint.
 */
export function MoveTitlesDialog({
  open,
  onOpenChange,
  titles,
  libraries,
  initialRootId,
  onStarted,
}: Props) {
  const client = useClient();
  const t = useTranslate();

  const sourceLibraryIds = React.useMemo(
    () => Array.from(new Set(titles.map((title) => title.libraryId))),
    [titles],
  );
  const soleSourceLibraryId =
    sourceLibraryIds.length === 1 ? sourceLibraryIds[0] : null;

  const [libraryId, setLibraryId] = React.useState<string>(
    soleSourceLibraryId ?? "",
  );
  const [rootId, setRootId] = React.useState<string>(initialRootId ?? "");
  const [mode, setMode] = React.useState<MoveMode>("MOVE_WITH_SCRYER");
  const [deselected, setDeselected] = React.useState<Set<string>>(new Set());
  const [preview, setPreview] = React.useState<LocationOperationPreview | null>(
    null,
  );
  const [previewLoading, setPreviewLoading] = React.useState(false);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [typedConfirmation, setTypedConfirmation] = React.useState("");
  const [starting, setStarting] = React.useState(false);
  const [startError, setStartError] = React.useState<string | null>(null);
  const [startedOperationId, setStartedOperationId] = React.useState<
    string | null
  >(null);
  const [planChanged, setPlanChanged] = React.useState(false);
  // Bumped to force a fresh preview when nothing else about the request changed
  // (a refused confirmation, or the user asking to re-preview).
  const [previewNonce, setPreviewNonce] = React.useState(0);

  const titleById = React.useMemo(
    () => new Map(titles.map((title) => [title.id, title])),
    [titles],
  );
  const libraryById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library])),
    [libraries],
  );
  const destinationRoots = React.useMemo(
    () => sortRoots(libraryById.get(libraryId)?.roots ?? []),
    [libraryById, libraryId],
  );
  const rootPathById = React.useMemo(() => {
    const paths = new Map<string, string>();
    for (const library of libraries) {
      for (const root of library.roots) {
        paths.set(root.id, root.path);
      }
    }
    return paths;
  }, [libraries]);

  // Reopening on a different selection must never inherit the previous plan.
  React.useEffect(() => {
    if (!open) {
      return;
    }
    setLibraryId(soleSourceLibraryId ?? "");
    setRootId(initialRootId ?? "");
    setMode("MOVE_WITH_SCRYER");
    setDeselected(new Set());
    setPreview(null);
    setPreviewError(null);
    setTypedConfirmation("");
    setStartError(null);
    setPlanChanged(false);
    setPreviewNonce(0);
    setStartedOperationId(null);
  }, [open, soleSourceLibraryId, initialRootId]);

  const selection = React.useMemo(
    () =>
      remainingSelection(
        titles.map((title) => title.id),
        deselected,
      ),
    [deselected, titles],
  );
  const selectionKey = selection.join(",");

  // Every destination or selection change voids the previous fingerprint, so
  // the preview is re-read rather than patched (FR-016, FR-086).
  React.useEffect(() => {
    if (!open || !rootId || selection.length === 0) {
      setPreview(null);
      setPreviewLoading(false);
      return undefined;
    }
    let active = true;
    setPreviewLoading(true);
    setPreviewError(null);
    setStartError(null);
    client
      .query(
        locationOperationPreviewQuery,
        {
          input: {
            titleIds: selection,
            destination: {
              libraryId: libraryId || null,
              rootId,
            },
          },
        },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (!active) {
          return;
        }
        if (error) {
          setPreview(null);
          setPreviewError(
            userFacingGraphQlErrorMessage(error, t("move.previewFailed")),
          );
          return;
        }
        const next = data?.locationOperationPreview as
          | LocationOperationPreview
          | undefined;
        if (!next) {
          setPreview(null);
          setPreviewError(t("move.previewFailed"));
          return;
        }
        setPreview(next);
        setPlanChanged(false);
      })
      .catch((error: unknown) => {
        if (!active) {
          return;
        }
        setPreview(null);
        setPreviewError(
          userFacingGraphQlErrorMessage(error, t("move.previewFailed")),
        );
      })
      .finally(() => {
        if (active) {
          setPreviewLoading(false);
        }
      });
    return () => {
      active = false;
    };
    // `selectionKey` stands in for `selection`: the identity changes on every
    // render, the contents do not.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, libraryId, open, previewNonce, rootId, selectionKey, t]);

  const groups = React.useMemo(
    () => orderedClassificationGroups(preview?.classification),
    [preview],
  );
  const blocked = React.useMemo(
    () => blockingTitles(preview?.classification),
    [preview],
  );
  const planKindCounts = React.useMemo(
    () => orderedPlanKindCounts(preview?.counts),
    [preview],
  );
  const sections = React.useMemo(
    () => orderedPlanSections(preview?.sections ?? []),
    [preview],
  );

  // FR-012's "current library/root/folder" rides on the classification payload
  // itself, so a no-op or catalog-only title states its placement too. These
  // plan-item folders are the fallback for the moving titles, whose calculated
  // destination folder only the plan knows.
  const foldersByTitle = React.useMemo(() => {
    const folders = new Map<string, { source: string | null; destination: string | null }>();
    for (const section of preview?.sections ?? []) {
      for (const item of section.items) {
        if (!item.titleId) {
          continue;
        }
        const existing = folders.get(item.titleId);
        folders.set(item.titleId, {
          source: existing?.source ?? item.sourcePath,
          destination: existing?.destination ?? item.destinationPath,
        });
      }
    }
    return folders;
  }, [preview]);

  const filesByTitle = React.useMemo(() => {
    const stats = new Map<string, { files: number; bytes: number }>();
    for (const section of preview?.sections ?? []) {
      if (section.kind !== "MOVE" && section.kind !== "RENAME") {
        continue;
      }
      for (const item of section.items) {
        if (!item.titleId) {
          continue;
        }
        const current = stats.get(item.titleId) ?? { files: 0, bytes: 0 };
        stats.set(item.titleId, {
          files: current.files + 1,
          bytes: current.bytes + toCount(item.sizeBytes),
        });
      }
    }
    return stats;
  }, [preview]);

  const deselect = React.useCallback((titleId: string) => {
    setDeselected((current) => {
      const next = new Set(current);
      next.add(titleId);
      return next;
    });
  }, []);

  const restoreSelection = React.useCallback(() => {
    setDeselected(new Set());
  }, []);

  const canStart =
    previewCanStart(preview) &&
    !previewLoading &&
    !starting &&
    typedConfirmationSatisfied(preview?.confirmation, typedConfirmation);

  const handleStart = React.useCallback(async () => {
    if (!preview || !rootId) {
      return;
    }
    setStarting(true);
    setStartError(null);
    try {
      const { data, error } = await client
        .mutation(startLocationOperationMutation, {
          input: {
            titleIds: preview.selection,
            destination: { libraryId: libraryId || null, rootId },
            planFingerprint: preview.planFingerprint,
            typedConfirmation:
              preview.confirmation.requirement === "TYPED"
                ? typedConfirmation
                : null,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const started = data?.startLocationOperation as
        | { operation: { id: string } }
        | undefined;
      if (!started?.operation?.id) {
        throw new Error(t("move.startFailed"));
      }
      // The dialog stays open on success: nothing lists location operations
      // yet, so this is where the user picks the operation up in Activity.
      setStartedOperationId(started.operation.id);
      onStarted?.(started.operation.id);
    } catch (error: unknown) {
      const message = userFacingGraphQlErrorMessage(
        error,
        t("move.startFailed"),
      );
      // A refused confirmation is nearly always "the plan moved under you", or
      // a title that became blocked between preview and confirm. Either way the
      // answer is a fresh plan, not a backend sentence about fingerprints.
      const refusal = recognizeStartRefusal(error, message);
      if (refusalNeedsFreshPreview(refusal)) {
        setPlanChanged(true);
        setStartError(null);
        setPreviewNonce((current) => current + 1);
      } else {
        // A refusal Scryer has its own words for says them; anything else
        // shows the server's sentence rather than a guess.
        const key = refusalMessageKey(refusal);
        setStartError(key ? t(key) : message);
      }
    } finally {
      setStarting(false);
    }
  }, [client, libraryId, onStarted, preview, rootId, t, typedConfirmation]);

  const rePreview = React.useCallback(() => {
    setPlanChanged(false);
    // The preview effect always fetches network-only; the nonce is what makes
    // it run again when the request itself is unchanged.
    setPreviewNonce((current) => current + 1);
  }, []);

  const destinationDisabledReasonKey = React.useCallback(
    (candidateLibraryId: string) =>
      destinationLibraryDisabledReasonKey(candidateLibraryId, sourceLibraryIds),
    [sourceLibraryIds],
  );

  const totalFiles = toCount(preview?.counts.filesTotal);
  const totalBytes = toCount(preview?.counts.bytesTotal);
  const freeSpace = preview?.freeSpace ?? null;
  const verification = preview?.verification ?? null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="move-titles-dialog" className="sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>{t("move.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("move.dialogDescription", { count: titles.length })}
          </DialogDescription>
        </DialogHeader>

        {startedOperationId ? (
          <div id="move-titles-started" className="space-y-3">
            <p className="flex items-start gap-2 rounded-lg border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] px-3 py-3 text-sm text-[var(--scry-success-text)]">
              <CircleCheck className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{t("move.startedHeading")}</span>
            </p>
            <p className="font-[var(--font-code)] text-xs break-all text-muted-foreground">
              {startedOperationId}
            </p>
            <ViewOperationButton
              operationId={startedOperationId}
              label={t("move.viewInActivity")}
              onNavigated={() => onOpenChange(false)}
            />
          </div>
        ) : (
        <div className="max-h-[65vh] space-y-4 overflow-y-auto pr-1">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="min-w-0">
              <label
                className="mb-1 block text-xs font-medium text-muted-foreground"
                htmlFor="move-titles-destination-library"
              >
                {t("move.destinationLibrary")}
              </label>
              <Select
                value={libraryId}
                onValueChange={(value) => {
                  setLibraryId(value);
                  setRootId("");
                }}
                disabled={starting}
              >
                <SelectTrigger
                  id="move-titles-destination-library"
                  className="h-9 w-full"
                >
                  <SelectValue placeholder={t("move.destinationLibrary")} />
                </SelectTrigger>
                <SelectContent>
                  {libraries.map((library) => {
                    const reasonKey = destinationDisabledReasonKey(library.id);
                    return (
                      <SelectItem
                        key={library.id}
                        value={library.id}
                        disabled={reasonKey !== null}
                      >
                        {reasonKey === null
                          ? library.name
                          : `${library.name} — ${t(reasonKey)}`}
                      </SelectItem>
                    );
                  })}
                </SelectContent>
              </Select>
              {sourceLibraryIds.length > 1 ? (
                <p
                  id="move-titles-mixed-libraries"
                  className="mt-1 text-xs text-[var(--scry-warning-text)]"
                >
                  {t("move.destinationMixedSourceLibraries")}
                </p>
              ) : null}
            </div>

            <div className="min-w-0">
              <label
                className="mb-1 block text-xs font-medium text-muted-foreground"
                htmlFor="move-titles-destination-root"
              >
                {t("move.destinationRoot")}
              </label>
              <Select
                value={rootId}
                onValueChange={setRootId}
                disabled={starting || destinationRoots.length === 0}
              >
                <SelectTrigger
                  id="move-titles-destination-root"
                  className="h-9 w-full font-[var(--font-code)] text-sm"
                >
                  <SelectValue placeholder={t("move.destinationRootPlaceholder")} />
                </SelectTrigger>
                <SelectContent>
                  {destinationRoots.map((root) => (
                    <SelectItem key={root.id} value={root.id}>
                      {root.path}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {/* FR-076: a fileless selection never presents a move-mode choice. */}
          {preview && offersModeSelection(preview) ? (
            <div className="rounded-lg border border-border bg-muted/20 px-3 py-3">
              <p className="mb-2 text-sm font-medium text-foreground">
                {t("move.modeHeading")}
              </p>
              <RadioGroup
                value={mode}
                onValueChange={(value) => setMode(value as MoveMode)}
              >
                {MOVE_MODES.map((option) => {
                  const unavailable = option !== "MOVE_WITH_SCRYER";
                  return (
                    <label
                      key={option}
                      className={cn(
                        "flex items-start gap-2 text-sm",
                        unavailable && "opacity-60",
                      )}
                    >
                      <RadioGroupItem
                        value={option}
                        disabled={unavailable}
                        id={`move-titles-mode-${option}`}
                        className="mt-0.5"
                      />
                      <span className="min-w-0">
                        <span className="block text-foreground">
                          {t(`move.mode.${option}`)}
                        </span>
                        <span className="block text-xs text-muted-foreground">
                          {unavailable
                            ? t("move.modeUnavailable")
                            : t(`move.modeHelp.${option}`)}
                        </span>
                      </span>
                    </label>
                  );
                })}
              </RadioGroup>
            </div>
          ) : null}

          {preview && !offersModeSelection(preview) ? (
            <p
              id="move-titles-catalog-only"
              className="rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm text-muted-foreground"
            >
              {t("move.catalogOnlyFastPath")}
            </p>
          ) : null}

          {!rootId ? (
            <p className="rounded-lg border border-border px-3 py-6 text-sm text-muted-foreground">
              {t("move.selectDestinationPrompt")}
            </p>
          ) : null}

          {previewLoading ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("move.previewLoading")}
            </p>
          ) : null}

          {previewError ? (
            <p
              id="move-titles-preview-error"
              className="rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3 text-sm text-[var(--scry-danger-text)]"
            >
              {previewError}
            </p>
          ) : null}

          {planChanged ? (
            <div
              id="move-titles-plan-changed"
              className="flex items-start justify-between gap-3 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-3 text-sm text-[var(--scry-warning-text)]"
            >
              <span>{t("move.planChanged")}</span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={rePreview}
              >
                {t("move.rePreview")}
              </Button>
            </div>
          ) : null}

          {preview ? (
            <>
              <dl className="grid gap-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm sm:grid-cols-3">
                <SummaryCell
                  label={t("move.summaryTitles")}
                  value={String(toCount(preview.counts.titlesTotal))}
                />
                <SummaryCell
                  label={t("move.summaryFiles")}
                  value={String(totalFiles)}
                />
                <SummaryCell
                  label={t("move.summarySize")}
                  value={formatByteCount(totalBytes)}
                />
              </dl>

              {planKindCounts.length > 0 ? (
                <div className="flex flex-wrap gap-2">
                  {planKindCounts.map((entry) => (
                    <Badge key={entry.kind} tone="outline">
                      {t(planKindLabelKey(entry.kind))} · {entry.count}
                    </Badge>
                  ))}
                </div>
              ) : null}

              {blocked.length > 0 ? (
                <div
                  id="move-titles-blocked"
                  className="space-y-2 rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3"
                >
                  <p className="flex items-center gap-2 text-sm font-medium text-[var(--scry-danger-text)]">
                    <TriangleAlert className="h-4 w-4 shrink-0" />
                    {t("move.blockedHeading", { count: blocked.length })}
                  </p>
                  <p className="text-xs text-[var(--scry-danger-text)]">
                    {t("move.blockedHelp")}
                  </p>
                  <ul className="space-y-1">
                    {blocked.map((entry) => (
                      <li
                        key={entry.titleId}
                        className="flex items-center justify-between gap-2 text-sm text-[var(--scry-danger-text)]"
                      >
                        <span className="min-w-0 truncate">
                          {titleById.get(entry.titleId)?.name ?? entry.titleId}
                          {entry.reason ? ` — ${entry.reason}` : ""}
                        </span>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          id={`move-titles-deselect-${entry.titleId}`}
                          onClick={() => deselect(entry.titleId)}
                          disabled={starting}
                        >
                          <X className="mr-1 h-3.5 w-3.5" />
                          {t("move.deselect")}
                        </Button>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}

              {deselected.size > 0 ? (
                <p className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2 text-xs text-muted-foreground">
                  <span>
                    {t("move.deselectedCount", { count: deselected.size })}
                  </span>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={restoreSelection}
                    disabled={starting}
                  >
                    {t("move.restoreSelection")}
                  </Button>
                </p>
              ) : null}

              {/* All six classes, always — an empty class is visible, not absent. */}
              <div className="space-y-2">
                {groups.map((group) => (
                  <ClassificationGroup
                    key={group.class}
                    groupClass={group.class}
                    count={toCount(group.count)}
                    entries={group.titles}
                    titleName={(titleId) =>
                      titleById.get(titleId)?.name ?? titleId
                    }
                    currentLibraryName={(entry) =>
                      libraryById.get(entry.sourceLibraryId)?.name ??
                      titleById.get(entry.titleId)?.libraryName ??
                      null
                    }
                    placement={(entry) =>
                      classifiedTitlePlacement(entry, {
                        planFolders: foldersByTitle,
                        rootPathById,
                      })
                    }
                    files={filesByTitle}
                    onDeselect={deselect}
                    deselectDisabled={starting}
                    t={t}
                  />
                ))}
              </div>

              {sections.length > 0 ? (
                <div className="space-y-2">
                  {sections.map((section) => (
                    <details
                      key={section.kind}
                      className="rounded-lg border border-border bg-muted/10 px-3 py-2"
                    >
                      <summary className="cursor-pointer text-sm text-foreground">
                        {t(planKindLabelKey(section.kind))} ·{" "}
                        {toCount(section.itemsTotal)} ·{" "}
                        {formatByteCount(toCount(section.bytesTotal))}
                        {section.complete ? "" : ` · ${t("move.sampledItems")}`}
                      </summary>
                      <ul className="mt-2 space-y-1">
                        {section.items.map((item, index) => (
                          <PlanItemRow
                            key={`${section.kind}-${index}`}
                            item={item}
                          />
                        ))}
                      </ul>
                    </details>
                  ))}
                </div>
              ) : null}

              {freeSpace ? (
                <div className="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm">
                  <HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                  <div className="min-w-0 space-y-0.5">
                    <p className="text-foreground">
                      {freeSpace.sameVolumeMove
                        ? t("move.freeSpaceSameVolume")
                        : t("move.freeSpaceRequired", {
                            required: formatByteCount(
                              toCount(freeSpace.destinationTotalRequiredBytes),
                            ),
                            available:
                              freeSpace.destinationAvailableBytes === null
                                ? t("move.freeSpaceUnknown")
                                : formatByteCount(
                                    toCount(freeSpace.destinationAvailableBytes),
                                  ),
                          })}
                    </p>
                    {freeSpace.sufficient === false ? (
                      <p className="text-xs text-[var(--scry-danger-text)]">
                        {t("move.freeSpaceInsufficient")}
                      </p>
                    ) : null}
                    {!freeSpace.probed ? (
                      <p className="text-xs text-muted-foreground">
                        {t("move.freeSpaceNotProbed")}
                      </p>
                    ) : null}
                    {freeSpace.recycleOnOtherVolume ? (
                      <p className="text-xs text-muted-foreground">
                        {t("move.freeSpaceRecycleOtherVolume", {
                          required: formatByteCount(
                            toCount(freeSpace.recycleRequiredBytes),
                          ),
                        })}
                      </p>
                    ) : null}
                  </div>
                </div>
              ) : null}

              {verification ? (
                <p
                  id="move-titles-verification"
                  className="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm text-foreground"
                >
                  <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                  <span>
                    {verification.applies
                      ? t(
                          verification.depth === "FULL"
                            ? "move.verificationStatementFull"
                            : "move.verificationStatementQuick",
                          {
                            files: toCount(verification.files),
                            bytes: formatByteCount(toCount(verification.bytes)),
                          },
                        )
                      : t("move.verificationNotApplicable")}
                  </span>
                </p>
              ) : null}

              {preview.warnings.length > 0 ? (
                <ul
                  id="move-titles-warnings"
                  className="space-y-1 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-3 text-sm text-[var(--scry-warning-text)]"
                >
                  {preview.warnings.map((warning) => (
                    <li key={warning} className="flex items-start gap-2">
                      <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                      <span>{warning}</span>
                    </li>
                  ))}
                </ul>
              ) : null}

              {preview.confirmation.requirement === "TYPED" ? (
                <div className="space-y-1">
                  <label
                    className="block text-xs font-medium text-muted-foreground"
                    htmlFor="move-titles-typed-confirmation"
                  >
                    {preview.confirmation.typedPrompt ??
                      t("move.typedConfirmationPrompt")}
                  </label>
                  <Input
                    id="move-titles-typed-confirmation"
                    value={typedConfirmation}
                    onChange={(event) =>
                      setTypedConfirmation(event.target.value)
                    }
                    placeholder={preview.confirmation.typedPhrase ?? ""}
                    disabled={starting}
                  />
                </div>
              ) : null}
            </>
          ) : null}

          {startError ? (
            <p
              id="move-titles-start-error"
              className="rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3 text-sm text-[var(--scry-danger-text)]"
            >
              {startError}
            </p>
          ) : null}
        </div>
        )}

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={starting}
          >
            {startedOperationId ? t("label.close") : t("label.cancel")}
          </Button>
          {startedOperationId ? null : (
            <Button
              type="button"
              variant="primary"
              id="move-titles-confirm"
              onClick={() => void handleStart()}
              disabled={!canStart}
            >
              {starting ? (
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
              ) : null}
              {t("move.confirm")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Router-dependent by design, and mounted only after a start succeeds: the
 * dialog itself must render outside a router (the title settings panels are
 * server-rendered in tests without one).
 */
function ViewOperationButton({
  operationId,
  label,
  onNavigated,
}: {
  operationId: string;
  label: string;
  onNavigated: () => void;
}) {
  const navigate = useNavigate();
  return (
    <Button
      type="button"
      variant="primary"
      id="move-titles-view-operation"
      onClick={() => {
        onNavigated();
        void navigate(`/activity?operation=${encodeURIComponent(operationId)}`);
      }}
    >
      {label}
    </Button>
  );
}

function SummaryCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="truncate text-foreground">{value}</dd>
    </div>
  );
}

function PlanItemRow({ item }: { item: LocationPlanItem }) {
  return (
    <li className="min-w-0 text-xs text-muted-foreground">
      <span className="font-[var(--font-code)] break-all">
        {item.sourcePath ?? "—"}
      </span>
      {item.destinationPath ? (
        <>
          {" → "}
          <span className="font-[var(--font-code)] break-all text-foreground">
            {item.destinationPath}
          </span>
        </>
      ) : null}
      {item.detail ? <span className="block">{item.detail}</span> : null}
    </li>
  );
}

type ClassificationGroupProps = {
  groupClass: TitleLocationClass;
  count: number;
  entries: LocationClassifiedTitle[];
  titleName: (titleId: string) => string;
  currentLibraryName: (entry: LocationClassifiedTitle) => string | null;
  placement: (entry: LocationClassifiedTitle) => ClassifiedTitlePlacement;
  files: Map<string, { files: number; bytes: number }>;
  onDeselect: (titleId: string) => void;
  deselectDisabled: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
};

function ClassificationGroup({
  groupClass,
  count,
  entries,
  titleName,
  currentLibraryName,
  placement,
  files,
  onDeselect,
  deselectDisabled,
  t,
}: ClassificationGroupProps) {
  const blocking = classBlocksStart(groupClass);
  return (
    <div
      id={`move-titles-group-${groupClass}`}
      data-count={count}
      className={cn(
        "rounded-lg border px-3 py-2",
        blocking && count > 0
          ? "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]"
          : "border-border bg-muted/10",
      )}
    >
      <p className="flex items-center justify-between gap-2 text-sm font-medium text-foreground">
        <span>{t(classificationLabelKey(groupClass))}</span>
        <Badge tone={count === 0 ? "neutral" : blocking ? "negative" : "info"}>
          {count}
        </Badge>
      </p>
      {count === 0 ? (
        <p className="text-xs text-muted-foreground">{t("move.groupEmpty")}</p>
      ) : (
        <ul className="mt-1 space-y-1">
          {entries.map((entry) => {
            const stats = files.get(entry.titleId);
            const where = placement(entry);
            return (
              <li key={entry.titleId} className="min-w-0 text-xs">
                <span className="flex items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-foreground">
                    {titleName(entry.titleId)}
                  </span>
                  {blocking ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => onDeselect(entry.titleId)}
                      disabled={deselectDisabled}
                    >
                      {t("move.deselect")}
                    </Button>
                  ) : null}
                </span>
                <span className="block text-muted-foreground">
                  {currentLibraryName(entry) ?? "—"}
                  {" · "}
                  <span className="font-[var(--font-code)] break-all">
                    {where.source ?? "—"}
                  </span>
                  <ArrowRight
                    aria-hidden="true"
                    className="mx-1 inline h-3 w-3 align-[-1px]"
                  />
                  <span className="font-[var(--font-code)] break-all text-foreground">
                    {where.destination ?? "—"}
                  </span>
                </span>
                {stats ? (
                  <span className="block text-muted-foreground">
                    {t("move.titleFileSummary", {
                      files: stats.files,
                      size: formatByteCount(stats.bytes),
                    })}
                  </span>
                ) : null}
                {entry.reason ? (
                  <span className="block text-muted-foreground">
                    {entry.reason}
                  </span>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
