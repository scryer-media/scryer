import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type { Translate } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import {
  applyRenameTemplatePreview,
  splitRenameTemplateSegments,
  validateRenameTemplateSyntax,
  type RenameTemplateSegment,
  type RenameTemplateValidationIssue,
} from "@/lib/utils/rename-template";
import type { ViewCategoryId } from "./indexer-category-picker";

const RENAME_COLLISION_POLICY_OPTIONS = [
  { value: "skip", label: "settings.renameCollisionPolicySkip" },
  { value: "error", label: "settings.renameCollisionPolicyError" },
  { value: "replace_if_better", label: "settings.renameCollisionPolicyReplaceIfBetter" },
];

const RENAME_MISSING_METADATA_POLICY_OPTIONS = [
  { value: "fallback_title", label: "settings.renameMissingMetadataPolicyFallbackTitle" },
  { value: "skip", label: "settings.renameMissingMetadataPolicySkip" },
];

const VALID_RENAME_TOKENS = new Set([
  "title", "year", "quality", "edition", "source",
  "video_codec", "audio_codec", "audio_channels", "group", "ext",
  "season", "season_order", "episode", "episode_title", "absolute_episode",
  "imdb_id", "tmdb_id", "tvdb_id", "anidb_id", "mal_id", "anilist_id",
]);
const VALID_FOLDER_TOKENS = new Set([
  "title", "year",
  "imdb_id", "tmdb_id", "tvdb_id", "anidb_id", "mal_id", "anilist_id",
]);
const VALID_SEASON_FOLDER_TOKENS = new Set(["season"]);

const SEASON_FOLDER_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
];

const FOLDER_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "year", labelKey: "settings.renameTokenYear" },
  { token: "imdb_id", labelKey: "settings.renameTokenImdbId" },
  { token: "tmdb_id", labelKey: "settings.renameTokenTmdbId" },
  { token: "tvdb_id", labelKey: "settings.renameTokenTvdbId" },
  { token: "anidb_id", labelKey: "settings.renameTokenAnidbId" },
  { token: "mal_id", labelKey: "settings.renameTokenMalId" },
  { token: "anilist_id", labelKey: "settings.renameTokenAnilistId" },
];

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

const EXTERNAL_ID_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "imdb_id", labelKey: "settings.renameTokenImdbId" },
  { token: "tmdb_id", labelKey: "settings.renameTokenTmdbId" },
  { token: "tvdb_id", labelKey: "settings.renameTokenTvdbId" },
  { token: "anidb_id", labelKey: "settings.renameTokenAnidbId" },
  { token: "mal_id", labelKey: "settings.renameTokenMalId" },
  { token: "anilist_id", labelKey: "settings.renameTokenAnilistId" },
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

type TokenDescription = {
  token: string;
  labelKey: string;
};

function getRenameTokenDescriptions(scopeId: ViewCategoryId): { token: string; labelKey: string }[] {
  const scopeSpecific = scopeId === "movie"
    ? MOVIE_RENAME_TOKEN_DESCRIPTIONS
    : scopeId === "anime"
      ? ANIME_RENAME_TOKEN_DESCRIPTIONS
      : SERIES_RENAME_TOKEN_DESCRIPTIONS;
  const shared = scopeId === "series"
    ? SHARED_RENAME_TOKEN_DESCRIPTIONS.filter((token) => token.token !== "group")
    : SHARED_RENAME_TOKEN_DESCRIPTIONS;
  return [...scopeSpecific, ...EXTERNAL_ID_RENAME_TOKEN_DESCRIPTIONS, ...shared];
}

function validateRenameTemplate(
  template: string,
  t: Translate,
): string | null {
  return formatRenameValidationIssue(
    validateRenameTemplateSyntax(template, VALID_RENAME_TOKENS),
    t,
  );
}

