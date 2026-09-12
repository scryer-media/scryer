import * as React from "react";
import { useClient } from "urql";

import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  createTitleTagDefinitionMutation,
  deleteTitleTagDefinitionMutation,
  updateTitleTagDefinitionMutation,
} from "@/lib/graphql/mutations";
import {
  fromTitleTagDefinitionPayload,
  useTitleTagDefinitions,
} from "@/lib/hooks/use-title-tag-definitions";
import type {
  TitleTagDefinition,
  TitleTagDefinitionDraft,
  TitleTagRewriteCounts,
} from "@/lib/types/title-tags";
import {
  EMPTY_TITLE_TAG_REWRITE_COUNTS,
  formatTitleTagRenameSummary,
  formatTitleTagRenameWarning,
  normalizeTitleTagLabel,
  titleTagLabelErrorKey,
} from "@/lib/utils/title-tags";

function countsFromPayload(
  payload: Partial<TitleTagRewriteCounts> | null | undefined,
): TitleTagRewriteCounts {
  return { ...EMPTY_TITLE_TAG_REWRITE_COUNTS, ...(payload ?? {}) };
}

export function emptyTitleTagDraft(): TitleTagDefinitionDraft {
  return { id: "", label: "", description: "" };
}

/**
 * Registry CRUD for the Settings > Tags section, shaped like
 * `useDelayProfilesManager`: the container owns the confirm dialogs and the
 * editor's open/closed state, this hook owns the network and the list.
 */
export function useTitleTagsManager() {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const { definitions, loading, error, reload } = useTitleTagDefinitions();
  const [saving, setSaving] = React.useState(false);
  const [draft, setDraft] = React.useState<TitleTagDefinitionDraft>(emptyTitleTagDraft);
  // The rename warning outlives the save, because it is the one piece of the
  // outcome the operator has to act on: rule sources still name the old label.
  const [renameWarning, setRenameWarning] = React.useState<string | null>(null);

  const resetDraft = React.useCallback(() => {
    setDraft(emptyTitleTagDraft());
  }, []);

  const loadDefinitionById = React.useCallback(
    (definitionId: string) => {
      const found = definitions.find((definition) => definition.id === definitionId);
      if (found) {
        setDraft({
          id: found.id,
          label: found.label,
          description: found.description ?? "",
        });
      }
    },
    [definitions],
  );

  const saveDefinition = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      event?.preventDefault();
      const labelErrorKey = titleTagLabelErrorKey(draft.label);
      if (labelErrorKey) {
        showStatus(t(labelErrorKey));
        return false;
      }
      const label = normalizeTitleTagLabel(draft.label);
      const description = draft.description.trim();
      const existing: TitleTagDefinition | undefined = draft.id
        ? definitions.find((definition) => definition.id === draft.id)
        : undefined;
      const previousLabel = existing?.label ?? label;

      setSaving(true);
      setRenameWarning(null);
      try {
        const result = draft.id
          ? await client
              .mutation(updateTitleTagDefinitionMutation, {
                input: { id: draft.id, label, description: description || null },
              })
              .toPromise()
          : await client
              .mutation(createTitleTagDefinitionMutation, {
                input: { label, description: description || null },
              })
              .toPromise();
        if (result.error) {
          showStatus(result.error.message || t("settings.titleTagSaveError"));
          return false;
        }
        const payload =
          result.data?.updateTitleTagDefinition ?? result.data?.createTitleTagDefinition;
        if (!payload) {
          showStatus(t("settings.titleTagSaveError"));
          return false;
        }
        const saved = fromTitleTagDefinitionPayload(payload.definition);
        const counts = countsFromPayload(payload.counts);
        await reload();
        resetDraft();

        const renamed = Boolean(draft.id) && previousLabel !== saved.label;
        if (renamed) {
          setRenameWarning(formatTitleTagRenameWarning(counts, previousLabel, t));
          showStatus(formatTitleTagRenameSummary(counts, saved.label, t));
        } else {
          showStatus(t("settings.titleTagSaved"));
        }
        return true;
      } catch {
        showStatus(t("settings.titleTagSaveError"));
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, definitions, draft, reload, resetDraft, showStatus, t],
  );

  const deleteDefinition = React.useCallback(
    async (definitionId: string) => {
      setSaving(true);
      setRenameWarning(null);
      try {
        const result = await client
          .mutation(deleteTitleTagDefinitionMutation, { id: definitionId })
          .toPromise();
        if (result.error) {
          showStatus(result.error.message || t("settings.titleTagDeleteError"));
          return false;
        }
        const payload = result.data?.deleteTitleTagDefinition;
        const counts = countsFromPayload(payload?.counts);
        await reload();
        if (draft.id === definitionId) {
          resetDraft();
        }
        showStatus(
          t("settings.titleTagDeleted", {
            label: payload?.label ?? "",
            count: counts.titles,
          }),
        );
        return true;
      } catch {
        showStatus(t("settings.titleTagDeleteError"));
        return false;
      } finally {
        setSaving(false);
      }
    },
    [client, draft.id, reload, resetDraft, showStatus, t],
  );

  return {
    definitions,
    loading,
    loadError: error,
    saving,
    draft,
    setDraft,
    renameWarning,
    dismissRenameWarning: React.useCallback(() => setRenameWarning(null), []),
    saveDefinition,
    deleteDefinition,
    loadDefinitionById,
    resetDraft,
    refreshDefinitions: reload,
  };
}
