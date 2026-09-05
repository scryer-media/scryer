import * as React from "react";
import { AlertTriangle, Edit, Plus, Trash2 } from "lucide-react";

import { AddNewButton } from "@/components/common/add-new-button";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleTagDefinition, TitleTagDefinitionDraft } from "@/lib/types/title-tags";
import { selectorId } from "@/lib/utils/dom-ids";

type SettingsTitleTagsSectionProps = {
  loading: boolean;
  loadError: boolean;
  saving: boolean;
  definitions: TitleTagDefinition[];
  draft: TitleTagDefinitionDraft;
  setDraft: React.Dispatch<React.SetStateAction<TitleTagDefinitionDraft>>;
  renameWarning: string | null;
  onDismissRenameWarning: () => void;
  saveDefinition: (event?: React.FormEvent<HTMLFormElement>) => void;
  deleteDefinition: (definitionId: string) => void;
  loadDefinitionById: (definitionId: string) => void;
  resetDraft: () => void;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateDefinition: () => void;
  /**
   * Whether this viewer may change the vocabulary. Deciding which tags exist is
   * catalog configuration, exactly like a delay profile; deciding which titles
   * carry them is title management and is checked per library elsewhere. A
   * viewer without the permission still reads the list.
   */
  canManageCatalogSettings: boolean;
};

const TAG_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const TAG_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const TAG_PANEL_TITLE_CLASS = "text-[15px] font-semibold text-[var(--scry-ink2)]";
const TAG_PANEL_BODY_CLASS = "p-4 sm:p-5";
const TAG_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