function formatRenameValidationIssue(
  issue: RenameTemplateValidationIssue | null,
  t: Translate,
): string | null {
  if (!issue) {
    return null;
  }

  switch (issue.kind) {
    case "empty":
      return t("settings.renameValidationEmpty");
    case "unmatchedOpen":
      return t("settings.renameValidationUnmatchedOpen");
    case "unmatchedClose":
      return t("settings.renameValidationUnmatchedClose");
    case "unknownToken":
      return t("settings.renameValidationUnknownToken", { token: issue.token });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
  }

  return null;
}

function validateFolderTemplate(
  template: string,
  t: Translate,
): string | null {
  const issue = validateRenameTemplateSyntax(template, VALID_FOLDER_TOKENS);
  if (!issue) {
    return null;
  }

  switch (issue.kind) {
    case "empty":
      return t("settings.folderValidationEmpty");
    case "unmatchedOpen":
      return t("settings.renameValidationUnmatchedOpen");
    case "unmatchedClose":
      return t("settings.renameValidationUnmatchedClose");
    case "unknownToken":
      return t("settings.folderValidationUnknownToken", { token: issue.token });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
  }

  return null;
}

function validateSeasonFolderTemplate(
  template: string,
  t: Translate,
): string | null {
  const issue = validateRenameTemplateSyntax(template, VALID_SEASON_FOLDER_TOKENS);
  if (!issue) {
    return null;
  }

  switch (issue.kind) {
    case "empty":
      return t("settings.seasonFolderValidationEmpty");
    case "unmatchedOpen":
      return t("settings.renameValidationUnmatchedOpen");
    case "unmatchedClose":
      return t("settings.renameValidationUnmatchedClose");
    case "unknownToken":
      return t("settings.seasonFolderValidationUnknownToken", { token: issue.token });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
  }

  return null;
}

const RENAME_PREVIEW_MOVIE_SAMPLE: Record<string, string> = {
  title: "The Dark Knight", year: "2008", quality: "2160p", edition: "IMAX",
  source: "BluRay", video_codec: "x265", audio_codec: "DTS-HD MA",
  audio_channels: "5.1", group: "FraMeSToR", ext: "mkv",
  imdb_id: "tt0468569", tmdb_id: "155", tvdb_id: "123456",
  anidb_id: "", mal_id: "", anilist_id: "",
  season: "1", episode: "5", episode_title: "Pilot",
};

const RENAME_PREVIEW_SERIES_SAMPLE: Record<string, string> = {
  title: "Friends", year: "1994", quality: "1080p", edition: "Director's Cut",
  source: "WEB-DL", video_codec: "x264", audio_codec: "AAC",
  audio_channels: "2.0", group: "NTb", ext: "mkv",
  imdb_id: "tt0108778", tmdb_id: "1668", tvdb_id: "79168",
  anidb_id: "", mal_id: "", anilist_id: "",
  season: "5", episode: "12", episode_title: "The One with the Embryos",
};

const RENAME_PREVIEW_ANIME_SAMPLE: Record<string, string> = {
  title: "Tidebreaker", year: "1999", quality: "1080p", edition: "Director's Cut",
  source: "WEB-DL", video_codec: "x265", audio_codec: "AAC",
  audio_channels: "2.0", group: "SubsPlease", ext: "mkv",
  imdb_id: "tt0388629", tmdb_id: "37854", tvdb_id: "81797",
  anidb_id: "69", mal_id: "21", anilist_id: "21",
  season: "1", season_order: "1", episode: "1",
  absolute_episode: "1", episode_title: "Romance Dawn",
};

function applyRenameTemplate(template: string, scopeId: ViewCategoryId): string | null {
  const sampleValues =
    scopeId === "movie"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "anime"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  return applyRenameTemplatePreview(template, VALID_RENAME_TOKENS, sampleValues);
}

function applyFolderTemplate(template: string, scopeId: ViewCategoryId): string | null {
  const sampleValues =
    scopeId === "movie"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "anime"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  const result = applyRenameTemplatePreview(template, VALID_FOLDER_TOKENS, sampleValues);
  return result?.trim() || null;
}

function splitFolderTemplateSegments(template: string): RenameTemplateSegment[] {
  return splitRenameTemplateSegments(template, VALID_FOLDER_TOKENS);
}

