import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsRulesSection } from "@/components/views/settings/settings-rules-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { RuleSetRecord, RuleSetDraft, RuleValidationResult } from "@/lib/types/rule-sets";
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

export function SettingsRulesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [ruleSetRecords, setRuleSetRecords] = useState<RuleSetRecord[]>([]);
  const [mutatingRuleSetId, setMutatingRuleSetId] = useState<string | null>(null);
  const [editingRuleSetId, setEditingRuleSetId] = useState<string | null>(null);
  const [pendingDeleteRuleSet, setPendingDeleteRuleSet] = useState<RuleSetRecord | null>(null);
  const [ruleSetDraft, setRuleSetDraft] = useState<RuleSetDraft>(() => ({ ...RULE_SET_INITIAL_DRAFT }));
  const [validating, setValidating] = useState(false);
  const [validationResult, setValidationResult] = useState<RuleValidationResult | null>(null);

  const resetRuleSetDraft = useCallback(() => {
    setEditingRuleSetId(null);
    setRuleSetDraft(() => ({ ...RULE_SET_INITIAL_DRAFT }));
    setValidationResult(null);
  }, []);

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
            input: {
              name: payload.name,
              description: payload.description || undefined,
              regoSource: payload.regoSource,
              priority: payload.priority,
              appliedFacets: payload.appliedFacets.length > 0 ? payload.appliedFacets : undefined,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.ruleCreated"));
      }
      resetRuleSetDraft();
      await refreshRuleSets();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingRuleSetId(null);
    }
  };

  const editRuleSet = (record: RuleSetRecord) => {
    setEditingRuleSetId(record.id);
    setRuleSetDraft({
      name: record.name,
      description: record.description,
      regoSource: record.regoSource,
      enabled: record.enabled,
      priority: record.priority,
      appliedFacets: [...record.appliedFacets],
    });
    setValidationResult(null);
    setGlobalStatus(t("status.editingRule", { name: record.name }));
  };

  const deleteRuleSet = async (record: RuleSetRecord) => {
    setPendingDeleteRuleSet(record);
  };

  const toggleRuleSetEnabled = useCallback(
    async (record: RuleSetRecord) => {
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
        resetRuleSetDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingRuleSetId(null);
      setPendingDeleteRuleSet(null);
    }
  };

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
        editingRuleSetId={editingRuleSetId}
        ruleSetDraft={ruleSetDraft}
        setRuleSetDraft={setRuleSetDraft}
        submitRuleSet={submitRuleSet}
        mutatingRuleSetId={mutatingRuleSetId}
        resetRuleSetDraft={resetRuleSetDraft}
        ruleSetRecords={ruleSetRecords}
        editRuleSet={editRuleSet}
        toggleRuleSetEnabled={toggleRuleSetEnabled}
        deleteRuleSet={deleteRuleSet}
        validateDraft={validateDraft}
        validating={validating}
        validationResult={validationResult}
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
    </>
  );
}
