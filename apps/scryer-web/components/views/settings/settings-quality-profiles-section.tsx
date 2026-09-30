
import * as React from "react";
import { ArrowDown, ArrowUp, ArrowLeft, ArrowRight, GripVertical, Edit, Plus, Trash2 } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { InfoHelp } from "@/components/common/info-help";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { PERSONA_OVERRIDE_DEFAULTS } from "@/lib/constants/quality-profiles";
import { selectorId } from "@/lib/utils/dom-ids";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";

const QUALITY_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const QUALITY_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const QUALITY_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const QUALITY_PANEL_BODY_CLASS = "p-4 sm:p-5";
const QUALITY_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const QUALITY_EDITOR_FIELD_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4";
const QUALITY_EDITOR_SECTION_CLASS =
  "overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const QUALITY_EDITOR_SECTION_SUMMARY_CLASS =
  "cursor-pointer select-none px-4 py-3 text-sm font-semibold text-[var(--scry-ink2)]";
const QUALITY_EDITOR_SECTION_BODY_CLASS =
  "border-t border-[var(--scry-border3)] p-4";
const QUALITY_EDITOR_LIST_CLASS =
  "max-h-60 overflow-auto rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-2";
const QUALITY_EDITOR_LIST_ITEM_CLASS =
  "mb-1 flex items-center justify-between gap-2 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-card2)] px-2 py-1.5 text-[var(--scry-ink2)] hover:bg-[var(--scry-hover)]";

type ViewCategoryId = "MOVIE" | "SERIES" | "ANIME";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type ScoringPersonaId = "BALANCED" | "AUDIOPHILE" | "EFFICIENT" | "COMPATIBLE";

type ScoringOverridesPayload = {
  allow_x265_non4k?: boolean | null;
  block_dv_without_fallback?: boolean | null;
  prefer_compact_encodes?: boolean | null;
  prefer_lossless_audio?: boolean | null;
  block_upscaled?: boolean | null;
};

type QualityProfileCriteriaPayload = {
  quality_tiers: string[];
  archival_quality: string | null;
  allow_unknown_quality: boolean;
  source_allowlist: string[];
  source_blocklist: string[];
  video_codec_allowlist: string[];
  video_codec_blocklist: string[];
  audio_codec_allowlist: string[];
  audio_codec_blocklist: string[];
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string | null;
  min_score_to_grab: number | null;
};

type QualityProfileDraft = {
  id: string;
  name: string;
  quality_tiers: string[];
  archival_quality: string;
  allow_unknown_quality: boolean;
  source_allowlist: string[];
  source_blocklist: string[];
  video_codec_allowlist: string[];
  video_codec_blocklist: string[];
  audio_codec_allowlist: string[];
  audio_codec_blocklist: string[];
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string;
  min_score_to_grab: number | null;
};

type QualityProfileListField =
  | "source_allowlist"
  | "source_blocklist"
  | "video_codec_allowlist"
  | "video_codec_blocklist"
  | "audio_codec_allowlist"
  | "audio_codec_blocklist";

type ProfileListChoice = {
  value: string;
  label: string;
};

type SettingsQualityProfilesSectionProps = {
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileParseError: string;
  getQualityProfileCriteria: (profileId: string) => QualityProfileCriteriaPayload | undefined;
  getQualityProfileBoolean: (
    profileId: string,
    field: keyof QualityProfileCriteriaPayload,
    fallback: boolean,
  ) => boolean;
  loadQualityProfileById: (profileId: string) => void;
  startNewQualityProfileDraft: () => void;
  activeQualityProfileTierOptions: string[];
  availableQualityTiers: Array<{ value: string; label: string }>;
  updateQualityProfileDraft: (
    patch: Partial<QualityProfileDraft> | ((current: QualityProfileDraft) => QualityProfileDraft),
  ) => void;
  qualityProfileDraft: QualityProfileDraft;
  availableSourceAllowlist: ReadonlyArray<ProfileListChoice>;
  availableVideoCodecAllowlist: ReadonlyArray<ProfileListChoice>;
  availableAudioCodecAllowlist: ReadonlyArray<ProfileListChoice>;
  activeSourceAllowlist: string[];
  activeSourceBlocklist: string[];
  activeVideoCodecAllowlist: string[];
  activeVideoCodecBlocklist: string[];
  activeAudioCodecAllowlist: string[];
  activeAudioCodecBlocklist: string[];
  qualityCategoryLabels: Record<ViewCategoryId, string>;
  moveProfileListToAllowed: (
    allowedField: QualityProfileListField,
    deniedField: QualityProfileListField,
    value: string,
  ) => void;
  moveProfileListToDenied: (
    allowedField: QualityProfileListField,
    deniedField: QualityProfileListField,
    value: string,
  ) => void;
  addQualityTier: (value: string) => void;
  removeQualityTier: (value: string) => void;
  reorderQualityTier: (value: string, targetIndex: number) => void;
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => Array<{ value: string; label: string }>;
  globalQualityProfileId: string;
  setGlobalQualityProfileId: (value: string) => void;
  globalScoringPersona: ScoringPersonaId;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  setCategoryQualityProfileOverrides: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  categoryPersonaSelections: Record<
    ViewCategoryId,
    {
      scope: ViewCategoryId;
      overridePersona: ScoringPersonaId | null;
      effectivePersona: ScoringPersonaId;
      inheritsGlobal: boolean;
    }
  >;
  mediaSettingsLoading: boolean;
  initialLoadComplete: boolean;
  qualityProfilesSaving: boolean;
  updateQualityProfilesGlobal: (event?: React.FormEvent<HTMLFormElement>) => Promise<boolean> | boolean;
  categoryQualityProfileSaving: Record<ViewCategoryId, boolean>;
  saveCategoryQualityProfile: (scopeId: ViewCategoryId, value: string) => Promise<void> | void;
  saveGlobalQualityProfile: (value: string) => Promise<void> | void;
  saveGlobalScoringPersona: (value: ScoringPersonaId) => Promise<void> | void;
  saveCategoryScoringPersona: (
    scopeId: ViewCategoryId,
    value: ScoringPersonaId | null,
  ) => Promise<void> | void;
  archivalQualityOptions: Array<{ value: string; label: string }>;
  deleteQualityProfile: (profileId: string) => Promise<void>;
};

type PendingQualityProfileEditorAction =
  | { type: "create" }
  | { type: "edit"; profileId: string }
  | { type: "close" }
  | null;

