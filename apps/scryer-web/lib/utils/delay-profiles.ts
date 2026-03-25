import type {
  DelayProfileDraft,
  DelayProfileFacet,
  DelayProfileProtocol,
  ParsedDelayProfile,
} from "@/lib/types/delay-profiles";

export const DELAY_PROFILE_CATALOG_KEY = "acquisition.delay_profiles";

const FACET_OPTIONS: readonly DelayProfileFacet[] = ["movie", "series", "anime"];

export function buildDelayProfileTemplate(existing: ParsedDelayProfile[]): DelayProfileDraft {
  const maxPriority = existing.reduce((max, p) => Math.max(max, p.priority), 0);
  return {
    id: "",
    name: "",
    usenet_delay_minutes: 0,
    torrent_delay_minutes: 0,
    preferred_protocol: "usenet",
    min_age_minutes: 0,
    bypass_score_threshold: null,
    applies_to_facets: [],
    tags: [],
    priority: maxPriority + 1,
    enabled: true,
  };
}

export function createDelayProfileId(name: string, existing: ParsedDelayProfile[]): string {
  const base = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  const slug = base || "profile";
  const ids = new Set(existing.map((p) => p.id));
  if (!ids.has(slug)) return slug;
  for (let i = 2; ; i++) {
    const candidate = `${slug}-${i}`;
    if (!ids.has(candidate)) return candidate;
  }
}

export function validateDelayProfileDraft(profile: DelayProfileDraft): string | null {
  if (!profile.name.trim()) {
    return "Delay profile name is required.";
  }
  if (profile.id && !profile.id.trim()) {
    return "Delay profile id must not be blank.";
  }
  if (profile.usenet_delay_minutes < 0 || profile.torrent_delay_minutes < 0) {
    return "Delay minutes must be zero or greater.";
  }
  if (profile.min_age_minutes < 0) {
    return "Minimum usenet age must be zero or greater.";
  }
  if (profile.bypass_score_threshold != null && profile.bypass_score_threshold < 0) {
    return "Bypass score threshold must be zero or greater.";
  }
  return null;
}

export function parseDelayProfileCatalog(json: string): ParsedDelayProfile[] {
  try {
    const raw = JSON.parse(json);
    if (!Array.isArray(raw)) return [];
    return raw.map(normalizeDelayProfile);
  } catch {
    return [];
  }
}

function normalizeDelayProfile(raw: Record<string, unknown>): ParsedDelayProfile {
  const appliesToFacets = Array.isArray(raw.applies_to_facets)
    ? raw.applies_to_facets
        .map((value) => normalizeFacet(value))
        .filter((value): value is DelayProfileFacet => value !== null)
    : [];

  return {
    id: String(raw.id ?? ""),
    name: String(raw.name ?? "").trim(),
    usenet_delay_minutes: normalizeWholeNumber(raw.usenet_delay_minutes),
    torrent_delay_minutes: normalizeWholeNumber(raw.torrent_delay_minutes),
    preferred_protocol: normalizeProtocol(raw.preferred_protocol),
    min_age_minutes: normalizeWholeNumber(raw.min_age_minutes),
    bypass_score_threshold:
      typeof raw.bypass_score_threshold === "number"
        ? Math.max(0, Math.trunc(raw.bypass_score_threshold))
        : null,
    applies_to_facets: Array.from(new Set(appliesToFacets)),
    tags: Array.isArray(raw.tags)
      ? raw.tags
          .filter((v): v is string => typeof v === "string")
          .map((value) => value.trim())
          .filter(Boolean)
      : [],
    priority: normalizeSignedWholeNumber(raw.priority),
    enabled: typeof raw.enabled === "boolean" ? raw.enabled : true,
  };
}

export function serializeDelayProfileCatalog(profiles: ParsedDelayProfile[]): string {
  return JSON.stringify(profiles.map((profile) => normalizeDelayProfile(profile)));
}

function normalizeProtocol(value: unknown): DelayProfileProtocol {
  return value === "torrent" ? "torrent" : "usenet";
}

function normalizeFacet(value: unknown): DelayProfileFacet | null {
  if (typeof value !== "string") {
    return null;
  }

  switch (value.trim().toLowerCase()) {
    case "movie":
      return "movie";
    case "series":
    case "tv":
      return "series";
    case "anime":
      return "anime";
    default:
      return null;
  }
}

function normalizeWholeNumber(value: unknown): number {
  return typeof value === "number" ? Math.max(0, Math.trunc(value)) : 0;
}

function normalizeSignedWholeNumber(value: unknown): number {
  return typeof value === "number" ? Math.trunc(value) : 0;
}

export { FACET_OPTIONS };
