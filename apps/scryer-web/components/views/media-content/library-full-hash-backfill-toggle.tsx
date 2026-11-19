import { useCallback, useEffect, useId, useState } from "react";
import { useClient } from "urql";
import { Checkbox } from "@/components/ui/checkbox";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { libraryFullHashBackfillQuery } from "@/lib/graphql/queries";
import { setLibraryFullHashBackfillMutation } from "@/lib/graphql/mutations";

type LibraryFullHashBackfillRow = {
  id: string;
  fullHashBackfillEnabled: boolean;
};

type LibraryFullHashBackfillResult = {
  libraries?: LibraryFullHashBackfillRow[] | null;
};

type SetLibraryFullHashBackfillResult = {
  setLibraryFullHashBackfill?: LibraryFullHashBackfillRow | null;
};

/**
 * Whether the nightly full-hash job reads this library's files. The change
 * saves as soon as the box is ticked; it is not part of the library draft.
 */
export function LibraryFullHashBackfillToggle({
  libraryId,
  disabled,
}: {
  libraryId: string;
  disabled?: boolean;
}) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const checkboxId = useId();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setEnabled(null);
    const { data, error } = await client
      .query<LibraryFullHashBackfillResult>(
        libraryFullHashBackfillQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise();
    if (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
      return;
    }
    const row = data?.libraries?.find((library) => library.id === libraryId);
    setEnabled(row ? row.fullHashBackfillEnabled : null);
  }, [client, libraryId, setGlobalStatus, t]);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async (next: boolean) => {
    const previous = enabled;
    setEnabled(next);
    setSaving(true);
    try {
      const { data, error } = await client
        .mutation<SetLibraryFullHashBackfillResult>(
          setLibraryFullHashBackfillMutation,
          { input: { libraryId, enabled: next } },
        )
        .toPromise();
      if (error) throw error;
      const saved = data?.setLibraryFullHashBackfill;
      if (saved) setEnabled(saved.fullHashBackfillEnabled);
      setGlobalStatus(
        t(
          next
            ? "settings.libraryFullHashBackfillEnabledStatus"
            : "settings.libraryFullHashBackfillDisabledStatus",
        ),
        { level: "SUCCESS" },
      );
    } catch (error) {
      setEnabled(previous);
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      id="library-full-hash-backfill"
      className="flex items-start justify-between gap-4"
    >
      <div className="min-w-0 space-y-1">
        <p className="text-sm font-medium text-[var(--scry-ink2)]">
          {t("settings.libraryFullHashBackfillLabel")}
        </p>
        <p className="text-xs text-muted-foreground">
          {t("settings.libraryFullHashBackfillDescription")}
        </p>
      </div>
      <label
        htmlFor={checkboxId}
        className="flex shrink-0 cursor-pointer items-center gap-3 text-sm font-medium"
      >
        <span>{t("label.enabled")}</span>
        <Checkbox
          id={checkboxId}
          size="large"
          checked={enabled === true}
          disabled={disabled || saving || enabled === null}
          onCheckedChange={(value) => void save(value === true)}
        />
      </label>
    </div>
  );
}
