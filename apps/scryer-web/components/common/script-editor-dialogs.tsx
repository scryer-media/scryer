import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useTranslate } from "@/lib/context/translate-context";
import type { ScriptEditorDialogsState } from "@/lib/hooks/use-script-editor";

/** Element ids for the confirmations, so each page keeps its own. */
export type ScriptEditorDialogIds = {
  deleteConfirm?: string;
  inlineShellContent: string;
  inlineShellAccept: string;
  inlineShellCancel: string;
};

/**
 * The confirmations around a script editor: delete, the inline-shell
 * acknowledgement, and discarding unsaved changes.
 */
export function ScriptEditorDialogs({
  state,
  ids,
}: {
  state: ScriptEditorDialogsState;
  ids: ScriptEditorDialogIds;
}) {
  const t = useTranslate();
  const {
    pendingDeleteScript,
    confirmDeleteScript,
    cancelDeleteScript,
    pendingEditorAction,
    confirmPendingEditorAction,
    cancelPendingEditorAction,
    pendingInlineShellAction,
    confirmPendingInlineShellAction,
    cancelPendingInlineShellAction,
    mutatingScriptId,
  } = state;

  return (
    <>
      <ConfirmDialog
        open={pendingDeleteScript !== null}
        title={t("label.delete")}
        description={
          pendingDeleteScript ? t("status.deletingRule", { name: pendingDeleteScript.name }) : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId={ids.deleteConfirm}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmDeleteScript}
        onCancel={cancelDeleteScript}
      />
      <ConfirmDialog
        open={pendingInlineShellAction !== null}
        title={t("settings.pp.inlineShellConfirmTitle")}
        description={t("settings.pp.inlineShellConfirmDescription")}
        confirmLabel={t("settings.pp.inlineShellConfirm")}
        cancelLabel={t("label.cancel")}
        contentId={ids.inlineShellContent}
        confirmButtonId={ids.inlineShellAccept}
        cancelButtonId={ids.inlineShellCancel}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmPendingInlineShellAction}
        onCancel={cancelPendingInlineShellAction}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.pp.confirmDiscardTitle")}
        description={t("settings.pp.confirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.pp.createNewScript")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={cancelPendingEditorAction}
      />
    </>
  );
}
