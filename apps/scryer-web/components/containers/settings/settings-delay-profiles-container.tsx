import { SettingsDelayProfilesSection } from "@/components/views/settings/settings-delay-profiles-section";
import { useDelayProfilesManager } from "@/lib/hooks/use-delay-profiles-manager";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import * as React from "react";
import type { DelayProfileDraft } from "@/lib/types/delay-profiles";
import { useTranslate } from "@/lib/context/translate-context";

type PendingDelayProfileEditorAction =
  | { type: "create" }
  | { type: "edit"; profileId: string }
  | { type: "close" }
  | null;

function cloneDelayProfileDraft(draft: DelayProfileDraft): DelayProfileDraft {
  return {
    ...draft,
    applies_to_facets: [...draft.applies_to_facets],
    tags: [...draft.tags],
    library_ids: [...draft.library_ids],
  };
}

export function SettingsDelayProfilesContainer() {
  const manager = useDelayProfilesManager();
  const t = useTranslate();
  const [isEditorOpen, setIsEditorOpen] = React.useState(false);
  const [editorMode, setEditorMode] = React.useState<"create" | "edit">(
    "create",
  );
  const [pendingEditorAction, setPendingEditorAction] =
    React.useState<PendingDelayProfileEditorAction>(null);
  const [pendingDeleteProfileId, setPendingDeleteProfileId] = React.useState<
    string | null
  >(null);
  const [draftBaseline, setDraftBaseline] = React.useState<DelayProfileDraft>(
    () => cloneDelayProfileDraft(manager.draft),
  );
  const [awaitingBaselineSync, setAwaitingBaselineSync] = React.useState(false);

  React.useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }
    setDraftBaseline(cloneDelayProfileDraft(manager.draft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, manager.draft]);

  const isDraftDirty =
    JSON.stringify(manager.draft) !== JSON.stringify(draftBaseline);

  const openCreateEditor = React.useCallback(() => {
    manager.resetDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [manager]);

  const openEditEditor = React.useCallback(
    (profileId: string) => {
      manager.loadProfileById(profileId);
      setEditorMode("edit");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    },
    [manager],
  );

  const requestCreateEditor = React.useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditProfile = React.useCallback(
    (profileId: string) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(profileId);
        return;
      }
      setPendingEditorAction({ type: "edit", profileId });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCloseEditor = React.useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, manager]);

  const confirmPendingEditorAction = React.useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.profileId);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
    }
    setPendingEditorAction(null);
  }, [manager, openCreateEditor, openEditEditor, pendingEditorAction]);

  const handleSaveProfile = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      const saved = await manager.saveProfile(event);
      if (saved) {
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    },
    [manager],
  );

  const confirmDeleteProfile = React.useCallback(async () => {
    if (!pendingDeleteProfileId) return;
    const deleted = await manager.deleteProfile(pendingDeleteProfileId);
    if (
      deleted &&
      editorMode === "edit" &&
      manager.draft.id === pendingDeleteProfileId
    ) {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
    }
    setPendingDeleteProfileId(null);
  }, [editorMode, manager, pendingDeleteProfileId]);

  return (
    <>
      <SettingsDelayProfilesSection
        loading={manager.loading}
        saving={manager.saving}
        profiles={manager.profiles}
        libraries={manager.libraries}
        parseError={manager.parseError}
        draft={manager.draft}
        setDraft={manager.setDraft}
        saveProfile={handleSaveProfile}
        deleteProfile={(profileId) => setPendingDeleteProfileId(profileId)}
        loadProfileById={requestEditProfile}
        resetDraft={requestCloseEditor}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        startCreateProfile={requestCreateEditor}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.delayProfileConfirmDiscardTitle")}
        description={t("settings.delayProfileConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.delayProfileCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={manager.saving}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteProfileId !== null}
        contentId="settings-delay-profile-delete-dialog"
        confirmButtonId="settings-delay-profile-delete-confirm"
        cancelButtonId="settings-delay-profile-delete-cancel"
        title={t("label.delete")}
        description={t("settings.delayProfileDeleteConfirm")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={manager.saving}
        onConfirm={confirmDeleteProfile}
        onCancel={() => setPendingDeleteProfileId(null)}
      />
    </>
  );
}
