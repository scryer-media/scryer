import { graphQlRuleValidationResult } from "@/lib/utils/rego-diagnostics";
import { revealFirstRuleBuilderProblem } from "@/components/views/settings/rule-builder";
import { userFacingGraphQlErrorMessage } from "../../../lib/graphql/error-message.ts";
import { useRegoValidation } from "@/lib/hooks/use-rego-validation";
import {
  type FormEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  LIFECYCLE_CANDIDATE_LIMIT,
  useLifecycleRuleState,
} from "@/components/containers/settings/use-lifecycle-rule-state";
import type { LifecycleRulesSection } from "@/components/root/types";
import {
  LifecycleCandidatesPanel,
  LifecycleExclusionsPanel,
  LifecycleInstanceSwitches,
  LifecycleRunsPanel,
} from "@/components/views/settings/settings-lifecycle-operations-panel";
import {
  SettingsLifecycleRulesSection,
  type LifecycleLibraryOption,
} from "@/components/views/settings/settings-lifecycle-rules-section";
import { useRuleBuilderEditor } from "@/lib/hooks/use-rule-builder-editor";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { useExternalRuleImport } from "@/lib/hooks/use-external-rule-import";
import { MaintainerrRuleImportDialog } from "@/components/views/settings/maintainerr-rule-import-dialog";
import { TextActionButton } from "@/components/ui/text-action-button";
import { FileInput } from "lucide-react";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import {
  loadListIdOptions,
  loadQualityProfileIdOptions,
  loadQualityProfileNames,
  loadUserIdOptions,
  loadTitleTagLabels,
} from "@/lib/utils/rule-builder-catalogs";
import {
  isIncompleteRuleBuilderDraft,
  openRuleTemplateDraft,
} from "@/lib/utils/rule-builder-mode";
import { ruleTemplateBuilderModel } from "@/lib/constants/rule-template-builder-models";
import { useRuleBuilderSchema } from "@/lib/hooks/use-rule-builder-schema";
import { LIFECYCLE_RULE_BUILDER_BASE } from "@/lib/utils/lifecycle-rule-builder-schema";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import type { LifecycleRuleTemplate } from "@/lib/constants/lifecycle-rule-templates";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LifecycleActionRun,
  LifecycleCandidate,
  LifecycleEvaluationRun,
  LifecycleExclusion,
  LifecycleGateKey,
  LifecycleInstanceGates,
  LifecyclePreviewResult,
  LifecycleTestSubject,
  LifecyclePreviewSource,
  LifecycleRuleSetDetail,
  LifecycleRuleSetDraft,
  LifecycleRuleSetRecord,
  LifecycleTriggerResult,
  LifecycleValidationResult,
} from "@/lib/types/lifecycle-rule-sets";
import {
  LIFECYCLE_FILTER_ALL,
  LIFECYCLE_PREVIEW_LIMIT_DEFAULT,
  lifecycleRuleBuilder,
  clampLifecyclePreviewLimit,
  copyLifecycleRuleDraft,
  createLifecycleRuleSetInput,
  excludeLifecycleSubjectInput,
  initialLifecycleRuleDraft,
  lifecycleFilterArgument,
  lifecyclePreviewInput,
  lifecycleRuleDraftFromDetail,
  lifecycleRuleUpdatePlan,
  newLifecycleRuleBuilderDraft,
  LIFECYCLE_CREATE_ENABLED,
} from "@/lib/utils/lifecycle-rule-sets";
import {
  librariesQuery,
  lifecycleActionRunsQuery,
  lifecycleCandidatesQuery,
  lifecycleEvaluationRunsQuery,
  lifecycleExclusionsQuery,
  lifecycleInstanceGatesQuery,
  lifecycleRuleSetQuery,
  lifecycleRuleSetsQuery,
} from "@/lib/graphql/queries";
import {
  createLifecycleRuleSetMutation,
  deleteLifecycleRuleSetMutation,
  excludeLifecycleSubjectMutation,
  previewLifecycleRuleMutation,
  removeLifecycleExclusionMutation,
  runLifecycleActionHandlerNowMutation,
  runLifecycleEvaluationNowMutation,
  setLifecycleInstanceGatesMutation,
  updateLifecycleRuleSourceMutation,
  updateLifecycleRuleMetadataMutation,
} from "@/lib/graphql/mutations";

