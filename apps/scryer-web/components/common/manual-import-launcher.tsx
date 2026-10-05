import * as React from "react";
import { FileInput } from "lucide-react";
import { useClient } from "urql";

import { LoadingMark } from "@/components/common/loading-mark";
import { ManualImportDialog } from "@/components/dialogs/manual-import-dialog";
import { Button } from "@/components/ui/button";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useDownloadPasswordRetry } from "@/lib/hooks/use-download-password-retry";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  beginManualImportSelectionMutation,
  queueManualImportMutation,
} from "@/lib/graphql/mutations";
import type { DownloadQueueItem } from "@/lib/types/download-queue";
import {
  type DirectMovieManualImportCandidate,
  allowsManualImport,
  directMovieManualImportMappings,
  manualImportNeedsMapping,
  manualImportSelectionNeedsDialog,
} from "@/lib/utils/manual-import-actions";

type ManualImportSelection = {
  selectionId?: string | null;
  archiveExtractionNeeded?: boolean | null;
  files?: Array<DirectMovieManualImportCandidate & { fileName: string }> | null;
};

type DialogTarget = {
  item: DownloadQueueItem;
  titleId: string;
  titleName: string;
  facet: string | null;
};

export type ManualImportLauncher = {
  /** Import a finished download. Failures are reported, never thrown. */
  launch: (item: DownloadQueueItem) => Promise<void>;
  /** The download a direct movie import is working on. */
  busyItemId: string | null;
  /** Render once; it is the file-mapping dialog when one is needed. */
  dialog: React.ReactNode;
};

/**
 * The one way the app imports a finished download by hand. A movie imports
 * its largest file straight away; series, anime and disc images open the
 * dialog so the files can be matched first. The dashboard, the activity page
 * and both title overviews all use this.
 */
