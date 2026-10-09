import { useCallback, useEffect, useState } from "react";
import { useClient } from "urql";

import { ScriptEditorDialogs } from "@/components/common/script-editor-dialogs";
import { SettingsPostProcessingSection } from "@/components/views/settings/settings-post-processing-section";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { updateScriptInterpreterSettingsMutation } from "@/lib/graphql/mutations";
import { scriptInterpreterSettingsQuery } from "@/lib/graphql/queries";
import { useScriptEditor } from "@/lib/hooks/use-script-editor";
import type {
  PostProcessingScript,
  PostProcessingScriptDraft,
  PostProcessingScriptRun,
  ScriptInterpreterSettings,
} from "@/lib/types/scripts";

export type PPScript = PostProcessingScript;
export type PPScriptRun = PostProcessingScriptRun;
export type PPScriptDraft = PostProcessingScriptDraft;

export type ScriptInterpreterDraft = Record<keyof ScriptInterpreterSettings, string>;

const EMPTY_INTERPRETERS: ScriptInterpreterDraft = {
  python: "",
  powershell: "",
  batch: "",
  go: "",
};

function interpreterDraftFrom(value: unknown): ScriptInterpreterDraft {
  const record = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  const read = (key: keyof ScriptInterpreterSettings) =>
    typeof record[key] === "string" ? (record[key] as string) : "";
  return {
    python: read("python"),
    powershell: read("powershell"),
    batch: read("batch"),
    go: read("go"),
  };
}

/** Interpreter paths saved as entered; an empty field clears the path. */
function interpreterInput(draft: ScriptInterpreterDraft): ScriptInterpreterSettings {
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
  const [draft, setDraft] = useState<ScriptInterpreterDraft>(EMPTY_INTERPRETERS);
  const [baseline, setBaseline] = useState<ScriptInterpreterDraft>(EMPTY_INTERPRETERS);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void client
      .query(scriptInterpreterSettingsQuery, {}, { requestPolicy: "network-only" })
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error) {
          setGlobalStatus(error.message);
          return;
        }
        const loaded = interpreterDraftFrom(data?.scriptInterpreterSettings);
        setDraft(loaded);
        setBaseline(loaded);
      });
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus]);

  const save = useCallback(async () => {
    setIsSaving(true);
    try {
      const { data, error } = await client
        .mutation(updateScriptInterpreterSettingsMutation, { input: interpreterInput(draft) })
        .toPromise();
      if (error) throw error;
      const saved = interpreterDraftFrom(data?.updateScriptInterpreterSettings);
      setDraft(saved);
      setBaseline(saved);
      setGlobalStatus(t("settings.scriptInterpreters.saved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setIsSaving(false);
    }
  }, [client, draft, setGlobalStatus, t]);

  return {
    draft,
    setDraft,
    isDirty: JSON.stringify(draft) !== JSON.stringify(baseline),
    isSaving,
    save,
  };
}

export function SettingsPostProcessingContainer() {
  const editor = useScriptEditor("POST_IMPORT");
  const interpreters = useScriptInterpreterSettings();
  const [expandedScriptId, setExpandedScriptId] = useState<string | null>(null);

  return (
    <>
      <SettingsPostProcessingSection
        scripts={editor.scripts}
        isEditorOpen={editor.isEditorOpen}
        editorMode={editor.editingScriptId ? "edit" : "create"}
        editingScriptId={editor.editingScriptId}
        scriptDraft={editor.scriptDraft}
        setScriptDraft={editor.setScriptDraft}
        submitScript={editor.submitScript}
        mutatingScriptId={editor.mutatingScriptId}
        resetDraft={editor.requestCloseEditor}
        startCreateScript={editor.requestCreateEditor}
        editScript={editor.requestEditScript}
        toggleScript={editor.toggleScript}
        deleteScript={editor.requestDeleteScript}
        expandedScriptId={expandedScriptId}
        setExpandedScriptId={setExpandedScriptId}
        scriptRuns={editor.scriptRuns}
        loadRunsForScript={editor.loadRunsForScript}
        interpreterDraft={interpreters.draft}
        setInterpreterDraft={interpreters.setDraft}
        interpretersDirty={interpreters.isDirty}
        interpretersSaving={interpreters.isSaving}
        saveInterpreters={interpreters.save}
      />
      <ScriptEditorDialogs
        state={editor.dialogs}
        ids={{
          inlineShellContent: "settings-post-processing-inline-shell-confirm",
          inlineShellAccept: "settings-post-processing-inline-shell-confirm-accept",
          inlineShellCancel: "settings-post-processing-inline-shell-confirm-cancel",
        }}
      />
    </>
  );
}