/// How many recent runs each history table shows. The API clamps its own
/// maximum; this is only how much of it the panel asks for.
const LIFECYCLE_RUN_HISTORY_LIMIT = 25;
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
  | { type: "copy"; record: LifecycleRuleSetRecord }
  | { type: "edit"; record: LifecycleRuleSetRecord }
  | { type: "template"; template: LifecycleRuleTemplate }
  | { type: "close" }
  | null;

export function SettingsLifecycleRulesContainer({
  section = "rules",
}: {
  section?: LifecycleRulesSection;
}) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  const [ruleSetRecords, setRuleSetRecords] = useState<
    LifecycleRuleSetRecord[]
  >([]);
  const [libraries, setLibraries] = useState<LifecycleLibraryOption[]>([]);
  /// Configured root paths, offered by the builder's root picker.
  const [rootPaths, setRootPaths] = useState<string[]>([]);

  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(
    null,
  );
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteRuleSet, setPendingDeleteRuleSet] =
    useState<LifecycleRuleSetRecord | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingEditorAction>(null);
  const [ruleSetDraft, setRuleSetDraft] = useState<LifecycleRuleSetDraft>(
    initialLifecycleRuleDraft,
  );
  const [ruleSetDraftBaseline, setRuleSetDraftBaseline] =
    useState<LifecycleRuleSetDraft>(initialLifecycleRuleDraft);
  /// Changes with every editor opening, so controls that keep their own
  /// typing state start again from the new draft.
  const [editorSessionKey, setEditorSessionKey] = useState(0);
  const { validating, validationResult, setValidationResult, validateDraft } =
    useRegoValidation(
      "lifecycle",
      ruleSetDraft.regoSource,
      editingRuleSetId,
      isEditorOpen,
    );

  const [previewSource, setPreviewSource] =
    useState<LifecyclePreviewSource>("stored");
  const [previewRuleSetId, setPreviewRuleSetId] = useState("");
  const [previewLibraryId, setPreviewLibraryId] = useState("");
  const [previewLimit, setPreviewLimit] = useState(
    LIFECYCLE_PREVIEW_LIMIT_DEFAULT,
  );
  const [previewing, setPreviewing] = useState(false);
  const [previewResult, setPreviewResult] =
    useState<LifecyclePreviewResult | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const previewRequest = useRef(0);

  const [gates, setGates] = useState<LifecycleInstanceGates | null>(null);
  /// True once the gates query has been refused. Reading the gates needs
  /// system-settings management, and a reader without it must still get a
  /// working page rather than a failed section.
  const [gatesLocked, setGatesLocked] = useState(false);
  const [savingSwitch, setSavingSwitch] = useState<LifecycleGateKey | null>(
    null,
  );
  const [pendingUnpause, setPendingUnpause] = useState(false);

  const [candidates, setCandidates] = useState<LifecycleCandidate[]>([]);
  const [candidatesLoading, setCandidatesLoading] = useState(false);
  const [candidatesError, setCandidatesError] = useState<string | null>(null);
  const [candidateRuleFilter, setCandidateRuleFilter] =
    useState(LIFECYCLE_FILTER_ALL);
  const [candidateStateFilter, setCandidateStateFilter] =
    useState(LIFECYCLE_FILTER_ALL);
  const [candidateLibraryFilter, setCandidateLibraryFilter] =
    useState(LIFECYCLE_FILTER_ALL);

  const [evaluationRuns, setEvaluationRuns] = useState<
    LifecycleEvaluationRun[]
  >([]);
  const [actionRuns, setActionRuns] = useState<LifecycleActionRun[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [runScopeRuleSetId, setRunScopeRuleSetId] =
    useState(LIFECYCLE_FILTER_ALL);
  const [evaluationTriggering, setEvaluationTriggering] = useState(false);
  const [actionTriggering, setActionTriggering] = useState(false);

  const [exclusions, setExclusions] = useState<LifecycleExclusion[]>([]);
  const [exclusionsError, setExclusionsError] = useState<string | null>(null);
  const [removingExclusionId, setRemovingExclusionId] = useState<string | null>(
    null,
  );
  const [pendingRemoveExclusion, setPendingRemoveExclusion] =
    useState<LifecycleExclusion | null>(null);
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
      roots: [...new Set(rootPaths)],
    }),
    [libraries, rootPaths],
  );
  // Listing users needs user management; without it a user id is typed in.
  const sessionUser = useSessionUser();
  const canListUsers =
    sessionUser !== null &&
    hasAppPermission(sessionUser, APP_PERMISSIONS.manageUsers);
  const builderOptionLoaders = useMemo(
    () => ({
      qualityProfiles: () => loadQualityProfileNames(client),
      titleTags: () => loadTitleTagLabels(client),
      qualityProfileIds: () => loadQualityProfileIdOptions(client),
      lists: () => loadListIdOptions(client),
      ...(canListUsers ? { users: () => loadUserIdOptions(client) } : {}),
    }),
    [client, canListUsers],
  );
  const servedSchema = useRuleBuilderSchema(LIFECYCLE_RULE_BUILDER_BASE);
  const builderBinding = useMemo(
    () => lifecycleRuleBuilder(servedSchema.schema),
    [servedSchema.schema],
  );
  const whenSchemaReady = servedSchema.whenReady;
  /// The draft the editor shows, for a template open that settles later.
  const shownDraftRef = useRef(ruleSetDraft);
  useLayoutEffect(() => {
    shownDraftRef.current = ruleSetDraft;
  }, [ruleSetDraft]);
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
    (next: LifecycleRuleSetDraft, baseline: LifecycleRuleSetDraft) => {
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(baseline);
      setEditorSessionKey((key) => key + 1);
      beginBuilderSession(next);
      setValidationResult(null);
    },
    [beginBuilderSession, setValidationResult],
  );

  const closeEditor = useCallback(() => {
    const next = initialLifecycleRuleDraft();
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    startEditorSession(next, next);
  }, [startEditorSession]);

  const openCreateEditor = useCallback(async () => {
    const binding = lifecycleRuleBuilder(await whenSchemaReady());
    const next = newLifecycleRuleBuilderDraft(binding);
    setEditingRuleSetId(null);
    startEditorSession(next, next);
    setIsEditorOpen(true);
  }, [startEditorSession, whenSchemaReady]);

  /// Load the source and declared actions for one rule set, surfacing failure
  /// instead of silently refusing to open the editor.
  const fetchRuleSetDetail = useCallback(
    async (id: string): Promise<LifecycleRuleSetDetail | null> => {
      try {
        const { data, error } = await client
          .query(lifecycleRuleSetQuery, { id })
          .toPromise();
        if (error) throw error;
        const detail =
          (data?.lifecycleRuleSet as LifecycleRuleSetDetail | null) ?? null;
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
    async (record: LifecycleRuleSetRecord) => {
      const [detail, schema] = await Promise.all([
        fetchRuleSetDetail(record.id),
        whenSchemaReady(),
      ]);
      if (!detail) return;
      const next = lifecycleRuleDraftFromDetail(
        detail,
        lifecycleRuleBuilder(schema),
      );
      setEditingRuleSetId(record.id);
      startEditorSession(next, next);
      setIsEditorOpen(true);
    },
    [startEditorSession, fetchRuleSetDetail, whenSchemaReady],
  );

  const openCopyEditor = useCallback(
    async (record: LifecycleRuleSetRecord) => {
      const [detail, schema] = await Promise.all([
        fetchRuleSetDetail(record.id),
        whenSchemaReady(),
      ]);
      if (!detail) return;
      const next = copyLifecycleRuleDraft(detail, lifecycleRuleBuilder(schema));
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
    (template: LifecycleRuleTemplate) =>
      openRuleTemplateDraft<LifecycleRuleSetDraft>({
        schema: servedSchema.schema,
        whenSchemaReady,
        binding: lifecycleRuleBuilder,
        draft: {
          ...initialLifecycleRuleDraft(),
          name: template.name,
          description: t(template.descriptionKey),
          regoSource: template.regoSource,
          graceDays: template.graceDays,
          maxTitlesPerRun: template.maxTitlesPerRun ?? null,
        },
        model: (schema) => ruleTemplateBuilderModel(schema, template.id),
        open: (next) => {
          shownDraftRef.current = next;
          setEditingRuleSetId(null);
          /// Baseline stays at the empty draft on purpose: a freshly applied
          /// template counts as unsaved work, so navigating away from it asks.
          startEditorSession(next, initialLifecycleRuleDraft());
          setIsEditorOpen(true);
        },
        isShowing: (draft) => shownDraftRef.current === draft,
      }),
    [servedSchema.schema, startEditorSession, t, whenSchemaReady],
  );

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditRuleSet = useCallback(
    (record: LifecycleRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(record);
        return;
      }
      setPendingEditorAction({ type: "edit", record });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCopyRuleSet = useCallback(
    (record: LifecycleRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        openCopyEditor(record);
        return;
      }
      setPendingEditorAction({ type: "copy", record });
    },
    [isDraftDirty, isEditorOpen, openCopyEditor],
  );

  const requestApplyTemplate = useCallback(
    (template: LifecycleRuleTemplate) => {
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
        .query(lifecycleRuleSetsQuery, {})
        .toPromise();
      if (error) throw error;
      setRuleSetRecords(data?.lifecycleRuleSets ?? []);
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
    }
  }, [client, setGlobalStatus, t]);

  // The import run belongs to the page rather than to its dialog, so closing
  // the dialog mid-run still refreshes the list when the run ends, and
  // reopening it shows that run.
  const canImportRules =
    sessionUser !== null &&
    hasAppPermission(sessionUser, APP_PERMISSIONS.manageSystemSettings);
  // A finished run may have saved rules even when the dialog is closed or
  // its report was cleared.
  const refreshAfterImport = useCallback(() => {
    void refreshRuleSets();
  }, [refreshRuleSets]);
  const maintainerrImport = useExternalRuleImport({
    onTerminal: refreshAfterImport,
  });
  const [isMaintainerrImportOpen, setIsMaintainerrImportOpen] = useState(false);

  const refreshCatalogs = useCallback(async () => {
    try {
      const libraryList = await client.query(librariesQuery, {}).toPromise();
      if (!libraryList.error) {
        const catalogLibraries = libraryList.data?.libraries ?? [];
        setLibraries(
          catalogLibraries.map(
            (library: { id: string; name: string; facet: string }) => ({
              id: library.id,
              name: library.name,
              facet: library.facet,
            }),
          ),
        );
        setRootPaths(
          catalogLibraries.flatMap((library: { roots?: { path: string }[] }) =>
            (library.roots ?? []).map((root) => root.path),
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
          lifecycleInstanceGatesQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const next =
        (data?.lifecycleInstanceGates as LifecycleInstanceGates) ?? null;
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
          lifecycleCandidatesQuery,
          {
            ruleSetId: lifecycleFilterArgument(candidateRuleFilter),
            states: lifecycleFilterArgument(candidateStateFilter)
              ? [candidateStateFilter]
              : undefined,
            libraryId: lifecycleFilterArgument(candidateLibraryFilter),
            limit: LIFECYCLE_CANDIDATE_LIMIT,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setCandidates((data?.lifecycleCandidates as LifecycleCandidate[]) ?? []);
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
        ruleSetId: lifecycleFilterArgument(runScopeRuleSetId),
        limit: LIFECYCLE_RUN_HISTORY_LIMIT,
      };
      const [evaluation, actions] = await Promise.all([
        client
          .query(lifecycleEvaluationRunsQuery, variables, {
            requestPolicy: "network-only",
          })
          .toPromise(),
        client
          .query(lifecycleActionRunsQuery, variables, {
            requestPolicy: "network-only",
          })
          .toPromise(),
      ]);
      if (evaluation.error) throw evaluation.error;
      setEvaluationRuns(
        (evaluation.data
          ?.lifecycleEvaluationRuns as LifecycleEvaluationRun[]) ?? [],
      );
      if (actions.error) throw actions.error;
      setActionRuns(
        (actions.data?.lifecycleActionRuns as LifecycleActionRun[]) ?? [],
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
          lifecycleExclusionsQuery,
          { ruleSetId: lifecycleFilterArgument(candidateRuleFilter) },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setExclusions((data?.lifecycleExclusions as LifecycleExclusion[]) ?? []);
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

  const deleteRuleSet = async (record: LifecycleRuleSetRecord) => {
    setPendingDeleteRuleSet(record);
  };

  const confirmDeleteRuleSet = async () => {
    if (!pendingDeleteRuleSet) return;
    const record = pendingDeleteRuleSet;
    setMutatingRuleSetId(record.id);
    try {
      const { error } = await client
        .mutation(deleteLifecycleRuleSetMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.lifecycleRuleDeleted", { name: record.name }),
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
    // Create Enabled is the form's first submit button, so Enter takes it too.
    const enableOnCreate =
      (event.nativeEvent as SubmitEvent).submitter?.getAttribute("value") ===
      LIFECYCLE_CREATE_ENABLED;
    if (
      builderEditor.attached &&
      isIncompleteRuleBuilderDraft(builderBinding, ruleSetDraft)
    ) {
      setValidationResult({
        valid: false,
        errors: [t("settings.ruleBuilderIncomplete")],
      });
      // The problems are marked in the builder: show it and go to the first.
      builderEditor.setMode("simple");
      if (builderBinding.schema)
        revealFirstRuleBuilderProblem(builderBinding.schema);
      return;
    }
    if (!ruleSetDraft.name.trim() || !ruleSetDraft.regoSource.trim()) {
      setValidationResult({
        valid: false,
        errors: [t("settings.lifecycleRuleValidationRequired")],
      });
      return;
    }
    const validation = await validateDraft();
    if (!validation?.valid) {
      return;
    }
    if (
      (validation as LifecycleValidationResult).declaredActions?.destructive &&
      ruleSetDraft.maxTitlesPerRun === null
    ) {
      setValidationResult({
        ...validation,
        valid: false,
        errors: [t("error.reason.LIFECYCLE_RULE_MAX_TITLES_PER_RUN_REQUIRED")],
      });
      return;
    }

    setMutatingRuleSetId(editingRuleSetId || "new");
    let created: LifecycleRuleSetRecord | null = null;
    try {
      if (editingRuleSetId) {
        const plan = lifecycleRuleUpdatePlan(
          editingRuleSetId,
          ruleSetDraftBaseline,
          ruleSetDraft,
          builderBinding,
        );
        if (plan.source) {
          const source = await client
            .mutation(updateLifecycleRuleSourceMutation, {
              input: plan.source,
            })
            .toPromise();
          if (source.error) throw source.error;
        }
        if (plan.metadata) {
          const metadata = await client
            .mutation(updateLifecycleRuleMetadataMutation, {
              input: plan.metadata,
            })
            .toPromise();
          if (metadata.error) throw metadata.error;
        }
        setGlobalStatus(t("settings.lifecycleRuleUpdated"), {
          level: "SUCCESS",
        });
      } else {
        const { data, error } = await client
          .mutation(createLifecycleRuleSetMutation, {
            input: createLifecycleRuleSetInput(ruleSetDraft, builderBinding),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.lifecycleRuleCreated"), {
          level: "SUCCESS",
        });
        created = data?.createLifecycleRuleSet?.ruleSet ?? null;
      }
      closeEditor();
      await refreshRuleSets();
      // Rules are created disabled; enabling goes through the same control
      // as the table, so a rule that deletes files still asks first.
      if (enableOnCreate && created) setRuleState(created, "ENABLED");
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
    async (subject?: LifecycleTestSubject) => {
      const request = ++previewRequest.current;
      setPreviewing(true);
      setPreviewError(null);
      try {
        const { data, error } = await client
          .mutation(previewLifecycleRuleMutation, {
            input: lifecyclePreviewInput({
              ruleSetId: previewSource === "stored" ? previewRuleSetId : null,
              draft: previewSource === "draft" ? ruleSetDraft : undefined,
              titleIds: subject ? [subject.titleId] : undefined,
              // A whole-title pick evaluates every level of the rule; a
              // season or episode picked from the results narrows to it.
              subjectIds:
                subject && subject.subjectId !== subject.titleId
                  ? [subject.subjectId]
                  : undefined,
              libraryId: previewLibraryId,
              limit: previewLimit,
            }),
          })
          .toPromise();
        if (request !== previewRequest.current) return;
        if (error) throw error;
        setPreviewResult(data.previewLifecycleRule as LifecyclePreviewResult);
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
    async (key: LifecycleGateKey, value: boolean) => {
      setSavingSwitch(key);
      try {
        const { error } = await client
          .mutation(setLifecycleInstanceGatesMutation, {
            input: { [key]: value },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.lifecycleSwitchesUpdated"), {
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
    (key: LifecycleGateKey, value: boolean) => {
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
  const { setRuleState, armingDialog } = useLifecycleRuleState({
    onChanged: refreshAfterRuleState,
    setBusyRuleSetId: setMutatingRuleSetId,
  });

  // ── Run now ─────────────────────────────────────────────────────────

  const announceTrigger = useCallback(
    (result: LifecycleTriggerResult | null) => {
      if (!result) return;
      setGlobalStatus(
        result.started
          ? (result.message ?? t("settings.lifecycleRunStarted"))
          : t("settings.lifecycleRunNotStarted", {
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
        .mutation(runLifecycleEvaluationNowMutation, {
          ruleSetId: lifecycleFilterArgument(runScopeRuleSetId),
        })
        .toPromise();
      if (error) throw error;
      announceTrigger(
        (data?.runLifecycleEvaluationNow as LifecycleTriggerResult) ?? null,
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

  /// Acting now is always for one rule: rules act only through their own
  /// fires, so the server refuses a call that names none.
  const runActionHandlerNow = useCallback(async () => {
    const ruleSetId = lifecycleFilterArgument(runScopeRuleSetId);
    if (!ruleSetId) return;
    setActionTriggering(true);
    try {
      const { data, error } = await client
        .mutation(runLifecycleActionHandlerNowMutation, { ruleSetId })
        .toPromise();
      if (error) throw error;
      announceTrigger(
        (data?.runLifecycleActionHandlerNow as LifecycleTriggerResult) ?? null,
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
    runScopeRuleSetId,
    setGlobalStatus,
    t,
  ]);

  // ── Exclusions ──────────────────────────────────────────────────────

  const excludeCandidate = useCallback((candidate: LifecycleCandidate) => {
    setPendingExclusion({
      titleId: candidate.titleId,
      subjectKind: candidate.subjectKind,
      subjectId: candidate.subjectId,
      titleName:
        candidate.subjectKind === "title"
          ? candidate.titleName
          : `${candidate.titleName} · ${candidate.subjectLabel}`,
      /// Defaults to the rule the candidate came from rather than to a global
      /// exclusion: excluding a title from every lifecycle rule at once is a
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
        .mutation(excludeLifecycleSubjectMutation, {
          input: excludeLifecycleSubjectInput({
            titleId: pendingExclusion.titleId,
            subjectKind: pendingExclusion.subjectKind,
            subjectId: pendingExclusion.subjectId,
            ruleSetId:
              lifecycleFilterArgument(pendingExclusion.ruleSetId) ?? null,
            reason: pendingExclusion.reason,
          }),
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.lifecycleExcluded", {
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
        .mutation(removeLifecycleExclusionMutation, { id: exclusion.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        t("settings.lifecycleExclusionRemoved", {
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
      <SettingsLifecycleRulesSection
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
          setPreviewLimit(clampLifecyclePreviewLimit(limit))
        }
        runPreview={runPreview}
        previewing={previewing}
        previewResult={previewResult}
        previewError={previewError}
        instanceSwitches={
          <LifecycleInstanceSwitches
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
              <LifecycleCandidatesPanel
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
              <LifecycleExclusionsPanel
                exclusions={exclusions}
                exclusionsError={exclusionsError}
                ruleSetRecords={ruleSetRecords}
                removingExclusionId={removingExclusionId}
                removeExclusion={setPendingRemoveExclusion}
              />
            </>
          ) : section === "history" ? (
            <LifecycleRunsPanel
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
        importActions={
          canImportRules ? (
            <TextActionButton
              id="settings-lifecycle-rules-maintainerr-import"
              tone="accentBright"
              size="default"
              onClick={() => setIsMaintainerrImportOpen(true)}
              leadingIcon={<FileInput className="h-4 w-4" />}
              label={t("settings.maintainerrImportAction")}
            />
          ) : null
        }
      />
      {canImportRules ? (
        <MaintainerrRuleImportDialog
          open={isMaintainerrImportOpen}
          onOpenChange={setIsMaintainerrImportOpen}
          controller={maintainerrImport}
        />
      ) : null}
      <ConfirmDialog
        open={pendingUnpause}
        contentId="settings-lifecycle-unpause-confirm"
        title={t("settings.lifecycleUnpauseConfirmTitle")}
        description={t("settings.lifecycleUnpauseConfirmBody")}
        confirmLabel={t("settings.lifecycleUnpauseConfirmAction")}
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
        contentId="settings-lifecycle-exclude-confirm"
        title={t("settings.lifecycleExcludeTitle")}
        description={
          pendingExclusion
            ? t("settings.lifecycleExcludeBody", {
                title: pendingExclusion.titleName,
              })
            : ""
        }
        confirmLabel={t("settings.lifecycleExcludeConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={excluding}
        onConfirm={() => void confirmExclusion()}
        onCancel={() => setPendingExclusion(null)}
      >
        {pendingExclusion ? (
          <div className="space-y-3">
            <SingleSelectField
              id="settings-lifecycle-exclude-scope"
              label={t("settings.lifecycleExcludeScope")}
              value={pendingExclusion.ruleSetId}
              onValueChange={(value) =>
                setPendingExclusion((prev) =>
                  prev ? { ...prev, ruleSetId: value } : prev,
                )
              }
              options={[
                {
                  value: LIFECYCLE_FILTER_ALL,
                  label: t("settings.lifecycleExcludeScopeGlobal"),
                },
                ...ruleSetRecords.map((record) => ({
                  value: record.id,
                  label: record.name,
                })),
              ]}
            />
            <label>
              <Label className="mb-2 block">
                {t("settings.lifecycleExcludeReason")}
              </Label>
              <Input
                id="settings-lifecycle-exclude-reason"
                value={pendingExclusion.reason}
                placeholder={t("settings.lifecycleExcludeReasonPlaceholder")}
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
        contentId="settings-lifecycle-exclusion-remove-confirm"
        title={t("settings.lifecycleExclusionRemove")}
        description={
          pendingRemoveExclusion
            ? t("settings.lifecycleExclusionRemoving", {
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
            ? t("settings.lifecycleRuleDeleting", {
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
        title={t("settings.lifecycleRuleConfirmDiscardTitle")}
        description={t("settings.lifecycleRuleConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create" ||
          pendingEditorAction?.type === "copy"
            ? t("settings.lifecycleRuleCreateNew")
            : pendingEditorAction?.type === "template"
              ? t("settings.lifecycleTemplateApply")
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
