import { graphQlRuleValidationResult } from "@/lib/utils/rego-diagnostics";
import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useRegoValidation } from "@/lib/hooks/use-rego-validation";
import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  MAINTENANCE_CANDIDATE_LIMIT,
  useMaintenanceRuleState,
} from "@/components/containers/settings/use-maintenance-rule-state";
import type { MaintenanceRulesSection } from "@/components/root/types";
import {
  MaintenanceCandidatesPanel,
  MaintenanceExclusionsPanel,
  MaintenanceInstanceSwitches,
  MaintenanceRunsPanel,
} from "@/components/views/settings/settings-maintenance-operations-panel";
import {
  SettingsMaintenanceRulesSection,
  type MaintenanceLibraryOption,
  type MaintenanceStorageRootOption,
} from "@/components/views/settings/settings-maintenance-rules-section";
import { useRuleBuilderEditor } from "@/lib/hooks/use-rule-builder-editor";
import {
  loadQualityProfileNames,
  loadTitleTagLabels,
} from "@/lib/utils/rule-builder-catalogs";
import { isIncompleteRuleBuilderDraft } from "@/lib/utils/rule-builder-mode";
import { useRuleBuilderSchema } from "@/lib/hooks/use-rule-builder-schema";
import { MAINTENANCE_RULE_BUILDER_BASE } from "@/lib/utils/maintenance-rule-builder-schema";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import type { MaintenanceRuleTemplate } from "@/lib/constants/maintenance-rule-templates";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  MaintenanceActionRun,
  MaintenanceCandidate,
  MaintenanceEvaluationRun,
  MaintenanceExclusion,
  MaintenanceGateKey,
  MaintenanceInstanceGates,
  MaintenancePreviewResult,
  MaintenanceTestSubject,
  MaintenancePreviewSource,
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
  MaintenanceTriggerResult,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_FILTER_ALL,
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  maintenanceRuleBuilder,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  excludeMaintenanceSubjectInput,
  initialMaintenanceRuleDraft,
  maintenanceFilterArgument,
  maintenancePreviewInput,
  maintenanceRuleDraftFromDetail,
  maintenanceRuleUpdatePlan,
  newMaintenanceRuleBuilderDraft,
} from "@/lib/utils/maintenance-rule-sets";
import {
  librariesQuery,
  maintenanceActionRunsQuery,
  maintenanceCandidatesQuery,
  maintenanceEvaluationRunsQuery,
  maintenanceExclusionsQuery,
  maintenanceInstanceGatesQuery,
  maintenanceRuleSetQuery,
  maintenanceRuleSetsQuery,
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
  updateMaintenanceRuleMatcherMutation,
  updateMaintenanceRuleMetadataMutation,
} from "@/lib/graphql/mutations";

/// How many recent runs each history table shows. The API clamps its own
/// maximum; this is only how much of it the panel asks for.
const MAINTENANCE_RUN_HISTORY_LIMIT = 25;
type PendingExclusion = {
  subjectKind: string;
  subjectId: string;
  titleId: string;
  titleName: string;
  ruleSetId: string;
  reason: string;
};

type PendingEditorAction =
  | { type: "create" }
  | { type: "copy"; record: MaintenanceRuleSetRecord }
  | { type: "edit"; record: MaintenanceRuleSetRecord }
  | { type: "template"; template: MaintenanceRuleTemplate }
  | { type: "close" }
  | null;

