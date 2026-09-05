import { type FormEvent, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsRequestRulesSection,
  type RequestQualityProfileOption,
} from "@/components/views/settings/settings-request-rules-section";
import type { RequestRuleTemplate } from "@/lib/constants/request-rule-templates";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { LibraryRecord } from "@/lib/types";
import type {
  RequestEvaluationMode,
  RequestRuleDecisionRecord,
  RequestRuleInstanceGates,
  RequestRulePreviewResult,
  RequestRulePreviewSample,
  RequestRulePreviewSource,
  RequestRuleSetDetail,
  RequestRuleSetDraft,
  RequestRuleSetRecord,
  RequestRuleUserOption,
  RequestRuleValidationResult,
} from "@/lib/types/request-rule-sets";
import {
  REQUEST_DECISION_LIMIT,
  REQUEST_FILTER_ALL,
  applyRequestersToSource,
  clampRequestLeaseDays,
  copyRequestRuleDraft,
  createRequestRuleSetInput,
  initialRequestRuleDraft,
  requestFilterArgument,
  requestRuleDraftFromDetail,
  updateRequestRuleMatcherInput,
  updateRequestRuleMetadataInput,
} from "@/lib/utils/request-rule-sets";
import {
  librariesQuery,
  qualityProfileOptionsQuery,
  requestRuleDecisionsQuery,
  requestRuleInstanceGatesQuery,
  requestRuleSetQuery,
  requestRuleSetsQuery,
  searchMetadataQuery,
  usersQuery,
} from "@/lib/graphql/queries";
import {
  createRequestRuleSetMutation,
  deleteRequestRuleSetMutation,
  previewRequestRuleMutation,
  setRequestRuleInstanceGatesMutation,
  setRequestRuleModeMutation,
  updateRequestRuleMatcherMutation,
  updateRequestRuleMetadataMutation,
  validateRequestRuleMutation,
} from "@/lib/graphql/mutations";

const PREVIEW_TITLE_SEARCH_LIMIT = 8;

/// The sample a preview starts from. Everything about it is a choice the author
/// makes; nothing is guessed from the rule, because a rule that only fires for
/// one library should still be previewable against another.
function initialPreviewSample(): RequestRulePreviewSample {
  return {
    userId: "",
    libraryId: "",
    externalIds: [],
    titleLabel: "",
    qualityProfileId: "",
    monitorType: "MONITORED",
    leaseForever: true,
    leaseDays: 30,
  };
}

type PendingEditorAction =
  | { type: "create" }
  | { type: "copy"; record: RequestRuleSetRecord }
  | { type: "edit"; record: RequestRuleSetRecord }
  | { type: "template"; template: RequestRuleTemplate }
  | { type: "close" }
  | null;