function applySeasonFolderTemplate(template: string, scopeId: ViewCategoryId): string | null {
  const sampleValues =
    scopeId === "anime" ? RENAME_PREVIEW_ANIME_SAMPLE : RENAME_PREVIEW_SERIES_SAMPLE;
  const result = applyRenameTemplatePreview(template, VALID_SEASON_FOLDER_TOKENS, sampleValues);
  return result?.trim() || null;
}

function splitSeasonFolderTemplateSegments(template: string): RenameTemplateSegment[] {
  return splitRenameTemplateSegments(template, VALID_SEASON_FOLDER_TOKENS);
}

function splitRenameInputSegments(template: string): RenameTemplateSegment[] {
  return splitRenameTemplateSegments(template, VALID_RENAME_TOKENS);
}

type HighlightedTemplateInputProps = React.ComponentProps<typeof Input> & {
  value: string;
  getSegments?: (value: string) => RenameTemplateSegment[];
};

type TemplateTokenContext = {
  key: string;
  query: string;
  replaceStart: number;
  replaceEnd: number;
  shouldCloseBrace: boolean;
};

function updateInputValue(
  input: HTMLInputElement,
  nextValue: string,
  selectionStart: number,
  selectionEnd = selectionStart,
) {
  const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  if (!nativeInputValueSetter) {
    return;
  }
  nativeInputValueSetter.call(input, nextValue);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  requestAnimationFrame(() => {
    input.setSelectionRange(selectionStart, selectionEnd);
    input.focus();
  });
}

function insertTemplateToken(
  input: HTMLInputElement,
  currentValue: string,
  token: string,
) {
  const insertion = `{${token}}`;
  const start = input.selectionStart ?? currentValue.length;
  const end = input.selectionEnd ?? start;
  const nextValue = currentValue.slice(0, start) + insertion + currentValue.slice(end);
  updateInputValue(input, nextValue, start + insertion.length);
}

function resolveTemplateTokenContext(
  value: string,
  cursor: number,
  tokenDescriptions: TokenDescription[],
): TemplateTokenContext | null {
  const lastOpen = value.lastIndexOf("{", cursor - 1);
  const lastClose = value.lastIndexOf("}", cursor - 1);
  if (lastOpen === -1 || lastOpen < lastClose) {
    return null;
  }

  if (value[lastOpen - 1] === "{" || value[lastOpen + 1] === "{") {
    return null;
  }

  const tokenBody = value.slice(lastOpen + 1, cursor);
  if (!tokenBody || tokenBody.includes("{") || tokenBody.includes("}")) {
    return null;
  }

  const colonIndex = tokenBody.indexOf(":");
  if (colonIndex !== -1) {
    return null;
  }

  const nextOpen = value.indexOf("{", lastOpen + 1);
  const nextClose = value.indexOf("}", lastOpen + 1);
  const shouldCloseBrace =
    nextClose === -1 || (nextOpen !== -1 && nextOpen < nextClose);
  const query = tokenBody.trim().toLowerCase();

  const matches = tokenDescriptions
    .filter(({ token }) => token.toLowerCase().includes(query))
    .sort((left, right) => {
      const leftToken = left.token.toLowerCase();
      const rightToken = right.token.toLowerCase();
      const leftStarts = leftToken.startsWith(query) ? 0 : 1;
      const rightStarts = rightToken.startsWith(query) ? 0 : 1;
      return leftStarts - rightStarts || leftToken.localeCompare(rightToken);
    });
  if (matches.length === 0) {
    return null;
  }

  return {
    key: `${lastOpen}:${query}`,
    query,
    replaceStart: lastOpen + 1,
    replaceEnd: lastOpen + 1 + tokenBody.length,
    shouldCloseBrace,
  };
}

function applyAutocompleteToken(
  input: HTMLInputElement,
  currentValue: string,
  context: TemplateTokenContext,
  token: string,
) {
  const suffix = context.shouldCloseBrace ? "}" : "";
  const nextValue =
    currentValue.slice(0, context.replaceStart) +
    token +
    suffix +
    currentValue.slice(context.replaceEnd);
  const cursor = context.replaceStart + token.length + suffix.length;
  updateInputValue(input, nextValue, cursor);
}

