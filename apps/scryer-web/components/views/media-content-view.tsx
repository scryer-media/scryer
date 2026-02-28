
import * as React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Button } from "@/components/ui/button";
import { InfoHelp } from "@/components/common/info-help";
import { LayoutGrid, LayoutList, Loader2, Search, Trash2, Zap } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SearchResultBuckets } from "@/components/common/release-search-results";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import type { ViewId } from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  DownloadClientRecord,
  IndexerCategoryRoutingSettings,
  IndexerRecord,
  LibraryScanSummary,
  NzbgetCategoryRoutingSettings,
  Release,
  TitleRecord,
} from "@/lib/types";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { ViewCategoryId } from "./media-content/indexer-category-picker";
import { MediaLibrarySettingsPanel } from "./media-content/media-library-settings-panel";
import { IndexerRoutingPanel } from "./media-content/indexer-routing-panel";
import { DownloadClientRoutingPanel } from "./media-content/download-client-routing-panel";
import { RulesRoutingPanel } from "./media-content/rules-routing-panel";
import { PosterGrid } from "./media-content/poster-grid";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { RuleSetRecord } from "@/lib/types/rule-sets";

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type Facet = "movie" | "tv" | "anime";
type ContentSettingsSection = "overview" | "settings";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

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

type TvdbSearchItem = MetadataTvdbSearchItem;

