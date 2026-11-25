import { useId, useState } from "react";
import { useClient } from "urql";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Textarea } from "@/components/ui/textarea";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { submitMediaIssueMutation } from "@/lib/graphql/mutations";
import type { MediaIssueRemediation } from "@/lib/types/media-issues";

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
  const [details, setDetails] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit() {
    if (busy) return;
    setBusy(true);
    setFailed(false);
    try {
      const { data, error } = await client
        .mutation<{ submitMediaIssue?: { id: string } }>(
          submitMediaIssueMutation,
          {
            input: {
              mediaFileId: file.id,
              reasonCode: "bad_media",
              details: details.trim() || null,
              remediation,
            },
          },
        )
        .toPromise();
      if (error || !data?.submitMediaIssue?.id) {
        setFailed(true);
        return;
      }
      setGlobalStatus(t("mediaFile.markBadSubmitted"), { level: "SUCCESS" });
      onClose();
    } catch {
      // The raw server error can contain a path to the reported file.
      setFailed(true);
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
      onConfirm={submit}
      onCancel={() => {
        if (!busy) onClose();
      }}
    >
      <div className="space-y-4">
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
        {failed ? (
          <p role="alert" className="text-sm text-destructive">
            {t("mediaFile.markBadSubmitFailed")}
          </p>
        ) : null}
      </div>
    </ConfirmDialog>
  );
}