export function SettingsTitleTagsSection({
  loading,
  loadError,
  saving,
  definitions,
  draft,
  setDraft,
  renameWarning,
  onDismissRenameWarning,
  saveDefinition,
  deleteDefinition,
  loadDefinitionById,
  resetDraft,
  isEditorOpen,
  editorMode,
  startCreateDefinition,
  canManageCatalogSettings,
}: SettingsTitleTagsSectionProps) {
  const t = useTranslate();
  const isEditing = editorMode === "edit";

  function updateField<K extends keyof TitleTagDefinitionDraft>(
    field: K,
    value: TitleTagDefinitionDraft[K],
  ) {
    setDraft((previous) => ({ ...previous, [field]: value }));
  }

  return (
    <div id="settings-title-tags-section" className="space-y-4 text-sm">
      {loadError ? (
        <div className="rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]">
          {t("settings.titleTagsLoadError")}
        </div>
      ) : null}

      {/* A rename rewrites titles and delay profiles inside one transaction, but
          rule revisions are immutable. The counts name what still points at the
          old label so the operator can go and fix those rules by hand. */}
      {renameWarning ? (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-[12px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm text-[var(--scry-warning-text)]"
        >
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1">{renameWarning}</span>
          <button
            id="settings-title-tag-rename-warning-dismiss"
            type="button"
            onClick={onDismissRenameWarning}
            className="shrink-0 text-xs font-semibold underline"
          >
            {t("label.dismiss")}
          </button>
        </div>
      ) : null}

      <section className={TAG_PANEL_CLASS}>
        <div className={TAG_PANEL_HEADER_CLASS}>
          <h2 className={TAG_PANEL_TITLE_CLASS}>{t("settings.titleTagsExisting")}</h2>
          <p className={`mt-1 text-[12.5px] ${TAG_MUTED_TEXT_CLASS}`}>
            {t("settings.titleTagsHelp")}
          </p>
        </div>
        <div>
          {loading ? (
            <p className={`${TAG_PANEL_BODY_CLASS} text-sm ${TAG_MUTED_TEXT_CLASS}`}>
              {t("label.loading")}
            </p>
          ) : definitions.length === 0 ? (
            <p className={`${TAG_PANEL_BODY_CLASS} text-sm ${TAG_MUTED_TEXT_CLASS}`}>
              {t("settings.titleTagsNone")}
            </p>
          ) : (
            <div className="overflow-hidden">
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader>
                  <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                    <TableHead className={`w-[26%] font-semibold ${TAG_MUTED_TEXT_CLASS}`}>
                      {t("settings.titleTagLabelLabel")}
                    </TableHead>
                    <TableHead className={`font-semibold ${TAG_MUTED_TEXT_CLASS}`}>
                      {t("settings.titleTagDescriptionLabel")}
                    </TableHead>
                    <TableHead
                      className={`w-28 text-center font-semibold ${TAG_MUTED_TEXT_CLASS}`}
                    >
                      {t("settings.titleTagTitleCount")}
                    </TableHead>
                    {canManageCatalogSettings ? (
                      <TableActionsHead className="w-24">
                        {t("label.actions")}
                      </TableActionsHead>
                    ) : null}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {definitions.map((definition) => (
                    <TableRow
                      data-ui="settings-table-row"
                      key={definition.id}
                      id={selectorId("settings-title-tag-row", definition.id)}
                      className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                    >
                      <TableCell className="truncate font-medium text-[var(--scry-ink2)]">
                        {definition.label}
                      </TableCell>
                      <TableCell className={`truncate ${TAG_MUTED_TEXT_CLASS}`}>
                        {definition.description?.trim() || "—"}
                      </TableCell>
                      <TableCell className="text-center tabular-nums text-[var(--scry-ink2)]">
                        {definition.titleCount}
                      </TableCell>
                      {canManageCatalogSettings ? (
                        <TableActionsCell className="w-24">
                          <div className="flex items-center justify-center gap-1">
                            <IconButton
                              id={selectorId("settings-title-tag-edit", definition.id)}
                              label={t("label.edit")}
                              tone="edit"
                              onClick={() => loadDefinitionById(definition.id)}
                              title={t("label.edit")}
                            >
                              <Edit className="h-4 w-4" />
                            </IconButton>
                            <IconButton
                              id={selectorId("settings-title-tag-delete", definition.id)}
                              label={t("label.delete")}
                              tone="delete"
                              onClick={() => deleteDefinition(definition.id)}
                              disabled={saving}
                            >
                              <Trash2 className="h-4 w-4" />
                            </IconButton>
                          </div>
                        </TableActionsCell>
                      ) : null}
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </section>

      {!canManageCatalogSettings ? null : isEditorOpen ? (
        <>
          <section className={TAG_PANEL_CLASS}>
            <div className={TAG_PANEL_HEADER_CLASS}>
              <h2 className={TAG_PANEL_TITLE_CLASS}>
                {isEditing ? t("settings.titleTagEdit") : t("settings.titleTagCreate")}
              </h2>
            </div>
            <div className={TAG_PANEL_BODY_CLASS}>
              <form onSubmit={saveDefinition} className="space-y-4">
                <div className="space-y-1.5">
                  <Label className="text-[var(--scry-ink2)]" htmlFor="title-tag-label">
                    {t("settings.titleTagLabelLabel")}
                  </Label>
                  <Input
                    id="title-tag-label"
                    value={draft.label}
                    onChange={(event) => updateField("label", event.target.value)}
                    placeholder={t("settings.titleTagLabelPlaceholder")}
                    maxLength={64}
                  />
                  <p className={`text-xs ${TAG_MUTED_TEXT_CLASS}`}>
                    {t("settings.titleTagLabelHelp")}
                  </p>
                </div>

                <div className="space-y-1.5">
                  <Label
                    className="text-[var(--scry-ink2)]"
                    htmlFor="title-tag-description"
                  >
                    {t("settings.titleTagDescriptionLabel")}
                  </Label>
                  <Input
                    id="title-tag-description"
                    value={draft.description}
                    onChange={(event) => updateField("description", event.target.value)}
                    placeholder={t("settings.titleTagDescriptionPlaceholder")}
                  />
                </div>

                <div className="flex flex-wrap gap-2 pt-2">
                  <Button id="settings-title-tag-save" type="submit" disabled={saving}>
                    {saving
                      ? t("label.saving")
                      : isEditing
                        ? t("label.save")
                        : t("settings.titleTagCreate")}
                  </Button>
                  <Button
                    id="settings-title-tag-cancel"
                    type="button"
                    variant="outline"
                    onClick={resetDraft}
                  >
                    {t("label.cancel")}
                  </Button>
                </div>
              </form>
            </div>
          </section>
          {isEditing ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-title-tag-create-new"
                icon={Plus}
                label={t("settings.titleTagCreateNew")}
                onClick={startCreateDefinition}
                disabled={saving}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-title-tag-create"
            icon={Plus}
            label={t("settings.titleTagCreateNew")}
            onClick={startCreateDefinition}
            disabled={saving}
          />
        </div>
      )}
    </div>
  );
}
