import * as React from "react";
import { createPortal } from "react-dom";
import { FileText, Folder, Save } from "lucide-react";
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
  validateFolderTemplateSyntax,
  validateRenameTemplateSyntax,
  type FolderTemplateValidationIssue,
  type RenameTemplateSegment,
  type RenameTemplateValidationIssue,
} from "@/lib/utils/rename-template";
import { FACET_REFERENCE_SLOT_ID } from "./facet-settings-section";
import type { ViewCategoryId } from "./indexer-category-picker";

const RENAME_COLLISION_POLICY_OPTIONS = [
  { value: "SKIP", label: "settings.renameCollisionPolicySkip" },
  { value: "ERROR", label: "settings.renameCollisionPolicyError" },
  { value: "REPLACE_IF_BETTER", label: "settings.renameCollisionPolicyReplaceIfBetter" },
];

const RENAME_MISSING_METADATA_POLICY_OPTIONS = [
  { value: "FALLBACK_TITLE", label: "settings.renameMissingMetadataPolicyFallbackTitle" },
  { value: "SKIP", label: "settings.renameMissingMetadataPolicySkip" },
];

const COMMON_RENAME_TOKENS = [
  "title", "year", "quality", "source",
  "video_codec", "audio_codec", "audio_channels", "group", "ext",
];
const EXTERNAL_ID_RENAME_TOKENS = [
  "imdb_id", "tmdb_id", "tvdb_id", "anidb_id", "mal_id", "anilist_id",
];
const EPISODE_RENAME_TOKENS = [
  "season", "season_order", "episode", "episode_title", "absolute_episode",
];
const VALID_MOVIE_RENAME_TOKENS = new Set([
  ...COMMON_RENAME_TOKENS,
  "edition",
  ...EXTERNAL_ID_RENAME_TOKENS,
]);
const VALID_EPISODE_RENAME_TOKENS = new Set([
  ...COMMON_RENAME_TOKENS,
  ...EPISODE_RENAME_TOKENS,
  ...EXTERNAL_ID_RENAME_TOKENS,
]);
const VALID_FOLDER_TOKENS = new Set([
  "title", "year",
  "imdb_id", "tmdb_id", "tvdb_id", "anidb_id", "mal_id", "anilist_id",
]);
const VALID_SEASON_FOLDER_TOKENS = new Set([
  ...VALID_FOLDER_TOKENS,
  "season",
]);

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
const SEASON_FOLDER_TOKEN_DESCRIPTIONS = [
  ...FOLDER_TOKEN_DESCRIPTIONS,
  { token: "season", labelKey: "settings.renameTokenSeason" },
];

const SHARED_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "year", labelKey: "settings.renameTokenYear" },
  { token: "quality", labelKey: "settings.renameTokenQuality" },
  { token: "source", labelKey: "settings.renameTokenSource" },
  { token: "video_codec", labelKey: "settings.renameTokenVideoCodec" },
  { token: "audio_codec", labelKey: "settings.renameTokenAudioCodec" },
  { token: "audio_channels", labelKey: "settings.renameTokenAudioChannels" },
  { token: "group", labelKey: "settings.renameTokenGroup" },
  { token: "ext", labelKey: "settings.renameTokenExt" },
];

const MOVIE_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
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
  { token: "season_order", labelKey: "settings.renameTokenSeasonOrder" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "absolute_episode", labelKey: "settings.renameTokenAbsoluteEpisode" },
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

const RENAME_SPACE_FILTER_REFERENCES: { code: string; labelKey: string }[] = [
  { code: "space:_", labelKey: "settings.renameSpaceUnderscoreFilterLabel" },
  { code: "space:.", labelKey: "settings.renameSpaceDotFilterLabel" },
  { code: "space:-", labelKey: "settings.renameSpaceDashFilterLabel" },
  { code: "space:", labelKey: "settings.renameSpaceRemoveFilterLabel" },
];

type TokenApplicability = "files" | "folders";

type TokenReference = TokenDescription & {
  appliesTo: TokenApplicability[];
};

type TemplateFilterSuggestion = {
  code: string;
  insert: string;
  labelKey: string;
  aliases?: string[];
  selectStart?: number;
  selectEnd?: number;
};

