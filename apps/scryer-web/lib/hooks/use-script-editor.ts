import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  createPostProcessingScriptMutation,
  deletePostProcessingScriptMutation,
  togglePostProcessingScriptMutation,
  updatePostProcessingScriptMutation,
} from "@/lib/graphql/mutations";
import {
  postProcessingScriptRunsQuery,
  postProcessingScriptsQuery,
  scheduledScriptsQuery,
} from "@/lib/graphql/queries";
import type {
  PostProcessingScript,
  PostProcessingScriptDraft,
  PostProcessingScriptRun,
  ScriptLanguage,
  ScriptTrigger,
} from "@/lib/types/scripts";
import {
  defaultScriptSchedule,
  normalizeScriptSchedule,
  toScriptScheduleInput,
} from "@/lib/utils/script-schedule";

const LANGUAGES: readonly ScriptLanguage[] = ["SHELL", "PYTHON", "POWERSHELL", "BATCH", "GO"];

function initialDraft(trigger: ScriptTrigger): PostProcessingScriptDraft {
  return {
    name: "",
    description: "",
    scriptType: "inline",
    scriptContent: "",
    appliedFacets: [],
    executionMode: "BLOCKING",
    timeoutSecs: 300,
    priority: 0,
    enabled: true,
    debug: true,
    language: "SHELL",
    trigger,
    schedule: trigger === "SCHEDULE" ? defaultScriptSchedule() : null,
    runOnStartup: false,
  };
}

function cloneDraft(draft: PostProcessingScriptDraft): PostProcessingScriptDraft {
  return {
    ...draft,
    appliedFacets: [...draft.appliedFacets],
    schedule: draft.schedule
      ? { ...draft.schedule, days: draft.schedule.days ? [...draft.schedule.days] : null }
      : null,
  };
}

function draftFromScript(record: PostProcessingScript): PostProcessingScriptDraft {
  return cloneDraft({
    name: record.name,
    description: record.description,
    scriptType: record.scriptType,
    scriptContent: record.scriptContent,
    appliedFacets: record.appliedFacets,
    executionMode: record.executionMode,
    timeoutSecs: record.timeoutSecs,
    priority: record.priority,
    enabled: record.enabled,
    debug: record.debug,
    language: record.language,
    trigger: record.trigger,
    schedule:
      record.trigger === "SCHEDULE" ? (record.schedule ?? defaultScriptSchedule()) : record.schedule,
    runOnStartup: record.runOnStartup,
  });
}

/** Reads a script payload, filling fields an older response may lack. */
export function normalizePostProcessingScript(value: unknown): PostProcessingScript | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== "string") return null;
  return {
    id: record.id,
    name: typeof record.name === "string" ? record.name : "",
    description: typeof record.description === "string" ? record.description : "",
    scriptType: typeof record.scriptType === "string" ? record.scriptType : "inline",
    scriptContent: typeof record.scriptContent === "string" ? record.scriptContent : "",
    appliedFacets: Array.isArray(record.appliedFacets)
      ? record.appliedFacets.filter((facet): facet is string => typeof facet === "string")
      : [],
    executionMode: typeof record.executionMode === "string" ? record.executionMode : "BLOCKING",
    timeoutSecs: typeof record.timeoutSecs === "number" ? record.timeoutSecs : 300,
    priority: typeof record.priority === "number" ? record.priority : 0,
    enabled: record.enabled === true,
    debug: record.debug === true,
    language: LANGUAGES.find((language) => language === record.language) ?? "SHELL",
    trigger: record.trigger === "SCHEDULE" ? "SCHEDULE" : "POST_IMPORT",
    schedule: normalizeScriptSchedule(record.schedule),
    runOnStartup: record.runOnStartup === true,
    scheduleDescription:
      typeof record.scheduleDescription === "string" ? record.scheduleDescription : null,
    createdAt: typeof record.createdAt === "string" ? record.createdAt : "",
    updatedAt: typeof record.updatedAt === "string" ? record.updatedAt : "",
  };
}

type PendingScriptEditorAction =
  | { type: "create" }
  | { type: "edit"; record: PostProcessingScript }
  | { type: "close" }
  | null;

type PendingInlineShellAction =
  | { type: "save" }
  | { type: "toggle"; record: PostProcessingScript }
  | null;

/**
 * Lists the user scripts with one trigger and drives their editor: create,
 * edit, enable or disable, and delete, with the inline-shell acknowledgement
 * and the discard-changes confirmation.
 */
