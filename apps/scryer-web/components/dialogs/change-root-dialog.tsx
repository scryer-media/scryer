import * as React from "react";
import { useClient } from "urql";
import { ArrowRight, HardDrive, Loader2, ShieldCheck, TriangleAlert } from "lucide-react";

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
import type { Translate } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { locationRootScopePreviewQuery } from "@/lib/graphql/queries";
import {
  toCount,
  typedConfirmationSatisfied,
  type LocationOperationPreview,
} from "@/lib/location-operations";
import {
  LocationDialogDismissButton,
  LocationDialogPrimaryButton,
  LocationOperationErrorNotice,
  LocationOperationStartedPanel,
  useLocationOperationStart,
} from "@/components/dialogs/location-operation-start";
import {
  accountingCloses,
  changedFolderNames,
  changedFolderNamesComplete,
  consolidationGroups,
  rootIdentityStatement,
  rootPlanCanStart,
  rootReasonKey,
  rootRefusalCode,
  rootRefusalMessageKey,
  retirementBlockerKey,
  type LocationRootContentBucket,
  type LocationRootContentInventory,
  type LocationRootRetirementContract,
  type LocationRootScopePreview,
  type LocationSampledPaths,
  type LocationTitleAccounting,
  type RootDestinationKind,
} from "@/lib/root-location-operations";
import { formatByteCount } from "@/lib/utils/activity-utils";
import { cn } from "@/lib/utils";

/** A configured root of the library the dialog was opened from. */
export type ChangeRootTarget = {
  id: string;
  path: string;
  isDefault: boolean;
};

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Library both roots belong to. A consolidation never crosses libraries. */
  libraryId: string;
  /** The root whose row the action was opened from. */
  root: ChangeRootTarget;
  /** Every other configured root of this library, as consolidation targets. */
  otherRoots: ChangeRootTarget[];
  /** Fires with the accepted operation id after a successful confirm. */
  onStarted?: (operationId: string) => void;
};

/**
 * FR-020's single settings action: change this root to a new, unconfigured path
 * (US4) or to another configured root of the same library (US5).
 *
 * One query and one payload for both. The radio below is how the *user*
 * expresses a destination — type a path, or pick a root — and the server is
 * what decides which branch that destination actually is: a typed path that is
 * already a root of this library is planned as a fold, with no notice to read
 * and no branch to flip. Which branch came back is read off the payload, whose
 * variant-only sections are null for the other one.
 *
 * There is no selection and no exclude affordance anywhere in this dialog: a
 * root-scoped operation takes every title assigned to the root, and a blocked
 * title stops it until it is repaired (FR-023).
 */