const RENAME_FILTER_AUTOCOMPLETE_SUGGESTIONS: TemplateFilterSuggestion[] = [
  {
    code: "truncate:N",
    insert: "truncate:N",
    labelKey: "settings.renameTruncateFilterLabel",
    aliases: ["length", "size"],
    selectStart: "truncate:".length,
    selectEnd: "truncate:N".length,
  },
  ...RENAME_SPACE_FILTER_REFERENCES.map((item) => ({
    code: item.code,
    insert: item.code,
    labelKey: item.labelKey,
  })),
];

function getRenameTokenDescriptions(scopeId: ViewCategoryId): { token: string; labelKey: string }[] {
  const scopeSpecific = scopeId === "MOVIE"
    ? MOVIE_RENAME_TOKEN_DESCRIPTIONS
    : scopeId === "ANIME"
      ? ANIME_RENAME_TOKEN_DESCRIPTIONS
      : SERIES_RENAME_TOKEN_DESCRIPTIONS;
  const shared = scopeId === "SERIES"
    ? SHARED_RENAME_TOKEN_DESCRIPTIONS.filter((token) => token.token !== "group")
    : SHARED_RENAME_TOKEN_DESCRIPTIONS;
  return [...scopeSpecific, ...EXTERNAL_ID_RENAME_TOKEN_DESCRIPTIONS, ...shared];
}

function getValidRenameTokens(scopeId: ViewCategoryId): ReadonlySet<string> {
  return scopeId === "MOVIE"
    ? VALID_MOVIE_RENAME_TOKENS
    : VALID_EPISODE_RENAME_TOKENS;
}

function getRenameReferenceTokens(
  renameTokenDescriptions: TokenDescription[],
  scopeId: ViewCategoryId,
): TokenReference[] {
  const tokens = new Map<string, TokenReference>();
  const addToken = (item: TokenDescription, appliesTo: TokenApplicability) => {
    const existing = tokens.get(item.token);
    if (existing) {
      if (!existing.appliesTo.includes(appliesTo)) {
        existing.appliesTo.push(appliesTo);
      }
      return;
    }
    tokens.set(item.token, {
      ...item,
      appliesTo: [appliesTo],
    });
  };

  const folderTokens = scopeId === "MOVIE"
    ? FOLDER_TOKEN_DESCRIPTIONS
    : SEASON_FOLDER_TOKEN_DESCRIPTIONS;
  folderTokens.forEach((item) => addToken(item, "folders"));
  renameTokenDescriptions.forEach((item) => addToken(item, "files"));

  return Array.from(tokens.values());
}

function validateRenameTemplate(
  template: string,
  scopeId: ViewCategoryId,
  t: Translate,
): string | null {
  return formatRenameValidationIssue(
    validateRenameTemplateSyntax(template, getValidRenameTokens(scopeId)),
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
    case "invalidPadding":
      return t("settings.renameValidationInvalidPadding", { padding: issue.padding });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
    case "invalidOptionalGroup":
      return t("settings.renameValidationInvalidOptionalGroup");
    case "nestedOptionalGroup":
      return t("settings.renameValidationNestedOptionalGroup");
    case "unsupportedOptionalFallback":
      return t("settings.renameValidationUnsupportedOptionalFallback");
  }

  return null;
}

function validateFolderTemplate(
  template: string,
  t: Translate,
  validTokens: ReadonlySet<string> = VALID_FOLDER_TOKENS,
  requiredToken?: string,
): string | null {
  const issue: FolderTemplateValidationIssue | null = validateFolderTemplateSyntax(
    template,
    validTokens,
    requiredToken,
  );
  if (!issue) return null;

  switch (issue.kind) {
    case "empty":
      return t("settings.folderValidationEmpty");
    case "unmatchedOpen":
      return t("settings.renameValidationUnmatchedOpen");
    case "unmatchedClose":
      return t("settings.renameValidationUnmatchedClose");
    case "unknownToken": {
      const key = validTokens === VALID_SEASON_FOLDER_TOKENS
        ? "settings.seasonFolderValidationUnknownToken"
        : "settings.folderValidationUnknownToken";
      return t(key, { token: issue.token });
    }
    case "invalidPadding":
      return t("settings.renameValidationInvalidPadding", { padding: issue.padding });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
    case "invalidOptionalGroup":
      return t("settings.renameValidationInvalidOptionalGroup");
    case "nestedOptionalGroup":
      return t("settings.renameValidationNestedOptionalGroup");
    case "unsupportedOptionalFallback":
      return t("settings.renameValidationUnsupportedOptionalFallback");
    case "illegalCharacter":
      return t("settings.folderValidationIllegalCharacter", {
        character: JSON.stringify(issue.character),
      });
    case "missingRequiredToken":
      return t("settings.seasonFolderTemplateMustContainSeason");
  }
}

