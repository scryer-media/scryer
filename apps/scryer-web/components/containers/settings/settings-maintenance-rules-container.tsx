import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  MaintenanceCandidatesPanel,
  MaintenanceExclusionsPanel,
  MaintenanceGatesPanel,
  MaintenanceRunsPanel,
} from "@/components/views/settings/settings-maintenance-operations-panel";
import {
  SettingsMaintenanceRulesSection,
  type MaintenanceLibraryOption,
  type MaintenanceQualityProfileOption,
} from "@/components/views/settings/settings-maintenance-rules-section";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionDescriptor,
  MaintenanceActionRun,
  MaintenanceCandidate,
  MaintenanceEffectArming,
  MaintenanceEvaluationMode,
  MaintenanceEvaluationRun,
  MaintenanceExclusion,
  MaintenanceGateKey,
  MaintenanceInstanceGates,
  MaintenancePreviewResult,
  MaintenancePreviewSource,
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceTriggerResult,
  MaintenanceValidationResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_FILTER_ALL,
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  excludeMaintenanceSubjectInput,
  initialMaintenanceRuleDraft,
  isNonTerminalCandidateState,
  maintenanceFilterArgument,
  maintenancePreviewInput,
  maintenanceRuleDraftFromDetail,
  parseAcknowledgedCandidateCountMismatch,
  setMaintenanceRuleArmingInput,
  updateMaintenanceRuleMatcherInput,
  updateMaintenanceRuleMetadataInput,
} from "@/lib/utils/maintenance-rule-sets";
import {
  librariesQuery,
  maintenanceActionDescriptorsQuery,
  maintenanceActionRunsQuery,
  maintenanceCandidatesQuery,
  maintenanceEvaluationRunsQuery,
  maintenanceExclusionsQuery,
  maintenanceInstanceGatesQuery,
  maintenanceRuleSetQuery,
  maintenanceRuleSetsQuery,
  qualityProfileOptionsQuery,
} from "@/lib/graphql/queries";
import {
  createMaintenanceRuleSetMutation,
  deleteMaintenanceRuleSetMutation,
  excludeMaintenanceSubjectMutation,
  previewMaintenanceRuleMutation,
  removeMaintenanceExclusionMutation,
  runMaintenanceActionHandlerNowMutation,
  runMaintenanceEvaluationNowMutation,
  setMaintenanceInstanceGatesMutation,
  setMaintenanceRuleArmingMutation,
  setMaintenanceRuleModeMutation,
  updateMaintenanceRuleMatcherMutation,
  updateMaintenanceRuleMetadataMutation,
  validateMaintenanceRuleMutation,
} from "@/lib/graphql/mutations";

/// How many recent runs each history table shows. The API clamps its own
/// maximum; this is only how much of it the panel asks for.
const MAINTENANCE_RUN_HISTORY_LIMIT = 25;
const MAINTENANCE_CANDIDATE_LIMIT = 200;
/// Titles named in the destructive-arming dialog before it falls back to a
/// count. Enough to recognize what is about to be armed, short enough to read.
const DESTRUCTIVE_ARMING_PREVIEW_TITLES = 5;

/// A destructive arming in flight. The count is the number the dialog actually
/// showed, and the same number it sends as the acknowledgement, so the server's
/// check compares against what the operator saw rather than a fresher count
/// they never read.
type PendingArming = {
  record: MaintenanceRuleSetRecord;
  candidateCount: number;
  sampleTitles: string[];
  acknowledged: boolean;
  loading: boolean;
  /// Set after the server refused the acknowledgement and the dialog re-asked
  /// against the count the server reported.
  countChanged: boolean;
};

