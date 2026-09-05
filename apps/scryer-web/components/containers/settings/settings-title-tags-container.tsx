import * as React from "react";

import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsTitleTagsSection } from "@/components/views/settings/settings-title-tags-section";
import { useTranslate } from "@/lib/context/translate-context";
import { emptyTitleTagDraft, useTitleTagsManager } from "@/lib/hooks/use-title-tags-manager";

type SettingsTitleTagsContainerProps = {
  /**
   * Deciding which tags exist is catalog configuration, so the write controls
   * follow the same permission the delay-profiles page uses. The list itself is
   * readable by anyone who can reach the page.
   */
  canManageCatalogSettings: boolean;
};

export function SettingsTitleTagsContainer({
  canManageCatalogSettings,
}: SettingsTitleTagsContainerProps) {
  const manager = useTitleTagsManager();
  const t = useTranslate();
  const [isEditorOpen, setIsEditorOpen] = React.useState(false);
  const [editorMode, setEditorMode] = React.useState<"create" | "edit">("create");
  const [pendingDeleteId, setPendingDeleteId] = React.useState<string | null>(null);

  const { resetDraft, setDraft, loadDefinitionById } = manager;

  const openCreateEditor = React.useCallback(() => {
    resetDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
  }, [resetDraft]);

  const openEditEditor = React.useCallback(
    (definitionId: string) => {
      loadDefinitionById(definitionId);
      setEditorMode("edit");
      setIsEditorOpen(true);
    },
    [loadDefinitionById],
  );

  const closeEditor = React.useCallback(() => {
    setIsEditorOpen(false);
    setEditorMode("create");
    setDraft(emptyTitleTagDraft());
  }, [setDraft]);

  const handleSave = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      const saved = await manager.saveDefinition(event);
      if (saved) {
        setIsEditorOpen(false);
        setEditorMode("create");
      }
    },
    [manager],
  );

  // Delete confirms with the affected title count first: the label is stripped
  // from every title carrying it, and rules naming it simply stop matching.
  const pendingDeleteDefinition = React.useMemo(
    () =>
      pendingDeleteId
        ? (manager.definitions.find(
            (definition) => definition.id === pendingDeleteId,
          ) ?? null)
        : null,
    [manager.definitions, pendingDeleteId],
  );

  const confirmDelete = React.useCallback(async () => {
    if (!pendingDeleteId) {
      return;
    }
    const deleted = await manager.deleteDefinition(pendingDeleteId);
    if (deleted && editorMode === "edit" && manager.draft.id === pendingDeleteId) {
      closeEditor();
    }
    setPendingDeleteId(null);
  }, [closeEditor, editorMode, manager, pendingDeleteId]);

  return (
    <>
      <SettingsTitleTagsSection
        loading={manager.loading}
        loadError={manager.loadError}
        saving={manager.saving}
        definitions={manager.definitions}
        draft={manager.draft}
        setDraft={manager.setDraft}
        renameWarning={manager.renameWarning}
        onDismissRenameWarning={manager.dismissRenameWarning}
        saveDefinition={handleSave}
        deleteDefinition={(definitionId) => setPendingDeleteId(definitionId)}
        loadDefinitionById={openEditEditor}
        resetDraft={closeEditor}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        startCreateDefinition={openCreateEditor}
        canManageCatalogSettings={canManageCatalogSettings}
      />
      <ConfirmDialog
        open={pendingDeleteDefinition !== null}
        title={t("settings.titleTagDeleteConfirmTitle")}
        description={t("settings.titleTagDeleteConfirm", {
          label: pendingDeleteDefinition?.label ?? "",
          count: pendingDeleteDefinition?.titleCount ?? 0,
        })}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-title-tag-delete-confirm"
        isBusy={manager.saving}
        onConfirm={confirmDelete}
        onCancel={() => setPendingDeleteId(null)}
      />
    </>
  );
}