const RENAME_PREVIEW_MOVIE_SAMPLE: Record<string, string> = {
  title: "The Grey Harbor", year: "2008", quality: "2160p", edition: "IMAX",
  source: "BluRay", video_codec: "x265", audio_codec: "DTS-HD MA",
  audio_channels: "5.1", group: "FraMeSToR", ext: "mkv",
  imdb_id: "tt0468569", tmdb_id: "155", tvdb_id: "123456",
  anidb_id: "", mal_id: "", anilist_id: "",
  season: "1", episode: "5", episode_title: "Pilot",
};

const RENAME_PREVIEW_SERIES_SAMPLE: Record<string, string> = {
  title: "Harbor Lights", year: "1994", quality: "1080p", edition: "Director's Cut",
  source: "WEB-DL", video_codec: "x264", audio_codec: "AAC",
  audio_channels: "2.0", group: "NTb", ext: "mkv",
  imdb_id: "tt0108778", tmdb_id: "1668", tvdb_id: "79168",
  anidb_id: "", mal_id: "", anilist_id: "",
  season: "5", season_order: "5", episode: "12",
  absolute_episode: "97", episode_title: "The One with the Embryos",
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
    scopeId === "MOVIE"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "ANIME"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  return applyRenameTemplatePreview(template, getValidRenameTokens(scopeId), sampleValues);
}

function applyFolderTemplate(
  template: string,
  scopeId: ViewCategoryId,
  validTokens: ReadonlySet<string> = VALID_FOLDER_TOKENS,
  season?: string,
): string | null {
  const baseSampleValues =
    scopeId === "MOVIE"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "ANIME"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  const sampleValues = season === undefined
    ? baseSampleValues
    : { ...baseSampleValues, season };
  return applyRenameTemplatePreview(template, validTokens, sampleValues)?.trim() || null;
}

function splitFolderTemplateSegments(
  template: string,
  validTokens: ReadonlySet<string> = VALID_FOLDER_TOKENS,
): RenameTemplateSegment[] {
  return splitRenameTemplateSegments(template, validTokens);
}

function splitRenameInputSegments(
  template: string,
  scopeId: ViewCategoryId,
): RenameTemplateSegment[] {
  return splitRenameTemplateSegments(template, getValidRenameTokens(scopeId));
}

type HighlightedTemplateInputProps = React.ComponentProps<typeof Input> & {
  value: string;
  getSegments?: (value: string) => RenameTemplateSegment[];
};

type TemplateTokenContext = {
  kind: "token" | "filter";
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
  const isOptionalGroupGuard = tokenBody.startsWith("?");
  const guardOrTokenBody = isOptionalGroupGuard ? tokenBody.slice(1) : tokenBody;

  const nextOpen = value.indexOf("{", lastOpen + 1);
  const nextClose = value.indexOf("}", lastOpen + 1);
  const shouldCloseBrace =
    nextClose === -1 || (nextOpen !== -1 && nextOpen < nextClose);
  const pipeIndex = guardOrTokenBody.lastIndexOf("|");
  if (pipeIndex !== -1) {
    if (isOptionalGroupGuard) {
      return null;
    }
    const tokenName = guardOrTokenBody.slice(0, pipeIndex).split("|", 1)[0]?.trim();
    if (!tokenName || tokenName.includes(":")) {
      return null;
    }
    const query = guardOrTokenBody.slice(pipeIndex + 1).trim().toLowerCase();
    return {
      kind: "filter",
      key: `filter:${lastOpen}:${pipeIndex}:${query}`,
      query,
      replaceStart: lastOpen + 1 + pipeIndex + 1,
      replaceEnd: lastOpen + 1 + tokenBody.length,
      shouldCloseBrace,
    };
  }

  const colonIndex = guardOrTokenBody.indexOf(":");
  if (colonIndex !== -1) {
    return null;
  }

  const query = guardOrTokenBody.trim().toLowerCase();

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
    kind: "token",
    key: `${lastOpen}:${isOptionalGroupGuard ? "?" : ""}${query}`,
    query,
    replaceStart: lastOpen + 1 + (isOptionalGroupGuard ? 1 : 0),
    replaceEnd: lastOpen + 1 + tokenBody.length,
    shouldCloseBrace: isOptionalGroupGuard ? false : shouldCloseBrace,
  };
}

