import { useCallback, useState } from "react";
import { useMutation } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { releaseHeldImportSourcesMutation } from "@/lib/graphql/mutations";
import type {
  DownloadQueueItem,
  HeldDownloadClientPolicy,
  HeldImportSourcesReason,
  HeldImportSourcesReleased,
  HeldImportSourcesSettlement,
  HeldWorkspacePreservedReason,
} from "@/lib/types";

type Translate = ReturnType<typeof useTranslate>;

const REASON_KEYS: Record<HeldImportSourcesReason, string> = {
  SUBTITLES_PENDING: "queue.releaseHeldSourcesReasonSubtitlesPending",
  SOURCE_CLEANUP_INCOMPLETE: "queue.releaseHeldSourcesReasonSourceCleanupIncomplete",
  UNKNOWN: "queue.releaseHeldSourcesReasonUnknown",
  ARCHIVE_EXTRACTION_FAILED: "queue.releaseHeldSourcesReasonArchiveExtractionFailed",
};

const POLICY_KEYS: Record<HeldDownloadClientPolicy, string> = {
  REMOVES: "queue.releaseHeldSourcesClientRemoves",
  REMOVES_AFTER_SEEDING: "queue.releaseHeldSourcesClientRemovesAfterSeeding",
  KEEPS: "queue.releaseHeldSourcesClientKeeps",
  UNKNOWN: "queue.releaseHeldSourcesClientPolicyUnknown",
};

const PRESERVED_KEYS: Record<HeldWorkspacePreservedReason, string> = {
  NOT_OWNED: "queue.releaseHeldSourcesPreservedNotOwned",
  UNSAFE: "queue.releaseHeldSourcesPreservedUnsafe",
  HOLDS_UNIMPORTED_VIDEO: "queue.releaseHeldSourcesPreservedHoldsUnimportedVideo",
  IN_USE: "queue.releaseHeldSourcesPreservedInUse",
  UNVERIFIED: "queue.releaseHeldSourcesPreservedUnverified",
  REMOVAL_FAILED: "queue.releaseHeldSourcesPreservedRemovalFailed",
  DOWNLOAD_NOT_IMPORTED: "queue.releaseHeldSourcesPreservedDownloadNotImported",
};

const SETTLEMENT_KEYS: Record<HeldImportSourcesSettlement, string> = {
  IMPORTED: "queue.releaseHeldSourcesSuccess",
  AWAITING_IMPORT: "queue.releaseHeldSourcesAwaitingImport",
  NOT_SETTLED: "queue.releaseHeldSourcesNotSettled",
  // The holds were released, but nothing settled the download, so nothing of
  // it was cleaned up. Never reported as success.
  UNCHANGED: "queue.releaseHeldSourcesNotSettledNothingCleaned",
  UNTRACKED: "queue.releaseHeldSourcesNotSettledNothingCleaned",
  // Nothing proves every file was imported, so the download was neither
  // marked imported nor cleaned up.
  UNPROVEN: "queue.releaseHeldSourcesUnproven",
};

/** Settlements the operator should look at: the download was not settled. */
const WARNING_SETTLEMENTS: ReadonlySet<HeldImportSourcesSettlement> = new Set([
  "NOT_SETTLED",
  "UNCHANGED",
  "UNTRACKED",
  "UNPROVEN",
]);

/** Holds whose release can remove content that was never imported. */
const SEVERE_REASONS: ReadonlySet<HeldImportSourcesReason> = new Set(["ARCHIVE_EXTRACTION_FAILED", "UNKNOWN"]);

export function canReleaseHeldSources(item: DownloadQueueItem): boolean {
  return Boolean(item.heldImportSources?.importId);
}

/** The toast for a release, telling the operator what was left in place and why. */
export function heldSourcesReleaseMessage(t: Translate, released: HeldImportSourcesReleased): string {
  const parts = [t(SETTLEMENT_KEYS[released.settlement])];
  const reasons = released.preservedWorkspaceReasons.map((reason) => t(PRESERVED_KEYS[reason]));
  if (released.workspaceLookupIncomplete) reasons.push(t("queue.releaseHeldSourcesPreservedLookupIncomplete"));
  if (!released.workspaceRemoved && reasons.length > 0) {
    parts.push(t("queue.releaseHeldSourcesWorkspacesPreserved", { reasons: reasons.join(", ") }));
  }
  return parts.join(" ");
}

/**
 * Asks before releasing the sources a download's completed imports are
 * holding, naming the download, every title the release covers, why the
 * sources are held and what the client's removal policy will then do.
 */
export function useReleaseHeldSources(onReleased: (item: DownloadQueueItem) => Promise<void> | void) {
  const t = useTranslate();
  const setStatus = useGlobalStatus();
  const [, execute] = useMutation(releaseHeldImportSourcesMutation);
  const [target, setTarget] = useState<DownloadQueueItem | null>(null);
  const [busy, setBusy] = useState(false);
  const request = useCallback((item: DownloadQueueItem) => {
    if (canReleaseHeldSources(item)) setTarget(item);
  }, []);
  const held = target?.heldImportSources ?? null;
  const client = target ? target.clientName || target.clientType : "";
  const severe = held ? SEVERE_REASONS.has(held.reason) : false;

  const dialog = (
    <ConfirmDialog
      open={target !== null && held !== null}
      title={t("queue.releaseHeldSourcesConfirmTitle")}
      description={t("queue.releaseHeldSourcesConfirmDescription", {
        download: target?.titleName || target?.downloadClientItemId || "",
        client,
      })}
      confirmLabel={t("queue.releaseHeldSources")}
      cancelLabel={t("label.cancel")}
      isBusy={busy}
      onCancel={() => setTarget(null)}
      onConfirm={async () => {
        if (!target || !held) return;
        setBusy(true);
        try {
          const result = await execute({ input: { importId: held.importId } });
          if (result.error) {
            setStatus(userFacingGraphQlErrorMessage(result.error, t("queue.releaseHeldSourcesFailed")), { level: "ERROR" });
            return;
          }
          const released = result.data?.releaseHeldImportSources as HeldImportSourcesReleased | undefined;
          if (released) {
            const preservedSomething = !released.workspaceRemoved && (released.preservedWorkspaceReasons.length > 0 || released.workspaceLookupIncomplete);
            const warn = WARNING_SETTLEMENTS.has(released.settlement) || preservedSomething;
            setStatus(heldSourcesReleaseMessage(t, released), { level: warn ? "WARNING" : "SUCCESS" });
          } else {
            setStatus(t("queue.releaseHeldSourcesSuccess"), { level: "SUCCESS" });
          }
          setTarget(null);
          await onReleased(target);
        } catch (error: unknown) {
          setStatus(userFacingGraphQlErrorMessage(error, t("queue.releaseHeldSourcesFailed")), { level: "ERROR" });
        } finally {
          setBusy(false);
        }
      }}
    >
      {held ? (
        <div className="space-y-2 text-xs">
          {held.titleNames.length > 0 ? (
            <p className="text-muted-foreground">
              {t("queue.releaseHeldSourcesTitles", { titles: held.titleNames.join(", ") })}
            </p>
          ) : null}
          <p className={severe ? "font-medium text-destructive" : "text-muted-foreground"}>{t(REASON_KEYS[held.reason])}</p>
          <p className="text-muted-foreground">{t(POLICY_KEYS[held.clientPolicy], { client })}</p>
        </div>
      ) : null}
    </ConfirmDialog>
  );

  return { request, dialog };
}
