import { useId, useState } from "react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Textarea } from "@/components/ui/textarea";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { graphQlErrorExtension } from "@/lib/graphql/error-message";
import { submitMediaIssueMutation } from "@/lib/graphql/mutations";
import type {
  MediaIssueReason,
  MediaIssueRemediation,
} from "@/lib/types/media-issues";
import {
  MEDIA_ISSUE_REASONS,
  mediaIssueReasonKey,
  mediaIssueRefusalKey,
} from "@/lib/utils/media-issue-review";

const selectClass =
  "h-9 max-w-full rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-2 focus-visible:outline-ring";

export function ReportMediaIssueDialog({
  file,
  onClose,
}: {
  file: { id: string; label: string };
  onClose: () => void;
}) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const choiceId = useId();
  const [remediation, setRemediation] =
    useState<MediaIssueRemediation>("MARK_BAD");
  const [reason, setReason] = useState<MediaIssueReason>("corrupted");
  const [details, setDetails] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const detailsRequired = reason === "other" && details.trim() === "";

  function fail(error?: unknown) {
    // The raw server error can contain a path to the reported file, so only
    // a known refusal code is shown.
    const key = mediaIssueRefusalKey(
      error ? graphQlErrorExtension(error, "VALIDATION_ERROR", "reason") : null,
    );
    setFailure(t(key ?? "mediaFile.markBadSubmitFailed"));
  }

  async function submit() {
    if (busy || detailsRequired) return;
    setBusy(true);
    setFailure(null);
    try {
      const { data, error } = await client
        .mutation<{ submitMediaIssue?: { id: string } }>(
          submitMediaIssueMutation,
          {
            input: {
              mediaFileId: file.id,
              reasonCode: reason,
              details: details.trim() || null,
              remediation,
            },
          },
        )
        .toPromise();
      if (error || !data?.submitMediaIssue?.id) {
        fail(error);
        return;
      }
      setGlobalStatus(t("mediaFile.markBadSubmitted"), { level: "SUCCESS" });
      onClose();
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  }

  return (
    <ConfirmDialog
      open
      title={t("mediaFile.markBad")}
      description={t("mediaFile.markBadDescription", { file: file.label })}
      confirmLabel={t("mediaFile.submitBadReport")}
      confirmButtonVariant="default"
      cancelLabel={t("label.cancel")}
      isBusy={busy}
      confirmDisabled={detailsRequired}
      onConfirm={submit}
      onCancel={() => {
        if (!busy) onClose();
      }}
    >
      <div className="space-y-4">
        <label className="grid gap-1.5 text-sm font-medium">
          {t("mediaIssue.reasonLabel")}
          <select
            className={selectClass}
            value={reason}
            disabled={busy}
            onChange={(event) =>
              setReason(event.target.value as MediaIssueReason)
            }
          >
            {MEDIA_ISSUE_REASONS.map((value) => (
              <option key={value} value={value}>
                {t(mediaIssueReasonKey(value))}
              </option>
            ))}
          </select>
        </label>
        <fieldset disabled={busy} className="space-y-2">
          <legend className="mb-2 text-sm font-medium">
            {t("mediaIssue.requestedAction")}
          </legend>
          {(
            [
              [
                "MARK_BAD",
                "mediaFile.markBadOnly",
                "mediaFile.markBadOnlyHint",
              ],
              [
                "RECYCLE_AND_SEARCH",
                "mediaFile.markBadRecycleSearch",
                "mediaFile.markBadRecycleSearchHint",
              ],
            ] as const
          ).map(([value, label, hint]) => (
            <label
              key={value}
              className="flex items-start gap-3 rounded-md border border-border p-3 has-[:checked]:border-primary"
            >
              <input
                type="radio"
                name={choiceId}
                className="mt-1 accent-primary"
                checked={remediation === value}
                onChange={() => setRemediation(value)}
              />
              <span className="grid gap-1 text-sm font-medium">
                {t(label)}
                <span className="text-xs font-normal text-muted-foreground">
                  {t(hint)}
                </span>
              </span>
            </label>
          ))}
        </fieldset>
        <label className="grid gap-1.5 text-sm font-medium">
          {t("mediaFile.markBadReason")}
          <Textarea
            value={details}
            onChange={(event) => setDetails(event.target.value)}
            disabled={busy}
          />
        </label>
        {detailsRequired ? (
          <p className="text-xs text-muted-foreground">
            {t("error.reason.MEDIA_REASON_REQUIRES_DETAILS")}
          </p>
        ) : null}
        {failure ? (
          <p role="alert" className="text-sm text-destructive">
            {failure}
          </p>
        ) : null}
      </div>
    </ConfirmDialog>
  );
}