function cloneQualityProfileDraft(draft: QualityProfileDraft): QualityProfileDraft {
  return {
    ...draft,
    source_allowlist: [...draft.source_allowlist],
    source_blocklist: [...draft.source_blocklist],
    video_codec_allowlist: [...draft.video_codec_allowlist],
    video_codec_blocklist: [...draft.video_codec_blocklist],
    audio_codec_allowlist: [...draft.audio_codec_allowlist],
    audio_codec_blocklist: [...draft.audio_codec_blocklist],
    quality_tiers: [...draft.quality_tiers],
    scoring_overrides: { ...draft.scoring_overrides },
  };
}

function QualityProfileActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "delete">;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
  );
}

function ProfileListEditor({
  title,
  allowed,
  denied,
  choices,
  onMoveToAllowed,
  onMoveToDenied,
  emptyStateMessage,
  info,
}: {
  title: string;
  allowed: string[];
  denied: string[];
  choices: ReadonlyArray<ProfileListChoice>;
  onMoveToAllowed: (value: string) => void;
  onMoveToDenied: (value: string) => void;
  emptyStateMessage?: string;
  info?: string;
}) {
  const t = useTranslate();
  const optionByValue = React.useMemo(() => {
    const next = new Map<string, string>();
    choices.forEach((option) => {
      const normalizedValue = option.value.trim();
      if (!normalizedValue) {
        return;
      }
      if (!next.has(normalizedValue)) {
        next.set(normalizedValue, option.label);
      }
    });
    return next;
  }, [choices]);
  const allChoiceValues = React.useMemo(() => Array.from(optionByValue.keys()), [optionByValue]);
  const sortedAllowedValues = React.useMemo(
    () =>
      dedupeOrdered(allowed)
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0),
    [allowed],
  );
  const sortedDeniedValues = React.useMemo(
    () =>
      dedupeOrdered(denied)
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0),
    [denied],
  );
  const deniedSet = React.useMemo(() => new Set(sortedDeniedValues), [sortedDeniedValues]);
  const sortedAllowed = React.useMemo(() => {
    const values = sortedAllowedValues.length === 0 ? allChoiceValues : sortedAllowedValues;
    const effectiveAllowed = values.filter((value) => !deniedSet.has(value));
    return [...effectiveAllowed]
      .map((value) => ({
        value,
        label: optionByValue.get(value) ?? value,
      }))
      .sort((left, right) => sortProfileListChoiceByNumericDesc(left, right));
  }, [allChoiceValues, sortedAllowedValues, deniedSet, optionByValue]);
  const sortedDenied = React.useMemo(
    () =>
      sortedDeniedValues
        .map((value) => ({
          value,
          label: optionByValue.get(value) ?? value,
        }))
        .sort((left, right) => sortProfileListChoiceByNumericDesc(left, right)),
    [sortedDeniedValues, optionByValue],
  );

  return (
    <details className={QUALITY_EDITOR_SECTION_CLASS}>
      <summary className={QUALITY_EDITOR_SECTION_SUMMARY_CLASS}>
        <span className="inline-flex items-center gap-2">
          <span>{title}</span>
          {info ? (
            <InfoHelp text={info} ariaLabel={t("qualityProfile.aboutSection", { title })} />
          ) : null}
        </span>
      </summary>
      <div className={`${QUALITY_EDITOR_SECTION_BODY_CLASS} grid gap-3 md:grid-cols-2`}>
        <div>
          <Label className="mb-2 block text-[var(--scry-ink2)]">Allowed</Label>
          <div className={QUALITY_EDITOR_LIST_CLASS}>
            {sortedAllowed.length === 0 ? (
              <p className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>
                {t("qualityProfile.noSelectedItems")}
              </p>
            ) : (
              sortedAllowed.map((option) => (
                <div
                  key={option.value}
                  className="mb-1 flex items-center justify-between gap-2 rounded-[9px] border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] px-2 py-1.5 text-[var(--scry-ink2)] ring-1 ring-inset ring-[var(--scry-success-border)] hover:border-[var(--scry-success-border-strong)]"
                >
                  <span className="text-xs">{option.label}</span>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => onMoveToDenied(option.value)}
                    aria-label={`Move ${option.label} to denied list`}
                  >
                    <ArrowRight className="h-3 w-3" />
                  </Button>
                </div>
              ))
            )}
          </div>
        </div>
        <div>
          <Label className="mb-2 block text-[var(--scry-ink2)]">Denied</Label>
          <div className={QUALITY_EDITOR_LIST_CLASS}>
            {sortedDenied.length === 0 ? (
              <p className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>
                {emptyStateMessage ?? t("qualityProfile.noSelectedItems")}
              </p>
            ) : (
              sortedDenied.map((option) => (
                <div
                  key={option.value}
                  className="mb-1 flex items-center justify-between gap-2 rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-2 py-1.5 text-[var(--scry-ink2)] ring-1 ring-inset ring-[color:var(--scry-danger-border)] hover:border-[var(--scry-danger-border-strong)]"
                >
                  <span className="text-xs">{option.label}</span>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => onMoveToAllowed(option.value)}
                    aria-label={`Move ${option.label} to allowed list`}
                  >
                    <ArrowLeft className="h-3 w-3" />
                  </Button>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </details>
  );
}

function getQualityTierLabel(value: string): string {
  const normalized = value.trim().toUpperCase();
  if (!normalized) {
    return "";
  }
  if (normalized === "SD") {
    return "SD";
  }
  if (normalized === "HD") {
    return "HD";
  }
  if (normalized === "UHD") {
    return "UHD";
  }
  if (normalized === "4K") {
    return "4K";
  }
  if (normalized === "8K") {
    return "8K";
  }
  return normalized;
}

function parseStringArrayValue(raw: unknown): string[] {
  if (Array.isArray(raw)) {
    return raw.map((entry) => (typeof entry === "string" ? entry : String(entry)));
  }
  return [];
}

function dedupeOrdered(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  values.forEach((value) => {
    const normalized = value.trim();
    if (!normalized || seen.has(normalized)) {
      return;
    }
    seen.add(normalized);
    result.push(normalized);
  });
  return result;
}

function sortStringByNumericDesc(left: string, right: string): number {
  const leftNumeric = Number.parseFloat(left.replace(/[^0-9.]/g, ""));
  const rightNumeric = Number.parseFloat(right.replace(/[^0-9.]/g, ""));
  if (!Number.isNaN(leftNumeric) && !Number.isNaN(rightNumeric)) {
    if (leftNumeric === rightNumeric) {
      return right.localeCompare(left);
    }
    return rightNumeric - leftNumeric;
  }
  return right.localeCompare(left);
}

function sortProfileListChoiceByNumericDesc(
  left: ProfileListChoice,
  right: ProfileListChoice,
): number {
  return sortStringByNumericDesc(left.label, right.label) || left.value.localeCompare(right.value);
}

export function SettingsQualityProfilesSection({
  qualityProfiles,
  qualityProfileParseError,
  getQualityProfileCriteria,
  getQualityProfileBoolean,
  loadQualityProfileById,
  startNewQualityProfileDraft,
  activeQualityProfileTierOptions,
  availableQualityTiers,
  updateQualityProfileDraft,
  qualityProfileDraft,
  availableSourceAllowlist,
  availableVideoCodecAllowlist,
  availableAudioCodecAllowlist,
  activeSourceAllowlist,
  activeSourceBlocklist,
  activeVideoCodecAllowlist,
  activeVideoCodecBlocklist,
  activeAudioCodecAllowlist,
  activeAudioCodecBlocklist,
  qualityCategoryLabels,
  moveProfileListToAllowed,
  moveProfileListToDenied,
  addQualityTier,
  removeQualityTier,
  reorderQualityTier,
  qualityProfileInheritValue,
  toProfileOptions,
  globalQualityProfileId,
  setGlobalQualityProfileId,
  globalScoringPersona,
  categoryQualityProfileOverrides,
  setCategoryQualityProfileOverrides,
  categoryPersonaSelections,
  mediaSettingsLoading,
  initialLoadComplete,
  qualityProfilesSaving,
  updateQualityProfilesGlobal,
  categoryQualityProfileSaving,
  saveCategoryQualityProfile,
  saveGlobalQualityProfile,
  saveGlobalScoringPersona,
  saveCategoryScoringPersona,
  archivalQualityOptions,
  deleteQualityProfile,
}: SettingsQualityProfilesSectionProps) {
  const t = useTranslate();
  const [globalQualityProfileDraft, setGlobalQualityProfileDraft] = React.useState(
    globalQualityProfileId,
  );
  const [globalScoringPersonaDraft, setGlobalScoringPersonaDraft] =
    React.useState<ScoringPersonaId>(globalScoringPersona);
  const [categoryQualityProfileDrafts, setCategoryQualityProfileDrafts] = React.useState<
    Record<ViewCategoryId, string>
  >(categoryQualityProfileOverrides);
  const [categoryPersonaDrafts, setCategoryPersonaDrafts] = React.useState<
    Record<ViewCategoryId, string>
  >({
    MOVIE: categoryPersonaSelections.MOVIE.overridePersona ?? "__default__",
    SERIES: categoryPersonaSelections.SERIES.overridePersona ?? "__default__",
    ANIME: categoryPersonaSelections.ANIME.overridePersona ?? "__default__",
  });
  const [pendingDeleteProfile, setPendingDeleteProfile] = React.useState<{ id: string; name: string } | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    React.useState<PendingQualityProfileEditorAction>(null);
  const [isEditorOpen, setIsEditorOpen] = React.useState(false);
  const [editorMode, setEditorMode] = React.useState<"create" | "edit">("create");
  const [qualityProfileDraftBaseline, setQualityProfileDraftBaseline] = React.useState(() =>
    cloneQualityProfileDraft(qualityProfileDraft),
  );
  const [awaitingBaselineSync, setAwaitingBaselineSync] = React.useState(false);

  React.useEffect(() => {
    setGlobalQualityProfileDraft(globalQualityProfileId);
  }, [globalQualityProfileId]);

  React.useEffect(() => {
    setGlobalScoringPersonaDraft(globalScoringPersona);
  }, [globalScoringPersona]);

  React.useEffect(() => {
    setCategoryQualityProfileDrafts(categoryQualityProfileOverrides);
  }, [categoryQualityProfileOverrides]);

  React.useEffect(() => {
    setCategoryPersonaDrafts({
      MOVIE: categoryPersonaSelections.MOVIE.overridePersona ?? "__default__",
      SERIES: categoryPersonaSelections.SERIES.overridePersona ?? "__default__",
      ANIME: categoryPersonaSelections.ANIME.overridePersona ?? "__default__",
    });
  }, [categoryPersonaSelections]);

  React.useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }
    setQualityProfileDraftBaseline(cloneQualityProfileDraft(qualityProfileDraft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, qualityProfileDraft]);

  const isProfileDraftDirty =
    JSON.stringify(qualityProfileDraft) !== JSON.stringify(qualityProfileDraftBaseline);

  const handleCategoryProfileOverrideChange = React.useCallback(
    (scopeId: ViewCategoryId, rawValue: string) => {
      if (!initialLoadComplete) return;
      const normalized = rawValue.trim();
      if (categoryQualityProfileOverrides[scopeId] === normalized) return;
      setCategoryQualityProfileDrafts((previous) => ({
        ...previous,
        [scopeId]: normalized,
      }));
      setCategoryQualityProfileOverrides((previous) => ({
        ...previous,
        [scopeId]: normalized,
      }));
      void saveCategoryQualityProfile(scopeId, normalized);
    },
    [initialLoadComplete, categoryQualityProfileOverrides, saveCategoryQualityProfile, setCategoryQualityProfileOverrides],
  );

  const handleGlobalProfileChange = React.useCallback(
    (rawValue: string) => {
      if (!initialLoadComplete) return;
      const normalized = rawValue.trim();
      if (globalQualityProfileId === normalized) return;
      setGlobalQualityProfileDraft(normalized);
      setGlobalQualityProfileId(normalized);
      void saveGlobalQualityProfile(normalized);
    },
    [initialLoadComplete, globalQualityProfileId, saveGlobalQualityProfile, setGlobalQualityProfileId],
  );

  const handleGlobalScoringPersonaChange = React.useCallback(
    (value: string) => {
      if (!initialLoadComplete) return;
      const normalized = value as ScoringPersonaId;
      if (globalScoringPersona === normalized) return;
      setGlobalScoringPersonaDraft(normalized);
      void saveGlobalScoringPersona(normalized);
    },
    [
      globalScoringPersona,
      initialLoadComplete,
      saveGlobalScoringPersona,
    ],
  );

  const handleCategoryPersonaChange = React.useCallback(
    (scopeId: ViewCategoryId, value: string) => {
      if (!initialLoadComplete) return;
      const normalizedValue = value.trim();
      const previousValue = categoryPersonaDrafts[scopeId];
      if (previousValue === normalizedValue) return;
      setCategoryPersonaDrafts((previous) => ({
        ...previous,
        [scopeId]: normalizedValue,
      }));
      void saveCategoryScoringPersona(
        scopeId,
        normalizedValue === "__default__"
          ? null
          : (normalizedValue as ScoringPersonaId),
      );
    },
    [categoryPersonaDrafts, initialLoadComplete, saveCategoryScoringPersona],
  );

  // Keep for backwards compat but these are now no-ops since we save on change
  const handleGlobalProfileBlur = React.useCallback(() => {}, []);

  const handleCategoryProfileOverrideBlur = React.useCallback(
    (_scopeId: ViewCategoryId) => {},
    [],
  );

  const handleStartCreateProfile = React.useCallback(() => {
    if (isEditorOpen && isProfileDraftDirty) {
      setPendingEditorAction({ type: "create" });
      return;
    }
    startNewQualityProfileDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [isEditorOpen, isProfileDraftDirty, startNewQualityProfileDraft]);

  const handleEditProfile = React.useCallback(
    (profileId: string) => {
      if (isEditorOpen && isProfileDraftDirty) {
        setPendingEditorAction({ type: "edit", profileId });
        return;
      }
      loadQualityProfileById(profileId);
      setEditorMode("edit");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    },
    [isEditorOpen, isProfileDraftDirty, loadQualityProfileById],
  );

  const handleSaveQualityProfile = React.useCallback(async () => {
    const saved = await updateQualityProfilesGlobal();
    if (saved) {
      setQualityProfileDraftBaseline(cloneQualityProfileDraft(qualityProfileDraft));
      if (editorMode === "create") {
        setIsEditorOpen(false);
        setEditorMode("create");
      }
    }
  }, [editorMode, qualityProfileDraft, updateQualityProfilesGlobal]);

  const handleCloseProfileEditor = React.useCallback(() => {
    if (!isEditorOpen) return;
    if (isProfileDraftDirty) {
      setPendingEditorAction({ type: "close" });
      return;
    }
    setIsEditorOpen(false);
    setEditorMode("create");
  }, [isEditorOpen, isProfileDraftDirty]);

  const confirmPendingEditorAction = React.useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      startNewQualityProfileDraft();
      setEditorMode("create");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    } else if (pendingEditorAction.type === "edit") {
      loadQualityProfileById(pendingEditorAction.profileId);
      setEditorMode("edit");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
    }
    setPendingEditorAction(null);
  }, [loadQualityProfileById, pendingEditorAction, startNewQualityProfileDraft]);

  return (
    <>
    <form
      id="settings-quality-profiles-section"
      className="space-y-4 text-sm"
      onSubmit={(event) => {
        event.preventDefault();
      }}
    >
      <section id="settings-quality-profiles-table-card" className={QUALITY_PANEL_CLASS}>
        <div className={QUALITY_PANEL_HEADER_CLASS}>
          <h2 className={QUALITY_PANEL_TITLE_CLASS}>
            {t("settings.qualityProfiles")}
          </h2>
        </div>
        <div className="overflow-x-auto">
          <Table id="settings-quality-profiles-table">
            <TableHeader>
              <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                <TableHead className={`font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("label.name")}</TableHead>
                <TableHead className={`max-w-72 font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.qualityTiers")}</TableHead>
                <TableHead className={`w-28 font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.archivalQuality")}</TableHead>
                <TableHead className={`w-24 text-center font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.allowBdDisk")}</TableHead>
                <TableHead className={`w-24 text-center font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.allowHdr")}</TableHead>
                <TableHead className={`w-16 text-center font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.allowDv")}</TableHead>
                <TableHead className={`w-28 font-semibold ${QUALITY_MUTED_TEXT_CLASS}`}>{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {qualityProfiles.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className={`text-sm ${QUALITY_MUTED_TEXT_CLASS}`}>
                    {t("qualityProfile.noProfilesFound")}
                  </TableCell>
                </TableRow>
              ) : (
                qualityProfiles.map((profile) => (
                  <TableRow
                    data-ui="settings-table-row"
                    key={profile.id}
                    id={selectorId("settings-quality-profile-row", profile.name)}
                    className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                  >
                    <TableCell className="font-medium text-[var(--scry-ink2)]">{profile.name}</TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {(() => {
                          const criteria = getQualityProfileCriteria(profile.id) as
                            | QualityProfileCriteriaPayload
                            | undefined;
                          const tiers = dedupeOrdered(parseStringArrayValue(criteria?.quality_tiers));
                          if (tiers.length === 0) {
                            return <span className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>—</span>;
                          }
                          return tiers.map((tier) => (
                            <span
                              key={`${profile.id}-${tier}`}
                              className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-[10px] text-[var(--scry-ink2)]"
                            >
                              {getQualityTierLabel(tier)}
                            </span>
                          ));
                        })()}
                      </div>
                    </TableCell>
                    <TableCell>
                      {(() => {
                        const criteria = getQualityProfileCriteria(profile.id) as
                          | QualityProfileCriteriaPayload
                          | undefined;
                        return (
                          <span className="inline-flex rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-[10px] text-[var(--scry-ink2)]">
                            {getQualityTierLabel(
                              typeof criteria?.archival_quality === "string" &&
                                criteria?.archival_quality?.trim().length
                                ? criteria.archival_quality
                                : "2160P",
                            )}
                          </span>
                        );
                      })()}
                    </TableCell>
                    <TableCell className="text-center">
                      <RenderBooleanIcon
                        value={getQualityProfileBoolean(profile.id, "allow_bd_disk", true) as boolean}
                        label={t("qualityProfile.allowBdDisk")}
                      />
                    </TableCell>
                    <TableCell className="text-center">
                      <RenderBooleanIcon
                        value={
                          getQualityProfileBoolean(profile.id, "detected_hdr_allowed", true) as boolean
                        }
                        label={t("qualityProfile.detectedHdrAllowed")}
                      />
                    </TableCell>
                    <TableCell className="text-center">
                      <RenderBooleanIcon
                        value={
                          getQualityProfileBoolean(profile.id, "dolby_vision_allowed", true) as boolean
                        }
                        label={t("qualityProfile.dolbyVisionAllowed")}
                      />
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1">
                        <QualityProfileActionButton
                          id={selectorId("settings-quality-profile-edit", profile.id)}
                          tone="edit"
                          onClick={() => handleEditProfile(profile.id)}
                          label={t("label.edit")}
                        >
                          <Edit className="h-4 w-4" />
                        </QualityProfileActionButton>
                        {(() => {
                          const isInUse =
                            profile.id === globalQualityProfileId ||
                            Object.values(categoryQualityProfileOverrides).some(
                              (v) => v === profile.id,
                            );
                          const deleteButton = (
                            <QualityProfileActionButton
                              id={selectorId("settings-quality-profile-delete", profile.id)}
                              tone="delete"
                              disabled={qualityProfilesSaving || isInUse}
                              onClick={() => setPendingDeleteProfile({ id: profile.id, name: profile.name })}
                              label={t("label.delete")}
                              tooltip={
                                isInUse
                                  ? t("qualityProfile.deleteDisabledInUse")
                                  : undefined
                              }
                            >
                              <Trash2 className="h-4 w-4" />
                            </QualityProfileActionButton>
                          );
                          return deleteButton;
                        })()}
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </section>

      {isEditorOpen ? (
        <>
          <section id="settings-quality-profile-editor" className={QUALITY_PANEL_CLASS}>
            <div className={QUALITY_PANEL_HEADER_CLASS}>
              <h2 className={QUALITY_PANEL_TITLE_CLASS}>
                {editorMode === "create"
                  ? t("qualityProfile.createNewProfile")
                  : t("qualityProfile.editProfile")}
              </h2>
            </div>
            <div className={`${QUALITY_PANEL_BODY_CLASS} space-y-4`}>
              <div className={QUALITY_EDITOR_FIELD_CLASS}>
                <label className="block max-w-2xl">
                  <Label
                    className="mb-2 block text-[var(--scry-ink2)]"
                    htmlFor="settings-quality-profile-name"
                  >
                    {t("qualityProfile.profileNameLabel")}
                  </Label>
              <Input
                id="settings-quality-profile-name"
                value={qualityProfileDraft.name}
                onChange={(event) => updateQualityProfileDraft({ name: event.target.value })}
              />
                </label>
              </div>

              <details className={QUALITY_EDITOR_SECTION_CLASS} open>
                <summary className={QUALITY_EDITOR_SECTION_SUMMARY_CLASS}>
              {t("qualityProfile.qualityTiersAndArchival")}
            </summary>
                <div className={`${QUALITY_EDITOR_SECTION_BODY_CLASS} space-y-4`}>
            <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]">
              <div>
                <Label className="mb-2 block text-[var(--scry-ink2)]">{t("qualityProfile.qualityPreference")}</Label>
                <p className={`mb-2 text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.qualityPreferenceHelp")}</p>
                <div className={QUALITY_EDITOR_LIST_CLASS} data-quality-tier-list>
                  {activeQualityProfileTierOptions.length === 0 ? (
                    <p className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.noQualityTiersSelected")}</p>
                  ) : (
                    activeQualityProfileTierOptions.map((qualityTier, index) => (
                      <div
                        key={qualityTier}
                        className={QUALITY_EDITOR_LIST_ITEM_CLASS}
                        data-quality-tier={qualityTier}
                      >
                        <span
                          className="cursor-grab touch-none active:cursor-grabbing"
                          title={t("qualityProfile.dragQualityTier", { value: getQualityTierLabel(qualityTier) })}
                          onPointerDown={(event) => {
                            if (event.button !== 0) return;
                            event.preventDefault();
                            event.currentTarget.setPointerCapture(event.pointerId);
                          }}
                          onPointerUp={(event) => {
                            if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
                            event.currentTarget.releasePointerCapture(event.pointerId);
                            const target = document.elementFromPoint(event.clientX, event.clientY)
                              ?.closest<HTMLElement>("[data-quality-tier]");
                            if (!target || target.closest("[data-quality-tier-list]") !==
                              event.currentTarget.closest("[data-quality-tier-list]")) return;
                            const targetIndex = activeQualityProfileTierOptions.indexOf(target.dataset.qualityTier ?? "");
                            if (targetIndex >= 0) reorderQualityTier(qualityTier, targetIndex);
                          }}
                        >
                          <GripVertical className="h-4 w-4" aria-hidden="true" />
                        </span>
                        <span className="flex-1 text-xs">{getQualityTierLabel(qualityTier)}</span>
                        <Button type="button" variant="secondary" size="sm"
                          disabled={index === 0}
                          aria-label={t("qualityProfile.moveQualityTierUp", { value: getQualityTierLabel(qualityTier) })}
                          onClick={() => reorderQualityTier(qualityTier, index - 1)}>
                          <ArrowUp className="h-4 w-4" aria-hidden="true" />
                        </Button>
                        <Button type="button" variant="secondary" size="sm"
                          disabled={index === activeQualityProfileTierOptions.length - 1}
                          aria-label={t("qualityProfile.moveQualityTierDown", { value: getQualityTierLabel(qualityTier) })}
                          onClick={() => reorderQualityTier(qualityTier, index + 1)}>
                          <ArrowDown className="h-4 w-4" aria-hidden="true" />
                        </Button>
                        <Button
                          id={selectorId(
                            "settings-quality-profile-tier-remove",
                            qualityTier,
                          )}
                          type="button"
                          variant="destructive"
                          size="sm"
                          onClick={() => removeQualityTier(qualityTier)}
                          aria-label={t("qualityProfile.removeQualityTier", {
                            value: getQualityTierLabel(qualityTier),
                          })}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    ))
                  )}
                </div>
              </div>
              <div className="hidden items-center justify-center md:flex">
                <div className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3 text-[var(--scry-ink2)] shadow-md">
                  <ArrowLeft className="h-6 w-6" />
                </div>
              </div>
              <div>
                <Label className="mb-2 block text-[var(--scry-ink2)]">{t("qualityProfile.availableQualityTiers")}</Label>
                <div className={QUALITY_EDITOR_LIST_CLASS}>
                  {availableQualityTiers.length === 0 ? (
                    <p className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.allQualityTiersSelected")}</p>
                  ) : (
                    availableQualityTiers.map((option) => (
                      <div
                        key={option.value}
                        className={QUALITY_EDITOR_LIST_ITEM_CLASS}
                      >
                        <span className="text-xs">{option.label}</span>
                        <Button
                          id={selectorId(
                            "settings-quality-profile-tier-add",
                            option.value,
                          )}
                          type="button"
                          variant="secondary"
                          size="sm"
                          onClick={() => addQualityTier(option.value)}
                          className="border border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)] hover:border-[var(--scry-success-border-strong)] hover:bg-[var(--scry-success-bg-strong)] hover:text-[var(--scry-success-text)]"
                          aria-label={t("qualityProfile.addQualityTier", { value: option.label })}
                        >
                          <Plus className="h-4 w-4" />
                        </Button>
                      </div>
                    ))
                  )}
                </div>
              </div>
            </div>
            <div className="max-w-md">
              <label>
                <Label className="mb-2 block">
                  <span className="inline-flex items-center gap-2">
                    {t("qualityProfile.archivalQuality")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.archivalQuality")}
                      text={t("qualityProfile.archivalQualityInfo")}
                    />
                  </span>
                </Label>
                <Select value={qualityProfileDraft.archival_quality || "__default__"} onValueChange={(v) => updateQualityProfileDraft({ archival_quality: v === "__default__" ? "" : v })}>
                  <SelectTrigger
                    id="settings-quality-profile-archival-quality"
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {archivalQualityOptions.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            </div>
                </div>
              </details>

              <details className={QUALITY_EDITOR_SECTION_CLASS} open>
                <summary className={QUALITY_EDITOR_SECTION_SUMMARY_CLASS}>
              <span className="inline-flex items-center gap-2">
                {t("qualityProfile.scoringAndPreferences")}
                <InfoHelp
                  ariaLabel={t("qualityProfile.scoringAndPreferences")}
                  text={t("qualityProfile.scoringAndPreferencesInfo")}
                />
              </span>
            </summary>
                <div className={`${QUALITY_EDITOR_SECTION_BODY_CLASS} space-y-4`}>
              {/* Preferences */}
              <div className="space-y-3">
                <label className="mb-2 flex items-center gap-3">
                  <Checkbox
                    checked={qualityProfileDraft.allow_unknown_quality}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        allow_unknown_quality: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.allowUnknownQuality")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.allowUnknownQuality")}
                      text={t("qualityProfile.allowUnknownQualityInfo")}
                    />
                  </span>
                </label>
                <div className="space-y-3">
                  <label className="mb-2 flex items-center gap-3">
                    <Checkbox
                      checked={qualityProfileDraft.detected_hdr_allowed}
                      onCheckedChange={(checked) =>
                        updateQualityProfileDraft({
                          detected_hdr_allowed: checked === true,
                          ...(checked === true ? {} : { dolby_vision_allowed: false }),
                        })
                      }
                    />
                    <span className="inline-flex items-center gap-2 text-sm">
                      {t("qualityProfile.detectedHdrAllowed")}
                      <InfoHelp
                        ariaLabel={t("qualityProfile.detectedHdrAllowed")}
                        text={t("qualityProfile.detectedHdrAllowedInfo")}
                      />
                    </span>
                  </label>
                  <div
                    className={`ml-8 flex items-center gap-3 ${
                      qualityProfileDraft.detected_hdr_allowed ? "" : "opacity-60"
                    }`}
                  >
                    <Checkbox
                      checked={
                        qualityProfileDraft.detected_hdr_allowed
                          ? qualityProfileDraft.dolby_vision_allowed
                          : false
                      }
                      onCheckedChange={(checked) =>
                        updateQualityProfileDraft({
                          dolby_vision_allowed: checked === true,
                        })
                      }
                      disabled={!qualityProfileDraft.detected_hdr_allowed}
                      aria-disabled={!qualityProfileDraft.detected_hdr_allowed}
                    />
                    <span className="inline-flex items-center gap-2 text-sm">
                      {t("qualityProfile.dolbyVisionAllowed")}
                      <InfoHelp
                        ariaLabel={t("qualityProfile.dolbyVisionAllowed")}
                        text={t("qualityProfile.dolbyVisionInfo")}
                      />
                    </span>
                  </div>
                </div>
                <label className="mb-2 flex items-center gap-3">
                  <Checkbox
                    checked={qualityProfileDraft.prefer_remux}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        prefer_remux: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.preferRemux")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.preferRemux")}
                      text={t("qualityProfile.preferRemuxInfo")}
                    />
                  </span>
                </label>
                <label className="mb-2 flex items-center gap-3">
                  <Checkbox
                    id={selectorId("settings-quality-profile-allow-bd-disk")}
                    checked={qualityProfileDraft.allow_bd_disk}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        allow_bd_disk: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.allowBdDisk")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.allowBdDisk")}
                      text={t("qualityProfile.allowBdDiskInfo")}
                    />
                  </span>
                </label>
              </div>

              {/* Scoring overrides */}
              <details
                id={selectorId("settings-quality-profile-scoring-overrides")}
                className="overflow-hidden rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)]"
              >
                <summary
                  id="settings-quality-profile-scoring-overrides-summary"
                  className={`cursor-pointer select-none px-3 py-2 text-xs font-medium ${QUALITY_MUTED_TEXT_CLASS}`}
                >
                  <span className="inline-flex items-center gap-2">
                    {t("qualityProfile.scoringOverrides")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.scoringOverrides")}
                      text={t("qualityProfile.scoringOverridesInfo")}
                    />
                  </span>
                </summary>
                <div className="space-y-3 border-t border-[var(--scry-border3)] p-3">
                  {([
                    ["allow_x265_non4k", "qualityProfile.overrideAllowX265Non4k", "qualityProfile.overrideAllowX265Non4kInfo"],
                    ["block_dv_without_fallback", "qualityProfile.overrideBlockDvNoFallback", "qualityProfile.overrideBlockDvNoFallbackInfo"],
                    ["prefer_compact_encodes", "qualityProfile.overridePreferCompact", "qualityProfile.overridePreferCompactInfo"],
                    ["prefer_lossless_audio", "qualityProfile.overridePreferLossless", "qualityProfile.overridePreferLosslessInfo"],
                    ["block_upscaled", "qualityProfile.overrideBlockUpscaled", "qualityProfile.overrideBlockUpscaledInfo"],
                  ] as const).map(([key, labelKey, infoKey]) => {
                    const explicitValue = qualityProfileDraft.scoring_overrides[key as keyof ScoringOverridesPayload];
                    const personaDefault = PERSONA_OVERRIDE_DEFAULTS[globalScoringPersonaDraft]?.[key] ?? false;
                    const effectiveValue = explicitValue ?? personaDefault;
                    return (
                      <div key={key} className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
                        <Select
                          value={effectiveValue ? "true" : "false"}
                          onValueChange={(v) => {
                            const newValue = v === "true";
                            const nextOverrides = { ...qualityProfileDraft.scoring_overrides };
                            if (newValue === personaDefault) {
                              delete nextOverrides[key as keyof ScoringOverridesPayload];
                            } else {
                              (nextOverrides as Record<string, boolean>)[key] = newValue;
                            }
                            updateQualityProfileDraft({ scoring_overrides: nextOverrides });
                          }}
                        >
                          <SelectTrigger
                            id={selectorId(
                              "settings-quality-profile-scoring-override",
                              key,
                            )}
                            className="w-28 shrink-0"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem
                              id={selectorId(
                                "settings-quality-profile-scoring-override-option",
                                key,
                                "true",
                              )}
                              value="true"
                            >
                              {t("label.yes")}
                            </SelectItem>
                            <SelectItem
                              id={selectorId(
                                "settings-quality-profile-scoring-override-option",
                                key,
                                "false",
                              )}
                              value="false"
                            >
                              {t("label.no")}
                            </SelectItem>
                          </SelectContent>
                        </Select>
                        <span className="inline-flex items-center gap-2 text-sm">
                          {t(labelKey)}
                          <InfoHelp ariaLabel={t(labelKey)} text={t(infoKey)} />
                        </span>
                      </div>
                    );
                  })}
                </div>
              </details>

              {/* Upgrade behavior */}
              <div className="space-y-3">
                <label className="mb-2 flex items-center gap-3">
                  <Checkbox
                    checked={qualityProfileDraft.allow_upgrades}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        allow_upgrades: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.allowUpgrades")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.allowUpgrades")}
                      text={t("qualityProfile.allowUpgradesInfo")}
                    />
                  </span>
                </label>
                <div className="grid gap-3 md:grid-cols-2">
                  <label className="space-y-2">
                    <Label className="inline-flex items-center gap-2">
                      {t("qualityProfile.cutoffTier")}
                      <InfoHelp
                        ariaLabel={t("qualityProfile.cutoffTier")}
                        text={t("qualityProfile.cutoffTierInfo")}
                      />
                    </Label>
                    <Select
                      value={qualityProfileDraft.cutoff_tier || "__none__"}
                      onValueChange={(v) =>
                        updateQualityProfileDraft({ cutoff_tier: v === "__none__" ? "" : v })
                      }
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__none__">{t("qualityProfile.cutoffNone")}</SelectItem>
                        {activeQualityProfileTierOptions.map((tier) => (
                          <SelectItem key={tier} value={tier}>
                            {getQualityTierLabel(tier)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>

                  <label className="space-y-2">
                    <Label className="inline-flex items-center gap-2">
                      {t("qualityProfile.minScoreToGrab")}
                      <InfoHelp
                        ariaLabel={t("qualityProfile.minScoreToGrab")}
                        text={t("qualityProfile.minScoreToGrabInfo")}
                      />
                    </Label>
                    <Input
                      {...signedIntegerInputProps}
                      placeholder={t("qualityProfile.minScorePlaceholder")}
                      value={qualityProfileDraft.min_score_to_grab ?? ""}
                      onChange={(event) => {
                        const raw = event.target.value.trim();
                        updateQualityProfileDraft({
                          min_score_to_grab: raw === "" ? null : Number(raw),
                        });
                      }}
                    />
                  </label>
                </div>
              </div>
            </div>
          </details>

          <div className="grid gap-3">
            <ProfileListEditor
              title={t("qualityProfile.sourceAllowlist")}
              allowed={activeSourceAllowlist}
              denied={activeSourceBlocklist}
              choices={availableSourceAllowlist}
              onMoveToAllowed={(value) =>
                moveProfileListToAllowed("source_allowlist", "source_blocklist", value)
              }
              onMoveToDenied={(value) =>
                moveProfileListToDenied("source_allowlist", "source_blocklist", value)
              }
              emptyStateMessage={t("qualityProfile.sourceBlocklistDefault")}
              info={t("qualityProfile.sourceAllowlistInfo")}
            />
            <ProfileListEditor
              title={t("qualityProfile.videoCodecAllowlist")}
              allowed={activeVideoCodecAllowlist}
              denied={activeVideoCodecBlocklist}
              choices={availableVideoCodecAllowlist}
              onMoveToAllowed={(value) =>
                moveProfileListToAllowed("video_codec_allowlist", "video_codec_blocklist", value)
              }
              onMoveToDenied={(value) =>
                moveProfileListToDenied("video_codec_allowlist", "video_codec_blocklist", value)
              }
              emptyStateMessage={t("qualityProfile.videoCodecBlocklistDefault")}
              info={t("qualityProfile.videoCodecAllowlistInfo")}
            />
            <ProfileListEditor
              title={t("qualityProfile.audioCodecAllowlist")}
              allowed={activeAudioCodecAllowlist}
              denied={activeAudioCodecBlocklist}
              choices={availableAudioCodecAllowlist}
              onMoveToAllowed={(value) =>
                moveProfileListToAllowed("audio_codec_allowlist", "audio_codec_blocklist", value)
              }
              onMoveToDenied={(value) =>
                moveProfileListToDenied("audio_codec_allowlist", "audio_codec_blocklist", value)
              }
              emptyStateMessage={t("qualityProfile.audioCodecBlocklistDefault")}
              info={t("qualityProfile.audioCodecAllowlistInfo")}
            />
          </div>

          {qualityProfileParseError ? (
            <p className="rounded-[10px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-xs text-[var(--scry-danger-text)]">
              {qualityProfileParseError}
            </p>
          ) : null}

          <div className="-mx-4 -mb-4 border-t border-[var(--scry-border3)] bg-[var(--scry-inset)] px-4 py-3 sm:-mx-5 sm:-mb-5 sm:px-5">
            <div className="flex justify-end gap-2">
              <Button
                id="settings-quality-profile-cancel"
                type="button"
                variant="secondary"
                onClick={handleCloseProfileEditor}
                disabled={mediaSettingsLoading || qualityProfilesSaving}
              >
                {t("label.cancel")}
              </Button>
              <Button
                id="settings-quality-profile-save"
                type="button"
                onClick={() => void handleSaveQualityProfile()}
                disabled={mediaSettingsLoading || qualityProfilesSaving}
              >
                {qualityProfilesSaving ? t("label.saving") : t("label.save")}
              </Button>
            </div>
          </div>
        </div>
      </section>
      {editorMode === "edit" ? (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-quality-profile-create"
            icon={Plus}
            label={t("qualityProfile.createNewProfile")}
            onClick={handleStartCreateProfile}
            disabled={mediaSettingsLoading || qualityProfilesSaving}
          />
        </div>
      ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-quality-profile-create"
            icon={Plus}
            label={t("qualityProfile.createNewProfile")}
            onClick={handleStartCreateProfile}
            disabled={mediaSettingsLoading || qualityProfilesSaving}
          />
        </div>
      )}

      <section id="settings-quality-profiles-defaults-card" className={QUALITY_PANEL_CLASS}>
        <div className={QUALITY_PANEL_HEADER_CLASS}>
          <h2 className={QUALITY_PANEL_TITLE_CLASS}>
            {t("qualityProfile.defaultCategoryProfiles")}
          </h2>
        </div>
        <div className={`${QUALITY_PANEL_BODY_CLASS} space-y-6`}>
          <label className="space-y-2">
            <Label className="inline-flex items-center gap-2">
              {t("settings.qualityProfileGlobalLabel")}
              <InfoHelp
                text={t("settings.qualityProfileGlobalHelp")}
                ariaLabel={t("settings.qualityProfileGlobalHelp")}
              />
            </Label>
            <Select
              value={globalQualityProfileDraft}
              onValueChange={handleGlobalProfileChange}
              disabled={mediaSettingsLoading || qualityProfilesSaving}
            >
              <SelectTrigger
                id="settings-quality-profile-global"
                className="w-full"
                onBlur={handleGlobalProfileBlur}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem
                    key={opt.value}
                    id={selectorId("settings-quality-profile-global-option", opt.value)}
                    value={opt.value}
                  >
                    {opt.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>

          <label>
            <Label className="mb-2 inline-flex items-center gap-2">
              {t("qualityProfile.scoringPersona")}
              <InfoHelp
                text={t("qualityProfile.scoringPersonaInfo")}
                ariaLabel={t("qualityProfile.scoringPersona")}
              />
            </Label>
            <Select
              value={globalScoringPersonaDraft}
              onValueChange={handleGlobalScoringPersonaChange}
              disabled={mediaSettingsLoading || qualityProfilesSaving}
            >
              <SelectTrigger
                id="settings-quality-profile-global-persona"
                className="w-full"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem
                  id={selectorId("settings-quality-profile-global-persona-option", "Balanced")}
                  value="BALANCED"
                >
                  {t("qualityProfile.personaBalanced")}
                </SelectItem>
                <SelectItem
                  id={selectorId("settings-quality-profile-global-persona-option", "Audiophile")}
                  value="AUDIOPHILE"
                >
                  {t("qualityProfile.personaAudiophile")}
                </SelectItem>
                <SelectItem
                  id={selectorId("settings-quality-profile-global-persona-option", "Efficient")}
                  value="EFFICIENT"
                >
                  {t("qualityProfile.personaEfficient")}
                </SelectItem>
                <SelectItem
                  id={selectorId("settings-quality-profile-global-persona-option", "Compatible")}
                  value="COMPATIBLE"
                >
                  {t("qualityProfile.personaCompatible")}
                </SelectItem>
              </SelectContent>
            </Select>
          </label>

          <div className="space-y-5">
            <h3 className="inline-flex items-center gap-2 text-base font-semibold text-[var(--scry-ink2)]">
              {t("settings.qualityProfileOverridesLabel")}
              <InfoHelp
                text={t("settings.qualityProfileOverrideHelp")}
                ariaLabel={t("settings.qualityProfileOverrideHelp")}
              />
            </h3>
            <div className="hidden gap-2 sm:grid sm:grid-cols-2">
              <span className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.editProfile")}</span>
              <span className={`text-xs ${QUALITY_MUTED_TEXT_CLASS}`}>{t("qualityProfile.scoringPersona")}</span>
            </div>
            {Object.keys(qualityCategoryLabels).map((scopeKey) => {
              const scopeId = scopeKey as ViewCategoryId;
              const overridePersona = categoryPersonaDrafts[scopeId];
              return (
                <div key={scopeId} className="space-y-2">
                  <Label className="text-[var(--scry-ink2)]">{qualityCategoryLabels[scopeId]}</Label>
                  <div className="grid gap-2 sm:grid-cols-2">
                    <Select
                      value={categoryQualityProfileDrafts[scopeId]}
                      onValueChange={(v) =>
                        handleCategoryProfileOverrideChange(scopeId, v)
                      }
                      disabled={
                        mediaSettingsLoading ||
                        categoryQualityProfileSaving[scopeId]
                      }
                    >
                      <SelectTrigger
                        id={selectorId("settings-quality-profile-override", scopeId)}
                        className="w-full"
                        onBlur={() => handleCategoryProfileOverrideBlur(scopeId)}
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {[
                          {
                            value: qualityProfileInheritValue,
                            label: t("settings.qualityProfileInheritLabel"),
                          },
                          ...toProfileOptions(qualityProfiles),
                        ].map((opt) => (
                          <SelectItem
                            key={opt.value}
                            id={selectorId(
                              "settings-quality-profile-override",
                              scopeId,
                              "option",
                              opt.value,
                            )}
                            value={opt.value}
                          >
                            {opt.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Select
                      value={overridePersona}
                      onValueChange={(v) => handleCategoryPersonaChange(scopeId, v)}
                      disabled={
                        mediaSettingsLoading ||
                        categoryQualityProfileSaving[scopeId]
                      }
                    >
                      <SelectTrigger
                        id={selectorId("settings-quality-profile-persona", scopeId)}
                        className="w-full"
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem
                          id={selectorId(
                            "settings-quality-profile-persona",
                            scopeId,
                            "option",
                            "__default__",
                          )}
                          value="__default__"
                        >
                          {t("qualityProfile.facetPersonaUseDefault")}
                        </SelectItem>
                        <SelectItem
                          id={selectorId(
                            "settings-quality-profile-persona",
                            scopeId,
                            "option",
                            "Balanced",
                          )}
                          value="BALANCED"
                        >
                          {t("qualityProfile.personaBalanced")}
                        </SelectItem>
                        <SelectItem
                          id={selectorId(
                            "settings-quality-profile-persona",
                            scopeId,
                            "option",
                            "Audiophile",
                          )}
                          value="AUDIOPHILE"
                        >
                          {t("qualityProfile.personaAudiophile")}
                        </SelectItem>
                        <SelectItem
                          id={selectorId(
                            "settings-quality-profile-persona",
                            scopeId,
                            "option",
                            "Efficient",
                          )}
                          value="EFFICIENT"
                        >
                          {t("qualityProfile.personaEfficient")}
                        </SelectItem>
                        <SelectItem
                          id={selectorId(
                            "settings-quality-profile-persona",
                            scopeId,
                            "option",
                            "Compatible",
                          )}
                          value="COMPATIBLE"
                        >
                          {t("qualityProfile.personaCompatible")}
                        </SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </section>
    </form>
    <ConfirmDialog
      open={pendingEditorAction !== null}
      title={t("qualityProfile.confirmDiscardTitle")}
      description={t("qualityProfile.confirmDiscardDescription")}
      confirmLabel={
        pendingEditorAction?.type === "create"
          ? t("qualityProfile.createNewProfile")
          : pendingEditorAction?.type === "edit"
            ? t("label.edit")
            : t("label.discard")
      }
      cancelLabel={t("label.cancel")}
      confirmButtonId="settings-quality-profile-editor-action-confirm"
      cancelButtonId="settings-quality-profile-editor-action-cancel"
      isBusy={qualityProfilesSaving}
      onConfirm={confirmPendingEditorAction}
      onCancel={() => setPendingEditorAction(null)}
    />
    <ConfirmDialog
      open={pendingDeleteProfile !== null}
      title={t("qualityProfile.confirmDeleteTitle")}
      description={
        pendingDeleteProfile
          ? t("qualityProfile.confirmDeleteDescription", { name: pendingDeleteProfile.name })
          : ""
      }
      confirmLabel={t("label.delete")}
      cancelLabel={t("label.cancel")}
      isBusy={qualityProfilesSaving}
      onConfirm={async () => {
        if (pendingDeleteProfile) {
          await deleteQualityProfile(pendingDeleteProfile.id);
          setPendingDeleteProfile(null);
        }
      }}
      onCancel={() => setPendingDeleteProfile(null)}
    />
    </>
  );
}