export function SettingsRequestRulesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  const [ruleSetRecords, setRuleSetRecords] = useState<RequestRuleSetRecord[]>([]);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [qualityProfiles, setQualityProfiles] = useState<
    RequestQualityProfileOption[]
  >([]);
  const [users, setUsers] = useState<RequestRuleUserOption[]>([]);

  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(null);
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteRuleSet, setPendingDeleteRuleSet] =
    useState<RequestRuleSetRecord | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingEditorAction>(null);
  const [ruleSetDraft, setRuleSetDraft] = useState<RequestRuleSetDraft>(
    initialRequestRuleDraft,
  );
  const [ruleSetDraftBaseline, setRuleSetDraftBaseline] =
    useState<RequestRuleSetDraft>(initialRequestRuleDraft);
  const [validating, setValidating] = useState(false);
  const [validationResult, setValidationResult] =
    useState<RequestRuleValidationResult | null>(null);

  const [previewSource, setPreviewSource] =
    useState<RequestRulePreviewSource>("stored");
  const [previewRuleSetId, setPreviewRuleSetId] = useState("");
  const [previewSample, setPreviewSample] = useState<RequestRulePreviewSample>(
    initialPreviewSample,
  );
  const [previewing, setPreviewing] = useState(false);
  const [previewResult, setPreviewResult] =
    useState<RequestRulePreviewResult | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);

  const [gates, setGates] = useState<RequestRuleInstanceGates | null>(null);
  /// True once the gate query has been refused. Reading it needs
  /// system-settings management, which is a *different* permission from the one
  /// this page otherwise requires, so a catalog administrator must still get a
  /// working page rather than a failed section.
  const [gatesLocked, setGatesLocked] = useState(false);
  const [savingGate, setSavingGate] = useState(false);

  const [decisions, setDecisions] = useState<RequestRuleDecisionRecord[]>([]);
  const [decisionsLoading, setDecisionsLoading] = useState(false);
  const [decisionsError, setDecisionsError] = useState<string | null>(null);
  const [decisionOutcomeFilter, setDecisionOutcomeFilter] =
    useState(REQUEST_FILTER_ALL);

  const isDraftDirty =
    JSON.stringify(ruleSetDraft) !== JSON.stringify(ruleSetDraftBaseline);

  const closeEditor = useCallback(() => {
    const next = initialRequestRuleDraft();
    setIsEditorOpen(false);
    setEditingRuleSetId(null);
    setRuleSetDraft(next);
    setRuleSetDraftBaseline(next);
    setValidationResult(null);
  }, []);

  const openCreateEditor = useCallback(() => {
    const next = initialRequestRuleDraft();
    setEditingRuleSetId(null);
    setRuleSetDraft(next);
    setRuleSetDraftBaseline(next);
    setValidationResult(null);
    setIsEditorOpen(true);
  }, []);

  const fetchRuleSetDetail = useCallback(
    async (id: string): Promise<RequestRuleSetDetail | null> => {
      try {
        const { data, error } = await client
          .query(requestRuleSetQuery, { id })
          .toPromise();
        if (error) throw error;
        const detail = (data?.requestRuleSet as RequestRuleSetDetail | null) ?? null;
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
    async (record: RequestRuleSetRecord) => {
      const detail = await fetchRuleSetDetail(record.id);
      if (!detail) return;
      const next = requestRuleDraftFromDetail(detail);
      setEditingRuleSetId(record.id);
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(next);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [fetchRuleSetDetail],
  );

  const openCopyEditor = useCallback(
    async (record: RequestRuleSetRecord) => {
      const detail = await fetchRuleSetDetail(record.id);
      if (!detail) return;
      const next = copyRequestRuleDraft(detail);
      setEditingRuleSetId(null);
      setRuleSetDraft(next);
      setRuleSetDraftBaseline(next);
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [fetchRuleSetDetail],
  );

  /// Load a starter template into the create-rule editor. A template prefills
  /// the name, the description and the matcher, and then stops: it never
  /// creates a rule, and the draft it leaves behind is dirty, so the operator
  /// has to read it and press create themselves. `libraryIds` stays empty
  /// because which libraries a rule covers is an instance-specific decision no
  /// template can make, and the named people in a person-targeted template are
  /// placeholders the user picker replaces.
  const openTemplateEditor = useCallback(
    (template: RequestRuleTemplate) => {
      const next: RequestRuleSetDraft = {
        ...initialRequestRuleDraft(),
        name: template.name,
        description: t(template.descriptionKey),
        regoSource: template.regoSource,
      };
      setEditingRuleSetId(null);
      setRuleSetDraft(next);
      /// Baseline stays at the empty draft on purpose: a freshly applied
      /// template counts as unsaved work, so navigating away from it asks.
      setRuleSetDraftBaseline(initialRequestRuleDraft());
      setValidationResult(null);
      setIsEditorOpen(true);
    },
    [t],
  );

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditRuleSet = useCallback(
    (record: RequestRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        void openEditEditor(record);
        return;
      }
      setPendingEditorAction({ type: "edit", record });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCopyRuleSet = useCallback(
    (record: RequestRuleSetRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        void openCopyEditor(record);
        return;
      }
      setPendingEditorAction({ type: "copy", record });
    },
    [isDraftDirty, isEditorOpen, openCopyEditor],
  );

  const requestApplyTemplate = useCallback(
    (template: RequestRuleTemplate) => {
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

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "copy") {
      void openCopyEditor(pendingEditorAction.record);
    } else if (pendingEditorAction.type === "edit") {
      void openEditEditor(pendingEditorAction.record);
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

  const refreshRuleSets = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(requestRuleSetsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setRuleSetRecords((data?.requestRuleSets as RequestRuleSetRecord[]) ?? []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);

  /// Libraries, quality profiles and accounts. Listing users needs a permission
  /// this page does not otherwise require, so a refusal leaves the picker empty
  /// and says so rather than failing the section.
  const refreshCatalogs = useCallback(async () => {
    try {
      const [libraryList, profiles, userList] = await Promise.all([
        client.query(librariesQuery, {}).toPromise(),
        client.query(qualityProfileOptionsQuery, {}).toPromise(),
        client.query(usersQuery, {}).toPromise(),
      ]);
      if (!libraryList.error) {
        setLibraries((libraryList.data?.libraries ?? []) as LibraryRecord[]);
      }
      if (!profiles.error) {
        setQualityProfiles(profiles.data?.qualityProfileSettings?.profiles ?? []);
      }
      setUsers(
        userList.error
          ? []
          : ((userList.data?.users ?? []) as Array<{
              id: string;
              username: string;
            }>).map((user) => ({ id: user.id, username: user.username })),
      );
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);

  const refreshGates = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(requestRuleInstanceGatesQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      const next =
        (data?.requestRuleInstanceGates as RequestRuleInstanceGates) ?? null;
      setGates(next);
      setGatesLocked(next === null);
    } catch {
      setGates(null);
      setGatesLocked(true);
    }
  }, [client]);

  const refreshDecisions = useCallback(async () => {
    setDecisionsLoading(true);
    setDecisionsError(null);
    try {
      const { data, error } = await client
        .query(
          requestRuleDecisionsQuery,
          {
            limit: REQUEST_DECISION_LIMIT,
            outcome: requestFilterArgument(decisionOutcomeFilter),
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setDecisions(
        (data?.requestRuleDecisions as RequestRuleDecisionRecord[]) ?? [],
      );
    } catch (error) {
      setDecisions([]);
      setDecisionsError(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setDecisionsLoading(false);
    }
  }, [client, decisionOutcomeFilter, t]);

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
    void refreshDecisions();
  }, [refreshDecisions]);

  const validateDraft =
    useCallback(async (): Promise<RequestRuleValidationResult | null> => {
      if (!ruleSetDraft.regoSource.trim()) return null;
      setValidating(true);
      setValidationResult(null);
      try {
        const { data, error } = await client
          .mutation(validateRequestRuleMutation, {
            input: { regoSource: ruleSetDraft.regoSource },
          })
          .toPromise();
        if (error) throw error;
        const result = data.validateRequestRule as RequestRuleValidationResult;
        setValidationResult(result);
        return result;
      } catch (error) {
        /// The API's refusals — the person-targeting one especially — are the
        /// message the author needs, so they are surfaced verbatim rather than
        /// replaced with a generic failure.
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

  const applyRequesters = useCallback(
    (usernames: string[]) => {
      setRuleSetDraft((prev) => {
        const next = applyRequestersToSource(prev.regoSource, usernames);
        return next === null ? prev : { ...prev, regoSource: next };
      });
      setValidationResult(null);
    },
    [],
  );

  const confirmDeleteRuleSet = useCallback(async () => {
    if (!pendingDeleteRuleSet) return;
    const record = pendingDeleteRuleSet;
    setMutatingRuleSetId(record.id);
    try {
      const { error } = await client
        .mutation(deleteRequestRuleSetMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.requestRuleDeleted", { name: record.name }));
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
  }, [
    client,
    closeEditor,
    editingRuleSetId,
    pendingDeleteRuleSet,
    previewRuleSetId,
    refreshRuleSets,
    setGlobalStatus,
    t,
  ]);

  /// Editing a saved rule writes both halves: the matcher (a new immutable
  /// revision) and the metadata (name, description, library scope), which the
  /// API keeps on separate mutations because only one of them versions.
  const submitRuleSet = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!ruleSetDraft.name.trim() || !ruleSetDraft.regoSource.trim()) {
      setValidationResult({
        valid: false,
        errors: [t("settings.requestRuleValidationRequired")],
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
          .mutation(updateRequestRuleMatcherMutation, {
            input: updateRequestRuleMatcherInput(editingRuleSetId, ruleSetDraft),
          })
          .toPromise();
        if (matcher.error) throw matcher.error;
        const metadata = await client
          .mutation(updateRequestRuleMetadataMutation, {
            input: updateRequestRuleMetadataInput(editingRuleSetId, ruleSetDraft),
          })
          .toPromise();
        if (metadata.error) throw metadata.error;
        setGlobalStatus(t("settings.requestRuleUpdated"));
      } else {
        const { error } = await client
          .mutation(createRequestRuleSetMutation, {
            input: createRequestRuleSetInput(ruleSetDraft),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.requestRuleCreated"));
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

  const setRuleMode = useCallback(
    async (record: RequestRuleSetRecord, mode: RequestEvaluationMode) => {
      if (mode === record.evaluationMode) return;
      setMutatingRuleSetId(record.id);
      try {
        const { error } = await client
          .mutation(setRequestRuleModeMutation, {
            input: { ruleSetId: record.id, mode },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.requestRuleModeUpdated", { name: record.name }),
        );
        await refreshRuleSets();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingRuleSetId(null);
      }
    },
    [client, refreshRuleSets, setGlobalStatus, t],
  );

  /// The gate write is never optimistic: the mutation returns the gate, but the
  /// panel refetches instead, so what it shows is what the server holds even
  /// when another operator moved it in the same moment.
  const setGate = useCallback(
    async (enabled: boolean) => {
      setSavingGate(true);
      try {
        const { error } = await client
          .mutation(setRequestRuleInstanceGatesMutation, {
            input: { evaluationEnabled: enabled },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.requestGateUpdated"));
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setSavingGate(false);
        await refreshGates();
      }
    },
    [client, refreshGates, setGlobalStatus, t],
  );

  const searchPreviewTitles = useCallback(
    async (query: string, facet: string): Promise<MetadataTvdbSearchItem[]> => {
      try {
        const { data, error } = await client
          .query(searchMetadataQuery, {
            query,
            type: facet,
            limit: PREVIEW_TITLE_SEARCH_LIMIT,
          })
          .toPromise();
        if (error) throw error;
        return (data?.searchMetadata ?? []) as MetadataTvdbSearchItem[];
      } catch {
        /// A failed lookup is not a failed preview: the author can still pick a
        /// different title, so this stays quiet rather than raising a banner.
        return [];
      }
    },
    [client],
  );

  const runPreview = useCallback(async () => {
    setPreviewing(true);
    setPreviewError(null);
    try {
      const { data, error } = await client
        .mutation(previewRequestRuleMutation, {
          input: {
            /// Exactly one of the two: sending a stored rule set *and* a draft
            /// source together is refused, because it does not say which one
            /// the author meant to preview.
            ruleSetId: previewSource === "stored" ? previewRuleSetId : null,
            regoSource: previewSource === "draft" ? ruleSetDraft.regoSource : null,
            sample: {
              userId: previewSample.userId,
              libraryId: previewSample.libraryId,
              externalIds: previewSample.externalIds,
              qualityProfileId: previewSample.qualityProfileId || null,
              monitorType: previewSample.monitorType || null,
              leaseForever: previewSample.leaseForever ? true : null,
              leaseDays: previewSample.leaseForever
                ? null
                : clampRequestLeaseDays(previewSample.leaseDays),
            },
          },
        })
        .toPromise();
      if (error) throw error;
      setPreviewResult(data.previewRequestRule as RequestRulePreviewResult);
    } catch (error) {
      setPreviewResult(null);
      setPreviewError(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setPreviewing(false);
    }
  }, [
    client,
    previewRuleSetId,
    previewSample,
    previewSource,
    ruleSetDraft.regoSource,
    t,
  ]);

  return (
    <>
      <SettingsRequestRulesSection
        isEditorOpen={isEditorOpen}
        editingRuleSetId={editingRuleSetId}
        ruleSetDraft={ruleSetDraft}
        setRuleSetDraft={setRuleSetDraft}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={requestCloseEditor}
        startCreateRuleSet={requestCreateEditor}
        applyTemplate={requestApplyTemplate}
        ruleSetRecords={ruleSetRecords}
        libraries={libraries}
        qualityProfiles={qualityProfiles}
        users={users}
        copyRuleSet={requestCopyRuleSet}
        editRuleSet={requestEditRuleSet}
        deleteRuleSet={setPendingDeleteRuleSet}
        validateDraft={validateDraft}
        validating={validating}
        validationResult={validationResult}
        applyRequesters={applyRequesters}
        setRuleMode={(record, mode) => void setRuleMode(record, mode)}
        gates={gates}
        gatesLocked={gatesLocked}
        savingGate={savingGate}
        setGate={(enabled) => void setGate(enabled)}
        previewSource={previewSource}
        setPreviewSource={setPreviewSource}
        previewRuleSetId={previewRuleSetId}
        setPreviewRuleSetId={setPreviewRuleSetId}
        previewSample={previewSample}
        setPreviewSample={setPreviewSample}
        searchPreviewTitles={searchPreviewTitles}
        runPreview={runPreview}
        previewing={previewing}
        previewResult={previewResult}
        previewError={previewError}
        decisions={decisions}
        decisionsLoading={decisionsLoading}
        decisionsError={decisionsError}
        decisionOutcomeFilter={decisionOutcomeFilter}
        setDecisionOutcomeFilter={setDecisionOutcomeFilter}
        refreshDecisions={() => void refreshDecisions()}
      />
      <ConfirmDialog
        open={pendingDeleteRuleSet !== null}
        contentId="settings-request-rule-delete-confirm"
        title={t("label.delete")}
        description={
          pendingDeleteRuleSet
            ? t("settings.requestRuleDeleting", {
                name: pendingDeleteRuleSet.name,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingRuleSetId !== null}
        onConfirm={() => void confirmDeleteRuleSet()}
        onCancel={() => setPendingDeleteRuleSet(null)}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        contentId="settings-request-rule-discard-confirm"
        confirmButtonId="settings-request-rule-discard-confirm-apply"
        cancelButtonId="settings-request-rule-discard-confirm-cancel"
        title={t("settings.requestRuleConfirmDiscardTitle")}
        description={t("settings.requestRuleConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create" ||
          pendingEditorAction?.type === "copy"
            ? t("settings.requestRuleCreateNew")
            : pendingEditorAction?.type === "template"
              ? t("settings.requestTemplateApply")
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