const HighlightedTemplateInput = React.forwardRef<HTMLInputElement, HighlightedTemplateInputProps>(
  ({ className, value, getSegments, onScroll, ...props }, ref) => {
    const [scrollLeft, setScrollLeft] = React.useState(0);
    const segments = React.useMemo(
      () => (getSegments ?? splitRenameInputSegments)(value),
      [getSegments, value],
    );
    const showOverlay = value.length > 0;

    return (
      <div className="relative">
        {showOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-10 flex items-center overflow-hidden rounded-md px-3 py-1 text-base md:text-sm"
          >
            <div
              className="min-w-full whitespace-pre"
              style={{ transform: `translateX(-${scrollLeft}px)` }}
            >
              {segments.map((segment, index) => (
                <span
                  key={`${index}-${segment.text}`}
                  className={segment.isToken ? "text-emerald-600 dark:text-emerald-400" : "text-foreground"}
                >
                  {segment.text}
                </span>
              ))}
            </div>
          </div>
        ) : null}
        <Input
          {...props}
          ref={ref}
          value={value}
          onScroll={(event) => {
            setScrollLeft(event.currentTarget.scrollLeft);
            onScroll?.(event);
          }}
          className={cn(
            showOverlay && "text-transparent caret-foreground selection:text-transparent",
            className,
          )}
        />
      </div>
    );
  },
);

HighlightedTemplateInput.displayName = "HighlightedTemplateInput";

type TokenAutocompleteInputProps = Omit<HighlightedTemplateInputProps, "ref"> & {
  inputRef: React.RefObject<HTMLInputElement | null>;
  tokenDescriptions: TokenDescription[];
  onAutocompleteToken: (token: string) => void;
  translateLabel: Translate;
};

