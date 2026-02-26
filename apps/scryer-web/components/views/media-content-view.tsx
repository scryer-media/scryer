
import * as React from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { InfoHelp } from "@/components/common/info-help";
import { ChevronDown, ChevronUp, Loader2, Power, PowerOff, Search, Trash2, Zap } from "lucide-react";
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
import { getDefaultIndexerRouting } from "@/lib/constants/indexers";
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

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type Facet = "movie" | "tv" | "anime";
type ContentSettingsSection = "overview" | "settings";
type ViewCategoryId = "movie" | "series" | "anime";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type IndexerCategoryDefinition = {
  code: string;
  labelKey: keyof typeof indexerCategoryLabelMap;
};

type IndexerCategoryGroupDefinition = {
  labelKey: keyof typeof indexerCategoryGroupLabelMap;
  code: string;
  categories: IndexerCategoryDefinition[];
};

function parseTagsInput(raw: string): string[] {
  return raw
    .split(",")
    .map((value) => value.trim())
    .map((value) => (value.length === 0 ? "" : value));
}

function formatTagsInput(tags: string[]): string {
  return tags.join(", ");
}

function sortCategoryCodes(values: string[]): string[] {
  return [...values].sort((left, right) => {
    const leftNumber = Number.parseInt(left, 10);
    const rightNumber = Number.parseInt(right, 10);

    if (Number.isNaN(leftNumber) || Number.isNaN(rightNumber)) {
      if (Number.isNaN(leftNumber) && Number.isNaN(rightNumber)) {
        return left.localeCompare(right, undefined, { numeric: true, sensitivity: "base" });
      }
      return Number.isNaN(leftNumber) ? 1 : -1;
    }

    return leftNumber === rightNumber
      ? left.localeCompare(right, undefined, { numeric: true, sensitivity: "base" })
      : leftNumber - rightNumber;
  });
}

const indexerCategoryGroupLabelMap = {
  tv: "settings.indexerCategoryTv",
  movies: "settings.indexerCategoryMovies",
  other: "settings.indexerCategoryOther",
  foreign: "settings.indexerCategoryForeign",
  sd: "settings.indexerCategorySd",
  hd: "settings.indexerCategoryHd",
  uhd: "settings.indexerCategoryUhd",
  anime: "settings.indexerCategoryAnime",
  documentary: "settings.indexerCategoryDocumentary",
  sport: "settings.indexerCategorySport",
  bluRay: "settings.indexerCategoryBluRay",
  threeD: "settings.indexerCategoryThreeD",
  misc: "settings.indexerCategoryMisc",
} as const;

const indexerCategoryLabelMap: Record<string, string> = {
  "5020": "settings.indexerCategoryForeign",
  "5030": "settings.indexerCategorySd",
  "5040": "settings.indexerCategoryHd",
  "5045": "settings.indexerCategoryUhd",
  "5050": "settings.indexerCategoryOther",
  "5060": "settings.indexerCategorySport",
  "5070": "settings.indexerCategoryAnime",
  "5080": "settings.indexerCategoryDocumentary",
  "8010": "settings.indexerCategoryMisc",
  "2010": "settings.indexerCategoryForeign",
  "2020": "settings.indexerCategoryOther",
  "2030": "settings.indexerCategorySd",
  "2040": "settings.indexerCategoryHd",
  "2045": "settings.indexerCategoryUhd",
  "2050": "settings.indexerCategoryBluRay",
  "2060": "settings.indexerCategoryThreeD",
};

const INDEXER_CATEGORY_DEFINITIONS: Record<string, IndexerCategoryGroupDefinition> = {
  tv: {
    labelKey: "tv",
    code: "5000",
    categories: [
      { code: "5020", labelKey: "settings.indexerCategoryForeign" },
      { code: "5030", labelKey: "settings.indexerCategorySd" },
      { code: "5040", labelKey: "settings.indexerCategoryHd" },
      { code: "5045", labelKey: "settings.indexerCategoryUhd" },
      { code: "5050", labelKey: "settings.indexerCategoryOther" },
      { code: "5060", labelKey: "settings.indexerCategorySport" },
      { code: "5070", labelKey: "settings.indexerCategoryAnime" },
      { code: "5080", labelKey: "settings.indexerCategoryDocumentary" },
    ],
  },
  movies: {
    labelKey: "movies",
    code: "2000",
    categories: [
      { code: "2010", labelKey: "settings.indexerCategoryForeign" },
      { code: "2020", labelKey: "settings.indexerCategoryOther" },
      { code: "2030", labelKey: "settings.indexerCategorySd" },
      { code: "2040", labelKey: "settings.indexerCategoryHd" },
      { code: "2045", labelKey: "settings.indexerCategoryUhd" },
      { code: "2050", labelKey: "settings.indexerCategoryBluRay" },
      { code: "2060", labelKey: "settings.indexerCategoryThreeD" },
    ],
  },
  other: {
    labelKey: "other",
    code: "8000",
    categories: [
      { code: "8010", labelKey: "settings.indexerCategoryMisc" },
    ],
  },
};