export function SettingsMaintenanceRulesContainer({
  section = "rules",
}: {
  section?: MaintenanceRulesSection;
}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  const [ruleSetRecords, setRuleSetRecords] = useState<
    MaintenanceRuleSetRecord[]
  >([]);
  const [libraries, setLibraries] = useState<MaintenanceLibraryOption[]>([]);
  const [storageRoots, setStorageRoots] = useState<
    MaintenanceStorageRootOption[]
  >([]);

  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(
    null,
  );
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
  /// Changes with every editor opening, so controls that keep their own
  /// typing state start again from the new draft.
  const [editorSessionKey, setEditorSessionKey] = useState(0);
  const { validating, validationResult, setValidationResult, validateDraft } =
    useRegoValidation(
      "maintenance",
      ruleSetDraft.regoSource,
      editingRuleSetId,
      isEditorOpen,
    );

  const [previewSource, setPreviewSource] =
    useState<MaintenancePreviewSource>("stored");
  const [previewRuleSetId, setPreviewRuleSetId] = useState("");
  const [previewLibraryId, setPreviewLibraryId] = useState("");
  const [previewLimit, setPreviewLimit] = useState(
    MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  );
  const [previewing, setPreviewing] = useState(false);
  const [previewResult, setPreviewResult] =
    useState<MaintenancePreviewResult | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const previewRequest = useRef(0);

  const [gates, setGates] = useState<MaintenanceInstanceGates | null>(null);
  /// True once the gates query has been refused. Reading the gates needs
  /// system-settings management, and a reader without it must still get a
  /// working page rather than a failed section.
  const [gatesLocked, setGatesLocked] = useState(false);
  const [savingSwitch, setSavingSwitch] = useState<MaintenanceGateKey | null>(
    null,
  );
  const [pendingUnpause, setPendingUnpause] = useState(false);

  const [candidates, setCandidates] = useState<MaintenanceCandidate[]>([]);
  const [candidatesLoading, setCandidatesLoading] = useState(false);
  const [candidatesError, setCandidatesError] = useState<string | null>(null);
  const [candidateRuleFilter, setCandidateRuleFilter] = useState(
    MAINTENANCE_FILTER_ALL,
  );
  const [candidateStateFilter, setCandidateStateFilter] = useState(
    MAINTENANCE_FILTER_ALL,
  );
  const [candidateLibraryFilter, setCandidateLibraryFilter] = useState(
    MAINTENANCE_FILTER_ALL,
  );

  const [evaluationRuns, setEvaluationRuns] = useState<
    MaintenanceEvaluationRun[]
  >([]);
  const [actionRuns, setActionRuns] = useState<MaintenanceActionRun[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [runScopeRuleSetId, setRunScopeRuleSetId] = useState(
    MAINTENANCE_FILTER_ALL,
  );
  const [evaluationTriggering, setEvaluationTriggering] = useState(false);
  const [actionTriggering, setActionTriggering] = useState(false);

  const [exclusions, setExclusions] = useState<MaintenanceExclusion[]>([]);
  const [exclusionsError, setExclusionsError] = useState<string | null>(null);
  const [removingExclusionId, setRemovingExclusionId] = useState<string | null>(
    null,
  );
  const [pendingRemoveExclusion, setPendingRemoveExclusion] =
    useState<MaintenanceExclusion | null>(null);
  const [pendingExclusion, setPendingExclusion] =
    useState<PendingExclusion | null>(null);
  const [excluding, setExcluding] = useState(false);

  const isDraftDirty =
    JSON.stringify(ruleSetDraft) !== JSON.stringify(ruleSetDraftBaseline);

  const reportLoadError = useCallback(
    (error: unknown) =>
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      ),
    [setGlobalStatus, t],
  );
  /// The builder's pickers: libraries and roots are already on the page, the
  /// other catalogs load the first time a chosen action needs them.
  const staticBuilderOptions = useMemo(
    () => ({
      libraries: libraries.map((library) => library.name),
      roots: [...new Set(storageRoots.map((root) => root.path))],
    }),
    [libraries, storageRoots],
  );
  const builderOptionLoaders = useMemo(
    () => ({
      qualityProfiles: () => loadQualityProfileNames(client),
      titleTags: () => loadTitleTagLabels(client),
    }),
    [client],
  );
  const servedSchema = useRuleBuilderSchema(MAINTENANCE_RULE_BUILDER_BASE);
  const builderBinding = useMemo(
    () => maintenanceRuleBuilder(servedSchema.schema),
    [servedSchema.schema],
  );
  const whenSchemaReady = servedSchema.whenReady;
  const builderEditor = useRuleBuilderEditor({
    binding: builderBinding,
    draft: ruleSetDraft,
    setDraft: setRuleSetDraft,
    isEditorOpen,
    staticOptions: staticBuilderOptions,
    optionLoaders: builderOptionLoaders,
    onLoadError: reportLoadError,
  });
  const beginBuilderSession = builderEditor.beginSession;

  /// Every editor opening goes through here: a rule still made by the builder
  /// opens in it, anything else opens as Rego.
  const startEditorSession = useCallback(
    (next: MaintenanceRuleSetDraft, baseline: MaintenanceRuleSetDraft) => {
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(baseline);
      setEditorSessionKey((key) => key + 1);
      beginBuilderSession(next);
      setValidationResult(null);
    },
    [beginBuilderSession, setValidationResult],
  );

  const closeEditor = useCallback(() => {
    const next = initialMaintenanceRuleDraft();
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    startEditorSession(next, next);
  }, [startEditorSession]);

  const openCreateEditor = useCallback(async () => {
    const binding = maintenanceRuleBuilder(await whenSchemaReady());
    const next = newMaintenanceRuleBuilderDraft(binding);
    setEditingRuleSetId(null);
    startEditorSession(next, next);
    setIsEditorOpen(true);
  }, [startEditorSession, whenSchemaReady]);

  /// Load the source and declared actions for one rule set, surfacing failure
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
          setGlobalStatus(t("status.failedToLoad"), { level: "ERROR" });
        }
        return detail;
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
          { level: "ERROR" },
        );
        return null;
      }
    },
    [client, setGlobalStatus, t],
  );

  const openEditEditor = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      const [detail, schema] = await Promise.all([
        fetchRuleSetDetail(record.id),
        whenSchemaReady(),
      ]);
      if (!detail) return;
      const next = maintenanceRuleDraftFromDetail(
        detail,
        maintenanceRuleBuilder(schema),
      );
      setEditingRuleSetId(record.id);
      startEditorSession(next, next);
      setIsEditorOpen(true);
    },
    [startEditorSession, fetchRuleSetDetail, whenSchemaReady],
  );

  const openCopyEditor = useCallback(
    async (record: MaintenanceRuleSetRecord) => {
      const [detail, schema] = await Promise.all([
        fetchRuleSetDetail(record.id),
        whenSchemaReady(),
      ]);
      if (!detail) return;
      const next = copyMaintenanceRuleDraft(detail, maintenanceRuleBuilder(schema));
      setEditingRuleSetId(null);
      startEditorSession(next, next);
      setIsEditorOpen(true);
    },
    [startEditorSession, fetchRuleSetDetail, whenSchemaReady],
  );

  /// Load a starter template into the create-rule editor. A template prefills
  /// every field the editor owns — name, description, source, scope, grace
  /// period — and then stops: it never creates a rule, and the
  /// draft it leaves behind is dirty, so the operator has to read it and press
  /// create themselves. `libraryIds` stays empty because which libraries a rule
  /// covers is an instance-specific decision no template can make.
  const openTemplateEditor = useCallback(
    (template: MaintenanceRuleTemplate) => {
      const next: MaintenanceRuleSetDraft = {
        ...initialMaintenanceRuleDraft(),
        name: template.name,
        description: t(template.descriptionKey),
        regoSource: template.regoSource,
        subjectKind: template.subjectKind ?? "TITLE",
        graceDays: template.graceDays,
      };
      setEditingRuleSetId(null);
      /// Baseline stays at the empty draft on purpose: a freshly applied
      /// template counts as unsaved work, so navigating away from it asks.
      startEditorSession(next, initialMaintenanceRuleDraft());
      setIsEditorOpen(true);
    },
    [startEditorSession, t],
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

  const requestApplyTemplate = useCallback(
    (template: MaintenanceRuleTemplate) => {
      if (!isEditorOpen || !isDraftDirty) {
        openTemplateEditor(template);
        return;
      }
      setPendingEditorAction({ type: "template", template });
    },
    [isDraftDirty, isEditorOpen, openTemplateEditor],
  );

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      closeEditor();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [closeEditor, isDraftDirty, isEditorOpen]);

  /// The list payload carries the declared actions and grace period of each rule's
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
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
    }
  }, [client, setGlobalStatus, t]);

  const refreshCatalogs = useCallback(async () => {
    try {
      const libraryList = await client.query(librariesQuery, {}).toPromise();
      if (!libraryList.error) {
        const catalogLibraries = libraryList.data?.libraries ?? [];
        setLibraries(
          catalogLibraries.map((library: { id: string; name: string }) => ({
            id: library.id,
            name: library.name,
          })),
        );
        setStorageRoots(
          catalogLibraries.flatMap(
            (library: {
              id: string;
              name: string;
              roots?: { id: string; path: string }[];
            }) =>
              (library.roots ?? []).map((root) => ({
                id: root.id,
                path: root.path,
                libraryName: library.name,
              })),
          ),
        );
      }
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
    }
  }, [client, setGlobalStatus, t]);

  /// The gates query is the one call on this page that can be refused outright.
  /// A refusal is a permission answer, not a page failure: it locks the panel
  /// and leaves the rest of the section working.
  const refreshGates = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(
          maintenanceInstanceGatesQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const next =
        (data?.maintenanceInstanceGates as MaintenanceInstanceGates) ?? null;
      setGates(next);
      setGatesLocked(next === null);
    } catch {
      setGates(null);
      setGatesLocked(true);
    }
  }, [client]);

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
            limit: MAINTENANCE_CANDIDATE_LIMIT,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setCandidates(
        (data?.maintenanceCandidates as MaintenanceCandidate[]) ?? [],
      );
    } catch (error) {
      setCandidates([]);
      setCandidatesError(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
      );
    } finally {
      setCandidatesLoading(false);
    }
  }, [
    candidateLibraryFilter,
    candidateRuleFilter,
    candidateStateFilter,
    client,
    t,
  ]);

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
        (evaluation.data
          ?.maintenanceEvaluationRuns as MaintenanceEvaluationRun[]) ?? [],
      );
      if (actions.error) throw actions.error;
      setActionRuns(
        (actions.data?.maintenanceActionRuns as MaintenanceActionRun[]) ?? [],
      );
    } catch (error) {
      setRunsError(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
      );
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
      setExclusions(
        (data?.maintenanceExclusions as MaintenanceExclusion[]) ?? [],
      );
    } catch (error) {
      setExclusions([]);
      setExclusionsError(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
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

  // Each pane pays only for what it shows. Rule sets, catalogs and the instance
  // switches are shared — every pane shows the switches and labels its rows
  // with the rule sets — but candidates, runs and exclusions are one pane's own query.
  useEffect(() => {
    if (section !== "candidates") {
      return;
    }
    void refreshCandidates();
  }, [refreshCandidates, section]);

  useEffect(() => {
    if (section !== "history") {
      return;
    }
    void refreshRuns();
  }, [refreshRuns, section]);

  useEffect(() => {
    if (section !== "candidates") {
      return;
    }
    void refreshExclusions();
  }, [refreshExclusions, section]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "copy") {
      openCopyEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "template") {
      openTemplateEditor(pendingEditorAction.template);
    } else {
      closeEditor();
    }
    setPendingEditorAction(null);
  }, [
    closeEditor,
    openCopyEditor,
    openCreateEditor,
    openEditEditor,
    openTemplateEditor,
    pendingEditorAction,
  ]);

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
      setGlobalStatus(
        t("settings.maintenanceRuleDeleted", { name: record.name }),
        { level: "SUCCESS" },
      );
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
        userFacingGraphQlErrorMessage(error, t("status.failedToDelete"), t),
        { level: "ERROR" },
      );
    } finally {
      setMutatingRuleSetId(null);
      setPendingDeleteRuleSet(null);
    }
  };

  /// Editing a saved rule writes up to two halves: the matcher (a new
  /// immutable revision) and the metadata (name, description, library scope,
  /// schedule), which the API keeps on separate mutations because only one of
  /// them versions. A half the editor did not change is not sent.
  const submitRuleSet = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (
      builderEditor.attached &&
      isIncompleteRuleBuilderDraft(builderBinding, ruleSetDraft)
    ) {
      setValidationResult({
        valid: false,
        errors: [t("settings.ruleBuilderIncomplete")],
      });
      return;
    }
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
        const plan = maintenanceRuleUpdatePlan(
          editingRuleSetId,
          ruleSetDraftBaseline,
          ruleSetDraft,
          builderBinding,
        );
        if (plan.matcher) {
          const matcher = await client
            .mutation(updateMaintenanceRuleMatcherMutation, {
              input: plan.matcher,
            })
            .toPromise();
          if (matcher.error) throw matcher.error;
        }
        if (plan.metadata) {
          const metadata = await client
            .mutation(updateMaintenanceRuleMetadataMutation, {
              input: plan.metadata,
            })
            .toPromise();
          if (metadata.error) throw metadata.error;
        }
        setGlobalStatus(t("settings.maintenanceRuleUpdated"), {
          level: "SUCCESS",
        });
      } else {
        const { error } = await client
          .mutation(createMaintenanceRuleSetMutation, {
            input: createMaintenanceRuleSetInput(ruleSetDraft, builderBinding),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.maintenanceRuleCreated"), {
          level: "SUCCESS",
        });
      }
      closeEditor();
      await refreshRuleSets();
    } catch (error) {
      const message = userFacingGraphQlErrorMessage(
        error,
        t("status.failedToUpdate"),
        t,
      );
      setValidationResult(graphQlRuleValidationResult(error, message));
    } finally {
      setMutatingRuleSetId(null);
    }
  };

  const runPreview = useCallback(
    async (subject?: MaintenanceTestSubject) => {
      const request = ++previewRequest.current;
      setPreviewing(true);
      setPreviewError(null);
      try {
        const { data, error } = await client
          .mutation(previewMaintenanceRuleMutation, {
            input: maintenancePreviewInput({
              ruleSetId: previewSource === "stored" ? previewRuleSetId : null,
              draft: previewSource === "draft" ? ruleSetDraft : undefined,
              titleIds: subject ? [subject.titleId] : undefined,
              subjectIds: subject ? [subject.subjectId] : undefined,
              libraryId: previewLibraryId,
              limit: previewLimit,
            }),
          })
          .toPromise();
        if (request !== previewRequest.current) return;
        if (error) throw error;
        setPreviewResult(
          data.previewMaintenanceRule as MaintenancePreviewResult,
        );
      } catch (error) {
        if (request !== previewRequest.current) return;
        setPreviewResult(null);
        setPreviewError(
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        );
      } finally {
        if (request === previewRequest.current) setPreviewing(false);
      }
    },
    [
      client,
      previewLibraryId,
      previewLimit,
      previewRuleSetId,
      previewSource,
      ruleSetDraft,
      t,
    ],
  );

  // Results and sample targets belong to the matcher and library that produced
  // them. Discard late responses when the operator changes those inputs.
  useEffect(() => {
    previewRequest.current += 1;
    setPreviewResult(null);
    setPreviewError(null);
    setPreviewing(false);
  }, [runPreview]);

  // ── Instance switches ───────────────────────────────────────────────

  /// Switch writes are never optimistic: the page refetches instead, so what
  /// it shows is what the server holds even when another operator moved a
  /// switch in the same moment.
  const applySwitch = useCallback(
    async (key: MaintenanceGateKey, value: boolean) => {
      setSavingSwitch(key);
      try {
        const { error } = await client
          .mutation(setMaintenanceInstanceGatesMutation, {
            input: { [key]: value },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.maintenanceSwitchesUpdated"), {
          level: "SUCCESS",
        });
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
      } finally {
        setSavingSwitch(null);
        await refreshGates();
        await refreshCandidates();
      }
    },
    [client, refreshCandidates, refreshGates, setGlobalStatus, t],
  );

  const handleSwitchChange = useCallback(
    (key: MaintenanceGateKey, value: boolean) => {
      /// Resuming actions is the one switch that can let files be deleted, so
      /// it confirms first. Pausing never does.
      if (key === "actionsPaused" && !value) {
        setPendingUnpause(true);
        return;
      }
      void applySwitch(key, value);
    },
    [applySwitch],
  );

  // ── Rule state ──────────────────────────────────────────────────────

  const refreshAfterRuleState = useCallback(async () => {
    await refreshRuleSets();
    await refreshCandidates();
  }, [refreshCandidates, refreshRuleSets]);
  const { setRuleState, armingDialog } = useMaintenanceRuleState({
    onChanged: refreshAfterRuleState,
    setBusyRuleSetId: setMutatingRuleSetId,
  });

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
        result.started ? undefined : { level: "WARNING" },
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
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
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
        (data?.runMaintenanceActionHandlerNow as MaintenanceTriggerResult) ??
          null,
      );
      await refreshRuns();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
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
      subjectKind: candidate.subjectKind,
      subjectId: candidate.subjectId,
      titleName:
        candidate.subjectKind === "title"
          ? candidate.titleName
          : `${candidate.titleName} · ${candidate.subjectLabel}`,
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
            subjectKind: pendingExclusion.subjectKind,
            subjectId: pendingExclusion.subjectId,
            ruleSetId:
              maintenanceFilterArgument(pendingExclusion.ruleSetId) ?? null,
            reason: pendingExclusion.reason,
          }),
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.maintenanceExcluded", {
          title: pendingExclusion.titleName,
        }),
      );
      setPendingExclusion(null);
      await refreshExclusions();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
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
        t("settings.maintenanceExclusionRemoved", {
          title: exclusion.titleName,
        }),
      );
      await refreshExclusions();
      await refreshCandidates();
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToDelete"), t),
        { level: "ERROR" },
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
        editorSessionKey={editorSessionKey}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={requestCloseEditor}
        startCreateRuleSet={requestCreateEditor}
        applyTemplate={requestApplyTemplate}
        ruleSetRecords={ruleSetRecords}
        libraries={libraries}
        storageRoots={storageRoots}
        editorMode={builderEditor.mode}
        setEditorMode={builderEditor.setMode}
        editRegoSource={builderEditor.editSource}
        setBuilderModel={builderEditor.setModel}
        editAsRego={builderEditor.editAsRego}
        builderOptions={builderEditor.options}
        builderSchema={servedSchema.schema}
        builderStatus={servedSchema.status}
        copyRuleSet={requestCopyRuleSet}
        editRuleSet={requestEditRuleSet}
        deleteRuleSet={deleteRuleSet}
        validating={validating}
        validationResult={validationResult}
        previewSource={previewSource}
        setPreviewSource={setPreviewSource}
        previewRuleSetId={previewRuleSetId}
        setPreviewRuleSetId={setPreviewRuleSetId}
        previewLibraryId={previewLibraryId}
        setPreviewLibraryId={setPreviewLibraryId}
        previewLimit={previewLimit}
        setPreviewLimit={(limit) =>
          setPreviewLimit(clampMaintenancePreviewLimit(limit))
        }
        runPreview={runPreview}
        previewing={previewing}
        previewResult={previewResult}
        previewError={previewError}
        instanceSwitches={
          <MaintenanceInstanceSwitches
            gates={gates}
            gatesLocked={gatesLocked}
            savingSwitch={savingSwitch}
            onSwitchChange={handleSwitchChange}
          />
        }
        setRuleState={setRuleState}
        section={section}
        operationsPanels={
          section === "candidates" ? (
            <>
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
              <MaintenanceExclusionsPanel
                exclusions={exclusions}
                exclusionsError={exclusionsError}
                ruleSetRecords={ruleSetRecords}
                removingExclusionId={removingExclusionId}
                removeExclusion={setPendingRemoveExclusion}
              />
            </>
          ) : section === "history" ? (
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
          ) : null
        }
      />
      <ConfirmDialog
        open={pendingUnpause}
        contentId="settings-maintenance-unpause-confirm"
        title={t("settings.maintenanceUnpauseConfirmTitle")}
        description={t("settings.maintenanceUnpauseConfirmBody")}
        confirmLabel={t("settings.maintenanceUnpauseConfirmAction")}
        cancelLabel={t("label.cancel")}
        isBusy={savingSwitch !== null}
        onConfirm={() => {
          setPendingUnpause(false);
          void applySwitch("actionsPaused", false);
        }}
        onCancel={() => setPendingUnpause(false)}
      />
      {armingDialog}
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
            : pendingEditorAction?.type === "template"
              ? t("settings.maintenanceTemplateApply")
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
