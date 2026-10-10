import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation } from "urql";
import { PasswordRetryDialog } from "@/components/dialogs/password-retry-dialog";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { DownloadQueueItem } from "@/lib/types";
import { importRetrySucceeded } from "@/lib/utils/download-queue";

const retryMutation = `mutation RetryDownloadPassword($input: RetryDownloadPasswordInput!) {
  retryDownloadPassword(input: $input) { status downloadClientItemId }
}`;

export function canRetryDownloadPassword(item: DownloadQueueItem): boolean {
  if (item.passwordRetryImportId) return true;
  return item.state === "FAILED" && Boolean(item.downloadId) &&
    ["sabnzbd", "nzbget", "weaver"].includes(item.clientType) &&
    (item.passwordFailureCode === "ARCHIVE_PASSWORD_REQUIRED" ||
     item.passwordFailureCode === "ARCHIVE_PASSWORD_OR_CORRUPTION" ||
     Boolean(item.attentionReason?.startsWith("ARCHIVE_PASSWORD_REQUIRED:") ||
       item.attentionReason?.startsWith("ARCHIVE_PASSWORD_OR_CORRUPTION:")));
}

export type DownloadPasswordRetryTarget = Pick<DownloadQueueItem,
  "downloadId" | "clientId" | "clientType" | "downloadClientItemId" | "titleName" | "passwordRetryImportId">;
type PasswordRetryOutcome = "accepted" | "failed" | "refused" | "uncertain" | "cancelled";
type Pending = { item: DownloadPasswordRetryTarget; resolve: (outcome: PasswordRetryOutcome) => void };

export function useDownloadPasswordRetry() {
  const t = useTranslate();
  const setStatus = useGlobalStatus();
  const [, execute] = useMutation<{ retryDownloadPassword: { status: "ACCEPTED" | "REFUSED" | "AWAITING_RECONCILIATION" } }>(retryMutation);
  const [, retryImport] = useMutation(`mutation RetryImportPassword($input: RetryImportInput!) {
    retryImport(input: $input) { importId decision skipReason }
  }`);
  const queue = useRef<Pending[]>([]);
  const [pending, setPending] = useState<Pending | null>(null);
  const request = useCallback((item: DownloadPasswordRetryTarget) => new Promise<PasswordRetryOutcome>((resolve) => {
    const next = { item, resolve };
    queue.current.push(next);
    if (queue.current.length === 1) setPending(next);
  }), []);
  const finish = useCallback((outcome: PasswordRetryOutcome) => {
    const completed = queue.current.shift();
    setPending(queue.current[0] ?? null);
    completed?.resolve(outcome);
  }, []);
  useEffect(() => () => {
    for (const entry of queue.current.splice(0)) entry.resolve("cancelled");
  }, []);
  return {
    request,
    dialog: <PasswordRetryDialog open={pending !== null} jobLabel={pending?.item.titleName ?? ""}
      confirmLabel={t("importHistory.retryWithPassword")}
      onCancel={() => finish("cancelled")} onConfirm={async (password) => {
        if (!pending) return;
        const item = pending.item;
        let outcome: PasswordRetryOutcome = "uncertain";
        try {
          if (item.passwordRetryImportId) {
            const result = await retryImport({ input: { importId: item.passwordRetryImportId, password } });
            const imported = result.data?.retryImport;
            outcome = !result.error && importRetrySucceeded(imported) ? "accepted" : "failed";
            setStatus(t(outcome === "accepted" ? "importHistory.retrySuccess" : "queue.passwordRetryFailed"));
            return;
          }
          const result = await execute({ input: {
            downloadId: item.downloadId, clientId: item.clientId, clientType: item.clientType,
            downloadClientItemId: item.downloadClientItemId, password,
          } });
          const status = result.data?.retryDownloadPassword.status;
          outcome = result.error ? "failed" : status === "ACCEPTED" ? "accepted" :
            status === "REFUSED" ? "refused" : "uncertain";
          setStatus(result.error ? t("queue.passwordRetryFailed") :
            status === "ACCEPTED" ? t("queue.passwordRetryAccepted") :
            status === "REFUSED" ? t("queue.passwordRetryRefused") : t("queue.passwordRetryUncertain"));
        } catch {
          setStatus(t("queue.passwordRetryUncertain"));
        } finally { finish(outcome); }
      }} />,
  };
}
