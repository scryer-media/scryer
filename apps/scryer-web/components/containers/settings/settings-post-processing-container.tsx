import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsPostProcessingSection } from "@/components/views/settings/settings-post-processing-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  postProcessingScriptsQuery,
  postProcessingScriptRunsQuery,
} from "@/lib/graphql/queries";
import {
  createPostProcessingScriptMutation,
  updatePostProcessingScriptMutation,
  deletePostProcessingScriptMutation,
  togglePostProcessingScriptMutation,
} from "@/lib/graphql/mutations";

export type PPScript = {
  id: string;
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  enabled: boolean;
  debug: boolean;
  createdAt: string;
  updatedAt: string;
};

export type PPScriptRun = {
  id: string;
  scriptId: string;
  scriptName: string;
  titleId: string | null;
  titleName: string | null;
  facet: string | null;
  filePath: string | null;
  status: string;
  exitCode: number | null;
  stdoutTail: string | null;
  stderrTail: string | null;
  durationMs: number | null;
  startedAt: string;
  completedAt: string | null;
};

export type PPScriptDraft = {
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  debug: boolean;
};

const INITIAL_DRAFT: PPScriptDraft = {
  name: "",
  description: "",
  scriptType: "inline",
  scriptContent: "",
  appliedFacets: [],
  executionMode: "blocking",
  timeoutSecs: 300,
  priority: 0,
  debug: false,
};

export function SettingsPostProcessingContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [scripts, setScripts] = useState<PPScript[]>([]);
  const [editingScriptId, setEditingScriptId] = useState<string | null>(null);
  const [pendingDeleteScript, setPendingDeleteScript] = useState<PPScript | null>(null);
  const [mutatingScriptId, setMutatingScriptId] = useState<string | null>(null);
  const [scriptDraft, setScriptDraft] = useState<PPScriptDraft>(() => ({ ...INITIAL_DRAFT }));
  const [expandedScriptId, setExpandedScriptId] = useState<string | null>(null);
  const [scriptRuns, setScriptRuns] = useState<Record<string, PPScriptRun[]>>({});

  const resetDraft = useCallback(() => {
    setEditingScriptId(null);
    setScriptDraft(() => ({ ...INITIAL_DRAFT }));
  }, []);

  const refreshScripts = useCallback(async () => {
    try {
      const { data, error } = await client.query(postProcessingScriptsQuery, {}).toPromise();
      if (error) throw error;
      setScripts(data.postProcessingScripts || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshScripts();
  }, [refreshScripts]);

  const loadRunsForScript = useCallback(
    async (scriptId: string) => {
      try {
        const { data, error } = await client
          .query(postProcessingScriptRunsQuery, { scriptId, limit: 20 })
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

  const submitScript = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: scriptDraft.name.trim(),
      description: scriptDraft.description.trim(),
      scriptType: scriptDraft.scriptType,
      scriptContent: scriptDraft.scriptContent,
      appliedFacets: scriptDraft.appliedFacets,
      executionMode: scriptDraft.executionMode,
      timeoutSecs: scriptDraft.timeoutSecs,
      priority: scriptDraft.priority,
      debug: scriptDraft.debug,
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
            input: { id: editingScriptId, ...payload },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.pp.updated"));
      } else {
        const { error } = await client
          .mutation(createPostProcessingScriptMutation, { input: payload })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.pp.created"));
      }
      resetDraft();
      await refreshScripts();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingScriptId(null);
    }
  };

  const editScript = (record: PPScript) => {
    setEditingScriptId(record.id);
    setScriptDraft({
      name: record.name,
      description: record.description,
      scriptType: record.scriptType,
      scriptContent: record.scriptContent,
      appliedFacets: [...record.appliedFacets],
      executionMode: record.executionMode,
      timeoutSecs: record.timeoutSecs,
      priority: record.priority,
      debug: record.debug,
    });
    setGlobalStatus(t("status.editingRule", { name: record.name }));
  };

  const deleteScript = (record: PPScript) => {
    setPendingDeleteScript(record);
  };

  const toggleScript = useCallback(
    async (record: PPScript) => {
      setMutatingScriptId(record.id);
      try {
        const { error } = await client
          .mutation(togglePostProcessingScriptMutation, { id: record.id })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.pp.toggled", {
            state: record.enabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await refreshScripts();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingScriptId(null);
      }
    },
    [client, refreshScripts, setGlobalStatus, t],
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
      await refreshScripts();
      if (editingScriptId === record.id) {
        resetDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingScriptId(null);
      setPendingDeleteScript(null);
    }
  };

  return (
    <>
      <SettingsPostProcessingSection
        scripts={scripts}
        editingScriptId={editingScriptId}
        scriptDraft={scriptDraft}
        setScriptDraft={setScriptDraft}
        submitScript={submitScript}
        mutatingScriptId={mutatingScriptId}
        resetDraft={resetDraft}
        editScript={editScript}
        toggleScript={toggleScript}
        deleteScript={deleteScript}
        expandedScriptId={expandedScriptId}
        setExpandedScriptId={setExpandedScriptId}
        scriptRuns={scriptRuns}
        loadRunsForScript={loadRunsForScript}
      />
      <ConfirmDialog
        open={pendingDeleteScript !== null}
        title={t("label.delete")}
        description={
          pendingDeleteScript
            ? t("status.deletingRule", { name: pendingDeleteScript.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmDeleteScript}
        onCancel={() => setPendingDeleteScript(null)}
      />
    </>
  );
}
