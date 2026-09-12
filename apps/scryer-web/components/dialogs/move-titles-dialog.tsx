import * as React from "react";
import { useClient } from "urql";
import {
  ArrowRight,
  HardDrive,
  Loader2,
  Merge,
  ShieldCheck,
  TriangleAlert,
  User,
  X,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "@/components/ui/table";
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
import { locationOperationPreviewQuery } from "@/lib/graphql/queries";
import {
  LocationDialogDismissButton,
  LocationDialogPrimaryButton,
  LocationOperationErrorNotice,
  LocationOperationStartedPanel,
  ViewOperationButton,
  useLocationOperationStart,
} from "@/components/dialogs/location-operation-start";
import {
  adoptionAccounting,
  adoptionBlockedReasonKey,
  ambiguousCandidates,
  blockingTitleRows,
  classBlocksStart,
  classificationLabelKey,
  classifiedTitlePlacement,
  crossLibraryDestinations,
  destinationLibraryDisabledReasonKey,
  eligibleSameLibraryRoots,
  initialMoveStep,
  moveWizardRequestMode,
  isAmbiguousDestinationBlock,
  isCrossLibraryDestination,
  isSameNameWarning,
  mergePreviewsBySourceTitle,
  mergeRoleGroupKey,
  mergeSummaryPresentation,
  movesThroughWizard,
  moveWizardCanAdvance,
  nextMoveStep,
  offersModeSelection,
  orderedPlanKindCounts,
  orderedPlanSections,
  planKindLabelKey,
  populatedClassificationGroups,
  previewCanStart,
  previousMoveStep,
  remainingSelection,
  sameNamedDestinationTitle,
  settledDestinationPick,
  startModeInput,
  toCount,
  transferStatement,
  typedConfirmationSatisfied,
  type AdoptionAccountingSummary,
  type AdoptionFileLine,
  type ClassifiedTitlePlacement,
  type LocationClassifiedTitle,
  type LocationOperationPreview,
  type LocationPlanItem,
  type MergeSummaryPresentation,
  type MoveDestinationKind,
  type MoveWizardStep,
  type RequestableMoveMode,
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

function sortRoots(roots: LibraryRootRecord[]): LibraryRootRecord[] {
  return [...roots].sort((left, right) => {
    if (left.isDefault !== right.isDefault) {
      return left.isDefault ? -1 : 1;
    }
    return left.path.localeCompare(right.path);
  });
}

/**
 * Choose a destination and who moves the files before requesting a plan.
 * A preselected destination opens at the method step. Manual instructions
 * use catalog data; only the managed preview discovers files.
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
  const initialLibraryId = libraries.find((library) =>
    library.roots.some((root) => root.id === initialRootId),
  )?.id ?? soleSourceLibraryId ?? "";

  // A preselected root skips destination selection, never method selection.
  const throughWizard = movesThroughWizard(initialRootId);
  const [step, setStep] = React.useState<MoveWizardStep>(() =>
    initialMoveStep(initialRootId),
  );
  const [kind, setKind] = React.useState<MoveDestinationKind | null>(null);
  const [libraryId, setLibraryId] = React.useState<string>(
    initialLibraryId,
  );
  const [rootId, setRootId] = React.useState<string>(initialRootId ?? "");
  const mode = moveWizardRequestMode(open, step);
  const chosenMethod = React.useRef<RequestableMoveMode | null>(null);
  const [deselected, setDeselected] = React.useState<Set<string>>(new Set());
  const [preview, setPreview] = React.useState<LocationOperationPreview | null>(
    null,
  );
  const [previewLoading, setPreviewLoading] = React.useState(false);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [typedConfirmation, setTypedConfirmation] = React.useState("");
  // Bumped to force a fresh preview when nothing else about the request changed
  // (a refused confirmation, or the user asking to re-preview).
  const [previewNonce, setPreviewNonce] = React.useState(0);

  // The preview effect always fetches network-only; the nonce is what makes it
  // run again when the request itself is unchanged.
  const refreshPreview = React.useCallback(
    () => setPreviewNonce((current) => current + 1),
    [],
  );
  const {
    starting,
    startError,
    planChanged,
    startedOperationId,
    start,
    clearStartError,
    resetAll: resetStartAll,
    clearPlanChanged,
  } = useLocationOperationStart({
    failedMessage: t("move.startFailed"),
    onNeedsFreshPreview: refreshPreview,
    onStarted,
  });

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
  // The wizard's two destination shapes: the source library's other roots, and
  // every library that is not the source one.
  const sameLibraryRoots = React.useMemo(
    () =>
      eligibleSameLibraryRoots(
        sortRoots(libraryById.get(soleSourceLibraryId ?? "")?.roots ?? []),
        titles.map((title) => title.rootFolderId ?? null),
      ),
    [libraryById, soleSourceLibraryId, titles],
  );
  const otherLibraries = React.useMemo(
    () => crossLibraryDestinations(libraries, soleSourceLibraryId),
    [libraries, soleSourceLibraryId],
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
    chosenMethod.current = null;
    if (!open) {
      return;
    }
    setStep(initialMoveStep(initialRootId));
    setKind(null);
    setLibraryId(initialLibraryId);
    setRootId(initialRootId ?? "");
    setDeselected(new Set());
    setPreview(null);
    setPreviewError(null);
    setTypedConfirmation("");
    setPreviewNonce(0);
    resetStartAll();
  }, [open, initialLibraryId, initialRootId, resetStartAll]);

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
    // The wizard's earlier steps are picks, not requests: nothing is planned
    // until the destination is settled and the plan step is on screen.
    // On reopen, the previous step may render before its reset takes effect.
    // Only an explicit choice in this wizard session can start discovery.
    if (!mode || chosenMethod.current !== mode || !rootId || selection.length === 0) {
      setPreview(null);
      setPreviewLoading(false);
      return undefined;
    }
    let active = true;
    setPreview(null);
    setPreviewLoading(true);
    setPreviewError(null);
    clearStartError();
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
            // The mode chooses which plan gets built, so it is part of the
            // request, not a flag applied afterwards (FR-050, FR-051).
            mode,
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
        clearPlanChanged();
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
  }, [client, libraryId, mode, open, previewNonce, rootId, selectionKey, step, t]);

  // Only the classes this selection actually falls into; an empty class is
  // omitted, not shown as an empty box.
  const groups = React.useMemo(
    () => populatedClassificationGroups(preview?.classification),
    [preview],
  );
  // Both ways a title can stop the plan: the classification refusing it, and an
  // adoption refusing its files while the title itself still classifies as a
  // plain root move (FR-052, FR-086).
  const blocked = React.useMemo(() => blockingTitleRows(preview), [preview]);
  const planKindCounts = React.useMemo(
    () => orderedPlanKindCounts(preview?.counts),
    [preview],
  );
  const sections = React.useMemo(
    () => orderedPlanSections(preview?.sections ?? []),
    [preview],
  );
  // FR-071: the plan carries one summary per merging title, so a merging row
  // can state exactly what the engine would do rather than a generic warning.
  const mergesByTitle = React.useMemo(
    () => mergePreviewsBySourceTitle(preview),
    [preview],
  );
  // FR-051's accounting, read back out of the plan items the adoption planner
  // already emitted. Null for every plan that is not an adoption.
  const adoption = React.useMemo(() => adoptionAccounting(preview), [preview]);

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
    for (const folder of preview?.folders ?? []) folders.set(folder.titleId, folder);
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
    await start({
      titleIds: preview.selection,
      destination: { libraryId: libraryId || null, rootId },
      // Read off the preview, not off the control: a confirmation states the
      // mode the plan in hand was built from, so a mode the user changed after
      // previewing meets the fingerprint refusal rather than starting the
      // other workflow.
      mode: startModeInput(preview),
      planFingerprint: preview.planFingerprint,
      typedConfirmation:
        preview.confirmation.requirement === "TYPED" ? typedConfirmation : null,
    });
  }, [libraryId, preview, rootId, start, typedConfirmation]);

  const rePreview = React.useCallback(() => {
    clearPlanChanged();
    refreshPreview();
  }, [clearPlanChanged, refreshPreview]);

  const destinationDisabledReasonKey = React.useMemo(
    () => destinationLibraryDisabledReasonKey(sourceLibraryIds),
    [sourceLibraryIds],
  );
  // A selection spanning several libraries has no single library to stay
  // inside, so "another root" is not a move it can express (FR-017).
  const mixedSourceLibraries = sourceLibraryIds.length > 1;
  // A library with no other root has nowhere to move the selection inside
  // itself; the kind step says so up front instead of leading to an empty
  // picker one click later.
  const noOtherRoots = !mixedSourceLibraries && sameLibraryRoots.length === 0;
  const rootKindUnavailable = mixedSourceLibraries || noOtherRoots;

  const chooseKind = React.useCallback(
    (value: MoveDestinationKind) => {
      setKind(value);
      // Staying inside the library pins the destination library to the source;
      // leaving it starts from no library at all, so the picker is a choice.
      setLibraryId(value === "root" ? soleSourceLibraryId ?? "" : "");
      setRootId("");
    },
    [soleSourceLibraryId],
  );

  const goNext = React.useCallback(() => {
    setStep((current) => nextMoveStep(current));
  }, []);

  const [manualPage, setManualPage] = React.useState(0);
  const [manualMappingChanged, setManualMappingChanged] = React.useState(false);
  React.useEffect(() => {
    if (open && step === "manual" && planChanged) {
      setManualMappingChanged(true);
      setPreview(null);
      clearPlanChanged();
      refreshPreview();
    }
  }, [open, step, planChanged, clearPlanChanged, refreshPreview]);
  const chooseMethod = (selected: RequestableMoveMode) => {
    chosenMethod.current = selected;
    setPreview(null);
    setPreviewError(null);
    setManualPage(0);
    setManualMappingChanged(false);
    setStep(selected === "USER_MOVED_FILES" ? "manual" : "plan");
  };

  const goBack = React.useCallback(() => {
    setStep((current) => {
      const previous = previousMoveStep(current);
      if (previous === "kind") {
        // Going back past the destination step discards the destination; the
        // kind itself stays selected, because that is the step's own control.
        // The library follows the kind the way `chooseKind` set it, so a
        // transfer never comes back with the source library silently pinned.
        setLibraryId(kind === "root" ? soleSourceLibraryId ?? "" : "");
        setRootId("");
      }
      return previous;
    });
  }, [kind, soleSourceLibraryId]);

  const canAdvance = moveWizardCanAdvance(step, { kind, libraryId, rootId });
  // The destination step's root list: the source library's other roots for a
  // same-library move, the chosen library's roots for a transfer.
  const wizardRoots = kind === "root" ? sameLibraryRoots : destinationRoots;

  // What each destination picker actually offers. The defaults below read from
  // these, so a default can never be a value its own list lacks.
  const libraryOptions =
    step === "destination" && kind === "library" ? otherLibraries : libraries;
  const rootOptions = step === "destination" ? wizardRoots : destinationRoots;
  const pickingDestination = open && step === "destination";

  // The pickers open on a usable destination instead of a placeholder: the
  // first library on offer, then that library's first root. This settles
  // whenever the options change rather than only at open, because `libraries`
  // can arrive after the dialog is already up, and because changing the
  // library clears the root to be refilled from the new library.
  React.useEffect(() => {
    if (!pickingDestination) {
      return;
    }
    setLibraryId((current) => settledDestinationPick(libraryOptions, current));
  }, [libraryOptions, pickingDestination]);

  React.useEffect(() => {
    if (!pickingDestination) {
      return;
    }
    setRootId((current) => settledDestinationPick(rootOptions, current));
  }, [pickingDestination, rootOptions]);

  // Naming the destination library is what makes a cross-library transfer
  // readable: every row otherwise states only paths (FR-016, US6).
  const libraryName = React.useCallback(
    (candidateLibraryId: string) =>
      libraryById.get(candidateLibraryId)?.name ?? null,
    [libraryById],
  );
  const titleName = React.useCallback(
    (titleId: string) => titleById.get(titleId)?.name ?? null,
    [titleById],
  );
  const crossLibrary = isCrossLibraryDestination(libraryId, sourceLibraryIds);

  const totalFiles = toCount(preview?.counts.filesTotal);
  const totalBytes = toCount(preview?.counts.bytesTotal);
  const freeSpace = preview?.freeSpace ?? null;
  // The free-space block appears only when it has a figure or a caveat to
  // state: a same-volume rename that probed fine has nothing to say, and a
  // plan that moves no files has no space to need.
  const freeSpaceNote =
    freeSpace &&
    totalFiles > 0 &&
    (!freeSpace.sameVolumeMove ||
      freeSpace.sufficient === false ||
      !freeSpace.probed ||
      freeSpace.recycleOnOtherVolume)
      ? freeSpace
      : null;
  const verification = preview?.verification ?? null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="move-titles-dialog" className="sm:max-w-4xl">
        <DialogHeader>
          <DialogTitle>{t("move.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {step === "kind"
              ? t("move.wizardKindDescription")
              : t("move.dialogDescription", { count: titles.length })}
          </DialogDescription>
        </DialogHeader>

        {startedOperationId ? (
          <LocationOperationStartedPanel
            idPrefix="move-titles"
            heading={t("move.startedHeading")}
          />
        ) : step === "kind" ? (
          /* Step 1: which of the two moves this is. The destination controls
             differ per kind, so the kind is asked first rather than inferred
             from a picker that can only express one of them. */
          <div className="space-y-2">
            <RadioGroup
              value={kind ?? ""}
              onValueChange={(value) => chooseKind(value as MoveDestinationKind)}
              className="space-y-2"
            >
              <label
                className={cn(
                  "flex items-start gap-3 rounded-lg border border-border px-3 py-3 text-sm",
                  rootKindUnavailable
                    ? "opacity-60"
                    : "cursor-pointer hover:bg-muted/20",
                )}
              >
                <RadioGroupItem
                  id="move-titles-kind-root"
                  value="root"
                  disabled={rootKindUnavailable}
                  className="mt-0.5"
                />
                <span className="min-w-0">
                  <span className="block font-medium text-foreground">
                    {t("move.kindRootHeading")}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {t("move.kindRootHelp")}
                  </span>
                  {mixedSourceLibraries ? (
                    <span
                      id="move-titles-mixed-libraries"
                      className="mt-1 block text-xs text-[var(--scry-warning-text)]"
                    >
                      {t("move.destinationMixedSourceLibraries")}
                    </span>
                  ) : null}
                  {noOtherRoots ? (
                    <span
                      id="move-titles-no-other-roots"
                      className="mt-1 block text-xs text-[var(--scry-warning-text)]"
                    >
                      {t("move.noOtherRoots")}
                    </span>
                  ) : null}
                </span>
              </label>

              <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border px-3 py-3 text-sm hover:bg-muted/20">
                <RadioGroupItem
                  id="move-titles-kind-library"
                  value="library"
                  className="mt-0.5"
                />
                <span className="min-w-0">
                  <span className="block font-medium text-foreground">
                    {t("move.kindLibraryHeading")}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {t("move.kindLibraryHelp")}
                  </span>
                </span>
              </label>
            </RadioGroup>
          </div>
        ) : step === "destination" ? (
          /* Step 2: where. Same-library moves pick a root the selection is not
             already on; cross-library moves pick the library first, because the
             roots on offer belong to it. */
          <div className="space-y-3">
            {kind === "library" ? (
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
                >
                  <SelectTrigger
                    id="move-titles-destination-library"
                    className="h-9 w-full"
                  >
                    <SelectValue placeholder={t("move.destinationLibrary")} />
                  </SelectTrigger>
                  <SelectContent>
                    {otherLibraries.map((library) => (
                      <SelectItem
                        key={library.id}
                        value={library.id}
                        disabled={destinationDisabledReasonKey !== null}
                      >
                        {destinationDisabledReasonKey === null
                          ? library.name
                          : `${library.name} — ${t(destinationDisabledReasonKey)}`}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {mixedSourceLibraries ? (
                  <p
                    id="move-titles-mixed-libraries"
                    className="mt-1 text-xs text-[var(--scry-warning-text)]"
                  >
                    {t("move.destinationMixedSourceLibraries")}
                  </p>
                ) : null}
                {crossLibrary ? (
                  <p
                    id="move-titles-cross-library-notice"
                    className="mt-1 text-xs text-muted-foreground"
                  >
                    {t("move.destinationCrossLibraryNotice", {
                      library: libraryName(libraryId) ?? libraryId,
                    })}
                  </p>
                ) : null}
              </div>
            ) : null}

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
                disabled={wizardRoots.length === 0}
              >
                <SelectTrigger
                  id="move-titles-destination-root"
                  className="h-9 w-full font-[var(--font-code)] text-sm"
                >
                  <SelectValue
                    placeholder={t("move.destinationRootPlaceholder")}
                  />
                </SelectTrigger>
                <SelectContent>
                  {wizardRoots.map((root) => (
                    <SelectItem key={root.id} value={root.id}>
                      {root.path}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {/* The honest answer when the library has nowhere else to put the
                  selection, rather than an empty picker that does nothing. */}
              {kind === "root" && wizardRoots.length === 0 ? (
                <p
                  id="move-titles-no-other-roots"
                  className="mt-1 text-xs text-[var(--scry-warning-text)]"
                >
                  {t("move.noOtherRoots")}
                </p>
              ) : null}
            </div>
          </div>
        ) : step === "method" ? (
          /* Who does the work, given a face each: Scryer's own mark against a
             person. `has-[>svg]:px-6` restates the padding the button's default
             size tightens as soon as it holds an icon -- without it the manual
             card, whose icon is an svg, would sit narrower than the managed
             one, whose icon is an img. */
          <div className="grid min-h-48 content-center gap-3 sm:grid-cols-2">
            <Button variant="outline" className="h-auto flex-col gap-3 whitespace-normal p-6 text-base has-[>svg]:px-6" onClick={() => chooseMethod("MOVE_WITH_SCRYER")}>
              <img src={`${import.meta.env.BASE_URL}scryer-icon-192.png`} alt="" aria-hidden="true" className="size-10" />
              {t("move.methodManaged")}
            </Button>
            <Button variant="outline" className="h-auto flex-col gap-3 whitespace-normal p-6 text-base has-[>svg]:px-6" onClick={() => chooseMethod("USER_MOVED_FILES")}>
              <User aria-hidden="true" className="size-10 text-[var(--scry-muted2)]" />
              {t("move.methodManual")}
            </Button>
          </div>
        ) : previewLoading ? (
          <div role="status" aria-live="polite" className="flex min-h-64 flex-col items-center justify-center gap-4">
            <Loader2 aria-hidden="true" className="h-9 w-9 animate-spin text-primary" />
            <p className="text-sm text-muted-foreground">{t(step === "manual" ? "move.manualLoading" : "move.gatheringInfo")}</p>
          </div>
        ) : step === "manual" ? (
          <div className="max-h-[65vh] min-h-64 space-y-4 overflow-y-auto">
            <p className="font-medium">{t("move.manualInstructions")}</p>
            <p className="text-sm text-muted-foreground">{t("move.manualStructure")}</p>
            <LocationOperationErrorNotice id="move-manual-preview-error" message={previewError ?? startError} />
            {manualMappingChanged && <p role="status" className="text-sm text-[var(--scry-warning-text)]">{t("move.manualMappingChanged")}</p>}
            {(previewError || planChanged) && <Button variant="outline" onClick={rePreview}>{t("move.retryInstructions")}</Button>}
            <Table>
              <TableHeader><TableRow><TableHead>{t("label.title")}</TableHead><TableHead>{t("move.manualSource")}</TableHead><TableHead>{t("move.manualDestination")}</TableHead></TableRow></TableHeader>
              <TableBody>{selection.slice(manualPage * 50, (manualPage + 1) * 50).map((id) => {
                const folders = foldersByTitle.get(id);
                const summary = mergesByTitle.get(id);
                const entry = groups.flatMap((group) => group.titles).find((row) => row.titleId === id);
                return <React.Fragment key={id}><TableRow>
                  <TableCell>{titleName(id) ?? id}</TableCell>
                  <TableCell className="break-all">{folders?.source ?? entry?.sourceFolderPath ?? "—"}</TableCell>
                  <TableCell className="break-all">{folders?.destination ?? (entry?.class === "NO_OP" ? entry.sourceFolderPath : null) ?? "—"}</TableCell>
                </TableRow>{summary && entry && <TableRow><TableCell colSpan={3}><MergeNote summary={mergeSummaryPresentation(entry, summary, { resolveTitleName: titleName })} t={t} /></TableCell></TableRow>}</React.Fragment>;
              })}</TableBody>
            </Table>
            {selection.length > 50 && <div className="flex items-center justify-between text-sm">
              <Button variant="outline" disabled={manualPage === 0} onClick={() => setManualPage((page) => page - 1)}>{t("move.transferPrevious")}</Button>
              <span>{manualPage * 50 + 1}–{Math.min((manualPage + 1) * 50, selection.length)} / {selection.length}</span>
              <Button variant="outline" disabled={(manualPage + 1) * 50 >= selection.length} onClick={() => setManualPage((page) => page + 1)}>{t("move.transferNext")}</Button>
            </div>}
            {preview?.warnings.map((warning) => <p key={warning} className="text-sm text-[var(--scry-warning-text)]">{warning}</p>)}
            {blocked.map((row) => <p key={row.titleId} className="text-sm text-[var(--scry-danger-text)]">{row.reason}</p>)}
          </div>
        ) : (
        <div className="max-h-[65vh] min-h-64 space-y-4 overflow-y-auto pr-1">
          <div className="flex items-start justify-between gap-4 rounded-lg border border-border p-3">
            <div className="min-w-0 text-sm">
              <p className="font-medium">{libraryName(libraryId) ?? libraryId}</p>
              <p className="break-all text-muted-foreground">{rootPathById.get(rootId) ?? rootId}</p>
            </div>
            <Button variant="outline" size="sm" disabled={starting} onClick={() => {
              setPreview(null);
              setKind(libraryId === soleSourceLibraryId ? "root" : "library");
              setStep("destination");
            }}>{t("move.changeDestination")}</Button>
          </div>

          {/* FR-051: adoption states what it found before it states what it
              would do, so a refusal is legible without opening the plan. */}
          {adoption ? (
            <AdoptionAccountingPanel accounting={adoption} t={t} />
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

          <LocationOperationErrorNotice
            id="move-titles-preview-error"
            message={previewError}
          />

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
                {totalFiles > 0 ? (
                  <>
                    <SummaryCell
                      label={t("move.summaryFiles")}
                      value={String(totalFiles)}
                    />
                    <SummaryCell
                      label={t("move.summarySize")}
                      value={formatByteCount(totalBytes)}
                    />
                  </>
                ) : null}
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
                    {blocked.map((row) => {
                      const adoptionReasonKey = adoptionBlockedReasonKey(
                        row.adoptionReasonCode,
                      );
                      return (
                        <li
                          key={row.titleId}
                          className="space-y-1 text-sm text-[var(--scry-danger-text)]"
                        >
                          <span className="flex items-center justify-between gap-2">
                            <span className="min-w-0 truncate">
                              {titleById.get(row.titleId)?.name ?? row.titleId}
                              {row.reason ? ` — ${row.reason}` : ""}
                            </span>
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              id={`move-titles-deselect-${row.titleId}`}
                              onClick={() => deselect(row.titleId)}
                              disabled={starting}
                            >
                              <X className="mr-1 h-3.5 w-3.5" />
                              {t("move.deselect")}
                            </Button>
                          </span>
                          {/* FR-052's refusal names its files in the accounting
                              panel; here it names the title the user is being
                              told to deselect. */}
                          {adoptionReasonKey ? (
                            <span
                              id={`move-titles-adoption-blocked-${row.titleId}`}
                              className="block text-xs"
                            >
                              {t(adoptionReasonKey)}
                            </span>
                          ) : null}
                          {row.entry ? (
                            <BlockedIdentityDetail
                              entry={row.entry}
                              titleName={titleName}
                              t={t}
                            />
                          ) : null}
                        </li>
                      );
                    })}
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

              {/* Only the classes with titles in them; the group counts still
                  sum to the selection, so nothing reads as dropped. */}
              {groups.length > 0 ? (
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
                      destinationLibraryName={libraryName}
                      mergeSummary={(entry) =>
                        mergeSummaryPresentation(
                          entry,
                          mergesByTitle.get(entry.titleId) ?? null,
                          { resolveTitleName: titleName },
                        )
                      }
                      onDeselect={deselect}
                      deselectDisabled={starting}
                      t={t}
                    />
                  ))}
                </div>
              ) : null}

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

              {freeSpaceNote ? (
                <div className="flex items-start gap-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm">
                  <HardDrive className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
                  <div className="min-w-0 space-y-0.5">
                    {freeSpaceNote.sameVolumeMove ? null : (
                      <p className="text-foreground">
                        {t("move.freeSpaceRequired", {
                          required: formatByteCount(
                            toCount(freeSpaceNote.destinationTotalRequiredBytes),
                          ),
                          available:
                            freeSpaceNote.destinationAvailableBytes === null
                              ? t("move.freeSpaceUnknown")
                              : formatByteCount(
                                  toCount(freeSpaceNote.destinationAvailableBytes),
                                ),
                        })}
                      </p>
                    )}
                    {freeSpaceNote.sufficient === false ? (
                      <p className="text-xs text-[var(--scry-danger-text)]">
                        {t("move.freeSpaceInsufficient")}
                      </p>
                    ) : null}
                    {!freeSpaceNote.probed ? (
                      <p className="text-xs text-muted-foreground">
                        {t("move.freeSpaceNotProbed")}
                      </p>
                    ) : null}
                    {freeSpaceNote.recycleOnOtherVolume ? (
                      <p className="text-xs text-muted-foreground">
                        {t("move.freeSpaceRecycleOtherVolume", {
                          required: formatByteCount(
                            toCount(freeSpaceNote.recycleRequiredBytes),
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

          <LocationOperationErrorNotice
            id="move-titles-start-error"
            message={startError}
          />
        </div>
        )}

        <DialogFooter>
          <LocationDialogDismissButton
            id="move-titles-dismiss"
            label={startedOperationId ? t("label.close") : t("label.cancel")}
            disabled={starting}
            onDismiss={() => onOpenChange(false)}
          />
          {startedOperationId ? (
            <ViewOperationButton
              id="move-titles-view-operation"
              operationId={startedOperationId}
              label={t("move.viewInActivity")}
              onNavigated={() => onOpenChange(false)}
            />
          ) : (
            <>
              {/* Back exists wherever there is a step behind this one: always on
                  the destination step, and on the plan step only when the
                  wizard is what got us there (a bulk edit opened on the plan). */}
              {step === "destination" || step === "plan" || step === "manual" || (step === "method" && throughWizard) ? (
                <Button
                  id="move-titles-back"
                  type="button"
                  variant="outline"
                  onClick={goBack}
                  disabled={starting}
                >
                  {t("move.back")}
                </Button>
              ) : null}
              {step === "plan" || step === "manual" ? (
                <LocationDialogPrimaryButton
                  id="move-titles-confirm"
                  label={t(step === "manual" ? "move.next" : "move.confirm")}
                  busy={starting}
                  disabled={!canStart}
                  onClick={() => void handleStart()}
                />
              ) : step !== "method" ? (
                <Button
                  id="move-titles-next"
                  type="button"
                  variant="primary"
                  onClick={goNext}
                  disabled={!canAdvance}
                >
                  {t("move.next")}
                </Button>
              ) : null}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
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
  destinationLibraryName: (libraryId: string) => string | null;
  mergeSummary: (
    entry: LocationClassifiedTitle,
  ) => MergeSummaryPresentation | null;
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
  destinationLibraryName,
  mergeSummary,
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
        <Badge tone={blocking ? "negative" : "info"}>{count}</Badge>
      </p>
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
              <TransferNote
                entry={entry}
                destinationLibraryName={destinationLibraryName}
                t={t}
              />
              <MergeNote summary={mergeSummary(entry)} t={t} />
              <SameNameWarning
                entry={entry}
                destinationLibraryName={destinationLibraryName}
                t={t}
              />
              {entry.reason ? (
                <span className="block text-muted-foreground">
                  {entry.reason}
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/**
 * FR-016: a title crossing libraries says so in words, naming the library it
 * lands in. The paths above it change root; only this line changes library.
 */
function TransferNote({
  entry,
  destinationLibraryName,
  t,
}: {
  entry: LocationClassifiedTitle;
  destinationLibraryName: (libraryId: string) => string | null;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const statement = transferStatement(entry, destinationLibraryName);
  if (!statement) {
    return null;
  }
  return (
    <span
      id={`move-titles-transfer-note-${entry.titleId}`}
      className="block text-foreground"
    >
      {t("move.transferIntoLibrary", {
        library:
          statement.destinationLibraryName ?? statement.destinationLibraryId,
      })}
    </span>
  );
}

/**
 * FR-055 never merges by name. When the destination already holds a title with
 * the same name but no shared identity, the transfer still happens — and the
 * user has to see, before confirming, that they are about to end up with two
 * same-named titles rather than one merged one.
 */
function SameNameWarning({
  entry,
  destinationLibraryName,
  t,
}: {
  entry: LocationClassifiedTitle;
  destinationLibraryName: (libraryId: string) => string | null;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  if (!isSameNameWarning(entry)) {
    return null;
  }
  const sameNamed = sameNamedDestinationTitle(entry);
  const library =
    destinationLibraryName(entry.destinationLibraryId) ??
    entry.destinationLibraryId;
  return (
    <span
      id={`move-titles-same-name-warning-${entry.titleId}`}
      className="flex items-start gap-1 text-[var(--scry-warning-text)]"
    >
      <TriangleAlert aria-hidden="true" className="mt-0.5 h-3 w-3 shrink-0" />
      <span>
        {t("move.sameNameWarning", {
          library,
          title: sameNamed?.name ?? sameNamed?.titleId ?? "",
        })}
      </span>
    </span>
  );
}

/**
 * FR-071: a merging row says, in words, that the two titles become one.
 *
 * This is the destructive-adjacent case in the whole workflow — the moving
 * title's identity is absorbed and the destination's settings win — so the
 * statement is always visible, and everything the engine would actually do
 * (what the surviving title takes over, every role change, and how much retires
 * with the merging title) sits one disclosure below it.
 */
function MergeNote({
  summary,
  t,
}: {
  summary: MergeSummaryPresentation | null;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  if (!summary) {
    return null;
  }
  const titleId = summary.statement.sourceTitleId;
  const destination =
    summary.statement.destinationTitleName ??
    summary.statement.destinationTitleId;
  return (
    <div className="space-y-1">
      <p
        id={`move-titles-merge-statement-${titleId}`}
        className="flex items-start gap-1 text-[var(--scry-warning-text)]"
      >
        <Merge aria-hidden="true" className="mt-0.5 h-3 w-3 shrink-0" />
        <span>{t("move.mergeStatement", { title: destination })}</span>
      </p>

      {summary.blocked ? (
        <div
          id={`move-titles-merge-blocked-${titleId}`}
          className="space-y-0.5 text-[var(--scry-danger-text)]"
        >
          <p>{t("move.mergeBlockedHeading")}</p>
          {summary.blockedRecords.map((record) => (
            <p key={`${record.table}-${record.sourceId}`} className="ml-3">
              {record.detail}
            </p>
          ))}
        </div>
      ) : null}

      {summary.empty ? (
        <p
          id={`move-titles-merge-empty-${titleId}`}
          className="text-muted-foreground"
        >
          {t("move.mergeNoDetails")}
        </p>
      ) : (
        <details
          id={`move-titles-merge-summary-${titleId}`}
          className="rounded-lg border border-border bg-muted/10 px-2 py-1"
        >
          <summary className="cursor-pointer text-foreground">
            {t("move.mergeSummaryHeading")}
          </summary>
          <div className="mt-1 space-y-1.5">
            <p id={`move-titles-merge-carried-${titleId}`}>
              {t("move.mergeCarried", {
                files: summary.mediaFilesRepointed,
                history: summary.historyRowsCarried,
              })}
            </p>

            {/* FR-070: every role change is named, and a demotion says so.
                The reason is said once per group; the files are listed under
                it by episode and name, the way a person would read them. */}
            {summary.roleChangeGroups.length > 0 ? (
              <div
                id={`move-titles-merge-role-changes-${titleId}`}
                className="space-y-1"
              >
                <p className="text-foreground">
                  {summary.demotionCount > 0
                    ? t("move.mergeRoleChangesHeading", {
                        demotions: summary.demotionCount,
                      })
                    : t("move.mergeRoleChangesHeadingPlain")}
                </p>
                {summary.roleChangeGroups.map((group) => (
                  <div
                    key={`${group.reason}-${group.titleSlot ? "title" : "episode"}`}
                    className={
                      group.demotion
                        ? "text-[var(--scry-warning-text)]"
                        : undefined
                    }
                  >
                    <p>{t(mergeRoleGroupKey(group), { title: destination })}</p>
                    <ul className="ml-4 list-disc text-muted-foreground">
                      {group.lines.map((change) => (
                        <li
                          key={change.fileId}
                          id={`move-titles-merge-role-change-${titleId}-${change.fileId}`}
                        >
                          {change.episodeLabel ? (
                            <span className="text-foreground">
                              {change.episodeLabel}{" · "}
                            </span>
                          ) : null}
                          {change.fileName ??
                            t("move.mergeRoleUnnamedFile", {
                              id: change.fileId,
                            })}
                        </li>
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            ) : null}

            {/* FR-064: everything else on the merging title goes with it. */}
            {summary.sourceRecordsDropped > 0 ? (
              <p
                id={`move-titles-merge-dropped-${titleId}`}
                className="text-[var(--scry-warning-text)]"
              >
                {t("move.mergeDropped", {
                  count: summary.sourceRecordsDropped,
                })}
              </p>
            ) : null}
          </div>
        </details>
      )}
    </div>
  );
}

/**
 * The one identity outcome that still stops a transfer from starting: several
 * destination titles share this title's identity and Scryer will not guess.
 *
 * The candidates are named here with the identities they share, which is what
 * makes them tellable apart — but there is no input for picking one yet, so the
 * row's only affordance stays the Deselect the blocked list already offers and
 * the prose says to resolve the identity in the destination library first.
 */
function BlockedIdentityDetail({
  entry,
  titleName,
  t,
}: {
  entry: LocationClassifiedTitle;
  titleName: (titleId: string) => string | null;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  if (!isAmbiguousDestinationBlock(entry)) {
    return null;
  }
  const candidates = ambiguousCandidates(entry, titleName);
  return (
    <div
      id={`move-titles-ambiguous-${entry.titleId}`}
      className="space-y-1 text-xs text-[var(--scry-danger-text)]"
    >
      <p>{t("move.ambiguousDestinationHelp")}</p>
      {candidates.length === 0 ? null : (
        <ul className="ml-4 list-disc space-y-0.5">
          {candidates.map((candidate) => (
            <li
              key={candidate.titleId}
              id={`move-titles-ambiguous-candidate-${entry.titleId}-${candidate.titleId}`}
            >
              {candidate.name ? (
                <>
                  <span>{candidate.name}</span>{" "}
                  <span className="font-[var(--font-code)] break-all opacity-80">
                    {candidate.titleId}
                  </span>
                </>
              ) : (
                <span className="font-[var(--font-code)] break-all">
                  {candidate.titleId}
                </span>
              )}
              {candidate.sharedIdentities.length > 0 ? (
                <span
                  id={`move-titles-ambiguous-shared-${entry.titleId}-${candidate.titleId}`}
                  className="block font-[var(--font-code)] break-all opacity-80"
                >
                  {t("move.ambiguousCandidateShared", {
                    identities: candidate.sharedIdentities.join(", "),
                  })}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * FR-051's accounting for an adoption: how much of the selection was found at
 * the destination, what is still unaccounted for, and what is there that no
 * tracked file claims.
 *
 * The four counts are the summary; the unresolved files are listed by name
 * underneath, because FR-052's refusal is only actionable if the user can see
 * which files it is about. The FR-053 line about source cleanup is stated
 * before confirmation rather than after it.
 */
function AdoptionAccountingPanel({
  accounting,
  t,
}: {
  accounting: AdoptionAccountingSummary;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const unresolvedCount =
    accounting.missing.length +
    accounting.ambiguous.length +
    accounting.unreadable.length;
  return (
    <div
      id="move-titles-adoption"
      className={cn(
        "space-y-3 rounded-lg border px-3 py-3",
        accounting.blocks
          ? "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]"
          : "border-border bg-muted/20",
      )}
    >
      <p className="text-sm font-medium text-foreground">
        {t("move.adoptionHeading")}
      </p>

      {/* FR-051's four-way accounting, stating only the ways that hold files. */}
      <dl className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
        {accounting.accountedForFiles > 0 ? (
          <div id="move-titles-adoption-accounted">
            <dt className="text-xs text-muted-foreground">
              {t("move.adoptionAccountedFor")}
            </dt>
            <dd className="text-foreground">
              {accounting.accountedForFiles}
              {accounting.accountedForBytes > 0 ? (
                <span className="ml-1 text-xs text-muted-foreground">
                  {formatByteCount(accounting.accountedForBytes)}
                </span>
              ) : null}
            </dd>
          </div>
        ) : null}
        {accounting.missing.length > 0 ? (
          <div id="move-titles-adoption-missing">
            <dt className="text-xs text-muted-foreground">
              {t("move.adoptionMissing")}
            </dt>
            <dd className="text-[var(--scry-danger-text)]">
              {accounting.missing.length}
            </dd>
          </div>
        ) : null}
        {accounting.ambiguous.length > 0 ? (
          <div id="move-titles-adoption-ambiguous">
            <dt className="text-xs text-muted-foreground">
              {t("move.adoptionAmbiguous")}
            </dt>
            <dd className="text-[var(--scry-danger-text)]">
              {accounting.ambiguous.length}
            </dd>
          </div>
        ) : null}
        {accounting.additionalFiles > 0 ? (
          <div id="move-titles-adoption-additional">
            <dt className="text-xs text-muted-foreground">
              {t("move.adoptionAdditional")}
            </dt>
            <dd className="text-foreground">
              {accounting.additionalFiles}
              {accounting.additionalBytes > 0 ? (
                <span className="ml-1 text-xs text-muted-foreground">
                  {formatByteCount(accounting.additionalBytes)}
                </span>
              ) : null}
            </dd>
          </div>
        ) : null}
      </dl>

      {accounting.blocks ? (
        <div
          id="move-titles-adoption-unresolved"
          className="space-y-2 text-sm text-[var(--scry-danger-text)]"
        >
          <p className="flex items-start gap-2">
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              {t("move.adoptionBlocked", { count: unresolvedCount })}
            </span>
          </p>
          <AdoptionFileList
            id="move-titles-adoption-missing-files"
            heading={t("move.adoptionMissingHeading")}
            lines={accounting.missing}
          />
          <AdoptionFileList
            id="move-titles-adoption-ambiguous-files"
            heading={t("move.adoptionAmbiguousHeading")}
            lines={accounting.ambiguous}
          />
          <AdoptionFileList
            id="move-titles-adoption-unreadable-files"
            heading={t("move.adoptionUnreadableHeading")}
            lines={accounting.unreadable}
          />
          {accounting.listingComplete ? null : (
            <p id="move-titles-adoption-sampled" className="text-xs">
              {t("move.adoptionSampled")}
            </p>
          )}
        </div>
      ) : null}

      {accounting.additional.length > 0 ? (
        <AdoptionFileList
          id="move-titles-adoption-additional-files"
          heading={t("move.adoptionAdditionalHeading")}
          lines={accounting.additional}
          tone="muted"
        />
      ) : null}

      {accounting.sourceCleanupNotice ? (
        <p
          id="move-titles-adoption-source-cleanup"
          className="text-xs text-muted-foreground"
        >
          {t("move.adoptionSourceCleanup")}
        </p>
      ) : null}
    </div>
  );
}

/** One named group of adoption file lines: path first, then what was concluded. */
function AdoptionFileList({
  id,
  heading,
  lines,
  tone = "danger",
}: {
  id: string;
  heading: string;
  lines: AdoptionFileLine[];
  tone?: "danger" | "muted";
}) {
  if (lines.length === 0) {
    return null;
  }
  return (
    <div id={id} className={tone === "muted" ? "text-muted-foreground" : undefined}>
      <p className="text-xs font-medium">{heading}</p>
      <ul className="ml-4 list-disc space-y-0.5 text-xs">
        {lines.map((line, index) => (
          <li key={`${line.sourcePath ?? line.destinationPath ?? "line"}-${index}`}>
            <span className="font-[var(--font-code)] break-all">
              {line.sourcePath ?? line.destinationPath ?? ""}
            </span>
            {line.detail ? (
              <span className="block opacity-80">{line.detail}</span>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