const INDEXER_CATEGORY_GROUPS_BY_SCOPE: Record<
  ViewCategoryId,
  Array<"movies" | "tv" | "other">
> = {
  movie: ["movies", "other"],
  series: ["tv", "other"],
  anime: ["tv", "other"],
};

function normalizeCategoryCodes(values: string[]): string[] {
  const seen = new Set<string>();
  const next: string[] = [];
  for (const value of values) {
    const normalized = value.trim();
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    next.push(normalized);
  }
  return next;
}

function formatCategoryCodeList(codes: string[]): string {
  return formatTagsInput(sortCategoryCodes(normalizeCategoryCodes(codes)));
}

function getSortedCategoryCodesByScope(scope: ViewCategoryId, values: string[]) {
  const normalized = normalizeCategoryCodes(values);
  const scopeGroups = INDEXER_CATEGORY_GROUPS_BY_SCOPE[scope]
    .map((groupKey) => {
      const group = INDEXER_CATEGORY_DEFINITIONS[groupKey];
      return [group.code, ...group.categories.map((category) => category.code)];
    })
    .flat();
  const orderedKnownCodes = scopeGroups.filter((code) => normalized.includes(code));
  const unknownCodes = normalized.filter((code) => !scopeGroups.includes(code));
  return [...orderedKnownCodes, ...unknownCodes];
}

