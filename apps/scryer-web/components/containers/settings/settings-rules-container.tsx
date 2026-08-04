import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsRulesSection } from "@/components/views/settings/settings-rules-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { RuleSetRecord, RuleSetDraft, RuleValidationResult } from "@/lib/types/rule-sets";
import { copyRuleSetDraft, createRuleSetInput } from "@/lib/utils/rule-sets";
import { conflictingFrenchPack, parseTagFilterInput } from "@/lib/utils/trash-packs";
import { ruleSetsQuery } from "@/lib/graphql/queries";
import {
  createRuleSetMutation,
  deleteRuleSetMutation,
  toggleRuleSetMutation,
  updateRuleSetMutation,
  validateRuleSetMutation,
} from "@/lib/graphql/mutations";

const RULE_SET_INITIAL_DRAFT: RuleSetDraft = {
  name: "",
  description: "",
  regoSource: 'import rego.v1\n\nscore_entry["size_guard"] := scryer.block_score() if {\n    scryer.size_gib(input.release.size_bytes) > 100\n}\n',
  enabled: true,
  priority: 0,
  appliedFacets: [],
};

type PendingRuleEditorAction =
  | { type: "create" }
  | { type: "copy"; record: RuleSetRecord }
  | { type: "edit"; record: RuleSetRecord }
  | { type: "close" }
  | {
      type: "template";
      template: {
        title: string;
        description: string;
        regoSource: string;
        appliedFacets?: string[];
      };
    }
  | null;

