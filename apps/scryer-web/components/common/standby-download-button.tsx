import { useRef, useState } from "react";
import { useClient } from "urql";
import { Download } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { queueStandbyReleaseMutation } from "@/lib/graphql/mutations";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type { PendingReleaseItem } from "@/lib/types";

type Conflict = {
  downloadClientId: string | null;
  downloadClientType: string;
  downloadClientItemId: string;
  sourceTitle: string | null;
  titleName: string;
  state: string | null;
};

export function StandbyDownloadButton({
  release,
  onQueued,
}: {
  release: PendingReleaseItem;
  onQueued: () => Promise<void>;
}) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const inFlight = useRef(false);
  const [busy, setBusy] = useState(false);
  const [conflicts, setConflicts] = useState<Conflict[]>([]);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  async function queue(choice: "CHECK" | "QUEUE" | "CANCEL_AND_QUEUE") {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setErrorMessage(null);
    try {
      const { data, error } = await client
        .mutation(queueStandbyReleaseMutation, {
          id: release.id,
          choice,
          confirmed:
            choice === "CHECK"
              ? []
              : conflicts.map((conflict) => ({
                  clientId: conflict.downloadClientId,
                  clientType: conflict.downloadClientType,
                  itemId: conflict.downloadClientItemId,
                })),
        })
        .toPromise();
      if (error) throw error;
      const result = data?.queueStandbyRelease;
      if (result?.grabbed) {
        setConflicts([]);
        // Cancellation is best effort; the release is queued either way.
        const notCancelled = result.cancelFailed?.length ?? 0;
        if (notCancelled > 0) {
          setGlobalStatus(
            t("wanted.standbyQueuedCancelFailed", { count: notCancelled }),
            { level: "WARNING" },
          );
        } else {
          setGlobalStatus(t("pending.grabbed"));
        }
        try {
          await onQueued();
        } catch {
          setGlobalStatus(t("wanted.standbyQueuedRefreshFailed"), {
            level: "ERROR",
          });
        }
      } else if (result?.conflicts?.length) {
        setConflicts(result.conflicts);
      } else {
        setErrorMessage(t("error.reason.STANDBY_NOT_QUEUEABLE"));
      }
    } catch (error) {
      const message = userFacingGraphQlErrorMessage(
        error,
        t("status.apiError"),
        t,
      );
      setErrorMessage(message);
      setGlobalStatus(message, { level: "ERROR" });
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  return (
    <>
      <Button
        size="sm"
        variant="secondary"
        disabled={busy}
        onClick={() => void queue("CHECK")}
        aria-label={t("wanted.downloadStandbyLabel", {
          name: release.releaseTitle,
        })}
      >
        <Download className="h-4 w-4" />
        {t("wanted.downloadStandby")}
      </Button>
      {errorMessage && conflicts.length === 0 ? (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage}
        </p>
      ) : null}
      <Dialog
        open={conflicts.length > 0}
        onOpenChange={(open) => {
          if (!open && !busy) {
            setConflicts([]);
            setErrorMessage(null);
          }
        }}
      >
        <DialogContent>
          <DialogTitle>{t("wanted.standbyConflictTitle")}</DialogTitle>
          <DialogDescription>
            {t("wanted.standbyConflictDescription", {
              name: release.releaseTitle,
            })}
          </DialogDescription>
          <ul className="space-y-2 text-sm break-words">
            {conflicts.map((item) => (
              <li
                key={`${item.downloadClientId}:${item.downloadClientType}:${item.downloadClientItemId}`}
              >
                {item.sourceTitle ?? item.titleName}
                <span className="block text-xs text-muted-foreground">
                  {item.downloadClientType} · {item.downloadClientItemId}
                </span>
              </li>
            ))}
          </ul>
          {errorMessage ? (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage}
            </p>
          ) : null}
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setConflicts([]);
                setErrorMessage(null);
              }}
            >
              {t("label.cancel")}
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void queue("QUEUE")}
            >
              {t("wanted.standbyQueue")}
            </Button>
            <Button
              disabled={busy}
              onClick={() => void queue("CANCEL_AND_QUEUE")}
            >
              {t("wanted.standbyCancelAndQueue")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