function IndexerCategoryPicker({
  t,
  value,
  scope,
  disabled,
  onChange,
  categoriesLabel,
}: {
  t: Translate;
  value: string[];
  scope: ViewCategoryId;
  disabled: boolean;
  categoriesLabel?: string;
  onChange: (categories: string[]) => void;
}) {
  const pickerRef = React.useRef<HTMLDivElement>(null);
  const floatingPanelRef = React.useRef<HTMLDivElement>(null);
  const [isOpen, setIsOpen] = React.useState(false);
  const [draftCategories, setDraftCategories] = React.useState<string[]>(() =>
    getSortedCategoryCodesByScope(scope, value),
  );
  const [pickerRect, setPickerRect] = React.useState<DOMRect | null>(null);

  React.useEffect(() => {
    if (!isOpen) {
      setDraftCategories(getSortedCategoryCodesByScope(scope, value));
      return;
    }
    if (pickerRef.current && typeof window !== "undefined") {
      setPickerRect(pickerRef.current.getBoundingClientRect());
    }
  }, [isOpen, scope, value]);

  React.useEffect(() => {
    if (!isOpen) {
      return;
    }
    const handlePointerDown = (event: MouseEvent) => {
      if (!pickerRef.current) {
        return;
      }
      if (
        !pickerRef.current?.contains(event.target as Node) &&
        !floatingPanelRef.current?.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };

    const handleScrollOrResize = () => {
      if (!pickerRef.current || typeof window === "undefined") {
        return;
      }
      setPickerRect(pickerRef.current.getBoundingClientRect());
    };

    document.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("scroll", handleScrollOrResize, true);
    window.addEventListener("resize", handleScrollOrResize, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("scroll", handleScrollOrResize, true);
      window.removeEventListener("resize", handleScrollOrResize, true);
    };
  }, [isOpen]);

  const selectedSet = React.useMemo(
    () => new Set<string>(normalizeCategoryCodes(draftCategories)),
    [draftCategories],
  );

  const toggleCategory = (code: string) => {
    setDraftCategories((previous) => {
      const current = new Set(previous.map((entry) => entry.trim()));
      if (current.has(code)) {
        current.delete(code);
      } else {
        current.add(code);
      }
      const sorted = getSortedCategoryCodesByScope(scope, Array.from(current));
      onChange(sorted);
      return sorted;
    });
  };

  const toggleCategoryHeader = (groupKey: "movies" | "tv" | "other") => {
    const groupCode = INDEXER_CATEGORY_DEFINITIONS[groupKey].code;
    setDraftCategories((previous) => {
      const next = new Set(previous.map((entry) => entry.trim()));
      if (next.has(groupCode)) {
        next.delete(groupCode);
      } else {
        next.add(groupCode);
      }
      const sorted = getSortedCategoryCodesByScope(scope, Array.from(next));
      onChange(sorted);
      return sorted;
    });
  };

  const floatingPanel = isOpen && pickerRect && !disabled
    ? createPortal(
        <div
          ref={floatingPanelRef}
          className="z-50 max-h-80 overflow-y-auto rounded-xl border border-border bg-popover p-2 shadow-lg"
          style={{
            position: "fixed",
            top: pickerRect.bottom + 4,
            left: pickerRect.left,
            width: Math.max(260, Math.round(pickerRect.width)),
          }}
        >
          {INDEXER_CATEGORY_GROUPS_BY_SCOPE[scope].map((groupKey) => {
            const group = INDEXER_CATEGORY_DEFINITIONS[groupKey];
            return (
              <div key={groupKey} className="mb-2 last:mb-0">
                <label className="mb-1 flex items-center justify-between rounded-md px-2 py-1 text-sm capitalize text-foreground hover:bg-accent/60">
                  <span className="flex items-center gap-2">
                    <Checkbox
                      checked={selectedSet.has(group.code)}
                      onCheckedChange={() => toggleCategoryHeader(groupKey)}
                      aria-label={`${t("indexerCategory.labelCategory")}: ${t(group.labelKey)} ${group.code}`}
                      disabled={disabled}
                    />
                    {t(group.labelKey)}
                  </span>
                  <span className="text-xs font-mono text-muted-foreground">{group.code}</span>
                </label>
                <div className="space-y-1 pl-3">
                  {group.categories.map((category) => (
                    <label
                      key={category.code}
                      className="flex items-center justify-between gap-3 rounded-md px-2 py-1 text-sm capitalize text-foreground hover:bg-accent/60"
                    >
                      <span className="flex items-center gap-2 text-foreground">
                        <Checkbox
                          checked={selectedSet.has(category.code)}
                          onCheckedChange={() => toggleCategory(category.code)}
                          aria-label={`${t("indexerCategory.labelCategory")}: ${t(category.labelKey)} ${category.code}`}
                          disabled={disabled}
                        />
                        {t(category.labelKey)}
                      </span>
                      <span className="text-xs font-mono text-muted-foreground">{category.code}</span>
                    </label>
                  ))}
                </div>
              </div>
            );
          })}
        </div>,
        document.body,
      )
    : null;

  return (
    <div ref={pickerRef} className="relative inline-block w-full">
      <Button
        type="button"
        variant="secondary"
        className="h-auto w-full justify-between gap-2 border border-input bg-field px-3 py-2 text-sm"
        onClick={() => setIsOpen((previous) => !previous)}
        disabled={disabled}
        aria-label={categoriesLabel || t("settings.indexerRoutingCategories")}
      >
        <span
          className={`truncate text-left ${formatCategoryCodeList(draftCategories) ? "text-card-foreground" : "text-muted-foreground"}`}
        >
          {formatCategoryCodeList(draftCategories) || t("settings.indexerRoutingCategoriesPlaceholder")}
        </span>
        <ChevronDown className={`h-4 w-4 transition-transform ${isOpen ? "rotate-180" : ""}`} />
      </Button>
      {floatingPanel}
    </div>
  );
}

type DownloadClientTypeOption = {
  value: string;
  icon: (props: React.ComponentPropsWithoutRef<"svg">) => React.JSX.Element;
};

const NzbgetIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox="182.47 40.33 528 528"
    fill="none"
    {...props}
  >
    <ellipse cx="446.47417" cy="304.33312" rx="264" ry="263.99999" fill="#fafafa" />
    <ellipse cx="445.47418" cy="304.99977" rx="239.8589" ry="239.66666" fill="#333733" />
    <ellipse cx="445.33311" cy="303.33311" rx="226" ry="226" fill="#37d134" />
    <path
      d="m330.34323,434.81804l116.49998,-116.66662l116.49998,116.66662l-232.99996,0z"
      fill="#000000"
      transform="rotate(-180 446.843 376.485)"
    />
    <rect x="398.66641" y="266.66647" width="94.66664" height="51.33332" fill="#000000" />
    <path d="m399.33309,215.33316l92.66665,0l0,33.33332l-92.66665,0l0,-33.33332z" fill="#000000" />
    <path d="m399.33309,163.99984l92.66664,0l0,33.33332l-92.66664,0l0,-33.33332z" fill="#000000" />
  </svg>
);

const QBitTorrentIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    viewBox="0 0 1024 1024"
    fill="none"
    {...props}
  >
    <circle
      cx="512"
      cy="512"
      r="496"
      fill="#72b4f5"
      stroke="#daefff"
      strokeWidth="32"
    />
    <path
      d="M712.9 332.4c44.4 0 78.9 15.2 103.4 45.7 24.7 30.2 37 73.1 37 128.7 0 55.5-12.4 98.8-37.3 129.6-24.7 30.7-59 46-103.1 46-22 0-42.2-4-60.5-12-18.1-8.2-33.3-20.8-45.7-37.6H603l-10.8 43.5h-36.7V196h51.2v116.6c0 26.1-.8 49.6-2.5 70.4h2.5c23.9-33.7 59.3-50.6 106.2-50.6m-7.4 42.9c-35 0-60.2 10.1-75.6 30.2-15.4 20-23.1 53.7-23.1 101.2s7.9 81.6 23.8 102.1c15.8 20.4 41.2 30.5 76.2 30.5 31.5 0 54.9-11.4 70.4-34.3 15.4-23 23.1-56.1 23.1-99.1q0-66-23.1-98.4c-15.5-21.4-39.4-32.2-71.7-32.2"
      fill="#ffffff"
    />
    <path
      d="M317.3 639.5c34.2 0 59-9.2 74.7-27.5 15.6-18.3 24-49.2 25-92.6V508c0-47.3-8-81.4-24.1-102.1-16-20.8-41.5-31.2-76.2-31.2-30 0-53.1 11.7-69.1 35.2-15.8 23.2-23.8 56.2-23.8 98.8s7.8 75.1 23.5 97.5c15.8 22.1 39.1 33.2 70 33.3m-7.7 42.8c-43.6 0-77.7-15.3-102.1-46-24.5-30.7-36.7-73.4-36.7-128.4 0-55.3 12.3-98.5 37-129.6s59-46.6 103.1-46.6q69.45 0 106.8 52.5h2.8l7.4-46.3h40.4v490h-51.2V683.3c0-20.6 1.1-38.1 3.4-52.5h-4c-23.8 34.4-59.4 51.5-106.9 51.5"
      fill="#c8e8ff"
    />
  </svg>
);

const SabnzbdIcon = (props: React.ComponentPropsWithoutRef<"svg">) => (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000" fill="none" {...props}>
    <path
      fill="none"
      stroke="#f5f5f5"
      strokeLinejoin="round"
      strokeWidth="74"
      d="M200.4 39.3h598.1v437.8h161l-460.1 483L39.4 477h161z"
    />
    <path fill="#ffb300" fillRule="evenodd" d="M200.4 39.3h598.1v437.8h161l-460.1 483-460-483h161z" />
    <path fill="#ffca28" fillRule="evenodd" d="M499.4 960.2 201.1 39.4h596.7z" />
    <path
      fill="none"
      stroke="#f5f5f5"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="74"
      d="M329.2 843.5H83v-51.8h146.1v-45.9H83V596.9h246.2v51.5H183.1v45.9h146.1zm292.2 0H375.2V694.3h146.1v-45.9H375.2v-51.5h246.2zm-146.1-97.8h46v46h-46zm192.1 97.8v-344h100.1v97.4h146.1v246.6zm100.1-195.2h46v143.4h-46z"
    />
    <path
      fill="#0f0f0f"
      fillRule="evenodd"
      d="M329.2 843.5H83v-51.8h146.1v-45.9H83V596.9h246.2v51.5H183.1v45.9h146.1zm292.2 0H375.2V694.3h146.1v-45.9H375.2v-51.5h246.2zm-146.1-51.8h46v-46h-46zm192.1 51.9v-344h100.1V597h146.1v246.6zm100.1-51.9h46V648.4h-46z"
    />
  </svg>
);

const DOWNLOAD_CLIENT_TYPE_OPTIONS: DownloadClientTypeOption[] = [
  {
    value: "nzbget",
    icon: NzbgetIcon,
  },
  {
    value: "sabnzbd",
    icon: SabnzbdIcon,
  },
  {
    value: "qbittorrent",
    icon: QBitTorrentIcon,
  },
];

function getDownloadClientTypeOption(typeValue: string) {
  const normalizedType = typeValue.trim().toLowerCase();
  return (
    DOWNLOAD_CLIENT_TYPE_OPTIONS.find((option) => option.value === normalizedType) ??
    DOWNLOAD_CLIENT_TYPE_OPTIONS[0]
  );
}

