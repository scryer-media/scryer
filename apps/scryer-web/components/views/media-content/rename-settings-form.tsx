import * as React from "react";
import { Button } from "@/components/ui/button";
import { InfoHelp } from "@/components/common/info-help";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ViewCategoryId } from "./indexer-category-picker";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

// --- Constants ---

const RENAME_COLLISION_POLICY_OPTIONS = [
  { value: "skip", label: "settings.renameCollisionPolicySkip" },
  { value: "error", label: "settings.renameCollisionPolicyError" },
  { value: "replace_if_better", label: "settings.renameCollisionPolicyReplaceIfBetter" },
];

const RENAME_MISSING_METADATA_POLICY_OPTIONS = [
  { value: "fallback_title", label: "settings.renameMissingMetadataPolicyFallbackTitle" },
  { value: "skip", label: "settings.renameMissingMetadataPolicySkip" },
];

const FILLER_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.fillerPolicyDownloadAll" },
  { value: "skip_filler", label: "settings.fillerPolicySkipFiller" },
];

const RECAP_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.recapPolicyDownloadAll" },
  { value: "skip_recap", label: "settings.recapPolicySkipRecap" },
];

const VALID_RENAME_TOKENS = new Set([
  "title", "year", "quality", "edition", "source",
  "video_codec", "audio_codec", "audio_channels", "group", "ext",
  "season", "season_order", "episode", "episode_title", "absolute_episode",
]);

const SHARED_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "quality", labelKey: "settings.renameTokenQuality" },
  { token: "source", labelKey: "settings.renameTokenSource" },
  { token: "video_codec", labelKey: "settings.renameTokenVideoCodec" },
  { token: "audio_codec", labelKey: "settings.renameTokenAudioCodec" },
  { token: "audio_channels", labelKey: "settings.renameTokenAudioChannels" },
  { token: "group", labelKey: "settings.renameTokenGroup" },
  { token: "ext", labelKey: "settings.renameTokenExt" },
];

const MOVIE_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "year", labelKey: "settings.renameTokenYear" },
  { token: "edition", labelKey: "settings.renameTokenEdition" },
];

const SERIES_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

const ANIME_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "season_order", labelKey: "settings.renameTokenSeasonOrder" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "absolute_episode", labelKey: "settings.renameTokenAbsoluteEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

function getRenameTokenDescriptions(scopeId: ViewCategoryId): { token: string; labelKey: string }[] {
  const scopeSpecific = scopeId === "movie"
    ? MOVIE_RENAME_TOKEN_DESCRIPTIONS
    : scopeId === "anime"
      ? ANIME_RENAME_TOKEN_DESCRIPTIONS
      : SERIES_RENAME_TOKEN_DESCRIPTIONS;
  const shared = scopeId === "series"
    ? SHARED_RENAME_TOKEN_DESCRIPTIONS.filter((token) => token.token !== "group")
    : SHARED_RENAME_TOKEN_DESCRIPTIONS;
  return [...scopeSpecific, ...shared];
}

function validateRenameTemplate(
  template: string,
  t: Translate,
): string | null {
  if (!template.trim()) {
    return t("settings.renameValidationEmpty");
  }

  let i = 0;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const tokenName = inner.includes(":") ? inner.split(":")[0] : inner;
      if (!VALID_RENAME_TOKENS.has(tokenName)) {
        return t("settings.renameValidationUnknownToken", { token: tokenName });
      }
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return t("settings.renameValidationUnmatchedClose");
    } else {
      i++;
    }
  }

  return null;
}

const RENAME_PREVIEW_MOVIE_SAMPLE: Record<string, string> = {
  title: "The Dark Knight",
  year: "2008",
  quality: "2160p",
  edition: "IMAX",
  source: "BluRay",
  video_codec: "x265",
  audio_codec: "DTS-HD MA",
  audio_channels: "5.1",
  group: "FraMeSToR",
  ext: "mkv",
  season: "1",
  episode: "5",
  episode_title: "Pilot",
};