function TokenAutocompleteInput({
  inputRef,
  value,
  tokenDescriptions,
  onAutocompleteToken,
  translateLabel,
  onBlur,
  onChange,
  onClick,
  onFocus,
  onKeyDown,
  onSelect,
  ...props
}: TokenAutocompleteInputProps) {
  const [isFocused, setIsFocused] = React.useState(false);
  const [cursor, setCursor] = React.useState(0);
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);
  const [dismissedKey, setDismissedKey] = React.useState<string | null>(null);

  const syncCursor = React.useCallback(() => {
    const input = inputRef.current;
    if (!input) {
      return;
    }
    setCursor(input.selectionStart ?? value.length);
  }, [inputRef, value.length]);

  const tokenContext = React.useMemo(
    () =>
      isFocused
        ? resolveTemplateTokenContext(value, cursor, tokenDescriptions)
        : null,
    [cursor, isFocused, tokenDescriptions, value],
  );

  const suggestions = React.useMemo(() => {
    if (!tokenContext || tokenContext.key === dismissedKey) {
      return [];
    }
    return tokenDescriptions
      .filter(({ token }) => token.toLowerCase().includes(tokenContext.query))
      .sort((left, right) => {
        const leftToken = left.token.toLowerCase();
        const rightToken = right.token.toLowerCase();
        const leftStarts = leftToken.startsWith(tokenContext.query) ? 0 : 1;
        const rightStarts = rightToken.startsWith(tokenContext.query) ? 0 : 1;
        return leftStarts - rightStarts || leftToken.localeCompare(rightToken);
      });
  }, [dismissedKey, tokenContext, tokenDescriptions]);

  React.useEffect(() => {
    setHighlightedIndex(0);
  }, [tokenContext?.key]);

  React.useEffect(() => {
    if (tokenContext && tokenContext.key !== dismissedKey) {
      return;
    }
    setDismissedKey(null);
  }, [dismissedKey, tokenContext]);

  return (
    <div className="relative">
      <HighlightedTemplateInput
        {...props}
        ref={inputRef}
        value={value}
        onChange={(event) => {
          onChange?.(event);
          requestAnimationFrame(syncCursor);
        }}
        onFocus={(event) => {
          setIsFocused(true);
          syncCursor();
          onFocus?.(event);
        }}
        onBlur={(event) => {
          setIsFocused(false);
          setDismissedKey(null);
          onBlur?.(event);
        }}
        onClick={(event) => {
          syncCursor();
          onClick?.(event);
        }}
        onSelect={(event) => {
          syncCursor();
          onSelect?.(event);
        }}
        onKeyDown={(event) => {
          if (suggestions.length > 0) {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setHighlightedIndex((current) => (current + 1) % suggestions.length);
              return;
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setHighlightedIndex((current) => (current - 1 + suggestions.length) % suggestions.length);
              return;
            }
            if (event.key === "Enter" || event.key === "Tab") {
              event.preventDefault();
              onAutocompleteToken(suggestions[highlightedIndex]?.token ?? suggestions[0].token);
              setDismissedKey(null);
              return;
            }
            if (event.key === "Escape") {
              event.preventDefault();
              if (tokenContext) {
                setDismissedKey(tokenContext.key);
              }
              return;
            }
          }
          onKeyDown?.(event);
        }}
      />
      {isFocused && suggestions.length > 0 ? (
        <div className="absolute left-0 right-0 top-[calc(100%+0.375rem)] z-20 rounded-md border border-border/80 bg-popover shadow-lg">
          <div className="max-h-56 overflow-auto p-1">
            {suggestions.map((item, index) => {
              const isActive = index === highlightedIndex;
              return (
                <button
                  key={item.token}
                  type="button"
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-sm px-2 py-1.5 text-left text-sm transition-colors",
                    isActive ? "bg-accent text-accent-foreground" : "text-popover-foreground hover:bg-accent/70",
                  )}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    onAutocompleteToken(item.token);
                    setDismissedKey(null);
                  }}
                >
                  <code className="font-mono text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                  <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                    {translateLabel(item.labelKey)}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function RenameSettingsPanel({
  activeQualityScopeId,
  mediaSettingsLoading,
  mediaSettingsSaving,
  categoryFolderTemplates,
  handleFolderTemplateChange,
  categorySeasonFolderTemplates,
  handleSeasonFolderTemplateChange,
  categoryRenameTemplates,
  handleRenameTemplateChange,
  categoryRenameEnabled,
  handleRenameEnabledChange,
  categoryRenameCollisionPolicies,
  handleRenameCollisionPolicyChange,
  categoryRenameMissingMetadataPolicies,
  handleRenameMissingMetadataPolicyChange,
  updateCategoryMediaProfileSettings,
}: {
  activeQualityScopeId: ViewCategoryId;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  categoryFolderTemplates: Record<ViewCategoryId, string>;
  handleFolderTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categorySeasonFolderTemplates: Record<ViewCategoryId, string>;
  handleSeasonFolderTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  handleRenameTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameEnabled: Record<ViewCategoryId, string>;
  handleRenameEnabledChange: (checked: boolean) => void;
  categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
  handleRenameCollisionPolicyChange: (value: string) => void;
  categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
  handleRenameMissingMetadataPolicyChange: (value: string) => void;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
}) {
  const t = useTranslate();
  const folderTemplateValue = categoryFolderTemplates[activeQualityScopeId];
  const showSeasonFolderTemplate = activeQualityScopeId !== "movie";
  const seasonFolderTemplateValue = categorySeasonFolderTemplates[activeQualityScopeId];
  const renameEnabled = categoryRenameEnabled[activeQualityScopeId] !== "false";
  const templateValue = categoryRenameTemplates[activeQualityScopeId];
  const folderValidationError = React.useMemo(
    () => validateFolderTemplate(folderTemplateValue, t),
    [folderTemplateValue, t],
  );
  const seasonFolderValidationError = React.useMemo(
    () =>
      showSeasonFolderTemplate
        ? validateSeasonFolderTemplate(seasonFolderTemplateValue, t)
        : null,
    [seasonFolderTemplateValue, showSeasonFolderTemplate, t],
  );
  const renameValidationError = React.useMemo(
    () => (renameEnabled ? validateRenameTemplate(templateValue, t) : null),
    [renameEnabled, templateValue, t],
  );

  const folderPreview = React.useMemo(
    () => applyFolderTemplate(folderTemplateValue, activeQualityScopeId),
    [activeQualityScopeId, folderTemplateValue],
  );

  const seasonFolderPreview = React.useMemo(
    () =>
      showSeasonFolderTemplate
        ? applySeasonFolderTemplate(seasonFolderTemplateValue, activeQualityScopeId)
        : null,
    [activeQualityScopeId, seasonFolderTemplateValue, showSeasonFolderTemplate],
  );

  const renamePreview = React.useMemo(
    () => (renameEnabled ? applyRenameTemplate(templateValue, activeQualityScopeId) : null),
    [activeQualityScopeId, renameEnabled, templateValue],
  );

  const folderInputRef = React.useRef<HTMLInputElement>(null);
  const seasonFolderInputRef = React.useRef<HTMLInputElement>(null);
  const templateInputRef = React.useRef<HTMLInputElement>(null);
  const renameTokenDescriptions = React.useMemo(
    () => getRenameTokenDescriptions(activeQualityScopeId),
    [activeQualityScopeId],
  );

  const insertFolderToken = React.useCallback(
    (token: string) => {
      const input = folderInputRef.current;
      if (!input) return;
      insertTemplateToken(input, folderTemplateValue, token);
    },
    [folderTemplateValue],
  );

  const insertSeasonFolderToken = React.useCallback(
    (token: string) => {
      const input = seasonFolderInputRef.current;
      if (!input) return;
      insertTemplateToken(input, seasonFolderTemplateValue, token);
    },
    [seasonFolderTemplateValue],
  );

  const insertToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) return;
      insertTemplateToken(input, templateValue, token);
    },
    [templateValue],
  );

  const autocompleteFolderToken = React.useCallback(
    (token: string) => {
      const input = folderInputRef.current;
      if (!input) {
        return;
      }
      const cursor = input.selectionStart ?? folderTemplateValue.length;
      const context = resolveTemplateTokenContext(folderTemplateValue, cursor, FOLDER_TOKEN_DESCRIPTIONS);
      if (!context) {
        insertTemplateToken(input, folderTemplateValue, token);
        return;
      }
      applyAutocompleteToken(input, folderTemplateValue, context, token);
    },
    [folderTemplateValue],
  );

  const autocompleteSeasonFolderToken = React.useCallback(
    (token: string) => {
      const input = seasonFolderInputRef.current;
      if (!input) {
        return;
      }
      const cursor = input.selectionStart ?? seasonFolderTemplateValue.length;
      const context = resolveTemplateTokenContext(
        seasonFolderTemplateValue,
        cursor,
        SEASON_FOLDER_TOKEN_DESCRIPTIONS,
      );
      if (!context) {
        insertTemplateToken(input, seasonFolderTemplateValue, token);
        return;
      }
      applyAutocompleteToken(input, seasonFolderTemplateValue, context, token);
    },
    [seasonFolderTemplateValue],
  );

  const autocompleteRenameToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) {
        return;
      }
      const cursor = input.selectionStart ?? templateValue.length;
      const context = resolveTemplateTokenContext(templateValue, cursor, renameTokenDescriptions);
      if (!context) {
        insertTemplateToken(input, templateValue, token);
        return;
      }
      applyAutocompleteToken(input, templateValue, context, token);
    },
    [renameTokenDescriptions, templateValue],
  );

  return (
    <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.renameSection")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <section className="space-y-5 rounded-lg border border-border/70 bg-card/40 p-4">
            <div className="space-y-1">
              <h3 className="text-sm font-semibold text-card-foreground">
                {t("settings.folderRenameSectionTitle")}
              </h3>
            </div>

            <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
              <div className="space-y-2.5">
                <Label className="text-sm text-card-foreground">
                  {t("settings.folderTemplateLabel")}
                </Label>
                <TokenAutocompleteInput
                  inputRef={folderInputRef}
                  value={folderTemplateValue}
                  onChange={handleFolderTemplateChange}
                  tokenDescriptions={FOLDER_TOKEN_DESCRIPTIONS}
                  onAutocompleteToken={autocompleteFolderToken}
                  translateLabel={t}
                  getSegments={splitFolderTemplateSegments}
                  placeholder={t("settings.folderTemplatePlaceholder")}
                  disabled={mediaSettingsLoading}
                  className={
                    folderTemplateValue.trim()
                      ? folderValidationError
                        ? "border-rose-500/60"
                        : "border-emerald-500/60"
                      : undefined
                  }
                />
                {folderValidationError ? (
                  <p className="text-xs text-rose-400">{folderValidationError}</p>
                ) : null}
              </div>

              <div className="space-y-2">
                <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                  Example
                </Label>
                {folderPreview ? (
                  <div className="rounded border border-border bg-muted px-3 py-1.5">
                    <p className="break-all font-mono text-sm text-card-foreground">{folderPreview}</p>
                  </div>
                ) : (
                  <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                    <p className="text-sm text-muted-foreground/60">&mdash;</p>
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-2.5">
              <p className="text-sm font-medium text-card-foreground">
                {t("settings.folderAvailableTokens")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {FOLDER_TOKEN_DESCRIPTIONS.map((item) => (
                  <button
                    key={item.token}
                    type="button"
                    className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-emerald-500 hover:bg-accent hover:text-foreground"
                    title={t(item.labelKey)}
                    onClick={() => insertFolderToken(item.token)}
                  >
                    <code className="text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                    <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                  </button>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                <span className="font-medium text-card-foreground">{t("settings.renameAliasTokenLabel")}:</span>{" "}
                <code className="text-emerald-600 dark:text-emerald-400">{"{Movie.Title} ({Release.Year})"}</code>{" "}
                <span className="text-muted-foreground/80">{t("settings.renameAliasTokenHint")}</span>
              </p>
            </div>

            {showSeasonFolderTemplate ? (
              <div className="space-y-5 border-t border-border/70 pt-5">
                <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
                  <div className="space-y-2.5">
                    <Label className="text-sm text-card-foreground">
                      {t("settings.seasonFolderTemplateLabel")}
                    </Label>
                    <TokenAutocompleteInput
                      inputRef={seasonFolderInputRef}
                      value={seasonFolderTemplateValue}
                      onChange={handleSeasonFolderTemplateChange}
                      tokenDescriptions={SEASON_FOLDER_TOKEN_DESCRIPTIONS}
                      onAutocompleteToken={autocompleteSeasonFolderToken}
                      translateLabel={t}
                      getSegments={splitSeasonFolderTemplateSegments}
                      placeholder={t("settings.seasonFolderTemplatePlaceholder")}
                      disabled={mediaSettingsLoading}
                      className={
                        seasonFolderTemplateValue.trim()
                          ? seasonFolderValidationError
                            ? "border-rose-500/60"
                            : "border-emerald-500/60"
                          : undefined
                      }
                    />
                    {seasonFolderValidationError ? (
                      <p className="text-xs text-rose-400">{seasonFolderValidationError}</p>
                    ) : null}
                  </div>

                  <div className="space-y-2">
                    <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                      Example
                    </Label>
                    {seasonFolderPreview ? (
                      <div className="rounded border border-border bg-muted px-3 py-1.5">
                        <p className="break-all font-mono text-sm text-card-foreground">{seasonFolderPreview}</p>
                      </div>
                    ) : (
                      <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                        <p className="text-sm text-muted-foreground/60">&mdash;</p>
                      </div>
                    )}
                  </div>
                </div>

                <div className="space-y-2.5">
                  <p className="text-sm font-medium text-card-foreground">
                    {t("settings.folderAvailableTokens")}
                  </p>
                  <div className="flex flex-wrap gap-1.5">
                    {SEASON_FOLDER_TOKEN_DESCRIPTIONS.map((item) => (
                      <button
                        key={item.token}
                        type="button"
                        className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-emerald-500 hover:bg-accent hover:text-foreground"
                        title={t(item.labelKey)}
                        onClick={() => insertSeasonFolderToken(item.token)}
                      >
                        <code className="text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                        <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                      </button>
                    ))}
                  </div>
                  <p className="text-xs text-muted-foreground">
                    <span className="font-medium text-card-foreground">{t("settings.renameAliasTokenLabel")}:</span>{" "}
                    <code className="text-emerald-600 dark:text-emerald-400">{"Season.{season:00}"}</code>
                  </p>
                </div>
              </div>
            ) : null}
          </section>

          <section className="space-y-6 rounded-lg border border-border/70 bg-card/40 p-4">
            <div className="flex items-start justify-between gap-4">
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-card-foreground">
                  {t("settings.renameSectionTitle")}
                </h3>
              </div>
              <SettingsToggleSwitch
                id="rename-settings-enabled-toggle"
                checked={renameEnabled}
                disabled={mediaSettingsLoading}
                ariaLabel={renameEnabled ? t("label.enabled") : t("label.disabled")}
                onChange={handleRenameEnabledChange}
              />
            </div>

            {renameEnabled ? (
              <>
                <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
                  <div className="space-y-2.5">
                    <Label className="text-sm text-card-foreground">
                      {t("settings.renameTemplateLabel")}
                    </Label>
                    <TokenAutocompleteInput
                      id="rename-settings-template-input"
                      inputRef={templateInputRef}
                      value={templateValue}
                      onChange={handleRenameTemplateChange}
                      tokenDescriptions={renameTokenDescriptions}
                      onAutocompleteToken={autocompleteRenameToken}
                      translateLabel={t}
                      placeholder={t("settings.renameTemplatePlaceholder")}
                      disabled={mediaSettingsLoading}
                      className={
                        templateValue.trim()
                          ? renameValidationError
                            ? "border-rose-500/60"
                            : "border-emerald-500/60"
                          : undefined
                      }
                    />
                    {renameValidationError ? (
                      <p id="rename-settings-validation-message" className="text-xs text-rose-400">{renameValidationError}</p>
                    ) : null}
                  </div>

                  <div className="space-y-2">
                    <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                      Example
                    </Label>
                    {renamePreview ? (
                      <div className="rounded border border-border bg-muted px-3 py-1.5">
                        <p id="rename-settings-preview-output" className="break-all font-mono text-sm text-card-foreground">{renamePreview}</p>
                      </div>
                    ) : (
                      <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                        <p className="text-sm text-muted-foreground/60">&mdash;</p>
                      </div>
                    )}
                  </div>
                </div>

                <div className="space-y-2.5">
                  <p className="text-sm font-medium text-card-foreground">
                    {t("settings.renameAvailableTokens")}
                  </p>
                  <div className="flex flex-wrap gap-1.5">
                    {renameTokenDescriptions.map((item) => (
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
                  <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
                    <p>
                      <span className="font-medium text-card-foreground">{t("settings.renameLiteralBracesLabel")}:</span>{" "}
                      <code className="text-emerald-600 dark:text-emerald-400">{"{{edition-{edition}}}"}</code>
                    </p>
                    <p>
                      <span className="font-medium text-card-foreground">{t("settings.renameSpaceFilterLabel")}:</span>{" "}
                      <code className="text-emerald-600 dark:text-emerald-400">{"{title|space:_}"}</code>
                    </p>
                    <p className="sm:col-span-2">
                      <span className="font-medium text-card-foreground">{t("settings.renameAliasTokenLabel")}:</span>{" "}
                      <code className="text-emerald-600 dark:text-emerald-400">
                        {"{Series.Title}.S{season:00}E{episode:00}.{Episode.Title}.{Quality.Full}"}
                      </code>
                      <br />
                      <span className="text-muted-foreground/80">{t("settings.renameAliasTokenHint")}</span>
                    </p>
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
              </>
            ) : null}
          </section>

          <div className="flex justify-end">
            <Button
              id="rename-settings-save"
              type="submit"
              disabled={
                mediaSettingsSaving ||
                folderValidationError !== null ||
                renameValidationError !== null
              }
            >
              {mediaSettingsSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </form>
  );
}
