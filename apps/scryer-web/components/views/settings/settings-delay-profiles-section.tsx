import * as React from "react";
import { Edit, Plus, Trash2 } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { InfoHelp } from "@/components/common/info-help";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
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
import type {
  DelayProfileDraft,
  DelayProfileFacet,
  ParsedDelayProfile,
} from "@/lib/types/delay-profiles";
import {
  applyDelayProfileProtocolMode,
  DELAY_PROFILE_PROTOCOL_MODES,
  delayProfileProtocolMode,
  FACET_OPTIONS,
  type DelayProfileProtocolMode,
} from "@/lib/utils/delay-profiles";
import { selectorId } from "@/lib/utils/dom-ids";

type SettingsDelayProfilesSectionProps = {
  loading: boolean;
  saving: boolean;
  profiles: ParsedDelayProfile[];
  parseError: string;
  draft: DelayProfileDraft;
  setDraft: React.Dispatch<React.SetStateAction<DelayProfileDraft>>;
  saveProfile: (event?: React.FormEvent<HTMLFormElement>) => void;
  deleteProfile: (profileId: string) => void;
  loadProfileById: (profileId: string) => void;
  resetDraft: () => void;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateProfile: () => void;
};

const FACET_LABELS: Record<string, string> = {
  movie: "Movies",
  series: "TV Series",
  anime: "Anime",
};

const DELAY_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const DELAY_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const DELAY_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const DELAY_PANEL_BODY_CLASS = "p-4 sm:p-5";
const DELAY_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const DELAY_TABLE_HEADER_CELL_CLASS =
  "text-center font-semibold text-[var(--scry-muted3)]";

function protocolModeLabelKey(mode: DelayProfileProtocolMode) {
  return `settings.delayProfileProtocolMode.${mode}`;
}

type DelayProfileHelpLabelProps = {
  htmlFor?: string;
  label: string;
  help: string;
};

function DelayProfileHelpLabel({
  htmlFor,
  label,
  help,
}: DelayProfileHelpLabelProps) {
  return (
    <div className="flex items-center gap-1.5">
      <Label className="text-[var(--scry-ink2)]" htmlFor={htmlFor}>
        {label}
      </Label>
      <InfoHelp text={help} ariaLabel={help} />
    </div>
  );
}

