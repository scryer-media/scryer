import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsMaintenanceRulesSection,
  type MaintenanceLibraryOption,
  type MaintenanceQualityProfileOption,
} from "@/components/views/settings/settings-maintenance-rules-section";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionDescriptor,
  MaintenancePreviewResult,
  MaintenancePreviewSource,
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceValidationResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  initialMaintenanceRuleDraft,
  maintenancePreviewInput,
  maintenanceRuleDraftFromDetail,
  updateMaintenanceRuleMatcherInput,
  updateMaintenanceRuleMetadataInput,
} from "@/lib/utils/maintenance-rule-sets";
import {
  librariesQuery,
  maintenanceActionDescriptorsQuery,
  maintenanceRuleSetQuery,
  maintenanceRuleSetsQuery,
  qualityProfileOptionsQuery,
} from "@/lib/graphql/queries";
import {
  createMaintenanceRuleSetMutation,
  deleteMaintenanceRuleSetMutation,
  previewMaintenanceRuleMutation,
  updateMaintenanceRuleMatcherMutation,
  updateMaintenanceRuleMetadataMutation,
  validateMaintenanceRuleMutation,
} from "@/lib/graphql/mutations";

type PendingEditorAction =
  | { type: "create" }
  | { type: "copy"; record: MaintenanceRuleSetRecord }
  | { type: "edit"; record: MaintenanceRuleSetRecord }
  | { type: "close" }
  | null;

export function SettingsMaintenanceRulesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  const [ruleSetRecords, setRuleSetRecords] = useState<MaintenanceRuleSetRecord[]>([]);
  const [actionDescriptors, setActionDescriptors] = useState<
    MaintenanceActionDescriptor[]
  >([]);
  const [libraries, setLibraries] = useState<MaintenanceLibraryOption[]>([]);
  const [qualityProfiles, setQualityProfiles] = useState<
    MaintenanceQualityProfileOption[]
  >([]);

  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(null);
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteRuleSet, setPendingDeleteRuleSet] =
    useState<MaintenanceRuleSetRecord | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingEditorAction>(null);
  const [ruleSetDraft, setRuleSetDraft] = useState<MaintenanceRuleSetDraft>(
    initialMaintenanceRuleDraft,
  );
  const [ruleSetDraftBaseline, setRuleSetDraftBaseline] =
    useState<MaintenanceRuleSetDraft>(initialMaintenanceRuleDraft);
  const [validating, setValidating] = useState(false);
  const [validationResult, setValidationResult] =
    useState<MaintenanceValidationResult | null>(null);

  const [previewSource, setPreviewSource] = useState<MaintenancePreviewSource>("stored");
  const [previewRuleSetId, setPreviewRuleSetId] = useState("");
  const [previewLibraryId, setPreviewLibraryId] = useState("");
  const [previewLimit, setPreviewLimit] = useState(MAINTENANCE_PREVIEW_LIMIT_DEFAULT);
  const [previewing, setPreviewing] = useState(false);
  const [previewResult, setPreviewResult] = useState<MaintenancePreviewResult | null>(
    null,
  );
  const [previewError, setPreviewError] = useState<string | null>(null);

  const isDraftDirty =
    JSON.stringify(ruleSetDraft) !== JSON.stringify(ruleSetDraftBaseline);

  const closeEditor = useCallback(() => {
    const next = initialMaintenanceRuleDraft();
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    setRuleSetDraft(next);
    setRuleSetDraftBaseline(next);
    setValidationResult(null);
  }, []);

  const openCreateEditor = useCallback(() => {
    const next = initialMaintenanceRuleDraft();
    setEditingRuleSetId(null);
    setRuleSetDraft(next);
    setRuleSetDraftBaseline(next);
    setValidationResult(null);
    setIsEditorOpen(true);
  }, []);

  /// Load the matcher source and action for one rule set, surfacing failure
  /// instead of silently refusing to open the editor.
  const fetchRuleSetDetail = useCallback(
    async (id: string): Promise<MaintenanceRuleSetDetail | null> => {
      try {
        const { data, error } = await client
          .query(maintenanceRuleSetQuery, { id })
          .toPromise();
        if (error) throw error;
        const detail =
          (data?.maintenanceRuleSet as MaintenanceRuleSetDetail | null) ?? null;
        if (!detail) {
          setGlobalStatus(t("status.failedToLoad"));
        }
        return detail;
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
        return null;
      }
    },
    [client, setGlobalStatus, t],
  );

  const openEditEditor = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      const detail = await fetchRuleSetDetail(record.id);
      if (!detail) return;
      const next = maintenanceRuleDraftFromDetail(detail);
      setEditingRuleSetId(record.id);
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(next);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [fetchRuleSetDetail],
  );

  const openCopyEditor = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      const detail = await fetchRuleSetDetail(record.id);
      if (!detail) return;
      const next = copyMaintenanceRuleDraft(detail);
      setEditingRuleSetId(null);
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(next);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [fetchRuleSetDetail],
  );

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditRuleSet = useCallback(
    (record: MaintenanceRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(record);
        return;
      }
      setPendingEditorAction({ type: "edit", record });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCopyRuleSet = useCallback(
    (record: MaintenanceRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        openCopyEditor(record);
        return;
      }
      setPendingEditorAction({ type: "copy", record });
    },
    [isDraftDirty, isEditorOpen, openCopyEditor],
  );

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      closeEditor();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [closeEditor, isDraftDirty, isEditorOpen]);

  /// The list payload carries the action and grace period of each rule's
  /// current revision, so the table renders from one query; the full detail
  /// (matcher source) is fetched only when an editor opens.
  const refreshRuleSets = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(maintenanceRuleSetsQuery, {})
        .toPromise();
      if (error) throw error;
      setRuleSetRecords(data?.maintenanceRuleSets ?? []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);


  const refreshCatalogs = useCallback(async () => {
    try {
      const [descriptors, libraryList, profiles] = await Promise.all([
        client.query(maintenanceActionDescriptorsQuery, {}).toPromise(),
        client.query(librariesQuery, {}).toPromise(),
        client.query(qualityProfileOptionsQuery, {}).toPromise(),
      ]);
      if (!descriptors.error) {
        setActionDescriptors(descriptors.data?.maintenanceActionDescriptors ?? []);
      }
      if (!libraryList.error) {
        setLibraries(
          (libraryList.data?.libraries ?? []).map(
            (library: { id: string; name: string }) => ({
              id: library.id,
              name: library.name,
            }),
          ),
        );
      }
      if (!profiles.error) {
        setQualityProfiles(
          profiles.data?.qualityProfileSettings?.profiles ?? [],
        );
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshRuleSets();
  }, [refreshRuleSets]);

  useEffect(() => {
    void refreshCatalogs();
  }, [refreshCatalogs]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "copy") {
      openCopyEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.record);
    } else {
      closeEditor();
    }
    setPendingEditorAction(null);
  }, [
    closeEditor,
    openCopyEditor,
    openCreateEditor,
    openEditEditor,
    pendingEditorAction,
  ]);

  const validateDraft = useCallback(async (): Promise<MaintenanceValidationResult | null> => {
    if (!ruleSetDraft.regoSource.trim()) return null;
    setValidating(true);
    setValidationResult(null);
    try {
      const { data, error } = await client
        .mutation(validateMaintenanceRuleMutation, {
          input: { regoSource: ruleSetDraft.regoSource },
        })
        .toPromise();
      if (error) throw error;
      const result = data.validateMaintenanceRule as MaintenanceValidationResult;
      setValidationResult(result);
      return result;
    } catch (error) {
      const result = {
        valid: false,
        errors: [error instanceof Error ? error.message : "Validation failed"],
      };
      setValidationResult(result);
      return result;
    } finally {
      setValidating(false);
    }
  }, [client, ruleSetDraft.regoSource]);

  const deleteRuleSet = async (record: MaintenanceRuleSetRecord) => {
    setPendingDeleteRuleSet(record);
  };

  const confirmDeleteRuleSet = async () => {
    if (!pendingDeleteRuleSet) return;
    const record = pendingDeleteRuleSet;
    setMutatingRuleSetId(record.id);
    try {
      const { error } = await client
        .mutation(deleteMaintenanceRuleSetMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.maintenanceRuleDeleted", { name: record.name }));
      if (editingRuleSetId === record.id) {
        closeEditor();
      }
      if (previewRuleSetId === record.id) {
        setPreviewRuleSetId("");
        setPreviewResult(null);
      }
      await refreshRuleSets();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingRuleSetId(null);
      setPendingDeleteRuleSet(null);
    }
  };

  /// Editing a saved rule writes both halves: the matcher (a new immutable
  /// revision) and the metadata (name, description, library scope), which the
  /// API keeps on separate mutations because only one of them versions.
  const submitRuleSet = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!ruleSetDraft.name.trim() || !ruleSetDraft.regoSource.trim()) {
      setValidationResult({
        valid: false,
        errors: [t("settings.maintenanceRuleValidationRequired")],
      });
      return;
    }

    const validation = await validateDraft();
    if (!validation?.valid) {
      return;
    }

    setMutatingRuleSetId(editingRuleSetId || "new");
    try {
      if (editingRuleSetId) {
        const matcher = await client
          .mutation(updateMaintenanceRuleMatcherMutation, {
            input: updateMaintenanceRuleMatcherInput(
              editingRuleSetId,
              ruleSetDraft,
              actionDescriptors,
            ),
          })
          .toPromise();
        if (matcher.error) throw matcher.error;
        const metadata = await client
          .mutation(updateMaintenanceRuleMetadataMutation, {
            input: updateMaintenanceRuleMetadataInput(editingRuleSetId, ruleSetDraft),
          })
          .toPromise();
        if (metadata.error) throw metadata.error;
        setGlobalStatus(t("settings.maintenanceRuleUpdated"));
      } else {
        const { error } = await client
          .mutation(createMaintenanceRuleSetMutation, {
            input: createMaintenanceRuleSetInput(ruleSetDraft, actionDescriptors),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.maintenanceRuleCreated"));
      }
      closeEditor();
      await refreshRuleSets();
    } catch (error) {
      const message = error instanceof Error ? error.message : null;
      setValidationResult({
        valid: false,
        errors: [message || t("status.failedToUpdate")],
      });
    } finally {
      setMutatingRuleSetId(null);
    }
  };

  const runPreview = useCallback(async () => {
    setPreviewing(true);
    setPreviewError(null);
    try {
      const { data, error } = await client
        .mutation(previewMaintenanceRuleMutation, {
          input: maintenancePreviewInput({
            ruleSetId: previewSource === "stored" ? previewRuleSetId : null,
            draft: previewSource === "draft" ? ruleSetDraft : undefined,
            descriptors: actionDescriptors,
            libraryId: previewLibraryId,
            limit: previewLimit,
          }),
        })
        .toPromise();
      if (error) throw error;
      setPreviewResult(data.previewMaintenanceRule as MaintenancePreviewResult);
    } catch (error) {
      setPreviewResult(null);
      setPreviewError(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setPreviewing(false);
    }
  }, [
    actionDescriptors,
    client,
    previewLibraryId,
    previewLimit,
    previewRuleSetId,
    previewSource,
    ruleSetDraft,
    t,
  ]);

  return (
    <>
      <SettingsMaintenanceRulesSection
        isEditorOpen={isEditorOpen}
        editingRuleSetId={editingRuleSetId}
        ruleSetDraft={ruleSetDraft}
        setRuleSetDraft={setRuleSetDraft}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={requestCloseEditor}
        startCreateRuleSet={requestCreateEditor}
        ruleSetRecords={ruleSetRecords}
        actionDescriptors={actionDescriptors}
        libraries={libraries}
        qualityProfiles={qualityProfiles}
        copyRuleSet={requestCopyRuleSet}
        editRuleSet={requestEditRuleSet}
        deleteRuleSet={deleteRuleSet}
        validateDraft={validateDraft}
        validating={validating}
        validationResult={validationResult}
        previewSource={previewSource}
        setPreviewSource={setPreviewSource}
        previewRuleSetId={previewRuleSetId}
        setPreviewRuleSetId={setPreviewRuleSetId}
        previewLibraryId={previewLibraryId}
        setPreviewLibraryId={setPreviewLibraryId}
        previewLimit={previewLimit}
        setPreviewLimit={(limit) => setPreviewLimit(clampMaintenancePreviewLimit(limit))}
        runPreview={runPreview}
        previewing={previewing}
        previewResult={previewResult}
        previewError={previewError}
      />
      <ConfirmDialog
        open={pendingDeleteRuleSet !== null}
        title={t("label.delete")}
        description={
          pendingDeleteRuleSet
            ? t("settings.maintenanceRuleDeleting", {
                name: pendingDeleteRuleSet.name,
              })
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
        title={t("settings.maintenanceRuleConfirmDiscardTitle")}
        description={t("settings.maintenanceRuleConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create" ||
          pendingEditorAction?.type === "copy"
            ? t("settings.maintenanceRuleCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.yes")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingRuleSetId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
    </>
  );
}