type PendingExclusion = {
  titleId: string;
  titleName: string;
  ruleSetId: string;
  reason: string;
};

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

  const [gates, setGates] = useState<MaintenanceInstanceGates | null>(null);
  /// True once the gates query has been refused. Reading the gates needs
  /// system-settings management, and a reader without it must still get a
  /// working page rather than a failed section.
  const [gatesLocked, setGatesLocked] = useState(false);
  const [savingGate, setSavingGate] = useState<MaintenanceGateKey | null>(null);
  const [pendingDestructiveGate, setPendingDestructiveGate] = useState(false);

  const [candidates, setCandidates] = useState<MaintenanceCandidate[]>([]);
  const [candidatesLoading, setCandidatesLoading] = useState(false);
  const [candidatesError, setCandidatesError] = useState<string | null>(null);
  const [candidateRuleFilter, setCandidateRuleFilter] =
    useState(MAINTENANCE_FILTER_ALL);
  const [candidateStateFilter, setCandidateStateFilter] =
    useState(MAINTENANCE_FILTER_ALL);
  const [candidateLibraryFilter, setCandidateLibraryFilter] =
    useState(MAINTENANCE_FILTER_ALL);

  const [evaluationRuns, setEvaluationRuns] = useState<MaintenanceEvaluationRun[]>([]);
  const [actionRuns, setActionRuns] = useState<MaintenanceActionRun[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [runScopeRuleSetId, setRunScopeRuleSetId] = useState(MAINTENANCE_FILTER_ALL);
  const [evaluationTriggering, setEvaluationTriggering] = useState(false);
  const [actionTriggering, setActionTriggering] = useState(false);

  const [exclusions, setExclusions] = useState<MaintenanceExclusion[]>([]);
  const [exclusionsError, setExclusionsError] = useState<string | null>(null);
  const [removingExclusionId, setRemovingExclusionId] = useState<string | null>(null);
  const [pendingRemoveExclusion, setPendingRemoveExclusion] =
    useState<MaintenanceExclusion | null>(null);
  const [pendingExclusion, setPendingExclusion] = useState<PendingExclusion | null>(
    null,
  );
  const [excluding, setExcluding] = useState(false);

  const [pendingArming, setPendingArming] = useState<PendingArming | null>(null);

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

  /// The gates query is the one call on this page that can be refused outright.
  /// A refusal is a permission answer, not a page failure: it locks the panel
  /// and leaves the rest of the section working.
  const refreshGates = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(maintenanceInstanceGatesQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      const next = (data?.maintenanceInstanceGates as MaintenanceInstanceGates) ?? null;
      setGates(next);
      setGatesLocked(next === null);
    } catch {
      setGates(null);
      setGatesLocked(true);
    }
  }, [client]);

  /// `includeShadow` is always true here: with the result-display gate closed
  /// the server otherwise returns nothing, and an operator deciding whether to
  /// open that gate has to see what shadow evaluation found first.
  const refreshCandidates = useCallback(async () => {
    setCandidatesLoading(true);
    setCandidatesError(null);
    try {
      const { data, error } = await client
        .query(
          maintenanceCandidatesQuery,
          {
            ruleSetId: maintenanceFilterArgument(candidateRuleFilter),
            states: maintenanceFilterArgument(candidateStateFilter)
              ? [candidateStateFilter]
              : undefined,
            libraryId: maintenanceFilterArgument(candidateLibraryFilter),
            includeShadow: true,
            limit: MAINTENANCE_CANDIDATE_LIMIT,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setCandidates((data?.maintenanceCandidates as MaintenanceCandidate[]) ?? []);
    } catch (error) {
      setCandidates([]);
      setCandidatesError(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setCandidatesLoading(false);
    }
  }, [candidateLibraryFilter, candidateRuleFilter, candidateStateFilter, client, t]);

  const refreshRuns = useCallback(async () => {
    setRunsLoading(true);
    setRunsError(null);
    try {
      const variables = {
        ruleSetId: maintenanceFilterArgument(runScopeRuleSetId),
        limit: MAINTENANCE_RUN_HISTORY_LIMIT,
      };
      const [evaluation, actions] = await Promise.all([
        client
          .query(maintenanceEvaluationRunsQuery, variables, {
            requestPolicy: "network-only",
          })
          .toPromise(),
        client
          .query(maintenanceActionRunsQuery, variables, {
            requestPolicy: "network-only",
          })
          .toPromise(),
      ]);
      if (evaluation.error) throw evaluation.error;
      setEvaluationRuns(
        (evaluation.data?.maintenanceEvaluationRuns as MaintenanceEvaluationRun[]) ?? [],
      );
      if (actions.error) throw actions.error;
      setActionRuns(
        (actions.data?.maintenanceActionRuns as MaintenanceActionRun[]) ?? [],
      );
    } catch (error) {
      setRunsError(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setRunsLoading(false);
    }
  }, [client, runScopeRuleSetId, t]);

  const refreshExclusions = useCallback(async () => {
    setExclusionsError(null);
    try {
      const { data, error } = await client
        .query(
          maintenanceExclusionsQuery,
          { ruleSetId: maintenanceFilterArgument(candidateRuleFilter) },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setExclusions((data?.maintenanceExclusions as MaintenanceExclusion[]) ?? []);
    } catch (error) {
      setExclusions([]);
      setExclusionsError(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [candidateRuleFilter, client, t]);

  useEffect(() => {
    void refreshRuleSets();
  }, [refreshRuleSets]);

  useEffect(() => {
    void refreshCatalogs();
  }, [refreshCatalogs]);

  useEffect(() => {
    void refreshGates();
  }, [refreshGates]);

  useEffect(() => {
    void refreshCandidates();
  }, [refreshCandidates]);

  useEffect(() => {
    void refreshRuns();
  }, [refreshRuns]);

  useEffect(() => {
    void refreshExclusions();
  }, [refreshExclusions]);

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

  // ── Instance gates ──────────────────────────────────────────────────

  /// Gate writes are never optimistic: the mutation returns the whole gate set,
  /// but the panel refetches instead, so what it shows is what the server holds
  /// even when another operator moved a gate in the same moment.
  const applyGate = useCallback(
    async (gate: MaintenanceGateKey, enabled: boolean) => {
      setSavingGate(gate);
      try {
        const { error } = await client
          .mutation(setMaintenanceInstanceGatesMutation, {
            input: { [gate]: enabled },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.maintenanceGatesUpdated"));
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setSavingGate(null);
        await refreshGates();
        await refreshCandidates();
      }
    },
    [client, refreshCandidates, refreshGates, setGlobalStatus, t],
  );

  const handleGateChange = useCallback(
    (gate: MaintenanceGateKey, enabled: boolean) => {
      /// Opening the destructive gate is the one switch that can let files be
      /// deleted, so it confirms first. Closing it never does.
      if (gate === "destructiveEffectsEnabled" && enabled) {
        setPendingDestructiveGate(true);
        return;
      }
      void applyGate(gate, enabled);
    },
    [applyGate],
  );

  // ── Rule mode and arming ────────────────────────────────────────────

  const setRuleMode = useCallback(
    async (record: MaintenanceRuleSetRecord, mode: MaintenanceEvaluationMode) => {
      if (mode === record.evaluationMode) return;
      setMutatingRuleSetId(record.id);
      try {
        const { error } = await client
          .mutation(setMaintenanceRuleModeMutation, {
            input: { id: record.id, mode },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.maintenanceRuleModeUpdated", { name: record.name }),
        );
        await refreshRuleSets();
        await refreshCandidates();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingRuleSetId(null);
      }
    },
    [client, refreshCandidates, refreshRuleSets, setGlobalStatus, t],
  );

  const applyArming = useCallback(
    async (
      record: MaintenanceRuleSetRecord,
      arming: MaintenanceEffectArming,
      acknowledgedCandidateCount?: number,
    ): Promise<{ ok: true } | { ok: false; message: string }> => {
      const { error } = await client
        .mutation(setMaintenanceRuleArmingMutation, {
          input: setMaintenanceRuleArmingInput(
            record.id,
            arming,
            acknowledgedCandidateCount,
          ),
        })
        .toPromise();
      if (error) {
        return { ok: false, message: error.message };
      }
      await refreshRuleSets();
      return { ok: true };
    },
    [client, refreshRuleSets],
  );

  /// Count what an armed handler could still act on for one rule, and name a
  /// few of the titles. The dialog shows both, and sends the count it showed.
  const openDestructiveArming = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      setPendingArming({
        record,
        candidateCount: 0,
        sampleTitles: [],
        acknowledged: false,
        loading: true,
        countChanged: false,
      });
      try {
        const { data, error } = await client
          .query(
            maintenanceCandidatesQuery,
            {
              ruleSetId: record.id,
              includeShadow: true,
              limit: MAINTENANCE_CANDIDATE_LIMIT,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;
        const ruleCandidates =
          (data?.maintenanceCandidates as MaintenanceCandidate[]) ?? [];
        const actionable = ruleCandidates.filter((candidate) =>
          isNonTerminalCandidateState(candidate.state),
        );
        setPendingArming((prev) =>
          prev && prev.record.id === record.id
            ? {
                ...prev,
                candidateCount: actionable.length,
                sampleTitles: actionable
                  .slice(0, DESTRUCTIVE_ARMING_PREVIEW_TITLES)
                  .map((candidate) => candidate.titleName),
                loading: false,
              }
            : prev,
        );
      } catch (error) {
        setPendingArming(null);
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      }
    },
    [client, setGlobalStatus, t],
  );

  const setRuleArming = useCallback(
    (record: MaintenanceRuleSetRecord, arming: MaintenanceEffectArming) => {
      if (arming === record.effectArming) return;
      if (arming === "DESTRUCTIVE") {
        void openDestructiveArming(record);
        return;
      }
      void (async () => {
        setMutatingRuleSetId(record.id);
        const result = await applyArming(record, arming);
        setMutatingRuleSetId(null);
        setGlobalStatus(
          result.ok
            ? t("settings.maintenanceRuleArmingUpdated", { name: record.name })
            : result.message,
        );
      })();
    },
    [applyArming, openDestructiveArming, setGlobalStatus, t],
  );

  /// The server rejects a stale acknowledgement and reports the count it holds.
  /// Rather than dropping the operator back to the table, the dialog adopts
  /// that count, clears the acknowledgement, and asks again against the real
  /// number.
  const confirmDestructiveArming = useCallback(async () => {
    if (!pendingArming || !pendingArming.acknowledged) return;
    const { record, candidateCount } = pendingArming;
    setPendingArming((prev) => (prev ? { ...prev, loading: true } : prev));
    const result = await applyArming(record, "DESTRUCTIVE", candidateCount);
    if (result.ok) {
      setPendingArming(null);
      setGlobalStatus(
        t("settings.maintenanceRuleArmingUpdated", { name: record.name }),
      );
      return;
    }
    const serverCount = parseAcknowledgedCandidateCountMismatch(result.message);
    if (serverCount === null) {
      setPendingArming(null);
      setGlobalStatus(result.message);
      return;
    }
    setPendingArming((prev) =>
      prev
        ? {
            ...prev,
            candidateCount: serverCount,
            acknowledged: false,
            loading: false,
            countChanged: true,
          }
        : prev,
    );
  }, [applyArming, pendingArming, setGlobalStatus, t]);

  // ── Run now ─────────────────────────────────────────────────────────

  const announceTrigger = useCallback(
    (result: MaintenanceTriggerResult | null) => {
      if (!result) return;
      setGlobalStatus(
        result.started
          ? (result.message ?? t("settings.maintenanceRunStarted"))
          : t("settings.maintenanceRunNotStarted", {
              message: result.message ?? "",
            }),
      );
    },
    [setGlobalStatus, t],
  );

  const runEvaluationNow = useCallback(async () => {
    setEvaluationTriggering(true);
    try {
      const { data, error } = await client
        .mutation(runMaintenanceEvaluationNowMutation, {
          ruleSetId: maintenanceFilterArgument(runScopeRuleSetId),
        })
        .toPromise();
      if (error) throw error;
      announceTrigger(
        (data?.runMaintenanceEvaluationNow as MaintenanceTriggerResult) ?? null,
      );
      await refreshRuns();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setEvaluationTriggering(false);
    }
  }, [
    announceTrigger,
    client,
    refreshCandidates,
    refreshRuns,
    runScopeRuleSetId,
    setGlobalStatus,
    t,
  ]);

  const runActionHandlerNow = useCallback(async () => {
    setActionTriggering(true);
    try {
      const { data, error } = await client
        .mutation(runMaintenanceActionHandlerNowMutation, {})
        .toPromise();
      if (error) throw error;
      announceTrigger(
        (data?.runMaintenanceActionHandlerNow as MaintenanceTriggerResult) ?? null,
      );
      await refreshRuns();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setActionTriggering(false);
    }
  }, [
    announceTrigger,
    client,
    refreshCandidates,
    refreshRuns,
    setGlobalStatus,
    t,
  ]);

  // ── Exclusions ──────────────────────────────────────────────────────

  const excludeCandidate = useCallback((candidate: MaintenanceCandidate) => {
    setPendingExclusion({
      titleId: candidate.titleId,
      titleName: candidate.titleName,
      /// Defaults to the rule the candidate came from rather than to a global
      /// exclusion: excluding a title from every maintenance rule at once is a
      /// bigger decision than the row the operator clicked.
      ruleSetId: candidate.ruleSetId,
      reason: "",
    });
  }, []);

  const confirmExclusion = useCallback(async () => {
    if (!pendingExclusion) return;
    setExcluding(true);
    try {
      const { error } = await client
        .mutation(excludeMaintenanceSubjectMutation, {
          input: excludeMaintenanceSubjectInput({
            titleId: pendingExclusion.titleId,
            ruleSetId: maintenanceFilterArgument(pendingExclusion.ruleSetId) ?? null,
            reason: pendingExclusion.reason,
          }),
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.maintenanceExcluded", { title: pendingExclusion.titleName }),
      );
      setPendingExclusion(null);
      await refreshExclusions();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setExcluding(false);
    }
  }, [
    client,
    pendingExclusion,
    refreshCandidates,
    refreshExclusions,
    setGlobalStatus,
    t,
  ]);

  const confirmRemoveExclusion = useCallback(async () => {
    if (!pendingRemoveExclusion) return;
    const exclusion = pendingRemoveExclusion;
    setRemovingExclusionId(exclusion.id);
    try {
      const { error } = await client
        .mutation(removeMaintenanceExclusionMutation, { id: exclusion.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.maintenanceExclusionRemoved", { title: exclusion.titleName }),
      );
      await refreshExclusions();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setRemovingExclusionId(null);
      setPendingRemoveExclusion(null);
    }
  }, [
    client,
    pendingRemoveExclusion,
    refreshCandidates,
    refreshExclusions,
    setGlobalStatus,
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
        gates={gates}
        setRuleMode={(record, mode) => void setRuleMode(record, mode)}
        setRuleArming={setRuleArming}
        operationsPanels={
          <>
            <MaintenanceGatesPanel
              gates={gates}
              gatesLocked={gatesLocked}
              savingGate={savingGate}
              onGateChange={handleGateChange}
            />
            <MaintenanceCandidatesPanel
              candidates={candidates}
              candidatesLoading={candidatesLoading}
              candidatesError={candidatesError}
              ruleSetRecords={ruleSetRecords}
              libraries={libraries}
              gates={gates}
              candidateRuleFilter={candidateRuleFilter}
              setCandidateRuleFilter={setCandidateRuleFilter}
              candidateStateFilter={candidateStateFilter}
              setCandidateStateFilter={setCandidateStateFilter}
              candidateLibraryFilter={candidateLibraryFilter}
              setCandidateLibraryFilter={setCandidateLibraryFilter}
              refreshCandidates={() => void refreshCandidates()}
              excludeCandidate={excludeCandidate}
            />
            <MaintenanceRunsPanel
              evaluationRuns={evaluationRuns}
              actionRuns={actionRuns}
              runsError={runsError}
              runsLoading={runsLoading}
              ruleSetRecords={ruleSetRecords}
              runScopeRuleSetId={runScopeRuleSetId}
              setRunScopeRuleSetId={setRunScopeRuleSetId}
              evaluationTriggering={evaluationTriggering}
              actionTriggering={actionTriggering}
              runEvaluationNow={() => void runEvaluationNow()}
              runActionHandlerNow={() => void runActionHandlerNow()}
              refreshRuns={() => void refreshRuns()}
            />
            <MaintenanceExclusionsPanel
              exclusions={exclusions}
              exclusionsError={exclusionsError}
              ruleSetRecords={ruleSetRecords}
              removingExclusionId={removingExclusionId}
              removeExclusion={setPendingRemoveExclusion}
            />
          </>
        }
      />
      <ConfirmDialog
        open={pendingDestructiveGate}
        contentId="settings-maintenance-gate-destructive-confirm"
        title={t("settings.maintenanceGateDestructiveConfirmTitle")}
        description={t("settings.maintenanceGateDestructiveConfirmBody")}
        confirmLabel={t("settings.maintenanceGateDestructiveConfirmAction")}
        cancelLabel={t("label.cancel")}
        isBusy={savingGate !== null}
        onConfirm={() => {
          setPendingDestructiveGate(false);
          void applyGate("destructiveEffectsEnabled", true);
        }}
        onCancel={() => setPendingDestructiveGate(false)}
      />
      <ConfirmDialog
        open={pendingArming !== null}
        contentId="settings-maintenance-arming-confirm"
        title={t("settings.maintenanceArmDestructiveTitle")}
        description={
          pendingArming
            ? t("settings.maintenanceArmDestructiveBody", {
                name: pendingArming.record.name,
              })
            : ""
        }
        confirmLabel={t("settings.maintenanceArmDestructiveConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={pendingArming?.loading ?? false}
        confirmDisabled={!pendingArming?.acknowledged}
        onConfirm={() => void confirmDestructiveArming()}
        onCancel={() => setPendingArming(null)}
      >
        {pendingArming ? (
          <div className="space-y-2 text-xs">
            {pendingArming.loading && pendingArming.candidateCount === 0 ? (
              <p className="text-muted-foreground">
                {t("settings.maintenanceArmDestructiveLoading")}
              </p>
            ) : (
              <>
                {pendingArming.countChanged ? (
                  <p
                    id="settings-maintenance-arming-count-changed"
                    className="rounded border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-1.5 text-[var(--scry-warning-text)]"
                  >
                    {t("settings.maintenanceArmDestructiveCountChanged", {
                      count: pendingArming.candidateCount,
                    })}
                  </p>
                ) : null}
                <p id="settings-maintenance-arming-count" className="font-semibold">
                  {pendingArming.candidateCount === 0
                    ? t("settings.maintenanceArmDestructiveNoCandidates")
                    : t("settings.maintenanceArmDestructiveCount", {
                        count: pendingArming.candidateCount,
                      })}
                </p>
                {pendingArming.sampleTitles.length > 0 ? (
                  <ul className="list-disc space-y-0.5 pl-5 text-muted-foreground">
                    {pendingArming.sampleTitles.map((title) => (
                      <li key={title}>{title}</li>
                    ))}
                  </ul>
                ) : null}
                <label className="flex items-start gap-2 pt-1">
                  <Checkbox
                    id="settings-maintenance-arming-acknowledge"
                    checked={pendingArming.acknowledged}
                    onCheckedChange={(value) =>
                      setPendingArming((prev) =>
                        prev ? { ...prev, acknowledged: value === true } : prev,
                      )
                    }
                  />
                  <span>
                    {t("settings.maintenanceArmDestructiveAcknowledge", {
                      count: pendingArming.candidateCount,
                    })}
                  </span>
                </label>
              </>
            )}
          </div>
        ) : null}
      </ConfirmDialog>
      <ConfirmDialog
        open={pendingExclusion !== null}
        contentId="settings-maintenance-exclude-confirm"
        title={t("settings.maintenanceExcludeTitle")}
        description={
          pendingExclusion
            ? t("settings.maintenanceExcludeBody", {
                title: pendingExclusion.titleName,
              })
            : ""
        }
        confirmLabel={t("settings.maintenanceExcludeConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={excluding}
        onConfirm={() => void confirmExclusion()}
        onCancel={() => setPendingExclusion(null)}
      >
        {pendingExclusion ? (
          <div className="space-y-3">
            <SingleSelectField
              id="settings-maintenance-exclude-scope"
              label={t("settings.maintenanceExcludeScope")}
              value={pendingExclusion.ruleSetId}
              onValueChange={(value) =>
                setPendingExclusion((prev) =>
                  prev ? { ...prev, ruleSetId: value } : prev,
                )
              }
              options={[
                {
                  value: MAINTENANCE_FILTER_ALL,
                  label: t("settings.maintenanceExcludeScopeGlobal"),
                },
                ...ruleSetRecords.map((record) => ({
                  value: record.id,
                  label: record.name,
                })),
              ]}
            />
            <label>
              <Label className="mb-2 block">
                {t("settings.maintenanceExcludeReason")}
              </Label>
              <Input
                id="settings-maintenance-exclude-reason"
                value={pendingExclusion.reason}
                placeholder={t("settings.maintenanceExcludeReasonPlaceholder")}
                onChange={(event) =>
                  setPendingExclusion((prev) =>
                    prev ? { ...prev, reason: event.target.value } : prev,
                  )
                }
              />
            </label>
          </div>
        ) : null}
      </ConfirmDialog>
      <ConfirmDialog
        open={pendingRemoveExclusion !== null}
        contentId="settings-maintenance-exclusion-remove-confirm"
        title={t("settings.maintenanceExclusionRemove")}
        description={
          pendingRemoveExclusion
            ? t("settings.maintenanceExclusionRemoving", {
                title: pendingRemoveExclusion.titleName,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={removingExclusionId !== null}
        onConfirm={() => void confirmRemoveExclusion()}
        onCancel={() => setPendingRemoveExclusion(null)}
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