function DownloadClientTypeLogo({
  typeValue,
  className = "h-4 w-4",
}: {
  typeValue: string;
  className?: string;
}) {
  const option = getDownloadClientTypeOption(typeValue);
  const FallbackIcon = option.icon;
  return <FallbackIcon className={`${className} object-contain`} aria-hidden="true" role="img" />;
}

type QualityProfileOption = {
  value: string;
  label: string;
};

const DOWNLOAD_PRIORITY_OPTIONS = [
  { value: "force", label: "settings.downloadClientPriorityForce" },
  { value: "very high", label: "settings.downloadClientPriorityVeryHigh" },
  { value: "high", label: "settings.downloadClientPriorityHigh" },
  { value: "normal", label: "settings.downloadClientPriorityNormal" },
  { value: "low", label: "settings.downloadClientPriorityLow" },
  { value: "very low", label: "settings.downloadClientPriorityVeryLow" },
];

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

function applyRenameTemplate(template: string, scopeId: ViewCategoryId): string | null {
  if (!template.trim()) return null;
  let result = "";
  let i = 0;
  const sampleValues =
    scopeId === "series" ? RENAME_PREVIEW_SERIES_SAMPLE : RENAME_PREVIEW_MOVIE_SAMPLE;
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

const PRIORITY_VALUES = new Set(DOWNLOAD_PRIORITY_OPTIONS.map((item) => item.value));

function normalizePriorityValue(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
}

function normalizePriorityValueForSave(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
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

  const clientById = React.useMemo(
    () => Object.fromEntries(downloadClients.map((client) => [client.id, client])),
    [downloadClients],
  );
  const orderedDownloadClientIds = React.useMemo(() => {
    const configuredIds = activeScopeRoutingOrder.filter((clientId) => clientById[clientId]);
    const configuredIdSet = new Set(configuredIds);
    const missingIds = downloadClients
      .map((client) => client.id)
      .filter((clientId) => !configuredIdSet.has(clientId));
    return [...configuredIds, ...missingIds];
  }, [activeScopeRoutingOrder, clientById, downloadClients]);

  const indexerById = React.useMemo(
    () => Object.fromEntries(indexers.map((indexer) => [indexer.id, indexer])),
    [indexers],
  );
  const orderedIndexerIds = React.useMemo(() => {
    const configuredIds = activeScopeIndexerRoutingOrder.filter((indexerId) => indexerById[indexerId]);
    const configuredIdSet = new Set(configuredIds);
    const missingIds = indexers
      .map((indexer) => indexer.id)
      .filter((indexerId) => !configuredIdSet.has(indexerId));
    return [...configuredIds, ...missingIds];
  }, [activeScopeIndexerRoutingOrder, indexerById, indexers]);

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

  const handleDownloadClientRoutingSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      void saveDownloadClientRouting();
    },
    [saveDownloadClientRouting],
  );

  const handleRoutingCategoryChange = React.useCallback(
    (clientId: string, value: string) => {
      updateDownloadClientRoutingForScope(clientId, {
        category: value,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingTagsChange = React.useCallback(
    (clientId: string, value: string) => {
      updateDownloadClientRoutingForScope(clientId, {
        tags: parseTagsInput(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRecentPriorityChange = React.useCallback(
    (clientId: string, value: string) => {
      updateDownloadClientRoutingForScope(clientId, {
        recentPriority: normalizePriorityValueForSave(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingOlderPriorityChange = React.useCallback(
    (clientId: string, value: string) => {
      updateDownloadClientRoutingForScope(clientId, {
        olderPriority: normalizePriorityValueForSave(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRemoveCompletedChange = React.useCallback(
    (clientId: string, checked: boolean) => {
      updateDownloadClientRoutingForScope(clientId, {
        removeCompleted: checked,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRemoveFailedChange = React.useCallback(
    (clientId: string, checked: boolean) => {
      updateDownloadClientRoutingForScope(clientId, {
        removeFailed: checked,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const moveClientUp = React.useCallback(
    (clientId: string) => {
      moveDownloadClientInScope(clientId, "up");
    },
    [moveDownloadClientInScope],
  );

  const moveClientDown = React.useCallback(
    (clientId: string) => {
      moveDownloadClientInScope(clientId, "down");
    },
    [moveDownloadClientInScope],
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
            <>
              <Card>
                <CardHeader>
                  <CardTitle>{mediaLibrarySettingsTitle}</CardTitle>
                </CardHeader>
                <CardContent>
                  <label>
                    <Label className="mb-2 block">{mediaLibraryPathLabel}</Label>
                    <Input
                      value={mediaLibraryPathValue}
                      onChange={mediaLibraryPathChangeHandler}
                      placeholder={mediaLibraryPathPlaceholder}
                      required={view === "movies" || view === "series"}
                      disabled={mediaSettingsLoading}
                    />
                    <p className="mt-1 text-xs text-muted-foreground">
                      {mediaSettingsLoading ? t("label.loading") : mediaLibraryPathHelp}
                    </p>
                  </label>
                </CardContent>
              </Card>
              <Card>
                <CardHeader>
                  <CardTitle>{t("settings.libraryScanTitle")}</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  <p className="text-sm text-muted-foreground">{t("settings.libraryScanHelp")}</p>
                  <div className="flex flex-wrap items-center gap-3">
                    <Button
                      type="button"
                      onClick={handleLibraryScan}
                      disabled={libraryScanLoading}
                    >
                      {libraryScanLoading
                        ? t("settings.libraryScanRunning")
                        : t("settings.libraryScanButton")}
                    </Button>
                    {libraryScanSummary ? (
                      <span className="text-xs text-muted-foreground">
                        {t("settings.libraryScanSummary", {
                          imported: libraryScanSummary.imported,
                          skipped: libraryScanSummary.skipped,
                          unmatched: libraryScanSummary.unmatched,
                        })}
                      </span>
                    ) : null}
                  </div>
                </CardContent>
              </Card>
            </>
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
            updateCategoryMediaProfileSettings={updateCategoryMediaProfileSettings}
            mediaSettingsSaving={mediaSettingsSaving}
          />

          <div>
            <Card>
              <CardHeader>
                <CardTitle>
                  {t("settings.indexerRoutingScope", {
                    scope: scopeLabel,
                  })}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <div className="overflow-x-auto rounded border border-border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                    <TableHead>{t("settings.indexerRoutingPriority")}</TableHead>
                        <TableHead>{t("settings.indexerName")}</TableHead>
                        <TableHead>{t("settings.indexerRoutingCategories")}</TableHead>
                        <TableHead className="text-center">{t("settings.indexerRoutingGloballyEnabled")}</TableHead>
                        <TableHead className="text-center">{t("settings.indexerRoutingEnabled")}</TableHead>
                        <TableHead className="text-right">{t("label.actions")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {orderedIndexerIds.length === 0 ? (
                        <TableRow>
                          <TableCell colSpan={6} className="text-muted-foreground">
                            {t("settings.indexerRoutingNoIndexers")}
                          </TableCell>
                        </TableRow>
                      ) : (
                        orderedIndexerIds.map((indexerId, index) => {
                          const indexer = indexerById[indexerId];
                          if (!indexer) {
                            return null;
                          }
                          const routing = activeScopeIndexerRouting[indexer.id] ?? getDefaultIndexerRouting(activeQualityScopeId);
                          return (
                            <TableRow key={indexer.id}>
                              <TableCell>{index + 1}</TableCell>
                              <TableCell>{indexer.name}</TableCell>
                              <TableCell className="w-[30rem] min-w-[30rem] max-w-[30rem]">
                                <IndexerCategoryPicker
                                  t={t}
                                  value={routing.categories}
                                  scope={activeQualityScopeId}
                                  disabled={indexerRoutingLoading}
                                  categoriesLabel={`${t("settings.indexerRoutingCategories")} (${indexer.name})`}
                                  onChange={(categories) =>
                                    handleIndexerCategoriesChange(indexer.id, categories)
                                  }
                                  />
                              </TableCell>
                              <TableCell className="text-center align-middle">
                                <RenderBooleanIcon
                                  value={indexer.isEnabled}
                                  label={`${t("settings.indexerRoutingGloballyEnabled")}: ${indexer.name}`}
                                />
                              </TableCell>
                              <TableCell className="text-center align-middle">
                                <RenderBooleanIcon
                                  value={routing.enabled}
                                  label={`${t("settings.indexerRoutingEnabled")}: ${indexer.name}`}
                                />
                              </TableCell>
                              <TableCell className="text-right">
                                <div className="flex items-center justify-end gap-1">
                                  <Button
                                    variant="secondary"
                                    size="icon-sm"
                                    type="button"
                                    aria-label={
                                      routing.enabled
                                        ? t("label.disabled")
                                        : t("label.enabled")
                                    }
                                    title={
                                      routing.enabled
                                        ? t("label.disabled")
                                        : t("label.enabled")
                                    }
                                    onClick={() =>
                                      handleIndexerEnabledChange(indexer.id, !routing.enabled)
                                    }
                                    disabled={indexerRoutingLoading || indexerRoutingSaving}
                                    className={
                                      routing.enabled
                                        ? "border-red-700/70 bg-red-900/60 text-red-200 hover:bg-red-900/80 hover:text-red-100"
                                        : "border-emerald-300/70 dark:border-emerald-700/70 bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-100 hover:bg-emerald-200 dark:hover:bg-emerald-800/80"
                                    }
                                  >
                                    {routing.enabled ? (
                                      <PowerOff className="h-3.5 w-3.5" />
                                    ) : (
                                      <Power className="h-3.5 w-3.5" />
                                    )}
                                    <span className="sr-only">
                                      {routing.enabled
                                        ? t("label.disabled")
                                        : t("label.enabled")}
                                    </span>
                                  </Button>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    type="button"
                                    className="border border-border bg-card/80 hover:bg-accent"
                                    aria-label={`${t("label.moveUp")} ${indexer.name}`}
                                    onClick={() => moveIndexerUp(indexer.id)}
                                    disabled={
                                      indexerRoutingLoading ||
                                      indexerRoutingSaving ||
                                      index === 0
                                    }
                                  >
                                    <ChevronUp className="h-4 w-4" />
                                  </Button>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    type="button"
                                    className="border border-border bg-card/80 hover:bg-accent"
                                    aria-label={`${t("label.moveDown")} ${indexer.name}`}
                                    onClick={() => moveIndexerDown(indexer.id)}
                                    disabled={
                                      indexerRoutingLoading ||
                                      indexerRoutingSaving ||
                                      index >= orderedIndexerIds.length - 1
                                    }
                                  >
                                    <ChevronDown className="h-4 w-4" />
                                  </Button>
                                </div>
                              </TableCell>
                            </TableRow>
                          );
                        })
                      )}
                    </TableBody>
                  </Table>
                </div>
              </CardContent>
            </Card>
          </div>

          <form
            onSubmit={handleDownloadClientRoutingSubmit}
          >
            <Card>
              <CardHeader>
                <CardTitle>
                  {t("settings.downloadClientRoutingScope", {
                    scope: scopeLabel,
                  })}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <div className="overflow-x-auto rounded border border-border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>{t("settings.downloadClientPriority")}</TableHead>
                        <TableHead>{t("settings.downloadClientName")}</TableHead>
                        <TableHead>{t("settings.downloadClientType")}</TableHead>
                        <TableHead>{t("settings.downloadClientCategory")}</TableHead>
                        <TableHead>{t("settings.downloadClientTags")}</TableHead>
                        <TableHead>{t("settings.downloadClientRecentPriority")}</TableHead>
                        <TableHead>{t("settings.downloadClientOlderPriority")}</TableHead>
                        <TableHead className="text-center">{t("settings.downloadClientRemoveCompleted")}</TableHead>
                        <TableHead className="text-center">{t("settings.downloadClientRemoveFailed")}</TableHead>
                        <TableHead className="text-right">{t("label.actions")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {orderedDownloadClientIds.length === 0 ? (
                        <TableRow>
                          <TableCell colSpan={10} className="text-muted-foreground">
                            {t("settings.noDownloadClientsFound")}
                          </TableCell>
                        </TableRow>
                      ) : (
                        orderedDownloadClientIds.map((clientId, index) => {
                          const client = clientById[clientId];
                          if (!client) {
                            return null;
                          }
                          const routing = activeScopeRouting[client.id] ?? {
                            category: "",
                            recentPriority: "",
                            olderPriority: "",
                            tags: [],
                            removeCompleted: false,
                            removeFailed: false,
                          };
                          return (
                            <TableRow key={client.id}>
                              <TableCell>{index + 1}</TableCell>
                              <TableCell>{client.name}</TableCell>
                              <TableCell className="text-center">
                                <span className="inline-flex items-center justify-center">
                                  <DownloadClientTypeLogo typeValue={client.clientType} />
                                  <span className="sr-only">{client.clientType}</span>
                                </span>
                              </TableCell>
                              <TableCell>
                                <Input
                                  value={routing.category}
                                  onChange={(event) =>
                                    handleRoutingCategoryChange(client.id, event.target.value)
                                  }
                                  disabled={downloadClientRoutingLoading}
                                  placeholder={t("settings.downloadClientCategoryPlaceholder")}
                                />
                              </TableCell>
                              <TableCell>
                                <Input
                                  value={formatTagsInput(routing.tags)}
                                  onChange={(event) => handleRoutingTagsChange(client.id, event.target.value)}
                                  disabled={downloadClientRoutingLoading}
                                  placeholder={t("settings.downloadClientTagsPlaceholder")}
                                />
                              </TableCell>
                              <TableCell>
                                <Select value={normalizePriorityValue(routing.recentPriority)} onValueChange={(v) => handleRoutingRecentPriorityChange(client.id, v)} disabled={downloadClientRoutingLoading}>
                                  <SelectTrigger className="w-full">
                                    <SelectValue />
                                  </SelectTrigger>
                                  <SelectContent>
                                    {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
                                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                                    ))}
                                  </SelectContent>
                                </Select>
                              </TableCell>
                              <TableCell>
                                <Select value={normalizePriorityValue(routing.olderPriority)} onValueChange={(v) => handleRoutingOlderPriorityChange(client.id, v)} disabled={downloadClientRoutingLoading}>
                                  <SelectTrigger className="w-full">
                                    <SelectValue />
                                  </SelectTrigger>
                                  <SelectContent>
                                    {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
                                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                                    ))}
                                  </SelectContent>
                                </Select>
                              </TableCell>
                              <TableCell className="text-center">
                                <Checkbox
                                  checked={routing.removeCompleted}
                                  onCheckedChange={(checked) =>
                                    handleRoutingRemoveCompletedChange(client.id, checked === true)
                                  }
                                  disabled={downloadClientRoutingLoading}
                                />
                              </TableCell>
                              <TableCell className="text-center">
                                <Checkbox
                                  checked={routing.removeFailed}
                                  onCheckedChange={(checked) =>
                                    handleRoutingRemoveFailedChange(client.id, checked === true)
                                  }
                                  disabled={downloadClientRoutingLoading}
                                />
                              </TableCell>
                              <TableCell className="text-right">
                                <div className="flex items-center justify-end gap-1">
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    type="button"
                                    className="border border-border bg-card/80 hover:bg-accent"
                                    aria-label={`${t("label.moveUp")} ${client.name}`}
                                    onClick={() => moveClientUp(client.id)}
                                    disabled={
                                      downloadClientRoutingLoading ||
                                      downloadClientRoutingSaving ||
                                      index === 0
                                    }
                                  >
                                    <ChevronUp className="h-4 w-4" />
                                  </Button>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    type="button"
                                    className="border border-border bg-card/80 hover:bg-accent"
                                    aria-label={`${t("label.moveDown")} ${client.name}`}
                                    onClick={() => moveClientDown(client.id)}
                                    disabled={
                                      downloadClientRoutingLoading ||
                                      downloadClientRoutingSaving ||
                                      index >= orderedDownloadClientIds.length - 1
                                    }
                                  >
                                    <ChevronDown className="h-4 w-4" />
                                  </Button>
                                </div>
                              </TableCell>
                            </TableRow>
                          );
                        })
                      )}
                    </TableBody>
                  </Table>
                </div>
                <div className="mt-3 flex justify-end">
                  <Button
                    type="submit"
                    disabled={
                      downloadClientRoutingLoading ||
                      downloadClientRoutingSaving ||
                      orderedDownloadClientIds.length === 0
                    }
                  >
                    {downloadClientRoutingSaving ? t("label.saving") : t("label.save")}
                  </Button>
                </div>
              </CardContent>
            </Card>
          </form>

        </div>
      ) : (
        view === "movies" || view === "series" || view === "anime" ? (
          <Card>
            <CardHeader>
              <CardTitle>{view === "movies" ? t("title.manageMovies") : view === "anime" ? t("nav.anime") : t("nav.series")}</CardTitle>
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
              {(() => {
                const isMovieView = view === "movies";
                const columnCount = isMovieView ? 6 : 5;
                const resolvedProfileName = (() => {
                  const overrideId = categoryQualityProfileOverrides[activeQualityScopeId];
                  if (!overrideId || overrideId === qualityProfileInheritValue) return null;
                  return qualityProfiles.find((p) => p.id === overrideId)?.name ?? null;
                })();

                return (
                  <Table>
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
                    <TableBody>
                      {monitoredTitles.map((item) => {
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
                      })}
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