export function useScriptEditor(trigger: ScriptTrigger, options: { onChanged?: () => void } = {}) {
  const { onChanged } = options;
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [scripts, setScripts] = useState<PostProcessingScript[]>([]);
  const [editingScriptId, setEditingScriptId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteScript, setPendingDeleteScript] = useState<PostProcessingScript | null>(null);
  const [pendingEditorAction, setPendingEditorAction] = useState<PendingScriptEditorAction>(null);
  const [pendingInlineShellAction, setPendingInlineShellAction] =
    useState<PendingInlineShellAction>(null);
  const [mutatingScriptId, setMutatingScriptId] = useState<string | null>(null);
  const [scriptDraft, setScriptDraft] = useState<PostProcessingScriptDraft>(() =>
    initialDraft(trigger),
  );
  const [scriptDraftBaseline, setScriptDraftBaseline] = useState<PostProcessingScriptDraft>(() =>
    initialDraft(trigger),
  );
  const [scriptRuns, setScriptRuns] = useState<Record<string, PostProcessingScriptRun[]>>({});

  const closeEditor = useCallback(() => {
    setIsEditorOpen(false);
    setEditingScriptId(null);
    setScriptDraft(() => initialDraft(trigger));
    setScriptDraftBaseline(() => initialDraft(trigger));
  }, [trigger]);

  const isDraftDirty = JSON.stringify(scriptDraft) !== JSON.stringify(scriptDraftBaseline);

  const scriptDraftRequiresInlineShellAcknowledgement =
    scriptDraft.scriptType === "inline" &&
    (!editingScriptId ||
      scriptDraftBaseline.scriptType !== "inline" ||
      scriptDraft.scriptContent !== scriptDraftBaseline.scriptContent ||
      (trigger === "SCHEDULE" && scriptDraft.enabled && !scriptDraftBaseline.enabled));

  const openCreateEditor = useCallback(() => {
    const nextDraft = initialDraft(trigger);
    setEditingScriptId(null);
    setScriptDraft(nextDraft);
    setScriptDraftBaseline(cloneDraft(nextDraft));
    setIsEditorOpen(true);
  }, [trigger]);

  const openEditEditor = useCallback(
    (record: PostProcessingScript) => {
      const nextDraft = draftFromScript(record);
      setEditingScriptId(record.id);
      setScriptDraft(nextDraft);
      setScriptDraftBaseline(cloneDraft(nextDraft));
      setIsEditorOpen(true);
      setGlobalStatus(t("status.editingRule", { name: record.name }));
    },
    [setGlobalStatus, t],
  );

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditScript = useCallback(
    (record: PostProcessingScript) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(record);
        return;
      }
      setPendingEditorAction({ type: "edit", record });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      closeEditor();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [closeEditor, isDraftDirty, isEditorOpen]);

  const refreshScripts = useCallback(async () => {
    try {
      const query = trigger === "SCHEDULE" ? scheduledScriptsQuery : postProcessingScriptsQuery;
      const { data, error } = await client
        .query(query, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setScripts(
        ((Array.isArray(data?.postProcessingScripts) ? data.postProcessingScripts : []) as unknown[])
          .map(normalizePostProcessingScript)
          .filter((script): script is PostProcessingScript => script !== null),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t, trigger]);

  useEffect(() => {
    void refreshScripts();
  }, [refreshScripts]);

  const afterChange = useCallback(async () => {
    await refreshScripts();
    onChanged?.();
  }, [onChanged, refreshScripts]);

  const loadRunsForScript = useCallback(
    async (scriptId: string) => {
      try {
        const { data, error } = await client
          .query(postProcessingScriptRunsQuery, { scriptId, limit: 20 }, { requestPolicy: "network-only" })
          .toPromise();
        if (error) throw error;
        setScriptRuns((prev) => ({
          ...prev,
          [scriptId]: data.postProcessingScriptRuns || [],
        }));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      }
    },
    [client, setGlobalStatus, t],
  );

  const saveScript = useCallback(
    async (inlineShellAcknowledged = false) => {
      const isScheduled = trigger === "SCHEDULE";
      const payload = {
        name: scriptDraft.name.trim(),
        description: scriptDraft.description.trim(),
        scriptType: scriptDraft.scriptType,
        scriptContent: scriptDraft.scriptContent,
        executionMode: scriptDraft.executionMode,
        timeoutSecs: scriptDraft.timeoutSecs,
        debug: scriptDraft.debug,
        language: scriptDraft.language,
        trigger,
        ...(isScheduled
          ? {
              schedule: toScriptScheduleInput(scriptDraft.schedule ?? defaultScriptSchedule()),
              runOnStartup: scriptDraft.runOnStartup,
            }
          : {
              appliedFacets: scriptDraft.appliedFacets,
              priority: scriptDraft.priority,
            }),
        ...(inlineShellAcknowledged ? { inlineShellAcknowledged: true } : {}),
      };

      if (!payload.name || !payload.scriptContent.trim()) {
        setGlobalStatus(t("settings.ruleValidationRequired"));
        return;
      }

      setMutatingScriptId(editingScriptId || "new");
      try {
        if (editingScriptId) {
          const { error } = await client
            .mutation(updatePostProcessingScriptMutation, {
              input: {
                id: editingScriptId,
                ...payload,
                ...(isScheduled ? { enabled: scriptDraft.enabled } : {}),
              },
            })
            .toPromise();
          if (error) throw error;
          setGlobalStatus(t("settings.pp.updated"));
        } else {
          const { data, error } = await client
            .mutation(createPostProcessingScriptMutation, { input: payload })
            .toPromise();
          if (error) throw error;
          // Creation takes no enabled state; a job saved disabled is switched off after.
          const createdId = data?.createPostProcessingScript?.id;
          if (isScheduled && !scriptDraft.enabled && typeof createdId === "string") {
            const { error: disableError } = await client
              .mutation(updatePostProcessingScriptMutation, {
                input: { id: createdId, enabled: false },
              })
              .toPromise();
            if (disableError) throw disableError;
          }
          setGlobalStatus(t("settings.pp.created"));
        }
        closeEditor();
        await afterChange();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingScriptId(null);
      }
    },
    [afterChange, client, closeEditor, editingScriptId, scriptDraft, setGlobalStatus, t, trigger],
  );

  const submitScript = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!scriptDraft.name.trim() || !scriptDraft.scriptContent.trim()) {
      setGlobalStatus(t("settings.ruleValidationRequired"));
      return;
    }
    if (scriptDraftRequiresInlineShellAcknowledgement) {
      setPendingInlineShellAction({ type: "save" });
      return;
    }
    void saveScript(false);
  };

  const executeToggleScript = useCallback(
    async (record: PostProcessingScript, inlineShellAcknowledged = false) => {
      setMutatingScriptId(record.id);
      try {
        const { error } = await client
          .mutation(togglePostProcessingScriptMutation, {
            id: record.id,
            ...(inlineShellAcknowledged ? { inlineShellAcknowledged: true } : {}),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.pp.toggled", {
            state: record.enabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await afterChange();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingScriptId(null);
      }
    },
    [afterChange, client, setGlobalStatus, t],
  );

  const toggleScript = useCallback(
    async (record: PostProcessingScript) => {
      if (record.scriptType === "inline" && !record.enabled) {
        setPendingInlineShellAction({ type: "toggle", record });
        return;
      }
      await executeToggleScript(record, false);
    },
    [executeToggleScript],
  );

  const confirmDeleteScript = async () => {
    if (!pendingDeleteScript) return;
    const record = pendingDeleteScript;
    setMutatingScriptId(record.id);
    try {
      const { error } = await client
        .mutation(deletePostProcessingScriptMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.pp.deleted"));
      await afterChange();
      if (editingScriptId === record.id) {
        closeEditor();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingScriptId(null);
      setPendingDeleteScript(null);
    }
  };

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.record);
    } else {
      closeEditor();
    }
    setPendingEditorAction(null);
  }, [closeEditor, openCreateEditor, openEditEditor, pendingEditorAction]);

  const confirmPendingInlineShellAction = useCallback(() => {
    const action = pendingInlineShellAction;
    if (!action) return;
    setPendingInlineShellAction(null);
    if (action.type === "save") {
      void saveScript(true);
    } else {
      void executeToggleScript(action.record, true);
    }
  }, [executeToggleScript, pendingInlineShellAction, saveScript]);

  return {
    scripts,
    refreshScripts,
    isEditorOpen,
    editingScriptId,
    scriptDraft,
    setScriptDraft,
    mutatingScriptId,
    submitScript,
    requestCreateEditor,
    requestEditScript,
    requestCloseEditor,
    toggleScript,
    requestDeleteScript: setPendingDeleteScript,
    scriptRuns,
    loadRunsForScript,
    dialogs: {
      pendingDeleteScript,
      confirmDeleteScript,
      cancelDeleteScript: () => setPendingDeleteScript(null),
      pendingEditorAction,
      confirmPendingEditorAction,
      cancelPendingEditorAction: () => setPendingEditorAction(null),
      pendingInlineShellAction,
      confirmPendingInlineShellAction,
      cancelPendingInlineShellAction: () => setPendingInlineShellAction(null),
      mutatingScriptId,
    },
  };
}

export type ScriptEditorDialogsState = ReturnType<typeof useScriptEditor>["dialogs"];