export function useManualImportLauncher({
  title,
  onImportQueued,
  onImportRetried,
}: {
  /** The title being viewed, when the download belongs to it. */
  title?: { id: string; name: string; facet: string } | null;
  /** Refreshes the page once an import is queued. */
  onImportQueued: (item: DownloadQueueItem) => Promise<void> | void;
  /** Refreshes a finished local retry without marking it pending. */
  onImportRetried?: (item: DownloadQueueItem) => Promise<void> | void;
}): ManualImportLauncher {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { request: requestPasswordRetry, dialog: passwordRetryDialog } = useDownloadPasswordRetry();
  const [busyItemId, setBusyItemId] = React.useState<string | null>(null);
  const [dialogTarget, setDialogTarget] = React.useState<DialogTarget | null>(null);
  const onImportQueuedRef = React.useRef(onImportQueued);
  const onImportRetriedRef = React.useRef(onImportRetried ?? onImportQueued);
  React.useEffect(() => {
    onImportQueuedRef.current = onImportQueued;
    onImportRetriedRef.current = onImportRetried ?? onImportQueued;
  });
  const notifyImportRetried = React.useCallback((item: DownloadQueueItem) => {
    void Promise.resolve().then(() => onImportRetriedRef.current(item)).catch((error: unknown) => {
      console.error("[manual-import] refresh after retry failed:", error);
    });
  }, []);
  const notifyImportQueued = React.useCallback((item: DownloadQueueItem) => {
    // The import is queued either way; a failed reload only leaves the page
    // stale until its next refresh.
    void Promise.resolve()
      .then(() => onImportQueuedRef.current(item))
      .catch((error: unknown) => {
        console.error("[manual-import] refresh after import failed:", error);
      });
  }, []);

  const beginSelection = React.useCallback(
    async (
      item: DownloadQueueItem,
      titleId: string,
      extractArchives: boolean,
    ): Promise<ManualImportSelection | null> => {
      const { data, error } = await client
        .mutation<{ beginManualImportSelection?: ManualImportSelection | null }>(
          beginManualImportSelectionMutation,
          {
            input: {
              clientId: item.clientId,
              clientType: item.clientType,
              downloadClientItemId: item.downloadClientItemId,
              titleId,
              ...(extractArchives ? { extractArchives: true } : {}),
            },
          },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      return data?.beginManualImportSelection ?? null;
    },
    [client],
  );

  const launch = React.useCallback(
    async (item: DownloadQueueItem) => {
      if (item.passwordRetryImportId) {
        const outcome = await requestPasswordRetry(item);
        if (outcome !== "cancelled") notifyImportRetried(item);
        return;
      }
      const titleId = title?.id ?? item.titleId;
      if (!titleId) {
        setGlobalStatus(t("queue.assignTitleBeforeImport"));
        return;
      }
      const target: DialogTarget = {
        item,
        titleId,
        titleName: title?.name ?? item.titleName,
        facet: title?.facet ?? item.facet,
      };
      if (manualImportNeedsMapping(target.facet)) {
        setDialogTarget(target);
        return;
      }

      // A movie lands one file, so only its largest candidate is mapped; the
      // server still picks the primary among whatever is mapped.
      setBusyItemId(item.id);
      try {
        let selection = await beginSelection(item, titleId, false);
        if (selection?.archiveExtractionNeeded) {
          selection = await beginSelection(item, titleId, true);
        }
        const candidates = selection?.files ?? [];
        if (manualImportSelectionNeedsDialog(candidates)) {
          setDialogTarget(target);
          return;
        }
        const files = directMovieManualImportMappings(candidates);
        if (!selection?.selectionId || files.length === 0) {
          setGlobalStatus(t("queue.manualImportFailed"), { level: "ERROR" });
          return;
        }
        const { error } = await client
          .mutation(queueManualImportMutation, {
            input: { selectionId: selection.selectionId, files },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("queue.manualImportQueued"));
        notifyImportQueued(item);
      } catch (error: unknown) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("queue.manualImportFailed")),
          { level: "ERROR" },
        );
      } finally {
        setBusyItemId((current) => (current === item.id ? null : current));
      }
    },
    [
      beginSelection,
      client,
      notifyImportQueued,
      notifyImportRetried,
      requestPasswordRetry,
      setGlobalStatus,
      t,
      title?.facet,
      title?.id,
      title?.name,
    ],
  );

  const dialog = dialogTarget ? (
    <ManualImportDialog
      open
      onOpenChange={(open) => {
        if (!open) {
          setDialogTarget(null);
        }
      }}
      titleId={dialogTarget.titleId}
      facet={dialogTarget.facet}
      titleName={dialogTarget.titleName}
      clientId={dialogTarget.item.clientId}
      clientType={dialogTarget.item.clientType}
      downloadClientItemId={dialogTarget.item.downloadClientItemId}
      onImportQueued={() => notifyImportQueued(dialogTarget.item)}
    />
  ) : null;

  return { launch, busyItemId, dialog: <>{dialog}{passwordRetryDialog}</> };
}

/**
 * A title overview's Manual Import button. It imports the most recent finished
 * download the server says can still be imported by hand, and appears only
 * when there is one. Taking the newest finished download blindly offered the
 * button for downloads that were already imported or still mid-import, and the
 * click then failed or re-ran an import nobody asked for.
 */
export function TitleManualImportButton({
  launcher,
  completedDownloads,
  canManageTitle,
  className,
}: {
  launcher: ManualImportLauncher;
  completedDownloads: readonly DownloadQueueItem[];
  canManageTitle: boolean;
  className?: string;
}) {
  const t = useTranslate();
  const item = completedDownloads.find((candidate) => allowsManualImport(candidate));
  if (!canManageTitle || !item) {
    return null;
  }
  const busy = launcher.busyItemId === item.id;
  return (
    <Button
      id="title-overview-manual-import"
      type="button"
      variant="outline"
      size="sm"
      className={className}
      disabled={busy}
      onClick={() => {
        void launcher.launch(item);
      }}
    >
      {busy ? (
        <LoadingMark className="mr-1.5 h-4 w-4" />
      ) : (
        <FileInput className="mr-1.5 h-4 w-4" />
      )}
      {t("queue.manualImport")}
    </Button>
  );
}