const RENAME_PREVIEW_SERIES_SAMPLE: Record<string, string> = {
  title: "Friends",
  year: "1994",
  quality: "1080p",
  edition: "Director's Cut",
  source: "WEB-DL",
  video_codec: "x264",
  audio_codec: "AAC",
  audio_channels: "2.0",
  group: "NTb",
  ext: "mkv",
  season: "5",
  episode: "12",
  episode_title: "The One with the Embryos",
};

const RENAME_PREVIEW_ANIME_SAMPLE: Record<string, string> = {
  title: "One Piece",
  year: "1999",
  quality: "1080p",
  edition: "Director's Cut",
  source: "WEB-DL",
  video_codec: "x265",
  audio_codec: "AAC",
  audio_channels: "2.0",
  group: "SubsPlease",
  ext: "mkv",
  season: "1",
  season_order: "1",
  episode: "1",
  absolute_episode: "1",
  episode_title: "Romance Dawn",
};

function applyRenameTemplate(template: string, scopeId: ViewCategoryId): string | null {
  if (!template.trim()) return null;
  let result = "";
  let i = 0;
  const sampleValues =
    scopeId === "movie"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "anime"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const colonIdx = inner.indexOf(":");
      const tokenName = colonIdx >= 0 ? inner.slice(0, colonIdx) : inner;
      const padWidth = colonIdx >= 0 ? parseInt(inner.slice(colonIdx + 1), 10) : 0;
      if (!VALID_RENAME_TOKENS.has(tokenName)) return null;
      let value = sampleValues[tokenName] ?? tokenName;
      if (padWidth > 0 && /^\d+$/.test(value)) {
        value = value.padStart(padWidth, "0");
      }
      result += value;
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return null;
    } else {
      result += template[i];
      i++;
    }
  }
  return result;
}

// --- Component ---