type ScopeRoutingRecord = Record<string, NzbgetCategoryRoutingSettings>;
type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function bytesToReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  if (raw > 1024 * 1024 * 1024) {
    return `${(raw / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  if (raw > 1024 * 1024) {
    return `${(raw / (1024 * 1024)).toFixed(2)} MB`;
  }
  if (raw > 1024) {
    return `${(raw / 1024).toFixed(2)} KB`;
  }
  return `${raw} B`;
}

function RenameSettingsForm({
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

export function MediaContentView({
  state,
}: {
  state: {
    t: Translate;
    view: ViewId;
    contentSettingsSection: ContentSettingsSection;
    contentSettingsLabel: string;
    moviesPath: string;
    setMoviesPath: (value: string) => void;
    seriesPath: string;
    setSeriesPath: (value: string) => void;
    mediaSettingsLoading: boolean;
    qualityProfiles: ParsedQualityProfile[];
    qualityProfileParseError: string;
    globalQualityProfileId: string;
    categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
    activeQualityScopeId: ViewCategoryId;
    setCategoryQualityProfileOverrides: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameTemplates: Record<ViewCategoryId, string>;
    setCategoryRenameTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameCollisionPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameMissingMetadataPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryFillerPolicies: Record<ViewCategoryId, string>;
    setCategoryFillerPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRecapPolicies: Record<ViewCategoryId, string>;
    setCategoryRecapPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryMonitorSpecials: Record<ViewCategoryId, string>;
    setCategoryMonitorSpecials: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryInterSeasonMovies: Record<ViewCategoryId, string>;
    setCategoryInterSeasonMovies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryPreferredSubGroup: Record<ViewCategoryId, string>;
    setCategoryPreferredSubGroup: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    nfoWriteOnImport: Record<ViewCategoryId, string>;
    setNfoWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    plexmatchWriteOnImport: Record<ViewCategoryId, string>;
    setPlexmatchWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    qualityProfileInheritValue: string;
    toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
    updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
    mediaSettingsSaving: boolean;
    titleNameForQueue: string;
    setTitleNameForQueue: (value: string) => void;
    queueFacet: Facet;
    setQueueFacet: (value: Facet) => void;
    monitoredForQueue: boolean;
    setMonitoredForQueue: (value: boolean) => void;
    seasonFoldersForQueue: boolean;
    setSeasonFoldersForQueue: (value: boolean) => void;
    monitorSpecialsForQueue: boolean;
    setMonitorSpecialsForQueue: (value: boolean) => void;
    interSeasonMoviesForQueue: boolean;
    setInterSeasonMoviesForQueue: (value: boolean) => void;
    preferredSubGroupForQueue: string;
    setPreferredSubGroupForQueue: (value: string) => void;
    minAvailabilityForQueue: string;
    setMinAvailabilityForQueue: (value: string) => void;
    selectedTvdb: TvdbSearchItem | null;
    tvdbCandidates: TvdbSearchItem[];
    selectedTvdbId: string | null;
    selectTvdbCandidate: (candidate: TvdbSearchItem) => void;
    searchNzbForSelectedTvdb: () => Promise<void>;
    searchResults: Release[];
    onAddSubmit: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
    addTvdbCandidateToCatalog: (candidate: TvdbSearchItem) => Promise<void> | void;
    queueFromSearch: (release: Release) => Promise<void> | void;
    titleFilter: string;
    setTitleFilter: (value: string) => void;
    refreshTitles: () => Promise<void> | void;
    titleLoading: boolean;
    titleStatus: string;
    monitoredTitles: TitleRecord[];
    queueExisting: (title: TitleRecord) => Promise<void> | void;
    runInteractiveSearchForTitle: (title: TitleRecord) => Promise<Release[]> | Release[];
    queueExistingFromRelease: (title: TitleRecord, release: Release) => Promise<void> | void;
    downloadClients: DownloadClientRecord[];
    activeScopeRouting: ScopeRoutingRecord;
    activeScopeRoutingOrder: string[];
    downloadClientRoutingLoading: boolean;
    downloadClientRoutingSaving: boolean;
    updateDownloadClientRoutingForScope: (clientId: string, nextValue: Partial<NzbgetCategoryRoutingSettings>) => void;
    moveDownloadClientInScope: (clientId: string, direction: "up" | "down") => void;
    saveDownloadClientRouting: () => Promise<void> | void;
    indexers: IndexerRecord[];
    activeScopeIndexerRouting: IndexerRoutingRecord;
    activeScopeIndexerRoutingOrder: string[];
    indexerRoutingLoading: boolean;
    indexerRoutingSaving: boolean;
    setIndexerEnabledForScope: (indexerId: string, enabled: boolean) => Promise<void> | void;
    updateIndexerRoutingForScope: (
      indexerId: string,
      nextValue: Partial<IndexerCategoryRoutingSettings>,
    ) => Promise<void> | void;
    moveIndexerInScope: (indexerId: string, direction: "up" | "down") => void;
    ruleSets: RuleSetRecord[];
    rulesLoading: boolean;
    rulesSaving: boolean;
    onToggleRuleFacet: (ruleSetId: string, enabled: boolean) => void;
    libraryScanLoading: boolean;
    libraryScanSummary: LibraryScanSummary | null;
    scanMovieLibrary: () => Promise<void> | void;
    onOpenOverview: (targetView: ViewId, titleId: string) => void;
    deleteCatalogTitle: (title: TitleRecord) => void;
    isDeletingCatalogTitleById: Record<string, boolean>;
  };
}) {
  const {
    t,
    view,
    contentSettingsSection,
    contentSettingsLabel,
    moviesPath,
    setMoviesPath,
    seriesPath,
    setSeriesPath,
    mediaSettingsLoading,
    qualityProfiles,
    qualityProfileParseError,
    globalQualityProfileId,
    categoryQualityProfileOverrides,
    activeQualityScopeId,
    setCategoryQualityProfileOverrides,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
    categoryRenameCollisionPolicies,
    setCategoryRenameCollisionPolicies,
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
    categoryFillerPolicies,
    setCategoryFillerPolicies,
    categoryRecapPolicies,
    setCategoryRecapPolicies,
    categoryMonitorSpecials,
    setCategoryMonitorSpecials,
    categoryInterSeasonMovies,
    setCategoryInterSeasonMovies,
    categoryPreferredSubGroup,
    setCategoryPreferredSubGroup,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    qualityProfileInheritValue,
    toProfileOptions,
    updateCategoryMediaProfileSettings,
    mediaSettingsSaving,
    titleNameForQueue,
    setTitleNameForQueue,
    queueFacet,
    setQueueFacet,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    monitorSpecialsForQueue,
    setMonitorSpecialsForQueue,
    interSeasonMoviesForQueue,
    setInterSeasonMoviesForQueue,
    preferredSubGroupForQueue,
    setPreferredSubGroupForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
    selectedTvdb,
    tvdbCandidates,
    selectedTvdbId,
    selectTvdbCandidate,
    addTvdbCandidateToCatalog,
    searchNzbForSelectedTvdb,
    searchResults,
    onAddSubmit,
    queueFromSearch,
    titleFilter,
    setTitleFilter,
    refreshTitles,
    titleLoading,
    titleStatus,
    monitoredTitles,
    queueExisting,
    runInteractiveSearchForTitle,
    queueExistingFromRelease,
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
    saveDownloadClientRouting,
    indexers,
    activeScopeIndexerRouting,
    activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
    ruleSets,
    rulesLoading,
    rulesSaving,
    onToggleRuleFacet,
    libraryScanLoading,
    libraryScanSummary,
    scanMovieLibrary,
    onOpenOverview,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
  } = state;

  const scopeLabel =
    activeQualityScopeId === "movie"
      ? t("search.facetMovie")
      : activeQualityScopeId === "series"
        ? t("search.facetTv")
        : t("search.facetAnime");
  const [expandedMovieRows, setExpandedMovieRows] = React.useState(new Set<string>());
  const [interactiveSearchResultsByTitle, setInteractiveSearchResultsByTitle] = React.useState<
    Record<string, Release[]>
  >({});
  const [interactiveSearchLoadingByTitle, setInteractiveSearchLoadingByTitle] = React.useState<
    Record<string, boolean>
  >({});
  const [autoQueueLoadingByTitle, setAutoQueueLoadingByTitle] = React.useState<Record<string, boolean>>({});

  type ContentViewMode = "table" | "poster";
  const [viewMode, setViewMode] = React.useState<ContentViewMode>(() => {
    try {
      const stored = localStorage.getItem("scryer:content-view-mode");
      return stored === "poster" ? "poster" : "table";
    } catch {
      return "table";
    }
  });
  React.useEffect(() => {
    try { localStorage.setItem("scryer:content-view-mode", viewMode); } catch { /* noop */ }
  }, [viewMode]);

  const titleTableScrollRef = React.useRef<HTMLDivElement>(null);
  const useVirtualTable = monitoredTitles.length > 50;
  const titleVirtualizer = useVirtualizer({
    count: monitoredTitles.length,
    getScrollElement: () => titleTableScrollRef.current,
    estimateSize: () => 64,
    overscan: 5,
    measureElement: useVirtualTable ? (element) => element.getBoundingClientRect().height : undefined,
    enabled: useVirtualTable,
  });

  const handleMoviesPathChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setMoviesPath(event.target.value);
    },
    [setMoviesPath],
  );

  const handleSeriesPathChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setSeriesPath(event.target.value);
    },
    [setSeriesPath],
  );

  const mediaLibraryPathValue = view === "series" ? seriesPath : moviesPath;
  const mediaLibraryPathLabel =
    view === "series" ? t("settings.seriesPathLabel") : t("settings.moviesPathLabel");
  const mediaLibraryPathPlaceholder =
    view === "series" ? t("settings.seriesPathPlaceholder") : t("settings.moviesPathPlaceholder");
  const mediaLibraryPathHelp =
    view === "series" ? t("settings.seriesPathHelp") : t("settings.moviesPathHelp");
  const mediaLibraryPathChangeHandler =
    view === "series" ? handleSeriesPathChange : handleMoviesPathChange;
  const mediaLibrarySettingsTitle =
    view === "series" ? t("settings.seriesLibrarySettings") : t("settings.moviesLibrarySettings");

  const handleQualityProfileOverrideChange = React.useCallback(
    (value: string) => {
      setCategoryQualityProfileOverrides((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryQualityProfileOverrides],
  );

  const handleRenameTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryRenameTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameTemplates],
  );

  const handleRenameCollisionPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameCollisionPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameCollisionPolicies],
  );

  const handleRenameMissingMetadataPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameMissingMetadataPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameMissingMetadataPolicies],
  );

  const handleFillerPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryFillerPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryFillerPolicies],
  );

  const handleRecapPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRecapPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRecapPolicies],
  );

  const handleMonitorSpecialsChange = React.useCallback(
    (checked: boolean) => {
      setCategoryMonitorSpecials((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setCategoryMonitorSpecials],
  );

  const handleInterSeasonMoviesChange = React.useCallback(
    (checked: boolean) => {
      setCategoryInterSeasonMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setCategoryInterSeasonMovies],
  );

  const handlePreferredSubGroupChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryPreferredSubGroup((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryPreferredSubGroup],
  );

  const handleNfoWriteChange = React.useCallback(
    (checked: boolean) => {
      setNfoWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setNfoWriteOnImport],
  );

  const handlePlexmatchWriteChange = React.useCallback(
    (checked: boolean) => {
      setPlexmatchWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: checked ? "true" : "false",
      }));
    },
    [activeQualityScopeId, setPlexmatchWriteOnImport],
  );

  const handleIndexerCategoriesChange = React.useCallback(
    (indexerId: string, categories: string[]) => {
      void updateIndexerRoutingForScope(indexerId, {
        categories,
      });
    },
    [updateIndexerRoutingForScope],
  );

  const handleIndexerEnabledChange = React.useCallback(
    (indexerId: string, checked: boolean) => {
      void setIndexerEnabledForScope(indexerId, checked);
    },
    [setIndexerEnabledForScope],
  );

  const moveIndexerUp = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "up");
    },
    [moveIndexerInScope],
  );

  const moveIndexerDown = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "down");
    },
    [moveIndexerInScope],
  );

  const handleTitleNameChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setTitleNameForQueue(event.target.value);
    },
    [setTitleNameForQueue],
  );

  const handleQueueFacetChange = React.useCallback(
    (value: string) => {
      setQueueFacet(value as Facet);
    },
    [setQueueFacet],
  );

  const handleTitleFilterChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setTitleFilter(event.target.value);
    },
    [setTitleFilter],
  );

  const handleRefreshTitles = React.useCallback(() => {
    void refreshTitles();
  }, [refreshTitles]);

  const handleSelectTvdbCandidate = React.useCallback(
    (candidate: TvdbSearchItem) => {
      selectTvdbCandidate(candidate);
    },
    [selectTvdbCandidate],
  );

  const handleAddTvdbToCatalog = React.useCallback(
    (candidate: TvdbSearchItem) => {
      void addTvdbCandidateToCatalog(candidate);
    },
    [addTvdbCandidateToCatalog],
  );

  const handleQueueFromSearch = React.useCallback(
    (release: Release) => {
      return Promise.resolve(queueFromSearch(release));
    },
    [queueFromSearch],
  );

  const handleSearchNzbForSelectedTvdb = React.useCallback(() => {
    void searchNzbForSelectedTvdb();
  }, [searchNzbForSelectedTvdb]);

  const handleQueueExisting = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setAutoQueueLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));

      void Promise.resolve(queueExisting(title)).finally(() => {
        setAutoQueueLoadingByTitle((previous) => {
          if (!previous[titleId]) {
            return previous;
          }
          const next = { ...previous };
          delete next[titleId];
          return next;
        });
      });
    },
    [queueExisting],
  );

  const handleRunInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setInteractiveSearchLoadingByTitle((previous) => ({
        ...previous,
        [titleId]: true,
      }));

      void Promise.resolve(runInteractiveSearchForTitle(title))
        .then((results) => {
          setInteractiveSearchResultsByTitle((previous) => ({
            ...previous,
            [titleId]: results ?? [],
          }));
        })
        .finally(() => {
          setInteractiveSearchLoadingByTitle((previous) => {
            if (!previous[titleId]) {
              return previous;
            }
            const next = { ...previous };
            delete next[titleId];
            return next;
          });
        });
    },
    [runInteractiveSearchForTitle],
  );

  const handleQueueExistingFromInteractive = React.useCallback(
    (title: TitleRecord, release: Release) => {
      return Promise.resolve(queueExistingFromRelease(title, release));
    },
    [queueExistingFromRelease],
  );

  const handleToggleInteractiveSearch = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      const isOpen = expandedMovieRows.has(titleId);
      setExpandedMovieRows((previous) => {
        const next = new Set(previous);
        if (next.has(titleId)) {
          next.delete(titleId);
        } else {
          next.add(titleId);
        }
        return next;
      });

      if (
        !isOpen &&
        !Object.prototype.hasOwnProperty.call(interactiveSearchResultsByTitle, titleId)
      ) {
        handleRunInteractiveSearch(title);
      }
    },
    [expandedMovieRows, handleRunInteractiveSearch, interactiveSearchResultsByTitle],
  );

  const handleLibraryScan = React.useCallback(() => {
    void scanMovieLibrary();
  }, [scanMovieLibrary]);

  const handleDeleteCatalogTitle = React.useCallback(
    (title: TitleRecord) => {
      deleteCatalogTitle(title);
    },
    [deleteCatalogTitle],
  );

  return (
    <div className="space-y-4">
      {contentSettingsSection === "settings" ? (
        <div className="space-y-4">
          {view === "movies" || view === "series" ? (
            <MediaLibrarySettingsPanel
              t={t}
              settingsTitle={mediaLibrarySettingsTitle}
              pathLabel={mediaLibraryPathLabel}
              pathValue={mediaLibraryPathValue}
              pathPlaceholder={mediaLibraryPathPlaceholder}
              pathHelp={mediaLibraryPathHelp}
              pathRequired={view === "movies" || view === "series"}
              onPathChange={mediaLibraryPathChangeHandler}
              loading={mediaSettingsLoading}
              scanLoading={libraryScanLoading}
              scanSummary={libraryScanSummary}
              onScan={handleLibraryScan}
            />
          ) : null}

          <RenameSettingsForm
            t={t}
            contentSettingsLabel={contentSettingsLabel}
            mediaSettingsLoading={mediaSettingsLoading}
            qualityProfiles={qualityProfiles}
            qualityProfileParseError={qualityProfileParseError}
            categoryQualityProfileOverrides={categoryQualityProfileOverrides}
            activeQualityScopeId={activeQualityScopeId}
            qualityProfileInheritValue={qualityProfileInheritValue}
            toProfileOptions={toProfileOptions}
            handleQualityProfileOverrideChange={handleQualityProfileOverrideChange}
            categoryRenameTemplates={categoryRenameTemplates}
            handleRenameTemplateChange={handleRenameTemplateChange}
            categoryRenameCollisionPolicies={categoryRenameCollisionPolicies}
            handleRenameCollisionPolicyChange={handleRenameCollisionPolicyChange}
            categoryRenameMissingMetadataPolicies={categoryRenameMissingMetadataPolicies}
            handleRenameMissingMetadataPolicyChange={handleRenameMissingMetadataPolicyChange}
            categoryFillerPolicies={categoryFillerPolicies}
            handleFillerPolicyChange={handleFillerPolicyChange}
            categoryRecapPolicies={categoryRecapPolicies}
            handleRecapPolicyChange={handleRecapPolicyChange}
            categoryMonitorSpecials={categoryMonitorSpecials}
            handleMonitorSpecialsChange={handleMonitorSpecialsChange}
            categoryInterSeasonMovies={categoryInterSeasonMovies}
            handleInterSeasonMoviesChange={handleInterSeasonMoviesChange}
            categoryPreferredSubGroup={categoryPreferredSubGroup}
            handlePreferredSubGroupChange={handlePreferredSubGroupChange}
            nfoWriteOnImport={nfoWriteOnImport}
            handleNfoWriteChange={handleNfoWriteChange}
            plexmatchWriteOnImport={plexmatchWriteOnImport}
            handlePlexmatchWriteChange={handlePlexmatchWriteChange}
            updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
            mediaSettingsSaving={mediaSettingsSaving}
          />

          <IndexerRoutingPanel
            t={t}
            scopeLabel={scopeLabel}
            activeQualityScopeId={activeQualityScopeId}
            indexers={indexers}
            activeScopeIndexerRouting={activeScopeIndexerRouting}
            activeScopeIndexerRoutingOrder={activeScopeIndexerRoutingOrder}
            indexerRoutingLoading={indexerRoutingLoading}
            indexerRoutingSaving={indexerRoutingSaving}
            onEnabledChange={handleIndexerEnabledChange}
            onCategoriesChange={handleIndexerCategoriesChange}
            onMoveUp={moveIndexerUp}
            onMoveDown={moveIndexerDown}
          />

          <DownloadClientRoutingPanel
            t={t}
            scopeLabel={scopeLabel}
            downloadClients={downloadClients}
            activeScopeRouting={activeScopeRouting}
            activeScopeRoutingOrder={activeScopeRoutingOrder}
            downloadClientRoutingLoading={downloadClientRoutingLoading}
            downloadClientRoutingSaving={downloadClientRoutingSaving}
            updateDownloadClientRoutingForScope={updateDownloadClientRoutingForScope}
            moveDownloadClientInScope={moveDownloadClientInScope}
            saveDownloadClientRouting={saveDownloadClientRouting}
          />

          <RulesRoutingPanel
            t={t}
            facet={activeQualityScopeId}
            ruleSets={ruleSets}
            loading={rulesLoading}
            saving={rulesSaving}
            onToggleFacet={onToggleRuleFacet}
          />

        </div>
      ) : (
        view === "movies" || view === "series" || view === "anime" ? (
          <Card>
            <CardHeader>
              <CardTitle>{view === "movies" ? t("title.manageMovies") : view === "anime" ? t("nav.anime") : t("nav.series")}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="mb-3 flex items-center gap-2">
                <Input
                  placeholder={t("title.filterPlaceholder")}
                  value={titleFilter}
                  onChange={handleTitleFilterChange}
                  className="flex-1"
                />
                <ToggleGroup
                  type="single"
                  value={viewMode}
                  onValueChange={(v) => {
                    if (v === "table" || v === "poster") setViewMode(v);
                  }}
                  size="sm"
                  aria-label={t("title.viewModeToggle")}
                >
                  <ToggleGroupItem value="table" size="sm" aria-label={t("title.viewModeTable")}>
                    <LayoutList className="h-4 w-4" />
                  </ToggleGroupItem>
                  <ToggleGroupItem value="poster" size="sm" aria-label={t("title.viewModePoster")}>
                    <LayoutGrid className="h-4 w-4" />
                  </ToggleGroupItem>
                </ToggleGroup>
                <Button variant="secondary" onClick={handleRefreshTitles} disabled={titleLoading}>
                  {titleLoading ? t("label.refreshing") : t("label.refresh")}
                </Button>
              </div>
              <p className="mb-2 text-sm text-muted-foreground">{titleStatus}</p>
              {(() => {
                const isMovieView = view === "movies";
                const overviewTargetView = isMovieView ? "movies" as const : view === "anime" ? "anime" as const : "series" as const;
                const resolvedProfileName = (() => {
                  const overrideId = categoryQualityProfileOverrides[activeQualityScopeId];
                  const effectiveId = (!overrideId || overrideId === qualityProfileInheritValue)
                    ? globalQualityProfileId
                    : overrideId;
                  return qualityProfiles.find((p) => p.id === effectiveId)?.name ?? null;
                })();

                if (viewMode === "poster") {
                  return (
                    <PosterGrid
                      t={t}
                      titles={monitoredTitles}
                      isMovieView={isMovieView}
                      resolvedProfileName={resolvedProfileName}
                      onOpenOverview={onOpenOverview}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={handleQueueExisting}
                      isDeletingById={isDeletingCatalogTitleById}
                      isAutoQueueLoadingById={autoQueueLoadingByTitle}
                      overviewTargetView={overviewTargetView}
                    />
                  );
                }

                const columnCount = isMovieView ? 6 : 5;

                const titleTableHeader = (
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-14">{t("title.table.poster")}</TableHead>
                      <TableHead>{t("title.table.name")}</TableHead>
                      <TableHead>{t("title.table.qualityTier")}</TableHead>
                      {isMovieView ? <TableHead>{t("title.table.size")}</TableHead> : null}
                      <TableHead>{t("title.table.monitored")}</TableHead>
                      <TableHead className="text-right">{t("title.table.actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                );

                const renderTitleRow = (item: TitleRecord) => {
                  const overviewTargetView = isMovieView ? "movies" : view === "anime" ? "anime" : "series";
                  const isPanelOpen = isMovieView && expandedMovieRows.has(item.id);
                  const interactiveSearchResults = interactiveSearchResultsByTitle[item.id] ?? [];
                  const interactiveSearchLoading = interactiveSearchLoadingByTitle[item.id] === true;
                  const autoQueueLoading = autoQueueLoadingByTitle[item.id] === true;
                  const deleteLoading = isDeletingCatalogTitleById[item.id] === true;

                  return (
                    <React.Fragment key={item.id}>
                      <TableRow className="h-24 cv-auto-row">
                        <TableCell className="align-middle">
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item.id)}
                            className="inline-block text-left"
                            aria-label={t("media.posterAlt", { name: item.name })}
                          >
                            <div className="h-20 w-14 overflow-hidden rounded border border-border bg-muted">
                              {item.posterUrl ? (
                                <img
                                  src={item.posterUrl}
                                  alt={t("media.posterAlt", { name: item.name })}
                                  className="h-full w-full object-cover"
                                  loading="lazy"
                                />
                              ) : (
                                <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground">
                                  {t("label.noArt")}
                                </div>
                              )}
                            </div>
                          </button>
                        </TableCell>
                        <TableCell className="align-middle">
                          <button
                            type="button"
                            onClick={() => onOpenOverview(overviewTargetView, item.id)}
                            className="inline-flex text-xl font-bold hover:text-foreground hover:underline"
                          >
                            {item.name}
                          </button>
                        </TableCell>
                        <TableCell className="align-middle">
                          {isMovieView
                            ? (item.qualityTier || t("label.unknown"))
                            : (resolvedProfileName || t("label.default"))}
                        </TableCell>
                        {isMovieView ? <TableCell className="align-middle">{bytesToReadable(item.sizeBytes)}</TableCell> : null}
                        <TableCell className="align-middle">{item.monitored ? t("label.yes") : t("label.no")}</TableCell>
                        <TableCell className="text-right align-middle">
                          <div className="inline-flex items-center justify-end gap-2">
                            {isMovieView ? (
                              <>
                                <HoverCard openDelay={3000} closeDelay={75}>
                                  <HoverCardTrigger asChild>
                                    <Button
                                      variant="ghost"
                                      size="sm"
                                      aria-label={t("label.search")}
                                      onClick={() => handleQueueExisting(item)}
                                      disabled={autoQueueLoading}
                                    >
                                      {autoQueueLoading ? (
                                        <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
                                      ) : (
                                        <Zap className="h-4 w-4" />
                                      )}
                                    </Button>
                                  </HoverCardTrigger>
                                  <HoverCardContent>
                                    <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                      {t("help.autoSearchTooltip")}
                                    </p>
                                  </HoverCardContent>
                                </HoverCard>
                                <HoverCard openDelay={3000} closeDelay={75}>
                                  <HoverCardTrigger asChild>
                                    <Button
                                      variant="ghost"
                                      size="sm"
                                      aria-label={t("label.interactiveSearch")}
                                      onClick={() => handleToggleInteractiveSearch(item)}
                                    >
                                      <Search className="h-4 w-4" />
                                    </Button>
                                  </HoverCardTrigger>
                                  <HoverCardContent>
                                    <p className="max-w-[18rem] whitespace-normal break-words text-sm">
                                      {t("help.interactiveSearchTooltip")}
                                    </p>
                                  </HoverCardContent>
                                </HoverCard>
                              </>
                            ) : null}
                            <Button
                              variant="destructive"
                              size="sm"
                              type="button"
                              aria-label={t("label.delete")}
                              onClick={() => handleDeleteCatalogTitle(item)}
                              disabled={deleteLoading}
                            >
                              {deleteLoading ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Trash2 className="h-4 w-4" />
                              )}
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                      {isPanelOpen ? (
                        <TableRow>
                          <TableCell colSpan={columnCount} className="border-t border-border bg-popover/40 p-0">
                            <div className="px-4 py-3">
                              <div className="mb-2 flex items-center justify-between gap-3">
                                <p className="text-sm text-card-foreground">
                                  {t("nzb.searchResultsFor", { name: item.name })}
                                </p>
                                <Button
                                  type="button"
                                  variant="ghost"
                                  size="sm"
                                  onClick={() => handleRunInteractiveSearch(item)}
                                  disabled={interactiveSearchLoading}
                                  aria-label={t("label.search")}
                                >
                                  <Search className="h-4 w-4" />
                                  <span className="ml-1">
                                    {interactiveSearchLoading ? t("label.searching") : t("label.refresh")}
                                  </span>
                                </Button>
                              </div>
                              {interactiveSearchLoading ? (
                                <div className="flex items-center gap-3 py-3">
                                  <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                                  <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                                </div>
                              ) : interactiveSearchResults.length === 0 ? (
                                <p className="text-sm text-muted-foreground">{t("nzb.noResultsYet")}</p>
                              ) : (
                                <SearchResultBuckets
                                  results={interactiveSearchResults}
                                  onQueue={(release) => handleQueueExistingFromInteractive(item, release)}
                                  t={t}
                                />
                              )}
                            </div>
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </React.Fragment>
                  );
                };

                if (!useVirtualTable) {
                  return (
                    <Table>
                      {titleTableHeader}
                      <TableBody>
                        {monitoredTitles.map(renderTitleRow)}
                        {monitoredTitles.length === 0 && !titleLoading ? (
                          <TableRow>
                            <TableCell colSpan={columnCount} className="text-muted-foreground">
                              {t("title.noManaged")}
                            </TableCell>
                          </TableRow>
                        ) : null}
                      </TableBody>
                    </Table>
                  );
                }

                const virtualItems = titleVirtualizer.getVirtualItems();

                return (
                  <div
                    ref={titleTableScrollRef}
                    className="relative w-full"
                    style={{ maxHeight: "70vh", overflow: "auto" }}
                  >
                    <table className="w-full caption-bottom text-sm">
                      <thead className="[&_tr]:border-b sticky top-0 z-10 bg-background">
                        <TableRow>
                          <TableHead className="w-14">{t("title.table.poster")}</TableHead>
                          <TableHead>{t("title.table.name")}</TableHead>
                          <TableHead>{t("title.table.qualityTier")}</TableHead>
                          {isMovieView ? <TableHead>{t("title.table.size")}</TableHead> : null}
                          <TableHead>{t("title.table.monitored")}</TableHead>
                          <TableHead className="text-right">{t("title.table.actions")}</TableHead>
                        </TableRow>
                      </thead>
                      {virtualItems.length > 0 ? (
                        <>
                          {virtualItems[0].start > 0 ? (
                            <tbody aria-hidden>
                              <tr><td style={{ height: virtualItems[0].start, padding: 0 }} /></tr>
                            </tbody>
                          ) : null}
                          {virtualItems.map((virtualRow) => {
                            const item = monitoredTitles[virtualRow.index];
                            return (
                              <tbody
                                key={virtualRow.key}
                                ref={titleVirtualizer.measureElement}
                                data-index={virtualRow.index}
                                className="[&_tr:last-child]:border-0"
                              >
                                {renderTitleRow(item)}
                              </tbody>
                            );
                          })}
                          {virtualItems[virtualItems.length - 1].end < titleVirtualizer.getTotalSize() ? (
                            <tbody aria-hidden>
                              <tr>
                                <td
                                  style={{
                                    height: titleVirtualizer.getTotalSize() - virtualItems[virtualItems.length - 1].end,
                                    padding: 0,
                                  }}
                                />
                              </tr>
                            </tbody>
                          ) : null}
                        </>
                      ) : !titleLoading ? (
                        <TableBody>
                          <TableRow>
                            <TableCell colSpan={columnCount} className="text-muted-foreground">
                              {t("title.noManaged")}
                            </TableCell>
                          </TableRow>
                        </TableBody>
                      ) : null}
                    </table>
                  </div>
                );
              })()}
            </CardContent>
          </Card>
        ) : (
          <>
            <Card>
              <CardHeader>
                <CardTitle>{t("title.addAndQueue")}</CardTitle>
              </CardHeader>
              <CardContent>
                <form className="grid gap-4 md:grid-cols-5" onSubmit={onAddSubmit}>
                  <label className="md:col-span-3">
                    <Label className="mb-2 block">{t("title.name")}</Label>
                    <Input
                      name="titleName"
                      placeholder={t("title.namePlaceholder")}
                      value={titleNameForQueue}
                      onChange={handleTitleNameChange}
                      required
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block">{t("title.facet")}</Label>
                    <Select value={queueFacet} onValueChange={handleQueueFacetChange}>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="movie">{t("search.facetMovie")}</SelectItem>
                        <SelectItem value="tv">{t("search.facetTv")}</SelectItem>
                        <SelectItem value="anime">{t("search.facetAnime")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </label>
                  <label className="flex items-center gap-2 pt-7">
                    <Checkbox
                      checked={monitoredForQueue}
                      onCheckedChange={(checked) =>
                        setMonitoredForQueue(checked === true)
                      }
                    />
                    <span className="text-sm">{t("title.monitored")}</span>
                  </label>
                  {queueFacet === "movie" && (
                    <label>
                      <Label className="mb-2 block">{t("settings.minAvailabilityLabel")}</Label>
                      <Select value={minAvailabilityForQueue} onValueChange={setMinAvailabilityForQueue}>
                        <SelectTrigger className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="announced">{t("settings.minAvailability.announced")}</SelectItem>
                          <SelectItem value="in_cinemas">{t("settings.minAvailability.in_cinemas")}</SelectItem>
                          <SelectItem value="released">{t("settings.minAvailability.released")}</SelectItem>
                        </SelectContent>
                      </Select>
                    </label>
                  )}
                  {queueFacet !== "movie" && (
                    <label className="flex items-center gap-2 pt-7">
                      <Checkbox
                        checked={seasonFoldersForQueue}
                        onCheckedChange={(checked) =>
                          setSeasonFoldersForQueue(checked === true)
                        }
                      />
                      <span className="text-sm">{t("search.addConfigSeasonFolder")}</span>
                    </label>
                  )}
                  {queueFacet === "anime" && (
                    <>
                      <label className="flex items-center gap-2 pt-7">
                        <Checkbox
                          checked={monitorSpecialsForQueue}
                          onCheckedChange={(checked) =>
                            setMonitorSpecialsForQueue(checked === true)
                          }
                        />
                        <span className="text-sm">{t("settings.monitorSpecialsLabel")}</span>
                      </label>
                      <label className="flex items-center gap-2 pt-7">
                        <Checkbox
                          checked={interSeasonMoviesForQueue}
                          onCheckedChange={(checked) =>
                            setInterSeasonMoviesForQueue(checked === true)
                          }
                        />
                        <span className="text-sm">{t("settings.interSeasonMoviesLabel")}</span>
                      </label>
                      <label className="md:col-span-2">
                        <Label className="mb-2 block">{t("settings.preferredSubGroupLabel")}</Label>
                        <Input
                          value={preferredSubGroupForQueue}
                          onChange={(e) => setPreferredSubGroupForQueue(e.target.value)}
                          placeholder={t("settings.preferredSubGroupPlaceholder")}
                        />
                      </label>
                    </>
                  )}
                  <div className="md:col-span-5 flex justify-end">
                    <Button type="submit">{t("tvdb.searchByTvdb")}</Button>
                  </div>
                </form>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>{t("tvdb.searchResults")}</CardTitle>
              </CardHeader>
              <CardContent>
                {tvdbCandidates.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{t("tvdb.searchPrompt")}</p>
                ) : (
                  <div className="space-y-2">
                    {tvdbCandidates.map((result) => (
                      <div
                        key={`${result.tvdb_id}-${result.name}`}
                        className="rounded-lg border border-border p-3"
                      >
                        <div className="mb-2 flex items-start justify-between gap-3">
                          <div className="flex min-h-20 gap-3">
                            <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                              {result.poster_url ? (
                                <img
                                  src={result.poster_url}
                                  alt={t("media.posterAlt", { name: result.name })}
                                  className="h-full w-full object-cover"
                                  loading="lazy"
                                />
                              ) : (
                                <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                                  {t("label.noArt")}
                                </div>
                              )}
                            </div>
                            <div>
                              <p className="text-sm font-medium text-foreground">{result.name}</p>
                            <p className="text-xs text-muted-foreground">
                              {result.type || t("label.unknownType")} • {result.year ? result.year : t("label.yearUnknown")} •{" "}
                              {result.sort_title || result.slug || t("label.unknown")}
                            </p>
                              {result.overview ? (
                                <p className="mt-2 text-xs text-muted-foreground line-clamp-2">
                                  {result.overview}
                                </p>
                              ) : null}
                            </div>
                          </div>
                          <div className="flex flex-col items-end gap-2">
                            <Button
                              size="sm"
                              variant={String(result.tvdb_id) === selectedTvdbId ? "secondary" : "ghost"}
                              onClick={() => handleSelectTvdbCandidate(result)}
                            >
                              {t("tvdb.select")}
                            </Button>
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={() => handleAddTvdbToCatalog(result)}
                            >
                              {t("title.addToCatalog")}
                            </Button>
                          </div>
                        </div>
                      </div>
                    ))}
                    <div className="pt-2">
                      <Button
                        type="button"
                        onClick={handleSearchNzbForSelectedTvdb}
                        disabled={!selectedTvdbId}
                      >
                        {t("tvdb.searchButton")}
                      </Button>
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>
                  {selectedTvdb ? t("nzb.searchResultsFor", { name: selectedTvdb.name }) : t("nzb.searchResults")}
                </CardTitle>
              </CardHeader>
              <CardContent>
                {searchResults.length === 0 ? (
                  <p className="text-sm text-muted-foreground">
                    {selectedTvdb ? t("nzb.noResultsYet") : t("tvdb.selectPrompt")}
                  </p>
                ) : (
                  <SearchResultBuckets
                    results={searchResults}
                    onQueue={handleQueueFromSearch}
                    t={t}
                  />
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>
                  {t("title.monitoredSection", {
                    facet: t("search.facetAnime"),
                  })}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <div className="mb-3 flex gap-2">
                  <Input
                    placeholder={t("title.filterPlaceholder")}
                    value={titleFilter}
                    onChange={handleTitleFilterChange}
                  />
                  <Button variant="secondary" onClick={handleRefreshTitles} disabled={titleLoading}>
                    {titleLoading ? t("label.refreshing") : t("label.refresh")}
                  </Button>
                </div>
                <p className="mb-2 text-sm text-muted-foreground">{titleStatus}</p>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t("title.table.name")}</TableHead>
                      <TableHead>{t("title.table.facet")}</TableHead>
                      <TableHead>{t("title.table.monitored")}</TableHead>
                      <TableHead className="text-right">{t("title.table.actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {monitoredTitles.map((item) => {
                      const overviewTargetView = item.facet === "movie"
                        ? "movies"
                        : item.facet === "tv"
                          ? "series"
                          : null;
                      return (
                        <TableRow key={item.id}>
                          <TableCell>
                            {overviewTargetView ? (
                              <button
                                type="button"
                                onClick={() => onOpenOverview(overviewTargetView, item.id)}
                                className="hover:text-foreground hover:underline"
                              >
                                {item.name}
                              </button>
                            ) : (
                              item.name
                            )}
                          </TableCell>
                          <TableCell>{item.facet}</TableCell>
                          <TableCell>{item.monitored ? t("label.yes") : t("label.no")}</TableCell>
                          <TableCell className="text-right">
                            <Button variant="ghost" size="sm" onClick={() => handleQueueExisting(item)}>
                              {t("title.queueLatest")}
                            </Button>
                          </TableCell>
                        </TableRow>
                      );
                    })}
                    {monitoredTitles.length === 0 && !titleLoading ? (
                      <TableRow>
                        <TableCell colSpan={4} className="text-muted-foreground">
                          {t("title.noManaged")}
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          </>
        )
      )}
    </div>
  );
}
