import { type FormEvent, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { RuleSetTestPanel } from "@/components/containers/settings/rule-set-test-panel";
import { SettingsRulesSection } from "@/components/views/settings/settings-rules-section";
import {
  TrackedRulePacksSection,
  type TrackedRulePackMember,
  type TrackedRulePackPreview,
  type TrackedRulePackRecord,
} from "@/components/views/settings/tracked-rule-packs-section";
import type { ArrCustomFormatDraft } from "@/components/views/settings/arr-custom-format-import-dialog";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { RuleSetRecord, RuleSetDraft, RuleValidationResult } from "@/lib/types/rule-sets";
import { copyRuleSetDraft, createRuleSetInput } from "@/lib/utils/rule-sets";
import { conflictingFrenchPack } from "@/lib/utils/trash-packs";
import { resolveRuleDetailRequest } from "@/lib/utils/rule-detail-request";
import { ruleSetQuery, ruleSetsQuery, trackedRulePacksQuery } from "@/lib/graphql/queries";
import {
  copyTrackedRulePackRuleMutation,
  createRuleSetMutation,
  deleteRuleSetMutation,
  installTrackedRulePackMutation,
  previewTrackedRulePackUpdateMutation,
  setTrackedRulePackSettingsMutation,
  toggleRuleSetMutation,
  uninstallTrackedRulePackMutation,
  updateTrackedRulePackMutation,
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
  | { type: "tracked-copy"; record: RuleSetRecord }
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
  | { type: "import"; draft: ArrCustomFormatDraft }
  | null;

export function SettingsRulesContainer({ canManageCatalogSettings, canManageSystemSettings }: {
  canManageCatalogSettings: boolean;
  canManageSystemSettings: boolean;
}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const canManageTrackedPacks =
    canManageCatalogSettings && canManageSystemSettings;
  const [ruleSetRecords, setRuleSetRecords] = useState<RuleSetRecord[]>([]);
  const [trackedRulePacks, setTrackedRulePacks] = useState<TrackedRulePackRecord[]>([]);
  const [trackedRulePacksLoaded, setTrackedRulePacksLoaded] = useState(false);
  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(null);
  const [mutatingRulePackId, setMutatingRulePackId] = useState<string | null>(null);
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [copyingTrackedRuleSetId, setCopyingTrackedRuleSetId] = useState<string | null>(null);
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
  const [translationDiagnostics, setTranslationDiagnostics] = useState<string[]>([]);
  const [focusImportedEditor, setFocusImportedEditor] = useState(false);
  const detailRequestRef = useRef(0);

  const closeRuleSetEditor = useCallback(() => {
    detailRequestRef.current += 1;
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    setCopyingTrackedRuleSetId(null);
    setRuleSetDraft(() => ({ ...RULE_SET_INITIAL_DRAFT }));
    setRuleSetDraftBaseline(() => ({ ...RULE_SET_INITIAL_DRAFT }));
    setValidationResult(null);
    setTranslationDiagnostics([]);
    setFocusImportedEditor(false);
  }, []);

  const isRuleDraftDirty =
    JSON.stringify(ruleSetDraft) !== JSON.stringify(ruleSetDraftBaseline);

  const editorStateRef = useRef({ isOpen: isEditorOpen, isDirty: isRuleDraftDirty });
  useLayoutEffect(() => {
    editorStateRef.current = { isOpen: isEditorOpen, isDirty: isRuleDraftDirty };
  }, [isEditorOpen, isRuleDraftDirty]);

  const openCreateRuleEditor = useCallback(() => {
    detailRequestRef.current += 1;
    const nextDraft = { ...RULE_SET_INITIAL_DRAFT };
    setEditingRuleSetId(null);
    setCopyingTrackedRuleSetId(null);
    setRuleSetDraft(nextDraft);
    setRuleSetDraftBaseline(nextDraft);
    setValidationResult(null);
    setTranslationDiagnostics([]);
    setFocusImportedEditor(false);
    setIsEditorOpen(true);
  }, []);

  const openEditRuleEditor = useCallback(
    (record: RuleSetRecord) => {
      detailRequestRef.current += 1;
      const nextDraft = {
        name: record.name,
        description: record.description,
        regoSource: record.regoSource,
        enabled: record.enabled,
        priority: record.priority,
        appliedFacets: [...record.appliedFacets],
      };
      setEditingRuleSetId(record.id);
      setCopyingTrackedRuleSetId(null);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setTranslationDiagnostics([]);
      setFocusImportedEditor(false);
      setIsEditorOpen(true);
      setGlobalStatus(t("status.editingRule", { name: record.name }));
    },
    [setGlobalStatus, t],
  );

  const openCopyRuleEditor = useCallback(
    (record: RuleSetRecord) => {
      detailRequestRef.current += 1;
      const nextDraft = copyRuleSetDraft(record);
      setEditingRuleSetId(null);
      setCopyingTrackedRuleSetId(null);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setTranslationDiagnostics([]);
      setFocusImportedEditor(false);
      setIsEditorOpen(true);
    },
    [],
  );

  const openTrackedRuleCopyEditor = useCallback((record: RuleSetRecord) => {
    detailRequestRef.current += 1;
    const nextDraft = copyRuleSetDraft(record);
    setEditingRuleSetId(null);
    setCopyingTrackedRuleSetId(record.id);
    setRuleSetDraft(nextDraft);
    setRuleSetDraftBaseline(nextDraft);
    setValidationResult(null);
    setTranslationDiagnostics([]);
    setFocusImportedEditor(false);
    setIsEditorOpen(true);
  }, []);

  const openTemplateRuleEditor = useCallback(
    (template: {
      title: string;
      description: string;
      regoSource: string;
      appliedFacets?: string[];
    }) => {
      detailRequestRef.current += 1;
      const nextDraft = {
        ...RULE_SET_INITIAL_DRAFT,
        name: template.title.toLowerCase().replace(/[^a-z0-9]+/g, "_"),
        description: template.description,
        regoSource: template.regoSource,
        appliedFacets: template.appliedFacets ?? [],
      };
      setEditingRuleSetId(null);
      setCopyingTrackedRuleSetId(null);
      setRuleSetDraft(nextDraft);
      setRuleSetDraftBaseline(nextDraft);
      setValidationResult(null);
      setTranslationDiagnostics([]);
      setFocusImportedEditor(false);
      setIsEditorOpen(true);
    },
    [],
  );

  const openImportedRuleEditor = useCallback((imported: ArrCustomFormatDraft) => {
    detailRequestRef.current += 1;
    const nextDraft = {
      ...RULE_SET_INITIAL_DRAFT,
      name: imported.name.trim() || t("settings.arrImportDraftName"),
      description: imported.description,
      regoSource: imported.regoSource,
      appliedFacets: imported.appliedFacets,
    };
    setEditingRuleSetId(null);
    setCopyingTrackedRuleSetId(null);
    setRuleSetDraft(nextDraft);
    setRuleSetDraftBaseline(nextDraft);
    setValidationResult(null);
    setTranslationDiagnostics(imported.translationDiagnostics);
    setFocusImportedEditor(true);
    setIsEditorOpen(true);
  }, [t]);

  const loadRuleSetDetail = useCallback(async (id: string, request: number): Promise<RuleSetRecord | null> => {
    try {
      const { data, error } = await client.query(ruleSetQuery, { id }).toPromise();
      if (error) throw error;
      return data.ruleSet as RuleSetRecord | null;
    } catch (error) {
      if (request === detailRequestRef.current) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      }
      return null;
    }
  }, [client, setGlobalStatus, t]);

  const requestCreateRuleEditor = useCallback(() => {
    if (!isEditorOpen || !isRuleDraftDirty) {
      openCreateRuleEditor();
      return;
    }
    detailRequestRef.current += 1;
    setPendingEditorAction({ type: "create" });
  }, [isEditorOpen, isRuleDraftDirty, openCreateRuleEditor]);

  const requestEditRuleSet = useCallback(
    async (record: RuleSetRecord) => {
      const request = ++detailRequestRef.current;
      const result = await resolveRuleDetailRequest(request, () => detailRequestRef.current, () => loadRuleSetDetail(record.id, request), () => editorStateRef.current);
      if (result.type === "ignore") return;
      if (result.type === "open") {
        openEditRuleEditor(result.detail);
        return;
      }
      setPendingEditorAction({ type: "edit", record: result.detail });
    },
    [loadRuleSetDetail, openEditRuleEditor],
  );

  const requestCopyRuleSet = useCallback(
    async (record: RuleSetRecord) => {
      const request = ++detailRequestRef.current;
      const result = await resolveRuleDetailRequest(request, () => detailRequestRef.current, () => loadRuleSetDetail(record.id, request), () => editorStateRef.current);
      if (result.type === "ignore") return;
      if (result.type === "open") {
        openCopyRuleEditor(result.detail);
        return;
      }
      setPendingEditorAction({ type: "copy", record: result.detail });
    },
    [loadRuleSetDetail, openCopyRuleEditor],
  );

  const requestTrackedRuleCopy = useCallback(async (member: TrackedRulePackMember) => {
    if (!member.ruleSetId) return;
    const request = ++detailRequestRef.current;
    const result = await resolveRuleDetailRequest(request, () => detailRequestRef.current, () => loadRuleSetDetail(member.ruleSetId, request), () => editorStateRef.current);
    if (result.type === "ignore") return;
    if (result.type === "open") {
      openTrackedRuleCopyEditor(result.detail);
    } else {
      setPendingEditorAction({ type: "tracked-copy", record: result.detail });
    }
  }, [loadRuleSetDetail, openTrackedRuleCopyEditor]);

  const requestCloseRuleEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isRuleDraftDirty) {
      closeRuleSetEditor();
      return;
    }
    detailRequestRef.current += 1;
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
      detailRequestRef.current += 1;
      setPendingEditorAction({ type: "template", template });
    },
    [isEditorOpen, isRuleDraftDirty, openTemplateRuleEditor],
  );

  const requestImportCustomFormats = useCallback(
    (draft: ArrCustomFormatDraft) => {
      if (!isEditorOpen || !isRuleDraftDirty) {
        openImportedRuleEditor(draft);
        return;
      }
      detailRequestRef.current += 1;
      setPendingEditorAction({ type: "import", draft });
    },
    [isEditorOpen, isRuleDraftDirty, openImportedRuleEditor],
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

  const refreshTrackedRulePacks = useCallback(async () => {
    try {
      const { data, error } = await client.query(trackedRulePacksQuery, {}).toPromise();
      if (error) throw error;
      setTrackedRulePacks(data.trackedRulePacks || []);
      setTrackedRulePacksLoaded(true);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshRuleSets();
    void refreshTrackedRulePacks();
  }, [refreshRuleSets, refreshTrackedRulePacks]);

  const updateTrackedRulePackSettings = useCallback(
    async (
      pack: TrackedRulePackRecord,
      changes: Partial<Pick<TrackedRulePackRecord, "autoUpdate" | "members">>,
      ruleSetId?: string | null,
    ) => {
      const members = changes.members ?? pack.members;
      setMutatingRulePackId(pack.packId);
      if (ruleSetId) setMutatingRuleSetId(ruleSetId);
      try {
        const { error } = await client
          .mutation(setTrackedRulePackSettingsMutation, {
            packId: pack.packId,
            enabledTemplateIds: members
              .filter((member) => !member.removed && member.enabled)
              .map((member) => member.templateId),
            priorities: members
              .flatMap((member) => (
                !member.removed && member.priority !== null
                  ? [{ templateId: member.templateId, priority: member.priority }]
                  : []
              )),
            autoUpdate: changes.autoUpdate ?? pack.autoUpdate,
            expectedRevision: pack.revision,
          })
          .toPromise();
        if (error) throw error;
        await Promise.all([refreshRuleSets(), refreshTrackedRulePacks()]);
        return true;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
        return false;
      } finally {
        setMutatingRulePackId(null);
        if (ruleSetId) setMutatingRuleSetId(null);
      }
    },
    [client, refreshRuleSets, refreshTrackedRulePacks, setGlobalStatus, t],
  );

  const installTrackedRulePack = useCallback(
    async (packId: string, templateIds: string[]) => {
      setMutatingRulePackId(packId);
      try {
        const { error } = await client
          .mutation(installTrackedRulePackMutation, { packId, templateIds })
          .toPromise();
        if (error) throw error;
        await Promise.all([refreshRuleSets(), refreshTrackedRulePacks()]);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingRulePackId(null);
      }
    },
    [client, refreshRuleSets, refreshTrackedRulePacks, setGlobalStatus, t],
  );

  const previewTrackedRulePackUpdate = useCallback(
    async (pack: TrackedRulePackRecord): Promise<TrackedRulePackPreview | null> => {
      setMutatingRulePackId(pack.packId);
      try {
        const { data, error } = await client
          .mutation(previewTrackedRulePackUpdateMutation, { packId: pack.packId })
          .toPromise();
        if (error) throw error;
        return data.previewTrackedRulePackUpdate;
      } catch (error) {
        const message = error instanceof Error ? error.message : "";
        setGlobalStatus(
          /same version|no update|already (?:up to date|latest)/i.test(message)
            ? "This rule pack is already up to date."
            : message || t("status.failedToUpdate"),
        );
        return null;
      } finally {
        setMutatingRulePackId(null);
      }
    },
    [client, setGlobalStatus, t],
  );

  const applyTrackedRulePackUpdate = useCallback(
    async (pack: TrackedRulePackRecord, preview: TrackedRulePackPreview) => {
      setMutatingRulePackId(pack.packId);
      try {
        const { error } = await client
          .mutation(updateTrackedRulePackMutation, {
            packId: pack.packId,
            version: preview.version,
            digest: preview.digest,
            revision: preview.revision,
          })
          .toPromise();
        if (error) throw error;
        await Promise.all([refreshRuleSets(), refreshTrackedRulePacks()]);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingRulePackId(null);
      }
    },
    [client, refreshRuleSets, refreshTrackedRulePacks, setGlobalStatus, t],
  );

  const uninstallTrackedRulePack = useCallback(
    async (pack: TrackedRulePackRecord) => {
      setMutatingRulePackId(pack.packId);
      try {
        const { error } = await client
          .mutation(uninstallTrackedRulePackMutation, {
            packId: pack.packId,
            revision: pack.revision,
          })
          .toPromise();
        if (error) throw error;
        await Promise.all([refreshRuleSets(), refreshTrackedRulePacks()]);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingRulePackId(null);
      }
    },
    [client, refreshRuleSets, refreshTrackedRulePacks, setGlobalStatus, t],
  );

  const toggleTrackedRulePackMember = useCallback(
    async (pack: TrackedRulePackRecord, member: TrackedRulePackMember) => {
      if (member.removed || !member.ruleSetId) return false;
      return updateTrackedRulePackSettings(
        pack,
        {
          members: pack.members.map((current) => current.templateId === member.templateId
            ? { ...current, enabled: !current.enabled }
            : current),
        },
        member.ruleSetId,
      );
    },
    [updateTrackedRulePackSettings],
  );

  const updateTrackedRulePackMemberPriority = useCallback(
    async (pack: TrackedRulePackRecord, member: TrackedRulePackMember, priority: number) => {
      if (member.removed || !member.ruleSetId) return false;
      return updateTrackedRulePackSettings(
        pack,
        {
          members: pack.members.map((current) => current.templateId === member.templateId
            ? { ...current, priority }
            : current),
        },
        member.ruleSetId,
      );
    },
    [updateTrackedRulePackSettings],
  );

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
    } else if (pendingEditorAction.type === "tracked-copy") {
      openTrackedRuleCopyEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "edit") {
      openEditRuleEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "template") {
      openTemplateRuleEditor(pendingEditorAction.template);
    } else if (pendingEditorAction.type === "import") {
      openImportedRuleEditor(pendingEditorAction.draft);
    } else {
      closeRuleSetEditor();
    }
    setPendingEditorAction(null);
  }, [
    closeRuleSetEditor,
    openCreateRuleEditor,
    openCopyRuleEditor,
    openEditRuleEditor,
    openImportedRuleEditor,
    openTrackedRuleCopyEditor,
    openTemplateRuleEditor,
    pendingEditorAction,
  ]);

  const validateDraft = useCallback(async (): Promise<RuleValidationResult | null> => {
    if (!ruleSetDraft.regoSource.trim()) return null;
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
      const result = data.validateRuleSet;
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
  }, [client, editingRuleSetId, ruleSetDraft.regoSource]);

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
      setValidationResult({
        valid: false,
        errors: [t("settings.ruleValidationRequired")],
      });
      return;
    }

    const validation = await validateDraft();
    if (!validation?.valid) {
      return;
    }

    setMutatingRuleSetId(editingRuleSetId || copyingTrackedRuleSetId || "new");
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
        if (payload.enabled !== ruleSetDraftBaseline.enabled) {
          const { error: toggleError } = await client
            .mutation(toggleRuleSetMutation, {
              input: { id: editingRuleSetId, enabled: payload.enabled },
            })
            .toPromise();
          if (toggleError) throw toggleError;
        }
        setGlobalStatus(t("status.ruleUpdated"));
      } else if (copyingTrackedRuleSetId) {
        const { error } = await client
          .mutation(copyTrackedRulePackRuleMutation, {
            ruleSetId: copyingTrackedRuleSetId,
            name: payload.name,
            description: payload.description,
            regoSource: payload.regoSource,
            appliedFacets: payload.appliedFacets,
            priority: payload.priority,
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.ruleCreated"));
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
      await Promise.all([refreshRuleSets(), refreshTrackedRulePacks()]);
    } catch (error) {
      const message = error instanceof Error ? error.message : null;
      const validationErrorIndex = message?.indexOf("Rule validation failed:") ?? -1;
      if (validationErrorIndex >= 0 && message) {
        setValidationResult({
          valid: false,
          errors: [
            message
              .slice(validationErrorIndex + "Rule validation failed:".length)
              .replace(/^\s*-?\s*/, ""),
          ],
        });
        return;
      }
      setGlobalStatus(message || t("status.failedToUpdate"));
    } finally {
      setMutatingRuleSetId(null);
    }
  };

  const trackedRuleSetIds = new Set(
    trackedRulePacks.flatMap((pack) => pack.members.map((member) => member.ruleSetId).filter((id): id is string => Boolean(id))),
  );
  const editableRuleSetRecords = (trackedRulePacksLoaded ? ruleSetRecords : []).filter(
    (record) => !trackedRuleSetIds.has(record.id),
  );

  return (
    <>
      <SettingsRulesSection
        canManageTrackedPacks={canManageTrackedPacks}
        isEditorOpen={isEditorOpen}
        editorMode={copyingTrackedRuleSetId ? "copy" : editingRuleSetId ? "edit" : "create"}
        editingRuleSetId={editingRuleSetId}
        ruleSetDraft={ruleSetDraft}
        setRuleSetDraft={setRuleSetDraft}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={requestCloseRuleEditor}
        startCreateRuleSet={requestCreateRuleEditor}
        ruleSetRecords={editableRuleSetRecords}
        copyRuleSet={requestCopyRuleSet}
        editRuleSet={requestEditRuleSet}
        toggleRuleSetEnabled={toggleRuleSetEnabled}
        deleteRuleSet={deleteRuleSet}
        validateDraft={validateDraft}
        validating={validating}
        validationResult={validationResult}
        applyTemplate={requestApplyTemplate}
        importCustomFormats={requestImportCustomFormats}
        translationDiagnostics={translationDiagnostics}
        focusEditor={focusImportedEditor}
        onEditorFocused={() => setFocusImportedEditor(false)}
        testScoring={
          <RuleSetTestPanel
            draft={ruleSetDraft}
            editRuleSetId={editingRuleSetId}
            copySourceRuleSetId={copyingTrackedRuleSetId}
          />
        }
        trackedRulePacks={
          <TrackedRulePacksSection
            packs={trackedRulePacks}
            canManage={canManageTrackedPacks}
            mutatingPackId={mutatingRulePackId}
            mutatingRuleSetId={mutatingRuleSetId}
            onPreviewUpdate={previewTrackedRulePackUpdate}
            onApplyUpdate={applyTrackedRulePackUpdate}
            onSetAutoUpdate={(pack, enabled) => updateTrackedRulePackSettings(pack, { autoUpdate: enabled })}
            onUninstall={uninstallTrackedRulePack}
            onToggleMember={toggleTrackedRulePackMember}
            onUpdateMemberPriority={updateTrackedRulePackMemberPriority}
            onCopyMember={(member) => void requestTrackedRuleCopy(member)}
          />
        }
        installTrackedRulePack={installTrackedRulePack}
        installingRulePackId={mutatingRulePackId}
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
              : pendingEditorAction?.type === "import"
                ? t("settings.arrImportApply")
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
