import * as React from "react";
import { Ban, Trash2 } from "lucide-react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { DeletePreviewSummary } from "@/components/common/delete-preview-summary";
import { IconButton } from "@/components/ui/icon-button";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import {
  blocklistExternalSubtitleMutation,
  deleteExternalSubtitleMutation,
} from "@/lib/graphql/mutations";
import { deleteExternalSubtitlePreviewQuery } from "@/lib/graphql/queries";
import { useDeletePreview } from "@/lib/hooks/use-delete-preview";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { LoadingMark } from "@/components/common/loading-mark";

function formatDateTime(value: string, dateTimeFormat: UiDateTimeFormat) {
  return formatUiDateTime(value, dateTimeFormat, { fallback: value });
}

function canBlocklistSubtitle(subtitle: ExternalSubtitleRecord) {
  return (
    subtitle.sourceKind === "downloaded" &&
    typeof subtitle.provider === "string" &&
    subtitle.provider.trim().length > 0 &&
    typeof subtitle.providerFileId === "string" &&
    subtitle.providerFileId.trim().length > 0
  );
}

function SubtitleFlag({
  label,
  className,
}: {
  label: string;
  className: string;
}) {
  return (
    <span className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${className}`}>
      {label}
    </span>
  );
}

type PendingSubtitleAction =
  | {
      kind: "delete" | "blocklist";
      subtitle: ExternalSubtitleRecord;
    }
  | null;

export function ExternalSubtitleSection({
  downloads,
  onChanged,
  allowBlocklist = false,
}: {
  downloads: ExternalSubtitleRecord[];
  onChanged?: () => void | Promise<void>;
  allowBlocklist?: boolean;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const setGlobalStatus = useGlobalStatus();
  const client = useClient();
  const [pendingAction, setPendingAction] = React.useState<PendingSubtitleAction>(null);
  const [typedConfirmation, setTypedConfirmation] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);

  const deletePreviewVariables = React.useMemo(
    () =>
      pendingAction
        ? { externalSubtitleId: pendingAction.subtitle.id }
        : null,
    [pendingAction],
  );
  const {
    preview: deletePreview,
    loading: deletePreviewLoading,
    error: deletePreviewError,
  } = useDeletePreview(
    deleteExternalSubtitlePreviewQuery,
    "deleteExternalSubtitlePreview",
    deletePreviewVariables,
    pendingAction !== null,
  );

  const closeDialog = React.useCallback(() => {
    if (submitting) {
      return;
    }
    setPendingAction(null);
    setTypedConfirmation("");
  }, [submitting]);

  const confirmDisabled =
    submitting ||
    deletePreviewLoading ||
    !!deletePreviewError ||
    !deletePreview ||
    (deletePreview.requiresTypedConfirmation &&
      typedConfirmation.trim() !== "DELETE");

  const handleConfirm = React.useCallback(async () => {
    if (!pendingAction || !deletePreview) {
      return;
    }

    setSubmitting(true);
    try {
      const variables =
        pendingAction.kind === "delete"
          ? {
              input: {
                externalSubtitleId: pendingAction.subtitle.id,
                previewFingerprint: deletePreview.fingerprint,
                typedConfirmation: typedConfirmation.trim() || undefined,
              },
            }
          : {
              input: {
                externalSubtitleId: pendingAction.subtitle.id,
                previewFingerprint: deletePreview.fingerprint,
                typedConfirmation: typedConfirmation.trim() || undefined,
              },
            };
      const mutation =
        pendingAction.kind === "delete"
          ? deleteExternalSubtitleMutation
          : blocklistExternalSubtitleMutation;

      const { error } = await client.mutation(mutation, variables).toPromise();
      if (error) {
        throw error;
      }

      setGlobalStatus(
        pendingAction.kind === "delete"
          ? t("subtitle.deleted")
          : t("subtitle.blocklisted"),
      );
      setPendingAction(null);
      setTypedConfirmation("");
      await onChanged?.();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.apiError"),
      );
    } finally {
      setSubmitting(false);
    }
  }, [
    client,
    deletePreview,
    onChanged,
    pendingAction,
    setGlobalStatus,
    t,
    typedConfirmation,
  ]);

  if (downloads.length === 0) {
    return null;
  }

  return (
    <>
      <div className="space-y-2">
        <p className="text-sm font-medium text-muted-foreground">{t("subtitle.external")}</p>
        <div className="space-y-2">
          {downloads.map((download) => {
            const canBlocklist = allowBlocklist && canBlocklistSubtitle(download);
            const canDelete = typeof onChanged === "function";
            return (
              <div
                key={download.id}
                className="rounded-lg border border-border/70 bg-background/40 px-3 py-3"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded-full border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-[var(--scry-info-text)]">
                      {download.language}
                    </span>
                    {download.provider ? (
                      <span className="rounded-full border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                        {download.provider}
                      </span>
                    ) : null}
                    {download.sourceKind === "discovered" ? (
                      <span className="rounded-full border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--scry-success-text)]">
                        {t("subtitle.onDisk")}
                      </span>
                    ) : null}
                    {download.synced ? (
                      <SubtitleFlag
                        label={t("subtitle.synced")}
                        className="bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
                      />
                    ) : null}
                    {download.hearingImpaired ? (
                      <SubtitleFlag
                        label={t("subtitle.hearingImpaired")}
                        className="bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]"
                      />
                    ) : null}
                    {download.forced ? (
                      <SubtitleFlag
                        label={t("subtitle.forced")}
                        className="bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]"
                      />
                    ) : null}
                    {download.aiTranslated ? (
                      <SubtitleFlag
                        label={t("subtitle.aiTranslated")}
                        className="bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text)]"
                      />
                    ) : null}
                    {download.machineTranslated ? (
                      <SubtitleFlag
                        label={t("subtitle.machineTranslated")}
                        className="bg-[var(--scry-danger-bg-strong)] text-[var(--scry-danger-text)]"
                      />
                    ) : null}
                    {download.scorePercent != null ? (
                      <span className="text-[11px] text-muted-foreground">
                        {t("subtitle.scoreWithValue", { score: `${download.scorePercent}%` })}
                      </span>
                    ) : null}
                  </div>
                  {canDelete || canBlocklist ? (
                    <div className="flex shrink-0 items-center gap-2">
                      {canBlocklist ? (
                        <IconButton
                          label={t("subtitle.blocklist")}
                          tone="disabled"
                          onClick={() => {
                            setPendingAction({ kind: "blocklist", subtitle: download });
                            setTypedConfirmation("");
                          }}
                        >
                          <Ban className="h-3.5 w-3.5" />
                        </IconButton>
                      ) : null}
                      {canDelete ? (
                        <IconButton
                          label={t("label.delete")}
                          tone="delete"
                          onClick={() => {
                            setPendingAction({ kind: "delete", subtitle: download });
                            setTypedConfirmation("");
                          }}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </IconButton>
                      ) : null}
                    </div>
                  ) : null}
                </div>
                <p className="mt-5 break-all font-[var(--font-code)] text-[11px] leading-5 text-muted-foreground">
                  {download.filePath}
                </p>
                <div className="mt-1.5 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                  <span>{formatDateTime(download.downloadedAt, dateTimeFormat)}</span>
                </div>
              </div>
            );
          })}
        </div>
      </div>
      <ConfirmDialog
        open={pendingAction !== null}
        title={pendingAction?.kind === "blocklist" ? t("subtitle.blocklist") : t("label.delete")}
        description={pendingAction?.subtitle.filePath ?? ""}
        confirmLabel={pendingAction?.kind === "blocklist" ? t("subtitle.blocklist") : t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={submitting}
        confirmDisabled={confirmDisabled}
        onConfirm={handleConfirm}
        onCancel={closeDialog}
      >
        <DeletePreviewSummary
          preview={deletePreview}
          loading={deletePreviewLoading}
          error={deletePreviewError}
          typedConfirmation={typedConfirmation}
          onTypedConfirmationChange={setTypedConfirmation}
        />
        {submitting ? (
          <div className="mt-3 flex items-center gap-2 text-xs text-muted-foreground">
            <LoadingMark className="h-3.5 w-3.5" />
            <span>
              {pendingAction?.kind === "blocklist"
                ? t("subtitle.blocklist")
                : t("label.delete")}
            </span>
          </div>
        ) : null}
      </ConfirmDialog>
    </>
  );
}