function applyAutocompleteToken(
  input: HTMLInputElement,
  currentValue: string,
  context: TemplateTokenContext,
  token: string,
  options: { selectionStart?: number; selectionEnd?: number } = {},
) {
  const suffix = context.shouldCloseBrace ? "}" : "";
  const nextValue =
    currentValue.slice(0, context.replaceStart) +
    token +
    suffix +
    currentValue.slice(context.replaceEnd);
  const selectionStart =
    context.replaceStart + (options.selectionStart ?? token.length + suffix.length);
  const selectionEnd =
    context.replaceStart + (options.selectionEnd ?? options.selectionStart ?? token.length + suffix.length);
  updateInputValue(input, nextValue, selectionStart, selectionEnd);
}

const HighlightedTemplateInput = React.forwardRef<HTMLInputElement, HighlightedTemplateInputProps>(
  ({ className, value, getSegments, onScroll, ...props }, ref) => {
    const [scrollLeft, setScrollLeft] = React.useState(0);
    const segments = React.useMemo(
      () =>
        getSegments
          ? getSegments(value)
          : splitRenameTemplateSegments(value, VALID_EPISODE_RENAME_TOKENS),
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
                  className={segment.isToken ? "text-[var(--scry-accent-text)]" : "text-foreground"}
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

type TemplateAutocompleteSuggestion = {
  key: string;
  kind: "token" | "filter";
  code: string;
  labelKey: string;
  token?: string;
  insert?: string;
  selectStart?: number;
  selectEnd?: number;
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

    if (tokenContext.kind === "filter") {
      return RENAME_FILTER_AUTOCOMPLETE_SUGGESTIONS
        .map((item, index) => ({ item, index }))
        .filter(({ item }) => {
          const code = item.code.toLowerCase();
          return code.includes(tokenContext.query) ||
            item.aliases?.some((alias) => alias.includes(tokenContext.query));
        })
        .sort((left, right) => {
          if (!tokenContext.query) {
            return left.index - right.index;
          }
          const leftCode = left.item.code.toLowerCase();
          const rightCode = right.item.code.toLowerCase();
          const leftStarts = leftCode.startsWith(tokenContext.query) ? 0 : 1;
          const rightStarts = rightCode.startsWith(tokenContext.query) ? 0 : 1;
          return leftStarts - rightStarts || leftCode.localeCompare(rightCode);
        })
        .map(({ item }): TemplateAutocompleteSuggestion => ({
          key: `filter:${item.code}`,
          kind: "filter",
          code: `|${item.code}`,
          labelKey: item.labelKey,
          insert: item.insert,
          selectStart: item.selectStart,
          selectEnd: item.selectEnd,
        }));
    }

    return tokenDescriptions
      .filter(({ token }) => token.toLowerCase().includes(tokenContext.query))
      .sort((left, right) => {
        const leftToken = left.token.toLowerCase();
        const rightToken = right.token.toLowerCase();
        const leftStarts = leftToken.startsWith(tokenContext.query) ? 0 : 1;
        const rightStarts = rightToken.startsWith(tokenContext.query) ? 0 : 1;
        return leftStarts - rightStarts || leftToken.localeCompare(rightToken);
      })
      .map((item): TemplateAutocompleteSuggestion => ({
        key: `token:${item.token}`,
        kind: "token",
        code: `{${item.token}}`,
        labelKey: item.labelKey,
        token: item.token,
      }));
  }, [dismissedKey, tokenContext, tokenDescriptions]);

  const applySuggestion = React.useCallback(
    (suggestion: TemplateAutocompleteSuggestion | undefined) => {
      if (!suggestion || !tokenContext) {
        return;
      }
      if (suggestion.kind === "token") {
        onAutocompleteToken(suggestion.token ?? suggestion.code);
        return;
      }
      const input = inputRef.current;
      if (!input || !suggestion.insert) {
        return;
      }
      applyAutocompleteToken(input, value, tokenContext, suggestion.insert, {
        selectionStart: suggestion.selectStart,
        selectionEnd: suggestion.selectEnd,
      });
    },
    [inputRef, onAutocompleteToken, tokenContext, value],
  );

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
              applySuggestion(suggestions[highlightedIndex] ?? suggestions[0]);
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
                  key={item.key}
                  type="button"
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-sm px-2 py-1.5 text-left text-sm transition-colors",
                    isActive ? "bg-accent text-accent-foreground" : "text-popover-foreground hover:bg-accent/70",
                  )}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    applySuggestion(item);
                    setDismissedKey(null);
                  }}
                >
                  <code className="font-[var(--font-code)] text-[var(--scry-accent-text)]">{item.code}</code>
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

