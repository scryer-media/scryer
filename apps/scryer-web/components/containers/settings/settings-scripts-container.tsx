import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";

import { SettingsScriptInterpretersSection } from "@/components/views/settings/settings-script-interpreters-section";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { updateScriptInterpreterSettingsMutation } from "@/lib/graphql/mutations";
import { scriptInterpreterSettingsQuery } from "@/lib/graphql/queries";
import type { ScriptInterpreterSettings } from "@/lib/types/scripts";

export type ScriptInterpreterDraft = Record<
  keyof ScriptInterpreterSettings,
  string
>;

const EMPTY_INTERPRETERS: ScriptInterpreterDraft = {
  python: "",
  powershell: "",
  batch: "",
  go: "",
};

function interpreterDraftFrom(value: unknown): ScriptInterpreterDraft {
  const record =
    typeof value === "object" && value !== null
      ? (value as Record<string, unknown>)
      : {};
  const read = (key: keyof ScriptInterpreterSettings) =>
    typeof record[key] === "string" ? (record[key] as string) : "";
  return {
    python: read("python"),
    powershell: read("powershell"),
    batch: read("batch"),
    go: read("go"),
  };
}

function sameInterpreters(
  left: ScriptInterpreterDraft,
  right: ScriptInterpreterDraft,
): boolean {
  return (
    left.python === right.python &&
    left.powershell === right.powershell &&
    left.batch === right.batch &&
    left.go === right.go
  );
}

/** Interpreter paths saved as entered; an empty field clears the path. */
function interpreterInput(
  draft: ScriptInterpreterDraft,
): ScriptInterpreterSettings {
  const value = (path: string) => (path.trim() ? path.trim() : null);
  return {
    python: value(draft.python),
    powershell: value(draft.powershell),
    batch: value(draft.batch),
    go: value(draft.go),
  };
}

function useScriptInterpreterSettings() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const [draft, setDraft] =
    useState<ScriptInterpreterDraft>(EMPTY_INTERPRETERS);
  const [baseline, setBaseline] =
    useState<ScriptInterpreterDraft>(EMPTY_INTERPRETERS);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void client
      .query(
        scriptInterpreterSettingsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error) {
          setGlobalStatus(
            userFacingGraphQlErrorMessage(error, t("status.apiError"), t),
            { level: "ERROR" },
          );
          return;
        }
        const loaded = interpreterDraftFrom(data?.scriptInterpreterSettings);
        // Keep anything typed while the settings were loading, and fill every
        // field still empty so saving an early edit cannot clear other paths.
        setDraft((current) => ({
          python: current.python || loaded.python,
          powershell: current.powershell || loaded.powershell,
          batch: current.batch || loaded.batch,
          go: current.go || loaded.go,
        }));
        setBaseline(loaded);
      });
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  const save = useCallback(async () => {
    setIsSaving(true);
    try {
      const { data, error } = await client
        .mutation(updateScriptInterpreterSettingsMutation, {
          input: interpreterInput(draft),
        })
        .toPromise();
      if (error) throw error;
      const returned = data?.updateScriptInterpreterSettings;
      // Show what the server stored; keep the entered paths if it echoed nothing back.
      const saved =
        typeof returned === "object" && returned !== null
          ? interpreterDraftFrom(returned)
          : interpreterDraftFrom(interpreterInput(draft));
      setDraft(saved);
      setBaseline(saved);
      setGlobalStatus(t("settings.scriptInterpreters.saved"), {
        level: "SUCCESS",
      });
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
      );
    } finally {
      setIsSaving(false);
    }
  }, [client, draft, setGlobalStatus, t]);

  return {
    draft,
    setDraft,
    isDirty: !sameInterpreters(draft, baseline),
    isSaving,
    save,
  };
}

export function SettingsScriptsContainer() {
  const interpreters = useScriptInterpreterSettings();

  return (
    <SettingsScriptInterpretersSection
      draft={interpreters.draft}
      setDraft={interpreters.setDraft}
      isDirty={interpreters.isDirty}
      isSaving={interpreters.isSaving}
      onSave={interpreters.save}
    />
  );
}