export function SettingsRulesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [ruleSetRecords, setRuleSetRecords] = useState<RuleSetRecord[]>([]);
  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(null);
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteRuleSet, setPendingDeleteRuleSet] = useState<RuleSetRecord | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingRuleEditorAction>(null);
  const [ruleSetDraft, setRuleSetDraft] = useState<RuleSetDraft>(() => ({ ...RULE_SET_INITIAL_DRAFT }));
  const [ruleSetDraftBaseline, setRuleSetDraftBaseline] = useState<RuleSetDraft>(() => ({
    ...RULE_SET_INITIAL_DRAFT,
  }));
  const [validating, setValidating] = useState(false);
  const [validationResult, setValidationResult] = useState<RuleValidationResult | null>(null);

  const closeRuleSetEditor = useCallback(() => {
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    setRuleSetDraft(() => ({ ...RULE_SET_INITIAL_DRAFT }));
    setRuleSetDraftBaseline(() => ({ ...RULE_SET_INITIAL_DRAFT }));
    setValidationResult(null);
  }, []);

  const isRuleDraftDirty =
    JSON.stringify(ruleSetDraft) !== JSON.stringify(ruleSetDraftBaseline);

  const openCreateRuleEditor = useCallback(() => {
    const nextDraft = { ...RULE_SET_INITIAL_DRAFT };
    setEditingRuleSetId(null);
    setRuleSetDraft(nextDraft);
    setRuleSetDraftBaseline(nextDraft);
    setValidationResult(null);
    setIsEditorOpen(true);
  }, []);

  const openEditRuleEditor = useCallback(
    (record: RuleSetRecord) => {
      const nextDraft = {
        name: record.name,
        description: record.description,
        regoSource: record.regoSource,
        enabled: record.enabled,
        priority: record.priority,
        appliedFacets: [...record.appliedFacets],
      };
      setEditingRuleSetId(record.id);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setIsEditorOpen(true);
      setGlobalStatus(t("status.editingRule", { name: record.name }));
    },
    [setGlobalStatus, t],
  );

  const openCopyRuleEditor = useCallback(
    (record: RuleSetRecord) => {
      const nextDraft = copyRuleSetDraft(record);
      setEditingRuleSetId(null);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [],
  );

  const openTemplateRuleEditor = useCallback(
    (template: {
      title: string;
      description: string;
      regoSource: string;
      appliedFacets?: string[];
    }) => {
      const nextDraft = {
        ...RULE_SET_INITIAL_DRAFT,
        name: template.title.toLowerCase().replace(/[^a-z0-9]+/g, "_"),
        description: template.description,
        regoSource: template.regoSource,
        appliedFacets: template.appliedFacets ?? [],
      };
      setEditingRuleSetId(null);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [],
  );

  const requestCreateRuleEditor = useCallback(() => {
    if (!isEditorOpen || !isRuleDraftDirty) {
      openCreateRuleEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isEditorOpen, isRuleDraftDirty, openCreateRuleEditor]);

  const requestEditRuleSet = useCallback(
    (record: RuleSetRecord) => {
      if (!isEditorOpen || !isRuleDraftDirty) {
        openEditRuleEditor(record);
        return;
      }
      setPendingEditorAction({ type: "edit", record });
    },
    [isEditorOpen, isRuleDraftDirty, openEditRuleEditor],
  );

  const requestCopyRuleSet = useCallback(
    (record: RuleSetRecord) => {
      if (!isEditorOpen || !isRuleDraftDirty) {
        openCopyRuleEditor(record);
        return;
      }
      setPendingEditorAction({ type: "copy", record });
    },
    [isEditorOpen, isRuleDraftDirty, openCopyRuleEditor],
  );

  const requestCloseRuleEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isRuleDraftDirty) {
      closeRuleSetEditor();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [closeRuleSetEditor, isEditorOpen, isRuleDraftDirty]);

  const requestApplyTemplate = useCallback(
    (template: {
      title: string;
      description: string;
      regoSource: string;
      appliedFacets?: string[];
    }) => {
      if (!isEditorOpen || !isRuleDraftDirty) {
        openTemplateRuleEditor(template);
        return;
      }
      setPendingEditorAction({ type: "template", template });
    },
    [isEditorOpen, isRuleDraftDirty, openTemplateRuleEditor],
  );

  const refreshRuleSets = useCallback(async () => {
    try {
      const { data, error } = await client.query(ruleSetsQuery, {}).toPromise();
      if (error) throw error;
      setRuleSetRecords(data.ruleSets || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshRuleSets();
  }, [refreshRuleSets]);

  const submitRuleSet = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: ruleSetDraft.name.trim(),
      description: ruleSetDraft.description.trim(),
      regoSource: ruleSetDraft.regoSource,
      enabled: ruleSetDraft.enabled,
      priority: ruleSetDraft.priority,
      appliedFacets: ruleSetDraft.appliedFacets,
    };

    if (!payload.name || !payload.regoSource.trim()) {
      setGlobalStatus(t("settings.ruleValidationRequired"));
      return;
    }

    setMutatingRuleSetId(editingRuleSetId || "new");
    try {
      if (editingRuleSetId) {
        const { error } = await client
          .mutation(updateRuleSetMutation, {
            input: {
              id: editingRuleSetId,
              name: payload.name,
              description: payload.description,
              regoSource: payload.regoSource,
              priority: payload.priority,
              appliedFacets: payload.appliedFacets,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.ruleUpdated"));
      } else {
        const { error } = await client
          .mutation(createRuleSetMutation, {
            input: createRuleSetInput(payload),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.ruleCreated"));
      }
      closeRuleSetEditor();
      await refreshRuleSets();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingRuleSetId(null);
    }
  };

  const deleteRuleSet = async (record: RuleSetRecord) => {
    setPendingDeleteRuleSet(record);
  };

  const toggleRuleSetEnabled = useCallback(
    async (record: RuleSetRecord) => {
      // The French locale packs read contradictory score sets, so the backend
      // refuses to enable a second one; catch it here for a translated message.
      if (!record.enabled) {
        const conflict = conflictingFrenchPack(ruleSetRecords, record);
        if (conflict) {
          setGlobalStatus(
            t("settings.trashPackFrenchConflict", { name: conflict.name }),
          );
          return;
        }
      }
      setMutatingRuleSetId(record.id);
      try {
        const { error } = await client
          .mutation(toggleRuleSetMutation, {
            input: { id: record.id, enabled: !record.enabled },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("status.ruleToggled", {
            name: record.name,
            state: record.enabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await refreshRuleSets();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingRuleSetId(null);
      }
    },
    [client, refreshRuleSets, ruleSetRecords, setGlobalStatus, t],
  );

  // Managed packs reject authored-field edits, so the filter update sends only
  // the id and the normalized tag list.
  const saveManagedTagFilter = useCallback(
    async (record: RuleSetRecord, raw: string) => {
      setMutatingRuleSetId(record.id);
      try {
        const { error } = await client
          .mutation(updateRuleSetMutation, {
            input: { id: record.id, managedTagFilter: parseTagFilterInput(raw) },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.trashPackFilterSaved", { name: record.name }),
        );
        await refreshRuleSets();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingRuleSetId(null);
      }
    },
    [client, refreshRuleSets, setGlobalStatus, t],
  );

  const confirmDeleteRuleSet = async () => {
    if (!pendingDeleteRuleSet) return;
    const record = pendingDeleteRuleSet;
    setMutatingRuleSetId(record.id);
    try {
      const { error } = await client
        .mutation(deleteRuleSetMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.ruleDeleted", { name: record.name }));
      await refreshRuleSets();
      if (editingRuleSetId === record.id) {
        closeRuleSetEditor();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingRuleSetId(null);
      setPendingDeleteRuleSet(null);
    }
  };

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateRuleEditor();
    } else if (pendingEditorAction.type === "copy") {
      openCopyRuleEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "edit") {
      openEditRuleEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "template") {
      openTemplateRuleEditor(pendingEditorAction.template);
    } else {
      closeRuleSetEditor();
    }
    setPendingEditorAction(null);
  }, [
    closeRuleSetEditor,
    openCreateRuleEditor,
    openCopyRuleEditor,
    openEditRuleEditor,
    openTemplateRuleEditor,
    pendingEditorAction,
  ]);

  const validateDraft = async () => {
    if (!ruleSetDraft.regoSource.trim()) return;
    setValidating(true);
    setValidationResult(null);
    try {
      const { data, error } = await client
        .mutation(validateRuleSetMutation, {
          input: {
            regoSource: ruleSetDraft.regoSource,
            ruleSetId: editingRuleSetId || undefined,
          },
        })
        .toPromise();
      if (error) throw error;
      setValidationResult(data.validateRuleSet);
    } catch (error) {
      setValidationResult({
        valid: false,
        errors: [error instanceof Error ? error.message : "Validation failed"],
      });
    } finally {
      setValidating(false);
    }
  };

  return (
    <>
      <SettingsRulesSection
        isEditorOpen={isEditorOpen}
        editorMode={editingRuleSetId ? "edit" : "create"}
        editingRuleSetId={editingRuleSetId}
        ruleSetDraft={ruleSetDraft}
        setRuleSetDraft={setRuleSetDraft}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={requestCloseRuleEditor}
        startCreateRuleSet={requestCreateRuleEditor}
        ruleSetRecords={ruleSetRecords}
        copyRuleSet={requestCopyRuleSet}
        editRuleSet={requestEditRuleSet}
        toggleRuleSetEnabled={toggleRuleSetEnabled}
        saveManagedTagFilter={saveManagedTagFilter}
        deleteRuleSet={deleteRuleSet}
        validateDraft={validateDraft}
        validating={validating}
        validationResult={validationResult}
        applyTemplate={requestApplyTemplate}
      />
      <ConfirmDialog
        open={pendingDeleteRuleSet !== null}
        title={t("label.delete")}
        description={
          pendingDeleteRuleSet
            ? t("status.deletingRule", { name: pendingDeleteRuleSet.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingRuleSetId !== null}
        onConfirm={confirmDeleteRuleSet}
        onCancel={() => setPendingDeleteRuleSet(null)}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.ruleConfirmDiscardTitle")}
        description={t("settings.ruleConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.ruleCreateNew")
            : pendingEditorAction?.type === "copy"
              ? t("settings.ruleCopyAsCustom")
            : pendingEditorAction?.type === "template"
              ? t("settings.ruleApplyTemplate")
              : pendingEditorAction?.type === "edit"
                ? t("label.edit")
                : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingRuleSetId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
    </>
  );
}