export function ChangeRootDialog({
  open,
  onOpenChange,
  libraryId,
  root,
  otherRoots,
  onStarted,
}: Props) {
  const client = useClient();
  const t = useTranslate();

  const [destinationKind, setDestinationKind] =
    React.useState<RootDestinationKind>("NEW_PATH");
  const [destinationPath, setDestinationPath] = React.useState("");
  const [destinationRootId, setDestinationRootId] = React.useState("");
  const [preview, setPreview] = React.useState<LocationRootScopePreview | null>(
    null,
  );
  const [previewing, setPreviewing] = React.useState(false);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [typedConfirmation, setTypedConfirmation] = React.useState("");

  // The plan moved between preview and confirm: drop the plan in hand and the
  // phrase that confirmed it, so nothing here can be re-sent against it.
  const dropStalePreview = React.useCallback(() => {
    setPreview(null);
    setTypedConfirmation("");
  }, []);
  const rootRefusalMessage = React.useCallback(
    (error: unknown) => {
      const code = rootRefusalCode(error);
      return code ? t(rootRefusalMessageKey(code)) : null;
    },
    [t],
  );
  const {
    starting,
    startError,
    planChanged,
    startedOperationId,
    start,
    reset: resetStart,
    resetAll: resetStartAll,
  } = useLocationOperationStart({
    failedMessage: t("rootChange.startFailed"),
    recognizeOwnRefusal: rootRefusalMessage,
    onNeedsFreshPreview: dropStalePreview,
    onStarted,
  });

  const resetPreview = React.useCallback(() => {
    setPreview(null);
    setPreviewError(null);
    setTypedConfirmation("");
    resetStart();
  }, [resetStart]);

  // Reopening on a different root must never inherit the previous plan.
  React.useEffect(() => {
    if (!open) {
      return;
    }
    setDestinationKind("NEW_PATH");
    setDestinationPath("");
    setDestinationRootId("");
    setPreview(null);
    setPreviewError(null);
    setTypedConfirmation("");
    resetStartAll();
  }, [open, resetStartAll, root.id]);

  const plan: LocationOperationPreview | null = preview?.plan ?? null;
  const accounting = preview?.accounting ?? null;
  const content: LocationRootContentInventory | null = preview?.content ?? null;
  const retirement = preview?.retirement ?? null;

  // Which branch the server planned, read off the payload rather than off the
  // radio: a typed path that is already a root of this library comes back as a
  // fold even though the user expressed it as a new path.
  const changePreview = preview?.retention ? preview : null;
  const consolidationPreview = preview?.classification ? preview : null;

  const destinationNamed =
    destinationKind === "NEW_PATH"
      ? destinationPath.trim().length > 0
      : destinationRootId.length > 0;

  // Exactly one of the two destination forms, for the preview and for the
  // confirmation that has to say back what was previewed.
  const destinationInput = React.useMemo(
    () =>
      destinationKind === "NEW_PATH"
        ? { destinationPath: destinationPath.trim() }
        : { destinationRootId },
    [destinationKind, destinationPath, destinationRootId],
  );

  const handlePreview = React.useCallback(async () => {
    if (!destinationNamed) {
      return;
    }
    setPreviewing(true);
    setPreviewError(null);
    resetStart();
    try {
      const { data, error } = await client
        .query(
          locationRootScopePreviewQuery,
          { input: { libraryId, rootId: root.id, ...destinationInput } },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      const next = data?.locationRootScopePreview as
        | LocationRootScopePreview
        | undefined;
      if (!next) {
        throw new Error(t("rootChange.previewFailed"));
      }
      setPreview(next);
    } catch (error: unknown) {
      const code = rootRefusalCode(error);
      setPreview(null);
      setPreviewError(
        code
          ? t(rootRefusalMessageKey(code))
          : userFacingGraphQlErrorMessage(error, t("rootChange.previewFailed")),
      );
    } finally {
      setPreviewing(false);
    }
  }, [
    client,
    destinationInput,
    destinationNamed,
    libraryId,
    resetStart,
    root.id,
    t,
  ]);

  const canStart =
    rootPlanCanStart(plan, accounting) &&
    accountingCloses(accounting) &&
    !previewing &&
    !starting &&
    typedConfirmationSatisfied(plan?.confirmation, typedConfirmation);

  const handleStart = React.useCallback(async () => {
    if (!plan) {
      return;
    }
    await start({
      rootScope: { libraryId, rootId: root.id, ...destinationInput },
      planFingerprint: plan.planFingerprint,
      typedConfirmation:
        plan.confirmation.requirement === "TYPED" ? typedConfirmation : null,
    });
  }, [destinationInput, libraryId, plan, root.id, start, typedConfirmation]);

  const identity = rootIdentityStatement(changePreview?.retention);
  const groups = consolidationGroups(consolidationPreview?.classification);
  const renamedFolders = changedFolderNames(consolidationPreview?.plan);
  const destinationRoot = otherRoots.find(
    (candidate) => candidate.id === destinationRootId,
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent id="root-change-dialog" className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{t("rootChange.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("rootChange.dialogDescription", { path: root.path })}
          </DialogDescription>
        </DialogHeader>

        {startedOperationId ? (
          <LocationOperationStartedPanel
            idPrefix="root-change"
            operationId={startedOperationId}
            heading={t("rootChange.startedHeading")}
            viewLabel={t("rootChange.viewInActivity")}
            onNavigated={() => onOpenChange(false)}
          />
        ) : (
          <div className="max-h-[65vh] space-y-4 overflow-y-auto pr-1">
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground">
                {t("rootChange.destinationHeading")}
              </p>
              <RadioGroup
                value={destinationKind}
                onValueChange={(value) => {
                  setDestinationKind(value as RootDestinationKind);
                  resetPreview();
                }}
                className="space-y-2"
                disabled={starting}
              >
                <label
                  className="flex items-start gap-2 text-sm"
                  htmlFor="root-change-destination-new-path"
                >
                  <RadioGroupItem
                    id="root-change-destination-new-path"
                    value="NEW_PATH"
                    className="mt-0.5"
                  />
                  <span>
                    <span className="block text-foreground">
                      {t("rootChange.destinationNewPath")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t("rootChange.destinationNewPathHelp")}
                    </span>
                  </span>
                </label>
                <label
                  className="flex items-start gap-2 text-sm"
                  htmlFor="root-change-destination-existing-root"
                >
                  <RadioGroupItem
                    id="root-change-destination-existing-root"
                    value="EXISTING_ROOT"
                    className="mt-0.5"
                    disabled={otherRoots.length === 0}
                  />
                  <span>
                    <span className="block text-foreground">
                      {t("rootChange.destinationExistingRoot")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {otherRoots.length === 0
                        ? t("rootChange.destinationNoOtherRoots")
                        : t("rootChange.destinationExistingRootHelp")}
                    </span>
                  </span>
                </label>
              </RadioGroup>

              {destinationKind === "NEW_PATH" ? (
                <Input
                  id="root-change-destination-path"
                  value={destinationPath}
                  onChange={(event) => {
                    setDestinationPath(event.target.value);
                    resetPreview();
                  }}
                  placeholder={t("rootChange.destinationPathPlaceholder")}
                  disabled={starting}
                />
              ) : (
                <Select
                  value={destinationRootId}
                  onValueChange={(value) => {
                    setDestinationRootId(value);
                    resetPreview();
                  }}
                  disabled={starting || otherRoots.length === 0}
                >
                  <SelectTrigger
                    id="root-change-destination-root"
                    className="h-9 w-full"
                  >
                    <SelectValue
                      placeholder={t("rootChange.destinationExistingRoot")}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {otherRoots.map((candidate) => (
                      <SelectItem key={candidate.id} value={candidate.id}>
                        {candidate.path}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

            </div>

            {planChanged ? (
              <p
                id="root-change-plan-changed"
                className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-xs text-[var(--scry-warning-text)]"
              >
                {t("rootChange.planChanged")}
              </p>
            ) : null}

            <LocationOperationErrorNotice
              id="root-change-preview-error"
              message={previewError}
            />

            {previewing ? (
              <p
                id="root-change-previewing"
                className="flex items-center gap-2 text-sm text-muted-foreground"
              >
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("rootChange.previewing")}
              </p>
            ) : null}

            {/* US4's content order: what the root keeps comes first. */}
            {changePreview && identity ? (
              <div
                id="root-change-identity"
                className="space-y-1 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm"
              >
                <p className="flex items-start gap-2 text-foreground">
                  <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>
                    {t("rootChange.identityKeepsRoot", {
                      titles: identity.titleAssignments,
                    })}
                  </span>
                </p>
                {identity.keepsDefault ? (
                  <p
                    id="root-change-identity-default"
                    className="text-xs text-muted-foreground"
                  >
                    {t("rootChange.identityKeepsDefault")}
                  </p>
                ) : null}
                {identity.losesDefault ? (
                  <p
                    id="root-change-identity-loses-default"
                    className="text-xs text-[var(--scry-warning-text)]"
                  >
                    {t("rootChange.identityLosesDefault")}
                  </p>
                ) : null}
                <p className="flex flex-wrap items-center gap-1 font-[var(--font-code)] text-xs break-all text-muted-foreground">
                  <span>{root.path}</span>
                  <ArrowRight className="h-3 w-3 shrink-0" />
                  <span>{destinationPath.trim()}</span>
                </p>
              </div>
            ) : null}

            {/* US5 opens with the two roots, then with where new content lands. */}
            {consolidationPreview ? (
              <div
                id="root-consolidation-statement"
                className="space-y-1 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm"
              >
                <p className="flex items-start gap-2 text-foreground">
                  <HardDrive className="mt-0.5 h-4 w-4 shrink-0" />
                  <span>
                    {t("rootChange.consolidationStatement", {
                      titles: toCount(
                        consolidationPreview.accounting.assignedTotal,
                      ),
                    })}
                  </span>
                </p>
                <p className="flex flex-wrap items-center gap-1 font-[var(--font-code)] text-xs break-all text-muted-foreground">
                  <span>{root.path}</span>
                  <ArrowRight className="h-3 w-3 shrink-0" />
                  <span>{destinationRoot?.path ?? destinationRootId}</span>
                </p>
                <p
                  id="root-consolidation-default-transfer"
                  className={cn(
                    "text-xs",
                    consolidationPreview.defaultTransfer?.transfersTheDefault
                      ? "text-[var(--scry-warning-text)]"
                      : "text-muted-foreground",
                  )}
                >
                  {consolidationPreview.defaultTransfer?.transfersTheDefault
                    ? t("rootChange.defaultTransfers")
                    : t("rootChange.defaultStays")}
                </p>
                <p
                  id="root-consolidation-retires-configuration"
                  className="text-xs text-muted-foreground"
                >
                  {t("rootChange.consolidationRetiresConfiguration")}
                </p>
              </div>
            ) : null}

            {accounting ? (
              <TitleLedger accounting={accounting} t={t} />
            ) : null}

            {/* FR-024's seven groups are the consolidation preview (US5.1). */}
            {consolidationPreview ? (
              <div
                id="root-consolidation-groups"
                className="space-y-2 rounded-lg border border-border bg-muted/20 px-3 py-3"
              >
                <p className="text-sm font-medium text-foreground">
                  {t("rootChange.groupsHeading")}
                </p>
                <dl className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
                  {groups.map((group) => (
                    <div
                      key={group.key}
                      id={`root-consolidation-group-${group.key}`}
                    >
                      <dt className="text-xs text-muted-foreground">
                        {t(`rootChange.group.${group.key}`)}
                      </dt>
                      <dd className="text-foreground">{group.count}</dd>
                    </div>
                  ))}
                </dl>
              </div>
            ) : null}

            {/* US5.4: every changed folder name, by name. */}
            {consolidationPreview && renamedFolders.length > 0 ? (
              <div
                id="root-consolidation-renamed-folders"
                className="space-y-1 rounded-lg border border-border bg-muted/20 px-3 py-3"
              >
                <p className="text-sm font-medium text-foreground">
                  {t("rootChange.renamedFoldersHeading")}
                </p>
                <ul className="space-y-1 text-xs">
                  {renamedFolders.map((line, index) => (
                    <li
                      key={`${line.titleId ?? "folder"}-${index}`}
                      id={`root-consolidation-renamed-folder-${index}`}
                      className="min-w-0"
                    >
                      <span className="font-[var(--font-code)] break-all text-foreground">
                        {line.from}
                      </span>
                      <span className="mx-1 text-muted-foreground">
                        {"→"}
                      </span>
                      <span className="font-[var(--font-code)] break-all text-foreground">
                        {line.to}
                      </span>
                      {line.detail ? (
                        <span className="block text-muted-foreground">
                          {line.detail}
                        </span>
                      ) : null}
                    </li>
                  ))}
                </ul>
                {changedFolderNamesComplete(consolidationPreview.plan) ? null : (
                  <p
                    id="root-consolidation-renamed-folders-sampled"
                    className="text-xs text-muted-foreground"
                  >
                    {t("rootChange.renamedFoldersSampled")}
                  </p>
                )}
              </div>
            ) : null}

            {content ? <ContentBuckets content={content} t={t} /> : null}

            {retirement ? (
              <RetirementContract retirement={retirement} t={t} />
            ) : null}

            {plan && plan.warnings.length > 0 ? (
              <ul
                id="root-change-warnings"
                className="space-y-1 rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-3 text-sm text-[var(--scry-warning-text)]"
              >
                {plan.warnings.map((warning) => (
                  <li key={warning} className="flex items-start gap-2">
                    <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                    <span>{warning}</span>
                  </li>
                ))}
              </ul>
            ) : null}

            {plan ? (
              <p
                id="root-change-verification"
                className="text-xs text-muted-foreground"
              >
                {t("rootChange.verificationStatement", {
                  files: toCount(plan.verification.files),
                  bytes: formatByteCount(toCount(plan.verification.bytes)),
                })}
              </p>
            ) : null}

            {plan && plan.freeSpace?.sufficient === false ? (
              <p
                id="root-change-insufficient-space"
                className="rounded-lg border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-3 text-sm text-[var(--scry-danger-text)]"
              >
                {t("rootChange.insufficientSpace", {
                  required: formatByteCount(
                    toCount(plan.freeSpace.destinationTotalRequiredBytes),
                  ),
                })}
              </p>
            ) : null}

            {plan && plan.confirmation.requirement === "TYPED" ? (
              <div className="space-y-1">
                <label
                  className="block text-xs font-medium text-muted-foreground"
                  htmlFor="root-change-typed-confirmation"
                >
                  {plan.confirmation.typedPrompt ??
                    t("rootChange.typedConfirmationPrompt")}
                </label>
                <Input
                  id="root-change-typed-confirmation"
                  value={typedConfirmation}
                  onChange={(event) => setTypedConfirmation(event.target.value)}
                  placeholder={plan.confirmation.typedPhrase ?? ""}
                  disabled={starting}
                />
              </div>
            ) : null}

            <LocationOperationErrorNotice
              id="root-change-start-error"
              message={startError}
            />
          </div>
        )}

        <DialogFooter>
          <LocationDialogDismissButton
            id="root-change-dismiss"
            label={startedOperationId ? t("label.close") : t("label.cancel")}
            disabled={starting}
            onDismiss={() => onOpenChange(false)}
          />
          {startedOperationId ? null : plan ? (
            <LocationDialogPrimaryButton
              id="root-change-confirm"
              label={t("rootChange.confirm")}
              busy={starting}
              disabled={!canStart}
              onClick={() => void handleStart()}
            />
          ) : (
            <LocationDialogPrimaryButton
              id="root-change-preview"
              label={t("rootChange.preview")}
              busy={previewing}
              disabled={!destinationNamed || previewing}
              onClick={() => void handlePreview()}
            />
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/**
 * FR-023's every-title ledger. There is deliberately no deselect control here:
 * a root-scoped operation takes every title assigned to the root, and a blocked
 * title is repaired rather than skipped.
 */
function TitleLedger({
  accounting,
  t,
}: {
  accounting: LocationTitleAccounting;
  t: Translate;
}) {
  const closes = accountingCloses(accounting);
  return (
    <div
      id="root-change-accounting"
      className={cn(
        "space-y-3 rounded-lg border px-3 py-3",
        accounting.blocksStart
          ? "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]"
          : "border-border bg-muted/20",
      )}
    >
      <p className="text-sm font-medium text-foreground">
        {t("rootChange.accountingHeading")}
      </p>
      <dl className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
        <div id="root-change-accounting-assigned">
          <dt className="text-xs text-muted-foreground">
            {t("rootChange.accountingAssigned")}
          </dt>
          <dd className="text-foreground">
            {toCount(accounting.assignedTotal)}
          </dd>
        </div>
        <div id="root-change-accounting-relocating">
          <dt className="text-xs text-muted-foreground">
            {t("rootChange.accountingRelocating")}
          </dt>
          <dd className="text-foreground">{toCount(accounting.relocating)}</dd>
        </div>
        <div id="root-change-accounting-catalog-only">
          <dt className="text-xs text-muted-foreground">
            {t("rootChange.accountingCatalogOnly")}
          </dt>
          <dd className="text-foreground">{toCount(accounting.catalogOnly)}</dd>
        </div>
        <div id="root-change-accounting-blocked">
          <dt className="text-xs text-muted-foreground">
            {t("rootChange.accountingBlocked")}
          </dt>
          <dd
            className={cn(
              "text-foreground",
              toCount(accounting.blocked) > 0 && "text-[var(--scry-danger-text)]",
            )}
          >
            {toCount(accounting.blocked)}
          </dd>
        </div>
      </dl>

      {closes ? null : (
        <p
          id="root-change-accounting-open"
          className="text-xs text-[var(--scry-danger-text)]"
        >
          {t("rootChange.accountingDoesNotClose")}
        </p>
      )}

      {accounting.blockedTitles.length > 0 ? (
        <div
          id="root-change-blocked-titles"
          className="space-y-1 text-sm text-[var(--scry-danger-text)]"
        >
          <p className="flex items-start gap-2">
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>
              {t("rootChange.blockedTitlesHeading", {
                count: accounting.blockedTitles.length,
              })}
            </span>
          </p>
          <ul className="ml-4 list-disc space-y-0.5 text-xs">
            {accounting.blockedTitles.map((title) => {
              const key = rootReasonKey(title.reasonCode);
              return (
                <li
                  key={title.titleId}
                  id={`root-change-blocked-title-${title.titleId}`}
                >
                  <span className="text-foreground">{title.titleName}</span>
                  <span className="block opacity-90">
                    {key ? t(key) : title.reason}
                  </span>
                </li>
              );
            })}
          </ul>
          <p className="text-xs">{t("rootChange.blockedTitlesNoExclude")}</p>
        </div>
      ) : null}
    </div>
  );
}

/**
 * FR-027's three buckets. The unknown one is listed separately and by name,
 * because it is what keeps the old location standing.
 */
function ContentBuckets({
  content,
  t,
}: {
  content: LocationRootContentInventory;
  t: Translate;
}) {
  return (
    <div
      id="root-change-content"
      className="space-y-3 rounded-lg border border-border bg-muted/20 px-3 py-3"
    >
      <p className="text-sm font-medium text-foreground">
        {t("rootChange.contentHeading")}
      </p>
      <dl className="grid grid-cols-3 gap-2 text-sm">
        <ContentCount
          id="root-change-content-managed"
          label={t("rootChange.contentManaged")}
          bucket={content.managed}
        />
        <ContentCount
          id="root-change-content-companions"
          label={t("rootChange.contentCompanions")}
          bucket={content.companions}
        />
        <ContentCount
          id="root-change-content-unknown"
          label={t("rootChange.contentUnknown")}
          bucket={content.unknown}
          danger={toCount(content.unknown.total) > 0}
        />
      </dl>

      {toCount(content.unknown.total) > 0 ? (
        <div
          id="root-change-unknown-content"
          className="space-y-1 text-xs text-[var(--scry-warning-text)]"
        >
          <p>
            {t("rootChange.unknownContentHelp", {
              bytes: formatByteCount(toCount(content.unknownBytes)),
            })}
          </p>
          <ul className="ml-4 list-disc space-y-0.5">
            {content.unknown.entries.map((entry) => (
              <li key={entry.path} className="font-[var(--font-code)] break-all">
                {entry.path}
              </li>
            ))}
          </ul>
          {content.unknown.complete ? null : (
            <p id="root-change-unknown-content-sampled">
              {t("rootChange.contentSampled")}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}

function ContentCount({
  id,
  label,
  bucket,
  danger = false,
}: {
  id: string;
  label: string;
  bucket: LocationRootContentBucket;
  danger?: boolean;
}) {
  return (
    <div id={id} className="min-w-0">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          "text-foreground",
          danger && "text-[var(--scry-warning-text)]",
        )}
      >
        {toCount(bucket.total)}
        {toCount(bucket.bytesTotal) > 0 ? (
          <span className="ml-1 text-xs text-muted-foreground">
            {formatByteCount(toCount(bucket.bytesTotal))}
          </span>
        ) : null}
      </dd>
    </div>
  );
}

/**
 * FR-028: only empty directories are ever removed automatically, and anything
 * that stops the old location from being retired is named.
 */
function RetirementContract({
  retirement,
  t,
}: {
  retirement: LocationRootRetirementContract;
  t: Translate;
}) {
  return (
    <div
      id="root-change-retirement"
      className="space-y-2 rounded-lg border border-border bg-muted/20 px-3 py-3 text-sm"
    >
      <p className="font-medium text-foreground">
        {t("rootChange.retirementHeading")}
      </p>
      <p className="text-xs text-muted-foreground">
        {t("rootChange.retirementEmptyDirectoriesOnly", {
          count: toCount(retirement.removableDirectories.total),
        })}
      </p>
      {retirement.requiresVerificationBeforeSourceRemoval ? (
        <p
          id="root-change-retirement-verification"
          className="text-xs text-muted-foreground"
        >
          {t("rootChange.retirementVerificationFirst")}
        </p>
      ) : null}
      <SampledPathList
        id="root-change-retirement-retained"
        heading={t("rootChange.retirementRetainedHeading")}
        paths={retirement.retainedDirectories}
        t={t}
      />
      {retirement.blockers.length > 0 ? (
        <div
          id="root-change-retirement-blockers"
          className="space-y-1 text-xs text-[var(--scry-warning-text)]"
        >
          <p>{t("rootChange.retirementBlockedHeading")}</p>
          <ul className="ml-4 list-disc space-y-0.5">
            {retirement.blockers.map((blocker) => {
              const key = retirementBlockerKey(blocker.code);
              return (
                <li
                  key={blocker.code}
                  id={`root-change-retirement-blocker-${blocker.code}`}
                >
                  {key ? t(key) : blocker.detail}
                </li>
              );
            })}
          </ul>
        </div>
      ) : (
        <p
          id="root-change-retirement-permitted"
          className="text-xs text-muted-foreground"
        >
          {t("rootChange.retirementPermitted")}
        </p>
      )}
    </div>
  );
}

function SampledPathList({
  id,
  heading,
  paths,
  t,
}: {
  id: string;
  heading: string;
  paths: LocationSampledPaths;
  t: Translate;
}) {
  if (toCount(paths.total) === 0) {
    return null;
  }
  return (
    <div id={id} className="text-xs text-muted-foreground">
      <p>{heading}</p>
      <ul className="ml-4 list-disc space-y-0.5">
        {paths.paths.map((path) => (
          <li key={path} className="font-[var(--font-code)] break-all">
            {path}
          </li>
        ))}
      </ul>
      {paths.complete ? null : <p>{t("rootChange.contentSampled")}</p>}
    </div>
  );
}