function RenameTokenReferenceList({
  tokens,
  onFileTokenClick,
  onFolderTokenClick,
  t,
}: {
  tokens: TokenReference[];
  onFileTokenClick: (token: string) => void;
  onFolderTokenClick: (token: string) => void;
  t: Translate;
}) {
  const applicabilityLabels: Record<TokenApplicability, string> = {
    files: t("settings.renameReferenceFiles"),
    folders: t("settings.renameReferenceFolders"),
  };
  const applicabilityActions: Record<TokenApplicability, (token: string) => void> = {
    files: onFileTokenClick,
    folders: onFolderTokenClick,
  };

  return (
    <section className="space-y-2.5">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">
        {t("settings.renameReferenceTokens")}
      </h3>
      <div className="grid gap-1.5 2xl:grid-cols-2">
        {tokens.map((item) => (
          <div
            key={item.token}
            className="flex items-center justify-between gap-3 rounded-md border border-border/70 bg-muted/40 px-2.5 py-2"
          >
            <div className="min-w-0 space-y-1">
              <code className="block font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
                {`{${item.token}}`}
              </code>
              <p className="text-xs leading-snug text-muted-foreground">
                {t(item.labelKey)}
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap justify-end gap-1.5">
              {item.appliesTo.map((scope) => (
                <button
                  key={`${item.token}-${scope}`}
                  type="button"
                  className="rounded-full border border-border bg-background/70 px-2 py-0.5 text-[10px] font-semibold uppercase text-muted-foreground transition-colors hover:border-[var(--scry-accent)] hover:bg-accent hover:text-foreground"
                  onClick={() => applicabilityActions[scope](item.token)}
                >
                  {applicabilityLabels[scope]}
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function RenameFunctionReference({ t }: { t: Translate }) {
  return (
    <section className="space-y-2.5">
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">
        {t("settings.renameReferenceFunctions")}
      </h3>
      <div className="space-y-3">
        <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
          <p className="text-sm font-medium text-card-foreground">
            {t("settings.renameFunctionTruncateTitle")}
          </p>
          <code className="mt-1 block break-all font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
            {"{title|truncate:N}"}
          </code>
          <p className="mt-1 text-xs leading-snug text-muted-foreground">
            {t("settings.renameTruncateFilterLabel")}
          </p>
        </div>

        <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
          <p className="text-sm font-medium text-card-foreground">
            {t("settings.renameFunctionOptionalGroupTitle")}
          </p>
          <code className="mt-1 block break-all font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
            {"{?absolute_episode: ({absolute_episode})}"}
          </code>
          <p className="mt-1 text-xs leading-snug text-muted-foreground">
            {t("settings.renameOptionalGroupDescription")}
          </p>
        </div>

        <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
          <p className="text-sm font-medium text-card-foreground">
            {t("settings.renameFunctionWhitespaceTitle")}
          </p>
          <p className="mt-1 text-xs leading-snug text-muted-foreground">
            {t("settings.renameFunctionWhitespaceDescription")}
          </p>
          <div className="mt-2 grid gap-1.5 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
            {RENAME_SPACE_FILTER_REFERENCES.map((item) => (
              <div
                key={item.code}
                className="rounded border border-border/60 bg-background/50 px-2 py-1.5"
              >
                <code className="font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
                  {item.code}
                </code>
                <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
                  {t(item.labelKey)}
                </p>
              </div>
            ))}
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
          <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
            <p className="text-sm font-medium text-card-foreground">
              {t("settings.renameFunctionPaddingTitle")}
            </p>
            <code className="mt-1 block break-all font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
              {"{season:2}"}
            </code>
            <p className="mt-1 text-xs leading-snug text-muted-foreground">
              {t("settings.renameNumberPaddingLabel")}
            </p>
          </div>

          <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
            <p className="text-sm font-medium text-card-foreground">
              {t("settings.renameFunctionChainTitle")}
            </p>
            <code className="mt-1 block break-all font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
              {"{title|truncate:64|space:_}"}
            </code>
            <p className="mt-1 text-xs leading-snug text-muted-foreground">
              {t("settings.renameFilterChainLabel")}
            </p>
          </div>

          <div className="rounded-md border border-border/70 bg-muted/40 px-3 py-2.5">
            <p className="text-sm font-medium text-card-foreground">
              {t("settings.renameFunctionBracesTitle")}
            </p>
            <code className="mt-1 block break-all font-[var(--font-code)] text-xs text-[var(--scry-accent-text)]">
              {"{{edition-{edition}}}"}
            </code>
            <p className="mt-1 text-xs leading-snug text-muted-foreground">
              {t("settings.renameLiteralBracesLabel")}
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function RenameReferencePanel({
  tokens,
  onFolderTokenClick,
  onRenameTokenClick,
  t,
}: {
  tokens: TokenReference[];
  onFolderTokenClick: (token: string) => void;
  onRenameTokenClick: (token: string) => void;
  t: Translate;
}) {
  return (
    <Card className="min-w-0 rounded-[16px] border-[var(--scry-border)] bg-[var(--scry-surf)]">
      <CardHeader>
        <CardTitle>{t("settings.renameReferenceTitle")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-5">
        <RenameTokenReferenceList
          tokens={tokens}
          onFileTokenClick={onRenameTokenClick}
          onFolderTokenClick={onFolderTokenClick}
          t={t}
        />

        <RenameFunctionReference t={t} />
      </CardContent>
    </Card>
  );
}

type RenameSectionIcon = React.ComponentType<{ className?: string }>;

function RenameSettingsSection({
  icon: Icon,
  title,
  action,
  children,
}: {
  icon: RenameSectionIcon;
  title: string;
  action?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <section className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6">
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-center gap-2.5">
          <Icon className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
          <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">{title}</h2>
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      {children ? <div className="mt-5 space-y-5">{children}</div> : null}
    </section>
  );
}

function TemplateExample({
  value,
  outputId,
}: {
  value: string | null;
  outputId?: string;
}) {
  return (
    <div className="w-full space-y-2">
      <Label className="text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
        Example
      </Label>
      {value ? (
        <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] px-3.5 py-2.5">
          <p
            id={outputId}
            className="break-all font-[var(--font-code)] text-[13.5px] text-[var(--scry-text2)]"
          >
            {value}
          </p>
        </div>
      ) : (
        <div className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3.5 py-2.5">
          <p className="text-[13px] text-[var(--scry-faint)]">&mdash;</p>
        </div>
      )}
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
  categoryUseSeasonFolders,
  handleUseSeasonFoldersChange,
  categorySpecialsFolderTemplates,
  handleSpecialsFolderTemplateChange,
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
  categoryUseSeasonFolders: Record<ViewCategoryId, boolean>;
  handleUseSeasonFoldersChange: (checked: boolean) => void;
  categorySpecialsFolderTemplates: Record<ViewCategoryId, string>;
  handleSpecialsFolderTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
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
  const episodicScope = activeQualityScopeId !== "MOVIE";
  const useSeasonFolders = categoryUseSeasonFolders[activeQualityScopeId] !== false;
  const seasonFolderTemplateValue = categorySeasonFolderTemplates[activeQualityScopeId];
  const specialsFolderTemplateValue = categorySpecialsFolderTemplates[activeQualityScopeId];
  const renameEnabled = categoryRenameEnabled[activeQualityScopeId] !== "false";
  const templateValue = categoryRenameTemplates[activeQualityScopeId];
  const folderValidationError = React.useMemo(
    () => validateFolderTemplate(folderTemplateValue, t),
    [folderTemplateValue, t],
  );
  const renameValidationError = React.useMemo(
    () => (renameEnabled ? validateRenameTemplate(templateValue, activeQualityScopeId, t) : null),
    [activeQualityScopeId, renameEnabled, templateValue, t],
  );
  const seasonFolderValidationError = React.useMemo(
    () => episodicScope
      ? validateFolderTemplate(
          seasonFolderTemplateValue,
          t,
          VALID_SEASON_FOLDER_TOKENS,
          "season",
        )
      : null,
    [episodicScope, seasonFolderTemplateValue, t],
  );
  const specialsFolderValidationError = React.useMemo(
    () => episodicScope
      ? validateFolderTemplate(
          specialsFolderTemplateValue,
          t,
          VALID_SEASON_FOLDER_TOKENS,
        )
      : null,
    [episodicScope, specialsFolderTemplateValue, t],
  );

  const folderPreview = React.useMemo(
    () => applyFolderTemplate(folderTemplateValue, activeQualityScopeId),
    [activeQualityScopeId, folderTemplateValue],
  );
  const seasonFolderPreview = React.useMemo(
    () => applyFolderTemplate(
      seasonFolderTemplateValue,
      activeQualityScopeId,
      VALID_SEASON_FOLDER_TOKENS,
      "1",
    ),
    [activeQualityScopeId, seasonFolderTemplateValue],
  );
  const specialsFolderPreview = React.useMemo(
    () => applyFolderTemplate(
      specialsFolderTemplateValue,
      activeQualityScopeId,
      VALID_SEASON_FOLDER_TOKENS,
      "0",
    ),
    [activeQualityScopeId, specialsFolderTemplateValue],
  );

  const renamePreview = React.useMemo(
    () => (renameEnabled ? applyRenameTemplate(templateValue, activeQualityScopeId) : null),
    [activeQualityScopeId, renameEnabled, templateValue],
  );

  const folderInputRef = React.useRef<HTMLInputElement>(null);
  const seasonFolderInputRef = React.useRef<HTMLInputElement>(null);
  const specialsFolderInputRef = React.useRef<HTMLInputElement>(null);
  const templateInputRef = React.useRef<HTMLInputElement>(null);
  const renameTokenDescriptions = React.useMemo(
    () => getRenameTokenDescriptions(activeQualityScopeId),
    [activeQualityScopeId],
  );
  const referenceTokens = React.useMemo(
    () => getRenameReferenceTokens(renameTokenDescriptions, activeQualityScopeId),
    [activeQualityScopeId, renameTokenDescriptions],
  );
  const [referenceSlot, setReferenceSlot] = React.useState<HTMLElement | null>(null);

  React.useEffect(() => {
    setReferenceSlot(document.getElementById(FACET_REFERENCE_SLOT_ID));
  }, []);

  const insertFolderToken = React.useCallback(
    (token: string) => {
      const seasonToken = episodicScope && token === "season";
      const input = seasonToken ? seasonFolderInputRef.current : folderInputRef.current;
      if (!input) return;
      insertTemplateToken(
        input,
        seasonToken ? seasonFolderTemplateValue : folderTemplateValue,
        token,
      );
    },
    [episodicScope, folderTemplateValue, seasonFolderTemplateValue],
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
      if (!input) return;
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

  const autocompleteSpecialsFolderToken = React.useCallback(
    (token: string) => {
      const input = specialsFolderInputRef.current;
      if (!input) return;
      const cursor = input.selectionStart ?? specialsFolderTemplateValue.length;
      const context = resolveTemplateTokenContext(
        specialsFolderTemplateValue,
        cursor,
        SEASON_FOLDER_TOKEN_DESCRIPTIONS,
      );
      if (!context) {
        insertTemplateToken(input, specialsFolderTemplateValue, token);
        return;
      }
      applyAutocompleteToken(input, specialsFolderTemplateValue, context, token);
    },
    [specialsFolderTemplateValue],
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
    <>
      <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-[18px]">
        <RenameSettingsSection
          icon={Folder}
          title={t("settings.folderRenameSectionTitle")}
        >
          <div className="space-y-3">
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
                      ? "border-[var(--scry-danger-border-strong)]"
                      : "border-[var(--scry-accent)]"
                    : undefined
                }
              />
              {folderValidationError ? (
                <p className="text-xs text-[var(--scry-danger-text-soft)]">{folderValidationError}</p>
              ) : null}
            </div>

            <TemplateExample value={folderPreview} />

            {episodicScope ? (
              <div className="space-y-5 border-t border-[var(--scry-border)] pt-5">
                <div className="flex items-center justify-between gap-4 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] px-3.5 py-3">
                  <div>
                    <Label className="text-sm text-card-foreground">
                      {t("settings.useSeasonFoldersLabel")}
                    </Label>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {t("settings.useSeasonFoldersHelp")}
                    </p>
                  </div>
                  <SettingsToggleSwitch
                    id="rename-settings-use-season-folders-toggle"
                    checked={useSeasonFolders}
                    disabled={mediaSettingsLoading}
                    ariaLabel={useSeasonFolders ? t("label.enabled") : t("label.disabled")}
                    onChange={handleUseSeasonFoldersChange}
                  />
                </div>
              <div className="grid gap-5 md:grid-cols-2">
                <div className="space-y-3">
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
                      getSegments={(value) => splitFolderTemplateSegments(
                        value,
                        VALID_SEASON_FOLDER_TOKENS,
                      )}
                      placeholder={t("settings.seasonFolderTemplatePlaceholder")}
                      disabled={mediaSettingsLoading}
                      className={
                        seasonFolderTemplateValue.trim()
                          ? seasonFolderValidationError
                            ? "border-[var(--scry-danger-border-strong)]"
                            : "border-[var(--scry-accent)]"
                          : undefined
                      }
                    />
                    {seasonFolderValidationError ? (
                      <p className="text-xs text-[var(--scry-danger-text-soft)]">
                        {seasonFolderValidationError}
                      </p>
                    ) : null}
                  </div>
                  <TemplateExample value={seasonFolderPreview} />
                </div>

                <div className="space-y-3">
                  <div className="space-y-2.5">
                    <Label className="text-sm text-card-foreground">
                      {t("settings.specialsFolderTemplateLabel")}
                    </Label>
                    <TokenAutocompleteInput
                      inputRef={specialsFolderInputRef}
                      value={specialsFolderTemplateValue}
                      onChange={handleSpecialsFolderTemplateChange}
                      tokenDescriptions={SEASON_FOLDER_TOKEN_DESCRIPTIONS}
                      onAutocompleteToken={autocompleteSpecialsFolderToken}
                      translateLabel={t}
                      getSegments={(value) => splitFolderTemplateSegments(
                        value,
                        VALID_SEASON_FOLDER_TOKENS,
                      )}
                      placeholder={t("settings.specialsFolderTemplatePlaceholder")}
                      disabled={mediaSettingsLoading}
                      className={
                        specialsFolderTemplateValue.trim()
                          ? specialsFolderValidationError
                            ? "border-[var(--scry-danger-border-strong)]"
                            : "border-[var(--scry-accent)]"
                          : undefined
                      }
                    />
                    {specialsFolderValidationError ? (
                      <p className="text-xs text-[var(--scry-danger-text-soft)]">
                        {specialsFolderValidationError}
                      </p>
                    ) : null}
                  </div>
                  <TemplateExample value={specialsFolderPreview} />
                </div>
              </div>
              </div>
            ) : null}
          </div>
        </RenameSettingsSection>

        <RenameSettingsSection
          icon={FileText}
          title={t("settings.renameSectionTitle")}
          action={
            <SettingsToggleSwitch
              id="rename-settings-enabled-toggle"
              checked={renameEnabled}
              disabled={mediaSettingsLoading}
              ariaLabel={renameEnabled ? t("label.enabled") : t("label.disabled")}
              onChange={handleRenameEnabledChange}
            />
          }
        >
          {renameEnabled ? (
            <div className="space-y-5">
              <div className="space-y-3">
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
                    getSegments={(value) => splitRenameInputSegments(value, activeQualityScopeId)}
                    placeholder={t("settings.renameTemplatePlaceholder")}
                    disabled={mediaSettingsLoading}
                    className={
                      templateValue.trim()
                        ? renameValidationError
                          ? "border-[var(--scry-danger-border-strong)]"
                          : "border-[var(--scry-accent)]"
                        : undefined
                    }
                  />
                  {renameValidationError ? (
                    <p id="rename-settings-validation-message" className="text-xs text-[var(--scry-danger-text-soft)]">{renameValidationError}</p>
                  ) : null}
                </div>

                <TemplateExample
                  value={renamePreview}
                  outputId="rename-settings-preview-output"
                />
              </div>

              <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
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
              </div>
            </div>
          ) : null}
        </RenameSettingsSection>

        <div className="flex justify-end">
          <Button
            id="rename-settings-save"
            type="submit"
            disabled={
              mediaSettingsSaving ||
              folderValidationError !== null ||
              seasonFolderValidationError !== null ||
              specialsFolderValidationError !== null ||
              renameValidationError !== null
            }
          >
            <Save className="mr-1.5 h-4 w-4" />
            {mediaSettingsSaving ? t("label.saving") : t("label.save")}
          </Button>
        </div>
      </form>
      {referenceSlot
        ? createPortal(
            <RenameReferencePanel
              tokens={referenceTokens}
              onFolderTokenClick={insertFolderToken}
              onRenameTokenClick={insertToken}
              t={t}
            />,
            referenceSlot,
          )
        : null}
    </>
  );
}