export function RenameSettingsForm({
  t,
  contentSettingsLabel,
  mediaSettingsLoading,
  qualityProfiles,
  qualityProfileParseError,
  categoryQualityProfileOverrides,
  activeQualityScopeId,
  qualityProfileInheritValue,
  toProfileOptions,
  handleQualityProfileOverrideChange,
  categoryRenameTemplates,
  handleRenameTemplateChange,
  categoryRenameCollisionPolicies,
  handleRenameCollisionPolicyChange,
  categoryRenameMissingMetadataPolicies,
  handleRenameMissingMetadataPolicyChange,
  categoryFillerPolicies,
  handleFillerPolicyChange,
  categoryRecapPolicies,
  handleRecapPolicyChange,
  categoryMonitorSpecials,
  handleMonitorSpecialsChange,
  categoryInterSeasonMovies,
  handleInterSeasonMoviesChange,
  categoryPreferredSubGroup,
  handlePreferredSubGroupChange,
  nfoWriteOnImport,
  handleNfoWriteChange,
  plexmatchWriteOnImport,
  handlePlexmatchWriteChange,
  updateCategoryMediaProfileSettings,
  mediaSettingsSaving,
}: {
  t: Translate;
  contentSettingsLabel: string;
  mediaSettingsLoading: boolean;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileParseError: string;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  activeQualityScopeId: ViewCategoryId;
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
  handleQualityProfileOverrideChange: (value: string) => void;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  handleRenameTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
  handleRenameCollisionPolicyChange: (value: string) => void;
  categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
  handleRenameMissingMetadataPolicyChange: (value: string) => void;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  handleFillerPolicyChange: (value: string) => void;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  handleRecapPolicyChange: (value: string) => void;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  handleMonitorSpecialsChange: (checked: boolean) => void;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  handleInterSeasonMoviesChange: (checked: boolean) => void;
  categoryPreferredSubGroup: Record<ViewCategoryId, string>;
  handlePreferredSubGroupChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  handleNfoWriteChange: (checked: boolean) => void;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  handlePlexmatchWriteChange: (checked: boolean) => void;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mediaSettingsSaving: boolean;
}) {
  const templateValue = categoryRenameTemplates[activeQualityScopeId];
  const renameValidationError = React.useMemo(
    () => validateRenameTemplate(templateValue, t),
    [templateValue, t],
  );

  const renamePreview = React.useMemo(
    () => applyRenameTemplate(templateValue, activeQualityScopeId),
    [activeQualityScopeId, templateValue],
  );

  const templateInputRef = React.useRef<HTMLInputElement>(null);

  const insertToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) return;
      const insertion = `{${token}}`;
      const start = input.selectionStart ?? templateValue.length;
      const end = input.selectionEnd ?? start;
      const next = templateValue.slice(0, start) + insertion + templateValue.slice(end);

      const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )?.set;
      if (nativeInputValueSetter) {
        nativeInputValueSetter.call(input, next);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }

      requestAnimationFrame(() => {
        const cursorPos = start + insertion.length;
        input.setSelectionRange(cursorPos, cursorPos);
        input.focus();
      });
    },
    [templateValue],
  );

  return (
    <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.qualityProfileSection")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <label>
            <Label className="mb-2 inline-flex items-center gap-2">
              {t("settings.qualityProfileOverrideLabel", {
                category: contentSettingsLabel.toLowerCase(),
              })}
              <InfoHelp
                text={t("settings.qualityProfileOverrideHelp")}
                ariaLabel={t("settings.qualityProfileOverrideHelp")}
              />
            </Label>
            <Select value={categoryQualityProfileOverrides[activeQualityScopeId]} onValueChange={handleQualityProfileOverrideChange} disabled={mediaSettingsLoading}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={qualityProfileInheritValue}>{t("settings.qualityProfileInheritLabel")}</SelectItem>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {qualityProfileParseError ? (
              <p className="mt-2 rounded border border-rose-500/60 bg-rose-500/10 p-2 text-xs text-rose-300">
                {qualityProfileParseError}
              </p>
            ) : null}
          </label>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("settings.renameSectionTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
            <div className="space-y-2.5">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameTemplateLabel")}
              </Label>
              <Input
                ref={templateInputRef}
                value={templateValue}
                onChange={handleRenameTemplateChange}
                placeholder={t("settings.renameTemplatePlaceholder")}
                disabled={mediaSettingsLoading}
                className={
                  templateValue.trim()
                    ? renameValidationError
                      ? "text-rose-400 border-rose-500/60"
                      : "text-emerald-600 dark:text-emerald-400 border-emerald-500/60"
                    : undefined
                }
              />
              {renameValidationError ? (
                <p className="text-xs text-rose-400">{renameValidationError}</p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                Example
              </Label>
              {renamePreview ? (
                <div className="rounded border border-border bg-muted px-3 py-1.5">
                  <p className="break-all font-mono text-sm text-card-foreground">{renamePreview}</p>
                </div>
              ) : (
                <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                  <p className="text-sm text-muted-foreground/60">—</p>
                </div>
              )}
            </div>
          </div>

          <div className="space-y-2.5">
            <p className="text-sm font-medium text-card-foreground">
              {t("settings.renameAvailableTokens")}
            </p>
            <div className="flex flex-wrap gap-1.5">
              {getRenameTokenDescriptions(activeQualityScopeId).map((item) => (
                <button
                  key={item.token}
                  type="button"
                  className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-emerald-500 hover:bg-accent hover:text-foreground"
                  title={t(item.labelKey)}
                  onClick={() => insertToken(item.token)}
                >
                  <code className="text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                  <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                </button>
              ))}
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <label className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameCollisionPolicyLabel")}
              </Label>
              <Select value={categoryRenameCollisionPolicies[activeQualityScopeId]} onValueChange={handleRenameCollisionPolicyChange} disabled={mediaSettingsLoading}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RENAME_COLLISION_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <label className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameMissingMetadataPolicyLabel")}
              </Label>
              <Select value={categoryRenameMissingMetadataPolicies[activeQualityScopeId]} onValueChange={handleRenameMissingMetadataPolicyChange} disabled={mediaSettingsLoading}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RENAME_MISSING_METADATA_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("settings.renamePolicyHelp")}
          </p>

          {activeQualityScopeId === "anime" && (
            <div className="grid gap-4 md:grid-cols-2">
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.fillerPolicyLabel")}
                </Label>
                <Select value={categoryFillerPolicies[activeQualityScopeId]} onValueChange={handleFillerPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {FILLER_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.recapPolicyLabel")}
                </Label>
                <Select value={categoryRecapPolicies[activeQualityScopeId]} onValueChange={handleRecapPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RECAP_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.monitorSpecialsLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={categoryMonitorSpecials[activeQualityScopeId] !== "false"}
                    className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${categoryMonitorSpecials[activeQualityScopeId] !== "false" ? "bg-primary" : "bg-muted"}`}
                    onClick={() => handleMonitorSpecialsChange(categoryMonitorSpecials[activeQualityScopeId] === "false")}
                    disabled={mediaSettingsLoading}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${categoryMonitorSpecials[activeQualityScopeId] !== "false" ? "translate-x-5" : "translate-x-0"}`}
                    />
                  </button>
                  <span className="text-xs text-muted-foreground">{t("settings.monitorSpecialsDescription")}</span>
                </div>
              </div>
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.interSeasonMoviesLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={categoryInterSeasonMovies[activeQualityScopeId] !== "false"}
                    className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${categoryInterSeasonMovies[activeQualityScopeId] !== "false" ? "bg-primary" : "bg-muted"}`}
                    onClick={() => handleInterSeasonMoviesChange(categoryInterSeasonMovies[activeQualityScopeId] === "false")}
                    disabled={mediaSettingsLoading}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${categoryInterSeasonMovies[activeQualityScopeId] !== "false" ? "translate-x-5" : "translate-x-0"}`}
                    />
                  </button>
                  <span className="text-xs text-muted-foreground">{t("settings.interSeasonMoviesDescription")}</span>
                </div>
              </div>
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.preferredSubGroupLabel")}
                </Label>
                <Input
                  value={categoryPreferredSubGroup[activeQualityScopeId]}
                  onChange={handlePreferredSubGroupChange}
                  placeholder={t("settings.preferredSubGroupPlaceholder")}
                  disabled={mediaSettingsLoading}
                />
              </label>
            </div>
          )}

          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.nfoWriteOnImportLabel")}
              </Label>
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  role="switch"
                  aria-checked={nfoWriteOnImport[activeQualityScopeId] === "true"}
                  className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${nfoWriteOnImport[activeQualityScopeId] === "true" ? "bg-primary" : "bg-muted"}`}
                  onClick={() => handleNfoWriteChange(nfoWriteOnImport[activeQualityScopeId] !== "true")}
                  disabled={mediaSettingsLoading}
                >
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${nfoWriteOnImport[activeQualityScopeId] === "true" ? "translate-x-5" : "translate-x-0"}`}
                  />
                </button>
                <span className="text-xs text-muted-foreground">{t("settings.nfoWriteOnImportDescription")}</span>
              </div>
            </div>
            {(activeQualityScopeId === "series" || activeQualityScopeId === "anime") && (
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.plexmatchWriteOnImportLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={plexmatchWriteOnImport[activeQualityScopeId] === "true"}
                    className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${plexmatchWriteOnImport[activeQualityScopeId] === "true" ? "bg-primary" : "bg-muted"}`}
                    onClick={() => handlePlexmatchWriteChange(plexmatchWriteOnImport[activeQualityScopeId] !== "true")}
                    disabled={mediaSettingsLoading}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${plexmatchWriteOnImport[activeQualityScopeId] === "true" ? "translate-x-5" : "translate-x-0"}`}
                    />
                  </button>
                  <span className="text-xs text-muted-foreground">{t("settings.plexmatchWriteOnImportDescription")}</span>
                </div>
              </div>
            )}
          </div>

          <div className="flex justify-end">
            <Button type="submit" disabled={mediaSettingsSaving || renameValidationError !== null}>
              {mediaSettingsSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </form>
  );
}
