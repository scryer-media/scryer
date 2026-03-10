
import * as React from "react";
import { ArrowLeft, ArrowRight, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
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

type ViewCategoryId = "movie" | "series" | "anime";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type ScoringPersonaId = "Balanced" | "Audiophile" | "Efficient" | "Compatible";

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
  atmos_preferred: boolean;
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  prefer_dual_audio: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_persona: ScoringPersonaId;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string | null;
  min_score_to_grab: number | null;
  facet_persona_overrides: Record<string, ScoringPersonaId>;
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
  atmos_preferred: boolean;
  dolby_vision_allowed: boolean;
  detected_hdr_allowed: boolean;
  prefer_remux: boolean;
  prefer_dual_audio: boolean;
  allow_bd_disk: boolean;
  allow_upgrades: boolean;
  scoring_persona: ScoringPersonaId;
  scoring_overrides: ScoringOverridesPayload;
  cutoff_tier: string;
  min_score_to_grab: number | null;
  facet_persona_overrides: Record<string, ScoringPersonaId>;
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
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => Array<{ value: string; label: string }>;
  globalQualityProfileId: string;
  setGlobalQualityProfileId: (value: string) => void;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  setCategoryQualityProfileOverrides: React.Dispatch<
    React.SetStateAction<Record<ViewCategoryId, string>>
  >;
  mediaSettingsLoading: boolean;
  initialLoadComplete: boolean;
  qualityProfilesSaving: boolean;
  updateQualityProfilesGlobal: (event?: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  categoryQualityProfileSaving: Record<ViewCategoryId, boolean>;
  saveCategoryQualityProfile: (scopeId: ViewCategoryId, value: string) => Promise<void> | void;
  saveGlobalQualityProfile: (value: string) => Promise<void> | void;
  archivalQualityOptions: Array<{ value: string; label: string }>;
  deleteQualityProfile: (profileId: string) => Promise<void>;
};

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
    <details className="rounded-xl border border-border bg-card p-3">
      <summary className="cursor-pointer select-none text-sm font-medium text-card-foreground">
        <span className="inline-flex items-center gap-2">
          <span>{title}</span>
          {info ? (
            <InfoHelp text={info} ariaLabel={t("qualityProfile.aboutSection", { title })} />
          ) : null}
        </span>
      </summary>
      <div className="mt-3 grid gap-3 md:grid-cols-2">
        <div>
          <Label className="mb-2 block">Allowed</Label>
          <div className="max-h-56 overflow-auto rounded border border-border p-2">
            {sortedAllowed.length === 0 ? (
              <p className="text-xs text-muted-foreground">{t("qualityProfile.noSelectedItems")}</p>
            ) : (
              sortedAllowed.map((option) => (
                <div
                  key={option.value}
                  className="mb-1 flex items-center justify-between rounded border border-emerald-400 bg-card px-2 py-1.5 border-opacity-35 hover:border-opacity-60 ring-1 ring-inset ring-emerald-500/30"
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
          <Label className="mb-2 block">Denied</Label>
          <div className="max-h-56 overflow-auto rounded border border-border p-2">
            {sortedDenied.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                {emptyStateMessage ?? t("qualityProfile.noSelectedItems")}
              </p>
            ) : (
              sortedDenied.map((option) => (
                <div
                  key={option.value}
                  className="mb-1 flex items-center justify-between rounded border border-rose-500 bg-card px-2 py-1.5 border-opacity-35 hover:border-opacity-60 ring-1 ring-inset ring-rose-500/30"
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
  qualityProfileInheritValue,
  toProfileOptions,
  globalQualityProfileId,
  setGlobalQualityProfileId,
  categoryQualityProfileOverrides,
  setCategoryQualityProfileOverrides,
  mediaSettingsLoading,
  initialLoadComplete,
  qualityProfilesSaving,
  updateQualityProfilesGlobal,
  categoryQualityProfileSaving,
  saveCategoryQualityProfile,
  saveGlobalQualityProfile,
  archivalQualityOptions,
  deleteQualityProfile,
}: SettingsQualityProfilesSectionProps) {
  const t = useTranslate();
  const [globalQualityProfileDraft, setGlobalQualityProfileDraft] = React.useState(
    globalQualityProfileId,
  );
  const [categoryQualityProfileDrafts, setCategoryQualityProfileDrafts] = React.useState<
    Record<ViewCategoryId, string>
  >(categoryQualityProfileOverrides);

  React.useEffect(() => {
    setGlobalQualityProfileDraft(globalQualityProfileId);
  }, [globalQualityProfileId]);

  React.useEffect(() => {
    setCategoryQualityProfileDrafts(categoryQualityProfileOverrides);
  }, [categoryQualityProfileOverrides]);

  const handleCategoryProfileOverrideChange = React.useCallback(
    (scopeId: ViewCategoryId, rawValue: string) => {
      if (!initialLoadComplete) return;
      const normalized = rawValue.trim();
      if (categoryQualityProfileDrafts[scopeId] === normalized) return;
      setCategoryQualityProfileDrafts((previous) => ({
        ...previous,
        [scopeId]: normalized,
      }));
    },
    [initialLoadComplete, categoryQualityProfileDrafts],
  );

  const handleGlobalProfileChange = React.useCallback(
    (rawValue: string) => {
      if (!initialLoadComplete) return;
      const normalized = rawValue.trim();
      if (globalQualityProfileDraft === normalized) return;
      setGlobalQualityProfileDraft(normalized);
    },
    [initialLoadComplete, globalQualityProfileDraft],
  );

  const handleGlobalProfileBlur = React.useCallback(() => {
    if (!initialLoadComplete) return;
    if (globalQualityProfileDraft === globalQualityProfileId) return;
    setGlobalQualityProfileId(globalQualityProfileDraft);
    void saveGlobalQualityProfile(globalQualityProfileDraft);
  }, [
    initialLoadComplete,
    globalQualityProfileDraft,
    globalQualityProfileId,
    saveGlobalQualityProfile,
    setGlobalQualityProfileId,
  ]);

  const handleCategoryProfileOverrideBlur = React.useCallback(
    (scopeId: ViewCategoryId) => {
      if (!initialLoadComplete) return;
      const draftValue = categoryQualityProfileDrafts[scopeId];
      const persistedValue = categoryQualityProfileOverrides[scopeId];
      if (draftValue === persistedValue) return;
      setCategoryQualityProfileOverrides((previous) => ({
        ...previous,
        [scopeId]: draftValue,
      }));
      void saveCategoryQualityProfile(scopeId, draftValue);
    },
    [
      initialLoadComplete,
      categoryQualityProfileDrafts,
      categoryQualityProfileOverrides,
      saveCategoryQualityProfile,
      setCategoryQualityProfileOverrides,
    ],
  );

  return (
    <form
      className="space-y-4 text-sm"
      onSubmit={(event) => {
        event.preventDefault();
      }}
    >
      <div className="space-y-2">
        <div className="rounded border border-border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.name")}</TableHead>
                <TableHead className="max-w-72">{t("qualityProfile.qualityTiers")}</TableHead>
                <TableHead className="w-28">{t("qualityProfile.archivalQuality")}</TableHead>
                <TableHead className="w-24 text-center">{t("qualityProfile.allowBdDisk")}</TableHead>
                <TableHead className="w-24 text-center">{t("qualityProfile.allowHdr")}</TableHead>
                <TableHead className="w-16 text-center">{t("qualityProfile.allowDv")}</TableHead>
                <TableHead className="w-28">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {qualityProfiles.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-sm text-muted-foreground">
                    {t("qualityProfile.noProfilesFound")}
                  </TableCell>
                </TableRow>
              ) : (
                qualityProfiles.map((profile) => (
                  <TableRow key={profile.id}>
                    <TableCell>{profile.name}</TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {(() => {
                          const criteria = getQualityProfileCriteria(profile.id) as
                            | QualityProfileCriteriaPayload
                            | undefined;
                          const tiers = dedupeOrdered(parseStringArrayValue(criteria?.quality_tiers)).sort(
                            sortStringByNumericDesc,
                          );
                          if (tiers.length === 0) {
                            return <span className="text-xs text-muted-foreground">—</span>;
                          }
                          return tiers.map((tier) => (
                            <span
                              key={`${profile.id}-${tier}`}
                              className="rounded border border-border bg-muted px-2 py-0.5 text-[10px] text-card-foreground"
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
                          <span className="inline-flex rounded border border-border bg-muted px-2 py-0.5 text-[10px] text-card-foreground">
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
                        <Button
                          type="button"
                          size="sm"
                          variant="secondary"
                          onClick={() => loadQualityProfileById(profile.id)}
                        >
                          {t("label.load")}
                        </Button>
                        <Button
                          type="button"
                          size="sm"
                          variant="destructive"
                          disabled={
                            qualityProfilesSaving ||
                            profile.id === globalQualityProfileId ||
                            Object.values(categoryQualityProfileOverrides).some(
                              (v) => v === profile.id,
                            )
                          }
                          onClick={() => void deleteQualityProfile(profile.id)}
                          aria-label={t("label.delete")}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">{t("qualityProfile.editProfile")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3">
            <label>
              <Label className="mb-2 block">{t("qualityProfile.profileNameLabel")}</Label>
              <Input
                value={qualityProfileDraft.name}
                onChange={(event) => updateQualityProfileDraft({ name: event.target.value })}
              />
            </label>
          </div>

          <details className="rounded-xl border border-border bg-card p-3" open>
            <summary className="cursor-pointer select-none text-sm font-medium text-card-foreground">
              {t("qualityProfile.qualityTiersAndArchival")}
            </summary>
            <div className="mt-3 grid gap-3 md:grid-cols-[1fr_auto_1fr]">
              <div>
                <Label className="mb-2 block">{t("qualityProfile.allowedQualityTiers")}</Label>
                <div className="max-h-56 overflow-auto rounded border border-border p-2">
                  {activeQualityProfileTierOptions.length === 0 ? (
                    <p className="text-xs text-muted-foreground">{t("qualityProfile.noQualityTiersSelected")}</p>
                  ) : (
                    activeQualityProfileTierOptions.map((qualityTier) => (
                      <div
                        key={qualityTier}
                        className="mb-1 flex items-center justify-between rounded border border-transparent bg-card px-2 py-1.5 hover:border-border"
                      >
                        <span className="text-xs">{getQualityTierLabel(qualityTier)}</span>
                        <Button
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
                <div className="rounded-full border border-border bg-muted/80 p-3 text-card-foreground shadow-md">
                  <ArrowLeft className="h-6 w-6" />
                </div>
              </div>
              <div>
                <Label className="mb-2 block">{t("qualityProfile.availableQualityTiers")}</Label>
                <div className="max-h-56 overflow-auto rounded border border-border p-2">
                  {availableQualityTiers.length === 0 ? (
                    <p className="text-xs text-muted-foreground">{t("qualityProfile.allQualityTiersSelected")}</p>
                  ) : (
                    availableQualityTiers.map((option) => (
                      <div
                        key={option.value}
                        className="mb-1 flex items-center justify-between rounded border border-transparent bg-card px-2 py-1.5 hover:border-border"
                      >
                        <span className="text-xs">{option.label}</span>
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          onClick={() => addQualityTier(option.value)}
                          className="bg-emerald-600 dark:bg-emerald-700 text-emerald-800 dark:text-emerald-100 hover:bg-emerald-600 hover:text-foreground"
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
            <div className="mt-3">
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
                  <SelectTrigger className="w-full">
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
          </details>

          <details className="rounded-xl border border-border bg-card p-3" open>
            <summary className="cursor-pointer select-none text-sm font-medium text-card-foreground">
              <span className="inline-flex items-center gap-2">
                {t("qualityProfile.scoringAndPreferences")}
                <InfoHelp
                  ariaLabel={t("qualityProfile.scoringAndPreferences")}
                  text={t("qualityProfile.scoringAndPreferencesInfo")}
                />
              </span>
            </summary>
            <div className="mt-3 space-y-4">
              {/* Persona */}
              <label className="space-y-2">
                <Label className="inline-flex items-center gap-2">
                  {t("qualityProfile.scoringPersona")}
                  <InfoHelp
                    ariaLabel={t("qualityProfile.scoringPersona")}
                    text={t("qualityProfile.scoringPersonaInfo")}
                  />
                </Label>
                <Select
                  value={qualityProfileDraft.scoring_persona}
                  onValueChange={(v) =>
                    updateQualityProfileDraft({ scoring_persona: v as ScoringPersonaId, scoring_overrides: {} })
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="Balanced">{t("qualityProfile.personaBalanced")}</SelectItem>
                    <SelectItem value="Audiophile">{t("qualityProfile.personaAudiophile")}</SelectItem>
                    <SelectItem value="Efficient">{t("qualityProfile.personaEfficient")}</SelectItem>
                    <SelectItem value="Compatible">{t("qualityProfile.personaCompatible")}</SelectItem>
                  </SelectContent>
                </Select>
              </label>

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
                    checked={qualityProfileDraft.atmos_preferred}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        atmos_preferred: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.atmosPreferred")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.atmosPreferred")}
                      text={t("qualityProfile.atmosPreferredInfo")}
                    />
                  </span>
                </label>
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
                    checked={qualityProfileDraft.prefer_dual_audio}
                    onCheckedChange={(checked) =>
                      updateQualityProfileDraft({
                        prefer_dual_audio: checked === true,
                      })
                    }
                  />
                  <span className="inline-flex items-center gap-2 text-sm">
                    {t("qualityProfile.preferDualAudio")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.preferDualAudio")}
                      text={t("qualityProfile.preferDualAudioInfo")}
                    />
                  </span>
                </label>
                <label className="mb-2 flex items-center gap-3">
                  <Checkbox
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
              <details className="rounded-lg border border-border/50 p-2">
                <summary className="cursor-pointer select-none text-xs font-medium text-muted-foreground">
                  <span className="inline-flex items-center gap-2">
                    {t("qualityProfile.scoringOverrides")}
                    <InfoHelp
                      ariaLabel={t("qualityProfile.scoringOverrides")}
                      text={t("qualityProfile.scoringOverridesInfo")}
                    />
                  </span>
                </summary>
                <div className="mt-3 space-y-3">
                  {([
                    ["allow_x265_non4k", "qualityProfile.overrideAllowX265Non4k", "qualityProfile.overrideAllowX265Non4kInfo"],
                    ["block_dv_without_fallback", "qualityProfile.overrideBlockDvNoFallback", "qualityProfile.overrideBlockDvNoFallbackInfo"],
                    ["prefer_compact_encodes", "qualityProfile.overridePreferCompact", "qualityProfile.overridePreferCompactInfo"],
                    ["prefer_lossless_audio", "qualityProfile.overridePreferLossless", "qualityProfile.overridePreferLosslessInfo"],
                    ["block_upscaled", "qualityProfile.overrideBlockUpscaled", "qualityProfile.overrideBlockUpscaledInfo"],
                  ] as const).map(([key, labelKey, infoKey]) => {
                    const explicitValue = qualityProfileDraft.scoring_overrides[key as keyof ScoringOverridesPayload];
                    const personaDefault = PERSONA_OVERRIDE_DEFAULTS[qualityProfileDraft.scoring_persona]?.[key] ?? false;
                    const effectiveValue = explicitValue ?? personaDefault;
                    return (
                      <div key={key} className="flex items-center gap-3">
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
                          <SelectTrigger className="w-28 shrink-0">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="true">{t("label.yes")}</SelectItem>
                            <SelectItem value="false">{t("label.no")}</SelectItem>
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
                      type="number"
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

          <div className="space-y-3">
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

          <div className="flex justify-end">
            <Button
              type="button"
              onClick={() => void updateQualityProfilesGlobal()}
              disabled={mediaSettingsLoading || qualityProfilesSaving}
            >
              {qualityProfilesSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>

          {qualityProfileParseError ? (
            <p className="rounded border border-rose-500/60 bg-rose-500/10 p-2 text-xs text-rose-300">
              {qualityProfileParseError}
            </p>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">{t("qualityProfile.defaultCategoryProfiles")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-6">
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
              <SelectTrigger className="w-full" onBlur={handleGlobalProfileBlur}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>

          <div className="space-y-5">
            <CardTitle className="inline-flex items-center gap-2 text-base">
              {t("settings.qualityProfileOverridesLabel")}
              <InfoHelp
                text={t("settings.qualityProfileOverrideHelp")}
                ariaLabel={t("settings.qualityProfileOverrideHelp")}
              />
            </CardTitle>
            <div className="hidden gap-2 sm:grid sm:grid-cols-2">
              <span className="text-xs text-muted-foreground">{t("qualityProfile.editProfile")}</span>
              <span className="text-xs text-muted-foreground">{t("qualityProfile.scoringPersona")}</span>
            </div>
            {Object.keys(qualityCategoryLabels).map((scopeKey) => {
              const scopeId = scopeKey as ViewCategoryId;
              const overridePersona = qualityProfileDraft.facet_persona_overrides[scopeId];
              return (
                <div key={scopeId} className="space-y-2">
                  <Label>{qualityCategoryLabels[scopeId]}</Label>
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
                          <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Select
                      value={overridePersona ?? "__default__"}
                      onValueChange={(v) => {
                        const next = { ...qualityProfileDraft.facet_persona_overrides };
                        if (v === "__default__") {
                          delete next[scopeId];
                        } else {
                          next[scopeId] = v as ScoringPersonaId;
                        }
                        updateQualityProfileDraft({ facet_persona_overrides: next });
                      }}
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__default__">{t("qualityProfile.facetPersonaUseDefault")}</SelectItem>
                        <SelectItem value="Balanced">{t("qualityProfile.personaBalanced")}</SelectItem>
                        <SelectItem value="Audiophile">{t("qualityProfile.personaAudiophile")}</SelectItem>
                        <SelectItem value="Efficient">{t("qualityProfile.personaEfficient")}</SelectItem>
                        <SelectItem value="Compatible">{t("qualityProfile.personaCompatible")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              );
            })}
          </div>
        </CardContent>
      </Card>
    </form>
  );
}