export function SettingsDelayProfilesSection({
  loading,
  saving,
  profiles,
  parseError,
  draft,
  setDraft,
  saveProfile,
  deleteProfile,
  loadProfileById,
  resetDraft,
  isEditorOpen,
  editorMode,
  startCreateProfile,
}: SettingsDelayProfilesSectionProps) {
  const t = useTranslate();

  const isEditing = editorMode === "edit";

  function updateField<K extends keyof DelayProfileDraft>(field: K, value: DelayProfileDraft[K]) {
    setDraft((prev) => ({ ...prev, [field]: value }));
  }

  function parseIntegerInput(raw: string) {
    const nextValue = sanitizeDigits(raw);
    return nextValue === "" ? 0 : Number(nextValue);
  }

  function toggleFacet(facet: DelayProfileFacet) {
    setDraft((prev) => {
      const has = prev.applies_to_facets.includes(facet);
      return {
        ...prev,
        applies_to_facets: has
          ? prev.applies_to_facets.filter((f) => f !== facet)
          : [...prev.applies_to_facets, facet],
      };
    });
  }

  function updateProtocolMode(mode: DelayProfileProtocolMode) {
    setDraft((prev) => applyDelayProfileProtocolMode(prev, mode));
  }

  return (
    <div id="settings-delay-profiles-section" className="space-y-4 text-sm">
      {parseError && (
        <div className="rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]">
          {parseError}
        </div>
      )}

      <section className={DELAY_PANEL_CLASS}>
        <div className={DELAY_PANEL_HEADER_CLASS}>
          <h2 className={DELAY_PANEL_TITLE_CLASS}>{t("settings.delayProfileExisting")}</h2>
        </div>
        <div>
          {loading ? (
            <p className={`${DELAY_PANEL_BODY_CLASS} text-sm ${DELAY_MUTED_TEXT_CLASS}`}>
              {t("label.loading")}
            </p>
          ) : profiles.length === 0 ? (
            <p className={`${DELAY_PANEL_BODY_CLASS} text-sm ${DELAY_MUTED_TEXT_CLASS}`}>
              {t("settings.delayProfileNone")}
            </p>
          ) : (
            <div className="overflow-hidden">
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader>
                  <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                    <TableHead className={`w-[18%] font-semibold ${DELAY_MUTED_TEXT_CLASS}`}>{t("settings.delayProfileNameLabel")}</TableHead>
                    <TableHead className={`w-28 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileUsenetDelay")}</TableHead>
                    <TableHead className={`w-28 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileTorrentDelay")}</TableHead>
                    <TableHead className={`w-32 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileProtocolModeLabel")}</TableHead>
                    <TableHead className={`w-24 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileMinAge")}</TableHead>
                    <TableHead className={`w-32 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileBypassLabel")}</TableHead>
                    <TableHead className={`w-[16%] ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileFacetsLabel")}</TableHead>
                    <TableHead className={`w-24 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfilePriorityLabel")}</TableHead>
                    <TableHead className={`w-24 ${DELAY_TABLE_HEADER_CELL_CLASS}`}>{t("settings.delayProfileEnabledLabel")}</TableHead>
                    <TableActionsHead className="w-24">{t("label.actions")}</TableActionsHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {profiles.map((profile) => (
                    <TableRow
                      data-ui="settings-table-row"
                      key={profile.id}
                      id={selectorId("settings-delay-profile-row", profile.id)}
                      className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                    >
                      <TableCell className="truncate font-medium text-[var(--scry-ink2)]">{profile.name}</TableCell>
                      <TableCell className={`text-center ${DELAY_MUTED_TEXT_CLASS}`}>{profile.usenet_delay_minutes}m</TableCell>
                      <TableCell className={`text-center ${DELAY_MUTED_TEXT_CLASS}`}>{profile.torrent_delay_minutes}m</TableCell>
                      <TableCell className="text-center text-[var(--scry-ink2)]">
                        {t(protocolModeLabelKey(delayProfileProtocolMode(profile)))}
                      </TableCell>
                      <TableCell className="text-center">{profile.min_age_minutes > 0 ? `${profile.min_age_minutes}m` : "—"}</TableCell>
                      <TableCell className="text-center">
                        {profile.bypass_score_threshold != null
                          ? `≥ ${profile.bypass_score_threshold}`
                          : "—"}
                      </TableCell>
                      <TableCell className="truncate text-center">
                        {profile.applies_to_facets.length === 0
                          ? t("settings.delayProfileAllFacets")
                          : profile.applies_to_facets
                              .map((f) => FACET_LABELS[f] ?? f)
                              .join(", ")}
                      </TableCell>
                      <TableCell className="text-center">{profile.priority}</TableCell>
                      <TableCell className="text-center">
                        <RenderBooleanIcon
                          value={profile.enabled}
                          label={`${t("settings.delayProfileEnabledLabel")}: ${profile.name}`}
                        />
                      </TableCell>
                      <TableActionsCell className="w-24">
                        <div className="flex items-center justify-center gap-1">
                          <IconButton
                            id={selectorId("settings-delay-profile-edit", profile.id)}
                            label={t("label.edit")}
                            tone="edit"
                            onClick={() => loadProfileById(profile.id)}
                            title={t("label.load")}
                          >
                            <Edit className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId("settings-delay-profile-delete", profile.id)}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => deleteProfile(profile.id)}
                            disabled={saving}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </div>
                      </TableActionsCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </section>

      {isEditorOpen ? (
        <>
          <section className={DELAY_PANEL_CLASS}>
            <div className={DELAY_PANEL_HEADER_CLASS}>
              <h2 className={DELAY_PANEL_TITLE_CLASS}>
                {isEditing
                  ? t("settings.delayProfileEdit")
                  : t("settings.delayProfileCreate")}
              </h2>
            </div>
            <div className={DELAY_PANEL_BODY_CLASS}>
              <form onSubmit={saveProfile} className="space-y-4">
            {/* Name */}
            <div className="space-y-1.5">
              <Label className="text-[var(--scry-ink2)]" htmlFor="dp-name">{t("settings.delayProfileNameLabel")}</Label>
              <Input
                id="dp-name"
                value={draft.name}
                onChange={(e) => updateField("name", e.target.value)}
                placeholder={t("settings.delayProfileNamePlaceholder")}
              />
            </div>

            {/* Protocol delays */}
            <div className="grid gap-4 sm:grid-cols-2">
              {draft.enable_usenet ? (
                <div className="space-y-1.5">
                  <DelayProfileHelpLabel
                    htmlFor="dp-usenet-delay"
                    label={t("settings.delayProfileUsenetDelay")}
                    help={t("settings.delayProfileUsenetDelayHelp")}
                  />
                  <Input
                    id="dp-usenet-delay"
                    {...integerInputProps}
                    value={draft.usenet_delay_minutes}
                    onChange={(e) =>
                      updateField(
                        "usenet_delay_minutes",
                        parseIntegerInput(e.target.value),
                      )
                    }
                  />
                </div>
              ) : null}
              {draft.enable_torrent ? (
                <div className="space-y-1.5">
                  <DelayProfileHelpLabel
                    htmlFor="dp-torrent-delay"
                    label={t("settings.delayProfileTorrentDelay")}
                    help={t("settings.delayProfileTorrentDelayHelp")}
                  />
                  <Input
                    id="dp-torrent-delay"
                    {...integerInputProps}
                    value={draft.torrent_delay_minutes}
                    onChange={(e) =>
                      updateField(
                        "torrent_delay_minutes",
                        parseIntegerInput(e.target.value),
                      )
                    }
                  />
                </div>
              ) : null}
            </div>

            {/* Protocol mode */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                label={t("settings.delayProfileProtocolModeLabel")}
                help={t("settings.delayProfileProtocolModeHelp")}
              />
              <RadioGroup
                className="flex flex-wrap gap-4"
                value={delayProfileProtocolMode(draft)}
                onValueChange={(value) => updateProtocolMode(value as DelayProfileProtocolMode)}
              >
                {DELAY_PROFILE_PROTOCOL_MODES.map((mode) => (
                  <label
                    key={mode}
                    htmlFor={selectorId(
                      "settings-delay-profile-preferred",
                      mode,
                    )}
                    className="flex items-center gap-2 text-sm text-[var(--scry-ink2)]"
                  >
                    <RadioGroupItem
                      id={selectorId("settings-delay-profile-preferred", mode)}
                      value={mode}
                    />
                    {t(protocolModeLabelKey(mode))}
                  </label>
                ))}
              </RadioGroup>
            </div>

            {/* Minimum age (usenet) */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                htmlFor="dp-min-age"
                label={t("settings.delayProfileMinAge")}
                help={t("settings.delayProfileMinAgeHelp")}
              />
              <Input
                id="dp-min-age"
                {...integerInputProps}
                value={draft.min_age_minutes}
                onChange={(e) => updateField("min_age_minutes", parseIntegerInput(e.target.value))}
              />
            </div>

            <CheckboxField
              id="settings-delay-profile-bypass-highest-quality"
              checked={draft.bypass_if_highest_quality}
              onCheckedChange={(checked) =>
                updateField("bypass_if_highest_quality", checked === true)
              }
              label={t("settings.delayProfileBypassHighestQualityLabel")}
              labelAccessory={
                <InfoHelp
                  text={t("settings.delayProfileBypassHighestQualityHelp")}
                  ariaLabel={t("settings.delayProfileBypassHighestQualityHelp")}
                />
              }
              className="items-start text-[var(--scry-ink2)]"
              checkboxClassName="mt-0.5"
            />

            {/* Bypass score threshold */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                htmlFor="dp-bypass"
                label={t("settings.delayProfileBypassLabel")}
                help={t("settings.delayProfileBypassHelp")}
              />
              <Input
                id="dp-bypass"
                {...integerInputProps}
                value={draft.bypass_score_threshold ?? ""}
                onChange={(e) => {
                  const val = sanitizeDigits(e.target.value);
                  updateField(
                    "bypass_score_threshold",
                    val === "" ? null : Number(val),
                  );
                }}
                placeholder={t("settings.delayProfileBypassPlaceholder")}
              />
            </div>

            {/* Applies to facets */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                label={t("settings.delayProfileFacetsLabel")}
                help={t("settings.delayProfileFacetsHelp")}
              />
              <div className="flex flex-wrap gap-4">
                {FACET_OPTIONS.map((facet) => (
                  <label key={facet} className="flex items-center gap-2 text-sm text-[var(--scry-ink2)]">
                    <Checkbox
                      id={selectorId("settings-delay-profile-facet", facet)}
                      checked={draft.applies_to_facets.includes(facet)}
                      onCheckedChange={() => toggleFacet(facet)}
                    />
                    {FACET_LABELS[facet] ?? facet}
                  </label>
                ))}
              </div>
            </div>

            {/* Tags */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                htmlFor="dp-tags"
                label={t("settings.delayProfileTagsLabel")}
                help={t("settings.delayProfileTagsHelp")}
              />
              <Input
                id="dp-tags"
                value={draft.tags.join(", ")}
                onChange={(e) =>
                  updateField(
                    "tags",
                    e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  )
                }
                placeholder={t("settings.delayProfileTagsPlaceholder")}
              />
            </div>

            {/* Priority */}
            <div className="space-y-1.5">
              <DelayProfileHelpLabel
                htmlFor="dp-priority"
                label={t("settings.delayProfilePriorityLabel")}
                help={t("settings.delayProfilePriorityHelp")}
              />
              <Input
                id="dp-priority"
                {...integerInputProps}
                value={draft.priority}
                onChange={(e) => updateField("priority", parseIntegerInput(e.target.value))}
              />
            </div>

            {/* Enabled */}
            <CheckboxField
              id="settings-delay-profile-enabled"
              checked={draft.enabled}
              onCheckedChange={(checked) =>
                updateField("enabled", checked === true)
              }
              label={t("settings.delayProfileEnabledLabel")}
              className="items-center text-[var(--scry-ink2)]"
              checkboxClassName="mt-0"
            />

            {/* Actions */}
            <div className="flex flex-wrap gap-2 pt-2">
              <Button id="settings-delay-profile-save" type="submit" disabled={saving}>
                {saving
                  ? t("label.saving")
                  : isEditing
                    ? t("label.save")
                    : t("settings.delayProfileCreate")}
              </Button>
              <Button id="settings-delay-profile-cancel" type="button" variant="outline" onClick={resetDraft}>
                {t("label.cancel")}
              </Button>
            </div>
          </form>
        </div>
      </section>
      {editorMode === "edit" ? (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-delay-profile-create-new"
            icon={Plus}
            label={t("settings.delayProfileCreateNew")}
            onClick={startCreateProfile}
            disabled={saving}
          />
        </div>
      ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-delay-profile-create"
            icon={Plus}
            label={t("settings.delayProfileCreateNew")}
            onClick={startCreateProfile}
            disabled={saving}
          />
        </div>
      )}
    </div>
  );
}
